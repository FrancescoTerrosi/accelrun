# syntax=docker/dockerfile:1
#
# Build accelrun without installing Rust on the host:
#   docker build -t accelrun:latest .
#   docker run --rm accelrun:latest probe
# Extract the static-ish binary to your host:
#   docker create --name ar accelrun:latest && docker cp ar:/usr/local/bin/accelrun . && docker rm ar

FROM rust:1.99-bookworm AS builder
WORKDIR /src
# .dockerignore keeps out target/ and the host-specific .cargo/config.toml
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update -qq && apt-get install -y -qq ca-certificates docker.io 2>/dev/null || apt-get install -y -qq ca-certificates
COPY --from=builder /src/target/release/accelrun /usr/local/bin/accelrun
ENTRYPOINT ["accelrun"]
CMD ["probe"]
