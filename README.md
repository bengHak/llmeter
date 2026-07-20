# llmeter

실행 중인 LLM 코딩 에이전트 세션의 TTFT, 현재/평균 출력 속도, 도구 실행 시간, stall 상태를 한 화면에서 비교하는 Rust 기반 TUI입니다.

## 목차

1. 현재 구현 범위
2. 설치와 실행
3. 측정값 해석
4. 도구별 연결
5. CLI 명령
6. 아키텍처
7. 프라이버시
8. 알려진 제한

## 1. 현재 구현 범위

Phase 1과 Phase 2의 8개 도구가 하나의 정규화 이벤트 모델을 사용합니다.

| 단계 | 도구 | 입력 표면 | 자동 탐색 |
|---|---|---|---|
| 1 | Pi | RPC/session JSONL | `~/.pi/agent/sessions/**/*.jsonl` |
| 1 | Factory Droid | stream JSON/JSON-RPC, hook | 프로세스 |
| 1 | Gemini CLI | hook, telemetry JSON | 프로세스 |
| 1 | Claude Code | hook, transcript JSONL | `~/.claude/projects/**/*.jsonl` |
| 2 | Codex CLI | rollout JSONL | `~/.codex/sessions/**/rollout-*.jsonl` |
| 2 | OpenCode | server/SSE/run JSON | 프로세스 |
| 2 | Qwen Code | hook, telemetry JSON, daemon event | 프로세스 |
| 2 | Kiro CLI | hook, ACP JSON-RPC | 프로세스 |

자동 프로세스 탐지는 세션의 존재, PID, 프로젝트 경로를 추정합니다. TTFT와 출력 속도는 구조화 이벤트 또는 세션 파일이 연결된 경우에만 계산됩니다.

## 2. 설치와 실행

요구사항은 Rust 1.85 이상입니다.

```bash
cargo build --release
./target/release/llmeter
```

실행 중인 프로세스와 알려진 세션 파일을 한 번만 조회해 JSON으로 출력합니다.

```bash
llmeter --once --json
```

특정 JSONL 소스를 명시적으로 연결합니다.

```bash
llmeter --source codex:$HOME/.codex/sessions/2026/07/20/rollout-demo.jsonl
llmeter --source pi:/tmp/pi-session.jsonl \
        --source qwen:/tmp/qwen-telemetry.jsonl
```

과거 파일을 재생합니다.

```bash
llmeter replay /tmp/pi-session.jsonl --tool pi --json
llmeter replay examples/normalized-session.jsonl --json
```

## 3. 측정값 해석

- `TTFT`: 요청 제출부터 첫 출력 이벤트까지의 시간
- `NOW`: 최근 2초 구간의 출력 처리량
- `AVG`: 현재 또는 최근 턴의 첫 출력부터 마지막 출력까지 평균 처리량
- `TOOL`: 도구 실행 누적 시간
- `STALL`: 출력 중이지만 기본 2초 이상 새 출력이 없었던 시간

측정 신뢰도는 메트릭마다 별도로 표시합니다.

| 표시 | 등급 | 의미 |
|---|---|---|
| `●` | Exact | 직접 계측된 타임스탬프 또는 토큰 수 |
| `◐` | Derived | 정확한 값을 조합해 계산 |
| `~` | Estimated | 문자량, 프로세스 또는 간접 이벤트 기반 추정 |
| `-` | Unknown | 사용 가능한 데이터 없음 |

토큰 수가 없는 이벤트는 `tok/s`로 위장하지 않고 `ch/s` 또는 `B/s`로 표시합니다.

## 4. 도구별 연결

현재 환경에서 사용할 수 있는 연결 방법은 다음 명령으로 확인할 수 있습니다.

```bash
llmeter adapters
llmeter setup pi
llmeter setup claude
llmeter setup codex
```

### Pi

Pi session JSONL은 자동 탐색됩니다. 명시적 파일도 연결할 수 있습니다.

```bash
llmeter --source pi:/path/to/session.jsonl
```

### Factory Droid

구조화 출력을 정규화 journal로 변환합니다. `ingest`는 표준 입력을 줄 단위로 처리하고 각 이벤트 묶음을 즉시 flush하므로, 별도 터미널에서 실행 중인 `llmeter`에 스트림 종료 전부터 반영됩니다.

```bash
droid exec -o stream-json <args> \
  | llmeter ingest --tool droid --input -
```

### Gemini CLI와 Qwen Code

hook 또는 telemetry exporter가 append하는 JSONL을 연결합니다.

```bash
llmeter --source gemini:/tmp/gemini-events.jsonl
llmeter --source qwen:/tmp/qwen-events.jsonl
```

