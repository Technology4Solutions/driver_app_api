FROM rust:1.90-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo test --locked --workspace && cargo build --locked --release -p device-api

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl python3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/device-api /usr/local/bin/device-api
COPY ci_cd/vault-entrypoint.py /usr/local/bin/vault-entrypoint.py
USER 65532:65532
ENV API_BIND=0.0.0.0:8080 ADMIN_BIND=0.0.0.0:9090
EXPOSE 8080 9090
HEALTHCHECK --interval=10s --timeout=3s --start-period=60s --retries=5 CMD curl -fsS http://localhost:9090/health/live >/dev/null || exit 1
ENTRYPOINT ["python3", "/usr/local/bin/vault-entrypoint.py", "api"]
