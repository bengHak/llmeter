# Curl Install + GitHub Releases Design (Approach A)

## 목차

1. 목표
2. 범위와 비범위
3. 아키텍처와 사용자 흐름
4. 릴리즈 아티팩트 규약
5. GitHub Actions 릴리즈 CI
6. install.sh 동작
7. 보안과 무결성
8. 오류 처리
9. README와 문서 UX
10. 성공 기준
11. 후속 확장 (비범위)

## 1. 목표

Rust 툴체인 없이 llmeter를 설치할 수 있게 한다. 사용자는 다음 원라이너로 최신 릴리즈 바이너리를 받는다.

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh | sh
```

배포는 GitHub Releases에 태그(`v*`) 트리거로 올린 prebuilt 바이너리를 사용한다. 소스는 계속 공개하며, 개발·기여 경로는 `cargo build --release`를 유지한다.

## 2. 범위와 비범위

### 포함

- `vX.Y.Z` 태그 push 시 크로스 플랫폼 릴리즈 빌드
- 타깃 플랫폼:
  - `aarch64-apple-darwin` (macOS arm64)
  - `x86_64-unknown-linux-gnu` (Linux x86_64)
  - `aarch64-unknown-linux-gnu` (Linux arm64)
- tarball + `SHA256SUMS` 아티팩트 규약
- `scripts/install.sh`: OS/arch 감지, latest 또는 버전 pin, SHA256 검증, `~/.local/bin` 설치
- README 설치 섹션을 curl|sh 1차 UX + cargo 개발용으로 갱신
- `Cargo.toml` version과 태그 version 일치 검사

### 제외 (non-goals)

- Windows 설치 경로 또는 Windows 바이너리
- macOS code signing / notarization / Gatekeeper 완전 제거
- Homebrew, cargo-binstall, cargo-dist
- 자동 업데이트 데몬 또는 in-app updater
- 별도 CDN / 자체 도메인 호스팅
- llmeter 런타임·TUI 동작 변경
- 이 스펙만으로 실제 프로덕션 태그 게시 의무 (구현 후 첫 태그는 별도 작업)

## 3. 아키텍처와 사용자 흐름

```text
개발자: Cargo.toml version 맞춤 → git tag v0.1.0 → git push --tags
        │
        ▼
GitHub Actions (.github/workflows/release.yml)
  ├─ matrix builds (3 targets)
  ├─ package llmeter-<ver>-<target>.tar.gz
  ├─ collect SHA256SUMS (all targets succeed first)
  └─ upload GitHub Release assets for that tag

사용자: curl -fsSL .../scripts/install.sh | sh
        │
        ▼
install.sh
  ├─ detect OS/arch → target triple
  ├─ resolve version (LLMETER_VERSION or latest)
  ├─ download tarball + SHA256SUMS from GitHub Releases
  ├─ verify checksum
  ├─ install to ${INSTALL_DIR:-$HOME/.local/bin}/llmeter
  └─ print PATH hint + version
```

**원칙**

- install 스크립트는 **항상 repo의 `master` 브랜치 raw URL**에서 받는다. 스크립트 갱신과 바이너리 버전을 분리한다.
- 바이너리는 **GitHub Releases asset**에서만 받는다. raw git blob에 바이너리를 올리지 않는다.
- 기본 설치 경로는 `$HOME/.local/bin`이다. `INSTALL_DIR`로 재정의 가능하다.
- `sudo`를 요구하지 않는다. 시스템 경로(`/usr/local/bin`)는 기본값이 아니다.

## 4. 릴리즈 아티팩트 규약

### 버전

- Git 태그: `v` + semver (`v0.1.0`)
- 아티팩트 파일명과 문서의 version 문자열: 선행 `v` 없음 (`0.1.0`)
- `Cargo.toml` `[package].version`은 태그의 `v` 제거 값과 **정확히 일치**해야 한다. CI가 불일치 시 fail한다.

### 파일 이름

```text
llmeter-<version>-<target>.tar.gz
예: llmeter-0.1.0-aarch64-apple-darwin.tar.gz
    llmeter-0.1.0-x86_64-unknown-linux-gnu.tar.gz
    llmeter-0.1.0-aarch64-unknown-linux-gnu.tar.gz
