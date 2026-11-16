//! api/app.py (FastAPI + pydantic) 와 같은 계약의 라우터와 요청 검증.
//!
//!   GET  /health → {"status": "ok", "model": "onnx"}
//!   POST /eta    → {"minutes": <float>, "model": "onnx"} 또는 422
//!
//! axum 의 Json 추출기는 형식 오류에 400·415 를 돌려주므로 본문을 직접 읽어 검증한다.
//! 모든 입력 오류는 FastAPI 처럼 422 와 {"detail": [{type, loc, msg, input, ctx?}, ...]} 로 응답한다.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::json::{self, Json, float_value, int_value};
use crate::predictor::Predictor;

/// /health 와 /eta 응답의 "model" 값 (api/app.py 의 MODEL_IMPL 에 해당). 이 서비스는 ONNX 모델만 실행한다.
pub const MODEL_NAME: &str = "onnx";

/// 요청 본문 크기 상한. 정상 요청은 100바이트도 안 된다.
const MAX_BODY_BYTES: usize = 1 << 20;

/// pydantic 이 문자열을 정수로 읽을 때의 길이 상한 (넘으면 int_parsing_size).
const MAX_INT_STR_LEN: usize = 4300;

/// pydantic 이 float 를 정수로 바꿀 수 있는 범위 (|x| < 2^63, 넘으면 int_parsing_size).
const MAX_INT_FROM_FLOAT: f64 = 9_223_372_036_854_775_808.0;

type SharedPredictor = Arc<dyn Predictor>;

pub fn router(predictor: SharedPredictor) -> Router {
    Router::new()
        // axum 은 GET 경로에 HEAD 를 자동으로 붙이지만 FastAPI 는 405 를 돌려주므로 따로 막는다.
        .route("/health", get(health).head(allow_get).fallback(allow_get))
        .route("/eta", post(eta).fallback(allow_post))
        .fallback(not_found)
        .with_state(predictor)
}

async fn health() -> Response {
    json_response(StatusCode::OK, json!({"status": "ok", "model": MODEL_NAME}))
}

async fn eta(State(predictor): State<SharedPredictor>, headers: HeaderMap, body: Body) -> Response {
    let Ok(raw) = to_bytes(body, MAX_BODY_BYTES).await else {
        return unprocessable(vec![field_error(
            "value_error",
            json!(["body"]),
            "Request body is too large or unreadable",
            Value::Null,
            None,
        )]);
    };
    let req = match parse_eta_request(headers.get(header::CONTENT_TYPE), &raw) {
        Ok(req) => req,
        Err(errors) => return unprocessable(errors),
    };

    // 모델 입력은 float32 다. float32 로 표현할 수 없을 만큼 크면 모델을 부르지 않고 거절한다.
    let distance = req.distance_km as f32;
    let pending = req.pending_orders.parse::<f32>().unwrap_or(f32::INFINITY);
    let minutes = if distance.is_finite() && pending.is_finite() {
        match predictor.predict(distance, pending) {
            Ok(minutes) => minutes,
            Err(e) => {
                eprintln!("predict failed: {e}");
                return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"detail": "Internal Server Error"}));
            }
        }
    } else {
        f32::INFINITY
    };
    if !minutes.is_finite() {
        // 입력이 너무 커서 결과를 숫자로 표현할 수 없다. minutes=null 로 응답하지 않고 거절한다.
        return unprocessable(vec![field_error(
            "value_error",
            json!(["body"]),
            "Input values are too large to estimate delivery time",
            json!({
                "distance_km": float_value(req.distance_km),
                "pending_orders": int_value(&req.pending_orders),
            }),
            None,
        )]);
    }
    json_response(StatusCode::OK, json!({"minutes": f64::from(minutes), "model": MODEL_NAME}))
}

async fn allow_get() -> Response {
    method_not_allowed("GET")
}

async fn allow_post() -> Response {
    method_not_allowed("POST")
}

