# Stage 1: Builder Stage
FROM rust:1.76-alpine AS builder

RUN apk add --no-cache git clang lld musl-dev nodejs npm openssl-dev pkgconfig g++ protobuf-dev

WORKDIR /usr/src/refact-lsp

# Copy only neccessary files
COPY src src
COPY build.rs build.rs
COPY Cargo.toml Cargo.toml

ENV CARGO_INCREMENTAL=0
ENV CARGO_NET_RETRY=10
ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld -C target-feature=-crt-static"
RUN cargo install --path .

# Stage 2: Final Stage
FROM alpine:3.19.1 AS final

RUN apk add --no-cache libstdc++

RUN adduser -D lspuser
USER lspuser

COPY --from=builder /usr/local/cargo/bin/refact-lsp /usr/local/bin/refact-lsp

ENTRYPOINT ["/usr/local/bin/refact-lsp"]
