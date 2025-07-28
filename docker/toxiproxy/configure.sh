#!/bin/sh
set -e

# This script just enables proxies, but does not install any toxics

echo "Configuring toxiproxy on standard mode for sequencer-0:\n"
TOXIPROXY_HOST="${TOXIPROXY_HOST:-toxiproxy}"

echo "Creating proxy..."
curl -v --fail -H "Content-Type: application/json" -d '{"name" : "sequencer-0", "listen" : "0.0.0.0:26659", "upstream" : "sequencer-0:26658"}' http://$TOXIPROXY_HOST:8474/proxies

echo "\n\n===== Final Configuration ====="
curl -s http://$TOXIPROXY_HOST:8474/proxies/sequencer-0

echo "\n\n=====\nConfiguration is completed!"