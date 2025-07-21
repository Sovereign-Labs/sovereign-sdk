ARG NAMESPACE_BASE_IMAGE_REF=""

# Your image must build FROM NAMESPACE_BASE_IMAGE_REF
FROM ${NAMESPACE_BASE_IMAGE_REF} AS base
# Multi-stage build for efficiency
#FROM ubuntu:24.04 AS base

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    curl \
    wget \
    jq \
    protobuf-compiler \
    clang \
    cmake \
    libclang-dev \
    lld \
    zstd \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0 --profile minimal \
    && rustup component add clippy --toolchain 1.88.0 \
    && rustup toolchain install nightly --component rustfmt --profile minimal \
    && rustup target add wasm32-unknown-unknown

# Install Node.js 20.x
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm
RUN npm install -g pnpm@9

# Install yq for YAML processing
RUN wget https://github.com/mikefarah/yq/releases/latest/download/yq_linux_amd64 -O /usr/bin/yq \
    && chmod +x /usr/bin/yq

# Install Node.js tools
RUN npm install -g doctoc

# Install protoc v23.2
RUN PROTOC_VERSION="23.2" \
    && PROTOC_ZIP="protoc-${PROTOC_VERSION}-linux-x86_64.zip" \
    && curl -OL "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/${PROTOC_ZIP}" \
    && unzip -o "${PROTOC_ZIP}" -d /usr/local bin/protoc \
    && unzip -o "${PROTOC_ZIP}" -d /usr/local 'include/*' \
    && rm -f "${PROTOC_ZIP}"

# Install RISC0 toolchain
SHELL ["/bin/bash", "-c"]
RUN curl -L https://risczero.com/install | bash \
    && export PATH="/root/.risc0/bin:$PATH" \
    && rzup install cargo-risczero 2.0.2 \
    && rzup install rust 1.88.0 \
    && rzup install cpp 2024.1.5 \
    && rm -rf $CARGO_HOME/registry \
    && rm -rf $CARGO_HOME/git
SHELL ["/bin/sh", "-c"]

# Install SP1 toolchain
RUN curl -L https://raw.githubusercontent.com/succinctlabs/sp1/main/sp1up/install | bash \
    && ~/.sp1/bin/sp1up --version 5.0.8 \
    && ~/.sp1/bin/cargo-prove prove install-toolchain \
    && rm -rf $CARGO_HOME/registry \
    && rm -rf $CARGO_HOME/git

## Move it to bottom for now for faster iteration

# Install core Rust development tools
RUN cargo install cargo-hack --locked \
    && cargo install cargo-nextest@0.9.96 --locked \
    && cargo install cargo-llvm-cov --locked \
    && cargo install cargo-deny@0.16.1 --locked \
    && cargo install cargo-insta@1.42.1 --locked \
    && cargo install zepter@1.5.0 --locked \
    && cargo install bashtestmd@0.5.0 --locked \
    # Clear cargo cache to reduce image size
    && rm -rf $CARGO_HOME/registry \
    && rm -rf $CARGO_HOME/git \
    && rm -rf /tmp/cargo-*

# Install additional tools on nightly
RUN cargo +nightly install cargo-udeps --locked \
    && rm -rf $CARGO_HOME/registry \
    && rm -rf $CARGO_HOME/git

# Set environment variables
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true \
    PATH="/root/.risc0/bin:/root/.sp1/bin:${PATH}" \
    RISC0_RUST_TOOLCHAIN_VERSION=1.88.0 \
    RISC0_CPP_TOOLCHAIN_VERSION=2024.1.5

# Set working directory
WORKDIR /workspace

CMD ["/bin/bash"]