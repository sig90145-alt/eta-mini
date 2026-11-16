//! 파이썬 `json.loads` 와 같은 규칙으로 요청 본문을 읽는 JSON 파서.
//!
//! FastAPI 는 본문을 `json.loads` 로 읽기 때문에 표준 JSON 과 몇 가지가 다르다.
//!   - `NaN`, `Infinity`, `-Infinity` 를 숫자로 받는다 (그 뒤 pydantic 이 finite_number 로 거절한다)
//!   - 같은 키가 여러 번 나오면 마지막 값을 쓴다
//!   - 정수는 크기 제한 없이 그대로, 소수점·지수가 있으면 float 로 읽는다 (1e999 → inf)
//!
//! serde_json 은 첫 번째 규칙 때문에 쓸 수 없다 (`NaN` 을 형식 오류로 보고 loc 가 달라진다).
//! 오류 메시지와 위치(문자 단위)도 파이썬과 같게 맞춰 422 응답의 detail 이 같아지게 한다.

use serde_json::{Map, Number, Value};

/// 파싱한 JSON 값. 숫자는 파이썬처럼 정수와 실수를 구분한다.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// 10진수 정수 표기 그대로 (`-0` 만 `0` 으로 바꾼다). 크기 제한이 없다.
    Int(String),
    /// 소수점이나 지수가 있는 숫자, 또는 NaN·Infinity. 범위를 넘으면 ±inf 다.
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    /// 키 순서를 지킨다. 같은 키는 처음 자리에 마지막 값이 들어간다 (파이썬 dict 와 같다).
    Object(Vec<(String, Json)>),
}

impl Json {
    /// 응답에 넣을 serde_json 값으로 바꾼다. NaN·Infinity 는 api/app.py 의 `_json_safe` 처럼
    /// "nan", "inf", "-inf" 문자열이 된다.
    pub fn to_value(&self) -> Value {
        match self {
            Json::Null => Value::Null,
            Json::Bool(b) => Value::Bool(*b),
            Json::Int(text) => int_value(text),
            Json::Float(f) => float_value(*f),
            Json::Str(s) => Value::String(s.clone()),
            Json::Array(items) => Value::Array(items.iter().map(Json::to_value).collect()),
            Json::Object(members) => Value::Object(
                members.iter().map(|(k, v)| (k.clone(), v.to_value())).collect::<Map<_, _>>(),
            ),
        }
    }
}

/// 10진수 정수 문자열을 자릿수 그대로 JSON 숫자로 만든다.
pub fn int_value(text: &str) -> Value {
    text.parse::<Number>().map_or_else(|_| Value::String(text.to_owned()), Value::Number)
}

/// float 를 JSON 숫자로 만든다. 유한하지 않으면 "nan", "inf", "-inf" 문자열이다.
pub fn float_value(f: f64) -> Value {
    match Number::from_f64(f) {
        Some(n) => Value::Number(n),
        None if f.is_nan() => Value::String("nan".into()),
        None if f > 0.0 => Value::String("inf".into()),
        None => Value::String("-inf".into()),
    }
}

/// 파이썬 `json.JSONDecodeError` 의 msg 와 pos(문자 위치).
#[derive(Debug, Clone, PartialEq)]
pub struct JsonError {
    pub msg: &'static str,
    pub pos: usize,
}

/// 중첩 깊이 상한. 파이썬은 재귀 한도에서 멈추지만 여기서는 스택이 넘치기 전에 형식 오류로 거절한다.
const MAX_DEPTH: usize = 512;

/// 본문 바이트를 파싱한다. 파이썬처럼 맨 앞의 UTF-8 BOM 은 건너뛴다.
pub fn parse_bytes(raw: &[u8]) -> Result<Json, JsonError> {
    let raw = raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw);
    match std::str::from_utf8(raw) {
        Ok(text) => parse(text),
        Err(e) => Err(JsonError {
            msg: "Invalid UTF-8",
            pos: String::from_utf8_lossy(&raw[..e.valid_up_to()]).chars().count(),
        }),
    }
}

