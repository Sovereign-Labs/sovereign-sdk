FROM ubuntu:jammy

RUN apt-get update && \
    apt-get install -y curl libssl3 ca-certificates && \
    rm -rf /var/lib/apt/lists/*

RUN mkdir -p /da/ && \
    groupadd -r avail && \
    useradd --no-log-init -r -g avail avail && \
    chown -R avail:avail /da

USER avail:avail
WORKDIR /da

ARG NODE_CLIENT_URL
ARG APP_ID

RUN curl -L https://availproject.github.io/configs/kate/avail-light-1.4.3/config.yaml --output config.yaml && \
    curl -L https://github.com/availproject/avail-light/releases/download/v1.4.4/avail-light-linux-aarch64.tar.gz --output avail-light-linux-aarch64.tar.gz && \
    tar -xf avail-light-linux-aarch64.tar.gz && \
		echo "app_id = ${APP_ID}" >> config.yaml && \
		sed -i "s#full_node_ws = .*#full_node_ws = ['$NODE_CLIENT_URL']#" config.yaml

ENV \
    APP_ID=0

# Opencontainers annotations
LABEL \
    org.opencontainers.image.authors="The Avail Project Team" \
    org.opencontainers.image.url="https://www.availproject.org/" \
    org.opencontainers.image.documentation="https://github.com/availproject/avail-light#readme" \
    org.opencontainers.image.version="1.4.3" \
    org.opencontainers.image.revision="1" \
    org.opencontainers.image.vendor="The Avail Project" \
    org.opencontainers.image.licenses="MIT" \
    org.opencontainers.image.title="Avail Light Client" \
    org.opencontainers.image.description="Data Availability Docker Node"

CMD ls; cat config.yaml; ./avail-light-linux-aarch64
