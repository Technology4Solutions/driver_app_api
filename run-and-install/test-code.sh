#!/usr/bin/env bash
# Run ordinary automated checks in Docker; no remote database access is required.
source "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
require_docker
docker run --rm \
  -e RUSTUP_TOOLCHAIN=1.90.0 \
  -e CARGO_TARGET_DIR=/build \
  --mount "type=bind,src=$PROJECT_ROOT,dst=/app,readonly" \
  --mount type=volume,src=device-api-test-registry,dst=/usr/local/cargo/registry \
  --mount type=volume,src=device-api-test-target,dst=/build \
  -w /app rust:1.90-bookworm bash -euc '
    rustup component add rustfmt clippy --toolchain 1.90.0
    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace
  '
