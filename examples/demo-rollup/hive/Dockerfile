# syntax=docker/dockerfile:1

FROM rust:1.93-bookworm AS builder
ARG HIVE_CHAIN_ID=3503995874084926
ARG PROTOC_VERSION=23.2
ARG MOLD_VERSION=2.40.4

ENV DEBIAN_FRONTEND=noninteractive \
    SKIP_GUEST_BUILD=1 \
    CARGO_NET_GIT_FETCH_WITH_CLI=true

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    clang \
    cmake \
    curl \
    git \
    libclang-dev \
    libssl-dev \
    pkg-config \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install protoc from pinned release (includes well-known .proto files)
RUN ARCH=$(dpkg --print-architecture) \
    && case "$ARCH" in \
         amd64) PROTOC_ARCH="linux-x86_64" ;; \
         arm64) PROTOC_ARCH="linux-aarch_64" ;; \
         *) echo "Unsupported arch: $ARCH" && exit 1 ;; \
       esac \
    && curl -fsSLO "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-${PROTOC_ARCH}.zip" \
    && unzip -o "protoc-${PROTOC_VERSION}-${PROTOC_ARCH}.zip" -d /usr/local bin/protoc 'include/*' \
    && rm -f "protoc-${PROTOC_VERSION}-${PROTOC_ARCH}.zip"

# Install mold linker for faster linking
RUN ARCH=$(dpkg --print-architecture) \
    && case "$ARCH" in \
         amd64) MOLD_ARCH="x86_64" ;; \
         arm64) MOLD_ARCH="aarch64" ;; \
         *) echo "Unsupported arch: $ARCH" && exit 1 ;; \
       esac \
    && curl -fsSL -o /tmp/mold.tar.gz \
         "https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-${MOLD_ARCH}-linux.tar.gz" \
    && tar -C /tmp -xzf /tmp/mold.tar.gz \
    && cp "/tmp/mold-${MOLD_VERSION}-${MOLD_ARCH}-linux/bin/mold" /usr/local/bin/mold \
    && mkdir -p /usr/local/libexec/mold \
    && cp "/tmp/mold-${MOLD_VERSION}-${MOLD_ARCH}-linux/libexec/mold/ld" /usr/local/libexec/mold/ld \
    && rm -rf /tmp/mold*

# Configure cargo to use mold
RUN mkdir -p /root/.cargo \
    && printf '[target.x86_64-unknown-linux-gnu]\nlinker = "clang"\nrustflags = ["-C", "link-arg=-fuse-ld=/usr/local/bin/mold"]\n\n[target.aarch64-unknown-linux-gnu]\nlinker = "clang"\nrustflags = ["-C", "link-arg=-fuse-ld=/usr/local/bin/mold"]\n' \
       > /root/.cargo/config.toml

WORKDIR /src
COPY . .

RUN sed -Ei "s/^(CHAIN_ID[[:space:]]*=[[:space:]]*).*/\\1${HIVE_CHAIN_ID}/" constants.toml \
    && sed -Ei "s/^(CHAIN_ID[[:space:]]*=[[:space:]]*).*/\\1${HIVE_CHAIN_ID}/" constants.testing.toml \
    && sed -Ei "s/^(EVM_GAS_METERING_MODE[[:space:]]*=[[:space:]]*).*/\\1\"EVM\"/" constants.toml \
    && sed -Ei "s/^(EVM_GAS_METERING_MODE[[:space:]]*=[[:space:]]*).*/\\1\"EVM\"/" constants.testing.toml

# Cache mounts persist cargo registry and build artifacts across Docker builds,
# so incremental compilation works even when the COPY layer is invalidated.
# Binaries are copied to /tmp/out/ since cache mounts aren't available in other stages.
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build -j 4 -p sov-demo-rollup --bin sov-demo-rollup --bin sov-hive-genesis-adapter \
    && mkdir -p /tmp/out \
    && cp target/debug/sov-demo-rollup target/debug/sov-hive-genesis-adapter /tmp/out/

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
    libssl3 \
    python3 \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /opt/sov/bin /opt/sov/hive /hive-bin /hive-data

COPY --from=builder /tmp/out/sov-demo-rollup /opt/sov/bin/sov-demo-rollup
COPY --from=builder /tmp/out/sov-hive-genesis-adapter /opt/sov/bin/sov-hive-genesis-adapter
COPY examples/demo-rollup/hive/entrypoint.sh /opt/sov/hive/entrypoint.sh
COPY examples/demo-rollup/hive/hive_services.py /opt/sov/hive/hive_services.py
COPY examples/demo-rollup/hive/wait_for_rpc.py /opt/sov/hive/wait_for_rpc.py
COPY examples/demo-rollup/hive/enode.sh /opt/sov/hive/enode.sh
COPY examples/demo-rollup/hive/mock_nomt_rollup_config.toml /opt/sov/hive/mock_nomt_rollup_config.toml
COPY examples/test-data/genesis/demo/mock /opt/sov/hive/genesis-template


RUN chmod +x \
    /opt/sov/hive/entrypoint.sh \
    /opt/sov/hive/hive_services.py \
    /opt/sov/hive/wait_for_rpc.py \
    /opt/sov/hive/enode.sh \
    && ln -sf /opt/sov/hive/enode.sh /hive-bin/enode.sh \
    && echo '{"payers": []}' > /opt/sov/hive/genesis-template/paymaster.json \
    && /opt/sov/bin/sov-demo-rollup --version > /version.txt

EXPOSE 8545 8551

ENTRYPOINT ["/opt/sov/hive/entrypoint.sh"]
