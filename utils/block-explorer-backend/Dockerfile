FROM rust:1-bookworm AS chef 

WORKDIR /app

COPY rust-toolchain.toml .
# Triggers the install of the Rust toolchain. We do this first, so it's always
# cached by Docker.
# See <https://github.com/rust-lang/rustup/issues/1397>.
RUN rustup show

# Installs `cargo-chef`.
RUN curl -o cargo-chef.tar.gz -L https://github.com/LukeMathWalker/cargo-chef/releases/latest/download/cargo-chef-x86_64-unknown-linux-gnu.tar.gz && \
    tar -xzf cargo-chef.tar.gz && \
    mv cargo-chef /usr/local/bin/ && \
    rm cargo-chef.tar.gz

FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

# We support both `release` and `dev`, just by setting `CARGO_PROFILE`.
ARG CARGO_PROFILE=dev

COPY --from=planner /app/recipe.json recipe.json

RUN	apt-get update && apt-get install -y libpq-dev clang

# Use cargo-chef to compile dependencies only - this will be cached by Docker.
RUN cargo chef cook --profile $CARGO_PROFILE --recipe-path recipe.json --bin block-explorer-backend
# ... and then build the rest of the application.
COPY . .
RUN cargo build --profile $CARGO_PROFILE --bin block-explorer-backend

# Instead of calculating where the binary is located based on $CARGO_PROFILE, we
# simply try to copy both `debug` and `release` binaries. One of these will
# fail, the other will succeed, depending on the value of $CARGO_PROFILE.
RUN cp target/release/block-explorer-backend /usr/local/bin | true && \
	cp target/debug/block-explorer-backend /usr/local/bin | true && \
	strip --strip-all /usr/local/bin/block-explorer-backend

FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update && \
	apt-get install -y libpq-dev libssl-dev && \
	apt-get clean

COPY --from=builder /usr/local/bin/block-explorer-backend /usr/local/bin

ENV RUST_LOG="trace"

EXPOSE 3010

ENTRYPOINT [ block-explorer-backend ]
