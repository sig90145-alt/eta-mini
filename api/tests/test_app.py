import math
import os
import subprocess
import sys
from pathlib import Path

import pytest


def test_health(client):
    """/health 가 200 과 상태·구현체 이름을 돌려주는지 확인한다."""
    res = client.get("/health")
    assert res.status_code == 200
    assert res.json() == {"status": "ok", "model": "rule"}


def test_eta_response_shape(client):
    """/eta 응답이 minutes(숫자)와 model(문자열) 두 필드만 갖는지 확인한다."""
    res = client.post("/eta", json={"distance_km": 2.5, "pending_orders": 3})
    assert res.status_code == 200
    body = res.json()
    assert set(body) == {"minutes", "model"}
    assert isinstance(body["minutes"], (int, float))
    assert body["model"] == "rule"


@pytest.mark.parametrize(
    ("distance_km", "pending_orders", "expected"),
    [
        (2.5, 3, 23.5),
        (0, 0, 10.0),  # 경계값: 둘 다 0
        (0, 5, 20.0),
        (4, 0, 22.0),
        (1000, 10000, 23010.0),  # 큰 값
    ],
)
def test_eta_known_values(client, distance_km, pending_orders, expected):
    """알려진 입력에 대해 예상 시간(분)이 기대값과 같은지 확인한다."""
    res = client.post(
        "/eta", json={"distance_km": distance_km, "pending_orders": pending_orders}
    )
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(expected)


@pytest.mark.parametrize(
    ("payload", "bad_field"),
    [
        ({"distance_km": -1, "pending_orders": 3}, "distance_km"),
        ({"distance_km": -0.01, "pending_orders": 0}, "distance_km"),
        ({"distance_km": 1, "pending_orders": -1}, "pending_orders"),
    ],
)
def test_eta_negative_is_422(client, payload, bad_field):
    """음수 입력은 422 로 거절되고, 잘못된 필드가 loc 에 표시되는지 확인한다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422
    locs = [err["loc"] for err in res.json()["detail"]]
    assert ["body", bad_field] in locs


@pytest.mark.parametrize(
    ("payload", "bad_field"),
    [
        ({"distance_km": "far", "pending_orders": 3}, "distance_km"),
        ({"distance_km": 1, "pending_orders": "many"}, "pending_orders"),
        ({"distance_km": None, "pending_orders": 3}, "distance_km"),
        ({"distance_km": 1, "pending_orders": 1.5}, "pending_orders"),  # 정수가 아님
    ],
)
def test_eta_non_numeric_is_422(client, payload, bad_field):
    """숫자가 아니거나 타입이 맞지 않는 입력은 422 로 거절되는지 확인한다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422
    locs = [err["loc"] for err in res.json()["detail"]]
    assert ["body", bad_field] in locs


@pytest.mark.parametrize(
    "payload",
    [
        {},
        {"distance_km": 1},
        {"pending_orders": 1},
    ],
)
def test_eta_missing_field_is_422(client, payload):
    """필수 필드가 빠지면 422 로 거절되는지 확인한다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422


def test_eta_get_not_allowed(client):
    """/eta 는 POST 전용이라 GET 은 405 인지 확인한다."""
    assert client.get("/eta").status_code == 405


# ---------------------------------------------------------------------------
# 비유한(non-finite) 숫자와 오버플로
# ---------------------------------------------------------------------------

JSON_HEADERS = {"content-type": "application/json"}


@pytest.mark.parametrize("literal", ["NaN", "Infinity", "-Infinity"])
def test_eta_non_finite_distance_is_422(raw_client, literal):
    """distance_km 가 NaN·Infinity 면 500 이 아니라 422 로 거절되고 loc 에 표시되는지 확인한다."""
    res = raw_client.post(
        "/eta",
        content=f'{{"distance_km": {literal}, "pending_orders": 0}}',
        headers=JSON_HEADERS,
    )
    assert res.status_code == 422
    locs = [err["loc"] for err in res.json()["detail"]]
    assert ["body", "distance_km"] in locs


@pytest.mark.parametrize(
    "payload",
    [
        {"distance_km": 1e308, "pending_orders": 0},  # 계산 결과가 inf 로 넘침
        {"distance_km": 1.7e308, "pending_orders": 10**308},  # 두 항을 더하다 넘침
        {"distance_km": 0, "pending_orders": 10**400},  # 정수를 float 로 바꾸다 OverflowError
    ],
    ids=["distance-overflow", "sum-overflow", "int-too-large"],
)
def test_eta_overflow_is_422(raw_client, payload):
    """계산이 넘칠 만큼 큰 입력은 500 이나 minutes=null 이 아니라 422 로 거절되는지 확인한다."""
    res = raw_client.post("/eta", json=payload)
    assert res.status_code == 422
    assert ["body"] in [err["loc"] for err in res.json()["detail"]]


def test_eta_largest_finite_result_is_200(client):
    """넘치기 직전의 큰 입력은 거절되지 않고 유한한 minutes 를 돌려주는지 확인한다."""
    res = client.post("/eta", json={"distance_km": 1e307, "pending_orders": 0})
    assert res.status_code == 200
    minutes = res.json()["minutes"]
    assert isinstance(minutes, (int, float)) and math.isfinite(minutes)


# ---------------------------------------------------------------------------
# 느슨한(lax) 타입 변환: 현재 허용하는 입력을 계약으로 고정한다.
# 정책을 바꿔 거절하기로 하면 이 테스트를 422 기대로 바꾼다.
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        ({"distance_km": "2.5", "pending_orders": "3"}, 23.5),  # 숫자 문자열
        ({"distance_km": 1, "pending_orders": 1.0}, 15.0),  # 소수부가 0 인 float
        ({"distance_km": True, "pending_orders": False}, 13.0),  # 불리언 → 1, 0
        ({"distance_km": 1, "pending_orders": 1, "extra": 9}, 15.0),  # 모르는 필드 무시
        ({"distance_km": -0.0, "pending_orders": 0}, 10.0),  # 음수 0 은 0 으로 취급
    ],
)
def test_eta_lax_inputs_accepted(client, payload, expected):
    """현재 허용하는 느슨한 입력이 200 과 기대값을 돌려주는지 확인한다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(expected)