fn method_not_allowed(allow: &'static str) -> Response {
    let mut res = json_response(StatusCode::METHOD_NOT_ALLOWED, json!({"detail": "Method Not Allowed"}));
    res.headers_mut().insert(header::ALLOW, HeaderValue::from_static(allow));
    res
}

async fn not_found() -> Response {
    json_response(StatusCode::NOT_FOUND, json!({"detail": "Not Found"}))
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

fn unprocessable(errors: Vec<Value>) -> Response {
    json_response(StatusCode::UNPROCESSABLE_ENTITY, json!({"detail": errors}))
}

/// 검증을 통과한 요청.
#[derive(Debug, PartialEq)]
struct EtaRequest {
    distance_km: f64,
    /// 0 이상의 10진수 정수. 파이썬 int 처럼 크기 제한이 없어 문자열로 들고 있다.
    pending_orders: String,
}

/// FastAPI + pydantic(lax 모드)의 EtaRequest 검증 규칙을 따른다.
///   - 본문이 비면 missing, JSON Content-Type 이 아니면 model_attributes_type, 형식이 깨지면 json_invalid
///   - distance_km: 숫자, 숫자 문자열, bool. 유한하고 0 이상
///   - pending_orders: 정수, 소수부가 0 인 실수, 정수 문자열, bool. 0 이상
///   - 모르는 필드는 무시한다. 두 필드의 오류를 모두 모아 돌려준다
fn parse_eta_request(content_type: Option<&HeaderValue>, raw: &[u8]) -> Result<EtaRequest, Vec<Value>> {
    if raw.is_empty() {
        return Err(vec![field_error("missing", json!(["body"]), "Field required", Value::Null, None)]);
    }
    if !is_json_content_type(content_type) {
        // FastAPI 는 JSON 이 아닌 본문을 문자열로 보고 객체가 아니라며 거절한다.
        return Err(vec![model_attributes_error(Value::String(String::from_utf8_lossy(raw).into_owned()))]);
    }
    let body = json::parse_bytes(raw).map_err(|e| {
        vec![field_error(
            "json_invalid",
            json!(["body", e.pos]),
            "JSON decode error",
            json!({}),
            Some(json!({"error": e.msg})),
        )]
    })?;
    let Json::Object(members) = &body else {
        return Err(vec![model_attributes_error(body.to_value())]);
    };
    let field = |name: &str| members.iter().find(|(k, _)| k == name).map(|(_, v)| v);

    let distance = parse_distance(field("distance_km"), &body);
    let pending = parse_pending_orders(field("pending_orders"), &body);
    match (distance, pending) {
        (Ok(distance_km), Ok(pending_orders)) => Ok(EtaRequest { distance_km, pending_orders }),
        (distance, pending) => Err(distance.err().into_iter().chain(pending.err()).collect()),
    }
}

/// Content-Type 이 application/json 또는 application/*+json 인지 본다 (FastAPI 와 같은 판단).
/// 파이썬 email 모듈처럼 `;` 앞부분만 보고, 대소문자를 가리지 않는다.
fn is_json_content_type(value: Option<&HeaderValue>) -> bool {
    let Some(value) = value else { return false };
    let value = String::from_utf8_lossy(value.as_bytes());
    let media_type = value.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match media_type.split_once('/') {
        Some((main, sub)) if !sub.contains('/') => main == "application" && (sub == "json" || sub.ends_with("+json")),
        _ => false,
    }
}

fn parse_distance(raw: Option<&Json>, body: &Json) -> Result<f64, Value> {
    const FIELD: &str = "distance_km";
    let Some(raw) = raw else { return Err(missing_error(FIELD, body)) };
    let loc = json!(["body", FIELD]);
    let float_type = || field_error("float_type", loc.clone(), "Input should be a valid number", raw.to_value(), None);

    let value = match raw {
        Json::Float(f) => *f,
        // 파이썬 int 를 float 로 바꿀 수 없을 만큼 크면(약 1.8e308 이상) float_type 이다.
        Json::Int(text) => text.parse::<f64>().ok().filter(|f| f.is_finite()).ok_or_else(float_type)?,
        Json::Bool(b) => f64::from(u8::from(*b)),
        Json::Str(s) => parse_float_str(s).ok_or_else(|| {
            field_error(
                "float_parsing",
                loc.clone(),
                "Input should be a valid number, unable to parse string as a number",
                raw.to_value(),
                None,
            )
        })?,
        Json::Null | Json::Array(_) | Json::Object(_) => return Err(float_type()),
    };
    if !value.is_finite() {
        // raw.to_value() 는 NaN·Infinity 를 "nan"/"inf"/"-inf" 로, 문자열은 원래 문자열로 돌려준다.
        return Err(field_error("finite_number", loc, "Input should be a finite number", raw.to_value(), None));
    }
    if value < 0.0 {
        return Err(ge_zero_error(loc, raw, json!({"ge": 0.0})));
    }
    Ok(value)
}

/// pydantic 의 문자열 → float 규칙: 앞뒤 공백을 빼고 Rust 의 f64 문법으로 읽는다 ("inf", "nan" 포함).
/// 읽지 못하면 밑줄을 지우고 한 번 더 읽는다. 단 맨 앞·맨 뒤 밑줄과 연속된 밑줄은 받지 않는다.
fn parse_float_str(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Ok(f) = s.parse() {
        return Some(f);
    }
    if !s.contains('_') || s.starts_with('_') || s.ends_with('_') || s.contains("__") {
        return None;
    }
    s.replace('_', "").parse().ok()
}

fn parse_pending_orders(raw: Option<&Json>, body: &Json) -> Result<String, Value> {
    const FIELD: &str = "pending_orders";
    let Some(raw) = raw else { return Err(missing_error(FIELD, body)) };
    let loc = json!(["body", FIELD]);
    let fail = |kind: &'static str, msg: &'static str| field_error(kind, loc.clone(), msg, raw.to_value(), None);
    let int_parsing = || fail("int_parsing", "Input should be a valid integer, unable to parse string as an integer");
    let int_parsing_size = || fail("int_parsing_size", "Unable to parse input string as an integer, exceeded maximum size");

    let text = match raw {
        Json::Int(text) => text.clone(),
        Json::Float(f) => {
            if !f.is_finite() {
                return Err(fail("finite_number", "Input should be a finite number"));
            }
            if f.fract() != 0.0 {
                return Err(fail("int_from_float", "Input should be a valid integer, got a number with a fractional part"));
            }
            if f.abs() >= MAX_INT_FROM_FLOAT {
                return Err(int_parsing_size());
            }
            (*f as i64).to_string() // -0.0 → "0"
        }
        Json::Bool(b) => u8::from(*b).to_string(),
        Json::Str(s) => {
            let s = s.trim();
            if s.len() > MAX_INT_STR_LEN {
                return Err(int_parsing_size());
            }
            parse_int_str(s).ok_or_else(int_parsing)?
        }
        Json::Null | Json::Array(_) | Json::Object(_) => {
            return Err(fail("int_type", "Input should be a valid integer"));
        }
    };
    if text.starts_with('-') {
        return Err(ge_zero_error(loc, raw, json!({"ge": 0})));
    }
    Ok(text)
}

/// pydantic 의 문자열 → int 규칙. "+7", "007", "1_000" 처럼 숫자 사이 밑줄을 받고,
/// "3.0", "3.00" 처럼 소수부가 모두 0 이면 정수로 본다 ("3.", "1e2", "3.5" 는 받지 않는다).
fn parse_int_str(s: &str) -> Option<String> {
    canonical_int(s).or_else(|| {
        let (int_part, frac) = s.split_once('.')?;
        if frac.is_empty() || !frac.bytes().all(|b| b == b'0') {
            return None;
        }
        canonical_int(int_part)
    })
}

/// `[+-]?\d+(_\d+)*` (ASCII 숫자) 를 부호·밑줄·앞자리 0 을 정리한 10진수로 바꾼다. "-0" 은 "0" 이다.
fn canonical_int(s: &str) -> Option<String> {
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let well_formed = !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'_');
    if !well_formed {
        return None;
    }
    let digits: String = digits.chars().filter(|&c| c != '_').collect();
    let digits = digits.trim_start_matches('0');
    Some(match (digits.is_empty(), negative) {
        (true, _) => "0".to_owned(),
        (false, true) => format!("-{digits}"),
        (false, false) => digits.to_owned(),
    })
}

