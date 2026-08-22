# Multi-stage: build workers once, ship minimal runtime image.
# Same image runs either worker via command override.

FROM rust:1-slim AS build
WORKDIR /app
COPY . .
RUN cargo build --release -p nats-link --bin policy-engine-worker --bin evidence-worker

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/policy-engine-worker /usr/local/bin/
COPY --from=build /app/target/release/evidence-worker /usr/local/bin/
CMD ["policy-engine-worker"]