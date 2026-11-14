FROM rust:1.96-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release --bin backend --bin healthcheck

FROM debian:bookworm-slim AS runtime

WORKDIR /app
COPY --from=builder /app/target/release/backend /app/server
COPY --from=builder /app/target/release/healthcheck /app/healthcheck

EXPOSE 4000

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && update-ca-certificates \
    && rm -rf /var/lib/apt/lists/*

CMD ["./server"]
