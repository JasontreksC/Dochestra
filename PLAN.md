# PLAN.md — 프로젝트 상세 구현 계획 및 상태 보드

> **관리 주체**: 오케스트레이터 전용 수정 파일 (워커 임의 수정 금지)
> **상위 요구사항 원천**: [`docs/RFP.md`](./docs/RFP.md) / **기술 계약**: [`CONTEXT.md`](./CONTEXT.md)
> **운영 규칙**: 오케스트레이터는 [`docs/RFP.md`](./docs/RFP.md)의 추상 요구사항과 스택 정의를 분석하여 본 문서에 **구체적인 마일스톤, 원자적 작업 티켓, 티켓별 검증 조건(`verify_cmd`)**을 생성하고, 워커 작업 진행에 따라 상태를 즉시 갱신합니다.

---

## 1. 프로젝트 목표 및 마일스톤

### 프로젝트 목표
- [`docs/RFP.md`](./docs/RFP.md)에 정의된 문서 중심 비주얼 워크플로 애플리케이션 **Dochestra**(`web`, `engine`, `extractor`, `db` 4컨테이너 구성) 구축

### 워커 트랙 (파일 경로 비중첩)
| 트랙 | 워커 | 담당 범위 |
| :--- | :--- | :--- |
| **R (Rust)** | `w1-kobe` | `engine/**`, `web/src/bindings/**` (ts-rs 생성물) |
| **I (Infra·Python·Web)** | `w2-fine` | `extractor/**`, `web/**`(bindings 제외), `docker-compose.yml`, `.env.example`, `.gitignore`, `scripts/**` |

### 마일스톤 목록
- [x] **M1 — 기반 스캐폴딩·인프라** (2026-11-26 완료): 3개 서비스 스켈레톤 + Dockerfile + Compose.
  - 완료 기준: `docker compose up -d --build --wait` 후 4개 컨테이너 healthy, `curl -fsS localhost/api/health` → `{"status":"ok"}`, extractor `/extract`가 샘플 PDF에서 MD/JSON/CSV/JPG 생성.
- [x] **M2 — 코어 엔진 (타입·DB·검증·실행기·API)** (2026-11-26 완료): 공유 타입/카탈로그/ts-rs, 워크플로 CRUD, 그래프 검증, Kahn 병렬 실행기, 런·SSE API, 번들 업로드·아티팩트 다운로드.
  - 완료 기준: 가짜 노드 러너로 병렬·실패 전파 테스트 통과, 실제 HTTP로 워크플로 저장→실행→SSE 수신 통합 테스트 통과.
- [x] **M3 — 노드 구현 및 외부 연동** (2026-11-26 — 12종 전부 실 API 로 동작 확인): 어댑터·소스 노드, extract(extractor 연동), LLM 계열(translate/analyze/summary), rag, voice, slide, fill.
  - 진입 조건: CONTEXT.md D-2/D-3/D-4/D-6 사용자 확정.
  - 완료 기준: 노드별 단위 테스트(외부 API는 mock 서버) + 실제 키로 1회 수동 스모크.
- [x] **M4 — 프론트엔드 에디터** (2026-11-26 완료, 브라우저 수동 확인은 M5): 랜딩, 사이드바, 캔버스(타입별 소켓·엣지 색상, 연결 검증), 파라미터 편집, 저장, 실행·SSE 상태 표시, 산출물 보기.
  - 완료 기준: `pnpm lint/typecheck/test/build` 통과 + 수동 시나리오(생성→노드 배치→연결→저장→실행→상태 표시).
- [ ] **M5 — 통합 E2E 및 문서화**: compose 기반 E2E 스크립트(번들 업로드 → bundle→pick_pdf→extract 실행 → 산출물 확인), README.

---

## 2. 현재 활성 상태 요약

