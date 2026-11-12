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
