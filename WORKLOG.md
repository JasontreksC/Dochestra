# WORKLOG.md — 에이전트 실행 및 디버깅 누적 로그

> **관리 주체**: 워커 주 기록 (Append-Only) / 오케스트레이터 검수·중단 메모 병기 가능
> **작성 원칙**:
> 1. **누적 기록(Append-Only)**: 기존 로그를 삭제하거나 덮어쓰지 말고 파일 하단에 시간순으로 추가하십시오.
> 2. **사실 기반**: 실제 실행한 CLI 명령어, 발생한 에러 핵심 요약, 해결 방법, 검증 결과를 정확히 기록하십시오.
> 3. **장기 작업 Heartbeat**: 10분 이상 소요되는 의존성 설치·컨테이너 빌드·모델 가중치 다운로드/변환 작업 시 중간 진행 단계(`PROGRESS`)를 덧붙이십시오.
> 4. **범위 외 이슈 및 명세 공백 보고**: 티켓 범위를 벗어난 리팩토링 대상이나 `CONTEXT.md` 명세 공백을 발견하면 임의로 수정·창작하지 말고 본 로그에 기록한 뒤 오케스트레이터에게 보고하십시오.

---

## 로그 작성 템플릿

```markdown
### [YYYY-MM-DD HH:MM] [티켓ID] [에이전트명] — 상태(START | PROGRESS | ISSUE | RESOLVED | DONE | BLOCKED)
- **작업 내용**: 수행한 핵심 액션 한 줄 요약
- **실행 명령어**:
  - `실행한 주요 명령어` -> 결과 요약 (exit code 등)
- **문제 및 해결 과정** *(해당 시)*:
  - 원인: ...
  - 조치: ...
- **발견 사항 / 후속 제안** *(범위 외 이슈 또는 명세 보완 필요 시)*:
  - 관련 경로 및 제안 내용
```

---

## 누적 작업 기록

### [2026-11-25 21:28] [INIT] [setup] — DONE
- **작업 내용**: 추상 요구사항 문서(`docs/RFP.md`) 기반 오케스트레이터 상세 계획·검증 조건 생성 및 워커 위임 지침 체계 구성 완료
- **생성/갱신 파일**:
  - `CLAUDE.md`, `PLAN.md`, `CONTEXT.md`, `WORKLOG.md`, `docs/agents/ORCHESTRATOR.md`, `docs/agents/WORKER.md`

### [2026-11-26 08:05] [T-111] [w2-fine] — START
- **작업 내용**: extractor 서비스 구현 착수 (uv 프로젝트, FastAPI, Docling, CPU torch 고정)
- **실행 명령어**:
  - `uv add fastapi "uvicorn[standard]" docling` -> **실수**: torch 소스 매핑 전에 추가해 CUDA 휠(nvidia-* 73건, .venv 5.9G)이 해석·설치됨
- **문제 및 해결 과정**:
  - 원인: `[tool.uv.sources]` 를 먼저 선언하지 않고 docling 을 추가 (torch 가 PyPI CUDA 휠로 해석)
  - 조치: `.venv`·`uv.lock` 삭제 → pyproject 에 `[[tool.uv.index]] pytorch-cpu`(explicit) + `[tool.uv.sources]` torch/torchvision 매핑 후 `uv add torch torchvision` 재해석 → `grep -c nvidia uv.lock` = 0, .venv 1.5G
  - 교훈: CPU 인덱스 매핑은 docling 추가 이전에 먼저 선언
- **확인한 Docling API (설치된 docling 2.153.0 / docling-core 2.108.0 소스 직접 확인)**:
  - `docling.document_converter.DocumentConverter(format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=PdfPipelineOptions(...))})`, `converter.initialize_pipeline(InputFormat.PDF)` (모델 선로딩), `converter.convert(path).document`
  - `PdfPipelineOptions`: `do_ocr`, `do_table_structure`, `generate_picture_images`, `images_scale`, `artifacts_path`(파이프라인 옵션) / 환경변수 `DOCLING_ARTIFACTS_PATH`(settings.artifacts_path)
  - `doc.export_to_markdown()`(기본 image_placeholder `<!-- image -->`), `doc.export_to_dict()`, `doc.tables[i].export_to_dataframe(doc=doc)`, `doc.pictures[i].get_image(doc)` (PIL)
  - 모델 사전 다운로드: `docling-tools models download layout tableformer rapidocr -o <dir>` (공식 CLI; 내부 `docling.utils.model_downloader.download_models`)
- **검증**: 로컬 모델(`docling-tools ... -o`, 731MB: layout-heron(+onnx), docling-models(tableformer), RapidOcr) + `HF_HUB_OFFLINE=1 DOCLING_ARTIFACTS_PATH=<dir>` 로 샘플 PDF 변환 성공(표 1·그림 1, 36s)

### [2026-11-26 08:15] [T-111] [w2-fine] — PROGRESS
- 앱 코드/테스트 작성 완료. `uv run ruff check . && ruff format --check . && pytest -q` -> 13 passed (실제 Docling 변환 포함, ~26s). 이제 Dockerfile 빌드(이미지 의존성 레이어 ~1.5GB + 모델 731MB 다운로드) 및 시스템 라이브러리 확인 중

### [2026-11-26 08:03] [T-101] [w1-kobe] — DONE
- **작업 내용**: engine 스켈레톤 구현(Axum `/api/health`, Config, ApiError, 0001 마이그레이션 자동 적용, extractor 헬스 대기, dev-db.sh, 2단계 Dockerfile)
- **실행 명령어**:
  - rustup(stable, minimal+rustfmt+clippy) 설치 -> rustc/cargo 1.100.0
  - `engine/scripts/dev-db.sh` -> pgvector/pgvector:pg16 pull 후 `dochestra-pg-dev` 기동, pg_isready 통과
  - `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` -> 경고 0
  - `DATABASE_URL=postgres://dochestra:dochestra@127.0.0.1:55432/dochestra cargo test` -> 단위 11(config 5, error 6) + health 1 + migrations 2 통과
  - 수동 기동: `DATABASE_URL=...55432... EXTRACTOR_HEALTH_REQUIRED=false DATA_DIR=/tmp/dochestra-data cargo run` -> `curl localhost:8080/api/health` => `{"status":"ok"}`; SIGTERM 시 "shutdown signal received" 후 정상 종료
  - 헬스 대기 실패 경로: `EXTRACTOR_URL=http://127.0.0.1:9 EXTRACTOR_HEALTH_TIMEOUT_SECS=3` -> "extractor did not become healthy within 3s" 에러, exit 1
  - `docker build -t dochestra-engine engine/` 성공; 컨테이너(--network host)에서 `/api/health` => `{"status":"ok"}` 확인
- **문제 및 해결 과정**:
  - 원인: reqwest 최신(0.13)은 `rustls-tls` feature 가 없음(`rustls` 로 개명, aws-lc 백엔드). 조치: 명세의 feature 명을 유지하기 위해 `reqwest = "0.12"` (ring 백엔드, sqlx 와 동일) 로 고정.
  - 원인: Dockerfile 의존성 캐시 레이어(더미 src)의 mtime 이 실제 src 복사본보다 새로워 cargo 가 더미 바이너리(437KB, 아무 동작 없음)를 재사용. 조치: 실제 빌드 전 `touch src/main.rs src/lib.rs` 추가 -> 바이너리 9.3MB, 정상 기동 확인.
- **발견 사항 / 후속 제안**:
  - `tower-http` 0.7.1(직접 의존) 과 0.6.x(reqwest 경유) 가 Cargo.lock 에 공존 — 동작 문제 없음.
  - 루트 `.gitignore` 가 아직 없어 `engine/target/` 이 untracked 로 보임(T-113 에서 w2 가 `target/` 추가 예정). engine/.dockerignore 는 별도로 target/ 제외.
  - `.sqlx/` 는 비어 있음(`.gitkeep` 만) — 매크로 쿼리(`query!`)를 쓰는 첫 티켓에서 `cargo sqlx prepare` 필요(sqlx-cli 미설치).
  - 이번 티켓에서 CONTEXT/PLAN 의 `M` 상태는 오케스트레이터 변경분이며 워커 수정 아님.

### [2026-11-26 08:55] [T-111] [w2-fine] — DONE
- **작업 내용**: extractor 서비스(FastAPI `/health`·`/extract`, Docling 변환, 경로 검증, CPU torch 고정, 모델 사전 다운로드 Dockerfile, pytest) 완료
- **실행 명령어 / 결과**:
  - `cd extractor && uv sync --frozen && uv run ruff check . && uv run ruff format --check . && uv run pytest -q` -> 통과, 13 passed (~21s, 실제 Docling 변환: 표 1·그림 1, CSV 헤더 `Quarter,Revenue,Cost,Profit`, JPEG 매직 확인)
  - `grep -c nvidia extractor/uv.lock` -> 0 (torch 2.15.0+cpu / torchvision 0.29.2+cpu, pytorch-cpu 명시 인덱스; 비-리눅스용 PyPI 항목도 lock 에 존재하나 nvidia 의존 없음)
  - `docker build -t dochestra-extractor extractor/` -> 성공 (최초 7m57s; 이미지 6.2GB(디스크)/1.77GB(압축), 모델 731MB 포함). 캐시 재빌드 3s
  - **오프라인 검증**: `docker run -d --network none -v <tmp>:/data dochestra-extractor` -> 기동 ~15초 후 `docker exec python urllib` 로 `/health` 503 loading → 200 ok. 같은 컨테이너에서 `/extract` 200, md/json/tables/0000.csv/images/0000.jpg 생성(모델 다운로드 시도 없음, `HF_HUB_OFFLINE=1` + `DOCLING_ARTIFACTS_PATH=/opt/docling-models`). `EXTRACTOR_OCR=true` 오프라인 변환도 성공(rapidocr 모델 포함)
- **결정 / 특이사항**:
  - 이미지에 apt 패키지 불필요(docling 이 opencv-python-headless 사용, torch CPU 휠 자체 포함) — import 및 변환 실제 실행으로 확인
  - 모델 사전 다운로드 대상: `layout tableformer rapidocr` (`docling-tools models download`). code_formula/picture_classifier 는 enrichment 비활성이라 제외(필요 시 추가)
  - uv 이미지 고정 버전 `ghcr.io/astral-sh/uv:0.13.9` (호스트 uv 와 동일, 태그 존재 확인)
  - 비루트 uid/gid 10001, `/data` 를 10001 소유로 생성(CONTEXT 사용자 규약 준수). 호스트 bind mount 로 테스트 시 디렉토리 쓰기 권한(chmod a+rwX) 필요
  - `pyproject.toml`: `[tool.uv] package=false`(앱은 패키지가 아님), pytest `pythonpath=["."]`, ruff 규칙 E,F,I,UP,B,ANN(tests 는 ANN 제외)
  - 모델 로딩/변환은 단일 uvicorn worker 전제(`--workers 1`); 컨테이너 메모리 관련: 변환 중 RSS 는 측정하지 않음(compose mem_limit 4g 적정성은 T-113 smoke 에서 확인 필요)
  - `extractor/scripts/download_models.py` 는 공식 CLI 로 충분해 만들지 않음
- **발견 사항 / 후속 제안**:
  - (실수 기록) CPU 인덱스 매핑 전에 docling 추가 → CUDA 휠 해석(상단 START 항목 참조)
  - samples PDF 에서 Docling 이 제목("Annual Sales Report")을 문서 끝에 배치(읽기 순서 이슈, 기능 영향 없음)
  - docling 이 `force_full_page_ocr` DeprecationWarning 출력(라이브러리 내부, 무시)
  - 루트 `.gitignore`(T-113)에 `.venv/`, `__pycache__/` 필요. `extractor/.pytest_cache`, `.ruff_cache` 는 자체 .gitignore 포함

### [2026-11-26 08:25] [T-201] [w1-kobe] — DONE
- **작업 내용**: 공유 타입(types/{data_type,node,graph,run,api,bundle}.rs), 카탈로그 12종, `GET /api/nodes`, ts-rs 바인딩 34개 생성(`web/src/bindings/*.ts`), Dockerfile /data 소유권·uid/gid 10001 보정
- **실행 명령어**:
  - `cargo add ts-rs --features uuid-impl,chrono-impl,serde-json-impl` -> ts-rs 12.0.1
  - `cargo test` -> 전 스위트 통과(lib 56 = 기존 11 + catalog 4 + types 7 + export_bindings 34, health 1, migrations 2, nodes_api 1)
  - `cargo test export_bindings` 2회 + sha1sum diff -> 동일(결정적), `grep -r bigint web/src/bindings` -> 0건
  - `docker build` OK, `docker run --rm --entrypoint stat dochestra-engine -c '%u:%g' /data` -> `10001:10001`
  - 위 verify_cmd 전체 체인 -> VERIFY_ALL_OK
