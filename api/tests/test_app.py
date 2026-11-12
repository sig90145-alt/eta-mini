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
