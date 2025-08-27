FROM node:20-alpine

ARG CLI_VERSION=latest

RUN npm install -g @hyperlane-xyz/cli@${CLI_VERSION}

LABEL hyperlane.cli.version=${CLI_VERSION}

ENTRYPOINT ["hyperlane"]