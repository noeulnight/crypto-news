# syntax=docker/dockerfile:1
FROM node:26-bookworm-slim AS web-build
WORKDIR /build/web
RUN npm install --global pnpm@10.32.1
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm check && pnpm build

FROM rust:1-trixie AS rust-build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN cargo test --release --locked && cargo build --release --locked

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 app \
    && useradd --uid 10001 --gid app --no-create-home app \
    && mkdir -p /app/data \
    && chown app:app /app/data
WORKDIR /app
COPY --from=rust-build /build/target/release/crypto-news /usr/local/bin/crypto-news
COPY --from=web-build /build/web/build ./web/build
ENV HOST=0.0.0.0 PORT=8787 DB_PATH=/app/data/crypto-news.db WEB_DIR=/app/web/build
USER app
EXPOSE 8787
VOLUME ["/app/data"]
ENTRYPOINT ["crypto-news"]
