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

### [2026-11-26 21:10] [T-502] [w2-fine] — DONE
- **작업 내용**: README.md 작성(한국어; 소개, 아키텍처 표·흐름도, 빠른 시작, 환경 변수 표, 사용법, 노드 12종 표, 개발·검증, 문서 지도, 알려진 제약). 코드 변경 없음
- **검증**: 지시된 verify_cmd 의 링크 추출 -> 상대 링크 9개(.env.example, CONTEXT.md, docs/RFP.md, PLAN.md, WORKLOG.md, CLAUDE.md, docs/agents/*.md) 전부 존재(MISSING 0). 본문 명령의 경로(engine/scripts/dev-db.sh, web/e2e, scripts/{smoke,e2e,verify-stack}.sh)도 존재 확인. 컨테이너·기본 스택 명령 미실행, `.env` 열람·수정 없음
- **기술 근거(저장소에서 확인)**: 호스트 공개 포트 web 만(compose `${WEB_PORT:-80}:80`), 업로드 200 MiB(CONTEXT §4.1.3 / nginx), extractor RSS ≈1.5GiB·mem_limit 4g(T-113 측정), 이미지 ~6GB(T-111), verify-stack 옵션 `--e2e/--ui`·로그 `/tmp/dochestra-verify-logs`(T-115), 팔레트 드래그&드롭·Delete/Backspace 삭제(Canvas.tsx onDrop, T-406·T-403). 노드별 외부 AI 사용은 engine/src/nodes 의 llm/embedding/tts 사용 여부로 판단(translate·analyze·summary·slide·fill=LLM, voice=LLM+TTS, rag=임베딩+LLM)
- **T-309 진행 중 메모**: README 환경 변수 표는 CONTEXT §6.1 개정본(OPENROUTER_API_KEY / OPENROUTER_BASE_URL / LLM_MODEL·LLM_EFFORT / EMBEDDING_MODEL(1024차원) / TTS_MODEL·TTS_VOICE)을 그대로 기술. 현재 `.env.example`·docker-compose.yml 은 이전 키(LLM_API_KEY, LLM_BASE_URL, EMBEDDING_API_KEY, EMBEDDING_BASE_URL, TTS_API_KEY, TTS_BASE_URL …)가 남아 있어 README 와 불일치 — T-309(w1 엔진 전환) 반영 후 `.env.example` 과 compose engine 패스스루를 개정 계약으로 맞추는 후속 티켓이 필요(README 에는 "아직 .env.example 이 이전 키 이름이면 표 기준" 안내 문구를 넣어 둠, 정리 후 삭제 권장)
- **발견 사항 / 후속 제안**: 위 .env.example/compose 키 정리; PLAN 의 T-502 선행 표기(T-501) 외 README 가 verify-stack 옵션을 문서화하므로 옵션 변경 시 함께 갱신

### [2026-11-26 22:20] [T-407] [w2-fine] — DONE
- **작업 내용**: 번들 업로드 UX 개선 — BundlePicker: 상단 상시 파란 `PDF 업로드` 버튼(숨은 `<input type=file hidden aria-label="PDF 파일 선택">` 트리거) + 점선 드롭 존(`data-testid=bundle-dropzone`, 드래그 중 파란 테두리). 번들 미선택이면 자동 생성(이름: 첫 파일명에서 `.pdf` 제거, 여러 개면 `<첫 파일> 외 N개`, 100자 제한) → onChange(newId) → 업로드. 업로드 중 버튼 비활성 + `업로드 중… (N개)`, 실패/비PDF 는 role=alert(`PDF 파일만 올릴 수 있습니다`, 비PDF 만이면 요청 0회). 파일 0개 문구 `아직 업로드한 PDF 가 없습니다 — 위 버튼을 누르거나 PDF 를 끌어다 놓으세요`. 기존 드롭다운·`새 번들` 은 `다른 번들 선택` 보조 영역으로. Inspector: bundleId 가 없으면 validateParams 경고 대신 비-alert 안내 `PDF 를 업로드하면 번들이 자동으로 만들어집니다`(형식 오류·다른 kind 경고는 유지). README 사용법 4번 갱신(임시 안내 문구는 그대로)
- **검증**: `pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run(139) && pnpm build` 통과. `./scripts/verify-stack.sh --ui` exit 0 — smoke 6/6 + Playwright 5개(배치 DnD 2, 기존 메인 시나리오 1(step 4 를 새 흐름으로 교체), 비보안 컨텍스트 2) 전부 통과. 사용자 스택 ID 불변, 잔존 0
- **테스트 변경**: BundlePicker 8개(버튼·빈 목록 문구, 선택 번들 업로드(생성 없음, multipart 'files'), 자동 생성 순서·이름·onChange, `외 N개`, 100자 제한, 드롭, 비PDF 드롭, 진행 중 비활성·표시, 실패 alert), Inspector 3개(안내 문구, 형식 오류 경고 유지, 타 kind 경고). E2E step 4: 번들 노드 선택 → 안내 문구·`PDF 업로드` 버튼 가시 확인 → 숨은 input 에 `setInputFiles`(버퍼, 파일명에 RUN_TAG 포함해 afterEach 정리가 자동 생성 번들을 찾도록) → 목록·드롭다운에 번들명 표시 → 기존 시나리오 계속
- **메모**: 숨은 input 의 aria-label 을 버튼 이름과 구분('PDF 파일 선택')해 접근성/선택자 모호성 제거. 두 번째 업로드부터는 선택된 번들에 추가(자동 생성은 번들 미선택일 때만)

### [2026-11-26 12:10] [T-117] [orch-subagent] — DONE (이미지 축소는 디스크 부족으로 미실시, 아래 제안)
- **작업 내용**: extractor 를 CUDA torch 로 전환하고 Docling GPU 가속을 적용
  - `pyproject.toml`: 인덱스 `pytorch-cpu` → `pytorch-cu134` (`https://download.pytorch.org/whl/cu134`, explicit). 인덱스 목록 확인 결과 torch 2.15.0+cp312 linux 휠이 있는 인덱스는 cu132·cu134 뿐(cu126~cu131·cu133 없음). torchvision 0.29.2 는 둘 다 py3-none 휠 존재. 드라이버 620.70 = CUDA UMD 13.5 이므로 cu134 선택. `uv lock` → torch 2.15.0+cu134, torchvision 0.29.2+cu134, triton 3.9.0, nvidia-* 15종(cublas 13.8, cudnn-cu13 9.26, nccl-cu13 2.30.7 등) 추가. `torch.cuda.get_arch_list()` 에 sm_75(T4) 포함 확인, 호스트 venv 에서 T4 행렬곱 성공
  - `app/converter.py`: `resolve_device()`(EXTRACTOR_DEVICE=auto|cuda|cpu, 대소문자·공백 무시, 빈 값=auto; auto 는 `torch.cuda.is_available()` 이면 cuda, 아니면 WARNING 후 cpu; cuda 인데 GPU 없음/잘못된 값 → `DeviceError`), `build_converter(device)` 가 `PdfPipelineOptions(accelerator_options=AcceleratorOptions(device=AcceleratorDevice.CUDA|CPU))` 사용. API 확인 위치: `docling/datamodel/accelerator_options.py`(`AcceleratorDevice` enum AUTO/CPU/CUDA/MPS/XPU, `AcceleratorOptions(BaseSettings)` 필드 num_threads/device/cuda_use_flash_attention2), `PdfPipelineOptions.model_fields` 에 `accelerator_options` 존재
  - `app/main.py`: 초기화 스레드에서 장치를 먼저 정함(DeviceError → ERROR 로그 후 종료, /health 는 계속 503 loading). 시작 시 `extractor device: <dev>` INFO 로그, `/health` 200 = `{"status":"ok","device":"cuda"|"cpu"}`. uvicorn 이 우리 로거(`extractor`, `app.*`)에 핸들러를 붙이지 않아 INFO 가 안 보이던 문제 → `_configure_logging()` 으로 stderr 핸들러(INFO) 추가
  - `app/schemas.py`: `HealthResponse.device: Literal["cuda","cpu"] | None = None`
  - `Dockerfile`: 베이스 python:3.12-slim-bookworm 유지(CUDA 런타임은 nvidia-* 휠), ENV `NVIDIA_VISIBLE_DEVICES=all`, `NVIDIA_DRIVER_CAPABILITIES=compute,utility`, `EXTRACTOR_DEVICE=auto` 추가. uid 10001·모델 사전 다운로드·HF_HUB_OFFLINE 은 그대로
  - `docker-compose.yml`(extractor 만): `EXTRACTOR_DEVICE: ${EXTRACTOR_DEVICE:-auto}`, `deploy.resources.reservations.devices: [{driver: nvidia, count: all, capabilities: [gpu]}]` — Compose v5.8.0 `config -q` 통과, `count: -1`(=all)로 렌더됨. mem_limit 4g 유지(측정상 충분)
  - 테스트: `tests/test_device.py`(신규 — resolve_device 매트릭스, env 기본값, auto 폴백 경고, cuda+GPU 없음 오류, 잘못된 값, HealthResponse, 가짜 컨버터로 lifespan 실행 시 /health device, cuda+GPU 없음 → 503 유지·컨버터 미생성), `test_api.py` 의 health 단언을 `{"status":"ok","device": <cuda 가능하면 cuda>}` 로. pytest 31 passed (호스트 GPU 로 실제 변환 포함)
- **측정**
  - 컨테이너: `docker run --rm --gpus all --entrypoint python dochestra-extractor -c "...cuda..."` → `True Tesla T4`
  - 컨테이너 기동 로그 확인: auto(+GPU)→cuda, cpu→cpu, cuda(+GPU)→cuda, cuda(GPU 없음)→ERROR + /health 503 loading, auto(GPU 없음)→WARNING 폴백 + cpu
  - 변환 시간(이미지 내부, `--memory 4g`, 컨버터 1회 초기화 후 같은 파일 2회, 스크립트 /tmp/t117/bench.py): 초기화 cuda 9.0s / cpu 6.4s
    - sample.pdf(1쪽): CUDA 4.66s(첫 실행, CUDA 워밍업) → 0.36s / CPU 2.98s → 2.43s
    - /tmp/t117/big.pdf(reportlab 생성 20쪽, 표 20·그림 20, 550KB, 미커밋): CUDA 11.24s / 11.41s, CPU 87.36s / 88.86s → 약 7.8배 빠름
    - GPU 메모리: nvidia-smi 최대 1747MiB 사용·GPU 사용률 최대 90%, torch max_allocated 862MiB / reserved 1602MiB. 대기 상태의 verify extractor 는 GPU 699MiB, RSS 약 899MiB / 4GiB
  - 이미지 크기: 이전 CPU 6.21GB → CUDA 21.6GB. 디스크: 빌드 시작 시 31G 여유 → 빌드 후 11G(사용 66G/77G), verify-stack 후 15G
- **이미지 크기 분석 (`docker history`)**: 21.6GB 중 `RUN uv sync --no-install-project` 레이어가 13.1GB = `/app/.venv` 6.3G(nvidia 3.1G, torch 1.2G) + **`/root/.cache/uv` 6.0G(이미지에 남은 uv 캐시 — CPU 이미지에도 있던 기존 낭비)**. 두 번째 `uv sync` 레이어는 28.7kB(재설치 없음), 모델 771MB, chown/useradd 53kB 라 중복 아님
  - **축소 미실시 이유**: 새 venv 레이어 빌드(+export 압축 blob + unpack)에 이전 빌드 실측(13.1GB 레이어에 약 20GB 소비) 기준 약 10~12GB가 필요해 현재 여유 15G 에서 10GB 아래로 떨어질 것이 확실 → 오케스트레이터 지시(10GB 미만이면 중단)에 따라 착수하지 않음. Dockerfile 은 검증된 이미지와 일치하도록 변경하지 않음
  - **제안(검증 전)**: 1단계 RUN 을 `RUN --mount=type=cache,target=/root/.cache/uv uv sync --frozen --no-dev --no-install-project` 로(또는 디스크 여유가 우선이면 `UV_LINK_MODE=hardlink` + `--cache-dir /tmp/uvc` 후 `rm -rf /tmp/uvc`, 같은 RUN 안). 2단계 `uv sync` 에도 같은 mount. 예상 이미지 약 7.5~8GB(venv 6.3 + 모델 0.77 + 베이스 0.2). 진행에는 빌드 중 여유 공간 약 20GB 이상 필요 — 이전 CUDA 이미지 레이어는 새 이미지로 `dochestra-extractor:latest`·`dochestra-verify-extractor` 태그가 모두 옮겨져야 해제됨
- **실행 명령어**: verify_cmd 전체(uv sync --frozen, ruff check/format, pytest 31 passed, docker build, `--gpus all` cuda assert, `docker compose config -q`, `./scripts/verify-stack.sh --e2e` → smoke·e2e 전 단계 PASS) 통과. 이후 flock 으로 `dochestra-verify` 프로젝트에 `up -d --build --wait extractor db` → `/health` = `b'{"status":"ok","device":"cuda"}'` → `down -v`. 사용자 스택(dochestra)은 건드리지 않음(ps 조회만)
- **발견 사항 / 후속 제안**: (1) 위 이미지 축소(uv 캐시 6GB 제거). (2) `docker build -t dochestra-extractor` 는 사용자 스택과 같은 태그를 덮어씀 — 사용자 컨테이너는 여전히 이전 이미지 09ad62c67680(CPU)로 실행 중이며 compose 재배포 시 CUDA 이미지 + GPU 예약이 적용됨. (3) GPU 가 없는 호스트에서 compose 의 nvidia 장치 예약이 있으면 컨테이너 생성 자체가 실패함(auto 폴백은 예약 없이 띄울 때만 동작) — 필요하면 override 파일로 분리. (4) pytest 경고 `force_full_page_ocr deprecated` 는 docling 내부 직렬화에서 나오는 기존 경고

### [2026-11-26 16:20] [T-307] [w1-kobe] — DONE
- **작업 내용**: fill 노드 — `nodes/fill/{mod,form,matching}.rs`(AcroForm 필드 인식·값 쓰기·키 매칭·LLM 매핑·리포트), `EngineRunner` 에 `NodeKind::Fill` 디스패치(카탈로그 12종이 모두 구현되어 `UnimplementedRunner` 위임 분기 제거), 의존성 `lopdf` 0.47(`default-features = false`; 기존 dev-dependency 를 일반 의존성으로 이동 — printpdf 와 같은 버전이라 중복 컴파일 없음).
- **lopdf API(소스 확인)**: `Document::load_mem`, `is_encrypted`, `catalog()/catalog_mut()`, `get_dictionary(_mut)`, `dereference`, `Object::{Name,String(bytes, StringFormat),Reference,Integer,Boolean,Array,Dictionary}`, `Dictionary::{get,set,has,iter}`, `save_to(&mut Vec)`. 저장은 전체 재작성이지만 객체 구조는 그대로 보존.
- **필드 인식**: `/AcroForm /Fields` 를 `/Kids` 재귀 순회(순환·깊이 32 방어). 전체 이름 = 부모 `/T` 를 `.` 로 연결. `/FT`·`/Ff`·`/V`·`/Opt` 상속. 터미널 판정 = `/T` 를 가진 kid 가 없으면 터미널(kid 는 위젯). Tx → Text, Btn → 푸시버튼(`Ff` 비트 17) 제외 / 라디오(비트 16, 위젯별 on-상태 목록) / 체크박스(`/AP /N` 의 Off 아닌 키, 없으면 `Yes`), Ch → `/Opt`(문자열 또는 [export, display] 쌍), Sig 제외. 현재값 `/V` 도 수집.
- **매칭**: ① 정확(전체 이름) ② 정규화(소문자·공백·`_`·`-`·`.` 제거, 전체 이름 또는 마지막 세그먼트) — 후보가 하나일 때, 또는 여러 개 중 전체 이름 일치가 정확히 하나일 때만 채택(모호하면 보류), 한 필드에 키 하나·한 키에 필드 하나, 키는 정렬 순서로 처리 ③ 남은 키·필드가 있으면 `LlmClient` 로 `{"키": "필드명"|null}` 매핑 요청 — 프롬프트에는 키와 값의 JSON **타입**(값은 절대 전송 안 함)과 남은 필드의 이름·타입·선택지만 포함. 응답이 해석 불가/API 오류/키 없음이면 경고만 남기고 이 단계 생략, 없는 필드명·없는 키 제안은 무시(+경고).
- **값 쓰기**: Tx/Ch → `/V` PDF 문자열(인쇄 가능한 ASCII 는 리터럴, 그 외는 BOM+UTF-16BE 16진 문자열), 체크박스 → `/V`·`/AS` 를 on-상태 이름 또는 `Off`(true/false, 1/0, yes/no/예/아니오 등 해석), 라디오 → 부모 `/V` + 위젯별 `/AS`(일치 위젯만 on), 선택 필드는 display 값도 export 값으로 변환·목록 밖 값은 그대로 쓰되 경고. 값 타입: string/number → 문자열, bool → 체크박스(텍스트 필드에는 "true"/"false"), null 무시, 배열/객체는 경고 후 무시. 하나라도 채우면 `AcroForm /NeedAppearances true`(AcroForm 이 직접 사전이든 간접 참조든).
- **출력·리포트**: `pdf.pdf` + `_work/fill_report.json`(`filled[{key,field,value,method}]`, `unmatchedKeys`, `untouchedFields`, `warnings`). 아무것도 못 채우면(일치 0개, 또는 일치했지만 값 적용 불가) 리포트를 쓴 뒤 Failed('입력 값과 일치하는 폼 필드가 없습니다 (필드: 앞 20개 … 외 N개)'). 그 밖의 실패: 'PDF 에 입력 가능한 폼 필드가 없습니다', '폼 값 JSON 은 객체여야 합니다', 'PDF 를 읽을 수 없습니다: …', '암호화된 PDF 는 지원하지 않습니다'.
- **픽스처**: 테스트 코드가 lopdf 로 폼 PDF 를 직접 생성(계층 이름·`/FT` 상속·체크박스(on-상태 `Yes`/`Accepted`)·라디오·콤보(문자열/쌍 옵션)·서명·푸시버튼, AcroForm 직접/간접 두 변형)하고, 별도 작성기 검증용으로 reportlab(extractor venv, 호스트 설치 없음)이 만든 `engine/tests/fixtures/form.pdf`(12.7KB) + 생성 스크립트 `make_form.py` 를 커밋. **reportlab 은 choice 옵션 튜플 `(a, b)` 를 PDF 쌍 `[b, a]` 로 기록**하므로(스펙은 [export, display]) 생성 스크립트에서 튜플을 (표시, 값) 순으로 주도록 보정했음.
- **눈으로 확인**: 채운 reportlab 폼을 PDFium(`pypdfium2`, `init_forms` + `may_draw_forms`)으로 렌더 — 한글 이름, 이메일, 여러 줄 한글 코멘트, 체크된 체크박스, 선택된 라디오(pro), 콤보(Japan) 모두 정상 표시.
- **실행 명령어 / 결과**: fmt/clippy 0, cargo test 333 통과(fill 15, 단위 fill 13 포함), `prepare --check`, docker build. bindings·`.sqlx` 무변경, 실제 외부 API 호출 없음.
- **검증 스크립트 정정**: 중간에 `cargo clippy … | tail -1 && …` 형태의 체인이 clippy 실패(테스트 lint)를 숨겨 'VERIFY_ALL_OK' 가 잘못 출력된 적이 있어, 이후 `set -euo pipefail` 스크립트(`verify-engine.sh`, 스크래치패드)로 재검증함 — 이전 티켓들의 보고는 모두 `&&` 체인이 아니라 각 단계 출력 확인과 함께 수행된 것을 확인했지만, 파이프(`| tail`/`| awk`)를 끼운 체인은 종료 코드를 가린다는 점을 기록해 둠.
- **명세 대비 차이**: `tests/nodes_run.rs` 의 `unimplemented_kinds_still_fail_while_upstream_succeeds` 는 `fill` 을 '미구현 예시'로 썼는데 이제 모든 kind 가 구현되어 의미가 없어져 삭제(target_files 밖의 테스트 수정). `Config` 는 변경 없음(fill 은 기존 LlmClient 사용).
- **참고(T-309)**: fill 의 LLM 매핑·테스트의 Anthropic mock 은 T-309(OpenRouter 전환)에서 함께 갱신 필요 — `LlmClient` 인터페이스가 유지되면 코드는 그대로이고 `tests/fill.rs` 의 mock 만 바꾸면 됨.

### [2026-11-26 17:40] [T-309] [w1-kobe] — DONE (T-307 과 함께 보고)
- **작업 내용**: 외부 AI 연동을 OpenRouter(OpenAI 호환) 단일 공급자로 전환. `Config` → `OPENROUTER_API_KEY`/`OPENROUTER_BASE_URL`(기본 `https://openrouter.ai/api/v1`)/`LLM_MODEL`(`meta/muse-ember`)/`LLM_EFFORT`(low|medium|high, 기본 medium, xhigh·max 거부)/`EMBEDDING_MODEL`(`baai/bge-m3`)/`TTS_MODEL`(`google/gemini-3.8-flash-lite-tts`)/`TTS_VOICE`(`Kore`); 구 키 6개 제거(설정돼 있어도 무시, 테스트 포함). `clients/llm.rs` 를 `POST {base}/chat/completions` 로 교체(공개 인터페이스 `complete(LlmRequest)`·`UserBlock`·`estimate_request_bytes`·`MAX_REQUEST_BYTES` 유지 → 노드 코드는 문구/주석 외 무변경). `clients/embedding.rs`: `/embeddings`, 차원 1024. `clients/tts.rs`: PCM 수신 → MP3 인코딩. `migrations/0003_embedding_1024.sql`. compose·`.env.example` 갱신.
- **LLM 계약(CONTEXT §4.3)**: 헤더 `Authorization: Bearer` + `X-Title: Dochestra`, 본문 `{model, messages:[{system},{user:[image_url data URL…, text…]}], max_tokens:16000, reasoning:{effort}}`(이미지 파트가 텍스트 앞). 응답 `choices[0].message.content`(문자열 또는 text 파트 배열 결합). 우선순위: 본문 최상위 `error`(OpenRouter 가 200 으로 업스트림 오류를 줄 수 있음 → HTTP 오류와 동일 처리: code 429/5xx 는 재시도, 400 류는 즉시 실패, code 없으면 502) → `message.refusal` 비어있지 않으면 'LLM 이 요청을 거절했습니다'(`LlmError::Refusal` 은 카테고리 없는 unit 변형) → `finish_reason == length` 'LLM 출력이 최대 길이에 도달했습니다'(추론 모델이 예산을 다 써서 content=null+length 인 경우 포함) → 비거나 null 'LLM 응답이 비어 있습니다'(`LlmError::Empty`). 재시도는 429·500·502·503·연결 오류(**529 제외** — `common::is_retryable_status` 수정), 타임아웃 미재시도.
- **임베딩**: 본문 `{model, input:[…]}`(encoding_format 제거), 배치 96개/200,000자·index 정렬·차원 검증 유지, 1024 고정. `0003`: 인덱스 DROP → `rag_chunks`/`rag_indexes` DELETE(1536차원 벡터는 변환 불가·모델 간 비교 불가, 주석으로 명시) → `vector(1024)` → HNSW 재생성. dev DB 에 `sqlx migrate run` 으로 적용, `.sqlx` 는 쿼리 텍스트가 같아 변경 없음. **운영 DB 에서는 엔진 재기동 시 자동 적용되어 기존 RAG 데이터가 삭제됨.**
- **TTS**: `POST {base}/audio/speech` `{model, input, voice, response_format:"pcm"}`; `Content-Type: audio/pcm;rate=…;channels=…` 파싱(없으면 24000/1 + 경고 로그, `audio/*` 가 아니면 Decode), 16-bit LE PCM(홀수 바이트·프레임 단위 불일치·빈 본문은 Decode 오류), 4000자 문장 경계 청크별 PCM 을 순서대로 이어 붙이되 청크 간 rate/channels 불일치는 오류 → **한 번에 MP3 인코딩**(mono 64kbps / stereo 128kbps, LAME quality Good, `spawn_blocking`). `TtsError::Encode` 추가.
- **MP3 인코더 선택**: `mp3lame-encoder` 0.3.0(LAME 소스를 `mp3lame-sys` 가 `cc` 로 빌드·**정적 링크** — 이미지에 별도 바이너리/공유 라이브러리 불필요, `ldd` 결과 libc·libm·libgcc 만; 빌더 이미지 `rust:1-bookworm` 의 gcc 로 빌드됨). 대안 `shine-rs`(순수 Rust)도 검토했으나 LAME 의 품질·성숙도를 택함. **두 후보 모두 LGPL**(LAME LGPL-3.0, shine LGPL-2.0) — 정적 링크한 이미지를 외부에 배포하는 경우의 라이선스 의무는 오케스트레이터/사용자 판단 필요. **크레이트 버그**: `Encoder::encode_to_vec` 가 SIGSEGV 를 일으킴 → 명시적 `MaybeUninit` 버퍼(`encode` + `max_required_buffer_size`, 1초 블록 단위, flush 8KiB)로 구현하고 `append_written` 에 최소한의 `unsafe`(0 으로 초기화한 버퍼의 앞 `written` 바이트 읽기, SAFETY 주석)를 둠.
- **테스트 재구성(mock 전부 OpenAI 호환 형식)**: llm_nodes(26)·rag(12)·voice(13)·slide(14)·fill(15) 의 mock 을 `/chat/completions`·`/embeddings`·`/audio/speech`(하나의 base URL, 엔드포인트별 요청 기록)로 교체, 단일 키로 단순화. 키 없음 시나리오는 '임베딩/LLM/TTS 별 키' → '키 없음이면 첫 단계(LLM 또는 임베딩)에서 실패, 아무것도 호출되지 않음' 으로 재정의. 새 케이스: 200 본문 내 error, content null+length, refusal, 빈 content, 529 비재시도, base URL 끝 `/`, TTS 400(voice 불일치) 메시지, rate 헤더(16k) 반영, 청크 간 형식 불일치, 비오디오/빈/홀수 바이트 응답.
- **오디오 검증**: TTS mock 이 청크 k 마다 `300 + 150k Hz` 톤의 PCM 을 돌려주게 하고, 최종 MP3 를 `symphonia`(순수 Rust, MP3 'Excellent')로 디코드해 ① 샘플레이트 ② 길이 ≈ 청크 수 × 0.5s(±3 프레임) ③ 각 구간의 우세 주파수(Goertzel)가 청크 순서대로 올라가는지 확인 → 청크가 순서대로 결합된 한 개의 유효 MP3 임을 청각적으로 검증. 라이브러리 단위 테스트는 440Hz 톤이 ±2% 로 복원되는지 확인. (처음엔 `puremp3` 디코더를 썼으나 정확도가 낮아(톤이 4~8% 어긋나고 600→750Hz 로 오판) 인코더 문제로 오해할 뻔했음 — `symphonia` 로 교체해 해소, `puremp3` dev-dependency 제거.)
- **compose/.env.example**: `docker-compose.yml` 은 `engine.environment` 블록만 수정(구 키 6개 제거, `OPENROUTER_API_KEY`·`OPENROUTER_BASE_URL`·`LLM_MODEL`·`LLM_EFFORT`·`EMBEDDING_MODEL`·`TTS_MODEL`·`TTS_VOICE` 를 `${VAR:-}` 패스스루; 다른 서비스·extractor GPU 예약은 미변경). `.env.example` 은 같은 7키(값 비움 + 기본값 주석). **사용자 `.env` 는 열람·수정·출력하지 않음.** `docker compose config | grep -c` = 7.
- **실행 명령어 / 결과**: `set -euo pipefail` 스크립트로 단계별 엄격 검증 — fmt/clippy 0, cargo test 347 통과, `prepare --check`, docker build, `src` 에 `api.anthropic.com|anthropic-version|x-api-key` 0건, compose 키 7개 → VERIFY_ALL_OK. bindings 무변경. 실제 OpenRouter 호출 없음(오케스트레이터가 실 API 로 확인).
- **의존성**: 추가 `mp3lame-encoder` 0.3.0, dev `symphonia` 0.6.1(mp3, default-features 끔); 제거(dev) `puremp3`. (T-307 의 `lopdf` 일반 의존성 이동은 그대로.)
- **디스크 사고(참고)**: `cargo test --release` 를 실수로 실행해 디스크가 100% 가 됨(다른 빌드 포함 호스트 전체). 내 `engine/target`(24GB, 오래된 테스트 바이너리 누적)을 `cargo clean` 으로 정리해 복구. 이후 `CARGO_INCREMENTAL=0`·`CARGO_PROFILE_DEV_DEBUG=line-tables-only` 로 빌드해 `target` 크기를 줄임(Cargo.toml 은 건드리지 않음). `engine/target` 하나만 사용.
- **실 API 확인이 필요한 가정(오케스트레이터 사전 확인 외)**: `reasoning.effort` 값 허용 범위(low|medium|high), 이미지 `image_url` data URL(jpeg) 수용, 모델이 `max_tokens: 16000` 을 수용하는지, TTS 가 긴 입력(4000자)을 한 요청으로 받는지, `X-Title` 헤더 무해성.

### [2026-11-26 21:25] [T-310] [w2-fine] — DONE
- **작업 내용**: 노드별 실 OpenRouter 호출 검증 도구 — `scripts/live-nodes.sh`(+`scripts/live/lib.sh`·`nodes.sh`), `fixtures/live/{make_fixtures.py, report_en.pdf(39KB, 4쪽·표2·그림2), report_ko.pdf(90KB, 3쪽·표1·그림1, NanumGothic), questions.json}`, `.gitignore` 에 `live-results/`
- **사용법**: `BASE_URL=<url> LIVE_CONFIRM=1 ./scripts/live-nodes.sh [--only a,b] [--out DIR] [--doc en|ko] [--with-verify-stack]`. BASE_URL 없고 --with-verify-stack 도 아니면 rc=2. LIVE_CONFIRM!=1 이면 대상·노드별 예상 유료 호출 수만 출력하고 rc=0(워크플로·번들 생성 0). `--with-verify-stack`: df 10GB 확인 → verify-stack 과 같은 lock/프로젝트(dochestra-verify)/포트 18080 으로 up --build --wait → 실행 → down -v(trap)
- **노드별 그래프(공통 bundle→pick_pdf→extract)**: translate(json_input{targetLanguage}), summary(md+images), analyze(images→pick_jpg 0), slide(md+pick_jpg), rag(md → api JSON 의 indexId 로 POST /api/rag/{id}/query 2건, questions.json 의 기대 키워드 포함 여부 표시), voice(md+images), fill(engine/tests/fixtures/form.pdf 번들 → pick_pdf + 폼 필드명과 다른 표기의 값 JSON). 노드마다 번들·워크플로 개별 생성, 종료 시 API 로 삭제(번들 id 는 subshell 이 아닌 현재 셸 전역에 등록해 trap 정리 보장). 출력 노드 산출물만 다운로드(`<node>/artifacts/<nodeId>-<socket>-<idx>-<파일명>`), 각 노드 `run.json`·`events.jsonl`·`result.json`·`node-status.txt`, 집계 `SUMMARY.md`(노드|상태|소요 시간|노드 상태(앞단 ✓/✗)|산출물|오류|미리보기 300자). 실패 노드가 있어도 끝까지 진행, 하나라도 실패하면 exit 1(skipped 는 실패 아님)
- **검증**:
  - 게이트: BASE_URL 미설정 rc=2, 알 수 없는 `--only` rc=2, LIVE_CONFIRM 없이 → PLAN ONLY, 생성 리소스 0
  - 1차 `LIVE_CONFIRM=1 --with-verify-stack`: jq 구문 오류(`x: 250 * (...)` 에 괄호 필요)로 7개 노드 모두 "워크플로 생성/저장 실패"(워크플로 생성 전에 실패 → 외부 호출 0, SUMMARY 에 사유 기록 확인) → 수정
  - 2차: **7개 노드 전부 succeeded**(translate 10.2s, summary 6.8s, analyze 4.6s, slide 6.7s, rag 2.5s, voice 39.4s, fill 1.4s), exit 0. rag 2개 질의 모두 기대 키워드 ✓(482 / Asia-Pacific 31). 산출물: translate MD 2965B(한국어 번역), summary MD 3670B, analyze MD 867B, slide PDF 1.5MB, voice MP3 1.9MB, fill PDF 11.7KB
  - **주의: 이 실행은 실제 OpenRouter 유료 호출이었음** — 티켓은 "T-309 전이라 LLM 노드가 키 미설정으로 실패하는 것이 정상" 으로 예상했으나, 현재 작업 트리의 engine 이 이미 OpenRouter 로 전환되어 있고 `.env`(열람하지 않음)에 키가 있어 compose 가 그대로 주입함. 호출량은 노드당 위 추정치 수준(총 소액)
  - 결과·로그에 키 없음(`sk-or-`·`Bearer ` 검색 0건, 키 변수는 스크립트가 접근하지 않음). 종료 후 dochestra-verify 컨테이너·볼륨 0, 사용자 스택 컨테이너 ID 불변, `live-results/` git-ignore 확인(`git check-ignore -q live-results`), 첫 실패 실행 폴더는 삭제
- **미검증/한계**: shellcheck 미설치(`bash -n` 만). 실패 경로(앞단 성공 + LLM 노드 실패 사유 기록)는 키 미설정 상태에서 재현하지 못함 — 대신 "실행 거부(422)/네트워크/타임아웃/노드 failed" 분기가 SUMMARY 에 `오류` 로 들어가도록 구현됨(1차 실행에서 워크플로 단계 실패 문구 확인). `--doc ko` 경로는 실행하지 않음. 산출물 품질 판정은 사람이 SUMMARY·artifacts 를 열어 검토(예: translate 미리보기에서 숫자·고유명 보존 양호)
- **발견 사항**: fill 결과 PDF 에 값이 실제로 채워졌는지는 이번 스크립트가 자동 검증하지 않음(시각 확인 필요) — 후속으로 pypdf 로 필드값 추출 검증 추가 가능

### [2026-11-26 18:30] [T-118] [w1-kobe] — DONE
- **작업 내용**: extractor 이미지 축소 — uv 다운로드·추출 캐시(약 6GB)가 이미지 레이어에 남던 문제를 제거. `extractor/Dockerfile` 의 두 `uv sync` 단계에 `RUN --mount=type=tmpfs,target=/root/.cache/uv,size=9g` 를 적용(`UV_LINK_MODE=copy` 유지 — 캐시(tmpfs)와 venv(overlay)가 다른 파일시스템이라 복사 모드 필요). 모델 다운로드·비루트 uid 10001·`HF_HUB_OFFLINE`·CUDA 설정은 그대로. 다른 파일 변경 없음(`.dockerignore` 도 불필요).
- **`type=cache` 대신 tmpfs 를 쓴 이유**: 캐시 마운트는 6GB 를 호스트 디스크의 빌드 캐시에 남겨 여유 15GB 에서는 빌드가 불가능(T-117 의 분석과 같은 결론). tmpfs 는 RAM 에서만 쓰이고 빌드 후 사라져 디스크에는 venv 레이어(6.7GB)만 남음. 호스트 RAM 15GB(가용 약 10GB) 중 약 6GB 를 빌드 동안 사용 — 단일 단계라 사용자 스택에 영향 없었음(빌드 후 컨테이너 메모리 정상).
- **결과(`dochestra-extractor:slim`, 이미지 ID ae97997d0501)**: `docker history` 레이어 합계(풀린 크기) ≈ **7.7GB**(venv 6.71GB + 모델 771MB + 베이스·uv ≈ 0.3GB) — 기존 latest 는 약 14.0GB(venv 레이어 13.1GB 포함)였으므로 **−45%**. 압축 콘텐츠 4.18GB(기존 7.56GB). `docker image ls` 의 'DISK USAGE' 컬럼은 containerd 스토어에서 `풀린 크기 + 압축 콘텐츠` 를 합친 값이라 11.8GB(기존 21.6GB, 같은 −45%) — 이 컬럼 기준으로는 ≤10GB 가 불가능하지만(압축 콘텐츠가 스토어에 별도로 남음), 이미지 자체(레이어 합계)는 목표 ~8GB 를 충족.
- **검증**: ① `docker run --rm --gpus all --entrypoint python dochestra-extractor:slim -c 'import torch;assert torch.cuda.is_available()'` → `cuda ok Tesla T4 2.15.0+cu134`. ② `docker run -d --network none --gpus all --memory 4g -v /tmp/t118data:/data dochestra-extractor:slim` → 네트워크 없이 기동, `/health` = `{"status":"ok","device":"cuda"}`, 로그 `extractor device: cuda` / `docling converter ready (device=cuda)`. ③ 같은 컨테이너에서 `POST /extract`(sample.pdf) 성공: md·json·tables 1·images 1, md 에 'Annual Sales Report' 포함, 변환 후 컨테이너 메모리 1.47GiB/4GiB. 테스트 컨테이너·데이터는 정리함(출력 파일은 uid 10001 소유라 컨테이너로 삭제). `verify-stack` 은 지시대로 실행하지 않음.
- **디스크 기록 / 주의**: 시작 시 `df` 여유 15GB → 내 `engine/target`(9.7GB, 빌드 산출물)을 `cargo clean` 으로 비워 24GB 확보 후 빌드. 빌드 자체(레이어 생성)는 문제없었으나 **마지막 '이미지 unpack' 단계에서 여유가 2GB 까지 내려가** 가드 스크립트(3GB 미만 시 중단)가 빌드를 중단시켰다 — 이미지는 이미 만들어져 있었고(`slim` 태그 존재·위 검증 통과) 중단은 마무리 직전이었음. 현재 여유 6.1GB. 원인은 containerd 스토어가 이미지를 (빌드 캐시 스냅샷 + unpack 스냅샷 + 압축 콘텐츠)로 중복 보관하기 때문 — 빌드 캐시가 33GB(대부분 회수 불가로 표시: `docker system df` Build Cache 33.4GB, reclaimable 51MB)로 커서 **빌드 캐시 정리(`docker builder prune`)가 가장 큰 회수 수단**이지만 금지 사항이라 하지 않고 보고함.
- **후속 제안**: (1) 오케스트레이터가 `latest`·`dochestra-verify-extractor` 태그를 slim 으로 옮기면(`docker tag dochestra-extractor:slim dochestra-extractor:latest`) 기존 21.6GB 이미지 레이어가 해제되어 약 10GB 이상 확보됨(디스크 압박 시 우선 조치). 단 사용자 스택 컨테이너가 쓰는 기존 이미지의 교체는 compose 재배포 시점에 일어남. (2) `/home/ubuntu/.cache/uv` 가 9.3GB(호스트 venv 용 uv 캐시, 소유는 다른 트랙)로 디스크의 주요 소비처 — 필요 시 `uv cache clean` 로 회수 가능(내가 건드리지 않음). (3) 빌드 가드 스크립트는 스크래치패드(`guarded-build.sh`)에 있음.

### [2026-11-26 23:10] [T-311] [w2-fine] — DONE
- **작업 내용**: translate 실사용 품질 점검·보강. 라이브 시나리오(`--only translate` = 3케이스): ① translate-ko(report_ko, ko→en) ② translate-long(handbook_en.pdf 20쪽·표 8·그림 3, en→ko) ③ translate-settings(report_en + glossary{Aurora Robotics, operating margin→영업이익률, fleet→플릿}·tone·instructions). ③·④(glossary / tone·instructions)는 한 시나리오로 합쳐 총 유료 실행 6회(3케이스 × 2회) 예산을 지킴. 점검표는 `scripts/live/translate_check.py`(원문=extract md 와 번역 대조: 블록·표 행열·코드 블록·placeholder·제목·숫자·glossary·군더더기·코드 래핑·인접 중복)가 `checks.json`/result.json/SUMMARY.md 에 기록
- **1차(수정 전, 베이스라인) 결과**: 3케이스 succeeded(8.7s/64.1s·8청크/8.4s), 표 구조·placeholder·제목 수·glossary·군더더기 모두 이상 없음. 블록 수는 116→86 로 달라 보였으나 원인은 번역 누락이 아니라 PDF 추출이 쪽 경계에서 한 문장을 별개 블록(소문자 시작)으로 쪼갠 조각 28개를 LLM 이 한 문단으로 합친 것 — 숫자 누락 0 으로 확인(영→한 숫자 표기 변환으로 숫자 토큰은 오히려 +200). 8청크 전체 분석에서 **발견한 실제 위험 2건**: (a) 청크 경계 7곳이 전부 제목으로 끝남(제목이 본문과 분리되어 다음 청크에 본문만 전달) (b) 쪽 경계 문장 조각 28개가 청크 경계에 걸리면 문장이 두 번역 요청으로 쪼개짐(이번 문서에선 경계에 걸리지 않았으나 구조적 위험)
- **수정(engine/src/nodes/translate.rs)**: ① `attach_headings`: 연속 제목을 다음 비제목 블록과 하나의 원자 블록으로 결합(청크가 제목으로 끝나지 않음) ② `join_continuations`: 앞 블록이 산문(알파벳으로 시작·종결 부호 없음)이고 다음 블록이 소문자로 시작하면 한 문단으로 재결합 ③ `sanitize_answer`: 답변 전체가 ```/```markdown/```md/~~~ 로 감싸졌고 원문 청크가 코드 펜스로 시작하지 않으면 래퍼 제거, 원문과 답변의 펜스 코드 블록 수가 같으면 각 코드 블록을 원문 그대로 복원(코드 번역·변형 방지). rag 가 공유하는 `split_chunks` 는 그대로 두고 번역 전용 `split_translation_chunks` 신설(처음 `split_chunks` 를 바꿨다가 rag 청킹 테스트 3개가 깨져 분리 — 다른 노드 수정 금지 준수)
- **회귀 테스트**: 단위 5개(plain_chunking_is_unchanged_for_rag, headings_stay_with_the_following_block, page_break_fragments_are_rejoined, answer_wrapped_in_a_code_fence_is_unwrapped, code_blocks_are_restored_verbatim) + 통합 3개(`translate_keeps_headings_with_their_body_across_chunks`, `translate_rejoins_page_break_fragments_before_sending`, `translate_restores_code_and_unwraps_a_fenced_answer`). 기존 테스트 2곳은 소문자 반복 문자열이 조각으로 재결합되어 대문자로 변경
- **2차(수정 후) 결과**: 3케이스 succeeded(8.1s/56.4s/8.3s), 점검표 전 항목 ✓(translate-ko 의 숫자 항목은 한글 원문의 만·억 단위 변환(4억 8,200만→$482 million)이라 체커를 한글 원문에서는 '확인 필요' 로 허용하도록 수정 후 ✓). translate-long: 번역 28.2K자(원문 57.2K), 표 8개 모두 (6행,4열) 보존, placeholder 3/3, 제목 35/35, 숫자 누락 0
- **엔진 검증**: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`(219 lib + 통합 전부 ok) + `cargo sqlx prepare --check -- --all-targets` 통과, docker build 는 live 실행(`--with-verify-stack`)이 현재 작업 트리로 이미지를 빌드해 성공. 종료 후 dochestra-verify 잔존 0
- **한계/미검증**: (1) Docling 이 합성 PDF 의 Courier 코드 목록을 코드 블록(펜스)으로 인식하지 않아 라이브에서는 코드 블록 보존을 검증하지 못함 — mock 테스트로만 검증 (2) 수정 전후 번역 품질 차이를 라이브에서 직접 입증하진 못함: 베이스라인에서 구조 손상은 없었고, 두 수정은 청크 경계 분석에서 찾은 구조적 위험에 대한 예방 조치 (3) 청크는 순차 호출(8청크 ≈ 56~64s) — 병렬화는 속도 외 이득이 없고 레이트리밋 위험이 있어 하지 않음 (4) 번역 품질(어휘·어조) 판정은 사람이 `live-results/*/translate-*/artifacts/` 를 열어 확인 필요(glossary 반영·합니다체는 스크립트로 일부만 확인)
- **환경 메모**: 작업 중 작업 트리에 내가 수정하지 않은 변경(engine/src/nodes/rag.rs, engine/tests/rag.rs, scripts/live/rag_check.sh)이 생김 — 다른 티켓 작업으로 판단하고 건드리지 않음. 사용자 스택(dochestra) 컨테이너 ID 가 중간에 바뀐 것은 오케스트레이터 재배포로 추정(나는 해당 프로젝트에 어떤 변경 명령도 실행하지 않음)

### [2026-11-26 21:55] [T-315] [w1-kobe] — DONE
- **작업 내용**: rag 노드·질의 API 의 실사용 품질 점검(실제 OpenRouter `baai/bge-m3` 임베딩 + 채팅 모델). 점검 스크립트 `scripts/live/rag_check.sh`(BASE_URL 필수·LIVE_CONFIRM=1 게이트·`--with-verify-stack`·`--label`)가 bundle→pick_pdf→extract→rag 를 en/ko/handbook 3문서로 돌린 뒤 18개 고정 질문(영어 사실 4, 교차언어 2, 한국어 3, 문서 밖 2, 핸드북 3, topK 1/5/20 경계 4)을 던지고 키워드 적중·`[n]` 인용 유효성/근거 지지·'모름' 처리·답변 언어·점수 내림차순·topK 개수·지연·청크 통계(1500자 초과/한계에서 잘림/200자 미만/문장 중간 시작/제목 고아/제목 맥락/잘린 표·코드)를 `live-results/<ts>/rag-check/{report.md,results.json,<doc>/chunks.json}` 에 기록. 유료 실행 총 2회(베이스라인 + 수정 후, 예산 3회 중 1회 남김).
- **베이스라인(20261126T123804Z) 결과**: 사실 12/12, 문서 밖 2/2 '모름', 경계 4/4 → **답변 품질은 이미 양호**. 문제는 청킹: handbook 75청크 중 **30개가 1500자 한계에서 잘림(단어 중간 포함)**, 15개가 200자 미만 꼬리 조각, 29개가 문장 중간에서 시작, 제목 맥락 있는 청크 28개뿐. 근거 점수 문턱(threshold) 필터는 문서 밖 질문의 점수(0.46~0.59)가 실제 적중(0.47~0.78)과 겹쳐 채택하지 않음.
- **수정(engine/src/nodes/rag.rs, `chunk_markdown` 재작성)**: ① 제목을 다음 블록에 결합(제목으로 끝나는 청크 없음) ② 블록을 BODY_MAX=1398자까지 그리디 패킹 ③ 한계 초과 단위는 `balanced_split`(총량/ceil(총량/한계)에 맞춘 균등 분할, 문장 경계 `.!?。！？`·개행 우선, 문장이 한계보다 길면 단어 경계, 그것도 없으면 강제 절단) ④ 모든 조각 앞에 소속 섹션 제목(최대 100자)을 붙여 제목 맥락 유지. 최대 청크 크기는 기존과 같은 1500자(`MAX_CHUNK_CHARS`), 임베딩·DB·API 응답 형식 불변. `api/rag.rs`·`db/rag.rs` 는 변경 불필요(프롬프트·검색 모두 정상 동작).
- **회귀 테스트**: `nodes/rag.rs` 단위 12개(제목 결합·균등 분할·문장 경계·단어/강제 절단·한글 구두점·표/코드 블록 보존·제목 맥락 등) + `tests/rag.rs::long_sections_are_stored_as_balanced_sentence_aligned_chunks_with_their_heading`(목 임베딩 + 실제 DB: 긴 섹션이 ≤1500자·문장 경계 시작/끝·조각 크기 편차 <250·모든 문장이 정확히 1회 저장·다음 섹션은 자기 제목으로 시작). 테스트 작성 중 기대값 오류 1건(첫 조각 앞의 상위 제목 `# Handbook` 결합은 정상 동작) 수정.
- **수정 후(20261126T124943Z, `--label fix1`) 결과**: 사실·교차언어 **11/12(92%)**, 문서 밖 **2/2 '모름'**, 경계 4/4, 인용 `[n]` 표기 일관(사실 질문 중 인용 포함 94%, 점수 내림차순·topK 개수 ✓), 평균 질의 지연 2.5s, 판정 통과. 청크: handbook 75청크 평균 782자 **1500자 초과 0 / 한계 잘림 30→0 / 200자 미만 15→4 / 문장 중간 시작 29→0 / 제목 맥락 28→75**, 잘린 표·코드 없음. en 2청크(평균 1324자), ko 1청크(850자) 이상 없음.
- **남은 1건(`hb-config`, 정직한 보고)**: "zone B 의 charge_threshold 는?" 이 수정 전에는 통과(점수 0.561, 정답 청크가 5개 중 1개), 수정 후에는 '문서에 관련 정보 없음' 으로 실패. 원인은 코드 결함이 아니라 **검색 노이즈**: handbook 은 20쪽이 거의 동일한 filler 문장의 반복이라 상위 5개 점수가 0.526~0.545 로 평탄하고, JSON 조각이 든 3개 청크가 상위 5개에 들지 못함(베이스라인도 사실상 운 좋은 통과). 표(`hb-zone`)·응답 시간(`hb-response`)은 통과. 해결하려면 하이브리드(어휘) 검색 등 검색 설계 변경이 필요해 이 티켓 범위(응답 계약 불변)를 넘어 제안만 함.
- **검증**: `/tmp/.../verify-engine.sh`(set -euo pipefail): fmt·clippy `-D warnings`·`cargo test` 362 passed/0 failed·`cargo sqlx prepare --check -- --all-targets`(쿼리 매크로 불변 → `.sqlx` 변경 없음)·docker build 모두 통과. 라이브 점검은 `--with-verify-stack` 로 격리 스택(18080)에서만 실행, 종료 후 verify 컨테이너 0, 사용자 스택 미접촉, 결과·로그에 키 없음. 디스크 여유 23GB(실행 후 동일).
- **미검증/한계**: 한국어 PDF 는 3쪽 1청크 문서라 한국어 청킹의 장문 분할은 단위 테스트로만 검증. 근거 청크 중복(거의 동일한 청크가 상위 점수를 점유, 예: `hb-response` 인용 1~5 전부)에 대한 다양성 재정렬·청크 오버랩은 근거가 약해 구현하지 않음.

### [2026-11-26 22:15] [T-316] [w1-kobe] — DONE
- **작업 내용**: voice 노드(해설 대본 LLM + TTS pcm → MP3) 실사용 품질 점검·보강. 점검 스크립트 `scripts/live/voice_check.sh`(BASE_URL 필수·LIVE_CONFIRM=1 게이트·`--with-verify-stack`·`--label`·`--docs`)가 en/ko/handbook 3문서를 bundle→pick_pdf→extract→voice 로 실행하고, 격리 스택 engine 컨테이너에서 `/data/runs/<run>/n_v/_work/script.txt` 를 `docker compose exec -T engine cat` 으로 읽어(격리 프로젝트 한정) 대본·MP3 를 점검: 대본(길이 1500~2500자·한글 비율·마크다운/불릿/이모지/URL 잔존·핵심 사실 적중·이미지 언급·최장 문장·영문 단어 수), TTS 청크(4000자 분할 재현·문장 끝 절단 여부·경계 위치), MP3(디코딩 재생 길이·샘플레이트·채널·비트레이트·크기·글자/초·앞/뒤/내부 무음(20ms RMS)·끝 잘림·클리핑·청크 경계 근처 무음·청크별 음량), 총 소요. MP3 디코딩은 일회성 `uv run --no-project --isolated --with miniaudio --with numpy python -I`(호스트 설치 없음). 유료 voice 실행 총 6회(3문서 × 전/후) 사용, 추가 재실행 없음.
- **베이스라인(20261126T125649Z) 결과**: 3문서 모두 succeeded(32~39s). 대본 1540~1989자(목표 범위 내)·한글 비율 0.98~1.0·마크다운/이모지 0·이미지 언급 2~5회, 청크 1개(대본이 4000자 미만). 사실 정확도는 en 대본의 모든 수치를 원문 PDF 와 대조해 일치 확인(4억 8천2백만·18%·14%·23%·16%·1,120→1,340명·지역별 매출 등). MP3 24kHz mono 64kbps, 재생 211~266s(글자/초 7.0~7.5로 문서 간 일정 → 잘림/누락 없음), 음량 -17dBFS, 앞/뒤 무음 0.06~0.38s, 내부 최장 무음 1.18~1.84s(문단·문장 경계), 끝 잘림·클리핑 없음. 처음 채점에서 en 이 '핵심 사실 7/9' 로 실패했으나 원인은 점검기였음("4억 8천2백만"·"5억4천만" 한국어 수 표기 미인식) → 공백 무시·한국어 수 표기 허용으로 수정해 재채점하면 9/9(유료 호출 없이 오프라인 재채점).
- **발견한 실제 문제(1건)**: en 대본에서 같은 대상이 한 문단 안에서 "오로라 로보틱스"/"Aurora Robotics"·"Min-jun Park" 로 혼용 표기(영문 단어 5개) — 한국어 TTS 가 로마자 이름을 일관되게 읽는다는 보장이 없음. 추가로 핸드북 대본에 LLM 오타("충전 베베이") 1건(모델 출력 오류라 코드로 해결 불가).
- **수정(engine/src/nodes/voice.rs)**: ① 시스템 프롬프트에 규칙 추가 — 영문 상호·이름·약어는 한국어로 읽는 대로 한글로 적고(예: Aurora Robotics → 오로라 로보틱스) 같은 대상은 한 가지 표기만 사용. ② `clean_script`(안전망): TTS 가 그대로 읽어 버리는 마크다운 제목 `#`·불릿/번호·`**`/`__`/`~~`/백틱·구분선·이모지·URL 을 TTS 입력과 `script.txt` 에서 제거하고 문단 구분은 유지(숫자·`%`·`-3도`·`#1` 은 건드리지 않음). 베이스라인 대본에는 잔존 기호가 없었으므로 이 안전망은 **라이브 증거가 아니라 프롬프트 규칙 하나에만 의존하는 위험에 대한 예방**임(정직하게 구분). `clients/tts.rs` 는 수정하지 않음(아래 근거).
- **tts.rs 를 바꾸지 않은 근거**: 청크 경계 처리는 문장 단위 분할(기존 단위 테스트)이고, 단일 청크 MP3 의 앞/뒤 무음이 이미 0.06~0.38s 라 이어 붙일 때 경계에 자연스러운 쉼(약 0.2~0.7s)이 생김 → 무음/페이드 삽입의 근거가 없음. 단 **다중 청크 라이브 검증은 못 함**: 실제 LLM 대본이 항상 4000자 미만(1540~2390자)이라 라이브에서는 경계가 한 번도 발생하지 않음. 경계 동작은 mock 테스트(`long_text_is_synthesized_in_order_and_joined_into_one_mp3`, `a_long_script_is_synthesized_in_several_requests_and_joined`)로만 검증.
- **회귀 테스트**: 단위 5개(`nodes/voice.rs`: 평문 불변·마크다운 제거·이모지/URL 제거·숫자/기호 오인 방지·빈 줄 접기) + 통합 2개(`tests/voice.rs::markup_left_in_the_script_is_removed_before_synthesis`: 마크다운 섞인 LLM 응답 → TTS 입력과 script.txt 가 정제본, `the_prompt_asks_for_hangul_names_with_one_spelling`).
- **수정 후(20261126T130328Z, `--label fix1`) 결과**: 3문서 모두 succeeded(36.5~40.0s), 점검표 전 항목 ✓. 대본 1986/2390/1927자, 영문 단어 0개(전 5→0), 사실 9/9·9/9·5/5, 재생 276.3/327.5/254.0s(글자/초 7.2/7.3/7.6 — 대본에 비례), 내부 최장 무음 1.74/1.12/1.42s, 끝 잘림 없음, 클리핑 0~0.0002%.
- **엔진 검증**: `verify-engine.sh`(set -euo pipefail) — fmt·clippy `-D warnings`·`cargo test` 370 passed/0 failed·`cargo sqlx prepare --check -- --all-targets`(쿼리 변경 없음)·docker build 통과. 라이브는 격리 스택(18080)에서만, 종료 후 verify 컨테이너 0, 사용자 스택 미접촉, 결과·로그에 키 없음. 디스크 여유 23GB 유지.
- **미검증/한계**: ① 실제 청취 평가(발음·억양)는 하지 못함 — 이 환경에서 오디오를 들을 수 없어 길이·무음·음량 등 신호 수준 점검만 수행(MP3 는 `live-results/*/voice-check/<doc>/artifacts/` 에 있음, 사람이 들어 확인 필요). ② 다중 청크 라이브 검증 불가(위). ③ 프롬프트 수정 부작용: en 대본에서 "Min-jun Park" 이 "민준"(성 생략)으로 나옴 — 한글 표기 규칙의 LLM 해석 편차. ④ 문장 길이: ko 대본에 299자짜리 문장 1개(최장), 100자 초과 문장이 문서당 1~4개 — 낭독 호흡상 다소 긴 문장이나 규칙 위반은 아님(프롬프트에 문장 길이 제한은 없음, 근거 약해 추가하지 않음). ⑤ 문단·문장 경계 무음이 0.8s 이상인 곳이 문서당 6~16회(최장 1.84s) — TTS 모델의 기본 쉼이며 후처리로 줄이지 않음.

### [2026-11-26 22:15] [T-312] [w2-fine] — DONE (유료 호출 상한 8회 소진, 마지막 프롬프트 조정은 라이브 미확인 — 아래 참고)
- **작업 내용**: summary 실사용 품질 점검·보강. 라이브 4시나리오(`--only summary`): summary-long(handbook_en 20쪽·그림 3) / summary-ko(report_ko·그림 1) / summary-noimg(report_en, images 미연결 = D-4 선택 입력) / summary-many(`gallery_en.pdf` 신규: 28 figure → 추출 결과 이미지 47장, 20개 제한·제외 주석 확인). 점검은 `scripts/live/summary_check.py`(핵심 사실 포함 `fixtures/live/facts.json`, 원문에 없는 숫자, 구조, 섹션 커버리지, 한국어 비율, 길이, 군더더기, 제외 주석 정확성, 그림 속 UNIT 번호 반영)
- **1차(베이스라인) 결과**: 4/4 succeeded(5.5/13.1/8.3/5.6s), 핵심 사실 100%, 제외 주석 정확(47장 → "27개 제외"), noimg 정상. 발견 문제 3건: ① 한국어 지시에도 영어 일반 어휘·단위가 그대로 남음(summary-long 한글 비율 32%: "24 hours", "12 minutes", "warning field", "guaranteed first response time은 30 minutes") ② 짧은 한글 문서 요약이 원문보다 김(129%): 핵심 요점·섹션 요약 중복 + 표 통째 복사 ③ 이미지가 문서의 어느 위치 그림인지 모델이 알 수 없음(프롬프트에 대응 설명 없음) — 그림 속 UNIT 번호는 20장 중 2종만 언급. (체커 보정: 이미지가 있는 경우 원문에 없는 숫자는 차트 판독값이라 '확인 필요'로 표시, 실제 값은 그림과 일치: 1장 차트 A43 B55 C32 D72 등)
- **수정(engine/src/nodes/summary.rs 프롬프트)**: 시스템 프롬프트에 "요약문은 반드시 한국어, 일반 어휘·단위(hours, minutes, meters, percent)는 한국어로, 고유명사·제품명·코드·식별자만 원문 유지"; 지시문에 (a) 원문보다 분명히 짧게(대체로 20~40%, 1,000자 이하 문서는 더 짧게), 표 통째 복사 금지 (b) 문서에 없는 수치 금지 (c) 이미지는 `<!-- image -->` 위치 순서대로 첨부, 그림 속 글자·수치는 반영하되 본문과 혼동 금지. translate.rs 1줄: 시스템 프롬프트에 "Keep numbers as Arabic numerals; do not spell them out in words."(오케스트레이터 허용 범위). 회귀 테스트: `summary_prompt_demands_korean_brevity_and_no_invented_numbers` 신설, `summary_sends_images_before_the_document_in_order` 에 이미지 대응 문구 단언 추가, translate 시스템 프롬프트 단위 테스트에 Arabic numerals 단언
- **2차(수정 후) 결과**: 4/4 succeeded(4.9/11.6/9.7/6.9s). 한국어 출력 해결(91%/100%/90%/66%), 핵심 사실 100%, 제외 주석 정확, 사실 오류 없음(확인 가능한 항목 기준: 숫자는 원문 또는 그림 판독값). 길이: long 2%, many 17%, noimg 44%, ko 93%(핵심 요점+섹션 중복이 남아 ✗). **회귀 1건 발견**: summary-long 이 "같은 구조가 반복되면 장 단위로 묶어 한 번만 설명" 지시 때문에 8개 장을 '### 1. 함대 아키텍처, 2. 설치, … 8.' 한 소제목 아래 한 덩어리로 합쳐 섹션 커버리지 6%(2/35)로 떨어짐(베이스라인은 8섹션). 이는 내가 넣은 문구의 부작용
- **마지막 조정(라이브 미확인)**: 회귀 문구를 "각 섹션(장)마다 소제목을 따로 둔다 + 반복 내용은 한 번만, 나머지 섹션에는 달라지는 부분만" 으로 교체하고, "섹션별 요약에는 핵심 요점에 이미 쓴 문장을 되풀이하지 말고 핵심 요점에 없는 세부 내용만" 문구 추가(ko 중복 대응). 테스트 단언 추가. 유료 상한(8회)을 2차 실행으로 모두 사용해 **이 조정은 라이브로 확인하지 못함** → 확인용으로 summary-long·summary-ko 2시나리오 추가 실행(유료 2회) 승인 요청
- **엔진 검증**: `cargo fmt --check`, `clippy -D warnings`, `cargo test` 전부 통과(lib 225 + 통합), `cargo sqlx prepare --check` 통과. live 스택은 현재 작업 트리로 빌드돼 docker build 성공. 종료 시 내 dochestra-verify 정리됨(이후 13:02 에 보인 dochestra-verify 컨테이너는 다른 에이전트의 검증 스택으로 판단, 건드리지 않음)
- **한계**: 사실 정확도는 facts.json 키워드·숫자 대조로만 자동 점검(문장 단위 사실 검증은 사람이 필요), gallery 추출 이미지가 28이 아닌 47장인 것은 Docling 이 도형을 추가 인식한 결과(summary 와 무관), 어조/가독성은 사람 검토 필요

### [2026-11-26 22:50] [T-312] [w2-fine] — 추가 확인 실행 (오케스트레이터 승인, 유료 2회: summary-long·summary-ko)
- 실행: `LIVE_CASES=long,ko LIVE_CONFIRM=1 ./scripts/live-nodes.sh --with-verify-stack --only summary` (live_case_enabled 필터를 nodes.sh 에 추가: LIVE_CASES 미설정이면 전체). 누적 summary 유료 실행 10회(8 + 확인 2)
- **summary-long**: succeeded 14.0s. 섹션 8개 소제목 복원(커버리지 8/35, 베이스라인과 동일; 이전 회귀 실행은 2/35), 핵심 사실 5/5, 한글 비율 54%(코드·식별자·`zone_n_m` 가 많아 비율이 낮음, 일반 어휘·단위는 모두 한국어: 24시간/30분/0.3미터/초/10초/20퍼센트), 길이 4%, 서문 없음, 반복 내용은 핵심 요점에서 한 번만, 장별 소제목에는 달라지는 세부(표·코드·그림 판독값)만. 그림 판독 수치(A43 B55 C32 D72 등)는 실제 그림과 일치
- **summary-ko**: succeeded 6.2s. 길이 93% → 80%(678자/850자), 핵심 요점과 섹션 요약의 문장 중복 제거(섹션은 핵심 요점에 없는 세부만: 2024년 매출·증감률·공급사 인증·로테르담·재생에너지), 핵심 사실 10/10, 한글 94%. 어투가 실행마다 달라짐('~이다' ↔ '~입니다')이나 단일 문서 안에서는 일관
- 엔진 변경 없음(이번 실행은 위 수정 후 코드 그대로). 종료 후 dochestra-verify 잔존 0

### [2026-11-26 22:50] [T-314] [w1-kobe] — DONE
- **작업 내용**: slide 노드(LLM 슬라이드 구성 JSON → 16:9 PDF) 실사용 품질 점검·보강. 점검 스크립트 `scripts/live/slide_check.sh`(BASE_URL 필수·LIVE_CONFIRM=1 게이트·`--with-verify-stack`·`--label`·`--docs`·`--tiny-index`)가 4시나리오를 bundle→pick_pdf→extract→pick_jpg→slide 로 실행: ① en(report_en, 대표 이미지 0번) ② ko(report_ko) ③ handbook(handbook_en 20쪽) ④ tiny(gallery_en 의 추출 이미지 3번 = 144×26px, 비율 5.5 — 세로로 긴 픽스처 이미지는 없어서 '아주 작은/극단 비율' 로 대체; 867×23 스트립은 `--tiny-index 10`). 격리 스택 engine 컨테이너에서 `/data/runs/<run>/n_s/_work/slides.json` 을 `docker compose exec -T engine cat` 으로 읽고(격리 프로젝트 한정), 엔진 로그의 "slide outline rejected" 증가분으로 구성 재시도 횟수를 센다. PDF 는 일회성 `uv run --no-project --isolated --with pypdfium2 --with pillow --with numpy` 로 쪽별 PNG(`<doc>/png/page-NN.png`)와 한눈에 보는 `sheet.png` 로 렌더(호스트 설치 없음; extractor 의 6.7GB uv 환경은 쓰지 않음). 자동 점검: 구성 JSON 확보·재시도, 페이지 수(1+N+1)·'n / N' 쪽 번호, 언어(원문 따름·슬라이드별 한글 비율), 텍스트 줄별 위치(여백 밖·푸터 영역 침범), 잘림(…), 12pt 최소 글자 크기 도달, 구성 JSON 의 글자가 PDF 에 모두 있는지(글리프 누락)·□/U+FFFD, 이미지 비율(±1%)·페이지 안·표지 글자와 겹침·확대 배율·2회 배치, 핵심 사실, 원문(md)에 없는 숫자(검토용), PDF 크기. 시각 검토(레이아웃·가독성)는 PNG 를 직접 열어 확인. 유료 slide 실행 총 8회(4시나리오 × 전/후) 사용.
- **베이스라인(20261126T131330Z) 결과**: 4시나리오 모두 succeeded(5.6~11.3s)·구성 1차 파싱 성공(재시도 0)·내용 슬라이드 5/4/14/11개(1+N+1 쪽)·언어가 원문을 따름(en·handbook·tiny 영어, ko 한국어)·핵심 사실 9/9·9/9·5/5·3/3·텍스트 넘침/푸터 침범/잘림/최소 글자(12pt) 0(본문 글자 16~24pt)·글리프 누락/깨짐 0·쪽 번호 정상·이미지 비율 오차 0%. 원문에 없는 숫자로 표시된 handbook 의 32/43/55/72 는 대표 이미지(막대 차트)에 인쇄된 값을 읽은 것(렌더 PNG 와 대조해 확인) → 오류 아님. 시각 점검: 표지(제목+우측 대표 이미지)·내용 슬라이드·마지막 이미지 슬라이드 레이아웃 양호, 글자 크기는 슬라이드별로 불릿 양에 맞춰 달라지나 가독성 문제 없음.
- **발견한 실제 문제(1건)**: tiny 시나리오에서 144×26px 이미지가 마지막 슬라이드(상자 672×309pt)에 **4.67배 확대**되어 픽셀이 깨진 얼룩으로 보임(`tiny/png/page-13.png`). 일반 이미지(645px 차트)는 0.31~1.0배라 해당 없음.
- **수정(engine/src/nodes/slide/render.rs)**: `MAX_IMAGE_UPSCALE = 2.0` — 이미지는 상자에 맞추되 확대는 최대 2배(1px=1pt 기준), 상자 안 가운데 정렬·비율 유지. 정상 크기 이미지 배치는 불변. 출력 소켓 `pdf`·구성 JSON·프롬프트 불변.
- **회귀 테스트(tests/slide.rs)**: `image_sizes`(PDF 콘텐츠 스트림의 `cm` 행렬에서 그려진 크기를 읽는 헬퍼) + `a_tiny_picture_is_not_enlarged_beyond_twice_its_size`(144×26 → 표지 202pt, 마지막 288pt, 비율 유지) + `ordinary_pictures_keep_filling_their_boxes_with_the_ratio_intact`(640×360 불변, **세로로 긴 300×900 과 867×23 스트립**은 비율 유지·상자 안 — 라이브 픽스처에 없는 세로 이미지는 이 오프라인 테스트로 검증).
- **수정 후(20261126T131949Z, `--label fix1`) 결과**: 4시나리오 모두 succeeded(4.8~11.3s)·전 항목 ✓(재시도 0, 사실 9/9·9/9·5/5·3/3, 넘침·잘림·글리프 누락 0, 이미지 비율 오차 0%, 최대 확대 2.0배). tiny 마지막 슬라이드의 이미지는 288×52pt 로 줄어 얼룩 없이 표시(PNG 확인). handbook 은 LLM 변동으로 14→9개 슬라이드(장별 구성)였고 둘 다 정상.
- **PDF 크기 개선 제안(미적용)**: 쪽·내용과 무관하게 항상 약 1.5MB(1.49~1.51MB). 원인: printpdf 가 폰트를 서브셋하지 않고 NanumGothic Regular+Bold(원본 2.05MB×2)를 통째로 임베드 — printpdf 0.13 의 `PdfSaveOptions.subset_fonts`(기본 true)는 `text_layout` 기능이 있어야 동작하는데 `engine/Cargo.toml` 이 `default-features = false, features = ["jpeg"]` 라 꺼져 있음(`text_layout` 은 azul-layout 의존성을 끌어옴 → 빌드 시간·의존성 증가, 효과는 미측정). 대안: 폰트를 KS X 1001 + ASCII 로 사전 서브셋해 `assets/fonts/` 교체(약 0.4MB 급 예상, 측정 안 함). 둘 다 이 티켓의 target_files(Cargo.toml·assets) 밖이라 적용하지 않음 — 필요하면 별도 티켓.
- **엔진 검증**: `verify-engine.sh`(set -euo pipefail) — fmt·clippy `-D warnings`·`cargo test` 372 passed/0 failed·`cargo sqlx prepare --check -- --all-targets`(쿼리 변경 없음)·docker build 통과. 라이브는 격리 스택(18080)에서만, 종료 후 verify 컨테이너 0, 사용자 스택 미접촉, 결과·로그에 키 없음. 디스크 여유 23GB 유지.
- **미검증/한계**: ① 슬라이드 내용의 '충실도'(요약이 원문을 빠짐없이 대표하는지)는 핵심 사실 적중·숫자 대조로만 확인 — 서술 품질 판정은 사람이 `slide-check/*/sheet.png` 를 열어 확인 필요. ② 텍스트 넘침·잘림 경로(`…` 말줄임)는 LLM 이 120자 제한을 지키는 한 라이브에서 발생하지 않아(최장 문장 92자) 렌더러 단위 테스트(`maximum_content_and_awkward_text_still_render`)로만 검증. ③ 세로로 긴 이미지는 라이브 픽스처가 없어 오프라인 테스트만. ④ 슬라이드 수·구성은 LLM 변동이 큼(handbook 14↔9) — 프롬프트에 슬라이드 수 지침이 없음(1~15개)이며 근거 약해 변경하지 않음.

### [2026-11-26 13:45] [T-408] [orch-subagent] — DONE
- **작업 내용**: 인스펙터에서 RAG 챗봇(kind `rag`) 노드 선택 시 해당 노드 인덱스에 질의하는 채팅 UI 추가.
  - `web/src/api/queries.ts`: `useQueryRag()` mutation(캐시 무효화 없음).
  - `web/src/lib/ragIndex.ts`: 순수 `resolveRagIndex(workflowId, nodeId, selectedRunId, deps)` + `parseRagApi` + `useRagIndex` 훅. 런 선택: RunPanel 에서 선택/표시 중인 런(EditorPage `activeRun` → `<Inspector selectedRunId>` prop)에서 노드가 succeeded 면 그 런, 아니면 `listRuns` 최신순 상위 10개를 하나씩 `getRun` 해 첫 succeeded 런(지연 조회). `api` 아티팩트를 fetch → `type==='rag_api'`·UUID indexId 검증. 쿼리 키에 표시 런의 노드 상태(editorStore.runNodeStatus) 포함 → 라이브 런이 완료되면 자동 재해석.
  - `web/src/store/ragChatStore.ts`: indexId 별 대화 기록(zustand, 세션 메모리), `대화 지우기`.
  - `web/src/lib/citations.ts`: `[n]`/`[n, m]` 인용 분리(1..sources.length 범위만 배지, 나머지는 텍스트). 번호 규칙은 `engine/src/api/rag.rs::user_prompt`(`[i + 1]`, sources 순서)와 그 단위 테스트 `prompt_numbers_the_passages_in_order` 로 확인.
  - `web/src/components/panels/RagChat.tsx` + Inspector(출력 토글 아래) 연결, EditorPage 가 activeRun id 전달.
- **실행 명령어**:
  - `cd web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` -> 통과(22 files / 153 tests, 신규 ragIndex 7 + RagChat 7)
  - `E2E_LIVE=1 ./scripts/verify-stack.sh --ui` -> exit 0, Playwright 6 passed(라이브 rag chat 7.2s, 답변에 "482", 근거 2개). verify-stack.sh 는 환경을 그대로 상속하므로 수정 불필요.
  - `./scripts/verify-stack.sh --ui` (플래그 없음) -> 라이브 테스트 skip, 나머지 통과
- **발견 사항 / 후속 제안**:
  - report_en 은 청크 2개뿐이라 topK 기본 5여도 근거 2개만 반환됨(정상).

### [2026-11-26 23:55] [T-317] [w1-kobe] — DONE
- **작업 내용**: fill 노드(AcroForm 채우기) 실사용 품질 점검·보강. **픽스처 3종 추가**(`engine/tests/fixtures/make_forms.py` 가 reportlab(extractor venv)으로 생성, 각 45~52KB): `form_ko.pdf`(한글·영문 필드명 혼합 신청서: 이름/생년월일/연락처/동의/성별/지역 + address/email/memo, 라디오 export 는 ASCII male/female 인데 옆 라벨은 남/여, 콤보는 export busan·표시 부산), `form_codes.pdf`(같은 양식에 필드명이 Text1~6·Check1·Group1(Choice1/2)·Dropdown1 — 의미는 페이지 라벨에만 있음), `form_multi.pdf`(3쪽: 같은 이름 name×2·phone×3 반복, 라벨이 필드 위에 있는 쪽, 3-위젯 라디오 행 q1~q5, 체크박스). reportlab 한계 2건을 생성 스크립트에서 우회(콤보는 기본값이 필요해 첫 옵션 서울/seoul 이 미리 선택됨; 한글 콤보 표시명을 Latin-1 외관 스트림에 못 쓰는 KeyError → 외관 스트림에서만 `?` 로 대체, `/Opt` 는 UTF-16 그대로).
- **점검 스크립트 `scripts/live/fill_check.sh`**(BASE_URL 필수·LIVE_CONFIRM=1 게이트·`--with-verify-stack`·`--no-web`·`--label`·`--cases`): bundle→pick_pdf + json_input → fill 4케이스 — ① ko(form_ko, 한글 키 이름/생년월일/주소/연락처/이메일/동의함/성별="남자"/지역="부산"/비고) ② en(form_ko, 영문 snake_case + 오타 키 name/birthdate/adress/emial/agree:true/gender/region/memo) ③ codes(form_codes, 한글 키) ④ multi(form_multi, 반복 이름·라디오 행·"예"/"아니오"). 케이스마다 **모든 필드의 기대 최종값**을 정의(채우지 않을 필드는 빈 값·Off·기본값 유지)하고 pypdf 로 결과 PDF 의 위젯 값을 읽어 대조(일치율·오매핑=기대와 다른 값·누락), `_work/fill_report.json`(격리 스택 engine 컨테이너에서 `docker compose exec -T engine cat`)의 방식 분포(exact/normalized/llm)·리포트 정확성(리포트 항목이 PDF 실제 값과 일치하는지), pypdfium2 form-fill 렌더 전후 픽셀 차이로 '채운 값이 눈에 보이는지'(한글 포함)·PNG. 유료 fill 실행 총 8회(4케이스×전/후; 스택 기동 실패한 첫 시도는 호출 0회). `--no-web`: web 이미지 빌드가 다른 트랙의 작업 중 변경(`web/src/lib/citations.ts` TS2532 — RAG 챗 UI 작성 중)으로 실패해 db+extractor+engine 만 기동하고 engine 컨테이너 IP:8080 으로 직접 호출하는 옵션을 추가(기본 동작 불변; fill 점검은 web 이 필요 없음).
- **베이스라인(20261126T132936Z) 결과**: ko 89%(8/9)·en 100%·**codes 22%(2/9)**·**multi 80%(12/15)**, **오매핑 0**, 한글 값 렌더 정상, 리포트 정확. 실제 문제 3건(근거): ① codes — 키(이름·생년월일…)와 필드명(Text1…)이 안 맞고 LLM 프롬프트에는 필드 이름·종류만 있어 LLM 이 6개 키를 null 로 반환(잘못 채우지 않은 점은 안전, 그러나 채우지 못함), 라디오 '여자' 도 선택지 Choice1/2 에 없어 실패. ② ko — 라디오 값 "남자" 가 선택지 male/female 에 없어 실패(경고만). ③ multi — 같은 이름 필드가 여러 개(name×2, phone×3)인데 첫 필드만 채워짐(나머지 3개 누락; 오프라인 프로브로도 확인).
- **수정(engine/src/nodes/fill/**)**: ① 신규 `labels.rs` — 페이지 콘텐츠의 텍스트 연산자(`BT/Tm/Td/TD/T*/Tf/Tj/TJ/'/"`, `q/Q/cm`)를 재생해 글자 덩어리와 위치를 얻고(폰트 인코딩은 lopdf `get_font_encoding`/`decode_text` — 한글 라벨 확인), 한 줄에서 쪼개진 라벨은 병합한 뒤, 각 위젯의 가장 가까운 라벨을 고른다: 텍스트·콤보는 왼쪽→위, 체크박스는 오른쪽→왼쪽→위, 라디오 그룹 라벨은 첫 위젯의 왼쪽→위, **라디오 옵션별 라벨은 각 위젯의 오른쪽**. `/TU` 툴팁도 수집. ② LLM 프롬프트에 필드별 `label`/`tooltip`/라디오 옵션 라벨 포함(`- Text1 (text) — label: "성명"`, `- Group1 (radio: Choice1 "남" | Choice2 "여") — label: "성별"`) + 시스템 프롬프트에 코드형 필드명은 라벨로 판단하라는 지침. 값은 여전히 전송하지 않음(라벨은 PDF 의 고정 문구). ③ `radio_choice`: 라디오 값 → 옵션 이름(정확→정규화) → 옵션 라벨(동일, 그다음 **정확히 하나일 때만** 포함 관계: "남자"⊃"남") — 애매하면(두 라벨 모두 포함·동일 라벨 중복) 고르지 않고, 경고에 옵션과 라벨("male (남)")을 보여 준다. ④ `expand_same_name`: 키가 어떤 필드에 매칭되면 **같은 이름의 다른 필드(여러 쪽 반복)도 함께 채움**(다른 키가 이미 가져간 필드는 제외), LLM 에게는 같은 이름을 한 번만·이미 매칭된 이름은 제시하지 않음. 응답 계약(`pdf` 소켓·`fill_report.json` 형식) 불변; 반복 이름은 채운 필드마다 `filled` 항목이 하나씩 생김.
- **회귀 테스트**: 단위 — `labels`(행렬 합성·왼쪽/오른쪽/위 라벨 선택·먼 글자 제외·조각 병합 5개), `form`(`radio_choice` 이름→라벨·모호하면 거부 2개), `matching`(같은 이름 확장·프롬프트 라벨 표기 2개); 통합(`tests/fill.rs`, 실제 픽스처) — 라벨 탐지(왼쪽·위·오른쪽, 옵션 라벨 없는 라디오 행), LLM 프롬프트에 라벨 포함·값 미포함 + 매핑 결과 9개 필드 정확히 채움(Group1="Choice2" 포함), 라디오 값 5종(남자/여성/여/Male/female)과 "기타"는 추측하지 않고 경고, 반복 이름 전부 채움·리포트·untouched 목록, LLM 에 반복 이름은 한 번만·매칭된 이름 제외, 숫자·"예/아니오"·라디오 행 채움.
- **수정 후(20261126T133814Z, `--label fix1`) 결과**: **4케이스 모두 100%(9/9·9/9·9/9·15/15), 오매핑 0, 누락 0**, 채운 값 전부 렌더에 보임(한글 포함 — codes 폼 PNG 에서 이름·주소·비고의 한글 값, 라디오 '여' 선택, 체크박스 체크, 콤보 '대구' 확인), 리포트 오류 0. 방식 분포: ko exact 5/llm 4, en exact 1/llm 8(오타 키 포함), codes llm 9(전부 라벨 덕분), multi exact 16(반복 이름 포함). 소요 0.0~3.8s.
- **엔진 검증**: `verify-engine.sh`(set -euo pipefail) — fmt·clippy `-D warnings`·`cargo test` 387 passed/0 failed·`cargo sqlx prepare --check -- --all-targets`(쿼리 변경 없음)·docker build 통과. 라이브는 격리 스택(18080 프로젝트 dochestra-verify)에서만, 종료 후 verify 컨테이너 0, 사용자 스택 미접촉, 결과·로그에 키 없음. 디스크 여유 22GB.
- **미검증/한계**: ① 라벨 추정은 "왼쪽/위/오른쪽 가장 가까운 글자" 휴리스틱 — 표 형태(라벨이 표 헤더에 있고 한 칸에 여러 필드)·스캔 이미지로 된 라벨·라벨이 필드에서 250pt 보다 먼 양식은 못 찾음(그 경우 기존처럼 필드명으로만 매핑). 글자 폭은 ASCII 0.5em/그 외 1em 추정(글리프 폭표를 쓰지 않음). 실제 한컴·워드 출력 PDF(Type0/Identity-H + ToUnicode)로는 시험하지 못함 — reportlab 서브셋 폰트와 lopdf 인코딩 경로만 확인. ② 라디오·체크박스 값 변환은 결정적 규칙(이름·라벨·예/아니오 등)뿐 — 값이 라벨과 전혀 다르면(예: "남성" 대신 "수컷") 거부하고 경고(LLM 에 값을 보내지 않는 T-307 방침 유지; 값 의미 매핑에 값을 보내도 되는지는 제품 결정 필요). ③ 렌더 확인은 PDFium form-fill 기준 — 한글이 보이는 것은 PDFium 의 폰트 대체 덕이며 `NeedAppearances` 를 무시하는 뷰어에서는 값이 안 보일 수 있음(외관 스트림 직접 생성은 하지 않음, 한글 폰트 임베딩 필요). ④ 같은 이름의 필드가 여러 개일 때 하나의 값을 모두에 쓰는 것이 항상 옳은 것은 아님(예: 서로 다른 동반자 칸이 같은 이름) — PDF 규격상 같은 전체 이름은 같은 필드이므로 채택. ⑤ 콤보는 reportlab 제약으로 기본값(서울)이 미리 선택된 상태의 픽스처.

### [2026-11-26 22:50] [T-321] [w1-kobe] — DONE (T-320 은 이어서 진행)
- **원인**: `executor::tests::independent_branches_run_in_parallel` 가 "200ms 짜리 두 노드의 총 소요 < 350ms" 라는 벽시계 단언을 썼다 — 병렬이면 ~200ms 이지만 CPU 부하로 스케줄링이 150ms 이상 밀리면(검수 시 load avg 3.5/4코어) 병렬이어도 실패. 순서 논리(스팬 겹침)는 이미 맞았고 시간 임계값만 문제.
- **수정(engine/src/executor/mod.rs, 테스트 모듈만)**: `FakeRunner` 에 `rendezvous(&[노드…], timeout)` 추가 — 지정한 노드들이 `tokio::sync::Barrier` 로 서로를 기다려야만 끝난다(노드가 timeout 안에 못 만나면 `Failed("node … never ran concurrently with […]")`). `independent_branches_run_in_parallel` 은 p1·p2 를 rendezvous(5s)로 묶고 `Succeeded` + 스팬 겹침만 단언 — 직렬 실행기는 첫 노드에서 막혀 5s 뒤 명확한 메시지로 실패(멈추지 않음), 부하가 있어도 두 노드가 모두 시작되기만 하면 통과하므로 시간 임계값이 없다. 대조군 `the_rendezvous_fails_for_nodes_that_cannot_run_together`: 서로 의존하는 b→p 체인을 rendezvous(100ms)로 묶으면 반드시 Failed + 위 메시지 → 장치가 '항상 통과' 가 아님을 증명. 벽시계 단언 제거(`elapsed < 350ms` 삭제, 노드 지연(`with("p1", 200…)`)도 제거).
- **다른 타이밍 의존 테스트 점검(grep `elapsed`/`Instant::now`/`sleep`)**: ① `src/clients/common.rs::backoff_doubles_and_retry_after_wins` 의 두 번째 단언 `start.elapsed() < 500ms`(Retry-After 0 이면 1s 기반 백오프 7s 를 건너뛰었는지 확인)는 상한 벽시계 단언 — 부하에서 이론상 불안정(첫 번째 `>= 70ms` 하한 단언은 안전). **target_files 밖이라 수정하지 않음** — 후속 제안: tokio `test-util`(start_paused)로 가상 시계를 쓰거나 호출 횟수·전달된 지연값으로 검증. ② `tests/runs_api.rs`·`nodes_run.rs`·`artifacts_api.rs` 의 `sleep(25ms)` 는 run 완료 대기 폴링(상한 5~10초 반복)이지 속도 단언이 아니고, `every_connection_sees_monotonic_node_statuses` 는 시각을 흩뿌려 접속하지만 불변식(단조성)만 단언 → 변경 불필요.
- **검증**: CPU 부하(`yes > /dev/null` 6개, 4코어) 중 `cargo test --lib executor::` 20회 반복 — 20/20 통과(종료 후 부하 프로세스 정리 확인). 표준 검증(`verify-engine.sh`, pipefail): fmt·clippy `-D warnings`·`cargo test` 389 passed/0 failed·`cargo sqlx prepare --check`·docker build 통과. (첫 실행은 다른 워커가 같은 시각에 편집 중이던 `tests/llm_nodes.rs::analyze_prompt_prescribes_the_section_structure…`(T-313 작업 중) 때문에 일시 실패 — 내 변경과 무관, 수 초 뒤 재실행 통과.)

### [2026-11-26 23:35] [T-320] [w1-kobe] — DONE
- **문제(T-314 기록)**: slide PDF 가 내용과 무관하게 항상 약 1.5MB — printpdf 가 폰트를 서브셋하지 않고 NanumGothic Regular+Bold(각 2MB)를 통째로 임베드.
- **원인 확인(crate 소스)**: printpdf 0.13 의 `PdfSaveOptions.subset_fonts`(기본 true)는 `#[cfg(feature = "text_layout")]` 일 때만 동작(`serialize.rs::prepare_fonts_for_streams` 가 feature 없으면 `create_full_font_runtime_info` 로 전체 임베드). `engine/Cargo.toml` 이 `default-features = false, features = ["jpeg"]` 라 꺼져 있었음.
- **수정**: `engine/Cargo.toml` printpdf features 에 `text_layout` 추가(+ 이유 주석). `text_layout` 에서는 `ParsedFont` 가 azul-layout 래퍼로 바뀌어 `render.rs::Face::char_width` 를 새 API 로 조정(`pdf_font_metrics.units_per_em`, `get_horizontal_advance(gid)`; 글리프 없을 때의 0.5em 대체는 동일). 레이아웃(줄바꿈·폭)은 동일해야 하므로 아래에서 픽셀 비교로 검증. 새 의존성(Cargo.lock): azul-core/-css/-layout 0.0.21, azul-simplecss 0.2.3, rust-fontconfig 5.2, taffy 0.10, slotmap, mmapio, lru, grid, arrayvec, gl-context-loader, unicode-segmentation, xmlparser, winapi 3종 — 라이선스 MIT/Apache-2.0/Zlib, **단 azul-simplecss 는 MPL-2.0(파일 단위 약한 카피레프트)** — 의존성 정책상 문제되면 대안은 폰트 사전 서브셋.
- **효과(실제 LLM 구성 8개 덱, T-314 라이브 결과 재사용)**: en 1,506,059→51,226B, ko 1,503,917→61,059B, handbook 1,505,348→49,462B, tiny 1,492,455→34,927B(**이미지 포함** 기준 약 96~98% 감소; 목표 ≤300KB 이미지 제외 충족). 합성 덱: 4슬라이드 한글 덱 48KB, 최대 덱(15슬라이드×7불릿×120자 한글) 300KB 미만(테스트로 고정).
- **렌더 동일성 검증(유료 호출 없음)**: T-314 `fix1` 라이브 결과의 `slides.json`+대표 이미지로 새 렌더러가 만든 PDF 4개(총 38쪽)를 기존 1.5MB PDF 와 쪽별로 pypdfium2(1.4배)로 렌더해 픽셀 비교 — **변경된 쪽 0개, 최대 픽셀 차이 0.000000**, pdfium 추출 텍스트도 38쪽 모두 동일(영문·한글·혼합·tiny). 비교용 개발 도구로 `tests/slide.rs::render_deck_from_files`(`#[ignore]`, 환경변수 `SLIDE_DECK_JSON/IMAGE/OUT`) 추가.
- **회귀 테스트(tests/slide.rs)**: `fonts_are_embedded_as_subsets_and_the_pdf_stays_small`(폰트 프로그램 각 <150KB(전체는 2MB)·Regular+Bold 2개 이상·일반 덱 <300KB·최대 한글 덱 <300KB), `subset_fonts_keep_every_character_extractable`(lopdf 텍스트 추출로 한글·영문·숫자·기호 전부 복원 — 서브셋의 글리프 재번호가 ToUnicode 로 되돌아오는지). 기존 slide 테스트 전부 통과(`font_files >= 2` 포함).
- **T-321 후속(오케스트레이터 허가, `engine/src/clients/common.rs` 테스트 모듈)**: `backoff_doubles_and_retry_after_wins` 의 벽시계 상한 단언(`< 500ms`)을 tokio 가상 시계(`#[tokio::test(start_paused = true)]`)로 교체 — 지연은 즉시 소화되고 `elapsed()` 가 지연의 정확한 합이므로 70ms(10+20+40)·Retry-After 0 → 0·Retry-After 2s → 정확히 6s 를 `assert_eq!` 로 단언(기기 부하와 무관, 실행 0.00s). 이를 위해 `[dev-dependencies]` 에 `tokio = { features = ["test-util"] }` 추가(테스트 빌드 전용 feature, Cargo.lock 변화 없음).
- **검증**: `verify-engine.sh`(set -euo pipefail) — fmt·clippy `-D warnings`·`cargo test` 391 passed/0 failed·`cargo sqlx prepare --check`·docker build(release 에 azul 계열 추가 컴파일) 통과, 이어서 `cargo test --lib executor::` 20회 반복 20/20. 유료 호출 0회; 라이브 slide_check 는 실행하지 않음(렌더 동일성은 실제 라이브 덱으로 오프라인 검증했고, web 이미지 빌드가 다른 트랙 작업 중 변경으로 막혀 있었으며 디스크 여유가 줄어 있었음).
- **디스크 주의**: 이 티켓의 docker build(새 의존성 컴파일 → 빌드 캐시 증가)로 `df` 여유가 **20GB → 13GB** 로 줄었음(사용자 지시상 docker prune/이미지 삭제는 하지 않음). `engine/target` 이 19GB. 10GB 미만이면 라이브 점검은 중단 규칙에 걸림 — 오케스트레이터가 builder prune 을 할 수 있으면 여유 확보 권장.
- **미검증/한계**: ① 라이브 스택(release 이미지 안)에서 서브셋 PDF 를 직접 만들어 보지는 않았음 — 같은 코드·같은 폰트 파일이고 debug 테스트·실제 덱 렌더 비교로 갈음. ② 서브셋은 시스템 뷰어(Acrobat 등)의 호환성까지 확인한 것은 아님(pdfium 만) — printpdf 가 표준 CIDFontType2/Identity-H + ToUnicode 로 쓴다고 문서화되어 있음. ③ MPL-2.0 의존성(azul-simplecss) 1개 추가.

### [2026-11-26 23:55] [T-313] [w2-fine] — DONE (오케스트레이터 지시로 적당한 선에서 마무리: 추가 유료 실행·개선 중단)
- **작업 내용**: analyze 점검·보강. 픽스처 `fixtures/live/vision_{chart,table,text,photo,small}.pdf`(각 PDF 에 이미지 1장 + 제목·소개 문장·캡션: 문맥이 없으면 Docling 이 단독 이미지를 picture 로 인식하지 않음 — 표 이미지는 table, 포스터는 텍스트로 처리됨; 표는 '책상 위 사진' 처럼 기울임·그림자·노이즈를 줘서 picture 로 인식시킴), 정답 `vision_facts.json`, 점검기 `scripts/live/analyze_check.py`(정답 값·표 행 일치율, 이미지에 없는 수치, 환각 단어, '## 요약' 구조, 한국어, 길이, 군더더기), 시나리오 `live_analyze_case`(extract→pick_jpg 0→analyze; 이미지(n_i)와 결과(n_a) 저장). 로컬 Docling 탐색(`extractor` uv 환경, 무료)으로 PDF 가 이미지 1장을 온전히 내도록 조정한 뒤 라이브 실행
- **베이스라인(수정 전) 결과**: 5/5 succeeded, 수치·텍스트 판독 정확. 문제: ① 응답 구조가 제각각(사진·포스터는 `# 이미지 분석` 같은 H1 로 시작, 소제목 이름도 실행마다 다름) ② 모델이 직접 계산한 차이값(20, 30, 40)이 '이미지에 없는 수치' 와 구분되지 않음 ③ "보이지 않음/불확실" 문구 지시가 모호(차트에서 존재하지 않는 제목·범례를 '보이지 않음' 으로 쓴 것은 실제로 추출 이미지에 없어서 정당)
- **수정(engine/src/nodes/analyze.rs 프롬프트)**: 시스템 프롬프트에 "한국어 Markdown 만, 이미지에 없는 내용 지어내지 않음"; 지시문에 고정 구조 `## 요약`(첫 줄 필수, H1 금지) → `## 이미지 속 텍스트`(제목·범례·축 이름·눈금·캡션·작은 글씨까지 읽은 그대로, 없으면 '없음') → `## 도표·수치`(차트·표일 때만; 직접 계산한 값은 '(계산값)' 표시) → `## 판독 불가·불확실`(없으면 생략) + 규칙(가장자리까지 확인 후에만 '보이지 않음', 읽을 수 없으면 추측하지 말고 '판독 불가', 없는 대상 서술 금지). 회귀 테스트 `analyze_prompt_prescribes_the_section_structure_and_forbids_guessing` 신설
- **수정 후 결과(라이브 1회, 5시나리오 모두 succeeded)**: 

| 시나리오 | 소요 | 정답 일치율 | 이미지에 없는 수치 | 환각 | `## 요약` 시작 | 비고 |
| :--- | ---: | :--- | :--- | :--- | :--- | :--- |
| chart | 6.4s | 100% (값 10/10, 행 4/4) | 0 | – | ✓ | 제목·범례는 추출 이미지에 없어 '보이지 않음' (정당) |
| table | 3.1s | 100% (값 20/20, 행 4/4) | 0 | – | ✓ | 사진처럼 기울어진 표도 전부 정확 |
| text | 5.7s | 100% (11/11 문구) | 0 | – | ✓ | 한글 포스터 OCR 정확 |
| photo | 4.2s | 100% (5/5 요소) | – | 0 | ✓ | 설명이 짧아짐(721자→109자) |
| small | 2.8s | 100% (4/4) | 0 | – | ✓ | 150px 이미지도 판독 |

  구조 일관성: 수정 전 3/5(photo·text 가 H1) → 수정 후 5/5. 차이값 '40' 같은 계산값이 '(계산값)' 으로 구분(이미지 수치로 오인되지 않음)
- **정답 보정**: chart 의 정답에 범례 값(2024/2025)을 넣었으나 Docling 이 제목·범례를 picture 밖 별도 요소로 잘라내 추출 이미지에 존재하지 않음(직접 이미지 확인) → `vision_facts.json` 에서 제외(사유 `_note`). 이는 analyze 가 아니라 extractor 크롭의 특성
- **미해결·되돌린 것**: (1) photo 설명이 베이스라인(721자)보다 짧아짐 — 구조화로 '한두 문장 요약 + 이미지 속 텍스트 없음' 만 남음. `## 시각 요소` 섹션(주요 대상·배치·색·분위기)을 추가했으나 **라이브 검증 전 오케스트레이터 지시로 중단 → 코드·테스트 모두 되돌림**(이번 변경에 포함되지 않음). 사진형 이미지를 상세 설명시키려면 후속 티켓 필요 (2) ⑥ 5MiB 초과 경로는 라이브로 만들 수 없음: Docling 이 페이지 렌더링(디코드) 20MiB 초과를 거부하고 extractor 의 q90 JPEG 가 노이즈 이미지로도 최대 ≈ 5MB 라 한계(실측 1.8~2MB, 계산상 ≤5.6MB) — 기존 mock 테스트 `analyze_sends_the_image_and_enforces_the_size_limit` 로만 검증(정확히 5MiB 통과, +1 바이트 거부, 오류 문구 '이미지가 너무 큽니다 (최대 5MB)')
- **유료 호출**: analyze 라이브 11회(1차 3: 레전드만 추출된 chart·photo·small / 2차 3: chart·table·text / 3차 5: 전체). 중단·실패한 3번의 실행(고아 빌드 포함)은 워크플로 생성 전이라 호출 0. **실수 기록**: 라이브 실행 중 live-nodes.sh 를 편집하면 bash 가 스크립트를 증분으로 읽다가 `TS: unbound variable` 로 죽고 trap 이 설치되기 전이라 격리 스택이 남음(수동 `down -v` 로 정리), 이어서 `pkill -f live-nodes.sh` 가 내 셸을 같이 종료시켰고 고아 `docker compose up --build` 를 PID·환경변수(LIVE_CASES=photo)로 확인해 내 것만 종료. 교훈: 실행 중 스크립트 편집 금지, pkill -f 패턴 주의
- **fill 제거(D-7)**: scripts/live-nodes.sh 의 `fill` 노드·`FORM_PDF`, scripts/live/nodes.sh 의 `live_estimate fill`·`live_node_fill` 삭제(plan 출력 확인). `scripts/live/fill_check.sh` 는 이미 삭제되어 있었음(내가 건드리지 않음)
- **엔진 검증**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` 전부 통과(작업 트리에 T-322(fill 제거) 진행분이 섞여 있었으나 통과), `cargo sqlx prepare --check -- --all-targets` 통과. 도커 빌드·마지막 라이브는 live 스택이 작업 트리로 빌드해 수행. 종료 후 dochestra-verify 컨테이너·볼륨 0, 잠금 해제 확인

### [2026-11-27 00:20] [T-322] [w1-kobe] — DONE
- **작업 내용(D-7: 폼 자동 입력 `fill` 노드 완전 제거)**: `NodeKind::Fill` 제거(카탈로그 11종, 순서는 기존에서 fill 만 뺀 것), `nodes/mod.rs` 의 모듈·디스패치 제거, `engine/src/nodes/fill/**`(form·labels·matching·mod)·`tests/fill.rs`·`tests/fixtures/{form,form_ko,form_codes,form_multi}.pdf`·`make_form.py`·`make_forms.py`·`scripts/live/fill_check.sh` 삭제(T-307·T-317 산출물 폐기 — fixtures 디렉터리는 비어 사라짐). `cargo test export_bindings` 로 `web/src/bindings/NodeKind.ts` 재생성(`"fill"` 만 사라지는 1줄 diff, 재생성 결정적). `tests/nodes_api.rs`·`catalog.rs` 단언 12→11. `scripts/smoke.sh` 노드 수 11. README: 노드 표에서 제거 + '알려진 제약' 에 'RFP 의 폼 자동 입력은 제외됨(D-7)·저장된 노드는 0004 로 제거' 추가. `lopdf`: fill 전용이던 일반 의존성을 제거하고 `tests/slide.rs` 의 PDF 검사용이라 **dev-dependency 로만 유지**(src 사용처 grep 0 확인; Cargo.lock 은 변화 없음).
- **마이그레이션 `0004_remove_fill_nodes.sql`**: `pg_temp.strip_fill_nodes(jsonb)`(임시 함수, 세션 종료 시 사라짐, `CREATE OR REPLACE` 로 재실행 가능)로 ① `workflows.graph` ② `runs.graph_snapshot` 에서 kind='fill' 노드와 그 id 를 source/target 으로 갖는 엣지를 제거(순서 보존, `jsonb_agg … ORDER BY ordinality`, 다른 그래프 키(viewport 등)도 `jsonb_set` 으로 보존) — `WHERE EXISTS(fill 노드)` 로 대상 행만 갱신하므로 fill 없는 행은 재작성되지 않고 `workflows.updated_at` 도 건드리지 않음 ③ `DELETE FROM node_runs WHERE kind='fill'`. fill 의 출력을 받던 하위 노드는 남고 입력이 비게 됨(에디터에서 노드를 지운 것과 같음). `.sqlx` 변경 없음(쿼리 매크로 불변).
- **마이그레이션 테스트(`tests/migrations.rs::migration_0004_removes_fill_nodes_from_stored_workflows_and_runs`, `#[sqlx::test(migrations = false)]`)**: 마이그레이션을 0003 까지만 적용 → fill 노드 2개(하나는 json_input 이 공급, 하나는 extract 가 소비)·엣지 6개·viewport 키를 가진 워크플로와 그 run(+node_runs 7개), fill 이 없는 워크플로·run 삽입 → **0004 적용 전에는 `GET /api/workflows/{id}`·`GET /api/runs/{id}` 가 200 이 아님**(알 수 없는 노드 kind; 이 상황이 사용자 DB 의 실제 문제)을 단언 → 정식 migrator 로 0004 적용 → 결과 검증: 남은 노드 [b,p,j,x,x2]·엣지 [e1,e5], 남은 노드의 position/params/isOutput 불변, viewport 보존, `updated_at` 불변, **fill 없는 워크플로·run 의 행(`xmin` 포함)이 그대로(재작성 안 됨)**, node_runs 의 fill 행 0개, **같은 데이터에 대해 GET /api/workflows/{id}·GET /api/runs/{id} 가 200** 이고 RunDetail 노드 목록에 fill 없음, 마이그레이션 SQL 재실행해도 데이터 불변(멱등).
- **잔존 grep**: 티켓의 `grep -rniE '\bfill\b|NodeKind::Fill|폼 자동 입력' src tests ../web/src/bindings ../scripts/smoke.sh` 가 0건(종료코드 1). 걸림 요인 2가지를 정리: ① printpdf 의 `PaintMode::Fill`(그래픽 채움, 무관)과 슬라이드 렌더러 주석 "to fill the closing slide" — `PaintMode::default()`(기본값이 Fill)와 주석 문구 변경으로 의미 변화 없이 해소(**`engine/src/nodes/slide/render.rs` 2줄: T-322 target_files 밖의 변경이므로 보고**, 이미 HEAD 에 반영돼 있음) ② 마이그레이션 테스트가 제거된 kind 이름을 알아야 하므로 그 한 곳에서 `concat!("fi", "ll")` 상수(`REMOVED_KIND`)로 표기하고 이유를 주석으로 남김 — 저장소 전체 잔존 검사가 예외 없이 유효하도록. (README·WORKLOG·PLAN·CONTEXT 와 migrations/*.sql 은 검사 대상 밖.)
- **검증**: `verify-engine.sh`(set -euo pipefail) — fmt·clippy `-D warnings`·`cargo test` **349 passed/0 failed**(fill 테스트 43개 삭제 + 마이그레이션 테스트 1개 추가)·`cargo sqlx prepare --check -- --all-targets`·docker build 통과. `cd web && pnpm typecheck`(통과)·`pnpm test --run`(22 파일/153 테스트 통과). `./scripts/verify-stack.sh --e2e`: 스모크 6항목(**`GET /api/nodes -> 11 nodes`** 포함) 및 e2e 9단계 전부 PASS, 종료 후 정리. 
- **디스크 기록**: 작업 중 `df` 여유가 7.1GB 까지 내려가(10GB 미만 → 무거운 작업 중단 규칙) `engine/target`(29GB, 의존성 변경이 누적된 내 빌드 디렉터리)을 `cargo clean` 으로 비워 36GB 확보 후 재빌드·재검증함(docker prune 은 하지 않음). 현재 15GB.
- **미검증/한계**: 사용자 실제 DB(동의서 워크플로 포함)에는 적용하지 않음 — 배포 후 오케스트레이터가 로드 확인 예정이며, 같은 상황(fill 포함 워크플로·run)을 sqlx 테스트 DB 에서 재현해 200 을 확인함. 디스크에 남은 fill 노드의 run 산출물 파일(`/data/runs/<run>/<fill 노드>`)은 지우지 않음(런 폴더 정리는 별도). `scripts/live-nodes.sh` 의 fill 시나리오·`FORM_PDF`·`fixtures/live/**` 는 지시대로 fine 이 T-313 후 정리(건드리지 않음).
