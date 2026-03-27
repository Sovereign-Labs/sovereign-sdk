FROM node:22-alpine AS builder

RUN apk add --no-cache git bash curl jq

ARG GITHUB_REF=integration

RUN git clone --depth 1 --branch ${GITHUB_REF} https://github.com/Sovereign-Labs/hyperlane-monorepo.git /tmp/hyperlane \
    && cd /tmp/hyperlane \
    && yarn install \
    && yarn build \
    && yarn workspace @hyperlane-xyz/cli bundle \
    && npm install -g ./typescript/cli

FROM node:22-alpine

COPY --from=builder /usr/local/lib/node_modules/@hyperlane-xyz/cli /usr/local/lib/node_modules/@hyperlane-xyz/cli
COPY --from=builder /usr/local/bin/hyperlane /usr/local/bin/hyperlane

ARG GITHUB_REF=integration
LABEL hyperlane.cli.ref="Sovereign-Labs/hyperlane-monorepo:${GITHUB_REF}"

ENTRYPOINT ["hyperlane"]