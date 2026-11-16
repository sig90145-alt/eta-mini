package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"log"
	"math"
	"mime"
	"net/http"
	"regexp"
	"strconv"
	"strings"
)

// modelName 은 /health 와 /eta 응답의 "model" 값이다 (api/app.py 의 MODEL_IMPL 에 해당).
// 이 서비스는 ONNX 모델만 실행하므로 고정이다.
const modelName = "onnx"

// maxBodyBytes 는 요청 본문 크기 상한이다. 정상 요청은 100바이트도 안 된다.
const maxBodyBytes = 1 << 20

// Predictor 는 원래 단위의 입력(거리 km, 대기 주문 수)으로 예상 시간(분)을 계산한다.
// 테스트에서는 ONNX Runtime 없이 가짜 구현을 넣는다.
type Predictor interface {
	Predict(distanceKm, pendingOrders float32) (float32, error)
}

// fieldError 는 FastAPI(pydantic) 422 응답의 detail 항목과 같은 모양이다.
type fieldError struct {
	Type  string         `json:"type"`
	Loc   []any          `json:"loc"`
	Msg   string         `json:"msg"`
	Input any            `json:"input"` // null 이어도 항상 넣는다
	Ctx   map[string]any `json:"ctx,omitempty"`
}

type etaResponse struct {
	Minutes float64 `json:"minutes"`
	Model   string  `json:"model"`
}

// newHandler 는 api/app.py 와 같은 계약의 라우터를 만든다.
//
//	GET  /health → {"status": "ok", "model": "onnx"}
//	POST /eta    → {"minutes": <float>, "model": "onnx"} 또는 422
func newHandler(p Predictor) http.Handler {
	routes := map[string]map[string]http.HandlerFunc{
		"/health": {http.MethodGet: handleHealth},
		"/eta":    {http.MethodPost: func(w http.ResponseWriter, r *http.Request) { handleEta(w, r, p) }},
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		methods, ok := routes[r.URL.Path]
		if !ok {
			writeJSON(w, http.StatusNotFound, map[string]string{"detail": "Not Found"})
			return
		}
		h, ok := methods[r.Method]
		if !ok {
			for m := range methods {
				w.Header().Set("Allow", m)
			}
			writeJSON(w, http.StatusMethodNotAllowed, map[string]string{"detail": "Method Not Allowed"})
			return
		}
		h(w, r)
	})
}

func handleHealth(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, struct {
		Status string `json:"status"`
		Model  string `json:"model"`
	}{"ok", modelName})
}

func handleEta(w http.ResponseWriter, r *http.Request, p Predictor) {
	distanceKm, pending, errs := parseEtaRequest(r)
	if len(errs) > 0 {
		writeJSON(w, http.StatusUnprocessableEntity, map[string]any{"detail": errs})
		return
	}

	// 모델 입력은 float32 다. float32 로 표현할 수 없을 만큼 크면 모델을 부르지 않고 거절한다.
	d32, o32 := float32(distanceKm), float32(pending.value)
	minutes := math.Inf(1)
	if !isInf32(d32) && !isInf32(o32) {
		got, err := p.Predict(d32, o32)
		if err != nil {
			log.Printf("predict failed: %v", err)
			writeJSON(w, http.StatusInternalServerError, map[string]string{"detail": "Internal Server Error"})
			return
		}
		minutes = float64(got)
	}
	if math.IsInf(minutes, 0) || math.IsNaN(minutes) {
		// 입력이 너무 커서 결과를 숫자로 표현할 수 없다. minutes=null 로 응답하지 않고 거절한다.
		writeJSON(w, http.StatusUnprocessableEntity, map[string]any{"detail": []fieldError{{
			Type:  "value_error",
			Loc:   []any{"body"},
			Msg:   "Input values are too large to estimate delivery time",
			Input: map[string]any{"distance_km": distanceKm, "pending_orders": json.Number(pending.text)},
		}}})
		return
	}
	writeJSON(w, http.StatusOK, etaResponse{Minutes: minutes, Model: modelName})
}

// intValue 는 검증을 통과한 pending_orders 다. 파이썬 int 처럼 크기 제한이 없으므로
// 계산용 float64 와 응답에 그대로 돌려줄 10진수 문자열을 함께 들고 있다.
type intValue struct {
	value float64
	text  string
}

