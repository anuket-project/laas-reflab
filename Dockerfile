FROM rustlang/rust:nightly AS chef

COPY rust-toolchain.toml rust-toolchain.toml
RUN rustup show 

RUN apt-get update && apt-get install -y mold clang lld && apt-get clean && rm -rf /var/lib/apt/lists/*

ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld"
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang

RUN cargo install cargo-chef

FROM chef AS planner

WORKDIR /app/

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src src
COPY crates crates

RUN cargo +nightly chef prepare --recipe-path recipe.json

FROM chef AS builder

WORKDIR /app/

COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --release --recipe-path recipe.json

RUN apt-get update && apt-get install -y python3-dev python3.11-dev && rm -rf /var/lib/apt/lists/*

COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY src src
COPY crates crates
COPY migrations migrations

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y python3.11-dev postgresql-client ipmitool iputils-ping && rm -rf /var/lib/apt/lists/* 

COPY --from=builder /app/target/release/laas-reflab /usr/local/bin
COPY --from=builder /app/migrations /app/migrations

ENTRYPOINT ["/usr/local/bin/laas-reflab", "--server"]

