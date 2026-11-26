# CONTEXT.md — 시스템 기술 배경 및 인터페이스 계약서

> **관리 주체**: 오케스트레이터 전용 수정 파일 (모든 에이전트 작업 전 필독)
> **원천 문서**: [`docs/RFP.md`](./docs/RFP.md) (추상 요구사항 및 기술 스택 정의)
> **역할**: [`docs/RFP.md`](./docs/RFP.md)에서 추상적으로 남겨 둔 세부 구현 규약(디렉토리 구조, API/SSE 스키마, DB 스키마, 공유 볼륨 규칙, 코딩 컨벤션)을 **오케스트레이터가 구체화하여 확정·공유하는 기술 기준서**입니다. 워커는 본 문서의 계약을 임의로 변경하지 않고 그대로 준수합니다.
> **최종 갱신**: 2026-11-26 (Bootstrap v1, orch-hula)

---

## 0. RFP 보완 결정 (Design Decisions)

RFP 원문만으로는 타입 체계가 닫히지 않는 지점이 있어 아래와 같이 보완한다. 2026-11-26 사용자가 D-2/D-3/D-4/D-6을 모두 권장안으로 확정했다.

| ID | 결정 | 근거 | 상태 |
| :--- | :--- | :--- | :--- |
| D-1 | **최종 출력 노드 지정 방식**: 그래프의 각 노드에 `isOutput: bool` 플래그를 둔다. 실행 대상 = `isOutput=true` 노드들에서 입력 엣지를 역추적해 도달하는 노드 집합. 출력 노드가 하나도 없으면 실행 거부(`NO_OUTPUT_NODE`). | RFP 1.6 "최종 출력 노드" 지정 수단 미정의 | 확정 |
| D-2 | **어댑터 노드 추가** `pick_pdf`(LIST[PDF]→PDF), `pick_jpg`(LIST[JPG]→JPG): `index` 파라미터로 리스트 원소 하나를 고른다. | `bundle`은 LIST[PDF]만 출력하고 `extract`는 PDF만 받음. 단일 JPG를 출력하는 노드가 없어 `analyze`·`slide`의 JPG 입력을 연결할 수 없음 | 확정 (2026-11-26 사용자) |
| D-3 | **소스 노드 추가** `json_input`(입력 없음→JSON): 사용자가 캔버스에서 편집한 JSON 값을 출력한다. | `translate`(설정)·`fill`(키-값)의 JSON 입력을 공급할 노드가 없음 | 확정 (2026-11-26 사용자) |
| D-4 | **선택 입력 소켓**: `SocketSpec.required=false`인 소켓은 비어 있어도 실행 가능. 현재 `summary.images`만 선택 소켓. | RFP 1.5 "이미지 리스트도 넣어 요약에 보탤 수 **있다**" | 확정 (2026-11-26 사용자) |
| D-5 | **엄격 타입 매칭**: 엣지는 `DataType`이 완전히 같을 때만 허용(`LIST[T]`↔`T` 자동 변환·자동 map 없음). | RFP 1.6 "소켓 타입이 맞지 않으면 실행하지 않는다" | 확정 |
| D-6 | **외부 공급자 (2026-11-26 사용자 변경)**: 모든 외부 AI 호출을 **OpenRouter**(OpenAI 호환 API, `https://openrouter.ai/api/v1`) 하나로 통합. 키 `OPENROUTER_API_KEY` 하나, 모델은 env: LLM `meta/muse-ember`(이미지 입력 가능), 임베딩 `baai/bge-m3`(**1024차원**), TTS `google/gemini-3.8-flash-lite-tts`(mp3 미지원 → pcm 수신 후 엔진에서 MP3 인코딩). 기존 Anthropic Messages API 직접 호출 계약(§4.3 구판)은 폐기. | RFP 2.2 "외부 API"로만 명시 | 확정 (사용자 .env 설정) |

---

## 1. 확정 기술 스택 요약 (`docs/RFP.md` 기준)

- **구성**: 단일 Docker Compose 네트워크 내 4개 컨테이너 (`web`만 호스트 `:80` 공개)
  - `web` (`:80`): TypeScript, React 19, Vite, `pnpm`(Node.js 22 LTS), `@xyflow/react`(React Flow), React Router, TanStack Query, Zustand, `EventSource`(SSE), `ts-rs` 생성 타입, Tailwind CSS v4, nginx 리버스 프록시(`/api/*` -> `http://engine:8080`)
  - `engine` (`:8080`): Rust (stable, edition 2024), Tokio, Axum 0.8(SSE), SQLx 0.8(오프라인 `.sqlx` 검증 및 마이그레이션), `pgvector`, `petgraph`, `serde`/`serde_json`, `ts-rs`, `reqwest`, `tokio::fs`, `tracing`, `thiserror`, `anyhow`
  - `extractor` (`:8000`): Python 3.12, `uv`, FastAPI, `uvicorn`, Docling (**CUDA torch, GPU 가속 — T-117**, `EXTRACTOR_DEVICE=auto|cuda|cpu`, GPU 없으면 auto 가 CPU 로 폴백; 이미지 빌드 시 모델 사전 다운로드)
  - `db` (`:5432`): `pgvector/pgvector:pg16`
