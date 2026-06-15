# syntax=docker/dockerfile:1

FROM rust:1.93-bookworm AS builder
ARG HIVE_CHAIN_ID=3503995874084926

ENV DEBIAN_FRONTEND=noninteractive \
    SKIP_GUEST_BUILD=1 \
    CARGO_NET_GIT_FETCH_WITH_CLI=true

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    clang \
    cmake \
    git \
    libclang-dev \
    libssl-dev \
    lld \
    pkg-config \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

RUN sed -Ei "s/^(CHAIN_ID[[:space:]]*=[[:space:]]*).*/\\1${HIVE_CHAIN_ID}/" constants.toml \
    && sed -Ei "s/^(CHAIN_ID[[:space:]]*=[[:space:]]*).*/\\1${HIVE_CHAIN_ID}/" constants.testing.toml \
    && sed -Ei "s/^(EVM_GAS_METERING_MODE[[:space:]]*=[[:space:]]*).*/\\1\"EVM\"/" constants.toml \
    && sed -Ei "s/^(EVM_GAS_METERING_MODE[[:space:]]*=[[:space:]]*).*/\\1\"EVM\"/" constants.testing.toml

RUN cargo build -p sov-demo-rollup --bin sov-demo-rollup --bin sov-hive-genesis-adapter

FROM debian:bookworm-slim AS runtime
ARG HIVE_CHAIN_ID=3503995874084926

ENV DEBIAN_FRONTEND=noninteractive \
    SOV_HIVE_COMPILED_CHAIN_ID=${HIVE_CHAIN_ID} \
    NO_COLOR=1 \
    CLICOLOR=0 \
    CLICOLOR_FORCE=0

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    jq \
    python3 \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /opt/sov/bin /opt/sov/hive /hive-bin /hive-data

COPY --from=builder /src/target/debug/sov-demo-rollup /opt/sov/bin/sov-demo-rollup
COPY --from=builder /src/target/debug/sov-hive-genesis-adapter /opt/sov/bin/sov-hive-genesis-adapter
COPY examples/demo-rollup/hive/entrypoint.sh /opt/sov/hive/entrypoint.sh
COPY examples/demo-rollup/hive/hive_services.py /opt/sov/hive/hive_services.py
COPY examples/demo-rollup/hive/wait_for_rpc.py /opt/sov/hive/wait_for_rpc.py
COPY examples/demo-rollup/hive/enode.sh /opt/sov/hive/enode.sh
COPY examples/demo-rollup/hive/mock_rollup_config.toml /opt/sov/hive/mock_rollup_config.toml
COPY examples/test-data/genesis/demo/mock /opt/sov/hive/genesis-template

RUN chmod +x \
    /opt/sov/hive/entrypoint.sh \
    /opt/sov/hive/hive_services.py \
    /opt/sov/hive/wait_for_rpc.py \
    /opt/sov/hive/enode.sh \
    && ln -sf /opt/sov/hive/enode.sh /hive-bin/enode.sh \
    && /opt/sov/bin/sov-demo-rollup --version > /version.txt

EXPOSE 8545 8551

ENTRYPOINT ["/opt/sov/hive/entrypoint.sh"]