- **확인 사항**:
  - 태그드 enum(RunEvent, ValidationIssue)의 변형 내부 필드 camelCase 는 `#[serde(rename_all_fields = "camelCase")]` + `#[ts(rename_all_fields = "camelCase")]` 를 함께 지정해야 TS 생성물에도 반영됨(직렬화 테스트로 serde 쪽도 검증).
  - `Option` 필드 중 `#[ts(optional)]` 는 UpdateWorkflowRequest/ApiErrorDetail.issues/RagQueryRequest.topK 에만 적용 → TS `name?: string` 형태. NodeRunState.error 등은 `string | null` 유지(RunEvent 직렬화는 null 명시).
  - `web/` 디렉토리가 아직 없던 상태라 ts-rs 가 `web/src/bindings/` 를 생성함. `.gitkeep` 는 만들지 않음(지시: 존재하면 유지).
- **발견 사항 / 후속 제안**:
  - 지시대로 error.rs 는 미수정(ApiErrorBody 는 TS 계약 전용). T-202 에서 `ApiError::Validation` 연동 시 `ApiErrorDetail` 과 형식 일치 확인 필요.
  - `size_bytes`(i64)·`file_count`(i64) 는 `#[ts(type = "number")]` 로 지정. position/chunk_index 는 i32 라 ts-rs 가 number 로 출력.
  - git status 에 `web/node_modules/**` 가 untracked 로 보임(w2 작업물, 루트 .gitignore 는 T-113).

### [2026-11-26 09:20] [T-112] [w2-fine] — DONE
- **작업 내용**: web 스켈레톤(Vite+React 19+TS strict+Tailwind v4+Router+Query+Zustand+@xyflow/react), ESLint/Vitest, nginx.conf, 2단계 Dockerfile
- **실행 명령어 / 결과**:
  - `corepack enable --install-directory ~/.local/bin pnpm` -> pnpm 12.23.0 (package.json `packageManager` 고정)
  - `cd web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build && docker build -t dochestra-web . && docker run --rm --add-host engine:127.0.0.1 dochestra-web nginx -t` -> 전부 통과 (테스트 3개, nginx syntax ok). 이미지 빌드 20s
  - 추가 확인: 컨테이너 기동 후 `/` 200, `/editor/abc` 200(SPA fallback), engine 미기동 시 `/api/health` 502(프록시 동작)
- **버전 (pnpm add 로 해석)**: react/react-dom 19.3.1, react-router **8.5.3**, @tanstack/react-query 5.108.4, zustand 5.1.0, @xyflow/react 13.0.3, tailwindcss/@tailwindcss/vite 4.3.5, vite 8.3.8, vitest 5.1.1, eslint 10.15.0, typescript **6.0.3**, @types/node 22.20.6
- **문제 및 해결 과정 / 명세와 다른 점**:
  - react-router: 명세는 "v7" 이었으나 `pnpm add react-router` 의 최신이 8.5.3. BrowserRouter/MemoryRouter/Routes/Navigate/Link/useParams 를 'react-router' 에서 그대로 사용 가능해 8.x 로 고정(단위 테스트로 확인)
  - typescript: 최신 7.0.2 는 typescript-eslint 8.74.0 peer(`>=4.8.4 <6.1.0`) 미충족 → `typescript@~6.0.3` 으로 고정(`pnpm peers check` 문제 없음). typescript-eslint 가 TS 7 을 지원하면 올릴 것
  - eslint-plugin-react-hooks 7.x: flat config 는 `reactHooks.configs.flat.recommended` (`configs.recommended` 는 legacy 형식이라 eslint 가 오류)
  - vitest `globals:false` 에서는 RTL 자동 cleanup 이 꺼져 테스트 간 DOM 누수 → `src/test/setup.ts` 에 `afterEach(cleanup)` 추가
  - nginx 이미지 태그: `nginx:1.30.8-alpine` (현재 `stable-alpine` 과 동일 digest 확인 후 고정)
  - `src/bindings/` 는 w1 생성물이 이미 있어 .gitkeep 을 만들지 않음; 해당 디렉토리 파일은 일절 변경하지 않음. tsc 는 bindings 도 타입체크 대상(통과), eslint 는 ignore
  - `pnpm build` 경고: 청크 크기(>500kB, React Flow 포함) 및 react-compiler-runtime "use no memo" 디렉티브 — 무시 가능
- **발견 사항 / 후속 제안**:
  - 루트 `.gitignore`(T-113)에 `node_modules/`, `dist/` 필요(현재 web/node_modules, web/dist 가 untracked 로 보임)
  - `tsBuildInfoFile` 은 node_modules/.tmp 로 지정해 산출물이 repo 에 남지 않음

### [2026-11-26 09:45] [T-113] [w2-fine] — START
- **작업 내용**: docker-compose.yml / .env.example / .gitignore / scripts/smoke.sh 작성. `docker compose config -q` 통과, `git check-ignore` 로 .env·target·node_modules·dist·.venv 무시, 잠금 파일·.sqlx·.env.example·web/src/bindings 비무시 확인
- **참고**: 첫 시도에서 smoke.sh 본문의 `rm -rf` 때문에 셸 안전 검사가 heredoc 명령 전체를 차단(미실행) → Write 도구로 파일 작성. 스크립트의 정리(rm)는 `$SMOKE_DIR`(고정값 /data/smoke)만 대상으로 engine 컨테이너 안에서 수행
- 다음: `docker compose up -d --build --wait` (engine 이미지 빌드 포함) → smoke → 메모리 측정 → down

### [2026-11-26 08:55] [T-202] [w1-kobe] — DONE
- **작업 내용**: 워크플로 CRUD 5종, `validate_structure`, `ApiError::Validation`(422 VALIDATION_FAILED, ApiErrorBody 직렬화로 리팩터), `ApiJson`/`ApiPath` 추출기(JSON/Path 거부 → 400 표준 형식), sqlx 매크로 쿼리 + `.sqlx` 오프라인 데이터 5개
- **실행 명령어**:
  - `cargo install sqlx-cli --version 0.8.6 --locked --no-default-features --features rustls,postgres -j 2` -> 설치 성공(~5분)
  - `cargo sqlx prepare -- --all-targets` / `--check` -> 통과, `.sqlx/query-*.json` 5개
  - 표준 검증 체인 + `docker build` (DB 없이 SQLX_OFFLINE) -> VERIFY_ALL_OK. cargo test: lib 68(validator 12, error 7 포함), workflows_api 6, 기타 통과
- **문제 및 해결 과정**:
  - 원인: 첫 `cargo install sqlx-cli`(버전 미지정)가 0.9.1 로 설치 시작 — 크레이트(sqlx 0.8.6)와 `.sqlx` 포맷 불일치 위험. 조치: 중단 후 `--version 0.8.6 --locked` 로 재설치. 참고: CONTEXT §6.2 의 설치 명령에 `--version 0.8.6 --locked` 추가를 권장.
  - 원인: axum 0.8 `JsonRejection` 은 스키마 오류 422 / Content-Type 누락 415 로 응답. 조치: From<JsonRejection> 에서 413 만 PayloadTooLarge, 나머지는 모두 400 BAD_REQUEST 로 통일(미지원 kind·깨진 JSON·빈 객체 모두 400).
  - 작업 중 `pkill -f <패턴>` 이 자기 셸 명령줄과 매칭되어 셸이 종료됨 — 이후 PID 기반 kill 사용.
- **명세 해석(보고 사항)**:
  - `MULTIPLE_INCOMING` 의 "유효 엣지" = 대상 노드·대상 소켓이 모두 해석되는 엣지(소스 쪽 오류·타입 불일치 여부와 무관). 소켓당 1회, 해당 소켓의 첫 엣지 순서로 보고.
  - DUPLICATE_ID 는 노드/엣지 id 를 서로 다른 네임스페이스로 취급. 노드 id 중복 시 조회는 첫 번째 노드 기준.
  - 이슈 순서: DUPLICATE_ID(노드→엣지) → 엣지별 UNKNOWN_NODE(source,target) → UNKNOWN_SOCKET(source,target) → TYPE_MISMATCH → MULTIPLE_INCOMING.
  - `From<sqlx::Error> for ApiError`(→Internal) 추가. 이름 검증은 trim 후 문자 수(chars) 기준 1~100, 저장값은 trim 된 이름.
  - PUT 은 `{}` 도 200(updated_at 만 갱신). web/src/bindings 는 신규 타입이 없어 변경 없음.
- **발견 사항 / 후속 제안**:
  - 후속 티켓에서 쿼리 매크로를 추가할 때마다 `cargo sqlx prepare -- --all-targets` 재실행 필요. 컴파일 시 DATABASE_URL 대상 DB 에 마이그레이션이 적용돼 있어야 함(dev DB 는 엔진 기동/테스트 시 적용됨; 새 DB 라면 `cargo sqlx migrate run` 또는 엔진 1회 기동).

