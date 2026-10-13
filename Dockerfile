FROM rust:1.90-slim AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main(){}" > src/main.rs && cargo build --release && rm -rf src target/release/deps/imgserve*
COPY . .
RUN cargo build --release

FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=builder /app/target/release/imgserve /app/imgserve
ENV DATA_DIR=/data
VOLUME ["/data"]
EXPOSE 8000
ENTRYPOINT ["/app/imgserve"]
