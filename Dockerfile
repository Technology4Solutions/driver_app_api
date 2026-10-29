FROM rust:1.90-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --locked --release -p device-api

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/device-api /usr/local/bin/device-api
USER 65532:65532
ENV API_BIND=0.0.0.0:8080 ADMIN_BIND=0.0.0.0:9090
EXPOSE 8080 9090
ENTRYPOINT ["device-api"]