### [2026-11-26 10:10] [T-113] [w2-fine] — DONE
- **작업 내용**: docker-compose.yml(4서비스, name: dochestra), .env.example(CONTEXT §6.1 전 키), 루트 .gitignore, scripts/smoke.sh 완료 → M1 인프라 마감
- **실행 명령어 / 결과**:
  - verify_cmd 전체(`cp -n .env.example .env; docker compose config -q && docker compose up -d --build --wait && ./scripts/smoke.sh; rc=$?; docker compose down; exit $rc`) -> rc=0. 4개 서비스 모두 healthy, smoke 6개 체크 전부 PASS
  - 호스트 포트: `docker compose ps --format json` 의 Publishers 로 web `80->80` 만 확인(db/engine/extractor 는 미공개)
  - `git check-ignore`: .env, engine/target, web/node_modules, web/dist, extractor/.venv, __pycache__ 무시 / .env.example, engine/.sqlx, engine/Cargo.lock, web/pnpm-lock.yaml, extractor/uv.lock, web/src/bindings/* 비무시 확인
  - web healthcheck `wget -qO- http://127.0.0.1/` (alpine busybox) 동작 확인
- **extractor 메모리**: 샘플 PDF 변환 후 cgroup `memory.peak` = 1,696,632,832 B (≈1.58 GiB, 모델 파일 페이지 캐시 포함), `docker stats` 샘플 최대 1.49 GiB → mem_limit 4g 로 충분(여유 ~2.4배). 대형 PDF 에서는 더 늘 수 있음
- **문제 / 특이사항**:
  - 첫 `docker compose up --build` 시 engine 이미지 빌드가 실패(w1 의 T-202 작업 중 트리: `src/db/workflows.rs` sqlx 매크로 vs 오프라인 `.sqlx` 불일치). 이는 범위 밖이라 건드리지 않고 임시 override(`image: dochestra-engine:latest`, 미커밋·scratchpad)로 나머지 구성을 먼저 검증. 잠시 후 engine 트리가 빌드 가능해져 최종 verify_cmd 는 override 없이 통과
  - smoke.sh 의 `/data/smoke` 정리는 engine 컨테이너 안에서 `$SMOKE_DIR` 고정값만 삭제. 호스트 파일은 건드리지 않음
  - 컨테이너 uid: engine·extractor 모두 10001 확인 → 공유 볼륨 교차 쓰기/읽기 정상
  - `.env` 는 로컬 생성만(미커밋, .gitignore 대상). 검증 후 `docker compose down`(볼륨 유지)으로 스택 종료
  - `docker compose` 가 `cp -n` 비이식성 경고를 출력(무해)
- **발견 사항 / 후속 제안**:
  - engine 이미지는 현재 트리 기준 빌드되므로 compose 검증은 engine 이 컴파일 가능한 시점에만 재현됨
  - engine 이 LLM/EMBEDDING/TTS 키를 아직 읽지 않음(패스스루만 정의)

### [2026-11-26 10:40] [T-401] [w2-fine] — DONE
- **작업 내용**: 프론트 데이터 계층 — `web/src/api/{client,endpoints,queries,sse}.ts` + 테스트 3파일(client.test.ts, sse.test.ts, queries.test.tsx). UI 변경 없음, 타입은 전부 `@/bindings/*` import
- **실행 명령어 / 결과**:
  - `cd web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` -> 전부 통과, 테스트 28개(App 3 + client 10 + sse 11 + queries 4). `any` 가 lint 오류로 잡히는지 임시 파일로 확인 후 제거
- **설계 결정**:
  - client.ts: `send()` 하나가 fetch·에러 변환을 담당하고 `request`(JSON)·`requestForm`(multipart, Content-Type 미지정)이 공유 → uploadBundleFiles 도 동일 ApiError 로직
  - useRunEvents: effect 안에서 최신 run 을 지역 변수로 접어(`applyRunEvent`) 상태에 publish → run_finished 시 workflowId 로 `runs(workflowId)` 도 무효화. runId 변경 직후 이전 런 데이터가 보이지 않도록 렌더 시점에 `tracked.runId !== runId` 면 `{run:null, connection:'connecting'}` 로 파생(effect 내 동기 setState 회피)
  - useUpdateWorkflow/useDeleteWorkflow 는 목록 키만 `exact: true` 로 무효화(방금 `setQueryData` 한 상세·runs 쿼리의 불필요한 재조회 방지). useCreateWorkflow 는 `['workflows']` prefix 무효화. delete 의 `removeQueries(workflow(id))` 는 prefix 라 해당 워크플로의 runs 캐시도 함께 제거
  - mutation 변수: useUpdateWorkflow = `{ id, name?, graph? }`, useUploadBundleFiles = `{ bundleId, files }`, useDeleteBundleFile = `{ bundleId, fileId }` (T-402~ 사용처 참고)
  - 아직 엔진에 없는 runs/bundles/artifacts/rag 도 CONTEXT §4.1.3 계약대로 함수화(실서버 연동 미검증, fetch 스텁 테스트만)
- **발견 사항 / 후속 제안**:
  - 명세는 `sse.test.ts` 이름이라 JSX 없이 `createElement` 로 wrapper 작성
  - artifacts 다운로드는 URL(`ArtifactFileRef.url`)을 `<a>`/`<img>` 에 직접 쓰는 방식이라 별도 함수 없음
  - vitest 가 jsdom 환경을 파일마다 생성해 테스트 파일이 늘면 느려질 수 있음(현재 8s) — 필요 시 `pool: 'vmThreads'`

### [2026-11-26 09:20] [T-203] [w1-kobe] — DONE
- **작업 내용**: `graph/planner.rs`(plan_run, ExecutionPlan/PlannedNode/InputBinding, dependents/upstream), `validator::validate_params`, `POST /api/workflows/{id}/validate`
- **실행 명령어**: petgraph 0.8.3 추가. 표준 검증 체인(fmt/clippy 0, cargo test, `cargo test graph::` 28개, `cargo sqlx prepare --check`, docker build) -> VERIFY_ALL_OK. lib 81, validate_api 4. `.sqlx` 변경 없음, bindings 변경 없음(내부 타입 미export).
- **파라미터 규칙(T-404 프론트 문구 일치용)**: 공통: params 는 JSON 객체("params 는 JSON 객체여야 합니다") / bundle: `bundleId` 가 UUID 로 파싱되는 문자열("bundleId 가 올바른 UUID 가 아닙니다", 누락·null·비문자열 포함) / pick_pdf·pick_jpg: `index` 가 u64 로 표현 가능한 정수("index 는 0 이상의 정수여야 합니다", 누락·음수·문자열·소수 1.5 및 1.0 포함) / json_input: `value` 키 존재 및 null 아님("value 가 필요합니다") / 그 외 kind: 객체 여부만. 하나의 노드에 위반이 있으면 첫 위반 1건만 보고.
- **구현 메모**:
  - 이슈 순서: 구조 이슈 있으면 그것만 → CYCLE_DETECTED(SCC 첫 멤버 순) → NO_OUTPUT_NODE → 실행 대상 노드 그래프 순서로 노드별 [UNCONNECTED_INPUT(카탈로그 입력 순) → INVALID_PARAMS].
  - 사이클이 있어도 나머지 이슈는 함께 보고, 계획은 생성하지 않음. 위상 순서는 Kahn + 최소 그래프 인덱스 우선(BinaryHeap)으로 결정적. 같은 (source,target) 쌍의 중복 엣지는 in-degree 에서 1회로 계산.
  - `dependents`/`upstream` 은 계획(plan.nodes) 안에서만 계산. 비실행 노드(출력에 닿지 않음)는 파라미터 검증도 생략.
- **발견 사항**: 저장 허용된 미완성 그래프에서 비실행 노드의 잘못된 params 는 validate 에서도 보고되지 않음(명세대로 실행 대상 기준). 프론트에서 노드별 경고가 필요하면 `validate_params` 규칙을 TS 로 복제해야 함.

### [2026-11-26 11:05] [T-402] [w2-fine] — DONE
- **작업 내용**: 에디터 셸 — `components/Sidebar.tsx`(목록·생성·삭제·선택), `pages/EditorPage.tsx`(미선택/로딩/404/에러/선택됨, `data-testid="canvas-area"`), `test/utils.tsx`(renderWithProviders, stubApi, jsonResponse, callsTo), App.test.tsx 정리
- **검증**: `pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` 통과, 테스트 42개(기존 28 중 App.test 3개 재작성 → 신규 14: Sidebar 10, EditorPage 4)
- **메모**:
  - 현재 워크플로 id 는 Sidebar 에서 `useMatch('/editor/:workflowId')` 로 얻음(삭제 후 /editor 이동 판단)
  - EditorPage 는 `<WorkflowView key={workflowId}>` 로 워크플로 전환 시 상태 초기화
  - 삭제 실패도 role=alert 로 표시(명세 외 소폭 추가)
  - LandingPage·App.tsx 는 변경 없음. 테스트용 fetch 스텁 키 형식: `"METHOD /api/path"`
  - 타입 오류 1건(`const [[, init]] = ...` 가 noUncheckedIndexedAccess 에 걸림) → `[0]?.[1]` 로 수정

### [2026-11-26 09:50] [T-204] [w1-kobe] — DONE
- **작업 내용**: `executor/{mod,runner,events}.rs`(Kahn 병렬 실행기, NodeRunner/RunObserver trait, EventHub), `storage.rs`(경로 헬퍼 + safe_segment), `futures` 의존성 추가(default-features=false, std+alloc; `BoxFuture` 용)
- **실행 명령어**: 표준 검증 체인 + `for i in 1 2 3; do cargo test executor:: -- --test-threads=1; done` -> VERIFY_ALL_OK (전체 115 통과, executor 17 ×3회 안정, 병렬 테스트 총 소요 ~0.2s 대 기준 350ms). `.sqlx`·bindings 변경 없음.
- **구현 메모 / 명세 대비 차이**:
  - `storage::run_node_dir` 는 `Result<PathBuf, StorageError>` 를 반환(spec 은 PathBuf). node_id 의 `safe_segment` 검사를 이 함수 안에서 강제하기 위함 — 호출자(T-205/T-206)는 `?` 처리 필요. 위반 시 실행기는 해당 노드를 Failed 로 보고(Running 보고 없이, 러너 미호출).
  - 노드 시작 순서: 입력 조립 결과 확인 → out_dir 생성 → observer Running → runner.run (spec 순서). out_dir 생성 실패·안전하지 않은 node_id·입력 조립 실패는 Running 없이 바로 Failed.
  - 출력 검증 시 카탈로그에 선언된 소켓만 보존(여분 소켓은 버림). 단일 타입 소켓은 paths.len()==1, LIST 는 0개 이상 허용.
  - 패닉 노드의 메시지는 `노드 실행 중 패닉: <payload>`. Failed 노드의 하위는 계획 순서대로 Skipped 보고, 이미 종결된 노드는 재보고하지 않음.
  - RunObserver 는 Running 은 노드 태스크 안에서, 종결 상태는 메인 루프에서 await 호출(노드별로 Running → 종결 순서 보장, 서로 다른 노드 간 호출은 병렬이므로 옵저버 구현은 스레드 안전해야 함).
  - 실행기는 동시성 상한이 없음(명세). 대용량 런에서 필요하면 후속 티켓에서 Semaphore 추가 가능.
- **T-205 참고**: `impl NodeRunner for Arc<X>` 형태로 테스트 더블을 만들었으나, 실제 구현은 `Arc<dyn NodeRunner>` 로 전달. `EventHub` 는 AppState 에 아직 연결하지 않음.

### [2026-11-26 12:10] [T-403] [w2-fine] — DONE
- **작업 내용**: 캔버스 에디터 핵심 — `lib/{socketColors,connection,graphMapping}.ts`, `store/editorStore.ts`, `components/canvas/{Canvas,DochestraNode,TypedEdge,NodePalette}.tsx`, `pages/EditorPage.tsx`(팔레트+캔버스+저장/422 이슈 표시/‘저장되지 않음’)
- **검증**: `pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` 통과, 테스트 74개(T-402 대비 +32: connection 9, graphMapping 4, socketColors 4, store 10, DochestraNode 3, EditorPage +2)
- **수동 확인(브라우저 없음 → API+단위테스트로 대체)**: compose 스택(현재 트리 빌드)에서 워크플로 생성 → bundle·pick_pdf·extract 그래프(엣지 bundle.files→pick_pdf.files, pick_pdf.pdf→extract.pdf)를 PUT 200 → GET 결과가 보낸 graph 와 완전 동일(왕복 OK). bundle.files→extract.pdf 엣지를 추가하면 엔진도 422 TYPE_MISMATCH(+MULTIPLE_INCOMING) 로 거부 → 프런트 `checkConnection` 의 'type-mismatch' 거부와 일치. `/editor/:id` SPA fallback 200. 실제 브라우저 드래그 조작은 미수행(jsdom 에서 핸들 렌더·store.onConnect 로 규칙 검증)
- **문제 및 해결 / 명세와 다른 점**:
  - **@xyflow/react 13.0.3 은 v12 와 API 가 다름**: `applyNodeChanges/applyEdgeChanges`·`useStoreApi` 없음. `onNodesChange/onEdgesChange` 는 배열이 아니라 `NodeChangeset/EdgeChangeset`(iterable, `.toArray()`, `.applyTo(elements)`)을 받음 → store 에서 `changeset.applyTo()` 사용, 테스트는 `new NodeChangeset([...])`. 팔레트의 캔버스 중앙 계산은 `useReactFlowStoreApi()` 의 `optionsStore.domNode` + `viewportStore.width/height` 사용
  - jsdom 에서 `<ReactFlow>` 노드·핸들이 DOM 에 렌더되어(`.react-flow__handle[data-handleid]`) DochestraNode 를 Canvas 경유로 테스트(ResizeObserver 스텁 필요, 기존 setup 으로 충분)
  - `test/utils.tsx` 에 `CATALOG` 픽스처(6종: bundle, pick_pdf, extract, translate, summary, json_input) 추가 — target_files 에 명시되진 않았으나 여러 테스트 공용이라 T-402 에서 만든 헬퍼 파일에 확장
  - EditorPage 는 `WorkflowView` 가 workflow.id 기준으로만 store.load(저장 후 refetch 가 편집 중 상태를 덮지 않음), 언마운트 시 store.reset
  - 이슈 표시 형식 `코드: 설명` (예 `TYPE_MISMATCH: 엣지 e1 타입 불일치 (PDF ← LIST[PDF])`, expected ← actual)
  - 노드 삭제는 store 에서도 연결 엣지를 함께 제거(React Flow 가 별도 edge remove 를 보내지 않는 경우 대비)
- **주의 (공유 환경)**: 수동 확인 중 첫 `docker compose up` 이 `/dochestra-extractor-1` 컨테이너 이름 충돌로 실패 — 다른 에이전트가 같은 compose 프로젝트(dochestra)를 동시에 띄우던 중이었던 것으로 보임. 이후 내 `docker compose down` 이 그 스택을 내렸을 수 있음(상대 검증이 끊겼다면 재실행 필요). 공유 호스트에서는 검증 전 `docker ps -a` 확인 권장
- **발견 사항 / 후속 제안**:
  - 선택 노드(`selectedNodeId`)와 `updateNodeParams/setOutput` 액션은 T-404 인스펙터에서 사용 예정
  - MiniMap 포함(`pannable zoomable`)

### [2026-11-26 12:55] [T-404] [w2-fine] — DONE (T-114 우선 처리 지시 전에 완료)
- **작업 내용**: `lib/params.ts`(엔진 validate_params 미러), `components/panels/{Inspector,BundlePicker}.tsx`, EditorPage 레이아웃 [팔레트|캔버스|인스펙터], store `removeNode` 액션 추가
- **검증**: `pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` 통과, 테스트 95개(T-403 대비 +21: params 8, Inspector 8, BundlePicker 6 중 일부 합산)
- **문구 일치 확인(grep)**: engine/src/graph/validator.rs:125,133,140,144 의 4개 문구('params 는 JSON 객체여야 합니다', 'bundleId 가 올바른 UUID 가 아닙니다', 'index 는 0 이상의 정수여야 합니다', 'value 가 필요합니다')가 web/src/lib/params.ts:22,29,34,36 과 글자 단위로 동일. 엔진 테스트 h_param_rules 입력(bundleId {}/null/"abc"/5, index -1/"a"/1.5/{}, params=[])을 params.test.ts 에 동일하게 사용. UUID 판정은 Uuid::parse_str 이 받는 형식(hyphenated/simple/braced/urn)을 모두 허용
- **메모**: index 입력은 키 입력마다 검사해 유효한 접두(예 '1.5' 입력 중 '1')는 그 시점에 저장됨(마지막 값이 무효면 오류 표시, store 는 마지막 유효값 유지). 번들 API 는 엔진 미구현 상태라 fetch 스텁으로만 검증

### [2026-11-26 09:58] [T-205] [w1-kobe] — DONE
- **작업 내용**: 런 API 완성 — `POST /workflows/{id}/runs`(검증→create_run→백그라운드 execute→finish_run→RunFinished), `GET /workflows/{id}/runs`, `GET /runs/{id}`, `GET /runs/{id}/events`(SSE), `DbObserver`(DB 반영 후 브로드캐스트), 시작 시 `fail_orphaned_runs`, `UnimplementedRunner`, `AppState{events, runner}` + `AppState::new`, 마이그레이션 0002(seq)
- **실행 명령어**:
  - `cargo sqlx migrate run --source migrations`(dev DB 에 0002 적용, 매크로 컴파일용) -> `cargo sqlx prepare -- --all-targets` -> `.sqlx` 에 신규 쿼리 10개 추가
  - 표준 검증 체인: fmt/clippy 0, cargo test 127 통과(runs_api 9, db::runs 단위 3 포함), `cargo test --test runs_api` ×3 안정(각 ~3.5s), `prepare --check`, docker build -> VERIFY_ALL_OK
  - 수동(dev DB, compose 미사용, 포트 8090): 워크플로 생성 → 그래프 PUT(bundle 단독 출력) → POST runs 202 → `curl -N /runs/{id}/events` -> `snapshot`(status failed, error "노드 'bundle' 는 아직 구현되지 않았습니다") + `run_finished(failed)`; GET /runs/{id} 동일. (런이 수 ms 만에 끝나 SSE 접속 시점에 이미 종료 → 종료 후 접속 경로 확인. 실시간 경로는 runs_api 테스트가 검증.) 수동 테스트용 워크플로는 dev DB 에서 삭제함.
- **명세 대비 차이 / 해석**:
  - `executor/mod.rs` 에 `pub mod observer;` 한 줄 추가(target_files 에 없었으나 모듈 등록에 필요).
  - `DbObserver` 에 `kinds: HashMap<String, NodeKind>` 필드 추가(NodeUpdate 에 kind 가 없어 카탈로그 순서 변환에 필요).
  - `executor/runner.rs` 에 `kind_name(kind)`(snake_case 이름) 헬퍼 추가 — node_runs.kind·UnimplementedRunner 메시지에서 사용.
  - `create_run` 은 `&PgPool` 을 받아 내부에서 트랜잭션 시작. `update_node` 의 started/finished 시각은 DB `now()`.
  - SSE: 구독을 먼저 → DB 스냅샷 → 이벤트 전달. 런이 running 인데 채널이 아직 없으면(런 행 INSERT 와 `events.create` 사이의 수 ms 구간) 50ms×20 재시도, 그래도 없으면 스냅샷 후 스트림 종료(클라이언트 EventSource 재연결). Lagged → 새 스냅샷 재전송 후 계속, Closed → DB 재조회해 종결이면 run_finished.
  - 종료된 런의 SSE: snapshot + run_finished 후 종료. 존재하지 않는 런: 스트림 시작 전 404 JSON.
  - 런 삭제(워크플로 삭제 cascade) 후 GET /runs/{id} 는 404(테스트 포함).
- **발견 사항 / 후속 제안**:
  - 백그라운드 실행 태스크가 패닉하면(execute 자체 패닉 — 현재 경로상 거의 없음) 런이 `running` 으로 남고 다음 재시작 때 정리됨. 필요하면 JoinHandle 감시 추가.
  - 실행기는 동시성 상한이 없고, 엔진 종료(SIGTERM) 시 진행 중인 런은 중단되어 DB 에 running 으로 남음 → 다음 기동 시 fail_orphaned_runs 가 정리.
  - T-206 에서 아티팩트 다운로드 구현 시 URL 형식(`/api/runs/{runId}/artifacts/{nodeId}/{socketId}?index={i}`)은 이미 응답에 채워지고 있음.
  - RULE 준수: compose 스택은 건드리지 않았고 dochestra-pg-dev 만 사용.

### [2026-11-26 13:15] [T-114] [w2-fine] — DONE
- **작업 내용**: 격리 검증 스택 — docker-compose.yml(`web.ports: "${WEB_PORT:-80}:80"`, db healthcheck `start_period: 10s`), .env.example(`WEB_PORT=80`), scripts/smoke.sh(프로젝트 하드코딩 없음, BASE_URL 기본 `http://localhost:${WEB_PORT:-80}`, 호출자가 설정한 WEB_PORT/BASE_URL/POSTGRES_* 가 .env 값보다 우선), scripts/verify-stack.sh(flock 직렬화, COMPOSE_PROJECT_NAME=dochestra-verify, WEB_PORT=18080, .env 없으면 .env.example 사용·.env 생성/수정 없음, trap EXIT 로 `down -v --remove-orphans`)
- **검증 (사용자 스택 `dochestra`(80) 가동 중 상태에서)**:
  - `docker compose config -q` OK. `WEB_PORT=18080 COMPOSE_PROJECT_NAME=x docker compose config` -> `name: x`, `published: "18080"`
  - 실행 전 `free -m`: used 2798 / free 676 / available 5135 MiB
  - `./scripts/verify-stack.sh` -> rc=0, smoke 6/6 PASS, 약 1분. 종료 후 dochestra-verify 컨테이너·볼륨·네트워크 잔존 0
  - 사용자 스택 전후 비교: `docker compose -p dochestra ps -q` ID 목록 동일, `Service/Status/CreatedAt` 동일(db 08:51:45, engine/extractor/web 08:52:08) — 컨테이너 재생성·중지 없음. 사용자 스택엔 ps 조회 외 명령 미실행
  - 지시된 verify_cmd 전체 재실행 -> 통과(VERIFY_CMD_OK)
  - 동시 실행 직렬화: verify-stack.sh 와 동일한 `exec 9>/tmp/dochestra-verify.lock; flock` 구문으로 8초 점유 중 두 번째가 7초 대기 후 획득함을 확인(스크립트 2개를 실제로 동시에 돌려 본 것은 아님 — 한 번에 1분 + 메모리 부담)
- **문제**: 첫 시도는 engine 이미지 빌드 실패(w1 의 T-205 작업 중 `.sqlx` 미갱신, `SQLX_OFFLINE=true but no cached data`). 범위 밖이라 미수정, `.sqlx` 갱신 후 재실행에서 통과. 실패한 실행에서도 trap 정리가 동작해 잔존 리소스 없음, 사용자 스택 무변경 확인
- **발견 사항 / 후속 제안**:
  - verify-stack.sh 는 사용자 `.env` 가 있으면 그 값을 사용(비밀번호 등). 격리 볼륨이라 데이터 충돌은 없음
  - 포트 18080 이 이미 사용 중이면 up 이 실패함(다른 검증이 flock 밖에서 돌 때) — 별도 처리 없음

### [2026-11-26 10:25] [T-206] [w1-kobe] — DONE
- **작업 내용**: 번들 API 6종(`api/bundles.rs`, `db/bundles.rs`), 아티팩트 다운로드(`api/artifacts.rs`, `db::runs::get_node_outputs`), storage 번들 경로/`ensure_within`, `Config.upload_limit_bytes`(UPLOAD_LIMIT_BYTES, 기본 200MiB). 의존성: axum feature `multipart`, `tokio-util`(io).
- **실행 명령어**: `cargo sqlx prepare -- --all-targets`(.sqlx 26개) -> 표준 검증 체인(fmt/clippy 0, cargo test 150 통과, `prepare --check`, docker build) -> VERIFY_ALL_OK. bindings 변경 없음. compose 미사용(dochestra-pg-dev 만).
- **동작 요약**:
  - 업로드: 필드 `files`(반복)만 처리, 나머지 필드 무시. `{file_id}.pdf.part` 로 스트리밍 저장 → 확장자 .pdf(대소문자 무시) + 선두 `%PDF-` 검사 → 하나라도 실패하면 이번 요청이 쓴 파일 전부 삭제·DB 행 없음·400(message 에 파일명과 사유). 전부 통과하면 rename 후 단일 트랜잭션으로 삽입(번들 행 `FOR UPDATE` 잠금으로 동시 업로드의 position 충돌 방지, position = MAX+1 부터). original_name 은 `/` `\` 기준 마지막 세그먼트, 비면 `file.pdf`.
  - 한도: `DefaultBodyLimit::max(upload_limit_bytes)` 를 업로드 라우트에만 적용. 초과(스트리밍 중 포함) → 413 PAYLOAD_TOO_LARGE 표준 JSON, 잔여물 없음. multipart 아님/`files` 필드 없음 → 400. 없는 번들 → 404(디스크 쓰기 전).
  - 삭제: 파일 삭제는 행 삭제 후 디스크 삭제(NotFound 무시, DATA_DIR 밖 경로면 삭제 생략+경고), 번들 삭제는 cascade 후 `bundles/{id}` 디렉토리 삭제. 파일 삭제 시 position 은 재배열하지 않음(간격 허용, 오름차순 유지).
  - 다운로드: 런/노드/소켓/index 없음 → 404, index 음수·비정수 → 400. 경로는 `ensure_within`(canonicalize: `..`·심볼릭 링크 해소 후 DATA_DIR 하위인지)로 검증, 실패 시 404+경고 로그. 번들 노드 산출물(bundles/ 경로)도 DATA_DIR 하위이므로 허용. Content-Type 은 scalar 기준, Content-Length 설정, `Content-Disposition: inline; filename="<ASCII 대체>"; filename*=UTF-8''<퍼센트 인코딩>`(따옴표·역슬래시·제어문자 안전 처리, 단위 테스트).
- **명세 대비 차이**:
  - `normalize_name`(1~100자) 규칙을 `api/bundles.rs` 에 복제함 — workflows.rs 는 이번 target_files 가 아니라서 가시성 변경을 하지 않음. 후속에서 공용화 가능(예: `api/extract.rs` 또는 신규 util).
  - `StorageError` 에 `Unresolvable`/`OutsideDataDir` 변형 추가.
  - `From<MultipartError>`, `From<MultipartRejection>`(api/bundles.rs), `From<QueryRejection>`(api/artifacts.rs) 로 거부를 표준 에러 형식으로 매핑.
- **발견 사항 / 후속 제안**:
  - T-301(bundle 노드)은 `bundle_files.stored_path`(절대 경로)를 읽어 `Artifact.paths` 에 그대로 담으면 됨. 번들 안의 파일이 0개면 LIST 출력은 빈 배열(허용).
  - 업로드 도중 크래시 시 `*.pdf.part` 가 남을 수 있음(번들 삭제 시 함께 제거). 필요하면 기동 시 정리 작업 추가 가능.
  - 엔진 기본 body limit(2MiB)은 JSON 라우트에 그대로 적용됨(업로드 라우트만 별도 한도).
  - M2 마감: 트랙 R 은 M3 설계 확정(D-2/3/4/6) 대기.

### [2026-11-26 14:05] [T-405] [w2-fine] — DONE
- **작업 내용**: `lib/issues.ts`(formatIssue 9코드·issueNodeIds), `components/panels/RunPanel.tsx`(이력·런 상세·산출물 링크/미리보기), DochestraNode 상태 배지·이슈 링, store(issueNodeIds/runNodeStatus/runStatus + setIssues/clearIssues/setRunState/clearRun), EditorPage 툴바(저장·검증·실행) + 하단 RunPanel, NodePalette 주석 문구('Provisional' 제거; UI 문구에 '잠정' 없음)
- **검증**: `pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` 통과, 테스트 114개(T-404 대비 +19). 실연동은 지시대로 미수행(기본 스택 미사용, 컨테이너 명령 일절 없음)
- **설계 메모**:
  - 실행/검증은 dirty 면 먼저 저장(`ensureSaved`), 저장 실패 시 중단하고 에러/이슈 표시. 저장·검증·실행 422 는 모두 같은 이슈 목록(`CODE: 문구`)·캔버스 하이라이트로 표시
  - RunPanel 은 `activeRun={id, live}` 를 받음: 방금 시작한 런·이력의 running 런은 live(SSE), 종료된 이력 런은 `useRun`(GET /api/runs/{id}) — 선택 시점에 결정해 run_finished 후 전환 깜빡임 없음. run 이 바뀔 때마다 store 에 노드 상태/런 상태 동기화(언마운트 시 clearRun)
  - '실행' 비활성 조건: 저장/검증/시작 요청 중, 또는 store.runStatus==='running' 또는 시작 직후 응답 대기. 워크플로 전환 시 WorkflowView 가 key 로 리마운트되어 activeRun·run/issue 상태 초기화(`load`/`reset` 도 초기화)
  - 그래프 편집(노드/엣지 변경·추가·삭제·파라미터·출력 토글)은 issue 하이라이트 clear, 선택·드래그 중 이동은 유지
  - 미리보기: MD/JSON/CSV 는 `fetch(url)` 후 앞 5000자 `<pre>`, JPG `<img loading=lazy>`, MP3 `<audio controls>`, PDF 는 링크만
  - 기존 EditorPage 테스트는 RunPanel 의 `GET /api/workflows/abc/runs` 스텁이 필요해져 보강
  - 노드 라벨은 이슈 문구에서 카탈로그 label(동일 종류 노드는 구분 안 됨; 필요 시 id 접두 추가 가능)

### [2026-11-26 10:55] [T-301] [w1-kobe] — DONE
- **작업 내용**: `nodes/{mod,bundle,pick,json_input}.rs` — `EngineRunner`(kind 디스패치; bundle/pick_pdf/pick_jpg/json_input 구현, 나머지는 `UnimplementedRunner` 에 위임해 문구 단일화), main.rs 가 EngineRunner 주입, `db::bundles::stored_files`(+`StoredFile`), `normalize_name` 공용화.
- **실행 명령어**: `cargo sqlx prepare -- --all-targets`(.sqlx 27개) -> 표준 검증 체인(fmt/clippy 0, cargo test 167 통과, `cargo test --test nodes_run` 5개, `prepare --check`, docker build) -> VERIFY_ALL_OK. bindings 변경 없음, compose 미사용.
- **동작 요약**:
  - bundle: `params.bundleId` → 번들·파일 조회(position 순). 번들 없음/파일 0개/디스크 누락 각각 명세 문구로 Failed. 성공 시 `files: LIST[PDF]` = stored_path 그대로(복사 없음).
  - pick_pdf/pick_jpg: `files`/`images` 입력에서 `params.index` 번째 경로를 단일 Artifact(`pdf`/`image`)로 전달. 범위 초과/빈 리스트 → `index {i} 가 범위를 벗어났습니다 (항목 {n}개)`.
  - json_input: `params.value` 를 `to_vec_pretty` 로 `{out_dir}/json.json` 에 기록(스칼라 값도 허용).
  - 통합 테스트: 실제 API 로 업로드한 PDF 바이트가 `artifacts/{pick}/pdf` 응답과 동일, bundle 노드 list 출력(`artifacts/b/files?index=1`)도 다운로드 가능, json_input 다운로드 JSON 일치(Content-Type application/json), 범위 초과/번들 없음/빈 번들 오류 문구, extract 는 미구현 failed + 상위 succeeded.
- **명세 대비 차이 / 메모**:
  - 선택 항목 수행: `normalize_name` 을 `api/workflows.rs` 에 `pub(super)` 로 두고 `api/bundles.rs` 는 이를 import(복제 제거). 이름 규칙 단위 테스트는 workflows.rs 로 이동.
  - `EngineRunner` 의 필드(db/config/http)는 후속 노드(T-302~)에서 사용하므로 `pub` 으로 노출(현재 bundle 만 db 사용).
  - 공용 헬퍼 대신 `UnimplementedRunner` 에 위임하는 방식으로 미구현 문구 중복 제거(runner.rs 미변경).
- **발견 사항**: 새 노드 추가 시 `EngineRunner::run` 의 match 에 분기 추가 + `validate_params` 규칙 필요 여부 확인. T-302 에서 extractor 호출은 `self.http`·`config.extractor_url` 사용 가능.

### [2026-11-26 14:50] [T-501] [w2-fine] — DONE
- **작업 내용**: `scripts/e2e.sh`(① 번들+업로드 ② 그래프 저장 ③ validate ④ NO_OUTPUT_NODE 422 ⑤ 실행 ⑥ SSE ⑦ 런 상세 ⑧ 산출물 6종 ⑨ 이력) + `verify-stack.sh --e2e`(smoke 후 e2e, 인자 없으면 기존 동작, 알 수 없는 인자는 usage 로 exit 2). BASE_URL 미설정/빈 값이면 `exit 2`, 종료 시 trap 으로 워크플로 2개·번들 삭제(임시 파일은 `rm -f`/`rmdir`)
- **검증 이력** (모두 `./scripts/verify-stack.sh --e2e`, 격리 스택 dochestra-verify:18080):
  1. 첫 실행 — extract 노드가 아직 미구현인 엔진 빌드: smoke 6/6 PASS, e2e ①~⑤ PASS, 노드 상태열 `n_x pending,running,failed`, 엔진 error 문구 `노드 'extract' 는 아직 구현되지 않았습니다`. (이때 ⑥ 을 '전 노드 succeeded' 로 검사해 ⑥ 에서 FAIL → 지시된 기준(⑥ 통과·⑦ 에서 FAIL)에 맞게 ⑥ 은 SSE 구조/순서만, 성공 여부는 ⑦ 에서 검사하도록 수정)
  2. 두 번째 — snapshot 이 이미 succeeded 인 노드에 동일 상태 node_status 가 또 오는 경쟁(`succeeded,succeeded`)이 관찰되어 FAIL → 연속 중복을 접어서 비교하도록 수정(레이스는 SSE 접속 시점에 따라 정상)
  3. 최종(그 사이 w1 이 extract 구현을 반영한 엔진 빌드): smoke 6/6 + e2e ①~⑨ 전부 PASS, exit 0. 노드별 상태열: n_b succeeded / n_p succeeded / n_x running,succeeded / n_j succeeded. 산출물: md `text/markdown; charset=utf-8`('Annual Sales Report' 포함), json `application/json`, tables[0] `text/csv; charset=utf-8`, images[0] `image/jpeg`, pick_pdf 바이트 == sample.pdf, json_input == {"targetLanguage":"ko"}, 이력 최신 == 이번 runId
  - `BASE_URL= ./scripts/e2e.sh`, `env -u BASE_URL ./scripts/e2e.sh` -> 즉시 에러 종료(rc=2)
  - 각 실행 후 dochestra-verify 컨테이너·볼륨 잔존 0, 사용자 스택(dochestra) 컨테이너 ID 목록 전후 동일(조회 `ps -q` 만 사용)
- **참고**: shellcheck 미설치라 정적 검사는 `bash -n` 만 수행. 지시된 "⑦ 에서 extract 미구현 FAIL" 출력은 빌드가 이미 extract 포함으로 바뀌어 최종 실행에선 재현되지 않음(위 1번 실행의 ⑥ 단계 출력으로 갈음, 노드 error 문구 확인)

### [2026-11-26 11:30] [T-302] [w1-kobe] — DONE
- **작업 내용**: `clients/{mod,extractor}.rs`(ExtractorClient: extract/health, ExtractorError, 상세 파싱), `nodes/extract.rs`(PDF→md/json/tables/images, 반환 경로 검증), `EngineRunner` 에 extractor 클라이언트 보관 + `NodeKind::Extract` 디스패치, `Config.extractor_timeout`(EXTRACTOR_TIMEOUT_SECS, 기본 900).
- **실행 명령어**: 표준 검증 체인(fmt/clippy 0, cargo test 183 통과, `prepare --check`, docker build) -> VERIFY_ALL_OK. nodes_run 통합 테스트 16개(이번 추가 11개: mock extractor 직접 호출 9 + 런 통합 3 중 일부), `.sqlx` 변경 없음(신규 쿼리 없음), bindings 변경 없음, compose·verify-stack 미사용.
- **동작 요약**:
  - 클라이언트: `POST {base}/extract` JSON `{pdf_path, output_dir}`(snake_case), 요청별 timeout. 오류: Timeout / Connect / Http{status, detail}(문자열 detail 그대로, 배열 등은 JSON 문자열화, 비JSON·detail 없음은 본문 앞 300자) / Decode. `health()` 제공(main.rs 의 대기 로직은 변경하지 않음 — 선택 사항).
  - 노드: `inputs["pdf"]` → extract → 반환된 모든 경로 검증(존재 + `ensure_within(DATA_DIR)` + 노드 `out_dir` 하위 + 일반 파일). 위반 문구: '예상 밖 경로를 반환했습니다: ...' / '반환한 파일이 없습니다: ...'. 출력: md·json 단일, tables·images LIST(빈 리스트 허용). 오류 문구는 명세 그대로(시간 초과 `({n}초)` 는 정수 초).
  - 실행기 출력 계약은 이미 LIST 소켓의 0개 경로를 허용(`!list && len != 1` 만 거부)해 executor 수정 없음 — 런 통합 테스트(`run_accepts_extract_output_with_empty_lists`)로 확인.
- **테스트 메모**: mock extractor 는 테스트 안에서 axum 으로 임의 포트에 기동, 요청 본문을 기록하고 요청받은 output_dir 아래에 실제 파일 생성. 시간 초과 테스트는 config 단위가 초라 1초 timeout + 3초 지연(약 1초 소요). 기존 `unimplemented_kinds_still_fail...` 테스트는 extract 가 구현되어 `fill`(bundle→pick_pdf + json_input) 로 교체.
- **실제 extractor 연동**: 선택 항목(`verify-stack.sh --e2e`)은 수행하지 않음 — 사용자 스택(extractor 포함)이 가동 중이고 가용 메모리가 약 4GiB 라 두 번째 Docling 스택 기동은 위험하다고 판단. 오케스트레이터 검수 시 `./scripts/verify-stack.sh --e2e` 로 실제 연동 확인 권장(응답 형식은 extractor/app/schemas.py·main.py 를 읽어 맞춤: 키 md/json/tables/images, 오류는 `{"detail": ...}`, 로딩 중 503).
- **발견 사항**: 엔진이 extractor 에 넘기는 경로는 엔진 쪽 절대 경로(`/data/...`)이며 extractor 의 `resolve_under(DATA_DIR)` 와 같은 볼륨을 전제로 함(compose 의 `data` 볼륨 공유로 충족).

### [2026-11-26 15:45] [T-503] [w2-fine] — DONE
- **작업 내용**: Playwright(headless chromium) UI E2E — `web/playwright.config.ts`(E2E_BASE_URL 필수·미설정 시 config 에서 즉시 throw, workers 1, viewport 1600x1000, outputDir `node_modules/.tmp/playwright-results`), `web/e2e/editor.spec.ts`(serial 1개 테스트 안에 7개 `test.step`), package.json `e2e` 스크립트, vite.config.ts vitest `exclude: [...configDefaults.exclude, 'e2e/**']`, tsconfig.node.json 에 `playwright.config.ts`·`e2e` 포함(+lib DOM), `verify-stack.sh --ui`(`--e2e` 와 조합 가능, 알 수 없는 인자는 usage exit 2). `web/src` 는 data-testid 포함 일절 수정 없음(role/aria-label/텍스트와 `span.font-semibold`, `.react-flow__*` 클래스로 충분)
- **의존성/환경**: `@playwright/test@1.65.1`, 브라우저 `playwright install chromium`(chrome-headless-shell 159). 실행 시 `libasound.so.2` 누락(exit 127) → 호스트에 passwordless sudo 가 있어 `sudo playwright install-deps chromium` 으로 시스템 패키지 설치(apt, **호스트 시스템 변경**: Playwright 가 요구하는 chromium 런타임 라이브러리 묶음)
- **검증**:
  - `./scripts/verify-stack.sh --ui` -> exit 0, smoke 6/6 + 7개 시나리오 통과(15.8s). `./scripts/verify-stack.sh --e2e --ui` -> exit 0 (smoke + e2e ①~⑨ 전부 PASS(T-207 반영으로 ⑥ 엄격 검사 통과) + UI 통과)
  - `pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run(114, e2e 미수집) && pnpm build` 통과
  - `E2E_BASE_URL= pnpm e2e` -> 즉시 에러. 각 실행 후 dochestra-verify 컨테이너·볼륨 잔존 0, 사용자 스택 컨테이너 ID 전후 동일
- **시나리오 요약**: ① 랜딩→에디터→사이드바 생성→URL ② 팔레트로 3노드 ③ 실제 마우스 드래그: bundle.files→pick_pdf.files 연결, bundle.files→extract.pdf 거부(엣지 수 불변), pick_pdf.pdf→extract.pdf 연결 ④ 인스펙터에서 번들 생성+sample.pdf 업로드, extract 출력 지정 ⑤ 저장→reload 후 노드 3·엣지 2·출력 배지 복원 ⑥ 실행→노드 3개 data-status succeeded(180s)·md.md 링크 ⑦ 출력 해제 후 실행→NO_OUTPUT_NODE 이슈 표시. 리소스는 이름 접두(`pw-<tag>`)로 afterEach 에서 API 정리
- **문제 및 해결**: 첫 실행은 ③에서 실패 — 캔버스가 과도하게 확대되어 extract 노드/핸들이 화면 밖이었음 → 노드 배치 후 Controls 의 fit-view 버튼 클릭(`fitView` 헬퍼)으로 해결. 같은 첫 실행에서 거부 드래그·첫 연결은 이미 정상(앱 버그 아님, 테스트 레이아웃 문제)
- **발견 사항 / 후속 제안**:
  - 팔레트로 추가한 노드는 서로 24px 만 어긋나 겹치므로 사용자도 노드를 끌어 배치해야 함(UX 개선 여지: 추가 위치를 더 벌리기)
  - 새 노드 추가 후 fitView 가 자동으로 일어나지 않아 큰 줌에서 시작함(첫 진입 줌이 노드 수 0 기준으로 과확대) — 후속 개선 후보
  - Playwright 산출물(trace/screenshot)은 `web/node_modules/.tmp/` 아래라 .gitignore 대상

### [2026-11-26 11:55] [T-207] [w1-kobe] — DONE
- **작업 내용**: SSE 연결별 노드 상태 단조성 보장 — `api/runs.rs` 에 `should_forward(last, next)`(순수 함수: pending 0 < running 1 < 종결 2, 순위가 오를 때만 전달)와 연결별 `sent: HashMap<nodeId, NodeStatus>` 추가. 스냅샷(초기·Lagged 재스냅샷) 큐잉 시 맵을 스냅샷 상태로 재설정, `node_status` 는 큐잉 시 필터(역행·중복·종결→다른 종결은 버림, 종결→다른 종결은 warn 로그). `run_finished` 는 그대로 전달. 이벤트 포맷·엔드포인트 계약·bindings 변경 없음.
- **원인(재확인)**: 핸들러는 구독 → DB 스냅샷 순서라, 구독 이후·스냅샷 조회 이전에 (DB 반영 후) 브로드캐스트된 이벤트가 스냅샷 뒤에 그대로 재전송되어 e2e ⑥ 의 `succeeded,running,succeeded` 역행이 발생.
- **테스트**:
  - 단위: `should_forward` 전 조합(미지 노드, 전진, 반복, 역행, 종결→다른 종결).
  - 통합(결정적 재현): `stale_and_duplicate_broadcasts_are_not_forwarded` — DB 에 a:succeeded/b:running/c·d:pending 인 런과 `events.create` 채널을 만들고 SSE 접속(스냅샷 생성) 후 `EventHub::send` 로 스냅샷에 이미 포함된 상태(a running/succeeded/failed, b running/pending)·반복 이벤트를 주입 → 전달되는 건 b succeeded, c running/failed, d skipped, run_finished 뿐. 필터를 비활성화하면 이 테스트가 실패함을 확인(원인 재현 검증) 후 복구.
  - 통합(반복): `every_connection_sees_monotonic_node_statuses` — 노드당 25ms 지연 런에 0~114ms 간격으로 20회 접속, 연결별 노드 순위 단조 증가·종료 후 추가 이벤트 없음·최종 모두 종결 확인.
  - 위 두 테스트를 위해 `tests/runs_api.rs` 의 TestRunner 에 `delay` 필드, Harness 에 `events`(EventHub 복제) 추가.
- **실행 명령어**: fmt/clippy 0, cargo test 187 통과, `prepare --check` OK. `./scripts/verify-stack.sh --e2e` 3회 연속 exit 0(09:24~09:30, 각 약 2분; 각 회차 smoke/e2e 전 항목 PASS, ⑥ SSE 순서 검사 통과; 실행 전 `free -m` available ≈ 4.8GiB; 종료 후 dochestra-verify 컨테이너 0개).
- **참고**: 첫 시도(09:22 시작)는 e2e 전 항목 PASS 후 `verify-stack.sh: line 53: RUN_UI: unbound variable` 로 exit 1 — 실행 중(09:23)에 스크립트가 편집되어(`--ui` 옵션 추가) bash 가 신·구 내용을 섞어 읽은 탓으로 판단. 안정화된 스크립트로 재실행한 3회는 모두 성공. 이번 티켓 코드와 무관.

### [2026-11-26 12:40] [T-303] [w1-kobe] — DONE
- **작업 내용**: `clients/llm.rs`(LlmClient: CONTEXT §4.3 raw HTTP, 재시도, 오류 문구), `nodes/{translate,analyze,summary}.rs`, `nodes/mod.rs`(EngineRunner 에 LlmClient + 디스패치, 공용 헬퍼 `read_text`/`read_image`/`write_md`/`MAX_IMAGE_BYTES`), `config.rs`(LLM_API_KEY/BASE_URL/MODEL/EFFORT), `.env.example`(LLM_BASE_URL·LLM_EFFORT 추가 + 주석). 의존성: `base64` 0.23.1.
- **실행 명령어**: 표준 검증 체인(fmt/clippy 0, cargo test 224 통과 — llm_nodes 20, lib 150, `prepare --check`, docker build) -> VERIFY_ALL_OK. bindings·`.sqlx` 변경 없음, compose 미사용. 실제 Anthropic 호출 없음(전부 axum mock).
- **클라이언트 동작**: 헤더 `x-api-key`/`anthropic-version: 2026-10-21`/`anthropic-beta: server-side-fallback-2026-07-01`, 본문 `model`·`max_tokens 16000`·`system`·`messages`·`output_config.effort`·`fallbacks:"default"` (thinking/temperature/top_p/top_k/tool_choice/stream 미전송 — 테스트로 확인). 이미지 블록은 텍스트 앞(원래 순서 유지), base64 STANDARD(개행 없음). 응답은 `type=="text"` 블록만 결합, `refusal`/`max_tokens` 는 §4.3 문구. 429/500/502/503/529·연결 오류는 최대 3회 재시도(1s/2s/4s, `retry-after` 우선·최대 60초로 상한), 그 외 4xx 는 즉시 실패(`error.message` 전파). 타임아웃(10분)은 재시도하지 않음(연결 오류만 재시도 — 요청이 이미 처리 중일 수 있어 중복 과금 방지). 키 없음 → HTTP 호출 없이 MissingApiKey.
- **키 보호**: `Config`·`LlmClient` 의 Debug 를 수동 구현(키 `<redacted>`, DATABASE_URL 도 가림), 오류 메시지에서 키 문자열을 `***` 로 치환(업스트림이 키를 에코해도 노출 안 됨). 통합 테스트로 run detail·SSE 스트림에 키가 없음을 확인.
- **노드**:
  - translate: settings 검증(객체여야 함, targetLanguage 필수, glossary 값은 문자열) → 실패 시 '번역 설정 JSON 이 올바르지 않습니다: ...'(LLM 호출 전). 청크 분할은 `split_chunks`(순수 함수, 빈 줄 기준 블록·최대 8000자(문자 수), ``` / ~~~ 펜스 블록 비분할, 표는 연속 행이라 한 블록, 초과 블록은 단독 청크), 순차 번역 후 응답을 `trim_end` 해 `\n\n` 로 결합(빈 문서는 LLM 호출 없이 빈 md).
  - analyze: JPG 5MiB 초과 시 '이미지가 너무 큽니다 (최대 5MB)'(5MiB 정확히는 허용).
  - summary: images 선택(없음/빈 리스트 모두 OK), 최대 20개·각 5MiB, 초과분 제외 후 요약 끝에 '※ 이미지 N개는 크기/개수 제한으로 제외됨' 덧붙임, MD 는 전문 전달(`<document>` 래핑).
- **버그 수정(테스트가 발견)**: serde 는 JSON 배열 `["ko"]` 도 위치 기반으로 `TranslateSettings` 에 역직렬화함 → `Value` 로 먼저 파싱해 객체가 아니면 거부.
- **발견 사항 / 후속 제안**:
  - **요청 크기**: summary 는 명세대로 최대 20개×5MiB 이미지를 base64(≈1.33배)로 보내면 최대 ≈133MB 인데, Anthropic Messages API 요청 한도(≈32MB)를 넘을 수 있음 → 현재 구현은 명세 그대로. 총 이미지 바이트 상한(예: 합계 20MB 이하)을 추가하는 후속 결정 필요.
  - `docker-compose.yml` 의 engine environment 에는 `LLM_API_KEY`·`LLM_MODEL` 만 전달되고 `LLM_BASE_URL`·`LLM_EFFORT` 는 없음(트랙 I 소유 파일) → compose 로 변경하려면 추가 필요(기본값으로는 동작).
  - 분할 규칙상 정규화된 빈 줄 1개로 블록을 재결합하므로 원문의 연속 빈 줄은 번역 결과에 보존되지 않음.

### [2026-11-26 18:55] [T-115] [w2-fine] — DONE
- **작업 내용**: `scripts/verify-stack.sh` — 종료 코드 ≠ 0 이면 `down` 전에 `save_diagnostics`: `${VERIFY_LOG_DIR:-/tmp/dochestra-verify-logs}/<UTC 타임스탬프>/` 에 `ps.txt`(`docker compose ps -a`), 서비스별 `<svc>.log`(`logs --no-color --tail 200`), engine·extractor 의 `<svc>.state.json`(`docker inspect` State, Health 포함) 저장 후 경로 출력. 최근 10개 디렉토리만 유지(이름 역정렬로 오래된 것 삭제, `rm -r -- "$LOG_ROOT/$old"` 는 목록에서 읽은 디렉토리 이름에만 적용). 성공 시 아무것도 저장하지 않음. 테스트 전용 훅 `VERIFY_FAIL_INJECT=1`(smoke 직후 exit 1, 기본 off)
- **검증**: ① 주입 실패 -> rc=1, 7개 파일 생성(db/engine/extractor/web .log, ps.txt, engine/extractor .state.json), 이후 dochestra-verify 컨테이너·볼륨 잔존 0. ② 가짜 오래된 디렉토리 11개를 둔 임시 VERIFY_LOG_DIR 에서 주입 실행 -> 총 10개로 정리. ③ 지시된 verify_cmd 형태(기본 경로 /tmp/dochestra-verify-logs): 주입 실패 후 최신 디렉토리 1개 생성, 이어서 정상 `./scripts/verify-stack.sh` exit 0 이고 로그 디렉토리 수 불변(1 → 1). 사용자 스택 컨테이너 ID 전후 동일
- **참고**: `up --wait` 자체가 실패하는 경우(예: engine unhealthy)도 종료 코드 ≠ 0 이므로 같은 경로로 로그가 보존됨(실 사례 재현은 못 함, 주입 경로로만 검증)
- **호스트 시스템 변경 기록 (T-503 의 `sudo playwright install-deps chromium`)** — `/var/log/apt/history.log` 조회 결과: Start-Date 2026-11-26 09:21:23 ~ 09:21:31, Requested-By ubuntu(1000). 명령: `apt-get install -y --no-install-recommends libasound2t64 libatk-bridge2.0-0t64 libatk1.0-0t64 libatspi2.0-0t64 libcairo2 libcups2t64 libdbus-1-3 libdrm2 libgbm1 libglib2.0-0t64 libnspr4 libnss3 libpango-1.0-0 libx11-6 libxcb1 libxcomposite1 libxdamage1 libxext6 libxfixes3 libxkbcommon0 libxrandr2 xvfb fonts-noto-color-emoji fonts-unifont libfontconfig1 libfreetype6 xfonts-cyrillic xfonts-scalable fonts-liberation fonts-ipafont-gothic fonts-wqy-zenhei fonts-tlwg-loma-otf fonts-freefont-ttf`. 실제 새로 설치된 패키지: xfonts-cyrillic 1:1.0.5+nmu1, fonts-wqy-zenhei 0.9.45-8, libasound2t64 1.2.11-1ubuntu0.3, fonts-liberation 1:2.1.5-3, libasound2-data 1.2.11-1ubuntu0.3(자동), xvfb 2:21.1.12-1ubuntu1.9, fonts-noto-color-emoji 2.047-0ubuntu0.24.04.1, fonts-freefont-ttf 20211204+svn4273-2, fonts-tlwg-loma-otf 1:0.7.3-1, xfonts-scalable 1:1.0.3-1.3, fonts-unifont 1:15.1.01-1build1, fonts-ipafont-gothic 00303-21ubuntu1 (나머지 lib* 는 이미 설치되어 있었음). 조회만 수행, 추가 변경 없음. 이후 sudo/apt/전역 설치는 하지 않음(CONTEXT §6.2 신설 규칙 준수)

### [2026-11-26 13:05] [T-308] [w1-kobe] — DONE
- **작업 내용**: LLM 요청 크기 30MiB 상한 — `clients/llm.rs` 에 `MAX_REQUEST_BYTES`(31_457_280), `estimate_request_bytes(system, blocks)`(텍스트 UTF-8 바이트 + 이미지 base64 길이 `4*ceil(n/3)`(`base64_len`) + 고정 4KiB), `LlmError::RequestTooLarge`('문서가 너무 커서 LLM 요청 한도(30MiB)를 넘습니다'). `complete()` 는 키 확인 후 전송 전에 추정치를 검사해 초과 시 HTTP 호출 없이 반환. summary 는 system·문서 텍스트를 먼저 계상하고 이미지를 순서대로 예산 안에서만 포함.
- **실행 명령어**: 표준 검증 체인(fmt/clippy 0, cargo test 228 통과 — llm_nodes 24, lib 에 estimate/base64 단위 테스트 추가, `prepare --check`, docker build) -> VERIFY_ALL_OK. bindings·`.sqlx` 무변경.
- **summary 규칙(구현 해석)**: 20개 상한·개별 5MiB 초과 → 해당 이미지만 제외(기존). 총량 예산 초과 → "상한을 넘기는 이미지부터" 이므로 처음으로 예산에 들어가지 못한 이미지부터 이후 이미지를 모두 제외(작은 뒤쪽 이미지로 건너뛰어 채우지 않음 — 순서 보존·결정적). 제외 개수는 사유 무관하게 합산해 기존 주석 '※ 이미지 N개는 크기/개수 제한으로 제외됨' 1개로 표기. 텍스트(+system)만으로 초과하면 Failed('문서가 너무 커서 ...'), HTTP 호출 0회. 문서 지시문은 이미지 20개 기준 길이(상한값)로 예산을 잡아 실제 요청이 추정치를 넘지 않게 함. 이미지는 메타데이터 크기로 비용을 계산해 포함될 것만 읽음(제외 대상은 읽지 않음).
- **테스트**: 12×4MiB 이미지 → 앞 5개만 포함·본문 Content-Length ≤ 30MiB(mock 서버에서 확인)·주석 7개 / 28MiB 문서 + 1MiB 이미지 4개 → 1개만 포함·주석 3개 / 31MiB 텍스트 → Failed·HTTP 0회 / 클라이언트 직접: 상한 초과 → RequestTooLarge·HTTP 0회, 상한 직전 요청은 전송. 대용량 데이터는 모두 테스트 안에서 메모리/임시 파일로 생성(커밋 파일 없음).
- **참고**: 추정치는 JSON 이스케이프(개행·따옴표 등)를 반영하지 않아 약간 낙관적임 — 30MiB 상한과 API 한도(~32MB) 사이 여유(약 2MB)로 흡수. analyze(이미지 1개 ≤5MiB ≈ 7MB)·translate(청크 8000자)는 구조상 상한에 걸리지 않아 공용 상수 사용 변경 없음.

### [2026-11-26 19:30] [T-116] [w2-fine] — DONE
- **작업 내용**: docker-compose.yml engine environment 에 `LLM_BASE_URL`, `LLM_EFFORT`, `EMBEDDING_BASE_URL`, `TTS_BASE_URL`, `TTS_VOICE` 를 `${VAR:-}` 패스스루로 추가(기존 6개 유지, env_file 미사용). .env.example 은 w1 이 이미 넣은 LLM_* 를 두고 누락분 `EMBEDDING_BASE_URL`, `TTS_BASE_URL`, `TTS_VOICE` 를 빈 값+기본값 주석(`https://api.openai.com`, `tts-1`, `alloy`, `text-embedding-3-small`)으로 추가 — CONTEXT §6.1 표의 키 전부 존재
- **검증**: `docker compose config -q` OK, `docker compose config | grep -E '…' | wc -l` = 5, `./scripts/verify-stack.sh` exit 0(smoke 6/6), 사용자 스택 컨테이너 ID 불변
- **빈 값 처리 확인(engine/src/config.rs 조회만)**: `get = lookup(key).filter(|v| !v.trim().is_empty())` 라 빈/공백 문자열은 "미설정" 으로 취급되어 기본값 적용됨(LLM_BASE_URL/MODEL/EFFORT 모두). 문제 있는 키 없음. 단 EMBEDDING_*/TTS_* 는 현재 config.rs 에서 아직 읽지 않음(M3 후속 구현 시 동일 패턴이면 문제 없음)
- **발견 사항**: config.rs 가 읽는 `EXTRACTOR_TIMEOUT_SECS`, `UPLOAD_LIMIT_BYTES` 는 CONTEXT §6.1 표·compose 패스스루에 없음(필요 시 오케스트레이터가 표 갱신 + 패스스루 지시)