- **공유 볼륨**: `data` -> `engine:/data`, `extractor:/data` / `pgdata` -> `db:/var/lib/postgresql/data`
- **크레이트/패키지 버전**: 위에 적은 메이저 버전을 기준으로 워커가 `cargo add` / `pnpm add` / `uv add`로 **실제 최신 호환 버전을 확인해** 잠금 파일에 고정한다(추측 금지).
  - 확정 고정(web, T-112): React 19.3, **react-router 8.x**(`react-router` 패키지, BrowserRouter/Routes API), TypeScript ~6.0.3(typescript-eslint peer 제약으로 7.x 제외), Vite 8, Vitest 5, @xyflow/react 13, Tailwind 4.3, pnpm 12.23.0, nginx 1.30.8-alpine. 경로 별칭 `@/*` → `web/src/*`.
  - **@xyflow/react 13 주의(T-403)**: v12와 API가 다르다 — `applyNodeChanges` 없음, `onNodesChange`는 `NodeChangeset`을 받아 `.applyTo()`/`.toArray()` 사용, `useStoreApi` → `useReactFlowStoreApi`. 웹 문서·예제(v12 기준)를 그대로 쓰지 말 것.
  - 확정 고정: `reqwest` 0.12 (`default-features=false`, `json`, `rustls-tls` — 0.13은 feature 명 변경·aws-lc 백엔드라 제외, sqlx와 동일 ring 백엔드 유지). Rust 1.100.0 stable, Docling 2.153.0.
- **GPU (T-117)**: 호스트 드라이버 620.70, Docker nvidia 런타임·CDI 사용 가능. GPU 는 extractor 만 사용(compose 에서 GPU 장치 예약). engine·web·db 는 GPU 불필요.
- **컨테이너 사용자 규약**: `engine`·`extractor` 런타임은 모두 **uid 10001 / gid 10001** 비루트 사용자로 실행하고, 이미지에 `/data`를 미리 만들어 `10001:10001` 소유로 둔다(공유 `data` 볼륨 쓰기 권한 일치).

---

## 2. 디렉토리 구조 및 모듈 경계

```
Dochestra/
├── docker-compose.yml          # [w2] 4개 서비스 정의
├── .env.example                # [w2] 키 목록 템플릿 (커밋)
├── .env                        # 미커밋
├── .gitignore                  # [w2]
├── scripts/
│   ├── smoke.sh                # [w2] compose 기동 후 헬스 스모크 테스트
│   └── e2e.sh                  # [M5] 통합 시나리오
├── engine/                     # [w1] Rust API 서버 + 실행기
│   ├── Cargo.toml / Cargo.lock
│   ├── Dockerfile              # rust:1-bookworm 빌드 -> debian:bookworm-slim 런타임
│   ├── .cargo/config.toml      # TS_RS_EXPORT_DIR 설정 (§3.1)
│   ├── .sqlx/                  # sqlx prepare 산출물 (커밋)
│   ├── migrations/             # 0001_init.sql ...
│   ├── scripts/dev-db.sh       # 로컬 개발/테스트용 pgvector 컨테이너 기동
│   ├── src/
│   │   ├── main.rs             # 부트스트랩: config -> tracing -> DB pool -> migrate -> extractor health -> serve
│   │   ├── lib.rs              # 모듈 공개 (통합 테스트용)
│   │   ├── config.rs           # 환경 변수 로딩 (Config 구조체)
│   │   ├── error.rs            # ApiError (thiserror) + IntoResponse (§4.1.1)
│   │   ├── state.rs            # AppState { db, config, http, runs(이벤트 허브) }
│   │   ├── types/              # ts-rs export 대상 공유 타입 (§3)
│   │   │   ├── mod.rs, data_type.rs, node.rs, graph.rs, run.rs, api.rs, bundle.rs
│   │   ├── catalog.rs          # 노드 카탈로그(NodeSpec 목록) 단일 정의처
│   │   ├── graph/              # validator.rs(구조·실행 전 검증), planner.rs(실행 대상 선별·위상 정보)
│   │   ├── executor/           # mod.rs(Kahn 병렬 스케줄러), runner.rs(NodeRunner trait), events.rs(브로드캐스트 허브)
│   │   ├── nodes/              # 노드별 구현: bundle.rs, pick.rs, json_input.rs, extract.rs, translate.rs, analyze.rs, summary.rs, slide.rs, voice.rs, fill.rs, rag.rs
│   │   ├── clients/            # extractor.rs, llm.rs, embedding.rs, tts.rs
│   │   ├── storage.rs          # /data 경로 규칙 헬퍼 (§3.5)
│   │   ├── db/                 # workflows.rs, runs.rs, bundles.rs, rag.rs (SQLx 쿼리)
│   │   └── api/                # mod.rs(Router), health.rs, nodes.rs, workflows.rs, runs.rs, bundles.rs, artifacts.rs, rag.rs
│   └── tests/                  # 통합 테스트 (#[sqlx::test])
├── extractor/                  # [w2] Python Docling 워커
│   ├── pyproject.toml / uv.lock / .python-version
│   ├── Dockerfile
│   ├── app/
│   │   ├── __init__.py
│   │   ├── main.py             # FastAPI 앱, /health, /extract
│   │   ├── schemas.py          # pydantic 요청/응답 모델 (§4.2)
│   │   ├── converter.py        # Docling 변환 + 파일 저장 로직
│   │   └── paths.py            # DATA_DIR 하위 경로 검증
│   ├── scripts/download_models.py  # (필요 시) 빌드 시 모델 프리패치
│   └── tests/                  # pytest (fixtures/sample.pdf 포함)
└── web/                        # [w2] React SPA
    ├── package.json / pnpm-lock.yaml
    ├── Dockerfile              # node:22 빌드 -> nginx:stable-alpine
    ├── nginx.conf
    ├── vite.config.ts, tsconfig*.json, eslint.config.js, index.html
    └── src/
        ├── main.tsx, App.tsx   # 라우터: "/" Landing, "/editor", "/editor/:workflowId"
        ├── bindings/           # ★ ts-rs 생성물 (engine이 생성·커밋, web 측 수동 편집 금지)
        ├── api/                # client.ts(fetch 래퍼·ApiError), queries.ts(TanStack hooks), sse.ts(useRunEvents)
        ├── store/              # editorStore.ts (Zustand: nodes, edges, selection, runState)
        ├── lib/                # connection.ts(isValidConnection 순수 함수), socketColors.ts, graphMapping.ts
        ├── pages/              # LandingPage.tsx, EditorPage.tsx
        └── components/         # Sidebar.tsx, canvas/(Canvas.tsx, DochestraNode.tsx, TypedEdge.tsx, NodePalette.tsx), panels/(Inspector.tsx, RunPanel.tsx)
```