def test_eta_huge_int_pending_orders(client):
    """아주 큰 정수 pending_orders 도 200 과 유한한 숫자를 돌려주는지 확인한다."""
    res = client.post("/eta", json={"distance_km": 0, "pending_orders": 10**30})
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(2e30)


# ---------------------------------------------------------------------------
# 요청 본문 형식 오류
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "kwargs",
    [
        {"content": "{bad", "headers": JSON_HEADERS},  # 깨진 JSON
        {"json": [1, 2]},  # 객체가 아닌 JSON
        {},  # 본문 없음
        {
            "content": '{"distance_km": 1, "pending_orders": 1}',
            "headers": {"content-type": "text/plain"},
        },  # JSON 이 아닌 Content-Type
    ],
    ids=["broken-json", "array-body", "no-body", "text-plain"],
)
def test_eta_malformed_body_is_422(client, kwargs):
    """형식이 잘못된 요청 본문은 422 로 거절되는지 확인한다."""
    assert client.post("/eta", **kwargs).status_code == 422


def test_eta_reports_all_invalid_fields(client):
    """두 필드가 모두 잘못되면 두 필드의 오류가 모두 보고되는지 확인한다."""
    res = client.post("/eta", json={"distance_km": -1, "pending_orders": "many"})
    assert res.status_code == 422
    locs = [err["loc"] for err in res.json()["detail"]]
    assert ["body", "distance_km"] in locs
    assert ["body", "pending_orders"] in locs


# ---------------------------------------------------------------------------
# 라우팅과 설정
# ---------------------------------------------------------------------------


def test_health_post_not_allowed(client):
    """/health 는 GET 전용이라 POST 는 405 인지 확인한다."""
    assert client.post("/health").status_code == 405


def test_unknown_path_is_404(client):
    """정의되지 않은 경로는 404 인지 확인한다."""
    assert client.get("/does-not-exist").status_code == 404


def test_health_and_eta_report_same_model(client):
    """/health 와 /eta 가 같은 구현체 이름을 보고하는지 확인한다."""
    health_model = client.get("/health").json()["model"]
    eta_model = client.post("/eta", json={"distance_km": 1, "pending_orders": 1}).json()["model"]
    assert health_model == eta_model


def test_unknown_model_impl_fails_at_import():
    """MODEL_IMPL 이 없는 구현체를 가리키면 앱 import 가 ModuleNotFoundError 로 실패하는지 확인한다."""
    root = Path(__file__).resolve().parents[2]
    env = {**os.environ, "MODEL_IMPL": "does_not_exist"}
    proc = subprocess.run(
        [sys.executable, "-c", "import api.app"],
        cwd=root, env=env, capture_output=True, text=True, timeout=60,
    )
    assert proc.returncode != 0
    assert "ModuleNotFoundError" in proc.stderr