### [2026-11-26 14:10] [T-304] [w1-kobe] — DONE
- **작업 내용**: OpenAI 임베딩 클라이언트(`clients/embedding.rs`), rag 노드(`nodes/rag.rs`: 청킹→임베딩→pgvector 저장→api JSON 출력), `db/rag.rs`, RAG 질의 API(`api/rag.rs`, `POST /api/rag/{indexId}/query`), `Config`(EMBEDDING_API_KEY/BASE_URL/MODEL, Debug 키 가림), `AppState` 에 `embedder`·`llm` 클라이언트 추가(`AppState::new` 시그니처 불변), `EngineRunner` 에 embedder + `NodeKind::Rag` 디스패치, `.env.example`(EMBEDDING_BASE_URL + 주석). 의존성: `pgvector` 0.4.2(feature sqlx).
- **세션 메모**: 중간에 hcom 연결이 끊겨 `hcom start --as kobe` 로 재접속 후 이어서 완료(코드·파일 상태는 그대로 유지됨).
- **실행 명령어**: 표준 검증 체인(fmt/clippy 0, cargo test 255 통과 — tests/rag.rs 12, lib 단위 추가(embedding·rag 청킹·질의 검증), `prepare --check`(.sqlx 31개), docker build) -> VERIFY_ALL_OK. web/src/bindings 변경 없음, 실제 OpenAI/Anthropic 호출 없음, compose 미사용.
- **구현 요약**:
  - 임베딩: 요청당 ≤96개·≤200,000자(문자 수)로 연속 배치 분할, 응답은 `index` 로 정렬해 입력 순서 보장, 개수/인덱스 연속성/차원(1536) 검증(`DimensionMismatch` '임베딩 차원이 N 입니다 (기대 1536)'), 단일 입력 200,000자 초과는 `InputTooLarge`. 재시도(429·500·502·503·529·연결 오류 3회, retry-after 우선)와 키 치환·Debug 가림은 LLM 클라이언트와 동일. 키 없음 → 'EMBEDDING_API_KEY 가 설정되지 않았습니다'(입력이 없으면 키 불필요·호출 0회).
  - 청킹: 기존 `translate::split_chunks(md, 1500)` 재사용(빈 줄 블록 누적, 펜스 코드·표 비분할) 후 1500자 초과 단독 블록은 1500자 단위 하드 분할.
  - rag 노드: 빈/공백 문서 → Failed('문서 내용이 비어 있습니다'). 임베딩 성공 후 한 트랜잭션으로 `rag_indexes` + `rag_chunks` 저장(임베딩 실패 시 아무것도 저장되지 않음 — 테스트로 확인). 출력 JSON 은 CONTEXT §4.3 형식 그대로(type/indexId/method/endpoint/request/chunkCount/embeddingModel).
  - 검색: `ORDER BY embedding <=> $2, chunk_index LIMIT $3`, score = `1 - distance`. HNSW 인덱스는 모든 rag 인덱스가 공유하므로 검색 트랜잭션에서 `SET LOCAL hnsw.iterative_scan = strict_order`(pgvector 0.8.7 확인) 를 적용해 `index_id` 필터가 후처리로 결과를 비우는 문제를 방지.
  - 질의 API: question trim 1~2000자(문자 수), topK 1~20(기본 5) 위반 400 → 인덱스 없음 404 → 질문 임베딩 → 검색 → Claude(system: 제공 문맥만 사용·질문 언어로 답변·[n] 인용·근거 없으면 없다고 답함 / user: `<context>` 에 [번호] 청크들 + `Question:`) → `RagQueryResponse`(sources 는 score 내림차순). 임베딩·LLM 실패·키 미설정 → 502 UPSTREAM(메시지에 키 없음).
