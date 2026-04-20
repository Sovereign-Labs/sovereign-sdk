FROM node:22-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends git bash curl jq ca-certificates unzip \
    && rm -rf /var/lib/apt/lists/*

# Install Foundry (forge is needed to build @hyperlane-xyz/core)
RUN curl -L https://foundry.paradigm.xyz | bash \
    && /root/.foundry/bin/foundryup
ENV PATH="/root/.foundry/bin:${PATH}"

ARG GITHUB_REF=sovereign-rust-integration

RUN git clone --depth 1 --branch ${GITHUB_REF} https://github.com/Sovereign-Labs/hyperlane-monorepo.git /tmp/hyperlane \
    && cd /tmp/hyperlane \
    && corepack enable \
    && corepack install \
    && pnpm install --frozen-lockfile \
    && cd solidity && forge soldeer install --quiet && cd .. \
    && pnpm --filter @hyperlane-xyz/cli... build \
    && pnpm --filter @hyperlane-xyz/cli bundle \
    && npm install -g ./typescript/cli

FROM node:22-alpine

COPY --from=builder /usr/local/lib/node_modules/@hyperlane-xyz/cli /usr/local/lib/node_modules/@hyperlane-xyz/cli
RUN ln -s /usr/local/lib/node_modules/@hyperlane-xyz/cli/bundle/index.js /usr/local/bin/hyperlane

ARG GITHUB_REF=sovereign-rust-integration
LABEL hyperlane.cli.ref="Sovereign-Labs/hyperlane-monorepo:${GITHUB_REF}"

ENTRYPOINT ["hyperlane"]