hook command의 stdin을 로컬 journal로 정규화할 수도 있습니다.

```bash
llmeter hook --tool gemini
llmeter hook --tool qwen
```

### Claude Code

transcript JSONL은 알려진 기본 루트에서 탐색됩니다. lifecycle hook은 다음 command sink를 사용할 수 있습니다.

```bash
llmeter hook --tool claude
```

### Codex CLI

rollout JSONL은 기본 세션 루트에서 자동 탐색됩니다.

```bash
llmeter --source codex:/path/to/rollout.jsonl
```

### OpenCode

server event, SSE 또는 `run --format json` 결과를 JSONL로 저장해 연결합니다.

```bash
llmeter --source opencode:/tmp/opencode-events.jsonl
```

### Kiro CLI

hook 또는 ACP wire JSONL을 연결합니다.

```bash
llmeter --source kiro:/tmp/kiro-events.jsonl
llmeter hook --tool kiro
```

## 5. CLI 명령

```text
llmeter [watch]
llmeter --once --json
llmeter replay <FILE> [--tool <TOOL>] [--json]
llmeter ingest --tool <TOOL> --input <FILE|-> [--output <JOURNAL>]
llmeter hook --tool <TOOL> [--output <JOURNAL>]
llmeter doctor [--json]
llmeter setup <TOOL>
llmeter adapters [--json]
```

주요 전역 옵션:

```text
--source <TOOL:PATH>       여러 번 지정 가능
--journal <PATH>           정규화 journal 경로
--no-auto-discover         프로세스와 기본 세션 루트 탐색 비활성화
--refresh-ms <MS>          TUI 갱신 주기, 기본 250ms
--process-scan-ms <MS>     프로세스·세션 파일 재탐색 주기, 기본 2000ms
--stall-threshold-ms <MS>  stall 임계값, 기본 2000ms
```

TUI 키:

```text
j/k 또는 ↑/↓  세션 선택
p              일시 정지/재개
r              즉시 갱신
q 또는 Esc     종료
```

## 6. 아키텍처

```text
process discovery ─┐
native JSONL tail ─┼─> tool adapter ─> TelemetryEvent ─> SessionAggregator
hook journal ──────┘                                      │
                                                          ├─> TUI
                                                          └─> JSON snapshot
```

코드는 단일 Rust crate 안에서 책임별 모듈로 분리됩니다.

- `src/model.rs`: 정규화 이벤트와 세션 snapshot 모델
- `src/adapters/`: 8개 도구 parser
- `src/discovery.rs`: 프로세스와 native session 탐색
- `src/aggregate/`: TTFT/TPS/stall 상태 계산
- `src/runtime.rs`: tailer, journal, 수집 루프
- `src/tui.rs`: Ratatui 대시보드

도구별 parser는 메트릭을 직접 계산하지 않습니다. 모든 parser는 `TelemetryEvent`만 만들고, 하나의 aggregator가 시간과 상태를 결정합니다.

## 7. 프라이버시

기본 journal에는 다음 정보만 저장됩니다.

- 세션·턴 식별자
- 이벤트 종류와 타임스탬프
- 토큰·문자·바이트 변화량
- 모델·프로젝트·PID 메타데이터
- 도구 이름과 call ID
- 오류 발생 여부(공급자 오류 원문은 제거)

프롬프트, 응답 본문, 도구 인자, API 키와 원문 오류 메시지는 정규화 journal에 저장하지 않습니다. Unix에서는 journal 디렉터리와 파일을 각각 `0700`, `0600` 권한으로 만들며, 기존 journal 권한도 append 시 `0600`으로 축소합니다. 원본 CLI transcript나 rollout 파일 자체의 보안은 해당 도구의 설정을 따릅니다.

## 8. 알려진 제한

- 외부 CLI 바이너리가 설치된 실제 사용자 환경에 대한 live smoke test는 포함하지 않았습니다. parser는 공식 이벤트 표면을 바탕으로 한 fixture와 replay 테스트로 검증했습니다.
- OpenCode SSE, Qwen daemon SSE, Gemini OTLP를 직접 수신하는 네트워크 서버는 아직 없습니다. 현재는 JSONL로 기록된 이벤트를 tail하거나 구조화 stdout을 `ingest`로 스트리밍합니다.
- 프로세스 발견과 세션 파일의 완전한 상관관계는 아직 구현하지 않아 같은 작업이 process-only 행과 native-session 행으로 함께 보일 수 있습니다.
- Codex, Claude 등 내부 session 파일 schema가 변경되면 해당 parser fixture와 mapping을 갱신해야 합니다.
- 동적 플러그인 ABI, 원격 호스트, Kubernetes, 웹 대시보드는 Phase 1–2 범위 밖입니다.
