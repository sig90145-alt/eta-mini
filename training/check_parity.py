"""Python API 와 다른 언어 서비스(Go, Rust)의 예측이 같은지 비교한다.

기준은 학습한 원본 모델(MODEL_IMPL=sklearn)로 띄운 Python API 다.
같은 입력을 모든 서비스의 POST /eta 에 보내고, 응답 minutes 의 최대 오차가 TOLERANCE 미만이면 PASS.
하나라도 FAIL 이면 exit 1 로 끝나 CI 가 멈춘다.

pytest 테스트가 아니다. 서버가 모두 떠 있어야 한다.
    PY_URL=http://127.0.0.1:8080 TARGETS=go=http://127.0.0.1:8081,rust=http://127.0.0.1:8082 \
        python -m training.check_parity
"""
import json
import os
import random
import sys
import urllib.error
import urllib.request

TOLERANCE = 1e-4  # 분
PY_URL = os.getenv("PY_URL", "http://127.0.0.1:8080").rstrip("/")
TARGETS = os.getenv("TARGETS", "go=http://127.0.0.1:8081,rust=http://127.0.0.1:8082")


def parse_targets(spec: str) -> dict[str, str]:
    """"go=http://...,rust=http://..." 를 {이름: URL} 로 바꾼다."""
    targets = {}
    for item in spec.split(","):
        if not item.strip():
            continue
        name, sep, url = item.partition("=")
        if not sep or not name.strip() or not url.strip():
            sys.exit(f"TARGETS 형식 오류: {item!r} (이름=URL 이어야 한다)")
        targets[name.strip()] = url.strip().rstrip("/")
    return targets


def build_inputs() -> list[tuple[float, int]]:
    """고정 입력 3건 + seed 0 무작위 20건 (거리 0.5~8km, 주문 0~10건)."""
    rng = random.Random(0)
    fixed = [(3.0, 2), (0.0, 0), (8.0, 10)]
    rand = [(rng.uniform(0.5, 8.0), rng.randint(0, 10)) for _ in range(20)]
    return fixed + rand


def request_json(url: str, payload: dict | None = None) -> dict:
    """payload 가 있으면 POST, 없으면 GET 으로 호출하고 JSON 응답을 돌려준다."""
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=5) as res:
        return json.load(res)


def predict(base_url: str, distance_km: float, pending_orders: int) -> float:
    """서비스의 POST /eta 를 호출해 minutes 를 돌려준다."""
    body = request_json(f"{base_url}/eta", {"distance_km": distance_km, "pending_orders": pending_orders})
    return float(body["minutes"])


def main() -> int:
    targets = parse_targets(TARGETS)
    inputs = build_inputs()

    # 기준 서버가 원본 모델로 떠 있는지 먼저 확인한다. rule/onnx 와 비교하면 의미가 없다.
    try:
        model = request_json(f"{PY_URL}/health").get("model")
        expected = [predict(PY_URL, d, o) for d, o in inputs]
    except (urllib.error.URLError, OSError, ValueError, KeyError) as e:
        print(f"FAIL [python] 기준 서버 {PY_URL} 호출 실패: {e}")
        return 1
    if model != "sklearn":
        print(f"FAIL [python] 기준 서버 모델이 {model!r} 다. MODEL_IMPL=sklearn 으로 띄워야 한다")
        return 1
    print(f"기준: python ({PY_URL}, model={model}), 입력 {len(inputs)}건, 허용 오차 {TOLERANCE:g}분")

    ok = True
    for name, url in targets.items():
        try:
            got = [predict(url, d, o) for d, o in inputs]
        except (urllib.error.URLError, OSError, ValueError, KeyError) as e:
            print(f"FAIL [{name}] {url} 호출 실패: {e}")
            ok = False
            continue
        diffs = [abs(g - w) for g, w in zip(got, expected)]
        worst = max(range(len(diffs)), key=diffs.__getitem__)
        passed = diffs[worst] < TOLERANCE
        ok &= passed
        d, o = inputs[worst]
        print(("PASS" if passed else "FAIL") + f" [{name}] 최대 오차 {diffs[worst]:.2e}분 "
              f"(입력 거리 {d:.3f}km, 주문 {o}건: python {expected[worst]:.6f} vs {name} {got[worst]:.6f})")

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
