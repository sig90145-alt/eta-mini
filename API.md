# delivery-eta API

배달 예상 시간을 계산하는 FastAPI 서비스입니다. 코드는 [api/app.py](api/app.py)에 있습니다.

| 엔드포인트 | 설명 |
| --- | --- |
| `GET /health` | 서비스 상태와 현재 구현체 이름 (`{"status": "ok", "model": "rule"}`) |
| `POST /eta` | 거리와 밀린 주문 수를 받아 예상 소요 시간(분)과 구현체 이름을 반환 |

## 예측 구현체 선택 (MODEL_IMPL)

API는 환경변수 `MODEL_IMPL` 값에 따라 배달 예상 시간을 계산할 구현체(predictor)를 고릅니다.
코드는 그대로 두고 배포 설정만 바꿔서 규칙 기반 baseline과 학습된 모델을 교체할 수 있습니다.

### 동작 방식

```python
MODEL_IMPL = os.getenv("MODEL_IMPL", "rule")
estimate_delivery_minutes = importlib.import_module(f"api.predictors.{MODEL_IMPL}").estimate_delivery_minutes
```

1. 서버 프로세스가 시작될 때 `MODEL_IMPL` 환경변수를 읽습니다. 값이 없으면 `"rule"`을 씁니다.
2. 값을 모듈 이름으로 사용해 `api.predictors.<MODEL_IMPL>` 모듈을 import합니다.
3. 그 모듈의 `estimate_delivery_minutes` 함수를 `/eta` 엔드포인트에서 호출합니다.

| `MODEL_IMPL` 값 | 로드되는 파일 |
| --- | --- |
| (미설정) / `rule` | `api/predictors/rule.py` |
| `ml` | `api/predictors/ml.py` (직접 작성해야 함) |

현재 어떤 구현체가 동작 중인지는 `GET /health` 응답의 `model` 필드와 `POST /eta` 응답의 `model` 필드에서 확인할 수 있습니다.

### 환경변수 주입 방법

코드는 값을 읽기만 합니다. 값은 서버를 실행하는 쪽에서 넣습니다.

#### 로컬: PowerShell

```powershell
$env:MODEL_IMPL = "rule"
uv run uvicorn api.app:app --reload
```

현재 터미널 세션에서만 유지됩니다.

#### 로컬: Git Bash / Linux / macOS

```bash
MODEL_IMPL=rule uv run uvicorn api.app:app
```

이 명령 한 번에만 적용됩니다.

#### `.env` 파일 + uv

```
# .env
MODEL_IMPL=rule
```

```bash
uv run --env-file .env uvicorn api.app:app
```

#### Docker

```bash
docker run -e MODEL_IMPL=ml my-image
```

Dockerfile에 `ENV MODEL_IMPL=rule`로 기본값을 두고, 실행 시 `-e`로 덮어쓸 수 있습니다.

#### Kubernetes

Deployment의 `env:` 항목이나 ConfigMap으로 주입합니다.

### 새 구현체 추가하기

1. `api/predictors/` 아래에 새 파일을 만듭니다. 예: `api/predictors/ml.py`
2. 아래와 **같은 이름, 같은 시그니처**의 함수를 정의합니다.

   ```python
   def estimate_delivery_minutes(distance_km: float, pending_orders: int) -> float:
       # 예: 학습된 모델을 로드해서 예측
       ...
   ```

3. `MODEL_IMPL=ml`로 서버를 실행합니다.

`app.py`는 수정할 필요가 없습니다. `estimate_delivery_minutes(distance_km, pending_orders) -> float`가 모든 구현체가 지켜야 하는 인터페이스입니다.

### 주의사항

- **시작할 때 한 번만 읽습니다.** 모듈 최상단에서 값을 읽고 구현체를 import하므로, 실행 중에 환경변수를 바꿔도 반영되지 않습니다. 구현체를 바꾸려면 서버를 재시작해야 합니다.
- **파일이 없으면 서버가 뜨지 않습니다.** `MODEL_IMPL` 값에 해당하는 모듈이 없으면 시작 시점에 `ModuleNotFoundError`가 발생합니다.
- **입력이 늘어나면 인터페이스도 바뀝니다.** 모델에 새 입력(예: 날씨, 시간대)이 필요하면 `EtaRequest`, `app.py`의 호출부, 그리고 기존 `rule.py`를 포함한 모든 구현체의 시그니처를 함께 맞춰야 합니다.

## 요청 입력 검증

`EtaRequest`는 Pydantic `Field`로 입력 범위를 제한합니다.

```python
distance_km: float = Field(ge=0, description="가게에서 배달지까지 거리(km)")
pending_orders: int = Field(ge=0, description="지금 밀려 있는 주문 수")
```

`ge=0`은 **greater than or equal to 0**, 즉 값이 0 이상이어야 한다는 뜻입니다. 거리나 밀린 주문 수는 음수가 될 수 없으므로 음수 입력을 막습니다.

FastAPI는 요청 본문을 `EtaRequest`로 변환할 때 이 조건을 자동으로 검사합니다. 조건을 어기면 `eta()` 함수까지 가지 않고 바로 **422** 에러를 돌려줍니다.

```json
// 요청: {"distance_km": -1, "pending_orders": 3}
// 응답: 422
{
  "detail": [{
    "type": "greater_than_equal",
    "loc": ["body", "distance_km"],
    "msg": "Input should be greater than or equal to 0"
  }]
}
```

그래서 predictor 구현체에서는 음수 입력을 따로 처리하지 않아도 됩니다.

같은 계열 옵션:

| 옵션 | 의미 |
| --- | --- |
| `ge` | ≥ (이상) |
| `gt` | > (초과) |
| `le` | ≤ (이하) |
| `lt` | < (미만) |

예를 들어 `Field(ge=0, le=50)`이라고 쓰면 "0km 이상 50km 이하"가 됩니다. 이 범위는 `/docs`(Swagger) 화면에도 그대로 표시됩니다.
