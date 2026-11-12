import html
import importlib
import math
import os
from typing import Any

from fastapi import FastAPI, HTTPException, Request
from fastapi.encoders import jsonable_encoder
from fastapi.exceptions import RequestValidationError
from fastapi.responses import HTMLResponse, JSONResponse
from pydantic import BaseModel, Field

#어떤 방식의 api를 쓸 것인지를 외부에서 환경변수를 통해 주입
MODEL_IMPL = os.getenv("MODEL_IMPL", "rule")
# 이미지 빌드 때 태그(v0.2 등)가 주입된다. 소개 페이지에 표시해 어떤 버전이 배포됐는지 확인한다.
APP_VERSION = os.getenv("APP_VERSION", "dev")
estimate_delivery_minutes = importlib.import_module(f"api.predictors.{MODEL_IMPL}").estimate_delivery_minutes

class EtaRequest(BaseModel):
    """배달 예상 시간 요청"""
    # 모델이 더 많은 정보를 원하면 여기에 필드를 추가
    # 아래 ge=0 와 같은 조건들을 유효성 검사 룰을 지정 (validation rules)
    distance_km: float = Field(ge=0, allow_inf_nan=False, description="가게에서 배달지까지 거리(km)")
    pending_orders: int = Field(ge=0, description="지금 밀려 있는 주문 수")

class EtaResponse(BaseModel):
    """배달 예상 시간 응답"""
    minutes: float
    model: str  # 어떤 구현체가 낸 예측인지 (장애 추적용)

app = FastAPI(title="delivery-eta")


def _json_safe(value: Any) -> Any:
    """NaN·Infinity 처럼 JSON 으로 표현할 수 없는 float 를 문자열로 바꾼다."""
    if isinstance(value, float) and not math.isfinite(value):
        return str(value)  # "nan", "inf", "-inf"
    if isinstance(value, dict):
        return {k: _json_safe(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_json_safe(v) for v in value]
    return value


@app.exception_handler(RequestValidationError)
async def validation_error_handler(request: Request, exc: RequestValidationError) -> JSONResponse:
    """기본 422 응답과 같지만, 오류에 담긴 NaN·Infinity 입력 때문에 500 이 나지 않게 한다."""
    return JSONResponse(status_code=422, content={"detail": _json_safe(jsonable_encoder(exc.errors()))})

INDEX_HTML = """<!doctype html>
<html lang="ko">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>delivery-eta API</title>
<style>
  :root { color-scheme: light dark; --fg: #1d1d1f; --bg: #fafafa; --card: #fff; --muted: #6e6e73; --line: #e5e5ea; --accent: #0a66c2; }
  @media (prefers-color-scheme: dark) {
    :root { --fg: #f2f2f7; --bg: #121214; --card: #1c1c1f; --muted: #a1a1a6; --line: #2c2c30; --accent: #5aa9ff; }
  }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--fg);
         font: 16px/1.6 system-ui, -apple-system, "Segoe UI", "Malgun Gothic", sans-serif; }
  main { max-width: 760px; margin: 0 auto; padding: 40px 16px 64px; }
  h1 { margin: 0 0 4px; font-size: 1.8rem; }
  .sub { margin: 0 0 24px; color: var(--muted); }
  .badges span { display: inline-block; margin: 0 6px 6px 0; padding: 2px 10px; border: 1px solid var(--line);
                 border-radius: 999px; font-size: .85rem; color: var(--muted); }
  section { margin-top: 20px; padding: 20px; background: var(--card); border: 1px solid var(--line); border-radius: 12px; }
  h2 { margin: 0 0 12px; font-size: 1.1rem; }
  table { width: 100%; border-collapse: collapse; font-size: .95rem; }
  th, td { padding: 8px 6px; border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; }
  th { color: var(--muted); font-weight: 600; }
  code, pre { font-family: ui-monospace, Consolas, monospace; font-size: .88rem; }
  pre { margin: 0; padding: 12px; overflow-x: auto; background: var(--bg); border: 1px solid var(--line); border-radius: 8px; }
  a { color: var(--accent); }
</style>
</head>
<body>
<main>
  <h1>delivery-eta API</h1>
  <p class="sub">거리와 밀린 주문 수로 배달 예상 시간(분)을 알려 주는 API입니다.</p>
  <div class="badges"><span>version {version}</span><span>model {model}</span></div>

  <section>
    <h2>엔드포인트</h2>
    <table>
      <tr><th>메서드</th><th>경로</th><th>설명</th></tr>
      <tr><td><code>GET</code></td><td><code>/</code></td><td>이 소개 페이지</td></tr>
      <tr><td><code>GET</code></td><td><a href="/health"><code>/health</code></a></td><td>서비스 상태와 현재 모델 이름</td></tr>
      <tr><td><code>POST</code></td><td><code>/eta</code></td><td>배달 예상 시간 계산</td></tr>
      <tr><td><code>GET</code></td><td><a href="/docs"><code>/docs</code></a></td><td>Swagger UI에서 직접 호출해 보기</td></tr>
    </table>
  </section>

  <section>
    <h2>요청 예시</h2>
    <pre>curl -X POST {{BASE}}/eta \\
  -H "Content-Type: application/json" \\
  -d '{"distance_km": 2.5, "pending_orders": 3}'</pre>
    <p>응답: <code>{"minutes": 23.5, "model": "{model}"}</code></p>
  </section>

  <section>
    <h2>입력 규칙</h2>
    <table>
      <tr><th>필드</th><th>타입</th><th>조건</th></tr>
      <tr><td><code>distance_km</code></td><td>number</td><td>0 이상, 유한한 숫자</td></tr>
      <tr><td><code>pending_orders</code></td><td>integer</td><td>0 이상</td></tr>
    </table>
    <p>규칙에 맞지 않거나 계산 결과가 너무 크면 <code>422</code>로 거절합니다.</p>
  </section>
</main>
<script>
  document.querySelector("pre").textContent =
    document.querySelector("pre").textContent.replace("{{BASE}}", location.origin);
</script>
</body>
</html>
"""


@app.get("/", response_class=HTMLResponse, include_in_schema=False)
def index() -> str:
    """API 소개 페이지(HTML)를 돌려준다."""
    return (
        INDEX_HTML.replace("{version}", html.escape(APP_VERSION))
        .replace("{model}", html.escape(MODEL_IMPL))
    )


@app.get("/health")
def health() -> dict[str, str]:
    """서비스 상태와 현재 구현체 이름을 돌려준다."""
    return {"status": "ok", "model": MODEL_IMPL}

@app.post("/eta", response_model=EtaResponse)
def eta(req: EtaRequest) -> EtaResponse:
    """배달 예상 시간을 계산"""
    try:
        minutes = estimate_delivery_minutes(req.distance_km, req.pending_orders)
    except OverflowError:
        minutes = math.inf  # 아주 큰 정수를 float 로 바꾸다 넘친 경우
    if not math.isfinite(minutes):
        # 입력이 너무 커서 결과를 숫자로 표현할 수 없다. minutes=null 로 응답하지 않고 거절한다.
        raise HTTPException(
            status_code=422,
            detail=[{
                "type": "value_error",
                "loc": ["body"],
                "msg": "Input values are too large to estimate delivery time",
                "input": _json_safe(jsonable_encoder(req)),
            }],
        )
    return EtaResponse(minutes=minutes, model=MODEL_IMPL)

