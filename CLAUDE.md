# CLAUDE.md — 에이전트 공통 운영 지침 (Global Instructions)

본 저장소는 `hcom` 기반의 **오케스트레이터-워커(Orchestrator-Worker)** 멀티 에이전트 구조로 운영됩니다.
장기 작업 중 컨텍스트 증발과 에이전트 간 설계 불일치를 막기 위해, LLM 내부 기억이나 `hcom` 메시지 히스토리에만 의존하지 않고 **물리적인 마크다운 상태 파일**을 단일 진실 공급원(Single Source of Truth, SSoT)으로 삼아 작업합니다.

---

## 1. 문서 계층 및 상태 파일 아키텍처 (Shared Workspace)

[`docs/RFP.md`](./docs/RFP.md)에는 전체적인 제품 그림, 도메인 요구사항, 애플리케이션 스택 정의만 **추상적·선언적 수준**으로 정의되어 있습니다.
**오케스트레이터**는 이를 바탕으로 구체적인 아키텍처 계약([`CONTEXT.md`](./CONTEXT.md))과 상세 구현 계획·검증 조건([`PLAN.md`](./PLAN.md))을 직접 생성·관리하며, **워커**에게 실행 가능한 상세 명세를 전달합니다.

| 파일 | 역할 및 수록 내용 | 읽기 | 쓰기 권한 |
| :--- | :--- | :--- | :--- |
| [`docs/RFP.md`](./docs/RFP.md) | **상위 요구사항 원천**: 프로젝트 개요, 도메인 개념·규칙, 프론트/백엔드/인프라 스택 정의 | 전체 | **수정 금지** (사용자 관리 문서) |
| [`PLAN.md`](./PLAN.md) | **상세 구현 계획 및 상태 보드**: 마일스톤, 작업 분해(티켓), 의존성, 티켓별 검증 조건, Kanban 상태(`Pending` / `In Progress` / `Blocked` / `Done`) | 전체 | **오케스트레이터 전용** |
| [`CONTEXT.md`](./CONTEXT.md) | **구체화된 기술 배경 및 계약**: 디렉토리 구조, DB 스키마, API/SSE 명세, 공통 타입 규약, 환경 변수 위치, 코딩 컨벤션 | 전체 | **오케스트레이터 전용** |
| [`WORKLOG.md`](./WORKLOG.md) | **실행 및 디버깅 누적 로그**: 워커의 시행착오·해결법, 주요 실행 명령어, 장기 작업 진행률, 범위 외 발견 이슈 | 전체 | **워커 (주 기록)** / 오케스트레이터 |

### 역할별 상세 지침서
세션 시작 시 자신의 역할을 확인하고 해당 지침서를 즉시 읽으십시오.
- **오케스트레이터 지침**: [`docs/agents/ORCHESTRATOR.md`](./docs/agents/ORCHESTRATOR.md)
- **워커 지침**: [`docs/agents/WORKER.md`](./docs/agents/WORKER.md)

---

## 2. 전체 에이전트 4대 필수 원칙

### 원칙 1. 상태 파일 우선 원칙 (State-File First)
- 작업을 시작하거나 인계받기 전, **반드시 `PLAN.md`와 `CONTEXT.md`(필요 시 `docs/RFP.md`)를 먼저 읽어** 현재 마일스톤, 확정된 기술 계약, 작업 상태를 파악하라.
- 만약 프로젝트 초기(`Bootstrap` 단계)여서 `CONTEXT.md`와 `PLAN.md`의 상세 설계·티켓이 아직 비어 있다면, **오케스트레이터가 코드 작업 지시 전에 먼저 `docs/RFP.md`를 분석해 두 파일을 구체화**해야 한다.

