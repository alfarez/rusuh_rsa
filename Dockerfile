FROM rust:alpine AS builder
WORKDIR /app

RUN apk add --no-cache musl-dev pkgconfig openssl-dev ca-certificates

ENV CARGO_BUILD_JOBS=1
ENV RUSTFLAGS="-C codegen-units=1"

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release --locked

COPY src ./src
RUN cargo build --release --locked \
    && strip target/release/rsa

FROM alpine:3.21
WORKDIR /app

RUN apk add --no-cache ca-certificates tzdata libssl3 libgcc libstdc++

COPY --from=builder /app/target/release/rsa /usr/local/bin/rsa

ENV TZ=Asia/Singapore
ENTRYPOINT ["/usr/local/bin/rsa"]