- **현재 단계**: `M3 완료 → 품질 보강 / M5` (2026-11-26) — 외부 AI 7종 실 API 동작 확인(커밋 38e6449). fine: T-313(analyze 품질 보강) — T-312 완료, kobe: T-317(fill 품질 보강) — T-314 완료.
- **호스트 변경 기록**: 2026-11-26 w2-fine 이 Playwright chromium 실행을 위해 `sudo playwright install-deps chromium`(apt 시스템 패키지) 설치. 이후 sudo 를 쓰는 호스트 변경은 사전 승인 필요(CONTEXT §6.2).
- **운영 메모**: 08:20경 오케스트레이터 hcom 세션이 pty 종료로 끊겨 T-202·T-113 보고가 유실됨 → 재연결 후 이벤트 로그에서 복구·검수.
- **디스크 고갈 (2026-11-26 11:2x)**: 루트 77G 중 100% 사용(361MB) → 링커 Bus error. 오케스트레이터 임시 검증 빌드(4.3G) 등 삭제로 32GB 확보. 원인: 동시 빌드(엔진 다중 target, CUDA 이미지, docker build cache 22GB). 규칙: 검증용 별도 CARGO_TARGET_DIR 금지, 빌드 전 `df -h /` 확인. T-307 검수는 T-309 와 합쳐 진행.
- **교훈 (T-406)**: 검증을 localhost(보안 컨텍스트)로만 해서 사용자의 http://서버IP 접속 환경 버그를 놓침 → UI E2E 에 비-localhost 출처 테스트 상시 포함.
- **호스트 재부팅 (2026-11-26 10:08)**: 오케스트레이터·워커 세션 종료, /tmp 초기화. 작업 트리 무손상. 10:17 사용자 스택을 승인 이미지(engine T-308 검증 빌드, web 현 트리)로 재배포·smoke PASS. 워커 세션은 사용자가 재개(limo→kobe, pita→fine)했으나 hcom 신원 미복구 상태.
- **사고 (2026-11-26 08:47)**: 사용자·워커의 동시 `docker compose up`으로 db 컨테이너 기동 실패. 오케스트레이터가 스택 재기동(4서비스 healthy, smoke PASS)으로 복구, 재발 방지 T-114 긴급 할당.
- **사용자 결정 (2026-11-26)**: D-2/D-3/D-4/D-6 모두 권장안 확정 → M3 착수 가능(T-206 완료 후 트랙 R 순차 진행). D-6 = Claude(LLM) + OpenAI(임베딩·TTS).
- **알려진 운영 리스크**: 워커들이 같은 작업 트리를 공유하므로, 한 트랙의 진행 중 코드가 다른 검증(이미지 빌드)을 깨뜨릴 수 있음(T-114 검증 시 실제 발생). 마일스톤 단위 커밋 후 커밋 기준 검증으로 전환하는 것을 권장.
- **환경 메모**: 호스트에 Rust·pnpm 미설치(각 트랙 첫 티켓에서 사용자 홈에 설치), Docker 사용 가능, 2 vCPU / 7 GiB — 무거운 빌드 동시 실행 주의.

---

## 3. 작업 보드 (Kanban State Board)

