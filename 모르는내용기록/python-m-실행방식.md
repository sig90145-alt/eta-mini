# `python -m` 으로 실행하는 이유

> 작성: 2026-11-13 15:54

학습 스크립트를 실행할 때 아래 명령을 씁니다.

```
uv run python -m training.train_sklearn
```

`uv run python training/train_sklearn.py`처럼 파일 경로로 실행하면 안 될까요?
결과 파일이 `training/` 폴더 안에 생겨서 그런 걸까요?

**아닙니다.** 결과 파일은 두 방식 모두 같은 곳에 저장됩니다.
`-m`을 쓰는 이유는 **import가 실패하지 않게 하기 위해서**입니다.

아래에서 두 가지를 하나씩 설명합니다.

1. 결과 파일이 어디에 저장되는지
2. import가 왜 실패하는지

---

## 1. 결과 파일이 저장되는 위치

### 상대 경로는 "지금 내가 서 있는 폴더"가 기준

[training/train_sklearn.py](../training/train_sklearn.py)에는 이런 코드가 있습니다.

```python
joblib.dump(model, "artifacts/sklearn.joblib")
```

`"artifacts/sklearn.joblib"`는 `C:\...`처럼 처음부터 적은 전체 경로가 아니라 **상대 경로**입니다.
상대 경로는 "어디를 기준으로" 찾을지가 정해져 있어야 하는데, Python은
**스크립트 파일이 있는 폴더가 아니라 명령을 실행한 폴더**를 기준으로 삼습니다.
이 폴더를 **현재 작업 디렉터리**(Current Working Directory, CWD)라고 부릅니다.

> 비유: 친구에게 "문 열고 오른쪽 방"이라고 알려주면, 친구는 **지금 서 있는 곳**에서 오른쪽 방을 찾습니다.
> 그 말을 적은 쪽지가 어디에 있었는지는 상관없습니다.

### 그래서 두 명령의 저장 위치는 같다

프로젝트 루트(`mlops-sample/`)에서 실행한다면:

| 실행 명령 | CWD | 저장 위치 |
| --- | --- | --- |
| `python -m training.train_sklearn` | `mlops-sample/` | `mlops-sample/artifacts/sklearn.joblib` |
| `python training/train_sklearn.py` | `mlops-sample/` | `mlops-sample/artifacts/sklearn.joblib` |

둘 다 같은 곳입니다.

만약 `cd training`으로 `training/` 폴더에 들어가서 실행하면 CWD가 `training/`이 됩니다.
그러면 Python은 `training/artifacts/`에 저장하려고 하고, 그 폴더가 없으니 에러가 납니다.

```
FileNotFoundError: [Errno 2] No such file or directory: 'artifacts/sklearn.joblib'
```

---

## 2. 진짜 이유: import

### 스크립트 맨 위의 import

```python
from training.data import collect_data
from api.predictors import rule
```

이 두 줄은 다른 폴더에 있는 코드를 가져옵니다.

- `training.data` → `training/data.py`
- `api.predictors` → `api/predictors/` 폴더

### Python은 정해진 폴더 목록에서만 찾는다

`import training`이라고 쓰면 Python은 컴퓨터 전체를 뒤지지 않습니다.
**`sys.path`라는 폴더 목록**을 순서대로 확인해서 그 안에 `training`이 있는지만 봅니다.

목록의 첫 번째 칸(`sys.path[0]`)은 **실행 방법에 따라 달라집니다.**

| 실행 방법 | `sys.path[0]`에 들어가는 폴더 |
| --- | --- |
| `python training/train_sklearn.py` (파일 경로) | 스크립트가 있는 폴더 → `mlops-sample/training/` |
| `python -m training.train_sklearn` (모듈 이름) | 현재 작업 디렉터리 → `mlops-sample/` |

### 파일 경로로 실행하면

Python은 `mlops-sample/training/` 안에서 `training`과 `api`를 찾습니다.

```
mlops-sample/training/
├── data.py
└── train_sklearn.py
```

이 안에는 `training`이라는 폴더도 `api`라는 폴더도 없습니다. 그래서 import가 실패합니다.

```
ModuleNotFoundError: No module named 'training'
```

### `-m`으로 실행하면

Python은 `mlops-sample/` 안에서 찾습니다.

```
mlops-sample/
├── api/            ← 여기서 api 를 찾음
├── artifacts/
└── training/       ← 여기서 training 을 찾음
```

두 폴더가 모두 있으니 import가 성공합니다.

### `-m` 사용법

- `-m` 뒤에는 **파일 경로가 아니라 모듈 이름**을 씁니다.
- 폴더 구분자 `/` 대신 점 `.`을 쓰고, 확장자 `.py`는 빼고 씁니다.

```
training/train_sklearn.py   →   training.train_sklearn
```

---

## 3. 정리

| 질문 | 답 |
| --- | --- |
| 결과 파일 위치가 달라서 `-m`을 쓰나? | 아니요. 상대 경로는 CWD 기준이라 두 방식 모두 같은 곳에 저장됩니다. |
| 그럼 왜 `-m`을 쓰나? | `from training...`, `from api...` import가 성공하려면 프로젝트 루트가 `sys.path`에 있어야 하기 때문입니다. |
| 꼭 지킬 것 | 프로젝트 루트(`mlops-sample/`)에서 실행합니다. |

---

## 4. 더 알아보기: 어디서 실행해도 같은 곳에 저장하기

실행 위치와 관계없이 항상 `mlops-sample/artifacts/`에 저장하고 싶다면,
CWD 대신 **스크립트 파일의 위치**를 기준으로 경로를 만들면 됩니다.

```python
from pathlib import Path

# __file__ = 지금 실행 중인 이 파일(train_sklearn.py)의 경로
# .parent        → training/
# .parent.parent → mlops-sample/
ARTIFACTS = Path(__file__).resolve().parent.parent / "artifacts"

joblib.dump(model, ARTIFACTS / "sklearn.joblib")
```

이렇게 바꿔도 import 문제는 그대로입니다. 그러니 `-m`으로 실행하는 습관은 계속 유지하세요.