**모듈 소유권 원칙**: `engine/**` 및 `web/src/bindings/**`는 Rust 트랙 워커, 그 외 `web/**`·`extractor/**`·루트 인프라 파일은 인프라/프론트 트랙 워커가 담당한다(티켓별 `target_files`가 최종 기준).

---

## 3. 데이터·공통 타입(`ts-rs`) 및 `/data` 파일 저장 규약

### 3.1. ts-rs 생성 규칙
- 모든 공유 타입은 `engine/src/types/`에 정의하고 `#[derive(Serialize, Deserialize, TS)]` + `#[ts(export)]`를 붙인다.
- 직렬화 규칙: 구조체 필드는 `#[serde(rename_all = "camelCase")]`, 열거형 값은 아래 명시된 표기를 따른다.
- 출력 경로: `engine/.cargo/config.toml`
  ```toml
  [env]
  TS_RS_EXPORT_DIR = { value = "../web/src/bindings", relative = true }
  ```
- 생성 명령: `cd engine && cargo test export_bindings` (ts-rs가 테스트 실행 시 파일 생성). 생성물은 커밋한다.
- 드리프트 검사: 생성 후 `git diff --exit-code web/src/bindings`가 깨끗해야 한다.
- 확정(T-201): ts-rs 12.0.1. 태그드 enum의 변형 필드 camelCase는 `#[serde(rename_all_fields = "camelCase")]`와 `#[ts(rename_all_fields = "camelCase")]`를 **함께** 지정해야 TS에 반영된다. 64비트 정수는 `#[ts(type = "number")]`.
- `serde_json::Value` 필드는 `#[ts(type = "unknown")]`, `Uuid`는 `string`, `DateTime<Utc>`는 `string`(ISO-8601)으로 매핑(ts-rs `uuid-impl`, `chrono-impl`, `serde-json-impl` feature 사용).

### 3.2. 데이터 타입 (`DataType`)
```rust
#[serde(rename_all = "UPPERCASE")]
pub enum ScalarType { Pdf, Md, Json, Csv, Jpg, Mp3 }        // "PDF" | "MD" | "JSON" | "CSV" | "JPG" | "MP3"

#[serde(rename_all = "camelCase")]
pub struct DataType { pub scalar: ScalarType, pub list: bool } // LIST[JPG] == { scalar: "JPG", list: true }
```
- 표시 문자열: `list=false` -> `"JPG"`, `list=true` -> `"LIST[JPG]"` (`impl Display`).
- 확장자: PDF `.pdf`, MD `.md`, JSON `.json`, CSV `.csv`, JPG `.jpg`, MP3 `.mp3`.
- LIST 중첩은 지원하지 않는다.
- **타입 색상 (web `src/lib/socketColors.ts`, 엣지·소켓 공통)**: PDF `#E5484D`, MD `#3E63DD`, JSON `#F5A524`, CSV `#30A46C`, JPG `#8E4EC6`, MP3 `#12A594`. LIST 타입은 같은 색 + 소켓은 사각형, 엣지는 두께 3px(단일 2px).

### 3.3. 노드 카탈로그 (`catalog.rs` — 단일 정의처)
```rust
#[serde(rename_all = "snake_case")]
pub enum NodeKind { Bundle, PickPdf, PickJpg, JsonInput, Extract, Translate, Analyze, Summary, Slide, Voice, Fill, Rag }

pub struct SocketSpec { pub id: String, pub label: String, pub data_type: DataType, pub required: bool }
pub struct NodeSpec   { pub kind: NodeKind, pub label: String, pub description: String,
                        pub inputs: Vec<SocketSpec>, pub outputs: Vec<SocketSpec>,
                        #[ts(type = "unknown")] pub default_params: serde_json::Value }
```

| kind | label | inputs (`id`: 타입) | outputs (`id`: 타입) | params (`defaultParams`) |
| :--- | :--- | :--- | :--- | :--- |
| `bundle` | PDF 번들 | — | `files`: LIST[PDF] | `{ "bundleId": null }` (UUID 문자열 필수) |
| `pick_pdf` [D-2] | PDF 선택 | `files`: LIST[PDF] | `pdf`: PDF | `{ "index": 0 }` |
| `pick_jpg` [D-2] | 이미지 선택 | `images`: LIST[JPG] | `image`: JPG | `{ "index": 0 }` |
| `json_input` [D-3] | JSON 입력 | — | `json`: JSON | `{ "value": {} }` |
| `extract` | 추출 | `pdf`: PDF | `md`: MD, `json`: JSON, `tables`: LIST[CSV], `images`: LIST[JPG] | `{}` |
| `translate` | 번역 | `md`: MD, `settings`: JSON | `md`: MD | `{}` |
| `analyze` | 이미지 분석 | `image`: JPG | `md`: MD | `{}` |
| `summary` | 요약 | `md`: MD, `images`: LIST[JPG] (required=false, D-4) | `md`: MD | `{}` |
| `slide` | 슬라이드 생성 | `md`: MD, `image`: JPG | `pdf`: PDF | `{}` |
| `voice` | 해설 음성 생성 | `md`: MD, `images`: LIST[JPG] | `mp3`: MP3 | `{}` |
| `fill` | 폼 자동 입력 | `pdf`: PDF, `values`: JSON | `pdf`: PDF | `{}` |
| `rag` | RAG 챗봇 | `md`: MD | `api`: JSON | `{}` |

- 모든 소켓은 `required=true`가 기본이며 예외는 표에 명시된 것만.
- React Flow의 `sourceHandle`/`targetHandle` 값 = 소켓 `id`.