```

### tarball 내용

- 단일 파일 `llmeter` (루트에 바이너리만; 상위 디렉터리 래핑 없음)
- 실행 비트 설정

### 체크섬

- Release에 `SHA256SUMS` 단일 파일 업로드
- 한 줄 형식 (GNU `sha256sum` / macOS `shasum -a 256` 호환):

```text
<64-hex>  llmeter-0.1.0-aarch64-apple-darwin.tar.gz
```

- 두 공백 구분 형식 사용
- install.sh는 대상 파일명 줄만 파싱한다

### Release 메타

- Release name: `llmeter <version>` (예: `llmeter 0.1.0`)
- prerelease: 태그에 pre-release 라벨이 있을 때만 (초기 구현은 stable 태그 가정; `v0.1.0-rc.1` 등은 선택적 후속)
- 본문: 자동 생성 최소 changelog 또는 빈 본문 허용; 수동 편집 가능

## 5. GitHub Actions 릴리즈 CI

### 트리거

```yaml
on:
  push:
    tags:
      - 'v*'
```

- PR 및 일반 branch push에서는 릴리즈 워크플로를 실행하지 않는다.
- `workflow_dispatch`는 초기 범위 밖 (필요 시 후속).

### 잡 구조

1. **prepare** (또는 각 build job 시작 시)
   - checkout
   - 태그에서 version 추출 (`GITHUB_REF_NAME`에서 선행 `v` 제거)
   - `Cargo.toml` version과 비교; 불일치 시 fail

2. **build** (matrix)
   | target | runner | 비고 |
   |---|---|---|
   | `aarch64-apple-darwin` | `macos-14` | native |
   | `x86_64-unknown-linux-gnu` | `ubuntu-latest` | native |
   | `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross` 또는 동등한 크로스 툴체인 |

   - `rust-toolchain.toml` 준수
   - `cargo build --release --target <triple>`
   - 바이너리 경로: `target/<triple>/release/llmeter`
   - tarball 생성 후 artifact upload (job 산출물)

3. **publish** (needs: 모든 build 성공)
   - 모든 tarball 수집
   - `SHA256SUMS` 생성
   - `softprops/action-gh-release` (또는 동등)로 tag Release 생성/업데이트 및 asset 첨부
   - **partial upload 금지**: matrix 중 하나라도 실패하면 publish job 미실행

### 권한

- `contents: write` (Release 생성)
- 기본 `GITHUB_TOKEN`으로 충분; 별도 PAT 불필요

### 빌드 프로필

- 기존 `Cargo.toml` `[profile.release]` (`lto = "thin"`, `strip = "symbols"`) 사용
- Linux는 동적 glibc 링크를 허용한다 (완전 static musl은 비범위). 최소 glibc 버전은 runner 기본값에 따른다.

## 6. install.sh 동작

### 위치

- 저장소: `scripts/install.sh`
- 사용자 원라이너 URL:

```text
https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh
```

### 환경 변수

| 변수 | 기본 | 의미 |
|---|---|---|
| `LLMETER_VERSION` | (비움 = latest) | 설치할 버전 (`0.1.0` 또는 `v0.1.0` 모두 허용; 내부적으로 `v` 제거) |
| `INSTALL_DIR` | `$HOME/.local/bin` | 바이너리 설치 디렉터리 |
| `LLMETER_REPO` | `bengHak/llmeter` | 테스트/포크용 override |
| `GITHUB_TOKEN` | (선택) | rate limit 완화용; 공개 다운로드에는 불필요 |

### 단계

1. `set -euo pipefail` (또는 동등한 안전한 기본값)
2. OS 감지: `uname -s` → `Darwin` | `Linux` only; 그 외 fail with message
3. Arch 감지: `uname -m` → `arm64`/`aarch64` → `aarch64`, `x86_64`/`amd64` → `x86_64`; 그 외 fail
4. target triple 매핑:
   - Darwin + aarch64 → `aarch64-apple-darwin`
   - Darwin + x86_64 → **비지원** (1차 범위 밖; 명확한 오류 메시지)
   - Linux + x86_64 → `x86_64-unknown-linux-gnu`
   - Linux + aarch64 → `aarch64-unknown-linux-gnu`
5. version 결정:
   - `LLMETER_VERSION` 설정 시 정규화 후 사용
   - 미설정 시 GitHub API `GET /repos/{repo}/releases/latest`의 `tag_name` 사용
6. asset URL:
   - `https://github.com/{repo}/releases/download/v{version}/llmeter-{version}-{target}.tar.gz`
   - `SHA256SUMS`: 동일 release 디렉터리
