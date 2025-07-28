#!/bin/bash

echo "check RPC"
curl -s -f http://127.0.0.1:26657/health

echo "check gRPC"
grpcurl -plaintext localhost:9090 list
