FROM node:20-slim AS dashboard-build
WORKDIR /app/dashboard-v2
COPY dashboard-v2/package.json dashboard-v2/package-lock.json ./
RUN npm ci
COPY dashboard-v2/ ./
RUN npm run build

FROM rust:1-slim AS build
WORKDIR /app
COPY . .
COPY --from=dashboard-build /app/dashboard-v2/dist ./dashboard-v2/dist
RUN cargo build --release -p nats-link --bin policy-engine-worker --bin evidence-worker \
    && cargo build --release -p governor-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin governor
COPY --from=build /app/target/release/policy-engine-worker /usr/local/bin/
COPY --from=build /app/target/release/evidence-worker /usr/local/bin/
COPY --from=build /app/target/release/governor-server /usr/local/bin/
COPY --from=build /app/dashboard-v2/dist /app/dashboard-v2/dist
COPY --from=build /app/seeds /seeds
RUN chown -R governor:governor /app /seeds
USER governor
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["sh", "-c", "wget -qO- http://127.0.0.1:${PORT:-8080}/health | grep -q ok"]
CMD ["policy-engine-worker"]
