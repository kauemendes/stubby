# syntax=docker/dockerfile:1.7
FROM rust:1.96-bookworm AS builder
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p stubby-dummy-frontend && \
    cp /src/target/release/stubby-dummy-frontend /tmp/app

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /tmp/app /usr/local/bin/app
USER nonroot
# The dummy listens on STUBBY_PORT (injected by the webhook). 8080 is the
# default non-privileged port; binding 80 as non-root needs NET_BIND_SERVICE.
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/app"]