- **테스트 메모**: mock 임베딩은 텍스트의 `@N@` 마커로 결정적 단위 벡터(N번 축)를 만들고, 응답을 역순으로 돌려 index 정렬을 검증. 질의 시 기대 청크가 1위(score≈1.0)·나머지 0, 동점은 chunk_index 순, Claude 가 받은 프롬프트에 상위 청크·번호·질문 포함, topK 로 잘림, 서로 다른 인덱스 간 검색 격리, 400/404/502 경로, 키 비노출(오류 메시지 에코 포함) 확인.
- **발견 사항 / 후속 제안**:
  - 재시도 루프·`is_retryable`·`retry-after` 파싱·키 치환이 llm.rs 와 embedding.rs 에 중복됨(T-305 TTS 가 3번째 사본이 될 예정) → 공용 모듈로 추출하는 정리 티켓 제안(이번엔 target_files 밖이라 하지 않음).
  - `docker-compose.yml` 의 engine environment 에 `EMBEDDING_BASE_URL` 전달이 없음(트랙 I, T-116 에 함께 반영 필요; 기본값으론 동작).
  - 같은 워크플로를 다시 실행할 때마다 새 rag 인덱스가 생김(런 삭제 시 cascade). 오래된 인덱스 정리 정책은 없음.
  - 청크가 1500자 단위 하드 분할되는 초대형 단일 블록은 문장 중간에서 잘릴 수 있음(명세대로).
