FROM rust:slim-bookworm

COPY rust-toolchain.toml rust-toolchain.toml

RUN rustup show

ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld"
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang

RUN apt-get update && apt-get install -y \
  mold clang lld postgresql-client python3-dev python3.11-dev pkg-config libssl-dev \
  && apt-get clean && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-make cargo-nextest sqlx-cli cargo-audit cargo-limit cargo-llvm-cov

WORKDIR /app
