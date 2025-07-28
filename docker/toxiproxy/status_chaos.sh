#!/bin/sh

echo "Checking toxiproxy status..."
echo ""

# Check if toxiproxy is running
if ! curl -s http://localhost:8474/proxies >/dev/null 2>&1; then
    echo "❌ Toxiproxy is not reachable at localhost:8474"
    exit 1
fi

# Get proxy status
echo "📊 Proxy Status:"
proxy_info=$(curl -s http://localhost:8474/proxies/sequencer-0)
echo "$proxy_info" | grep -o '"enabled":[^,]*' | cut -d: -f2

# Show active toxics
echo ""
echo "🧪 Active Toxics:"
curl -s http://localhost:8474/proxies/sequencer-0/toxics | \
    jq -r '.[].name' | \
    while read toxic; do
        echo "  - $toxic"
    done

echo ""
echo "📋 Full Configuration:"
curl -s http://localhost:8474/proxies/sequencer-0