# Dochestra

PDF 문서를 중심으로 추출·번역·요약·슬라이드 생성·음성 해설 같은 작업을 **노드로 연결해 구성하고 실행**하는 비주얼 워크플로 에디터입니다.

- **노드**: 하나의 작업 단위(예: PDF 추출, 요약). 입력 소켓과 출력 소켓을 가집니다.
- **엣지**: 한 노드의 출력 소켓을 다른 노드의 입력 소켓에 연결하는 선. 데이터가 이 선을 따라 흐릅니다.
- **데이터 타입**: 소켓은 `PDF`, `MD`, `JSON`, `CSV`, `JPG`, `MP3` 중 하나이거나 그 목록(`LIST[JPG]` 등)입니다. 타입이 다른 소켓끼리는 연결되지 않습니다.
- **실행**: 출력 노드로 지정한 노드에서 거꾸로 거슬러 올라가 필요한 노드만, 서로 독립인 것은 병렬로 실행합니다. 진행 상태는 캔버스에 실시간으로 표시됩니다.

## 아키텍처

단일 Docker Compose 네트워크의 4개 컨테이너입니다. 호스트에 공개되는 포트는 `web`의 80(기본값)뿐입니다.

| 서비스 | 역할 | 스택 | 포트 |
| :--- | :--- | :--- | :--- |
| `web` | SPA 정적 파일 서빙, `/api/*` 를 engine 으로 프록시 | React 19, Vite, TypeScript, nginx | 80 (호스트 공개) |
| `engine` | REST API·SSE, 그래프 검증, 병렬 실행기, 노드 구현, 외부 AI 호출 | Rust, Axum, SQLx, Tokio | 8080 (내부) |
| `extractor` | PDF → MD / JSON / CSV / JPG 추출 | Python 3.12, FastAPI, Docling (CPU torch) | 8000 (내부) |
| `db` | 워크플로·실행 기록·번들·RAG 벡터 저장 | PostgreSQL 16 + pgvector | 5432 (내부) |

```
브라우저 → web(:80) ──/api──▶ engine(:8080) ──▶ db(:5432)
                                  └──────────▶ extractor(:8000)
                                  └──────────▶ OpenRouter (외부 AI)
        공유 볼륨 data: engine:/data ⇄ extractor:/data   (업로드 PDF와 노드 산출물)
```

## 빠른 시작

**요구사항**: Docker 와 Docker Compose v2. 메모리 6 GiB 이상 권장(Docling 변환 시 extractor 가 약 1.5 GiB 사용, 컨테이너 한도 4 GiB).

```bash
cp .env.example .env
# .env 를 열어 OPENROUTER_API_KEY 등을 입력합니다(아래 "환경 변수" 참고).
# 키 없이도 기동되며, 외부 AI 를 쓰는 노드만 실행 시 실패합니다.

docker compose up -d --build --wait
```

브라우저에서 <http://localhost> 를 엽니다(`WEB_PORT` 로 포트 변경 가능).

- **첫 빌드는 오래 걸립니다.** extractor 이미지는 Docling 모델을 빌드 단계에서 미리 내려받아 포함하므로 약 6 GB 이고 처음 수 분~십수 분이 걸립니다. 이후에는 캐시를 사용합니다.
- extractor 는 기동 후 모델을 메모리에 올리는 동안 `loading` 상태이며, engine 은 extractor 가 준비된 뒤에 요청을 받습니다(`--wait` 가 모든 서비스가 healthy 가 될 때까지 기다립니다).
- 중지: `docker compose down` (데이터 볼륨 `data`, `pgdata` 는 유지). 데이터까지 지우려면 `docker compose down -v`.

## 환경 변수

루트 `.env` 에 설정합니다(커밋되지 않음). 템플릿은 [`.env.example`](./.env.example) 입니다. 값에 비밀이 들어가므로 `.env` 는 공유하지 마세요.

