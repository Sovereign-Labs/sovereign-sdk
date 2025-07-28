ARG NAMESPACE_BASE_IMAGE_REF=""
# Your image must build FROM NAMESPACE_BASE_IMAGE_REF
FROM ${NAMESPACE_BASE_IMAGE_REF} AS base

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies
RUN sudo apt-get update && sudo apt-get install -y \
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
    && sudo rm -rf /var/lib/apt/lists/*

# Build and install mold linker from source
RUN git clone https://github.com/rui314/mold.git /tmp/mold \
    && cd /tmp/mold \
    && git checkout $(git describe --tags --abbrev=0) \
    && cmake -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_CXX_COMPILER=clang++ \
    && cmake --build build -j$(nproc) \
    && sudo cmake --build build --target install \
    && cd / \
    && rm -rf /tmp/mold

# Install Rust as user
# The base image runs as runner user with home at /home/runner
ENV RUSTUP_HOME=/home/runner/.rustup \
    CARGO_HOME=/home/runner/.cargo \
    PATH=/home/runner/.cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0 --profile minimal \
    && . /home/runner/.cargo/env \
    && rustup component add clippy rustfmt rust-src llvm-tools-preview --toolchain 1.88.0 \
    && rustup toolchain install nightly --component rustfmt --component rust-src --profile minimal \
    && rustup target add wasm32-unknown-unknown

# Configure cargo to use mold linker
RUN mkdir -p /home/runner/.cargo \
    && echo '[target.x86_64-unknown-linux-gnu]' >> /home/runner/.cargo/config.toml \
    && echo 'rustflags = ["-C", "link-arg=-fuse-ld=/usr/local/bin/mold"]' >> /home/runner/.cargo/config.toml

# Install Node.js 20.x
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash - \
    && sudo apt-get install -y nodejs \
    && sudo rm -rf /var/lib/apt/lists/*

# Install pnpm and Node.js tools
RUN sudo npm install -g pnpm@9 doctoc

# Install yq for YAML processing
RUN sudo wget https://github.com/mikefarah/yq/releases/latest/download/yq_linux_amd64 -O /usr/bin/yq \
    && sudo chmod +x /usr/bin/yq

# Install protoc v23.2
RUN PROTOC_VERSION="23.2" \
    && PROTOC_ZIP="protoc-${PROTOC_VERSION}-linux-x86_64.zip" \
    && curl -OL "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/${PROTOC_ZIP}" \
    && sudo unzip -o "${PROTOC_ZIP}" -d /usr/local bin/protoc \
    && sudo unzip -o "${PROTOC_ZIP}" -d /usr/local 'include/*' \
    && rm -f "${PROTOC_ZIP}"

# Install RISC0 toolchain (AMD64 only - RISC0 doesn't support ARM64)
SHELL ["/bin/bash", "-c"]
RUN . /home/runner/.cargo/env \
        && curl -L https://risczero.com/install | bash \
        && export PATH="/home/runner/.risc0/bin:$PATH" \
        && /home/runner/.risc0/bin/rzup install cargo-risczero 2.0.2 \
        && /home/runner/.risc0/bin/rzup install rust 1.88.0 \
        && /home/runner/.risc0/bin/rzup install cpp 2024.1.5 \
        && rm -rf /home/runner/.cargo/registry \
        && rm -rf /home/runner/.cargo/git;
SHELL ["/bin/sh", "-c"]

# Install SP1 toolchain (AMD64 only - SP1 doesn't support ARM64)
RUN . /home/runner/.cargo/env \
        && curl -L https://raw.githubusercontent.com/succinctlabs/sp1/main/sp1up/install | bash \
        && /home/runner/.sp1/bin/sp1up --version 5.0.8 \
        && /home/runner/.sp1/bin/cargo-prove prove install-toolchain \
        && rm -rf /home/runner/.cargo/registry \
        && rm -rf /home/runner/.cargo/git

## Move it to bottom for now for faster iteration

# Install core Rust development tools
RUN . /home/runner/.cargo/env \
    && cargo install cargo-hack@0.6.37 --locked \
    && cargo install cargo-nextest@0.9.100 --locked \
    && cargo install cargo-llvm-cov@0.6.18 --locked \
    && cargo install cargo-deny@0.18.3 --locked \
    && cargo install cargo-insta@1.43.1 --locked \
    && cargo install zepter@1.78.2 --locked \
    && cargo install bashtestmd@0.5.0 --locked \
    # Clear cargo cache to reduce image size
    && rm -rf /home/runner/.cargo/registry \
    && rm -rf /home/runner/.cargo/git \
    && rm -rf /tmp/cargo-*

# Important step to make cache mountable
RUN rm -rf /home/runner/.cargo/.global-cache

ENV CARGO_NET_GIT_FETCH_WITH_CLI=true \
    PATH="/home/runner/.risc0/bin:/home/runner/.sp1/bin:/home/runner/.cargo/bin:${PATH}" \
    RISC0_RUST_TOOLCHAIN_VERSION=1.88.0 \
    RISC0_CPP_TOOLCHAIN_VERSION=2024.1.5