/// 문자열 하나가 앞뒤 공백을 빼고 JSON 값 하나여야 한다 (`json.loads` 와 같다).
pub fn parse(text: &str) -> Result<Json, JsonError> {
    let chars: Vec<char> = text.chars().collect();
    let mut p = Parser { s: &chars, depth: 0 };
    let start = p.ws(0);
    let (value, end) = p.value(start)?;
    let end = p.ws(end);
    if end != chars.len() {
        return Err(err("Extra data", end));
    }
    Ok(value)
}

fn err(msg: &'static str, pos: usize) -> JsonError {
    JsonError { msg, pos }
}

struct Parser<'a> {
    s: &'a [char],
    depth: usize,
}

impl Parser<'_> {
    fn at(&self, i: usize) -> Option<char> {
        self.s.get(i).copied()
    }

    fn is_digit(&self, i: usize) -> bool {
        self.at(i).is_some_and(|c| c.is_ascii_digit())
    }

    fn starts_with(&self, i: usize, word: &str) -> bool {
        let mut j = i;
        for c in word.chars() {
            if self.at(j) != Some(c) {
                return false;
            }
            j += 1;
        }
        true
    }

    /// 파이썬 json 이 공백으로 보는 문자(스페이스, 탭, 줄바꿈, 캐리지 리턴)를 건너뛴다.
    fn ws(&self, mut i: usize) -> usize {
        while matches!(self.at(i), Some(' ' | '\t' | '\n' | '\r')) {
            i += 1;
        }
        i
    }

    /// i 에서 값 하나를 읽고 (값, 다음 위치) 를 돌려준다.
    fn value(&mut self, i: usize) -> Result<(Json, usize), JsonError> {
        let lit = |v: Json, len: usize| Ok((v, i + len));
        match self.at(i) {
            Some('"') => self.string(i + 1).map(|(s, end)| (Json::Str(s), end)),
            Some('{') => self.nested(i, Self::object),
            Some('[') => self.nested(i, Self::array),
            Some('n') if self.starts_with(i, "null") => lit(Json::Null, 4),
            Some('t') if self.starts_with(i, "true") => lit(Json::Bool(true), 4),
            Some('f') if self.starts_with(i, "false") => lit(Json::Bool(false), 5),
            Some('N') if self.starts_with(i, "NaN") => lit(Json::Float(f64::NAN), 3),
            Some('I') if self.starts_with(i, "Infinity") => lit(Json::Float(f64::INFINITY), 8),
            Some('-') if self.starts_with(i, "-Infinity") => lit(Json::Float(f64::NEG_INFINITY), 9),
            _ => self.number(i).ok_or(err("Expecting value", i)),
        }
    }

    fn nested(
        &mut self,
        i: usize,
        parse: fn(&mut Self, usize) -> Result<(Json, usize), JsonError>,
    ) -> Result<(Json, usize), JsonError> {
        if self.depth >= MAX_DEPTH {
            return Err(err("Maximum nesting depth exceeded", i));
        }
        self.depth += 1;
        let result = parse(self, i + 1);
        self.depth -= 1;
        result
    }

    /// `{` 다음 위치에서 객체를 읽는다.
    fn object(&mut self, start: usize) -> Result<(Json, usize), JsonError> {
        let mut members: Vec<(String, Json)> = Vec::new();
        let mut i = self.ws(start);
        if self.at(i) == Some('}') {
            return Ok((Json::Object(members), i + 1));
        }
        loop {
            if self.at(i) != Some('"') {
                return Err(err("Expecting property name enclosed in double quotes", i));
            }
            let (key, next) = self.string(i + 1)?;
            i = self.ws(next);
            if self.at(i) != Some(':') {
                return Err(err("Expecting ':' delimiter", i));
            }
            let (value, next) = self.value(self.ws(i + 1))?;
            match members.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = value,
                None => members.push((key, value)),
            }
            i = self.ws(next);
            match self.at(i) {
                Some('}') => return Ok((Json::Object(members), i + 1)),
                Some(',') => {}
                _ => return Err(err("Expecting ',' delimiter", i)),
            }
            let comma = i;
            i = self.ws(i + 1);
            if self.at(i) == Some('}') {
                return Err(err("Illegal trailing comma before end of object", comma));
            }
        }
    }

    /// `[` 다음 위치에서 배열을 읽는다.
    fn array(&mut self, start: usize) -> Result<(Json, usize), JsonError> {
        let mut items = Vec::new();
        let mut i = self.ws(start);
        if self.at(i) == Some(']') {
            return Ok((Json::Array(items), i + 1));
        }
        loop {
            let (value, next) = self.value(i)?;
            items.push(value);
            i = self.ws(next);
            match self.at(i) {
                Some(']') => return Ok((Json::Array(items), i + 1)),
                Some(',') => {}
                _ => return Err(err("Expecting ',' delimiter", i)),
            }
            let comma = i;
            i = self.ws(i + 1);
            if self.at(i) == Some(']') {
                return Err(err("Illegal trailing comma before end of array", comma));
            }
        }
    }

    /// 여는 따옴표 다음 위치(start)에서 문자열을 읽는다. 오류 위치는 CPython 의 _json 모듈과 같다.
    fn string(&self, start: usize) -> Result<(String, usize), JsonError> {
        let begin = start - 1;
        let unterminated = || err("Unterminated string starting at", begin);
        let mut out = String::new();
        let mut i = start;
        loop {
            let c = self.at(i).ok_or_else(unterminated)?;
            match c {
                '"' => return Ok((out, i + 1)),
                '\\' => {
                    let e = self.at(i + 1).ok_or_else(unterminated)?;
                    if e != 'u' {
                        out.push(match e {
                            '"' => '"',
                            '\\' => '\\',
                            '/' => '/',
                            'b' => '\u{8}',
                            'f' => '\u{c}',
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            _ => return Err(err("Invalid \\escape", i)),
                        });
                        i += 2;
                        continue;
                    }
                    // \uXXXX: 16진수 4자리가 모자라거나 틀리면 'u' 위치에서 오류다.
                    let bad_u = |at: usize| err("Invalid \\uXXXX escape", at);
                    let mut cp = self.hex4(i + 2).ok_or(bad_u(i + 1))?;
                    i += 6;
                    // 상위 서로게이트 뒤에 \uXXXX 가 오면 한 글자로 합친다.
                    if (0xD800..0xDC00).contains(&cp)
                        && i + 6 < self.s.len()
                        && self.at(i) == Some('\\')
                        && self.at(i + 1) == Some('u')
                    {
                        let lo = self.hex4(i + 2).ok_or(bad_u(i + 1))?;
                        if (0xDC00..0xE000).contains(&lo) {
                            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            i += 6;
                        }
                    }
                    // 짝 없는 서로게이트는 Rust 문자열에 넣을 수 없어 U+FFFD 로 바꾼다.
                    out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                }
                c if (c as u32) < 0x20 => return Err(err("Invalid control character at", i)),
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
    }

    fn hex4(&self, i: usize) -> Option<u32> {
        (i..i + 4).try_fold(0u32, |acc, j| Some(acc * 16 + self.at(j)?.to_digit(16)?))
    }

    /// 파이썬 json 의 숫자 문법 `-?(0|[1-9]\d*)(\.\d+)?([eE][-+]?\d+)?` 을 i 에서 찾는다.
    fn number(&self, start: usize) -> Option<(Json, usize)> {
        let mut i = start;
        if self.at(i) == Some('-') {
            i += 1;
        }
        match self.at(i) {
            Some('0') => i += 1,
            Some('1'..='9') => {
                while self.is_digit(i) {
                    i += 1;
                }
            }
            _ => return None,
        }
        let mut is_float = false;
        if self.at(i) == Some('.') && self.is_digit(i + 1) {
            i += 1;
            while self.is_digit(i) {
                i += 1;
            }
            is_float = true;
        }
        if matches!(self.at(i), Some('e' | 'E')) {
            let mut j = i + 1;
            if matches!(self.at(j), Some('+' | '-')) {
                j += 1;
            }
            if self.is_digit(j) {
                while self.is_digit(j) {
                    j += 1;
                }
                i = j;
                is_float = true;
            }
        }
        let text: String = self.s[start..i].iter().collect();
        let value = if is_float {
            // Rust 의 f64 파싱은 파이썬 float() 처럼 가장 가까운 값으로 반올림하고, 넘치면 ±inf 다.
            Json::Float(text.parse().ok()?)
        } else if text == "-0" {
            Json::Int("0".into())
        } else {
            Json::Int(text)
        };
        Some((value, i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(members: &[(&str, Json)]) -> Json {
        Json::Object(members.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    /// 파이썬 json.loads 가 받는 표기를 같은 값으로 읽는지 확인한다.
    #[test]
    fn parses_python_json() {
        assert_eq!(
            parse(r#" {"a": 1, "b": -0, "c": 2.5e1, "d": [true, null], "e": "xé😀"} "#).unwrap(),
            obj(&[
                ("a", Json::Int("1".into())),
                ("b", Json::Int("0".into())),
                ("c", Json::Float(25.0)),
                ("d", Json::Array(vec![Json::Bool(true), Json::Null])),
                ("e", Json::Str("xé😀".into())),
            ])
        );
        assert_eq!(parse("1e999").unwrap(), Json::Float(f64::INFINITY));
        assert_eq!(parse("-Infinity").unwrap(), Json::Float(f64::NEG_INFINITY));
        assert!(matches!(parse("NaN").unwrap(), Json::Float(f) if f.is_nan()));
        assert_eq!(parse(r#"{"a": 1, "b": 2, "a": 3}"#).unwrap(), obj(&[("a", Json::Int("3".into())), ("b", Json::Int("2".into()))]));
        assert_eq!(parse_bytes(b"\xEF\xBB\xBF[]").unwrap(), Json::Array(vec![]));
    }

    /// 형식 오류의 메시지와 문자 위치가 파이썬과 같은지 확인한다.
    #[test]
    fn errors_match_python() {
        let cases: &[(&str, &str, usize)] = &[
            ("", "Expecting value", 0),
            ("not json", "Expecting value", 0),
            ("{bad", "Expecting property name enclosed in double quotes", 1),
            (r#"{"a" 1}"#, "Expecting ':' delimiter", 5),
            (r#"{"a": 1 "b": 2}"#, "Expecting ',' delimiter", 8),
            (r#"{"a": }"#, "Expecting value", 6),
            (r#"{"a": 1,}"#, "Illegal trailing comma before end of object", 7),
            ("[1,]", "Illegal trailing comma before end of array", 2),
            (r#"{"a": 1} {}"#, "Extra data", 9),
            (r#""abc"#, "Unterminated string starting at", 0),
            ("\"a\u{1}\"", "Invalid control character at", 2),
            (r#""\x""#, "Invalid \\escape", 1),
            (r#""\u12x4""#, "Invalid \\uXXXX escape", 2),
            (r#""\u12"#, "Invalid \\uXXXX escape", 2),
            (r#""ሴ"#, "Unterminated string starting at", 0),
            (r#""\ud83d\ude0"#, "Invalid \\uXXXX escape", 8),
            (r#""\ud83d\uZZZZ""#, "Invalid \\uXXXX escape", 8),
            (r#""\ud83d\x""#, "Invalid \\escape", 7),
            ("01", "Extra data", 1),
            ("\"가\" x", "Extra data", 4),
        ];
        for &(text, msg, pos) in cases {
            assert_eq!(parse(text), Err(JsonError { msg, pos }), "{text:?}");
        }
    }

    /// 아주 깊게 중첩된 본문도 스택이 넘치지 않고 형식 오류가 되는지 확인한다.
    #[test]
    fn deep_nesting_is_rejected() {
        let text = "[".repeat(100_000);
        assert_eq!(parse(&text).unwrap_err().msg, "Maximum nesting depth exceeded");
    }

    /// 응답에 넣을 때 큰 정수는 자릿수 그대로, 무한대는 문자열이 되는지 확인한다.
    #[test]
    fn to_value_is_json_safe() {
        let big = "9".repeat(50);
        assert_eq!(Json::Int(big.clone()).to_value().to_string(), big);
        assert_eq!(Json::Float(f64::INFINITY).to_value(), Value::String("inf".into()));
        assert_eq!(Json::Float(f64::NAN).to_value(), Value::String("nan".into()));
        assert_eq!(Json::Float(-0.0).to_value().to_string(), "-0.0");
    }
}
