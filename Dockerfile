FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY master_fetch.rs ./

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 appuser \
    && mkdir -p /data \
    && chown -R appuser:appuser /data

WORKDIR /app
COPY --from=builder /app/target/release/umamoe-resources /usr/local/bin/umamoe-resources

USER appuser
EXPOSE 3000
VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/umamoe-resources"]
CMD ["--bind", "0.0.0.0:3000", "--master", "/data/master.mdb", "--data-dir", "/data/generated-data"]
