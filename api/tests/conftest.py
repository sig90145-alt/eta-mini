import os

# app.py 는 import 시점에 MODEL_IMPL 을 읽으므로, 앱을 import 하기 전에 고정한다.
os.environ["MODEL_IMPL"] = "rule"

import pytest
from fastapi.testclient import TestClient

from api.app import app


@pytest.fixture
def client() -> TestClient:
    return TestClient(app)