### In Progress (진행 중)
| 티켓 ID | 마일스톤 | 상세 작업 목표 | 담당 워커 | 대상 경로 | 검증 명령어 (`verify_cmd`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| T-317 | M3 | fill 실사용 품질 보강(실제 양식형 폼·한글 라벨·키 표기 차이·LLM 매핑 정확도·체크박스/라디오/선택 값 변환) | w1-kobe | `engine/src/nodes/fill/**`, `engine/tests/fill.rs`, `engine/tests/fixtures/**`, `scripts/live/fill_check.sh`(신규, 단독) | engine 표준 검증 + fill_check 실 API |
| T-313 | M3 | analyze 실사용 품질 보강(차트·표 이미지·사진·텍스트 많은 이미지·작은 이미지, 수치 판독 정확도, 한국어 구조화) | w2-fine | `engine/src/nodes/analyze.rs`, `engine/tests/llm_nodes.rs`, `scripts/live/**`(slide_check.sh 제외), `fixtures/live/**` | live-nodes --only analyze + engine 표준 검증 |

### Blocked (보류 / 설계 조정 또는 외부 확인 필요)
| 티켓 ID | 마일스톤 | 작업 요약 | 블로커 사유 | 해소 방안 |
| :--- | :--- | :--- | :--- | :--- |
| ~~T-319~~ | M3 | rag 하이브리드 검색 | **사용자 결정으로 취소(2026-11-26)** — 대신 T-408 RAG 챗봇 UI | - |
| *(없음)* | - | - | - | - |

### Pending (대기 중)
| 티켓 ID | 마일스톤 | 트랙 | 상세 작업 목표 | 선행 티켓 | 대상 경로 | 검증 조건 및 명령어 (`verify_cmd`) |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| T-320 | M3 | R | slide PDF 폰트 서브셋(1.5MB → 축소) | T-314 | `engine/Cargo.toml`, `engine/src/nodes/slide/render.rs` | PDF 크기·글리프 |

### Done (검수 완료)
| 티켓 ID | 마일스톤 | 완료된 작업 요약 | 실행한 검증 명령어 및 결과 | 완료 일자 |
| :--- | :--- | :--- | :--- | :--- |
| `INIT` | Setup | `hcom` 파일 기반 상태 관리 체계 및 에이전트 지침 구축 | 상태·지침 파일 구조 확인 완료 | 2026-11-25 |
| `BOOT` | Setup | RFP 분석 → CONTEXT.md v1(디렉토리·타입·API·SSE·DB·env·컨벤션) 및 PLAN.md 마일스톤·티켓 수립 | 문서 검토 | 2026-11-26 |
| T-101 | M1 | engine 스켈레톤(Axum `/api/health`, Config, ApiError, 0001 마이그레이션 자동 적용, extractor 헬스 대기, dev-db.sh, Dockerfile) | 오케스트레이터 재실행: fmt/clippy 0 경고, cargo test 14 통과(단위 11·health 1·migrations 2), `docker build` 성공, 0001 SQL이 CONTEXT §5와 diff 0 | 2026-11-26 |
| T-111 | M1 | extractor: FastAPI `/health`·`/extract`(Docling 2.153.0, CPU torch), 모델 내장 Dockerfile(uid 10001) | 오케스트레이터 재실행: ruff OK, pytest 13 통과, uv.lock nvidia 0, docker build OK; `--network none` 컨테이너에서 health 200(24s)·`/extract` md/json/tables 1/images 1 생성, RSS ~1GiB | 2026-11-26 |
| T-201 | M2 | 공유 타입 34종 + 카탈로그 12종 + `GET /api/nodes` + ts-rs 바인딩 생성(`web/src/bindings`) + engine `/data` 10001:10001 | 오케스트레이터 재실행: fmt/clippy 0, cargo test 전부 통과(단위 56 등), 바인딩 결정적(sha1 동일), bigint 0, `/data` 10001:10001; 생성 TS가 CONTEXT §3·§4 계약과 일치함을 육안 확인 | 2026-11-26 |
| T-112 | M1 | web 스켈레톤(Vite 8, React 19.3, react-router 8, TanStack Query, Zustand, @xyflow/react 13, Tailwind 4, ESLint, Vitest 3케이스), nginx.conf(SSE·업로드·SPA), Dockerfile(nginx:1.30.8-alpine) | 오케스트레이터 재실행: lint/typecheck/test/build 통과, docker build OK, `nginx -t` OK | 2026-11-26 |
| T-202 | M2 | 워크플로 CRUD 5종 + `validate_structure` + ApiError VALIDATION_FAILED + JSON/Path 거부 400 통일 + `.sqlx` 5개 | 오케스트레이터 재실행: fmt/clippy 0, test 전부(lib 68, workflows_api 6 등), `sqlx prepare --check` OK, docker build(오프라인) OK; compose 스택에서 nginx 경유 생성 201→타입 불일치 PUT 422(TYPE_MISMATCH)→잘못된 UUID 400→DELETE 204 확인 | 2026-11-26 |
| T-113 | M1 | `docker-compose.yml`·`.env.example`·`.gitignore`·`scripts/smoke.sh` | 오케스트레이터 재실행: `compose config -q` OK, `up --build --wait` 4서비스 healthy(공개 포트 web 80만), smoke 6/6 PASS, check-ignore 기대대로; extractor 상주 RSS ~950MiB | 2026-11-26 |
| T-401 | M4 | 프론트 데이터 계층: `client.ts`(ApiError), `endpoints.ts`(§4.1.3 전부), `queries.ts`(qk·훅), `sse.ts`(applyRunEvent·useRunEvents) | 오케스트레이터 재실행: lint/typecheck/test(28)/build 통과(빌드 경고는 청크 크기·서드파티 directive 뿐), bindings 무변경, 타입 재선언 없음 | 2026-11-26 |
| T-203 | M2 | `plan_run`(구조→사이클 SCC→출력 노드→역추적 대상→미연결·params) + ExecutionPlan(결정적 Kahn 순서, dependents/upstream) + `POST /validate` | 오케스트레이터 재실행: fmt/clippy 0, test 전부(lib 81, validate_api 4 등), prepare --check OK, docker build OK; .sqlx·bindings 무변경 | 2026-11-26 |
| T-402 | M4 | 사이드바(목록·생성·삭제·선택) + EditorPage 상태 화면 + `test/utils.tsx` 헬퍼 | 오케스트레이터 재실행: lint/typecheck/test(42)/build 통과, 데이터 접근은 T-401 훅만 사용 확인 | 2026-11-26 |
| T-204 | M2 | Kahn 병렬 실행기(NodeRunner/RunObserver, 팬아웃 재사용, 출력 계약 검사, 실패→하위 skipped, 패닉 처리) + EventHub + storage 경로 헬퍼 | 오케스트레이터 재실행: fmt/clippy 0, cargo test 전부(lib 101), executor 17개 3회 연속 통과, prepare --check·docker build OK | 2026-11-26 |
| T-403 | M4 | 캔버스: checkConnection(7개 거부 규칙), graphMapping 왕복, editorStore, DochestraNode/TypedEdge/NodePalette/Canvas, 저장·422 표시 | 오케스트레이터 재실행: lint/typecheck/test(74)/build 통과. 실제 브라우저 드래그 조작은 미검증(M5 E2E/수동 확인 대상) | 2026-11-26 |
| T-205 | M2 | 런 API(POST/GET runs, GET run, SSE events) + DbObserver(DB 반영 후 브로드캐스트) + 고아 런 정리 + UnimplementedRunner + 0002(node_runs.seq) | 오케스트레이터 재실행: fmt/clippy 0, cargo test 전부(lib 104, runs_api 9×3 안정), prepare --check·docker build OK; SSE 구독→스냅샷 순서·Lagged/Closed 처리·X-Accel-Buffering 확인 | 2026-11-26 |
| T-114 | M1(보수) | compose 충돌 방지: `WEB_PORT` 변수화, db `start_period`, `scripts/verify-stack.sh`(dochestra-verify·18080·down -v·flock), smoke 프로젝트 비의존화 | 오케스트레이터 재실행: 사용자 스택 가동 중 실행 → 사용자 컨테이너 ID 불변, verify 잔존 컨테이너·볼륨 0 확인. 단 engine 빌드가 T-206 진행 중 코드(.sqlx 미갱신)로 실패해 smoke 단계는 T-206 완료 후 재확인 예정(워커 측 기록상 6/6 PASS) | 2026-11-26 |
| T-404 | M4 | 인스펙터(출력 토글·kind별 파라미터·노드 삭제) + BundlePicker(선택·생성·업로드·파일 삭제) + `params.ts`(엔진과 문구 동일) | 오케스트레이터 재실행: lint/typecheck/test(95)/build 통과, 파라미터 문구 4종 engine validator.rs 와 일치 확인. 번들 API 실연동은 T-206 후 확인 | 2026-11-26 |
| T-206 | M2 | 번들 API 6종(multipart, %PDF- 검사, 전부-또는-전무, FOR UPDATE, 413) + 아티팩트 다운로드(canonicalize 경로 제한, Content-Type/Disposition) | 오케스트레이터 재실행: fmt/clippy 0, cargo test 전부(lib 114, bundles_api 9, artifacts_api 4 등), prepare --check·docker build OK. 격리 스택(verify) E2E: smoke 6/6, 업로드·비PDF 400·validate ok·런→SSE snapshot/run_finished·실패 전파(bundle failed→skipped)·사용자 스택 불변 확인 | 2026-11-26 |
| T-405 | M4 | 검증·실행 버튼(dirty 시 저장 선행), 이슈 문구 9종·노드 하이라이트, SSE 상태 배지, RunPanel(노드 상태·산출물 미리보기·런 이력) | 오케스트레이터 재실행: lint/typecheck/test(114)/build 통과. 실제 브라우저 상호작용은 미검증(M5 UI E2E 대상) | 2026-11-26 |
| T-301 | M3 | EngineRunner(kind 디스패치) + bundle·pick_pdf·pick_jpg·json_input 노드, normalize_name 공용화 | 오케스트레이터 재실행: fmt/clippy 0, cargo test 전체 167 통과(nodes_run 5), prepare --check·docker build OK | 2026-11-26 |
| T-302 | M3 | extract 노드 + ExtractorClient(타임아웃·오류 문구·경로 검증) | 오케스트레이터 재실행: fmt/clippy 0, cargo test 전체 183 통과, prepare --check·docker build OK. 실제 Docling 연동은 E2E 에서 성공(md 'Annual Sales Report'·json·csv·jpg) | 2026-11-26 |
| T-501 | M5 | `scripts/e2e.sh`(①~⑨) + `verify-stack.sh --e2e` | 오케스트레이터 재실행: BASE_URL 미설정 rc=2, 격리 스택 smoke 6/6 + e2e ①~⑤ PASS, ⑥ 에서 엔진 SSE 상태 역행 버그를 정확히 검출(→ T-207). 사용자 스택 불변·잔존 0 | 2026-11-26 |
| T-207 | M2(보수) | SSE 연결별 상태 단조성 필터(should_forward, 스냅샷 기준 재설정) | 오케스트레이터 재실행: cargo test 187 통과, 격리 스택 e2e 3회 연속 14/14 PASS(⑥ 순서 검사 포함). 참고: 직전 verify-stack 3회가 engine unhealthy 로 실패했으나 로그 미보존으로 원인 불명 → T-115 | 2026-11-26 |
| T-503 | M5 | Playwright UI E2E(7단계: 생성·팔레트·드래그 허용/거부·업로드·저장/reload·실행 succeeded·NO_OUTPUT_NODE) + `verify-stack.sh --ui` | 오케스트레이터 재실행: web lint/typecheck/test 114/build, E2E_BASE_URL 미설정 실패, verify-stack --ui 1 passed(14s), 사용자 스택 불변. 호스트 변경: 워커가 `sudo playwright install-deps chromium`(apt) 실행 | 2026-11-26 |
| T-303 | M3 | Claude LLM 클라이언트(raw HTTP, fallbacks default, 재시도, refusal/max_tokens 처리, 키 비노출) + translate(청크 번역)·analyze·summary | 오케스트레이터 재실행: cargo test 224 통과(llm_nodes 20), prepare --check·docker build OK, 요청 계약(CONTEXT §4.3) 코드 대조. 실제 API 호출은 키 미제공으로 미검증 | 2026-11-26 |
| T-115 | M1(보수) | verify-stack 실패 시 ps/logs/inspect 보존(최근 10개), 실패 주입 훅 | 오케스트레이터 재실행: 주입 rc=1 → db/engine/extractor/web 로그·state.json·ps 생성, 정상 rc=0 시 미저장, 잔존 0. apt 설치 이력 WORKLOG 기록 확인 | 2026-11-26 |
| T-308 | M3 | LLM 요청 크기 상한 30MiB(전송 전 검사, summary 이미지 예산·제외 주석) | 오케스트레이터 재실행: cargo test 228 통과, prepare --check·docker build OK | 2026-11-26 |
| T-116 | M1(보수) | compose engine 환경변수 패스스루 5키 + .env.example 보강 | 오케스트레이터: `compose config` 5키 확인, 사용자 스택 불변. verify-stack 은 w1 T-304 미완성 코드(.sqlx 미갱신)로 engine 빌드 실패 — 범위 밖(T-115 로그로 즉시 원인 확인) | 2026-11-26 |
| T-304 | M3 | OpenAI 임베딩 클라이언트 + rag 노드(청킹·pgvector) + RAG 질의 API(검색+Claude 답변) | 오케스트레이터 재실행: cargo test 255(rag 12), prepare --check·docker build OK. 사용자 스택 engine 재배포·smoke PASS. 커밋 acf22bf | 2026-11-26 |
| T-305 | M3 | 공용 재시도·키 마스킹 모듈(common.rs) + OpenAI TTS 클라이언트(4000자 분할·결합) + voice 노드(대본→MP3) | 오케스트레이터 재실행: cargo test 281(voice 11, 기존 llm/rag 무수정 통과), prepare --check·docker build OK. 사용자 스택 engine 재배포·smoke PASS | 2026-11-26 |
| T-406 | M4(보수) | 노드 배치 UX: 팔레트 드래그 앤 드롭, 클릭 추가 시 겹침 회피·자동 선택·뷰 이동, fitView maxZoom 1 + **비보안 컨텍스트(http://서버IP)에서 crypto.randomUUID 부재로 노드/엣지 추가 불가 버그 수정**(lib/ids.ts newId 폴백) | 오케스트레이터 재실행: web test 130·lint·typecheck·build 통과. 사용자 스택 web 재배포 후 http://<서버IP>(isSecureContext=false)에서 클릭 추가 2·드롭 1·엣지 연결·인스펙터 표시·겹침 없음·페이지 에러 0 확인 | 2026-11-26 |
| T-306 | M3 | slide 노드: Claude 구성 JSON(검증·1회 재요청) → printpdf 16:9 렌더(나눔고딕 OFL 내장, 표지·본문·이미지 슬라이드, 페이지 번호) | 오케스트레이터 재실행: cargo test 306(slide 14), prepare --check·docker build OK, 컨테이너 폰트 존재 확인. 사용자 스택 engine 재배포·smoke PASS | 2026-11-26 |
| T-502 | M5 | README(소개·아키텍처·빠른 시작·환경 변수(OpenRouter)·사용법·노드 표·검증·문서 지도·제약) | 오케스트레이터: 상대 링크 전부 존재, env 표 CONTEXT §6.1 개정본 일치 확인. 번들 업로드 절차(4번)는 T-407 반영 시 갱신 | 2026-11-26 |
| T-407 | M4(보수) | 번들 노드 'PDF 업로드' 버튼·드롭존(번들 자동 생성·선택), 미선택 안내 문구, README 4번 갱신 | 오케스트레이터 재실행: web test 139·lint·typecheck·build 통과. 사용자 스택 web 재배포 후 http://<서버IP> 에서 버튼 노출·업로드·번들 자동 생성·파일 목록 확인, 에러 0 | 2026-11-26 |
| T-117 | M1(보수) | extractor Docling GPU(CUDA 13.4 torch, AcceleratorOptions, EXTRACTOR_DEVICE auto/cuda/cpu, /health device, compose GPU 예약) | 오케스트레이터: ruff·pytest 31(실 GPU 변환)·컨테이너 cuda(Tesla T4) 확인, 사용자 스택 extractor 재배포 → health device=cuda, smoke PASS. 측정(서브에이전트): 20쪽 PDF CPU 88s → GPU 11s(7.8배). 이미지 21.6GB(uv 캐시 6GB 잔존) — 축소는 T-118 | 2026-11-26 |
| T-307 | M3 | fill 노드(AcroForm 인식·정확/정규화/LLM 매핑·UTF-16 한글·체크박스/라디오/선택·NeedAppearances·리포트) | T-309 와 합동 검수: cargo test 347 통과. 실 OpenRouter(T-310): 키 표기가 다른 JSON 으로 6개 필드 전부 채움(PDFium 렌더 육안 확인) | 2026-11-26 |
| T-309 | M3 | 외부 AI OpenRouter 전환(OpenAI 호환 LLM·임베딩 1024(0003)·TTS pcm→MP3(LAME)), env·compose·.env.example | 오케스트레이터 엄격 검증(pipefail): fmt·clippy·test 347·prepare --check·docker build·Anthropic 잔존 0·compose 7키. 사용자 스택 engine 재배포(마이그레이션 적용, 키·모델 주입 확인)·smoke PASS | 2026-11-26 |
| T-310 | M3 | 실 API 검증 도구 `scripts/live-nodes.sh`(LIVE_CONFIRM 게이트, --with-verify-stack, SUMMARY.md) + 픽스처(영/한 보고서) | 실 OpenRouter 로 7개 노드 전부 succeeded: translate 10.2s·summary 6.8s·analyze 4.6s·slide 6.7s(8쪽)·rag 2.5s(질의 2/2 정답)·voice 39.4s(MP3 1.9MB)·fill 1.4s. 오케스트레이터가 산출물 직접 검토(번역·요약·차트 수치·슬라이드 렌더·폼 채움) | 2026-11-26 |
| T-118 | M1(보수) | extractor 이미지 축소: uv sync 에 tmpfs 캐시 마운트(uv 캐시 6GB 레이어 제거) | 오케스트레이터: slim 이미지 cuda 인식 확인 → latest 로 교체·사용자 스택 재배포(health device=cuda, smoke PASS). 이미지 21.6GB → 11.8GB(풀린 레이어 14.0→7.7GB). 구 이미지 삭제·빌드 캐시 정리로 디스크 여유 6.1GB → 39GB | 2026-11-26 |
| T-311 | M3 | translate 품질 보강: 제목-다음 블록 결합, 쪽 경계 문장 조각 재결합(split_translation_chunks), 응답 코드펜스 래퍼 제거·코드 블록 복원 + 회귀 테스트 8 | 오케스트레이터: HEAD+T-311 파일 깨끗한 worktree 에서 fmt·clippy·test 355·prepare 통과. 실 API 3케이스 산출물 직접 확인 — 20쪽 문서 제목 35/35·표 행 48/48·이미지 3/3 보존, 용어집 준수, 한→영 표 수치 보존. 사용자 스택 engine 재배포(worktree 빌드)·smoke PASS. 경미: 숫자 한글 풀어쓰기('아흔 일') | 2026-11-26 |
| T-315 | M3 | rag 품질 보강: chunk_markdown 재작성(제목 결합·문장 경계 균등 분할·섹션 제목 맥락) + scripts/live/rag_check.sh | 오케스트레이터: 깨끗한 worktree 에서 fmt·clippy·test 362·prepare 통과. 실 API 리포트 확인 — handbook 강제 잘림 30→0·문장 중간 시작 29→0·제목 맥락 28→75, 사실 11/12(92%)·모름 2/2·topK 경계 4/4. 미해결: 반복 문서 설정값 질문 검색 실패 1건(하이브리드 검색 후속), 모름 답변에 근거 표기 1건 | 2026-11-26 |
| T-316 | M3 | voice 품질 보강: 영문 상호·이름 한글 단일 표기 규칙, clean_script 안전망 + scripts/live/voice_check.sh(대본·MP3 디코딩 점검) | 오케스트레이터: 깨끗한 worktree 에서 fmt·clippy·test 369·prepare 통과. 실 API 리포트 확인 — 3문서 대본 1927~2390자·한국어·기호 0·사실 9/9·9/9·5/5, MP3 254~328s 24kHz 클리핑 0%·끝 잘림 없음. 한계: 청취 평가 불가, 다중 TTS 청크는 mock 만 | 2026-11-26 |
| T-312 | M3 | summary 품질 보강: 한국어 출력 강제(영어 어휘·단위 번역), 길이 비율 제어, 장 소제목 유지·반복 1회, 핵심요점-섹션 중복 금지, 이미지-본문 대응 / translate 숫자 아라비아 표기 1줄 | 오케스트레이터: 깨끗한 worktree 에서 fmt·clippy·test 370·prepare 통과. 실 API(누적 유료 10회): 4시나리오 succeeded, 20쪽 요약 장 8/8 소제목·사실 5/5, 한글 문서 사실 10/10, 이미지 47장→27개 제외 주석 정확. 관찰: 장 제목은 원문(영어) 유지 | 2026-11-26 |
| T-314 | M3 | slide 품질 보강: 대표 이미지 업스케일 상한 2.0(작은 이미지 깨짐 방지) + 비율 회귀 테스트 + scripts/live/slide_check.sh(PNG 렌더 점검) | 오케스트레이터: 깨끗한 worktree 에서 fmt·clippy·test 372·prepare 통과, 한글 슬라이드 7장 PNG 육안 확인(글리프·수치·차트 비율 정상). 관찰: 개요 슬라이드 내용 중복·희박(경미) | 2026-11-26 |
| T-408 | M4 | RAG 노드 인스펙터 챗봇: 실행 패널 선택 런/최근 성공 런에서 indexId 확보, 질의·답변·[n] 근거 배지·근거 목록, indexId 별 세션 기록, 대화 지우기, 한글 조합 입력 처리 | 오케스트레이터: web test 153·lint·typecheck·build 통과, 서브에이전트 실 API E2E(E2E_LIVE=1) 스크린샷 확인 — 답변 '482 million'·근거 2개. 사용자 스택 web 재배포·smoke PASS | 2026-11-26 |

---

## 4. 티켓 작성 및 상태 전이 기준 (오케스트레이터 전용)

1. **티켓 생성 기준**: 모든 티켓은 워커가 추측 없이 바로 착수할 수 있도록 **단일 책임 목표, 대상 파일 경로, 참조할 `CONTEXT.md` 섹션, 실행 가능한 검증 명령어**를 갖춰야 한다.
2. **상태 전이**:
   - `Pending -> In Progress`: `hcom`으로 워커에게 `TASK_ASSIGN` 발송 직전 갱신
   - `In Progress -> Done`: 워커 보고 수신 후 **오케스트레이터가 직접 검증 명령어를 실행하여 통과했을 때만** 갱신
   - `In Progress -> Blocked`: 외부 환경 요인 대기 또는 3회 실패 후 재설계 진행 시 사유와 함께 갱신
3. **트랙 진행 순서**: R: T-101 → T-201 → T-202 → T-203 → T-204 → T-205/T-206 → M3 / I: T-111 → T-112 → T-113 → (T-201 완료 후) T-401 → … → M5