### 원칙 2. 역할 엄수 (Strict Role Separation)
- 자신이 **`orchestrator`** 인지 **`worker`** 인지 명확히 인지하고 권한 밖의 행동을 하지 마라.
  - **오케스트레이터**: 직접 애플리케이션 코드를 작성하지 않는다. 추상적인 `docs/RFP.md`로부터 **상세 구현 설계·티켓·검증 조건을 도출**하고, 작업을 할당·검수하며 `PLAN.md`와 `CONTEXT.md`를 최신으로 유지한다.
  - **워커**: 오케스트레이터가 구체화해 전달한 단일 티켓 명세와 검증 조건 달성에만 집중한다. `PLAN.md`·`CONTEXT.md`·`docs/RFP.md`를 임의로 수정하거나, 정의되지 않은 API·스키마·구조를 자의적으로 창작하지 않는다.

### 원칙 3. 간결하고 구조화된 통신 (Structured `hcom` Messaging)
- `hcom` 메시지 전달 시 인사말과 불필요한 서술을 생략하라.
- 아래 필수 필드만 담은 **구조화된 JSON(또는 키-값 텍스트)**으로 전달하고, 긴 에러 출력이나 디버깅 내역은 `WORKLOG.md`에 기록하라.

#### [오케스트레이터 -> 워커] 작업 할당 (`TASK_ASSIGN`)
```json
{
  "type": "TASK_ASSIGN",
  "task_id": "T-101",
  "goal": "티켓의 구체적 단일 구현 목표",
  "target_files": ["생성/수정할 정확한 파일 경로 목록"],
  "context_refs": ["CONTEXT.md#섹션", "docs/RFP.md#섹션"],
  "spec": [
    "워커가 추측 없이 바로 구현할 수 있는 핵심 입출력·동작·예외 처리 명세"
  ],
  "acceptance_criteria": [
    "충족해야 할 구체적 완료 기준"
  ],
  "verify_cmd": "통과해야 할 빌드·린트·테스트 CLI 명령어"
}
```

#### [워커 -> 오케스트레이터] 결과 보고 (`TASK_REPORT`)
```json
{
  "type": "TASK_REPORT",
  "task_id": "T-101",
  "status": "DONE | BLOCKED | FAILED",
  "changed_files": ["실제 생성/수정한 정확한 파일 경로들"],
  "dependencies": { "added": [], "removed": [] },
  "verify_cmd": "오케스트레이터가 재현 실행할 검증 명령어",
  "next_action": "검수 요청 또는 명세 확정/블로커 해소 요청",
  "notes": "특이사항 한 줄 요약 (상세 내역: WORKLOG.md 참조)"
}
```

### 원칙 4. 환각 방지 (Zero Hallucination / CLI Grounding)
- 존재하지 않는 파일, 확인되지 않은 패키지 옵션, 실행하지 않은 테스트 결과를 추측하여 작업하거나 보고하지 마라.
- 모든 파일 경로·코드 상태·로그는 반드시 CLI 명령어(`ls`, `cat`, `grep`, `rg`, `git status`, `git diff` 등)로 직접 확인하라.

---

## 3. 표준 운영 사이클 (Lifecycle)

1. **[오케스트레이터 · 초기/마일스톤 설계]** `docs/RFP.md` 분석 -> 아키텍처·인터페이스 계약을 `CONTEXT.md`에 구체화 -> 상세 구현 티켓과 검증 조건을 생성하여 `PLAN.md`에 등록.
2. **[오케스트레이터 · 작업 지시]** `PLAN.md`에서 다음 티켓을 `In Progress`로 변경 -> 상세 명세(`spec`)와 검증 조건(`verify_cmd`)을 담아 `hcom`으로 워커에 지시.
3. **[워커 · 구현 및 자체 검증]** `hcom` 수신 -> `PLAN.md`·`CONTEXT.md` 참조하여 구현 (에러 해결·장기 작업 진행률은 `WORKLOG.md` 기록) -> 로컬에서 `verify_cmd` 직접 실행 확인 -> `hcom`으로 결과 반환.
4. **[오케스트레이터 · 검수 및 동기화]** 보고된 검증 명령어를 직접 실행해 검수 -> 통과 시 `PLAN.md`를 `Done`으로 갱신 후 다음 티켓 할당 (3회 연속 실패·핑퐁 시 작업 중단 지시 후 직접 원인 분석 및 재설계).
