# syntax=docker/dockerfile:1.7
FROM rust:1.96-bookworm AS builder
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p stubby-webhook && \
    cp /src/target/release/stubby-webhook /tmp/stubby-webhook

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /tmp/stubby-webhook /usr/local/bin/stubby-webhook
USER nonroot
EXPOSE 8443
ENTRYPOINT ["/usr/local/bin/stubby-webhook"]
