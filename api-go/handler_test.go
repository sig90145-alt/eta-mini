package main

import (
	"encoding/json"
	"math"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// rulePredictor 는 ONNX Runtime 없이 계약만 확인하려고 쓰는 가짜 모델이다 (api/predictors/rule.py 와 같은 식).
type rulePredictor struct{}

func (rulePredictor) Predict(d, o float32) (float32, error) { return 10 + 3*d + 2*o, nil }

type infPredictor struct{}

func (infPredictor) Predict(float32, float32) (float32, error) { return float32(math.Inf(1)), nil }

type response struct {
	status int
	body   map[string]any
}

func call(t *testing.T, h http.Handler, method, path, contentType, body string) response {
	t.Helper()
	req := httptest.NewRequest(method, path, strings.NewReader(body))
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if ct := rec.Header().Get("Content-Type"); ct != "application/json" {
		t.Fatalf("Content-Type = %q, want application/json", ct)
	}
	var m map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &m); err != nil {
		t.Fatalf("응답이 JSON 객체가 아니다: %v (%s)", err, rec.Body.String())
	}
	return response{rec.Code, m}
}

// detailTypes 는 422 응답 detail 의 type 들을 순서대로 돌려준다.
func detailTypes(t *testing.T, body map[string]any) []string {
	t.Helper()
	items, ok := body["detail"].([]any)
	if !ok {
		t.Fatalf("detail 이 배열이 아니다: %v", body)
	}
	var types []string
	for _, it := range items {
		types = append(types, it.(map[string]any)["type"].(string))
	}
	return types
}

// TestHealth 는 GET /health 가 상태와 모델 이름을 돌려주는지 확인한다.
func TestHealth(t *testing.T) {
	r := call(t, newHandler(rulePredictor{}), http.MethodGet, "/health", "", "")
	if r.status != 200 || r.body["status"] != "ok" || r.body["model"] != "onnx" {
		t.Fatalf("got %d %v", r.status, r.body)
	}
}

// TestEtaAccepted 는 pydantic lax 모드가 받아 주는 입력을 같은 값으로 계산하는지 확인한다.
func TestEtaAccepted(t *testing.T) {
	cases := []struct {
		body string
		want float64
	}{
		{`{"distance_km": 2.5, "pending_orders": 3}`, 23.5},
		{`{"distance_km": 0, "pending_orders": 0}`, 10},
		{`{"distance_km": -0.0, "pending_orders": -0}`, 10},
		{`{"distance_km": 2, "pending_orders": 3.0}`, 22},
		{`{"distance_km": "2.5", "pending_orders": "3"}`, 23.5},
		{`{"distance_km": " 2.5 ", "pending_orders": " 3.0 "}`, 23.5},
		{`{"distance_km": true, "pending_orders": false}`, 13},
		{`{"distance_km": 1, "pending_orders": 1e2}`, 213},
		{`{"distance_km": 1, "pending_orders": 3, "extra": "무시"}`, 19},
	}
	h := newHandler(rulePredictor{})
	for _, c := range cases {
		r := call(t, h, http.MethodPost, "/eta", "application/json", c.body)
		if r.status != 200 || r.body["minutes"] != c.want || r.body["model"] != "onnx" {
			t.Errorf("%s → %d %v, want minutes=%v", c.body, r.status, r.body, c.want)
		}
	}
}