/// FastAPI(pydantic) 422 응답의 detail 항목. ctx 가 없으면 키를 넣지 않는다.
fn field_error(kind: &str, loc: Value, msg: &str, input: Value, ctx: Option<Value>) -> Value {
    let mut error = json!({"type": kind, "loc": loc, "msg": msg, "input": input});
    if let Some(ctx) = ctx {
        error["ctx"] = ctx;
    }
    error
}

fn missing_error(field: &str, body: &Json) -> Value {
    field_error("missing", json!(["body", field]), "Field required", body.to_value(), None)
}

fn model_attributes_error(input: Value) -> Value {
    field_error(
        "model_attributes_type",
        json!(["body"]),
        "Input should be a valid dictionary or object to extract fields from",
        input,
        None,
    )
}

fn ge_zero_error(loc: Value, raw: &Json, ctx: Value) -> Value {
    field_error("greater_than_equal", loc, "Input should be greater than or equal to 0", raw.to_value(), Some(ctx))
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    use super::*;

    /// ONNX Runtime 없이 계약만 확인하려고 쓰는 가짜 모델 (api/predictors/rule.py 와 같은 식).
    struct RulePredictor;
    impl Predictor for RulePredictor {
        fn predict(&self, d: f32, o: f32) -> Result<f32, String> {
            Ok(10.0 + 3.0 * d + 2.0 * o)
        }
    }

    struct InfPredictor;
    impl Predictor for InfPredictor {
        fn predict(&self, _: f32, _: f32) -> Result<f32, String> {
            Ok(f32::INFINITY)
        }
    }

    struct Reply {
        status: StatusCode,
        allow: Option<String>,
        body: Value,
        text: String,
    }

    async fn call_with(p: SharedPredictor, method: Method, path: &str, content_type: Option<&str>, body: &str) -> Reply {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(ct) = content_type {
            req = req.header(header::CONTENT_TYPE, ct);
        }
        let res = router(p).oneshot(req.body(Body::from(body.to_owned())).unwrap()).await.unwrap();
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        let status = res.status();
        let allow = res.headers().get(header::ALLOW).map(|v| v.to_str().unwrap().to_owned());
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        // HEAD 응답은 본문이 없다 (Content-Type 은 GET 과 같게 남는다).
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("응답이 JSON 이 아니다: {e} ({text})"))
        };
        Reply { status, allow, body, text }
    }

    async fn call(method: Method, path: &str, content_type: Option<&str>, body: &str) -> Reply {
        call_with(Arc::new(RulePredictor), method, path, content_type, body).await
    }

    async fn post_eta(body: &str) -> Reply {
        call(Method::POST, "/eta", Some("application/json"), body).await
    }

    fn detail_types(r: &Reply) -> Vec<String> {
        r.body["detail"]
            .as_array()
            .unwrap_or_else(|| panic!("detail 이 배열이 아니다: {}", r.text))
            .iter()
            .map(|e| e["type"].as_str().unwrap().to_owned())
            .collect()
    }

    /// GET /health 가 상태와 모델 이름을 돌려주는지 확인한다.
    #[tokio::test]
    async fn health_reports_model() {
        let r = call(Method::GET, "/health", None, "").await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body, json!({"status": "ok", "model": "onnx"}));
    }

    /// pydantic lax 모드가 받아 주는 입력을 같은 값으로 계산하는지 확인한다.
    #[tokio::test]
    async fn eta_accepts_lax_inputs() {
        let cases: &[(&str, f64)] = &[
            (r#"{"distance_km": 2.5, "pending_orders": 3}"#, 23.5),
            (r#"{"distance_km": 0, "pending_orders": 0}"#, 10.0),
            (r#"{"distance_km": -0.0, "pending_orders": -0}"#, 10.0),
            (r#"{"distance_km": 2, "pending_orders": 3.0}"#, 22.0),
            (r#"{"distance_km": 2, "pending_orders": -0.0}"#, 16.0),
            (r#"{"distance_km": "2.5", "pending_orders": "3"}"#, 23.5),
            (r#"{"distance_km": " 2.5 ", "pending_orders": " 3.0 "}"#, 23.5),
            (r#"{"distance_km": "1.", "pending_orders": "3.00"}"#, 19.0),
            (r#"{"distance_km": ".5", "pending_orders": "+3"}"#, 17.5),
            (r#"{"distance_km": "1_0", "pending_orders": "1_0"}"#, 60.0),
            (r#"{"distance_km": "+_1", "pending_orders": "007"}"#, 27.0),
            (r#"{"distance_km": "1e1_0", "pending_orders": 0}"#, 3e10 + 10.0),
            (r#"{"distance_km": true, "pending_orders": false}"#, 13.0),
            (r#"{"distance_km": 1, "pending_orders": 1e2}"#, 213.0),
            (r#"{"distance_km": 1, "pending_orders": 3, "extra": "무시"}"#, 19.0),
            (r#"{"distance_km": 1, "pending_orders": 3, "distance_km": 2}"#, 22.0),
            ("\u{feff}{\"distance_km\": 1, \"pending_orders\": 1}", 15.0),
        ];
        for &(body, want) in cases {
            let r = post_eta(body).await;
            assert_eq!(r.status, StatusCode::OK, "{body} → {}", r.text);
            assert_eq!(r.body["model"], "onnx");
            let got = r.body["minutes"].as_f64().unwrap();
            assert!((got - want).abs() <= want.abs() * 1e-6, "{body} → {got}, want {want}");
        }
    }

    /// 잘못된 입력이 FastAPI 와 같은 422 와 오류 type 으로 거절되는지 확인한다.
    #[tokio::test]
    async fn eta_rejects_invalid_inputs() {
        const JSON: Option<&str> = Some("application/json");
        let big = "9".repeat(41);
        let huge_int = format!("1{}", "0".repeat(400));
        let long_str = format!(r#"{{"distance_km": 1, "pending_orders": "1{}"}}"#, "0".repeat(4300));
        let cases: Vec<(Option<&str>, String, &[&str])> = vec![
            (JSON, r#"{"distance_km": -0.1, "pending_orders": 3}"#.into(), &["greater_than_equal"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": -1}"#.into(), &["greater_than_equal"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": "-1_0"}"#.into(), &["greater_than_equal"]),
            (JSON, r#"{"distance_km": "abc", "pending_orders": 3}"#.into(), &["float_parsing"]),
            (JSON, r#"{"distance_km": "0x10", "pending_orders": 3}"#.into(), &["float_parsing"]),
            (JSON, r#"{"distance_km": "1__0", "pending_orders": "1__0"}"#.into(), &["float_parsing", "int_parsing"]),
            (JSON, r#"{"distance_km": "_1", "pending_orders": "+_1"}"#.into(), &["float_parsing", "int_parsing"]),
            (JSON, r#"{"distance_km": "", "pending_orders": ""}"#.into(), &["float_parsing", "int_parsing"]),
            (JSON, r#"{"distance_km": null, "pending_orders": null}"#.into(), &["float_type", "int_type"]),
            (JSON, r#"{"distance_km": [1], "pending_orders": {"a": 1}}"#.into(), &["float_type", "int_type"]),
            (JSON, r#"{"distance_km": "inf", "pending_orders": "inf"}"#.into(), &["finite_number", "int_parsing"]),
            (JSON, r#"{"distance_km": NaN, "pending_orders": 0}"#.into(), &["finite_number"]),
            (JSON, r#"{"distance_km": Infinity, "pending_orders": NaN}"#.into(), &["finite_number", "finite_number"]),
            (JSON, r#"{"distance_km": -Infinity, "pending_orders": 0}"#.into(), &["finite_number"]),
            (JSON, r#"{"distance_km": 1e309, "pending_orders": 3}"#.into(), &["finite_number"]),
            (JSON, format!(r#"{{"distance_km": {huge_int}, "pending_orders": 0}}"#), &["float_type"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": 3.5}"#.into(), &["int_from_float"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": 1e19}"#.into(), &["int_parsing_size"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": -1e19}"#.into(), &["int_parsing_size"]),
            (JSON, long_str, &["int_parsing_size"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": "x"}"#.into(), &["int_parsing"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": "3.5"}"#.into(), &["int_parsing"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": "3."}"#.into(), &["int_parsing"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": "1e2"}"#.into(), &["int_parsing"]),
            (JSON, r#"{"distance_km": 2.5, "pending_orders": [1]}"#.into(), &["int_type"]),
            (JSON, r#"{"distance_km": 2.5}"#.into(), &["missing"]),
            (JSON, "{}".into(), &["missing", "missing"]),
            (JSON, r#"{"distance_km": -1, "pending_orders": "many"}"#.into(), &["greater_than_equal", "int_parsing"]),
            (JSON, "[]".into(), &["model_attributes_type"]),
            (JSON, "null".into(), &["model_attributes_type"]),
            (JSON, "not json".into(), &["json_invalid"]),
            (JSON, "   ".into(), &["json_invalid"]),
            (JSON, r#"{"distance_km": 1, "pending_orders": 1} {}"#.into(), &["json_invalid"]),
            (JSON, r#"{"distance_km": 1, "pending_orders": 1,}"#.into(), &["json_invalid"]),
            (JSON, "".into(), &["missing"]),
            (Some("text/plain"), r#"{"distance_km": 2.5, "pending_orders": 3}"#.into(), &["model_attributes_type"]),
            (Some(""), r#"{"distance_km": 2.5, "pending_orders": 3}"#.into(), &["model_attributes_type"]),
            (Some("json"), r#"{"distance_km": 2.5, "pending_orders": 3}"#.into(), &["model_attributes_type"]),
            (None, r#"{"distance_km": 2.5, "pending_orders": 3}"#.into(), &["model_attributes_type"]),
            // 결과가 유한하지 않을 만큼 큰 입력 (float32 범위 초과)
            (JSON, r#"{"distance_km": 1e307, "pending_orders": 0}"#.into(), &["value_error"]),
            (JSON, format!(r#"{{"distance_km": 2.5, "pending_orders": {big}}}"#), &["value_error"]),
        ];
        for (content_type, body, want) in cases {
            let r = call(Method::POST, "/eta", content_type, &body).await;
            assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{content_type:?} {body} → {}", r.text);
            assert_eq!(detail_types(&r), want, "{content_type:?} {body} → {}", r.text);
        }
    }

    /// 422 detail 의 loc·msg·input·ctx 가 FastAPI 응답과 똑같은지 확인한다 (Python API 에서 받은 값).
    #[tokio::test]
    async fn eta_error_detail_matches_fastapi() {
        let cases: &[(Option<&str>, &str, Value)] = &[
            (
                Some("application/json"),
                r#"{"distance_km": "-1", "pending_orders": -1.0}"#,
                json!([
                    {"type": "greater_than_equal", "loc": ["body", "distance_km"], "msg": "Input should be greater than or equal to 0", "input": "-1", "ctx": {"ge": 0.0}},
                    {"type": "greater_than_equal", "loc": ["body", "pending_orders"], "msg": "Input should be greater than or equal to 0", "input": -1.0, "ctx": {"ge": 0}},
                ]),
            ),
            (
                Some("application/json"),
                r#"{"distance_km": -Infinity, "pending_orders": Infinity}"#,
                json!([
                    {"type": "finite_number", "loc": ["body", "distance_km"], "msg": "Input should be a finite number", "input": "-inf"},
                    {"type": "finite_number", "loc": ["body", "pending_orders"], "msg": "Input should be a finite number", "input": "inf"},
                ]),
            ),
            (
                Some("application/json"),
                r#"{"distance_km": 2.5}"#,
                json!([{"type": "missing", "loc": ["body", "pending_orders"], "msg": "Field required", "input": {"distance_km": 2.5}}]),
            ),
            (
                Some("application/json"),
                r#"{"distance_km": 1 "pending_orders": 1}"#,
                json!([{"type": "json_invalid", "loc": ["body", 18], "msg": "JSON decode error", "input": {}, "ctx": {"error": "Expecting ',' delimiter"}}]),
            ),
            (
                Some("application/json"),
                "",
                json!([{"type": "missing", "loc": ["body"], "msg": "Field required", "input": null}]),
            ),
            (
                Some("text/plain"),
                "abc",
                json!([{"type": "model_attributes_type", "loc": ["body"], "msg": "Input should be a valid dictionary or object to extract fields from", "input": "abc"}]),
            ),
            (
                Some("application/json"),
                r#"{"distance_km": 0, "pending_orders": 1e39}"#,
                json!([{"type": "int_parsing_size", "loc": ["body", "pending_orders"], "msg": "Unable to parse input string as an integer, exceeded maximum size", "input": 1e39}]),
            ),
            (
                Some("application/json"),
                r#"{"distance_km": 1e308, "pending_orders": "0_3"}"#,
                json!([{"type": "value_error", "loc": ["body"], "msg": "Input values are too large to estimate delivery time", "input": {"distance_km": 1e308, "pending_orders": 3}}]),
            ),
        ];
        for (content_type, body, want) in cases {
            let r = call(Method::POST, "/eta", *content_type, body).await;
            assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert_eq!(&r.body["detail"], want, "{body} → {}", r.text);
        }
    }

    /// 모델이 무한대를 내면 minutes=null 대신 422 로 거절하는지 확인한다.
    #[tokio::test]
    async fn eta_rejects_infinite_result() {
        let r = call_with(Arc::new(InfPredictor), Method::POST, "/eta", Some("application/json"), r#"{"distance_km": 1, "pending_orders": 1}"#).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(detail_types(&r), ["value_error"]);
    }

    /// 아주 큰 pending_orders 를 잘리지 않은 정수로 돌려주는지 확인한다.
    #[tokio::test]
    async fn value_error_echoes_big_int() {
        let big = "9".repeat(41);
        let r = post_eta(&format!(r#"{{"distance_km": 2.5, "pending_orders": "0_{big}"}}"#)).await;
        assert!(r.text.contains(&format!(r#""pending_orders":{big}}}"#)), "{}", r.text);
    }

    /// 없는 경로·메서드가 FastAPI 처럼 JSON 404/405 와 Allow 헤더를 돌려주는지 확인한다.
    #[tokio::test]
    async fn routing_matches_fastapi() {
        let r = call(Method::GET, "/nope", None, "").await;
        assert_eq!((r.status, r.body), (StatusCode::NOT_FOUND, json!({"detail": "Not Found"})));
        for (method, path, allow) in [
            (Method::GET, "/eta", "POST"),
            (Method::PUT, "/eta", "POST"),
            (Method::POST, "/health", "GET"),
            (Method::HEAD, "/health", "GET"),
        ] {
            let r = call(method.clone(), path, None, "").await;
            assert_eq!(r.status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
            assert_eq!(r.allow.as_deref(), Some(allow), "{method} {path}");
            if method != Method::HEAD {
                assert_eq!(r.body, json!({"detail": "Method Not Allowed"}), "{method} {path}");
            }
        }
    }

    /// FastAPI 가 JSON 으로 읽는 Content-Type 만 받아들이는지 확인한다.
    #[test]
    fn json_content_types() {
        for (value, want) in [
            ("application/json", true),
            ("Application/JSON; charset=utf-8", true),
            ("application/vnd.api+json", true),
            ("application/x-www-form-urlencoded", false),
            ("application/json/x", false),
            ("text/json", false),
            ("json", false),
            ("", false),
        ] {
            assert_eq!(is_json_content_type(Some(&HeaderValue::from_static(value))), want, "{value:?}");
        }
        assert!(!is_json_content_type(None));
    }
}
