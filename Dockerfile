FROM rust:alpine AS builder
WORKDIR /app

RUN apk add --no-cache \
    musl-dev pkgconf openssl-dev openssl-libs-static \
    && rm -rf /var/cache/apk/*

ENV CARGO_BUILD_JOBS=1
ENV OPENSSL_STATIC=1

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN nice -n 19 cargo build --locked

COPY src ./src
RUN touch src/main.rs \
    && nice -n 19 cargo build --locked

FROM alpine:latest
WORKDIR /app
RUN apk add --no-cache ca-certificates tzdata
COPY --from=builder /app/target/debug/rsa /usr/local/bin/rsa
ENV TZ=Asia/Singapore
ENTRYPOINT ["/usr/local/bin/rsa"]