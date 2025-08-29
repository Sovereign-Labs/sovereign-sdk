FROM node:20-alpine

RUN apt-get install -y --no-cache git bash curl jq

# ARG for branch/tag/commit
ARG GITHUB_REF=integration
#ARG REPO=

# Clone and install from GitHub
RUN git clone --depth 1 --branch ${GITHUB_REF} https://github.com/Sovereign-Labs/hyperlane-monorepo.git /tmp/hyperlane \
    npm install -g yarn && \
    && cd /tmp/hyperlane \
    && yarn workspace @hyperlane-xyz/cli bundle \
    && npm install -g ./typescript/cli && \
    && apt-get remove --purge -y git jq bash \
    && apt-get autoremove -y \
    && apt-get clean \
    && rm -rf /tmp/hyperlane \
    && yarn cache clean \
    && npm cache clean --force

LABEL hyperlane.cli.ref="Sovereign-Labs/hyperlane-monorepo:${GITHUB_REF}"

ENTRYPOINT ["hyperlane"]