// TestEtaRejected 는 잘못된 입력이 FastAPI 와 같은 422 와 오류 type 으로 거절되는지 확인한다.
func TestEtaRejected(t *testing.T) {
	cases := []struct {
		contentType, body string
		types             []string
	}{
		{"application/json", `{"distance_km": -0.1, "pending_orders": 3}`, []string{"greater_than_equal"}},
		{"application/json", `{"distance_km": 2.5, "pending_orders": -1}`, []string{"greater_than_equal"}},
		{"application/json", `{"distance_km": "abc", "pending_orders": 3}`, []string{"float_parsing"}},
		{"application/json", `{"distance_km": "0x10", "pending_orders": 3}`, []string{"float_parsing"}},
		{"application/json", `{"distance_km": null, "pending_orders": 3}`, []string{"float_type"}},
		{"application/json", `{"distance_km": "inf", "pending_orders": 3}`, []string{"finite_number"}},
		{"application/json", `{"distance_km": 1e309, "pending_orders": 3}`, []string{"finite_number"}},
		{"application/json", `{"distance_km": 2.5, "pending_orders": 3.5}`, []string{"int_from_float"}},
		{"application/json", `{"distance_km": 2.5, "pending_orders": "x"}`, []string{"int_parsing"}},
		{"application/json", `{"distance_km": 2.5, "pending_orders": "3.5"}`, []string{"int_parsing"}},
		{"application/json", `{"distance_km": 2.5, "pending_orders": [1]}`, []string{"int_type"}},
		{"application/json", `{"distance_km": 2.5}`, []string{"missing"}},
		{"application/json", `{}`, []string{"missing", "missing"}},
		{"application/json", `{"distance_km": -1, "pending_orders": -1}`, []string{"greater_than_equal", "greater_than_equal"}},
		{"application/json", `[]`, []string{"model_attributes_type"}},
		{"application/json", `not json`, []string{"json_invalid"}},
		{"application/json", `{"distance_km": 1, "pending_orders": 1} {}`, []string{"json_invalid"}},
		{"application/json", ``, []string{"missing"}},
		{"text/plain", `{"distance_km": 2.5, "pending_orders": 3}`, []string{"model_attributes_type"}},
		{"", `{"distance_km": 2.5, "pending_orders": 3}`, []string{"model_attributes_type"}},
		// 결과가 유한하지 않을 만큼 큰 입력 (float32 범위 초과)
		{"application/json", `{"distance_km": 1e308, "pending_orders": 3}`, []string{"value_error"}},
		{"application/json", `{"distance_km": 2.5, "pending_orders": 99999999999999999999999999999999999999999}`, []string{"value_error"}},
	}
	h := newHandler(rulePredictor{})
	for _, c := range cases {
		r := call(t, h, http.MethodPost, "/eta", c.contentType, c.body)
		got := detailTypes(t, r.body)
		if r.status != 422 || strings.Join(got, ",") != strings.Join(c.types, ",") {
			t.Errorf("%q %s → %d %v, want 422 %v", c.contentType, c.body, r.status, got, c.types)
		}
	}
}

// TestEtaInfiniteResult 는 모델이 무한대를 내면 minutes=null 대신 422 로 거절하는지 확인한다.
func TestEtaInfiniteResult(t *testing.T) {
	r := call(t, newHandler(infPredictor{}), http.MethodPost, "/eta", "application/json", `{"distance_km": 1, "pending_orders": 1}`)
	if got := detailTypes(t, r.body); r.status != 422 || len(got) != 1 || got[0] != "value_error" {
		t.Fatalf("got %d %v", r.status, r.body)
	}
}

// TestValueErrorEchoesBigInt 는 아주 큰 pending_orders 를 잘리지 않은 정수로 돌려주는지 확인한다.
func TestValueErrorEchoesBigInt(t *testing.T) {
	const big = "99999999999999999999999999999999999999999"
	req := httptest.NewRequest(http.MethodPost, "/eta", strings.NewReader(`{"distance_km": 2.5, "pending_orders": `+big+`}`))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	newHandler(rulePredictor{}).ServeHTTP(rec, req)
	if !strings.Contains(rec.Body.String(), `"pending_orders":`+big) {
		t.Fatalf("응답에 원래 정수가 없다: %s", rec.Body.String())
	}
}

// TestRouting 은 없는 경로·메서드가 FastAPI 처럼 JSON 404/405 를 돌려주는지 확인한다.
func TestRouting(t *testing.T) {
	h := newHandler(rulePredictor{})
	if r := call(t, h, http.MethodGet, "/nope", "", ""); r.status != 404 || r.body["detail"] != "Not Found" {
		t.Errorf("404: got %d %v", r.status, r.body)
	}
	if r := call(t, h, http.MethodGet, "/eta", "", ""); r.status != 405 || r.body["detail"] != "Method Not Allowed" {
		t.Errorf("405: got %d %v", r.status, r.body)
	}
}
