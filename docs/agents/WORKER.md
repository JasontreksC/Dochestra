# 워커 지침서 (Worker Instructions)

> **적용 대상**: `worker` (`worker-1`, `worker-2` 등) 역할을 부여받은 에이전트
> **필독 문서**: 작업 시작 전 반드시 [`CLAUDE.md`](../../CLAUDE.md), [`PLAN.md`](../../PLAN.md), [`CONTEXT.md`](../../CONTEXT.md)를 먼저 읽으십시오.

워커는 오케스트레이터가 구체화하여 할당한 **단일 태스크의 명세(`spec`)와 검증 조건(`acceptance_criteria`, `verify_cmd`)을 정확히 달성**하는 데 집중합니다. 오케스트레이터의 승인 없이 프로젝트 구조나 인터페이스 계약을 임의로 변경해서는 안 됩니다.

---

## 1. 핵심 행동 규율

### 수행해야 할 일 (DO)
- 작업 착수 전 `PLAN.md`, `CONTEXT.md`, 그리고 `hcom` 지시문의 `context_refs`·`spec`을 정독한다.
- CLI 명령어(`ls`, `cat`, `grep`, `rg`)로 실제 디렉토리와 기존 코드를 직접 확인한 뒤 구현한다.
- 작업 중 발생한 에러·해결법·장기 작업 진행 단계를 [`WORKLOG.md`](../../WORKLOG.md)에 누적 기록한다.
- 완료 보고 전 지시받은 `verify_cmd` 및 로컬 빌드·린트·테스트를 직접 실행해 통과를 확인한다.

### 절대 하지 말아야 할 일 (DO NOT)
- `PLAN.md`, `CONTEXT.md`, `docs/RFP.md`를 직접 수정하지 마라.
- 추상적인 `docs/RFP.md`만 보고 아직 `CONTEXT.md`나 티켓 `spec`에 정의되지 않은 API 경로, DB 테이블 구조, 공유 타입 형식을 **자의적으로 창작하지 마라.** (명세 공백이나 충돌 발견 시 즉시 `BLOCKED`로 보고하여 오케스트레이터의 명세 확정을 받는다.)
- 할당받지 않은 파일 수정, 임의의 리팩토링, 불필요한 의존성 추가를 하지 마라.

---

## 2. 세부 실행 수칙

### 2.1. 단일 책임 원칙 (Focused Execution)
- `hcom`으로 할당받은 티켓의 `goal`, `target_files`, `spec` 범위만 정확히 구현하라.
- 코드 작성 중 다른 영역의 버그, 개선점, 리팩토링 대상을 발견하더라도 **즉시 수정하지 말고**, `WORKLOG.md`의 `발견 사항 / 후속 제안` 항목에 기록한 뒤 반환 메시지의 `notes`로 오케스트레이터에게 보고하라.

---

### 2.2. 스스로 검증 (Self-Verification)
코드를 작성하거나 수정한 후, 오케스트레이터에게 완료 보고를 하기 전에 **반드시 로컬 환경에서 컴파일·린트·테스트 및 티켓의 `verify_cmd`를 직접 실행**하여 `acceptance_criteria` 충족 여부를 확인하라.
- 검증 실패 시 에러 로그를 바탕으로 스스로 원인을 파악해 수정하고, 주요 문제와 해결 과정을 `WORKLOG.md`에 기록한다.
- 만약 티켓에 추가한 기능에 대한 테스트가 빠져 있다면, 오케스트레이터가 즉시 재현 검증할 수 있는 단위/통합 테스트를 함께 작성하여 실행 확인한다.

---

### 2.3. 명시적 보고 (Explicit Reporting)
작업 완료(또는 블로커 발생) 후 `hcom` 반환 메시지에는 인사말 없이 **반드시 다음 항목을 정확히 포함**하라:
1. **수정한 정확한 파일 경로들 (`changed_files`)**: `git status --short`로 직접 확인한 루트 기준 상대 경로 목록.
2. **추가되거나 제거된 주요 의존성 (`dependencies`)**: 패키지명 및 버전 (`added` / `removed`).
3. **검증용 테스트 명령어 (`verify_cmd`)**: 오케스트레이터가 작업을 검증하기 위해 그대로 실행해 볼 수 있는 정확한 CLI 명령어.
4. **다음 액션 및 특이사항 (`next_action`, `notes`)**: 검수 요청 사항 또는 `WORKLOG.md`에 기록한 발견 이슈 요약.

#### 반환 메시지 예시
```json
{
  "type": "TASK_REPORT",
  "task_id": "T-102",
  "status": "DONE",
  "changed_files": [
    "engine/src/graph/mod.rs",
    "engine/src/graph/validator.rs"
  ],
  "dependencies": {
    "added": ["petgraph@0.7"],
    "removed": []
  },
  "verify_cmd": "cd engine && cargo clippy -- -D warnings && cargo test graph::validator -- --nocapture",
  "next_action": "검수 및 T-102 Done 갱신 요청",
  "notes": "5개 필수 검증 케이스 모두 통과. 상세 내역 WORKLOG.md 기록"
}
```

---

### 2.4. 진행 상황 기록 (`WORKLOG.md` 운영)
- **디버깅 이력 기록**: 작업 중 겪은 빌드·런타임 문제, 원인 분석, 해결 방법, 실행한 주요 명령어를 `WORKLOG.md` 하단에 시간순으로 추가(Append)하라.
- **장기 작업 중간 기록**: 대규모 패키지 설치, 컨테이너 빌드, Docling 등 모델 가중치 다운로드·변환처럼 **10분 이상 소요되는 작업**을 수행 중일 때는, 중간중간 `WORKLOG.md`에 현재 진행 단계(`PROGRESS`)를 덧붙여 오케스트레이터가 진행 상황을 알 수 있게 하라.
