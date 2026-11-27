# syntax=docker/dockerfile:1
#
# Build accelrun without installing Rust on the host.
#
# The image is a BUILD VEHICLE ONLY: it exists to compile the binary and
# carry it to the host via `docker cp`. accelrun is a host-side tool — it
# probes the host's accelerators and shells out to the host `docker` CLI —
# so it runs on the host, never inside this image.
#
# One command does the whole thing:
#   ./build.sh          # build + extract + verify ./accelrun
#
# Or by hand:
#   docker build -t accelrun:latest .
#   docker create --name ar accelrun:latest
#   docker cp ar:/accelrun ./accelrun && docker rm ar
#   chmod +x accelrun

FROM rust:1.99-bookworm AS builder
WORKDIR /src
# .dockerignore keeps out target/ and the host-specific .cargo/config.toml
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

# Carrier stage: just the finished binary at a stable path for `docker cp`.
# scratch (no userspace at all) — the binary is glibc-linked and runs on the
# host, so the image never needs a libc of its own.
FROM scratch
COPY --from=builder /src/target/release/accelrun /accelrun
