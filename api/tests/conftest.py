import os

# app.py 는 import 시점에 MODEL_IMPL 을 읽으므로, 앱을 import 하기 전에 고정한다.
os.environ["MODEL_IMPL"] = "rule"

import pytest
from fastapi.testclient import TestClient

from api.app import app


@pytest.fixture
def client() -> TestClient:
    return TestClient(app)


@pytest.fixture
def raw_client() -> TestClient:
    """서버 예외를 다시 던지지 않고 500 응답으로 돌려주는 클라이언트."""
    return TestClient(app, raise_server_exceptions=False)
