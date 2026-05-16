FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY master_fetch.rs ./

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 appuser \
    && mkdir -p /data \
    && chown -R appuser:appuser /data

WORKDIR /app
COPY --from=builder /app/target/release/umamoe-resources /usr/local/bin/umamoe-resources

USER appuser
EXPOSE 3000 3204
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=5 \
    CMD curl -fsS "http://127.0.0.1:3000/healthz" >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/umamoe-resources"]
CMD ["--bind", "0.0.0.0:3000", "--master", "/data/master.mdb", "--data-dir", "/data/generated-data"]
