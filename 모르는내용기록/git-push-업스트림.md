# `git push`가 안 될 때: 업스트림(upstream)

> 작성: 2026-11-13 17:22

`git push`를 실행하니 아래 메시지가 나왔습니다.

```
fatal: The current branch main has no upstream branch.
To push the current branch and set the remote as upstream, use

    git push --set-upstream origin main
```

처음 보면 "올릴 저장소가 없다"는 말처럼 읽힙니다.
하지만 **저장소는 이미 연결되어 있었습니다.** 빠진 것은 **업스트림 설정**이었습니다.

---

## 1. 먼저 용어 세 개

| 용어 | 뜻 | 이 프로젝트에서 |
| --- | --- | --- |
| **원격 저장소 (remote)** | 인터넷(GitHub 등)에 있는 저장소 | GitHub의 `eta-mini` 저장소 |
| **`origin`** | 원격 저장소에 붙인 **별명** (기본 이름) | `origin` = `git@github.com:sig90145-alt/eta-mini.git` |
| **업스트림 (upstream)** | 로컬 브랜치가 짝지어진 원격 브랜치 | 로컬 `main` ↔ `origin`의 `main` (아직 짝이 없었음) |

> 비유: `origin`은 **휴대폰 주소록에 저장한 친구 이름**이고,
> 업스트림은 **"이 대화방은 그 친구와의 1:1 대화방"이라고 정해 두는 것**입니다.
> 친구 번호(저장소)는 저장되어 있어도, 어느 대화방으로 보낼지 정하지 않으면
> 메시지를 보낼 곳이 정해지지 않습니다.

---

## 2. 저장소가 연결되어 있는지 확인하기

```powershell
git remote -v
```

```
origin  git@github.com:sig90145-alt/eta-mini.git (fetch)
origin  git@github.com:sig90145-alt/eta-mini.git (push)
```

이렇게 나오면 원격 저장소는 연결된 것입니다.
아무것도 안 나와야 정말 "저장소가 없는" 상태입니다. (→ 5번 참고)

## 3. 업스트림이 있는지 확인하기

```powershell
git branch -vv
```

```
* main  65bee7a 모르는내용기록에 초보자용 정리 문서 3개 추가
```

업스트림이 있으면 커밋 번호 뒤에 `[origin/main]`처럼 대괄호가 붙습니다.

```
* main  65bee7a [origin/main: ahead 3] 모르는내용기록에 ...
```

대괄호가 없으면 업스트림이 없는 것입니다.

---

## 4. 해결: 처음 한 번만 `-u`

```powershell
git push -u origin main
```

| 부분 | 뜻 |
| --- | --- |
| `git push` | 내 커밋을 원격 저장소로 올린다 |
| `-u` (`--set-upstream`의 줄임) | 이번에 올리는 곳을 **업스트림으로 기억**한다 |
| `origin` | 어느 원격 저장소로? → `origin` |
| `main` | 어느 브랜치를? → `main` |

한 번 이렇게 실행하면 다음부터는 아래처럼 짧게 써도 됩니다.

```powershell
git push
git pull
```

Git이 "로컬 `main`은 `origin/main`과 짝"이라는 것을 기억하기 때문입니다.

---

## 5. 참고: 정말 원격 저장소가 없을 때

`git remote -v`에 아무것도 안 나온다면, GitHub에서 저장소를 먼저 만든 뒤 연결합니다.

```powershell
git remote add origin git@github.com:<계정>/<저장소>.git
git push -u origin main
```

---

## 6. 참고: 이 프로젝트에서 push하면 무슨 일이 생길까?

[.github/workflows/ci.yml](../.github/workflows/ci.yml)에 GitHub Actions 설정이 있습니다.

| 무엇을 push했나 | 실행되는 것 |
| --- | --- |
| `main` 브랜치 | 테스트만 |
| `v`로 시작하는 태그 (예: `v0.3`) | 테스트 → 이미지 배포 → Render 배포 |

그래서 `main`에 push해도 서버에 배포되지는 않습니다.
배포는 태그를 push할 때만 일어납니다.

---

## 7. 한눈에 보기

| 상황 | 확인 명령 | 해결 |
| --- | --- | --- |
| 원격 저장소가 없음 | `git remote -v` 결과가 비어 있음 | `git remote add origin <주소>` |
| 업스트림이 없음 | `git branch -vv`에 `[origin/...]`이 없음 | `git push -u origin main` (처음 한 번) |
| 둘 다 있음 | | `git push` |