### 3.4. 워크플로 그래프 (`WorkflowGraph`, DB `workflows.graph` JSONB와 동일 구조)
```rust
pub struct Position      { pub x: f64, pub y: f64 }
pub struct GraphNode     { pub id: String, pub kind: NodeKind, pub position: Position,
                           #[ts(type = "unknown")] pub params: serde_json::Value, pub is_output: bool }
pub struct GraphEdge     { pub id: String, pub source: String, pub source_handle: String,
                           pub target: String, pub target_handle: String }
pub struct WorkflowGraph { pub nodes: Vec<GraphNode>, pub edges: Vec<GraphEdge> }
```
- 노드/엣지 `id`는 클라이언트가 생성(`crypto.randomUUID()`)하는 불투명 문자열. 그래프 내 유일해야 함.
- 엣지는 항상 `source`(출력 소켓) -> `target`(입력 소켓).

### 3.5. 실행 산출물 및 `/data` 경로 규약
`DATA_DIR`(기본 `/data`) 하위:
```
{DATA_DIR}/bundles/{bundle_id}/{file_id}.pdf                         # 업로드 원본
{DATA_DIR}/runs/{run_id}/{node_id}/{socket_id}.{ext}                 # 단일 타입 출력
{DATA_DIR}/runs/{run_id}/{node_id}/{socket_id}/{index:04}.{ext}      # LIST 타입 출력 (0000부터)
{DATA_DIR}/runs/{run_id}/{node_id}/_work/                            # 노드 임시 작업 디렉토리
```
- 예: extract 노드 출력 = `.../{node_id}/md.md`, `.../{node_id}/json.json`, `.../{node_id}/tables/0000.csv`, `.../{node_id}/images/0000.jpg`
- `bundle` 노드는 파일을 복사하지 않고 원본 경로를 그대로 출력 아티팩트로 전달한다. `pick_*`도 선택된 원소 경로를 그대로 전달.
- 엔진 내부 아티팩트 표현:
  ```rust
  pub struct Artifact { pub data_type: DataType, pub paths: Vec<PathBuf> } // 단일 타입은 paths.len()==1
  ```
- `node_runs.outputs` JSONB 저장 형식: `{ "<socket_id>": { "dataType": {...}, "paths": ["/data/..."] } }`
- 경로 생성은 반드시 `engine/src/storage.rs` 헬퍼를 통해서만 한다(`run_node_dir`는 `safe_segment`(`[A-Za-z0-9_-]{1,64}`) 검사 후 `Result<PathBuf, StorageError>` 반환 — node_id가 규칙을 벗어나면 해당 노드는 Failed). 외부에서 받은 경로는 `DATA_DIR` 하위인지 canonicalize 후 검증한다.

---

## 4. 서비스 간 API 및 SSE 프로토콜 명세

### 4.1. `web` <-> `engine` (`/api/*`)
모든 요청/응답은 `application/json`(업로드·파일 다운로드·SSE 제외), 필드는 camelCase. 타입명은 ts-rs export 이름.

#### 4.1.1. 에러 응답 형식 (모든 4xx/5xx)
```json
{ "error": { "code": "VALIDATION_FAILED", "message": "사람이 읽을 메시지", "issues": [ /* ValidationIssue[], 해당 시에만 */ ] } }
```
| HTTP | `code` |
| :--- | :--- |
| 400 | `BAD_REQUEST` |
| 404 | `NOT_FOUND` |
| 409 | `CONFLICT` |
| 413 | `PAYLOAD_TOO_LARGE` |
| 422 | `VALIDATION_FAILED` (`issues` 포함) |
| 500 | `INTERNAL` |
| 502 | `UPSTREAM` (extractor·외부 API 실패) |

- **요청 거부 규칙(T-202 확정)**: 핸들러는 `ApiJson<T>`/`ApiPath<T>` 추출기만 사용한다. JSON 문법·스키마 오류·Content-Type 누락·잘못된 경로 파라미터는 모두 400 `BAD_REQUEST`(본문 초과만 413). 422는 그래프 검증 실패 전용.

#### 4.1.2. 검증 이슈 (`ValidationIssue`, `#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]`)
| code | 필드 | 검사 시점 |
| :--- | :--- | :--- |
| `DUPLICATE_ID` | `id` | 저장·실행 |
| `UNKNOWN_NODE` (엣지가 존재하지 않는 노드를 참조) | `edgeId`, `nodeId` | 저장·실행 |
| `UNKNOWN_SOCKET` | `edgeId`, `nodeId`, `socketId` | 저장·실행 |
| `TYPE_MISMATCH` | `edgeId`, `expected: DataType`, `actual: DataType` | 저장·실행 |
| `MULTIPLE_INCOMING` | `nodeId`, `socketId` | 저장·실행 |
| `CYCLE_DETECTED` | `nodeIds: string[]` | 실행 |
| `NO_OUTPUT_NODE` | — | 실행 |
| `UNCONNECTED_INPUT` | `nodeId`, `socketId` (실행 대상 노드의 required 소켓만) | 실행 |
| `INVALID_PARAMS` | `nodeId`, `message` | 실행 |

- **이슈 보고 순서(결정적)**: DUPLICATE_ID(노드→엣지) → 엣지 순서대로 UNKNOWN_NODE(source, target) → UNKNOWN_SOCKET(source, target) → TYPE_MISMATCH → MULTIPLE_INCOMING(소켓당 1회) → [실행 시] CYCLE_DETECTED → NO_OUTPUT_NODE → UNCONNECTED_INPUT/INVALID_PARAMS(실행 대상 노드 그래프 순서). 노드 id와 엣지 id는 별도 네임스페이스.
- **저장(PUT) 시**: 구조 검증(앞 5개)만 수행 → 위반 시 422. 미완성 그래프(미연결 소켓, 출력 노드 없음)는 저장 허용.
- **실행(POST runs) 시**: 전체 검증. 사이클은 전체 그래프 기준, `UNCONNECTED_INPUT`·`INVALID_PARAMS`는 실행 대상 노드 기준.

