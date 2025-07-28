#!/bin/bash

TARGET_HEIGHT=4

# Check existence of the token file
echo "Checking network head. Token file: '$1'"
if [ ! -f "$1" ]; then
  echo "Error: Token file $1 does not exist."
  exit 1
fi
TOKEN=$(cat "$1")

# Check header.NetworkHead
CURL_OUTPUT=$(curl -s --fail -X POST http://127.0.0.1:26658 \
     -H "Content-Type: application/json" \
     -d '{"id": 1, "jsonrpc": "2.0", "method": "header.NetworkHead", "params": []}')

echo "Raw curl output: '$CURL_OUTPUT'"
HEIGHT=$(echo "$CURL_OUTPUT" | jq -r ".result.header.height" | bc)

echo "Network head height is: '$HEIGHT'"

# Check if HEIGHT is below $TARGET_HEIGHT
if [ "$HEIGHT" -lt $TARGET_HEIGHT ]; then
    echo "Error: HEIGHT is below $TARGET_HEIGHT"
    exit 1
fi
echo "header.NetworkHead is above or equal to $TARGET_HEIGHT, checking header.."

# Check header.GetByHeight to be more confident in readiness
HEADER=$(curl -s --fail -X POST http://127.0.0.1:26658 \
     -H "Content-Type: application/json" \
     -d "{\"id\": 1, \"jsonrpc\": \"2.0\", \"method\": \"header.GetByHeight\", \"params\": [$TARGET_HEIGHT]}")

if echo "$HEADER" | jq -e '.result.header' >/dev/null; then
  echo "result.header is present at $TARGET_HEIGHT."
else
  echo "result.header is not present at $TARGET_HEIGHT, might contain error: $HEADER"
  exit 1;
fi
