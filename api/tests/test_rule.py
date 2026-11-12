import pytest

from api.predictors.rule import estimate_delivery_minutes


@pytest.mark.parametrize(
    ("distance_km", "pending_orders", "expected"),
    [
        (0, 0, 10.0),
        (1, 0, 13.0),
        (0, 1, 12.0),
        (2.5, 3, 23.5),
        (1000, 10000, 23010.0),
    ],
)
def test_known_inputs(distance_km, pending_orders, expected):
    """기본 10분 + 1km당 3분 + 대기 주문 1건당 2분 공식대로 계산하는지 확인한다."""
    assert estimate_delivery_minutes(distance_km, pending_orders) == pytest.approx(expected)


def test_returns_float():
    """정수 입력이어도 float 를 돌려주는지 확인한다."""
    assert isinstance(estimate_delivery_minutes(1, 1), float)


def test_fractional_distance():
    """부동소수 오차가 생기는 소수 거리도 공식대로 계산하는지 확인한다."""
    assert estimate_delivery_minutes(0.1, 0) == pytest.approx(10.3)


@pytest.mark.parametrize(("distance_km", "pending_orders"), [(0, 0), (1.5, 2), (10, 50)])
def test_monotonic_in_distance(distance_km, pending_orders):
    """거리가 늘면 예상 시간이 줄지 않는지 확인한다."""
    assert estimate_delivery_minutes(distance_km + 1, pending_orders) >= estimate_delivery_minutes(
        distance_km, pending_orders
    )


@pytest.mark.parametrize(("distance_km", "pending_orders"), [(0, 0), (1.5, 2), (10, 50)])
def test_monotonic_in_pending_orders(distance_km, pending_orders):
    """대기 주문이 늘면 예상 시간이 줄지 않는지 확인한다."""
    assert estimate_delivery_minutes(distance_km, pending_orders + 1) >= estimate_delivery_minutes(
        distance_km, pending_orders
    )
