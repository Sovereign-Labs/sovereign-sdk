ARG NAMESPACE_BASE_IMAGE_REF=""
# Your image must build FROM NAMESPACE_BASE_IMAGE_REF
FROM ${NAMESPACE_BASE_IMAGE_REF} AS base

ARG RUST_STABLE_VERSION=1.93.0
ARG RUST_NIGHTLY_VERSION=nightly-2026-03-10
ARG NODE_VERSION=20.19.0
ARG MOLD_VERSION=2.40.4
ARG PROTOC_VERSION=23.2

ENV DEBIAN_FRONTEND=noninteractive
ENV RUSTUP_HOME=/home/runner/.rustup \
    CARGO_HOME=/home/runner/.cargo \
    PATH=/home/runner/.cargo/bin:$PATH

# This image is amd64-only.
RUN test "$(dpkg --print-architecture)" = "amd64"

# Install system dependencies
RUN sudo rm -f /etc/apt/sources.list.d/*git-lfs*.list /etc/apt/sources.list.d/*git-lfs*.sources || true \
    && sudo apt-get update \
    && sudo apt-get install -y \
        build-essential \
        pkg-config \
        libssl-dev \
        git \
        curl \
        wget \
        jq \
        clang \
        cmake \
        libclang-dev \
        lld \
        zstd \
        unzip \
        xz-utils \
    && sudo rm -rf /var/lib/apt/lists/*

# Install mold linker from pinned release
RUN curl -fsSL -o /tmp/mold.tar.gz \
         "https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-x86_64-linux.tar.gz" \
    && tar -C /tmp -xzf /tmp/mold.tar.gz \
    && sudo cp "/tmp/mold-${MOLD_VERSION}-x86_64-linux/bin/mold" /usr/local/bin/mold \
    && sudo mkdir -p /usr/local/libexec/mold \
    && sudo cp "/tmp/mold-${MOLD_VERSION}-x86_64-linux/libexec/mold/ld" /usr/local/libexec/mold/ld \
    && rm -rf /tmp/mold*

# Install Rust toolchains
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain ${RUST_STABLE_VERSION} --profile minimal \
    && . /home/runner/.cargo/env \
    && rustup component add clippy rustfmt rust-src llvm-tools-preview --toolchain ${RUST_STABLE_VERSION} \
    && rustup toolchain install ${RUST_NIGHTLY_VERSION} --component rustfmt --component rust-src --profile minimal \
    && rustup target add --toolchain ${RUST_STABLE_VERSION} wasm32-unknown-unknown \
    && rustup target add --toolchain ${RUST_NIGHTLY_VERSION} wasm32-unknown-unknown

# Configure cargo to use mold linker
RUN mkdir -p /home/runner/.cargo \
    && cat > /home/runner/.cargo/config.toml <<'EOF'
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=/usr/local/bin/mold"]
EOF

# Install Node.js from pinned tarball
RUN curl -fsSL -o /tmp/node.tar.xz \
         "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
    && sudo tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
    && rm -f /tmp/node.tar.xz

# Install pnpm and Node.js tools
RUN sudo npm install -g pnpm@9 doctoc

# Install yq (intentionally unpinned)
RUN sudo wget -qO /usr/local/bin/yq \
         "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_amd64" \
    && sudo chmod +x /usr/local/bin/yq

# Install protoc from pinned release
RUN curl -fsSLO \
         "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip" \
    && sudo unzip -o "protoc-${PROTOC_VERSION}-linux-x86_64.zip" -d /usr/local bin/protoc 'include/*' \
    && rm -f "protoc-${PROTOC_VERSION}-linux-x86_64.zip"

# Install RISC0 toolchain
SHELL ["/bin/bash", "-c"]
RUN . /home/runner/.cargo/env \
    && curl -L https://risczero.com/install | bash \
    && export PATH="/home/runner/.risc0/bin:$PATH" \
    && /home/runner/.risc0/bin/rzup install cargo-risczero 2.0.2 \
    && /home/runner/.risc0/bin/rzup install rust 1.88.0 \
    && /home/runner/.risc0/bin/rzup install cpp 2024.1.5 \
    && rm -rf /home/runner/.cargo/registry /home/runner/.cargo/git

# Install SP1 toolchain
RUN . /home/runner/.cargo/env \
    && curl -L https://sp1up.succinct.xyz | bash \
    && /home/runner/.sp1/bin/sp1up --version 6.0.2 \
    && /home/runner/.sp1/bin/cargo-prove prove install-toolchain \
    && rm -rf /home/runner/.cargo/registry /home/runner/.cargo/git
SHELL ["/bin/sh", "-c"]

# Install core Rust development tools
RUN . /home/runner/.cargo/env \
    && cargo install cargo-hack@0.6.37 --locked \
    && cargo install cargo-nextest@0.9.100 --locked \
    && cargo install cargo-llvm-cov@0.6.18 --locked \
    && cargo install cargo-deny@0.18.3 --locked \
    && cargo install cargo-insta@1.43.1 --locked \
    && cargo install zepter@1.78.2 --locked \
    && cargo install bashtestmd@0.5.0 --locked \
    && rm -rf /home/runner/.cargo/registry /home/runner/.cargo/git /tmp/cargo-*

# Important step to make cache mountable
RUN rm -rf /home/runner/.cargo/.global-cache

ENV CARGO_NET_GIT_FETCH_WITH_CLI=true \
    PATH="/home/runner/.risc0/bin:/home/runner/.sp1/bin:/home/runner/.cargo/bin:${PATH}" \
    RISC0_RUST_TOOLCHAIN_VERSION=1.88.0 \
    RISC0_CPP_TOOLCHAIN_VERSION=2024.1.5