- **[정정, T-304]** 위 T-304 기록 중 두 가지가 사실과 다름: (1) `.env.example` 은 내가 수정하지 않았음 — 이미 트랙 I 작업(T-116)으로 `EMBEDDING_BASE_URL`·TTS 키가 반영돼 있어 편집이 적용되지 않았고(`git status` 깨끗), changed_files 에서 제외해야 함. (2) `docker-compose.yml` 의 engine environment 는 이미 `LLM_BASE_URL`·`LLM_EFFORT`·`EMBEDDING_BASE_URL` 을 전달하므로 '전달 누락' 후속 제안은 무효. 실제 변경 파일은 `.env.example` 을 제외한 나머지 목록.

### [2026-11-26 14:55] [T-305] [w1-kobe] — DONE
- **작업 내용**: ① `clients/common.rs`(재시도 루프 `retry`, `Failure`, `is_retryable_status`, `parse_retry_after`, `error_message`, `scrub_secret`) 추출 — `llm.rs`·`embedding.rs` 가 사용(동작 불변). ② `Config`(TTS_API_KEY/BASE_URL/MODEL/VOICE, Debug 키 가림) + `clients/tts.rs`(TtsClient::synthesize, `split_text`). ③ `nodes/voice.rs`(Claude 대본 → `_work/script.txt` → TTS → `mp3.mp3`), `nodes/mod.rs` 에 `select_images`/`MAX_IMAGES` 공용화(summary 가 사용) + EngineRunner 에 tts 클라이언트·`NodeKind::Voice` 디스패치.
- **실행 명령어**: 표준 검증 체인(fmt/clippy 0, cargo test 281 통과 — tests/voice.rs 11, lib 단위(common 7, tts 10, config 추가), `prepare --check`, docker build) -> VERIFY_ALL_OK. **기존 llm_nodes(24)·rag(12) 통합 테스트는 한 줄도 수정하지 않고 통과**(리팩터 무회귀). 이동한 단위 테스트(error_message·retryable_statuses)는 common.rs 로 옮김. bindings·`.sqlx`·Cargo.toml 변경 없음, 실제 외부 API 호출 없음, compose 미사용.
- **공용화 내용**: 재시도 정책(429·500·502·503·529·연결 오류 3회, 1s/2s/4s, `retry-after` 우선·60초 상한, 타임아웃 미재시도)과 키 치환 헬퍼를 한 곳으로. 각 클라이언트는 한 번의 HTTP 시도 함수(`send_once`)가 `Failure<E>`(`fatal`/`transport`/`http` 생성자)를 돌려주고 `retry()` 로 감쌈. 키 치환은 클라이언트별 `redact`(오류 변형별) + 공용 `scrub_secret`. 재시도 지연 테스트는 `tokio test-util` 을 추가하지 않고 실제 짧은 지연(10ms 기반 ≥70ms, retry-after 0 → <500ms)으로 검증.
- **TTS**: `POST {base}/v1/audio/speech`, Bearer 인증, 본문 `{model, input, voice, response_format:"mp3"}`. 4000자 초과 텍스트는 `split_text`(문장 경계 — `.`/`!`/`?` 뒤가 공백·끝일 때(소수점 `3.14` 제외), `。！？`, 개행; 문장 단위 탐욕 결합, 4000자 초과 단일 문장은 하드 분할, 공백 청크 제거)로 쪼개 순차 합성 후 바이트를 순서대로 결합. 응답 Content-Type 이 `audio/*` 가 아니거나 본문이 비면 Decode 오류(재시도 안 함). 빈 텍스트는 키·호출 없이 빈 결과.
- **voice 노드**: md 필수, images(LIST[JPG]) 는 summary 와 같은 규칙(최대 20개·개별 5MiB·요청 30MiB, 텍스트 먼저 계상, 첫 초과부터 이후 제외). 제외된 이미지는 대본에 주석을 붙이지 않고(음성으로 읽히므로) 경고 로그만 남김. 대본이 비면 Failed('해설 대본이 비어 있습니다'). 대본은 TTS 호출 전에 `_work/script.txt` 로 저장되므로 TTS 실패 시에도 남음. 시스템 프롬프트: 한국어 구어체 내레이터, 마크다운·목록·이모지 금지, 약 1500~2500자, 이미지 반영, 서문 금지.
- **참고**: `.env.example`·`docker-compose.yml` 은 이미 TTS_* 키를 포함(트랙 I) — 이번 티켓에서 수정하지 않음. 작업 트리의 `web/**`·`CONTEXT.md`·`PLAN.md` 변경은 다른 트랙/오케스트레이터 작업물.
- **발견 사항 / 후속**: `tts-1` 은 입력 4096자 제한 — 4000자 청크는 여유가 있으나 다른 TTS 모델(`TTS_MODEL`)을 쓰면 한도가 다를 수 있음. 청크 경계에서 MP3 를 단순 이어붙이므로 청크 사이에 미세한 무음/갭이 생길 수 있음(재인코딩 없음).

