FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl python3 python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 appuser \
    && mkdir -p /data \
    && chown -R appuser:appuser /data

# The game art converter reads Unity bundles with UnityPy; its packages are pinned.
COPY scripts/requirements-game-art.txt /tmp/requirements-game-art.txt
RUN python3 -m venv /opt/game-art \
    && /opt/game-art/bin/pip install --no-cache-dir -r /tmp/requirements-game-art.txt \
    && rm /tmp/requirements-game-art.txt

WORKDIR /app
COPY --from=builder /app/target/release/umamoe-resources /usr/local/bin/umamoe-resources
COPY scripts/game_art_convert.py /app/scripts/game_art_convert.py
COPY scripts/game_art_model.py /app/scripts/game_art_model.py
COPY scripts/game_art_face.py /app/scripts/game_art_face.py
COPY scripts/game_art_shader.py /app/scripts/game_art_shader.py
COPY scripts/game_art_uma.py /app/scripts/game_art_uma.py

USER appuser
EXPOSE 3000 3204
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=5 \
    CMD curl -fsS "http://127.0.0.1:3000/healthz" >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/umamoe-resources"]
CMD ["--bind", "0.0.0.0:3000", "--master", "/data/master.mdb", "--data-dir", "/data/generated-data"]
