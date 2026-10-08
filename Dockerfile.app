FROM rust:1.88-bookworm AS builder

WORKDIR /build/services/app
COPY services/app/Cargo.toml services/app/Cargo.lock ./
COPY services/app/src ./src
COPY services/app/prompts ./prompts
COPY services/app/migrations ./migrations
COPY samples /build/samples
RUN cargo build --locked --release
COPY scripts/rust-notices.py /build/scripts/rust-notices.py
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 \
    && python3 /build/scripts/rust-notices.py /build/RUST_THIRD_PARTY_NOTICES.txt

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 10001 interdeck \
    && mkdir -p /data/assets \
    && chown interdeck:interdeck /data/assets
COPY LICENSE THIRD_PARTY_NOTICES.md /usr/share/licenses/interdeck/
COPY --from=builder /build/RUST_THIRD_PARTY_NOTICES.txt /usr/share/licenses/interdeck/
COPY --from=builder /build/services/app/target/release/interdeck-app /usr/local/bin/interdeck-app

USER interdeck
ENV APP_ENV=production
ENV ASSET_LOCAL_DIR=/data/assets
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s CMD curl --fail --silent http://127.0.0.1:8080/health/ready || exit 1
EXPOSE 8080
CMD ["interdeck-app"]