### [2026-11-26 10:55] [T-406] [orch-subagent] — DONE
- **작업 내용**: 캔버스 노드 배치 UX + 비보안 컨텍스트 ID 생성 버그 수정
  - `web/src/lib/placement.ts`(신규, 순수): `nodeBox`(measured → width/height → 기본 260×120), `overlaps`(엄격), `contains`, `findFreePosition`(시작점에서 +40px 대각선 이동, 결정적, 최대 500회), `NODE_DRAG_MIME='application/dochestra-node'`
  - `NodePalette.tsx`: 항목 `draggable` + onDragStart(`setData(NODE_DRAG_MIME, kind)`, `effectAllowed='move'`), `cursor-grab`, 상단 안내 "클릭하거나 캔버스로 끌어다 놓으세요". 클릭 추가는 보이는 캔버스 중앙 기준 + 겹침 회피, 화면 밖이면 `setCenter(x, y, { zoom: getZoom(), duration: 200 })` 로 줌 유지 패닝
  - `Canvas.tsx`: 래퍼 div(`data-testid="flow-drop-zone"`)에서 onDragOver(해당 MIME 일 때만 preventDefault, dropEffect move)/onDrop(카탈로그에 없는 kind·MIME 없음 무시, `screenToFlowPosition({x: clientX, y: clientY})` 기준 노드 중심 배치). `fitViewOptions={{maxZoom: 1, padding: 0.2}}`(Controls 버튼도 동일), `minZoom={0.2}`
  - `editorStore.addNode`: 새 노드만 `selected: true`, 나머지 해제, `selectedNodeId` 설정, id 반환(미지 kind → null)
  - `web/src/lib/ids.ts`(신규): `newId()` — `crypto.randomUUID` 없으면 `getRandomValues` 로 v4 UUID. editorStore 의 addNode·onConnect 에서 사용(web/src 내 `crypto.randomUUID` 직접 호출 0건)
