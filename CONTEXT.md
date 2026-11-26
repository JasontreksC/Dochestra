# CONTEXT.md — 시스템 기술 배경 및 인터페이스 계약서

> **관리 주체**: 오케스트레이터 전용 수정 파일 (모든 에이전트 작업 전 필독)
> **원천 문서**: [`docs/RFP.md`](./docs/RFP.md) (추상 요구사항 및 기술 스택 정의)
> **역할**: [`docs/RFP.md`](./docs/RFP.md)에서 추상적으로 남겨 둔 세부 구현 규약(디렉토리 구조, API/SSE 스키마, DB 스키마, 공유 볼륨 규칙, 코딩 컨벤션)을 **오케스트레이터가 구체화하여 확정·공유하는 기술 기준서**입니다. 워커는 본 문서의 계약을 임의로 변경하지 않고 그대로 준수합니다.

---

## 1. 확정 기술 스택 요약 (`docs/RFP.md` 기준)

- **구성**: 단일 Docker Compose 네트워크 내 4개 컨테이너 (`web`만 호스트 `:80` 공개)
  - `web` (`:80`): TypeScript, React 19, Vite, `pnpm`(Node.js LTS), `@xyflow/react`(React Flow), React Router, TanStack Query, Zustand, `EventSource`(SSE), `ts-rs` 생성 타입, Tailwind CSS, nginx 리버스 프록시(`/api/*` -> `http://engine:8080`)
  - `engine` (`:8080`): Rust (stable), Tokio, Axum(SSE), SQLx(오프라인 `.sqlx` 검증 및 마이그레이션), `pgvector`, `petgraph`(사이클 검출 및 Kahn 위상 정렬 병렬 실행), `serde`/`serde_json`, `ts-rs`, `reqwest`, `tokio::fs`, `tracing`, `thiserror`, `anyhow`
  - `extractor` (`:8000`): Python 3.10+, `uv`, FastAPI, `uvicorn`, Docling (레이아웃·표·그림·MD/JSON 추출, 이미지 빌드 시 모델 사전 다운로드)
  - `db` (`:5432`): `pgvector/pgvector:pg16` (PostgreSQL 16 + `vector` 확장)
- **공유 볼륨 및 설정**:
  - `data` 볼륨 -> `engine:/data`, `extractor:/data` 동일 경로 마운트
  - `pgdata` 볼륨 -> `db:/var/lib/postgresql/data`
  - 비밀키 및 접속 정보 -> 미커밋 루트 `.env` 파일 관리

---

## 2. 디렉토리 구조 및 모듈 경계

> *(Bootstrap 단계에서 오케스트레이터가 확정하여 기입하고, 구조 확장 시 갱신합니다.)*

- 상태: *(오케스트레이터 구체화 대기)*

---

## 3. 데이터·공통 타입(`ts-rs`) 및 `/data` 파일 저장 규약

> *(Bootstrap 단계에서 오케스트레이터가 `docs/RFP.md` 1.4~1.6절을 바탕으로 Rust-TypeScript 공유 타입 구조, 노드 입출력 소켓 표현 방식, `/data` 하위 디렉토리/파일 네이밍 규칙을 확정하여 기입합니다.)*

- 상태: *(오케스트레이터 구체화 대기)*

---

## 4. 서비스 간 API 및 SSE 프로토콜 명세

> *(Bootstrap 단계에서 오케스트레이터가 아래 구간의 정확한 엔드포인트, 요청/응답 JSON 페이로드, 에러 응답 형식, SSE 이벤트 스키마를 확정하여 기입합니다.)*

### 4.1. `web` <-> `engine` (`/api/*` 및 SSE 스트림)
- 상태: *(오케스트레이터 구체화 대기)*

### 4.2. `engine` <-> `extractor` (`http://extractor:8000`)
- 상태: *(오케스트레이터 구체화 대기)*

---

## 5. 데이터베이스 스키마 및 마이그레이션 규약

> *(Bootstrap 단계에서 오케스트레이터가 워크플로, 실행 상태/산출물, RAG 벡터 테이블 스키마 및 SQLx 오프라인 준비 절차를 확정하여 기입합니다.)*

- 상태: *(오케스트레이터 구체화 대기)*

---

## 6. 환경 변수 위치 및 코딩·검증 컨벤션

- **환경 변수 파일**: 루트 `.env` (Git 커밋 금지) 및 `.env.example` (키 목록 템플릿 커밋 유지)
  - 기본 필수 키(`docs/RFP.md` 2.3절): `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `DATABASE_URL`, `EXTRACTOR_URL=http://extractor:8000`, `DATA_DIR=/data`, 외부 LLM·임베딩 API 키
- **공통 코딩 원칙**:
  - 의존성 잠금 파일(`pnpm-lock.yaml`, `Cargo.lock`, `uv.lock`) 필수 유지
  - 프론트엔드 도메인/API 타입은 직접 중복 선언하지 않고 `engine`의 `ts-rs` 산출물을 사용
- **모듈별 표준 검증 명령어**: *(오케스트레이터가 프로젝트 스켈레톤 티켓 구성 시 확정 기입)*
