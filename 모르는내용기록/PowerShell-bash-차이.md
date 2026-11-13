# PowerShell에서 bash 명령이 안 되는 이유

> 작성: 2026-11-13 16:40

문서나 블로그에 나온 명령을 Windows 터미널(PowerShell)에 그대로 붙여 넣으면 에러가 날 때가 많습니다.
대부분의 문서가 **bash**(리눅스·맥에서 쓰는 셸) 기준으로 쓰여 있기 때문입니다.

> **셸(shell)이란?** 내가 입력한 명령을 읽고 실행해 주는 프로그램입니다.
> bash와 PowerShell은 둘 다 셸이지만 **문법이 다릅니다.**
> 같은 내용을 영어와 한국어로 말하는 것과 비슷합니다. 뜻은 같아도 쓰는 방식이 다릅니다.

이 문서는 실제로 겪은 두 가지 에러를 정리한 것입니다.

---

## 1. 환경변수를 설정하면서 실행하기

### 겪은 에러

```powershell
MODEL_IMPL=rule uv run uvicorn api.app:app --port 8080 --reload
```

```
MODEL_IMPL=rule : 'MODEL_IMPL=rule' 용어가 cmdlet, 함수, 스크립트 파일 또는
실행할 수 있는 프로그램 이름으로 인식되지 않습니다.
```

### 왜 안 될까?

`변수=값 명령`은 bash 문법입니다. "이 명령을 실행하는 동안만 환경변수를 설정하라"는 뜻입니다.

PowerShell은 이 문법을 모릅니다. 그래서 `MODEL_IMPL=rule` 전체를 **실행할 프로그램 이름**으로 읽습니다.
그런 이름의 프로그램은 없으니 "인식되지 않습니다" 에러가 납니다.

### PowerShell에서는 이렇게

```powershell
$env:MODEL_IMPL = "rule"
uv run uvicorn api.app:app --port 8080 --reload
```

- `$env:이름 = "값"`이 PowerShell에서 환경변수를 설정하는 방법입니다.
- 설정만 하면 **아무것도 출력되지 않습니다.** 조용히 끝나면 정상입니다.
- 값이 잘 들어갔는지 보려면 `$env:MODEL_IMPL`을 입력합니다. `rule`이 출력되면 됩니다.
- 한 줄로 쓰려면 세미콜론(`;`)으로 두 명령을 이어 씁니다.

  ```powershell
  $env:MODEL_IMPL = "rule"; uv run uvicorn api.app:app --port 8080 --reload
  ```

### bash와 다른 점: 값이 계속 남는다

| | bash `VAR=값 명령` | PowerShell `$env:VAR = "값"` |
| --- | --- | --- |
| 값이 유지되는 범위 | 그 명령 한 번만 | **터미널 창을 닫을 때까지** |

다른 값으로 바꾸려면 다시 설정합니다. 지우려면 아래 명령을 씁니다.

```powershell
Remove-Item Env:MODEL_IMPL
```

### 셸과 상관없이 쓰는 방법: `.env` 파일

프로젝트 루트에 `.env` 파일을 만들고 아래 내용을 적습니다.

```
MODEL_IMPL=rule
```

그다음 uv에게 이 파일을 읽으라고 알려주면 bash와 PowerShell 모두에서 똑같이 동작합니다.

```powershell
uv run --env-file .env uvicorn api.app:app --port 8080 --reload
```

---

## 2. curl로 API 호출하기

### 겪은 에러

```powershell
curl -X POST localhost:8080/eta -H 'Content-Type: application/json' -d '{"distance_km": 3, "pending_orders": 2}'
```

```
Invoke-WebRequest : 'Headers' 매개 변수를 바인딩할 수 없습니다.
"System.String" 유형의 "Content-Type: application/json" 값을
"System.Collections.IDictionary" 유형으로 변환할 수 없습니다.
```

### 왜 안 될까?

에러 메시지 맨 앞에 `curl`이 아니라 **`Invoke-WebRequest`**라고 적혀 있습니다.

