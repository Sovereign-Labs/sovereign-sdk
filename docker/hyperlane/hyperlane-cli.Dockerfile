FROM node:20-alpine

RUN apk add --no-cache git bash curl jq

ARG GITHUB_REF=integration-2025-08-27-rebase
#ARG REPO=

RUN git clone --depth 1 --branch ${GITHUB_REF} https://github.com/Sovereign-Labs/hyperlane-monorepo.git /tmp/hyperlane \
    && cd /tmp/hyperlane \
    && yarn install \
    && yarn build \
    && yarn workspace @hyperlane-xyz/cli bundle \
    && npm install -g ./typescript/cli \
    && rm -rf /tmp/hyperlane/node_modules \
    && rm -rf /tmp/hyperlane/solidity \
    && rm -rf /tmp/hyperlane/starknet \
    && rm -rf /tmp/hyperlane/rust \
    && yarn cache clean \
    && npm cache clean --force \
    && apk del git bash curl jq

LABEL hyperlane.cli.ref="Sovereign-Labs/hyperlane-monorepo:${GITHUB_REF}"

ENTRYPOINT ["hyperlane"]