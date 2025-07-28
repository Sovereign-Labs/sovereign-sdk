#!/bin/sh
set -e

TOXIPROXY_HOST=${TOXIPROXY_HOST:-127.0.0.1}
PROXY_NAME="sequencer-0"

echo "🧪 Removing all toxics from proxy: $PROXY_NAME"

if ! curl -s --fail http://$TOXIPROXY_HOST:8474/proxies/$PROXY_NAME > /dev/null 2>&1; then
    echo "❌ Error: Proxy '$PROXY_NAME' does not exist"
    exit 1
fi

# Get all toxics for the proxy
echo "📊 Fetching current toxics..."
toxics_response=$(curl -s --fail http://$TOXIPROXY_HOST:8474/proxies/$PROXY_NAME/toxics)

if [ "$toxics_response" = "[]" ]; then
    echo "No toxics found for proxy '$PROXY_NAME'"
    exit 0
fi

# Parse toxic names using jq
toxic_names=$(echo "$toxics_response" | jq -r '.[].name')

if [ -z "$toxic_names" ]; then
    echo "No toxics found for proxy '$PROXY_NAME'"
    exit 0
fi

echo "Found toxics to remove:"
echo "$toxic_names"

# Remove each toxic
for toxic_name in $toxic_names; do
    echo "Removing toxic: $toxic_name"
    if curl -s --fail -X DELETE http://$TOXIPROXY_HOST:8474/proxies/$PROXY_NAME/toxics/$toxic_name > /dev/null; then
        echo "  ✓ Removed toxic: $toxic_name"
    else
        echo "  ✗ Failed to remove toxic: $toxic_name"
    fi
done

echo "📋 Final Configuration:"
curl -s http://$TOXIPROXY_HOST:8474/proxies/$PROXY_NAME/toxics
echo "\n✅ All toxics have been removed from proxy '$PROXY_NAME'"
