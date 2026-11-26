# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS builder
WORKDIR /app
ENV SQLX_OFFLINE=true

# Dependency cache layer: build a dummy crate from the manifests only.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src target/release/deps/dochestra* target/release/dochestra* target/release/.fingerprint/dochestra*

COPY src ./src
COPY migrations ./migrations
COPY .sqlx ./.sqlx
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 dochestra \
    && useradd --system --uid 10001 --gid 10001 --no-create-home dochestra \
    && mkdir -p /data \
    && chown 10001:10001 /data
COPY --from=builder /app/target/release/dochestra-engine /usr/local/bin/dochestra-engine
# Fonts for the slide node (SIL OFL; see assets/fonts/OFL.txt).
COPY assets/fonts /usr/local/share/dochestra/fonts
USER dochestra
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/dochestra-engine"]
