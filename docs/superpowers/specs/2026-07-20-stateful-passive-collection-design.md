# Stateful Passive Collection Design

## 목차

1. 목표
2. 범위
3. 아키텍처
4. 소스 탐색
5. 증분 수집
6. 프로세스 상관관계
7. 오류 처리와 프라이버시
8. 테스트 전략

## 1. 목표

`llmeter`를 설치하고 실행하기만 하면 지원되는 LLM CLI 프로세스와 로컬 세션 저장소를 자동으로 찾아 표시한다. TUI의 매 refresh마다 모든 세션 파일을 다시 읽지 않고, 최초 bootstrap 이후 새로 추가된 바이트만 처리한다.

## 2. 범위

이번 변경은 다음을 포함한다.

- 지원 도구의 알려진 세션 루트 자동 탐색
- 도구별 파일명 규칙으로 설정 파일과 무관한 JSON을 제외
- 소스별 offset, partial line, file identity, checkpoint 유지
- 파일 truncate·replace 감지와 parser state 초기화
- normalized journal 증분 tail
- 프로세스 발견 상태 유지 및 종료 감지
- 명확한 1:1 관계에 한한 process-only 행과 native session 행 병합
- 기존 `llmeter`, `llmeter once`, `llmeter json` 인터페이스 유지

이번 변경에서 제외한다.

- 다른 CLI의 설정 파일을 자동 수정하는 hook 설치
- OpenCode SQLite 직접 조회
- filesystem watcher 의존성
- Windows 전용 프로세스 API 교체
- 불확실한 다대다 프로세스·세션 병합

## 3. 아키텍처

`runtime::load_snapshot`의 외부 시그니처는 유지한다. 내부적으로 data directory별 `LiveCollector`를 process-global cache에 보관한다.

```text
TUI refresh
    │
    ▼
load_snapshot(data_dir)
    │
    ▼
LiveCollector cache
    ├── SourceCatalog: 알려진 root와 도구별 matcher 재탐색
    ├── SourceCursor*: native JSONL offset tail
    ├── JournalCursor: normalized journal offset tail
    ├── ProcessIndex: process 등장·종료 상태
    └── Aggregator: 누적 session snapshot
```

기존 `runtime.rs`의 ingest, SSE, wrapper, replay 구현은 `runtime_live.rs`의 private legacy module로 포함하고 그대로 재사용한다. 새 `load_snapshot`만 stateful collector로 위임한다.

## 4. 소스 탐색

각 `ToolDescriptor`의 기존 session root를 재사용하되 파일은 다음 규칙으로 제한한다.

| 도구 | passive source 규칙 |
|---|---|
| Pi | `.jsonl` |
| Factory Droid | 파일명에 `session`, `event`, `stream`이 있는 `.jsonl` |
| Gemini CLI | 파일명에 `session`, `event`, `telemetry`가 있는 `.jsonl` |
| Claude Code | `.jsonl` |
| Codex CLI | `rollout-*.jsonl` |
| OpenCode | 파일명에 `session`, `event`, `message`가 있는 `.jsonl` |
| Qwen Code | `.jsonl` |
| Kiro CLI | `.jsonl` |
| Grok Build | `updates.jsonl` |

심볼릭 링크는 따라가지 않는다. 후보는 도구별 수정 시각 내림차순으로 제한한 뒤 전역 수정 시각으로 다시 정렬한다. 한 도구의 오래된 세션이 전체 슬롯을 독점하지 않도록 per-tool limit와 total limit을 별도로 둔다.

## 5. 증분 수집

각 `SourceCursor`는 다음 상태를 유지한다.

```text
path
tool
offset
pending partial line
last 64-byte checkpoint
file identity
adapter parser state
```

최초 발견 시 파일이 4 MiB보다 크면 마지막 4 MiB에서 시작하고 첫 불완전 line을 버린다. 이후에는 offset 이후의 바이트만 읽는다. inode/device 또는 파일 생성 정보가 달라지거나 길이가 줄거나 checkpoint가 바뀌면 rotation/truncate로 보고 cursor와 adapter state를 초기화한다.

normalized journal도 같은 line cursor를 사용하되 `TelemetryEvent`를 직접 deserialize한다. journal의 UUID는 bounded deduper로 중복 적용을 방지한다.

## 6. 프로세스 상관관계

프로세스는 `(tool, pid)` 인덱스와 `now - elapsed`로 계산한 근사 시작 시각을 함께 추적한다. 같은 PID라도 command 또는 시작 시각이 달라지면 PID 재사용으로 보고 이전 process session을 종료한 뒤 새 session ID를 만든다. 사라진 process에는 `SessionEnded`를 적용한다.

snapshot 후처리에서 다음 순서로 process-only 행을 native session에 병합한다.

1. 같은 tool과 같은 PID
2. 같은 tool에서 process 1개와 native session 1개뿐인 경우

그 외에는 잘못된 병합을 피하기 위해 별도 행으로 유지한다. 병합 시 native session의 모델·상태·메트릭을 보존하고 PID, 시작·마지막 관측 시각만 보강한다.

## 7. 오류 처리와 프라이버시

한 파일의 open·parse 실패는 다른 source를 중단하지 않는다. 오류 수는 collector 내부 통계로 누적한다. 원문 prompt, response, thought, tool input/output은 기존 adapter가 수량 이벤트로 변환한 뒤에만 aggregator에 전달된다.

자동 passive mode는 외부 네트워크에 연결하지 않고 다른 도구의 설정을 수정하지 않는다. `setup`은 계속 선택 기능이다.

## 8. 테스트 전략

- matcher가 Codex rollout과 Grok updates만 선택하고 설정 JSON을 제외하는지 검증
- 동일 파일을 두 번 refresh해도 event가 재적용되지 않는지 검증
- append된 complete line만 한 번 처리하는지 검증
- partial line이 newline 전에는 처리되지 않는지 검증
- truncate 후 parser state가 초기화되는지 검증
- normalized journal UUID 중복이 걸러지는지 검증
- 1:1 process/native correlation이 PID를 병합하고 ambiguous case는 유지하는지 검증
- 기존 adapter, aggregate, journal, setup 테스트 전체 회귀 실행