7. 임시 디렉터리에 다운로드 (`mktemp -d`)
8. SHA256 검증 (macOS: `shasum -a 256`, Linux: `sha256sum`)
9. `tar -xzf` 후 `llmeter` 존재·실행 가능 확인
10. `mkdir -p "$INSTALL_DIR"` 후 atomic에 가깝게 설치 (`mv`/`install`)
11. `chmod +x`
12. `"$INSTALL_DIR/llmeter" --version` (또는 `-V`/clap default) 출력; 실패해도 설치 자체는 완료 메시지로 안내 가능하나 version 확인 실패는 경고
13. `INSTALL_DIR`이 PATH에 없으면 export 예시 출력

### 의존 명령

- 필수: `curl` 또는 `wget` 중 하나, `tar`, `uname`, `mktemp`, `shasum` 또는 `sha256sum`
- shell: POSIX `sh` 호환 목표 (bash 전용 기능 최소화)

### 버전 pin 예

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh | LLMETER_VERSION=0.1.0 sh
```

## 7. 보안과 무결성

- **HTTPS only** for script and assets
- **Checksum required**: SHA256 검증 실패 시 설치 중단, 부분 파일 삭제
- install.sh는 원격 코드를 pipe로 실행하는 관례를 따른다. README에 다음을 명시한다:
  - raw 스크립트를 먼저 검토하는 방법 (`curl ... -o install.sh && less install.sh`)
  - 바이너리는 checksum으로 검증됨
- **서명/notarization 없음** (1차). macOS에서 quarantine 속성이 붙을 수 있음; 필요 시 사용자가 `xattr -d com.apple.quarantine` 등을 적용. 문서에 제한으로 기록.
- 스크립트는 사용자 홈 밖 시스템 경로를 기본으로 쓰지 않으며 sudo를 호출하지 않는다.
- `GITHUB_TOKEN`이 있으면 Authorization 헤더로 API/asset 요청 가능하나 토큰을 로그에 찍지 않는다.

## 8. 오류 처리

| 상황 | 동작 |
|---|---|
| 미지원 OS/arch | non-zero exit, 지원 목록 출력 |
| network/HTTP 실패 | non-zero, URL 힌트 |
| latest release 없음 | non-zero, 태그 릴리즈 필요 안내 |
| asset 404 | non-zero, 버전·플랫폼 확인 안내 |
| checksum mismatch | non-zero, 임시 파일 삭제 |
| `INSTALL_DIR` 생성/쓰기 실패 | non-zero, 권한·경로 안내 |
| 디스크 부족 등 tar 실패 | non-zero |

모든 오류 메시지는 stderr로 보낸다. 성공 시 설치 경로와 버전을 stdout에 요약한다.

## 9. README와 문서 UX

README §2 「설치와 실행」을 다음 순서로 재구성한다.

1. **권장 (사용자):** curl|sh 원라이너
2. **버전 고정:** `LLMETER_VERSION=…`
3. **요구사항:** 지원 OS/arch, `~/.local/bin` PATH
4. **개발/소스 빌드:** Rust 1.85+, `cargo build --release`
5. **설치 후 실행:** 기존 사용법 유지

스펙/플랜 문서는 `docs/superpowers/`에 유지한다. install 스크립트 자체 주석에 지원 타깃과 환경 변수를 짧게 적는다.

## 10. 성공 기준

- 태그 `vX.Y.Z` push 후 세 타깃 tarball과 `SHA256SUMS`가 해당 Release에 존재한다.
- 지원 플랫폼에서 원라이너가 `~/.local/bin/llmeter`를 설치하고 실행 가능하다.
- `LLMETER_VERSION`으로 과거 릴리즈를 설치할 수 있다.
- checksum 실패 시 설치되지 않는다.
- README만 보고 cargo 없이 설치할 수 있다.
- Windows / notarization / cargo-dist는 문서상 non-goal로 명시된다.

## 11. 후속 확장 (비범위)

- macOS x86_64 (Intel) 타깃
- musl static Linux
- cosign/sigstore 서명
- Homebrew formula
- `llmeter update` 서브커맨드
- install.sh를 release asset으로도 고정 버전 핀
- draft release + 수동 publish 게이트
