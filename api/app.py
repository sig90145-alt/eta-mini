import importlib
import math
import os
from typing import Any

from fastapi import FastAPI, HTTPException, Request
from fastapi.encoders import jsonable_encoder
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from pydantic import BaseModel, Field

#어떤 방식의 api를 쓸 것인지를 외부에서 환경변수를 통해 주입
MODEL_IMPL = os.getenv("MODEL_IMPL", "rule")
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