| 키 | 사용처 | 설명 / 기본값 |
| :--- | :--- | :--- |
| `POSTGRES_USER` / `POSTGRES_PASSWORD` / `POSTGRES_DB` | db | `dochestra` / (직접 설정) / `dochestra` |
| `DATABASE_URL` | engine | `postgres://dochestra:<password>@db:5432/dochestra` — `POSTGRES_*` 와 일치해야 합니다 |
| `EXTRACTOR_URL` | engine | `http://extractor:8000` |
| `DATA_DIR` | engine, extractor | `/data` (공유 볼륨 마운트 위치) |
| `ENGINE_BIND` | engine | `0.0.0.0:8080` |
| `RUST_LOG` | engine | `info,dochestra_engine=debug` |
| `EXTRACTOR_HEALTH_REQUIRED` | engine | `true` — 로컬 개발에서 `false` 로 두면 시작 시 extractor 대기를 생략 |
| `EXTRACTOR_HEALTH_TIMEOUT_SECS` | engine | `600` |
| `EXTRACTOR_OCR` | extractor | `false` — `true` 로 두면 Docling OCR 사용(CPU·메모리 증가) |
| `WEB_PORT` | compose | `80` — web 의 호스트 포트(검증 스택은 `18080`) |
| `OPENROUTER_API_KEY` | engine | `<your-openrouter-key>` — LLM·임베딩·TTS 호출에 공통으로 쓰는 [OpenRouter](https://openrouter.ai) 키. 미설정이면 해당 노드만 실패하고 엔진은 정상 기동 |
| `OPENROUTER_BASE_URL` | engine | `https://openrouter.ai/api/v1` |
| `LLM_MODEL` / `LLM_EFFORT` | engine | `meta/muse-ember` / `medium` |
| `EMBEDDING_MODEL` | engine | `baai/bge-m3` — **임베딩 차원은 1024 로 고정**(DB 컬럼 `vector(1024)`). 다른 차원의 모델은 오류가 납니다 |
| `TTS_MODEL` / `TTS_VOICE` | engine | `google/gemini-3.8-flash-lite-tts` / `Kore` |

> 외부 AI 공급자는 OpenRouter 하나로 통합되었습니다(상세: [`CONTEXT.md`](./CONTEXT.md) §4.3, §6.1). 아직 `.env.example` 이 이전 키 이름을 담고 있다면 위 표를 기준으로 `.env` 를 작성하세요.

## 사용법

1. **워크플로 만들기**: 좌측 사이드바에서 이름을 입력하고 `생성`. 목록에서 선택하면 열리고, `✕` 로 삭제합니다.
2. **노드 배치**: 왼쪽 팔레트에서 노드를 클릭하거나 캔버스로 끌어다 놓습니다. 노드는 끌어서 옮길 수 있고 선택 후 `Delete`/`Backspace` 로 지웁니다.
3. **소켓 연결**: 출력 소켓(오른쪽)에서 입력 소켓(왼쪽)으로 드래그합니다. 소켓 색은 타입을 뜻하며 다음 규칙이 적용됩니다.
   - 타입이 완전히 같아야 연결됩니다(`PDF` 와 `LIST[PDF]` 는 다름 — 자동 변환 없음).
   - 입력 소켓에는 선을 하나만 연결할 수 있습니다(출력 소켓은 여러 곳으로 나눠도 됩니다).
   - 순환(사이클)이 생기는 연결은 거부됩니다.
4. **번들 준비**: `PDF 번들` 노드를 선택하면 우측 인스펙터에 파란 `PDF 업로드` 버튼이 보입니다. 버튼을 눌러 PDF 를 고르거나 점선 영역에 PDF 를 끌어다 놓으면 업로드되고(여러 개 가능, 요청당 최대 200 MiB), 번들이 없으면 첫 파일 이름으로 **번들이 자동으로 만들어져** 노드에 연결됩니다. 이미 만든 번들을 쓰려면 아래 `다른 번들 선택` 에서 고르거나 이름을 입력해 `새 번들` 을 만듭니다.
5. **파라미터와 출력 지정**: 인스펙터에서 `index`(`PDF 선택`/`이미지 선택`), JSON 값(`JSON 입력`) 등을 편집하고, 결과를 얻을 노드에 `출력 노드로 지정` 을 체크합니다. 출력 노드가 하나도 없으면 실행되지 않습니다.
6. **저장 · 검증 · 실행**: 상단 `저장`(변경이 있으면 `저장되지 않음` 표시), `검증`(연결 누락·타입 불일치 등을 목록으로 표시하고 해당 노드를 빨간 테두리로 강조), `실행`. 검증·실행은 저장되지 않은 변경을 먼저 저장합니다.
7. **결과 확인**: 하단 실행 패널에서 노드별 상태(대기·실행 중·완료·실패·건너뜀)와 오류, 산출물 링크·미리보기를 봅니다. 한 노드가 실패하면 그 아래 노드는 건너뛰고 독립 경로는 계속 실행됩니다. `실행 이력` 에서 이전 실행을 다시 볼 수 있습니다.

최소 예시: `PDF 번들` → `PDF 선택` → `추출` 을 연결하고 `추출` 을 출력 노드로 지정한 뒤 실행하면 Markdown·JSON·표(CSV)·그림(JPG) 이 산출됩니다.

## 노드

| 노드 (`kind`) | 입력 | 출력 | 외부 AI(OpenRouter) |
| :--- | :--- | :--- | :--- |
| PDF 번들 (`bundle`) | — | `files`: LIST[PDF] | — |
| PDF 선택 (`pick_pdf`) | `files`: LIST[PDF] | `pdf`: PDF | — |
| 이미지 선택 (`pick_jpg`) | `images`: LIST[JPG] | `image`: JPG | — |
| JSON 입력 (`json_input`) | — | `json`: JSON | — |
| 추출 (`extract`) | `pdf`: PDF | `md`: MD, `json`: JSON, `tables`: LIST[CSV], `images`: LIST[JPG] | — (extractor) |
| 번역 (`translate`) | `md`: MD, `settings`: JSON | `md`: MD | LLM |
| 이미지 분석 (`analyze`) | `image`: JPG | `md`: MD | LLM(이미지 입력) |
| 요약 (`summary`) | `md`: MD, `images`: LIST[JPG] *(선택)* | `md`: MD | LLM |
| 슬라이드 생성 (`slide`) | `md`: MD, `image`: JPG | `pdf`: PDF | LLM |
| 해설 음성 생성 (`voice`) | `md`: MD, `images`: LIST[JPG] | `mp3`: MP3 | LLM + TTS |
| RAG 챗봇 (`rag`) | `md`: MD | `api`: JSON | 임베딩 + LLM |

`RAG 챗봇` 노드는 입력 MD 를 색인하고, 질의 API 를 설명하는 JSON 을 출력합니다. 질의는 `POST /api/rag/{indexId}/query`(`{"question": "...", "topK": 5}`)로 보냅니다.

## 개발 · 검증

호스트에는 Docker, Rust(`rustup`), Node 22(+corepack pnpm), `uv` 가 필요합니다. 모듈별 표준 검증 명령은 다음과 같습니다.

| 모듈 | 명령어 |
| :--- | :--- |
| engine | `cd engine && source ~/.cargo/env && cargo fmt --check && cargo clippy --all-targets -- -D warnings && DATABASE_URL=postgres://dochestra:dochestra@127.0.0.1:55432/dochestra cargo test` (테스트용 DB: `engine/scripts/dev-db.sh`) |
| extractor | `cd extractor && uv sync --frozen && uv run ruff check . && uv run ruff format --check . && uv run pytest -q` |
| web | `cd web && pnpm install --frozen-lockfile && pnpm lint && pnpm typecheck && pnpm test --run && pnpm build` |
| 이미지 빌드 | `docker build -t dochestra-<svc> <svc>/` |

pnpm 은 `corepack enable --install-directory ~/.local/bin pnpm` 후 `export PATH=$HOME/.local/bin:$PATH` 로 사용합니다.

### 통합 검증: `scripts/verify-stack.sh`

기본 스택(`dochestra`, 포트 80)과 **완전히 분리된** 일회용 프로젝트 `dochestra-verify`(포트 `18080`, 전용 볼륨)를 만들어 검증하고, 끝나면 모두 지웁니다. 동시 실행은 `flock` 으로 직렬화됩니다. 실행 중인 기본 스택에는 영향을 주지 않습니다.

```bash
./scripts/verify-stack.sh                 # 빌드·기동·smoke(health, SPA, 노드 카탈로그, pgvector, engine→extractor)
./scripts/verify-stack.sh --e2e           # + API 시나리오(scripts/e2e.sh: 업로드→실행→SSE→산출물)
./scripts/verify-stack.sh --ui            # + 브라우저 UI 시나리오(Playwright, web/e2e/)
./scripts/verify-stack.sh --e2e --ui      # 모두
```

- 실패하면 정리 전에 컨테이너 상태·로그를 `/tmp/dochestra-verify-logs/<UTC시각>/` 에 저장합니다(`VERIFY_LOG_DIR` 로 변경, 최근 10개 유지).
- `scripts/smoke.sh`·`scripts/e2e.sh` 는 이미 떠 있는 스택을 대상으로 하는 개별 스크립트입니다. `e2e.sh` 는 `BASE_URL` 이 필수입니다(기본 스택 오사용 방지).
- `--ui` 는 Playwright 용 chromium 이 필요합니다: `cd web && pnpm exec playwright install chromium` (리눅스에서 시스템 라이브러리가 부족하면 관리자 권한으로 `playwright install-deps chromium` 이 필요할 수 있습니다).

## 문서 지도

| 문서 | 내용 |
| :--- | :--- |
| [`docs/RFP.md`](./docs/RFP.md) | 제품 요구사항과 기술 스택(원천 문서) |
| [`CONTEXT.md`](./CONTEXT.md) | 구체화된 기술 계약: 디렉토리 구조, 타입, API·SSE, DB 스키마, 환경 변수, 컨벤션 |
| [`PLAN.md`](./PLAN.md) | 마일스톤·티켓·진행 상태 |
| [`WORKLOG.md`](./WORKLOG.md) | 실행·디버깅 누적 기록 |
| [`CLAUDE.md`](./CLAUDE.md) | 에이전트 공통 운영 지침 |
| [`docs/agents/ORCHESTRATOR.md`](./docs/agents/ORCHESTRATOR.md) / [`docs/agents/WORKER.md`](./docs/agents/WORKER.md) | 역할별 지침 |

## 알려진 제약

- **리스트 자동 변환 없음**: `LIST[T]` 와 `T` 는 서로 다른 타입이며 자동 map 도 없습니다. 목록에서 하나를 고르려면 `PDF 선택`·`이미지 선택` 노드를 사용하세요.
- **인증 없음**: 로그인·사용자 구분이 없습니다. 신뢰할 수 있는 네트워크에서만 사용하세요.
- **로컬 저장**: 업로드 PDF 와 산출물은 Docker 볼륨 `data` 에, 메타데이터는 `pgdata` 에 저장되며 외부 스토리지는 지원하지 않습니다.
- **리소스**: Docling 변환은 CPU 와 메모리를 많이 사용하며 extractor 는 변환을 한 번에 1건씩만 처리합니다.
- **폼 자동 입력은 제외됨**: RFP 의 '폼 자동 입력'(`fill`) 노드는 구현하지 않습니다(D-7). 이미 저장된 워크플로·실행 기록의 해당 노드는 마이그레이션 `0004` 로 제거됩니다.
- 외부 AI 를 쓰는 노드는 OpenRouter 키와 네트워크 연결이 필요하며, 응답 품질·지연은 선택한 모델에 따릅니다.