#### 4.1.3. 엔드포인트
| 메서드 | 경로 | 요청 | 응답 |
| :--- | :--- | :--- | :--- |
| GET | `/api/health` | — | 200 `{ "status": "ok" }` |
| GET | `/api/nodes` | — | 200 `NodeSpec[]` (카탈로그 표 순서) |
| GET | `/api/workflows` | — | 200 `WorkflowSummary[]` = `{ id, name, updatedAt }[]` (updatedAt 내림차순) |
| POST | `/api/workflows` | `CreateWorkflowRequest { name }` | 201 `Workflow` = `{ id, name, graph: WorkflowGraph, createdAt, updatedAt }` (빈 그래프) |
| GET | `/api/workflows/{id}` | — | 200 `Workflow` / 404 |
| PUT | `/api/workflows/{id}` | `UpdateWorkflowRequest { name?: string, graph?: WorkflowGraph }` | 200 `Workflow` / 404 / 422 |
| DELETE | `/api/workflows/{id}` | — | 204 / 404 (실행 기록·산출물 DB 행 cascade) |
| POST | `/api/workflows/{id}/validate` | — | 200 `ValidationReport { ok: bool, issues: ValidationIssue[] }` (실행 시점 전체 검증, 저장된 그래프 기준) |
| POST | `/api/workflows/{id}/runs` | — | 202 `RunCreated { runId }` / 404 / 422 (저장된 그래프 스냅샷으로 실행) |
| GET | `/api/workflows/{id}/runs` | — | 200 `RunSummary[]` = `{ id, status, startedAt, finishedAt }[]` (최신순, 최대 50) |
| GET | `/api/runs/{runId}` | — | 200 `RunDetail` |
| GET | `/api/runs/{runId}/events` | — | SSE (§4.1.4) |
| GET | `/api/runs/{runId}/artifacts/{nodeId}/{socketId}?index={n}` | `index` 기본 0 | 200 파일 스트림 (`Content-Type` 확장자 기준, `Content-Disposition: inline; filename=...`) / 404 |
| GET | `/api/bundles` | — | 200 `BundleSummary[]` = `{ id, name, fileCount, createdAt }[]` |
| POST | `/api/bundles` | `CreateBundleRequest { name }` | 201 `Bundle` |
| GET | `/api/bundles/{id}` | — | 200 `Bundle` = `{ id, name, createdAt, files: BundleFile[] }`, `BundleFile = { id, originalName, sizeBytes, position, createdAt }` |
| DELETE | `/api/bundles/{id}` | — | 204 (디스크 파일 포함 삭제) |
| POST | `/api/bundles/{id}/files` | `multipart/form-data`, 필드명 `files` (복수 가능, PDF만) | 201 `BundleFile[]` / 400(PDF 아님: 확장자 `.pdf` + 매직 바이트 `%PDF-` 확인) / 413 |
| DELETE | `/api/bundles/{id}/files/{fileId}` | — | 204 |
| POST | `/api/rag/{indexId}/query` | `RagQueryRequest { question, topK?: number (기본 5) }` | 200 `RagQueryResponse { answer, sources: { chunkIndex, content, score }[] }` — M3에서 구현 |

- 업로드 최대 크기: 요청당 200 MiB (engine `DefaultBodyLimit`, nginx `client_max_body_size 200m`).

**실행 상태 타입**
```rust
#[serde(rename_all = "snake_case")] pub enum RunStatus  { Running, Succeeded, Failed }
#[serde(rename_all = "snake_case")] pub enum NodeStatus { Pending, Running, Succeeded, Failed, Skipped }
pub struct ArtifactFileRef { pub index: u32, pub file_name: String, pub url: String }  // url = /api/runs/{runId}/artifacts/{nodeId}/{socketId}?index={i}
pub struct NodeOutput      { pub socket_id: String, pub data_type: DataType, pub files: Vec<ArtifactFileRef> }
pub struct NodeRunState    { pub node_id: String, pub kind: NodeKind, pub status: NodeStatus, pub error: Option<String>,
                             pub outputs: Vec<NodeOutput>, pub started_at: Option<DateTime<Utc>>, pub finished_at: Option<DateTime<Utc>> }
pub struct RunDetail       { pub id: Uuid, pub workflow_id: Uuid, pub status: RunStatus, pub started_at: DateTime<Utc>,
                             pub finished_at: Option<DateTime<Utc>>, pub nodes: Vec<NodeRunState> }   // nodes = 실행 대상 노드만
```
- `Skipped` = 상위 노드 실패로 실행하지 않음(RFP 1.6 실패 처리). 실행 대상이 아닌 노드는 `nodes`에 포함하지 않는다.
- 런 상태: 실행 대상 노드 중 하나라도 `Failed`면 런은 `Failed`, 모두 `Succeeded`면 `Succeeded`.

#### 4.1.4. SSE (`GET /api/runs/{runId}/events`)
- `Content-Type: text/event-stream`, 응답 헤더 `X-Accel-Buffering: no`, 15초 간격 keep-alive 코멘트.
- **이벤트 이름(`event:`)을 쓰지 않는다**(브라우저 `onmessage` 하나로 처리). 각 메시지의 `data:`는 `RunEvent` JSON 한 줄.
```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    Snapshot    { run: RunDetail },                                   // 연결 직후 항상 1회 먼저 전송
    NodeStatus  { node_id: String, status: NodeStatus, error: Option<String>, outputs: Vec<NodeOutput> },
    RunFinished { status: RunStatus },                                // 전송 후 서버가 스트림 종료
}
```
- 이미 종료된 런에 접속하면 `snapshot` + `run_finished` 전송 후 종료.
- **상태 단조성 보장(T-207)**: 한 SSE 연결에서 노드 상태는 절대 역행하지 않는다. 순위 pending(0) < running(1) < succeeded/failed/skipped(2). 서버는 연결별로 노드의 마지막 전송 상태(스냅샷 포함)를 기억하고, 순위가 더 낮거나 같은 상태의 `node_status` 이벤트는 전송하지 않는다(같은 종결 상태의 중복도 생략). Lagged 후 재스냅샷 시 기준을 스냅샷으로 갱신.
- 엔진 내부: 런별 `tokio::sync::broadcast` 채널을 `AppState`의 이벤트 허브에 보관하고, 상태 변경은 **DB 반영 후** 브로드캐스트한다(스냅샷과 이벤트 간 유실 방지를 위해 구독 → 스냅샷 조회 순서로 처리).