// parseEtaRequest 는 FastAPI + pydantic(lax 모드)의 EtaRequest 검증 규칙을 따른다.
//   - distance_km: 숫자, 숫자 문자열, bool. 유한하고 0 이상
//   - pending_orders: 정수, 소수부가 0 인 실수(3.0), 정수 문자열, bool. 0 이상
//   - 모르는 필드는 무시한다
func parseEtaRequest(r *http.Request) (float64, intValue, []fieldError) {
	raw, err := io.ReadAll(io.LimitReader(r.Body, maxBodyBytes+1))
	if err != nil || len(raw) > maxBodyBytes {
		return 0, intValue{}, []fieldError{{Type: "value_error", Loc: []any{"body"}, Msg: "Request body is too large or unreadable", Input: nil}}
	}
	if len(bytes.TrimSpace(raw)) == 0 {
		return 0, intValue{}, []fieldError{{Type: "missing", Loc: []any{"body"}, Msg: "Field required", Input: nil}}
	}
	if !isJSONContentType(r.Header.Get("Content-Type")) {
		// FastAPI 는 JSON 이 아닌 본문을 문자열로 보고 객체가 아니라며 거절한다.
		return 0, intValue{}, []fieldError{modelAttributesError(string(raw))}
	}

	body, err := decodeJSON(raw)
	if err != nil {
		return 0, intValue{}, []fieldError{jsonInvalidError(err)}
	}
	obj, ok := body.(map[string]any)
	if !ok {
		return 0, intValue{}, []fieldError{modelAttributesError(body)}
	}

	var errs []fieldError
	distanceKm, ferr := parseDistance(obj)
	if ferr != nil {
		errs = append(errs, *ferr)
	}
	pending, ferr := parsePendingOrders(obj)
	if ferr != nil {
		errs = append(errs, *ferr)
	}
	return distanceKm, pending, errs
}

func isJSONContentType(v string) bool {
	if v == "" {
		return false
	}
	mt, _, err := mime.ParseMediaType(v)
	if err != nil {
		return false
	}
	return mt == "application/json" || (strings.HasPrefix(mt, "application/") && strings.HasSuffix(mt, "+json"))
}

// decodeJSON 은 숫자를 json.Number 로 남겨 둔 채 본문 전체를 하나의 JSON 값으로 읽는다.
func decodeJSON(raw []byte) (any, error) {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var v any
	if err := dec.Decode(&v); err != nil {
		return nil, err
	}
	if _, err := dec.Token(); err != io.EOF {
		return nil, errors.New("Extra data")
	}
	return v, nil
}

func modelAttributesError(input any) fieldError {
	return fieldError{
		Type:  "model_attributes_type",
		Loc:   []any{"body"},
		Msg:   "Input should be a valid dictionary or object to extract fields from",
		Input: input,
	}
}

func jsonInvalidError(err error) fieldError {
	pos := int64(0)
	var se *json.SyntaxError
	if errors.As(err, &se) {
		pos = se.Offset
	}
	return fieldError{
		Type:  "json_invalid",
		Loc:   []any{"body", pos},
		Msg:   "JSON decode error",
		Input: map[string]any{},
		Ctx:   map[string]any{"error": err.Error()},
	}
}

func missingError(field string, obj map[string]any) *fieldError {
	return &fieldError{Type: "missing", Loc: []any{"body", field}, Msg: "Field required", Input: obj}
}

func geZeroError(field string, input any, ge json.Number) *fieldError {
	return &fieldError{
		Type:  "greater_than_equal",
		Loc:   []any{"body", field},
		Msg:   "Input should be greater than or equal to 0",
		Input: input,
		Ctx:   map[string]any{"ge": ge},
	}
}

// 파이썬 float() 가 받는 10진수 표기 (16진수·밑줄은 받지 않는다).
var (
	decimalRe = regexp.MustCompile(`^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$`)
	integerRe = regexp.MustCompile(`^[+-]?\d+$`)
	nonFinite = regexp.MustCompile(`(?i)^[+-]?(inf|infinity|nan)$`)
)

func parseDistance(obj map[string]any) (float64, *fieldError) {
	const field = "distance_km"
	raw, ok := obj[field]
	if !ok {
		return 0, missingError(field, obj)
	}
	loc := []any{"body", field}

	var v float64
	switch x := raw.(type) {
	case json.Number:
		v, _ = strconv.ParseFloat(string(x), 64) // 범위를 넘으면 ±Inf 가 되고 아래에서 거절된다
	case string:
		s := strings.TrimSpace(x)
		switch {
		case decimalRe.MatchString(s):
			v, _ = strconv.ParseFloat(s, 64)
		case nonFinite.MatchString(s):
			v = math.Inf(1)
		default:
			return 0, &fieldError{Type: "float_parsing", Loc: loc, Msg: "Input should be a valid number, unable to parse string as a number", Input: x}
		}
	case bool:
		v = boolToFloat(x)
	default:
		return 0, &fieldError{Type: "float_type", Loc: loc, Msg: "Input should be a valid number", Input: raw}
	}

	if math.IsInf(v, 0) || math.IsNaN(v) {
		return 0, &fieldError{Type: "finite_number", Loc: loc, Msg: "Input should be a finite number", Input: nonFiniteInput(raw, v)}
	}
	if v < 0 {
		return 0, geZeroError(field, raw, "0.0")
	}
	return v, nil
}