Windows PowerShell에서 `curl`은 진짜 curl 프로그램이 아니라
PowerShell 명령인 `Invoke-WebRequest`의 **별칭(alias, 다른 이름)**입니다.
`curl`이라고 입력해도 실제로는 `Invoke-WebRequest`가 실행됩니다.

`Invoke-WebRequest`의 `-H`(Headers)는 `@{ "이름" = "값" }` 형태의 **딕셔너리**만 받습니다.
그런데 `'Content-Type: application/json'`이라는 **문자열**을 넣었으니
"문자열을 딕셔너리로 바꿀 수 없다"는 에러가 났습니다.

### 방법 1: PowerShell 명령 쓰기 (추천)

```powershell
Invoke-RestMethod -Method Post -Uri http://localhost:8080/eta -ContentType "application/json" -Body '{"distance_km": 3, "pending_orders": 2}'
```

| curl 옵션 | `Invoke-RestMethod` 옵션 |
| --- | --- |
| `-X POST` | `-Method Post` |
| `localhost:8080/eta` | `-Uri http://localhost:8080/eta` |
| `-H 'Content-Type: application/json'` | `-ContentType "application/json"` |
| `-d '...'` | `-Body '...'` |

`Invoke-RestMethod`는 JSON 응답을 표 형태로 보기 좋게 보여줍니다.

### 방법 2: 진짜 curl 쓰기 (`curl.exe`)

Windows 10 이상에는 진짜 curl이 함께 설치되어 있습니다.
이름 끝에 **`.exe`**를 붙이면 별칭이 아니라 진짜 curl이 실행됩니다.

```powershell
curl.exe -X POST localhost:8080/eta -H "Content-Type: application/json" -d '{\"distance_km\": 3, \"pending_orders\": 2}'
```

이 경우 JSON 안의 큰따옴표를 `\"`로 써야 합니다.
Windows PowerShell 5.1은 외부 프로그램(`curl.exe`)에 값을 넘길 때 큰따옴표를 지워버리기 때문입니다.
따옴표를 지우지 않으면 서버는 `{distance_km: 3, ...}`처럼 깨진 JSON을 받습니다.
이 점이 번거로워서 방법 1을 추천합니다.

---

## 3. 한눈에 보기: bash → PowerShell 바꾸기

| 하고 싶은 일 | bash (문서·블로그) | PowerShell |
| --- | --- | --- |
| 환경변수 설정 후 실행 | `VAR=값 명령` | `$env:VAR = "값"; 명령` |
| 환경변수 값 확인 | `echo $VAR` | `$env:VAR` |
| 환경변수 지우기 | `unset VAR` | `Remove-Item Env:VAR` |
| POST 요청 보내기 | `curl -X POST URL -H '...' -d '...'` | `Invoke-RestMethod -Method Post -Uri URL -ContentType "..." -Body '...'` |
| 진짜 curl 쓰기 | `curl` | `curl.exe` |

## 4. 에러 메시지에서 힌트 찾기

- **"용어가 cmdlet, 함수, 스크립트 파일 또는 실행할 수 있는 프로그램 이름으로 인식되지 않습니다"**
  → PowerShell이 명령 이름을 못 찾았다는 뜻입니다. bash 전용 문법을 쓰지 않았는지 확인합니다.
- **에러 메시지에 내가 입력하지 않은 명령 이름이 나올 때** (예: `curl`을 입력했는데 `Invoke-WebRequest`가 나옴)
  → 별칭 때문입니다. `Get-Alias curl`을 입력하면 실제로 어떤 명령으로 연결되는지 볼 수 있습니다.

## 5. 아예 bash를 쓰고 싶다면

Git for Windows를 설치하면 **Git Bash**가 함께 설치됩니다.
VS Code 터미널 오른쪽 위의 `+` 옆 화살표(`∨`)를 누르고 **Git Bash**를 고르면,
문서에 나온 bash 명령을 바꾸지 않고 그대로 쓸 수 있습니다.