### 4.2. `engine` <-> `extractor` (`http://extractor:8000`, Compose 내부 전용)
| 메서드 | 경로 | 요청 | 응답 |
| :--- | :--- | :--- | :--- |
| GET | `/health` | — | 200 `{ "status": "ok" }` (Docling 컨버터 초기화 완료) / 503 `{ "status": "loading" }` |
| POST | `/extract` | `{ "pdf_path": "/data/....pdf", "output_dir": "/data/runs/{run_id}/{node_id}" }` | 200 `ExtractResponse` |

```json
// ExtractResponse (snake_case, 키 = extract 노드 출력 소켓 id)
{ "md": "/data/runs/r/n/md.md", "json": "/data/runs/r/n/json.json",
  "tables": ["/data/runs/r/n/tables/0000.csv"], "images": ["/data/runs/r/n/images/0000.jpg"] }
```
- 출력 파일명은 §3.5 규칙을 extractor가 그대로 따른다. 표·그림이 없으면 빈 배열(디렉토리 생성 불필요).
- 변환 내용: Markdown = `export_to_markdown()`(이미지 자리 placeholder), JSON = `export_to_dict()` 직렬화, 표 = 표마다 DataFrame → CSV(UTF-8, 헤더 포함), 그림 = 문서의 picture 요소마다 이미지 추출 → RGB 변환 → JPEG(quality 90). 페이지 이미지는 추출하지 않는다.
- OCR: 기본 비활성(`EXTRACTOR_OCR=false`), env로 활성화 가능.
- 동시 변환은 1건으로 직렬화(메모리 보호, `asyncio.Semaphore(1)` 또는 동등). 변환은 스레드풀에서 실행해 `/health`가 블로킹되지 않게 한다.
- 에러: 400 `{ "detail": "..." }` — 경로가 `DATA_DIR` 밖이거나 PDF 미존재 / 422(FastAPI 기본) — 요청 스키마 오류 / 500 `{ "detail": "..." }` — 변환 실패.
- 엔진 측 클라이언트 타임아웃: `/extract` 15분, `/health` 5초.
- **구현 확정 사항 (T-111)**: Docling 2.153.0, CPU torch 2.15.0. 모델(layout, tableformer, rapidocr, 약 731MB)은 이미지 빌드 시 `docling-tools models download ... -o /opt/docling-models`로 내장, 런타임 env `DOCLING_ARTIFACTS_PATH=/opt/docling-models`, `HF_HUB_OFFLINE=1` (네트워크 없이 기동 확인). 기동~healthy 약 25초, 샘플 변환 시 RSS 약 1GiB. 이미지에 curl 없음 → compose healthcheck는 `python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8000/health')"` 형태로 수행.

### 4.3. `engine` -> 외부 API — OpenRouter (D-6 개정, T-309~)
- **공통**: 베이스 `{OPENROUTER_BASE_URL}`(기본 `https://openrouter.ai/api/v1`), 헤더 `Authorization: Bearer {OPENROUTER_API_KEY}`, `Content-Type: application/json`, 선택 헤더 `X-Title: Dochestra`. 키 미설정 시 해당 노드만 실패('OPENROUTER_API_KEY 가 설정되지 않았습니다'), 엔진은 기동. 오류 본문 `{error:{message, code}}`. 재시도: 429·500·502·503·연결 오류 3회(1s/2s/4s, retry-after 우선), 타임아웃 미재시도. 키·Authorization 값은 로그/오류 메시지에 절대 노출 금지(clients/common.rs).
- **LLM**: `POST /chat/completions` 본문 `{ model: LLM_MODEL, messages: [{role:"system",content}, {role:"user",content:[{type:"text",text}, {type:"image_url",image_url:{url:"data:image/jpeg;base64,<b64>"}}...]}], max_tokens: 16000, reasoning: {effort: LLM_EFFORT} }` (기본 모델 `meta/muse-ember`, effort 기본 `medium`; 이미지 파트는 텍스트 앞). 응답 `choices[0].message.content`(문자열; 배열이면 text 파트 결합). `finish_reason`: `stop` 정상, `length` → 'LLM 출력이 최대 길이에 도달했습니다', 내용이 비거나 null → 'LLM 응답이 비어 있습니다', `message.refusal` 존재 → 'LLM 이 요청을 거절했습니다'. 요청 크기 상한 30MiB 규칙 유지.
- **임베딩**: `POST /embeddings` `{ model: EMBEDDING_MODEL, input: [...] }` → `data[].{index, embedding}`. 기본 `baai/bge-m3`, **차원 1024 고정**(DB `vector(1024)`, 다르면 오류). 배치 96개/200,000자 유지.
- **TTS**: `POST /audio/speech` `{ model: TTS_MODEL, input, voice: TTS_VOICE, response_format: "pcm" }` → 응답 `audio/pcm;rate=<Hz>;channels=<n>` 16-bit LE 원시 PCM(관측: 24000Hz mono). 청크(최대 4000자, 문장 경계)별 PCM 을 순서대로 이어 붙인 뒤 **엔진이 한 번에 MP3 로 인코딩**(rate/channels 는 응답 헤더에서 파싱, 청크 간 불일치 시 오류). 기본 모델 `google/gemini-3.8-flash-lite-tts`, 기본 음성 `Kore`(OpenAI 음성명 `alloy` 등은 이 모델에서 400).
- **RAG 노드 출력 JSON·RAG 질의 API**: 형식 동일(§ 이전 정의 유지), 단 임베딩 차원 1024, 답변 생성은 위 LLM 사용.
- **RAG 노드 출력 JSON (`api` 소켓)**: `{ "type": "rag_api", "indexId": "<uuid>", "method": "POST", "endpoint": "/api/rag/<indexId>/query", "request": { "question": "string", "topK": "number (선택, 기본 5, 1~20)" }, "chunkCount": N, "embeddingModel": "..." }`
- **RAG 질의 (`POST /api/rag/{indexId}/query`)**: question 1~2000자, topK 1~20(기본 5) 위반 400 / 인덱스 없음 404 / 질문 임베딩 → pgvector 코사인 거리 상위 topK → score = 1 - distance → LLM 에 [번호] 청크와 질문 → `RagQueryResponse`. 임베딩/LLM 실패·키 미설정 → 502 UPSTREAM.
- **voice 노드**: md + images → LLM 해설 대본(한국어 구어체, 1500~2500자, 이미지 규칙 20개·5MiB·30MiB) → `_work/script.txt` → TTS(pcm) → MP3 인코딩 → `mp3.mp3`.
---

