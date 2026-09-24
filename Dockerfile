# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS frontend
WORKDIR /src/frontend
RUN corepack enable && corepack prepare pnpm@10.33.3 --activate
COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY frontend/ ./
RUN pnpm build

FROM rust:1.97.1-bookworm AS backend
WORKDIR /src/backend
ENV SQLX_OFFLINE=true \
    CARGO_TERM_COLOR=never
COPY backend/ ./
RUN cargo build --release -p ferrobox-server \
    && cp target/release/ferrobox-server /ferrobox-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=backend /ferrobox-server /usr/local/bin/ferrobox-server
COPY --from=frontend /src/frontend/dist /app/www
ENV FRONTEND_DIR=/app/www \
    BIND_ADDRESS=0.0.0.0:3000
EXPOSE 3000
USER nobody
HEALTHCHECK --interval=10s --timeout=5s --retries=5 --start-period=20s \
    CMD curl -fsS http://127.0.0.1:3000/health >/dev/null
CMD ["ferrobox-server"]
