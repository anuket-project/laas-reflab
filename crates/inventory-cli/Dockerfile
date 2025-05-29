FROM rust:1.87 AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y \
  clang mold && \
  rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release --package inventory_cli --bin inventory-cli

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
  libssl-dev ca-certificates  && \
  rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/inventory-cli /usr/local/bin/inventory-cli

ENTRYPOINT ["/usr/local/bin/inventory-cli"]