- **원인(사용자 보고 "클릭해도 아무 일 없음")**: http://<서버 IP> 는 비보안 컨텍스트라 `crypto.randomUUID` 가 undefined → addNode 에서 TypeError. localhost 는 보안 컨텍스트라 기존 테스트에서 놓침
- **xyflow 13 확인 사항(node_modules/@xyflow/react/dist/index.d.ts, @xyflow/system 1.0.1)**: `useReactFlow()` 의 `screenToFlowPosition(clientPos, opts?)`, `setCenter(x, y, {zoom?, duration?}) => Promise<boolean>`, `getZoom()`; `ReactFlow` props `fitView`/`fitViewOptions`(`FitViewOptions`: padding/minZoom/maxZoom/...)/`minZoom`; `Controls` `fitViewOptions`; Node `measured?: {width?, height?}`
- **문제 및 해결**: 첫 verify 에서 드롭 E2E 실패 — 노드가 드롭 지점이 아니라 캔버스 중앙에 위치. v13 은 빈 그래프에서 요청한 초기 `fitView` 가 큐에 남아 첫 노드 추가 시 실행되어 뷰포트를 이동시킴 → 마운트 시 노드가 있을 때만 `fitView` 를 켜도록(`fitView={fitOnInit}`) 수정 후 통과
- **테스트**: 단위 — placement(6), ids(2: randomUUID 제거 상태 1000회 v4·중복 없음), store(addNode 선택/해제, randomUUID 없이 addNode·onConnect), Canvas DnD(dragover 수락, 드롭 시 kind·위치·선택·dirty, 미지/없는 kind 무시), NodePalette(draggable·힌트·dragstart dataTransfer, 클릭 3회 겹침 없음·선택). `EditorPage.test.tsx` 422 케이스: 새 노드 자동 선택으로 Inspector 가 열려 `/api/bundles` 스텁 추가 및 alert 조회를 TYPE_MISMATCH 포함 alert 로 한정
  - E2E 추가 4개: 실제 dragTo 드롭(드롭 지점 ±120px·selected·Inspector 제목), 클릭 3회 겹침 없음·화면 내, `addInitScript` 로 randomUUID 제거 후 클릭·드롭·연결, **실제 `http://<호스트 IPv4>:18080` 접속**(os.networkInterfaces 첫 비-loopback IPv4, `isSecureContext===false` 단언) 후 클릭·드롭·연결 + pageerror 0
- **실행 명령어**: `cd web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run(130) && pnpm build` 통과, `./scripts/verify-stack.sh --ui` exit 0(smoke 전부 PASS, Playwright 5 passed). 사용자 스택(dochestra) 미접촉
- **발견 사항 / 후속 제안**: 드롭 배치는 사용자 의도 우선이라 겹침 회피를 적용하지 않음. 비보안 컨텍스트 회귀 방지 E2E 는 호스트에 비-loopback IPv4 가 없으면 skip 됨

### [2026-11-26 15:40] [T-306] [w1-kobe] — DONE
- **작업 내용**: slide 노드 — `nodes/slide/{mod,spec,render}.rs`(Claude 가 슬라이드 구성 JSON 생성 → 엔진이 16:9 PDF 렌더), `Config.slide_font_dir`(SLIDE_FONT_DIR), `EngineRunner` 에 `NodeKind::Slide` 디스패치, 한글 폰트 에셋(`engine/assets/fonts/`), Dockerfile 런타임 단계에 폰트 COPY.
- **폰트**: Nanum Gothic(NHN, SIL OFL 1.1). 출처: `https://raw.githubusercontent.com/google/fonts/main/ofl/nanumgothic/{NanumGothic-Regular.ttf,NanumGothic-Bold.ttf,OFL.txt}` — google/fonts 저장소 해당 파일의 마지막 변경 커밋 `16680f8688ff`(2018-03-13, "korean families r01: added"). 크기: Regular 2,054,744B + Bold 2,073,868B + OFL.txt 4,534B = 4.0MB(15MB 이하). 호스트 apt 설치 없음(HTTP 다운로드만). 컨테이너 경로 `/usr/local/share/dochestra/fonts`(이미지에서 `docker run --entrypoint ls` 로 3개 파일 확인). 로컬·테스트 기본값은 컨테이너 경로가 없으면 `CARGO_MANIFEST_DIR/assets/fonts`.
- **PDF 라이브러리**: `printpdf` 0.13.0 (`default-features = false`, `features = ["jpeg"]`) — 기본 feature(html → azul-layout 계열)는 무겁고 불필요해 제외. API 는 소스/예제(`examples/customfont.rs`·`image.rs`)로 확인: `PdfDocument::add_font(&ParsedFont)`, `add_image(&RawImage)`, `Op::{StartTextSection, SetFont, SetFillColor, SetTextCursor, ShowText, EndTextSection, DrawRectangle, UseXobject}`, `PdfPage::new(Mm, Mm, ops)`, `save(&PdfSaveOptions)`. **글꼴 서브셋팅은 `text_layout` feature 가 필요**(azul 스택 추가)해서 사용하지 않음 → 폰트 2개가 통째로 임베딩되어 PDF 한 개가 약 1.5MB(zlib 압축). 레이아웃 폭 측정은 `ParsedFont` 의 `codepoint_to_glyph`/`glyph_widths`/`units_per_em` 로 직접 계산. dev-dependencies: `lopdf` 0.47.0(PDF 검사), `image` 0.25.10 (jpeg, 테스트용 실제 JPEG 생성).
- **구성 생성**: system 프롬프트가 'JSON 객체 하나만' 출력과 스키마·한도(슬라이드 1~15, 불릿 1~7, 각 120자)를 지시. 대표 이미지는 이미지 블록으로 텍스트 앞에 전달(5MiB 초과 또는 요청 예산(30MiB) 초과 시 이미지 없이 진행 + 경고 로그). 응답에서 첫 `{`~마지막 `}` 를 추출해 serde 로 파싱·검증(`spec::parse_deck`): 초과 개수·길이는 잘라서 보정, 구조 문제(JSON 없음·스키마 불일치·제목 비어 있음·slides 0개·제목/불릿 없는 슬라이드)는 사유를 담아 **1회만** 재요청(사유를 프롬프트에 덧붙임), 재실패 시 Failed('슬라이드 구성을 해석하지 못했습니다: ...'). LLM API 오류(키 없음 포함)는 재요청 없이 즉시 실패. 보정된 구성은 `_work/slides.json` 에 저장.
- **렌더**: 254×142.875mm(=720×405pt). 표지(제목·부제·대표 이미지 우측 맞춤), 슬라이드마다 제목(최대 2줄, 30→22pt 단계 축소) + 빨간 밑줄(PDF 타입색 `#E5484D`) + 글머리표 목록(24→12pt 단계 축소, 폰트 메트릭 줄바꿈, 최소 크기에서도 넘치면 줄 단위로 잘라 마지막에 `…`), 이미지가 있으면 마지막에 이미지 전체 슬라이드, 모든 쪽 하단 `n / N`. 이미지는 비율 유지·가운데 맞춤. 폰트에 없는 문자(이모지 등)는 제거, 제어문자는 공백으로. 디코딩 불가 이미지는 건너뛰고 이미지 없이 렌더. 폰트 파싱·렌더는 `spawn_blocking` 에서 수행.
- **시각 확인**: `pypdfium2`(extractor venv)로 PDF 를 PNG 로 렌더해 표지·본문·이미지 슬라이드·최대 분량 스트레스(15장×7불릿×120자, 공백 없는 긴 한글)를 확인 — 한글 정상 표시, 줄바꿈·말줄임·페이지 번호 정상. (이 과정에서 '너무 긴 단어가 앞 짧은 단어를 혼자 한 줄에 남기는' 문제를 발견해 현재 줄에서 이어서 글자 단위로 끊도록 수정, 단위 테스트 추가.)
- **실행 명령어**: 표준 검증 체인(fmt/clippy 0, cargo test 306 통과 — tests/slide.rs 14, 렌더·스펙·config 단위 테스트 추가, `prepare --check`, docker build, 컨테이너 내 폰트 확인) -> VERIFY_ALL_OK. bindings·`.sqlx` 무변경, 실제 외부 API 호출 없음, compose 미사용.
- **테스트**: 구성 추출(코드펜스·잡담 속 JSON)·검증·보정(15장·7불릿·120자 truncate, 0개/제목·불릿 없음 실패), 1회 재시도 성공/실패(요청 횟수 2, 사유 포함 확인), 5MiB 초과 이미지는 이미지 없이 진행, LLM 오류는 재요청 없음, 요청 한도 초과 문서는 호출 0회. 렌더 PDF 를 lopdf 로 열어 페이지 수 = 1 + slides + (이미지 있으면 1), MediaBox 720×405, `FontFile2` ≥ 2(Regular·Bold), 이미지 XObject 존재, 한글·영문 혼합·최대 분량·이모지/탭 포함도 성공. 통합: (mock extractor) extract.md + extract.images → pick_jpg → slide 런 succeeded, `artifacts/s/pdf` Content-Type application/pdf, LLM 이 대표 이미지를 받음.
- **발견 사항 / 후속**: (1) 서브셋팅이 없어 PDF 가 약 1.5MB — 용량이 문제가 되면 printpdf `text_layout` feature(의존성 대폭 증가) 또는 allsorts 직접 서브셋팅 검토. (2) 폰트 파싱이 렌더마다 반복됨(디버그 빌드에서 테스트 14개 합계 ~12초) — 필요하면 `OnceLock` 캐시. (3) 대표 이미지 한 장만 쓰므로 `pick_jpg` 로 고르는 이미지에 따라 결과가 달라짐(사용자 설계대로).