## 5. 데이터베이스 스키마 및 마이그레이션 규약

- 마이그레이션 위치: `engine/migrations/NNNN_name.sql` (SQLx 단순 마이그레이션, 되돌리기 파일 없음). 엔진 시작 시 `sqlx::migrate!()`로 적용.
- 애플리케이션 쿼리는 `sqlx::query!`/`query_as!`/`query_scalar!` **매크로 사용**, `cargo sqlx prepare`로 `engine/.sqlx/` 갱신 후 커밋. Docker 빌드는 `SQLX_OFFLINE=true`.
- 테스트 코드에서는 런타임 쿼리(`sqlx::query`) 사용 가능.

### `0001_init.sql` (확정)
```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE workflows (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    graph       JSONB NOT NULL DEFAULT '{"nodes":[],"edges":[]}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE bundles (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE bundle_files (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id     UUID NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    original_name TEXT NOT NULL,
    stored_path   TEXT NOT NULL,
    size_bytes    BIGINT NOT NULL,
    position      INTEGER NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (bundle_id, position)
);

CREATE TABLE runs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id     UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    status          TEXT NOT NULL CHECK (status IN ('running','succeeded','failed')),
    graph_snapshot  JSONB NOT NULL,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at     TIMESTAMPTZ
);
CREATE INDEX runs_workflow_started_idx ON runs (workflow_id, started_at DESC);

CREATE TABLE node_runs (
    run_id       UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    node_id      TEXT NOT NULL,
    kind         TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('pending','running','succeeded','failed','skipped')),
    outputs      JSONB,
    error        TEXT,
    started_at   TIMESTAMPTZ,
    finished_at  TIMESTAMPTZ,
    PRIMARY KEY (run_id, node_id)
);

CREATE TABLE rag_indexes (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id           UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    node_id          TEXT NOT NULL,
    embedding_model  TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE rag_chunks (
    id           BIGSERIAL PRIMARY KEY,
    index_id     UUID NOT NULL REFERENCES rag_indexes(id) ON DELETE CASCADE,
    chunk_index  INTEGER NOT NULL,
    content      TEXT NOT NULL,
    embedding    vector(1536) NOT NULL
);
CREATE INDEX rag_chunks_index_idx ON rag_chunks (index_id, chunk_index);
CREATE INDEX rag_chunks_embedding_idx ON rag_chunks USING hnsw (embedding vector_cosine_ops);
```
- 임베딩 차원은 `0003` 마이그레이션으로 **1024** 로 변경(T-309). 변경은 신규 마이그레이션으로만.
- 엔진 재시작 시 `status='running'`으로 남은 런은 `failed`로 정리한다(T-205): 해당 런의 `running` 노드 → `failed`(error '엔진 재시작으로 중단됨'), `pending` 노드 → `skipped`.

### `0002_node_runs_seq.sql` (T-205)
```sql
ALTER TABLE node_runs ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;  -- ExecutionPlan 순서(0부터)
```
- `RunDetail.nodes`는 `seq` 오름차순으로 반환한다.
- 새 DB에서 매크로 컴파일 전 `cd engine && cargo sqlx migrate run`으로 마이그레이션을 먼저 적용할 것(엔진 기동 시에도 자동 적용됨).
- 엔진 기본 러너는 M3 전까지 `UnimplementedRunner`(모든 노드를 "노드 '{kind}' 는 아직 구현되지 않았습니다"로 Failed). 런 흐름 자체는 E2E로 확인 가능.

---

## 6. 환경 변수 위치 및 코딩·검증 컨벤션

### 6.1. 환경 변수 (루트 `.env`, 미커밋 / `.env.example` 커밋)
| 키 | 사용처 | 예시·기본값 |
| :--- | :--- | :--- |
| `POSTGRES_USER` / `POSTGRES_PASSWORD` / `POSTGRES_DB` | db | `dochestra` / (비밀) / `dochestra` |
| `DATABASE_URL` | engine | `postgres://dochestra:<pw>@db:5432/dochestra` |
| `EXTRACTOR_URL` | engine | `http://extractor:8000` |
| `DATA_DIR` | engine, extractor | `/data` |
| `ENGINE_BIND` | engine | `0.0.0.0:8080` |
| `RUST_LOG` | engine | `info,dochestra_engine=debug` |
| `EXTRACTOR_HEALTH_REQUIRED` | engine | `true` (로컬 개발 시 `false`로 시작 시 대기 생략) |
| `EXTRACTOR_HEALTH_TIMEOUT_SECS` | engine | `600` |
| `EXTRACTOR_OCR` | extractor | `false` |
| `WEB_PORT` | compose(web 호스트 포트) | `80` (검증 스택은 `18080`) |
| `OPENROUTER_API_KEY` | engine | OpenRouter 키(LLM·임베딩·TTS 공통, 미설정 시 해당 노드만 실패) |
| `OPENROUTER_BASE_URL` | engine | `https://openrouter.ai/api/v1` |
| `LLM_MODEL` / `LLM_EFFORT` | engine | `meta/muse-ember` / `medium` |
| `EMBEDDING_MODEL` | engine | `baai/bge-m3` (1024차원 고정) |
| `TTS_MODEL` / `TTS_VOICE` | engine | `google/gemini-3.8-flash-lite-tts` / `Kore` |