func parsePendingOrders(obj map[string]any) (intValue, *fieldError) {
	const field = "pending_orders"
	raw, ok := obj[field]
	if !ok {
		return intValue{}, missingError(field, obj)
	}
	loc := []any{"body", field}
	notInt := &fieldError{Type: "int_parsing", Loc: loc, Msg: "Input should be a valid integer, unable to parse string as an integer", Input: raw}
	fractional := &fieldError{Type: "int_from_float", Loc: loc, Msg: "Input should be a valid integer, got a number with a fractional part", Input: raw}
	infinite := &fieldError{Type: "finite_number", Loc: loc, Msg: "Input should be a finite number", Input: raw}

	var iv intValue
	switch x := raw.(type) {
	case json.Number:
		s := string(x)
		if integerRe.MatchString(s) {
			iv = intFromDigits(s)
		} else {
			var ferr *fieldError
			if iv, ferr = intFromDecimal(s, fractional, infinite); ferr != nil {
				return intValue{}, ferr
			}
		}
	case string:
		s := strings.TrimSpace(x)
		switch {
		case integerRe.MatchString(s):
			iv = intFromDigits(s)
		case decimalRe.MatchString(s):
			// 문자열은 "3.0" 만 받고, "3.5"·"1e400" 은 pydantic 처럼 int_parsing 으로 거절한다
			var ferr *fieldError
			if iv, ferr = intFromDecimal(s, notInt, notInt); ferr != nil {
				return intValue{}, ferr
			}
		default:
			return intValue{}, notInt
		}
	case bool:
		iv = intValue{value: boolToFloat(x), text: strconv.Itoa(int(boolToFloat(x)))}
	default:
		return intValue{}, &fieldError{Type: "int_type", Loc: loc, Msg: "Input should be a valid integer", Input: raw}
	}

	if strings.HasPrefix(iv.text, "-") {
		return intValue{}, geZeroError(field, raw, "0")
	}
	return iv, nil
}

// intFromDigits 는 정수 표기("+007", "-0", "123...")를 정규화한다. 아주 큰 수는 value 가 +Inf 가 된다.
func intFromDigits(s string) intValue {
	neg := strings.HasPrefix(s, "-")
	digits := strings.TrimLeft(strings.TrimLeft(s, "+-"), "0")
	if digits == "" {
		return intValue{value: 0, text: "0"}
	}
	if neg {
		digits = "-" + digits
	}
	v, _ := strconv.ParseFloat(digits, 64)
	return intValue{value: v, text: digits}
}

// intFromDecimal 은 "3.0", "1e2" 처럼 소수점·지수가 있는 표기를 소수부가 0 일 때만 정수로 받는다.
func intFromDecimal(s string, fractional, infinite *fieldError) (intValue, *fieldError) {
	v, _ := strconv.ParseFloat(s, 64)
	if math.IsInf(v, 0) {
		return intValue{}, infinite
	}
	if v != math.Trunc(v) {
		return intValue{}, fractional
	}
	if v == 0 {
		v = 0 // -0.0 → 0
	}
	return intValue{value: v, text: strconv.FormatFloat(v, 'f', -1, 64)}, nil
}

func boolToFloat(b bool) float64 {
	if b {
		return 1
	}
	return 0
}

// nonFiniteInput 은 api/app.py 의 _json_safe 처럼 NaN·Infinity 를 "nan"/"inf"/"-inf" 문자열로 돌려준다.
func nonFiniteInput(raw any, v float64) any {
	if s, ok := raw.(string); ok {
		return s
	}
	switch {
	case math.IsNaN(v):
		return "nan"
	case v > 0:
		return "inf"
	default:
		return "-inf"
	}
}

func isInf32(f float32) bool { return math.IsInf(float64(f), 0) }

func writeJSON(w http.ResponseWriter, status int, v any) {
	body, err := json.Marshal(v)
	if err != nil {
		log.Printf("encode response failed: %v", err)
		http.Error(w, `{"detail":"Internal Server Error"}`, http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_, _ = w.Write(body)
}