### 6.2. 로컬 개발 환경 규약 (호스트: Linux, 4 vCPU / 15 GiB RAM, NVIDIA Tesla T4 16GB — 2026-11-26 재부팅 후 사양 변경)
- Rust: `rustup` stable을 사용자 홈에 설치(`~/.cargo`). 명령 실행 전 `source ~/.cargo/env`.
- pnpm: `corepack enable --install-directory ~/.local/bin pnpm` 후 `export PATH=$HOME/.local/bin:$PATH`. `web/package.json`의 `packageManager` 필드로 버전 고정.
- Python: `uv`(설치됨), `extractor/.python-version` = `3.12`.
- 개발/테스트 DB: `engine/scripts/dev-db.sh`로 컨테이너 `dochestra-pg-dev` 기동 (`pgvector/pgvector:pg16`, `127.0.0.1:55432`, user/pw/db = `dochestra`/`dochestra`/`dochestra`).
  - 엔진 테스트용 `DATABASE_URL=postgres://dochestra:dochestra@127.0.0.1:55432/dochestra`
- 저사양 주의: Rust 빌드와 Docling 설치/이미지 빌드를 동시에 돌리면 메모리가 부족할 수 있다. `cargo build -j 2` 이하, 장시간 작업은 `WORKLOG.md`에 PROGRESS 기록.
- **sqlx-cli**: `cargo install sqlx-cli --version 0.8.6 --locked --no-default-features --features rustls,postgres` (sqlx 크레이트와 버전 일치 필수, 0.9.x 금지) (매크로 쿼리를 처음 도입하는 티켓에서 설치). `.sqlx` 갱신: `cd engine && DATABASE_URL=... cargo sqlx prepare -- --all-targets`.
- **호스트 시스템 변경 금지**: `sudo`, apt, 전역 설치 등 저장소·사용자 홈 밖을 바꾸는 작업은 오케스트레이터(→사용자) 사전 승인 없이 하지 않는다. 필요하면 BLOCKED 로 보고. (2026-11-26 Playwright install-deps 가 승인 없이 실행된 뒤 신설)
- **Compose 스택 소유 규칙 (2026-11-26 db 기동 실패 사고 후 개정, 10:18 재개정)**: 기본 프로젝트 `dochestra`(호스트 포트 80)는 **사용자가 상시 확인하는 스택**이며 **오케스트레이터가 운영**한다 — engine/web/extractor 티켓을 승인할 때마다 오케스트레이터가 **검증을 통과한 이미지로** 재배포(`docker compose build <svc>` 또는 검증 시 만든 승인 이미지 사용 → `docker compose up -d --no-build --wait` → `./scripts/smoke.sh`). 작업 트리에 다른 워커의 미완성 코드가 있으면 그 서비스는 마지막 승인 이미지를 유지한다. 워커는 이 프로젝트에 `up`/`down`/`restart` 등 어떤 compose 명령도 실행하지 않는다(`ps`·`logs` 조회만 허용). 워커·오케스트레이터의 통합 검증은 `scripts/verify-stack.sh`(T-114)만 사용 — 별도 프로젝트 `dochestra-verify`, 호스트 포트 `WEB_PORT=18080`, 전용 볼륨(종료 시 `down -v`), `flock`으로 동시 실행 직렬화.
  - 사고 원인: 08:47 사용자의 `docker compose up`과 워커(T-403 수동 확인)의 `docker compose up --build --wait`가 같은 프로젝트에서 동시에 실행되어 db 컨테이너가 생성 1초 만에 제거됨 → 사용자 측 `dependency db failed to start`.
- **커밋 정책 (2026-11-26 사용자 확정)**: 워커는 `git commit`/`git push`를 하지 않는다. 오케스트레이터가 티켓 승인 시점마다 로컬 브랜치 `dev/integration` 에 **검증 통과한 내용만** 커밋한다(다른 워커의 진행 중 파일은 index 에서 제외). **원격 푸시·PR 금지.**

### 6.3. 코딩 컨벤션
- **Rust**: `rustfmt` 기본 설정, `clippy -D warnings`. 라이브러리 에러는 `thiserror`, 바이너리 진입부만 `anyhow`. `unwrap()`/`expect()`는 테스트·초기화 코드 외 금지. 노드 실행마다 `tracing::info_span!("node", run_id, node_id, kind)`.
- **TypeScript**: `strict: true`, ESLint flat config(typescript-eslint + react-hooks), `any` 금지. API 타입은 `src/bindings`에서만 import.
- **Python**: `ruff check` + `ruff format --check`, 타입 힌트 필수, pydantic v2 모델.

### 6.4. 모듈별 표준 검증 명령어
| 모듈 | 명령어 |
| :--- | :--- |
| engine | `cd engine && source ~/.cargo/env && cargo fmt --check && cargo clippy --all-targets -- -D warnings && DATABASE_URL=postgres://dochestra:dochestra@127.0.0.1:55432/dochestra cargo test` |
| engine (바인딩 드리프트) | `cd engine && cargo test export_bindings && git diff --exit-code ../web/src/bindings` |
| extractor | `cd extractor && uv sync --frozen && uv run ruff check . && uv run ruff format --check . && uv run pytest -q` |
| web | `cd web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` |
| 이미지 빌드 | `docker build -t dochestra-<svc> <svc>/` |
| 통합 | `./scripts/verify-stack.sh` (격리 프로젝트에서 빌드·기동·smoke·정리) |
