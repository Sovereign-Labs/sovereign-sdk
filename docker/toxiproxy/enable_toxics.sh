#!/bin/sh
set -e

TOXIPROXY_HOST="${TOXIPROXY_HOST:-127.0.0.1}"
PROXY_TARGET="${PROXY_TARGET:-sequencer-0}"
TIMEOUT_RATIO="${TIMEOUT_RATIO:-0.1}"
LATENCY_RATIO="${LATENCY_RATIO:-0.1}"
LIMIT_DATA_RATIO="${LIMIT_DATA_RATIO:-0.3}"


echo "⚠️  WARNING: Adding toxics to a proxy with active connections may cause toxiproxy to crash."
echo "🔄 If you encounter crashes, restart the toxiproxy service and try again."
echo ""

# Check if toxiproxy is accessible
echo "🔍 Checking toxiproxy connectivity..."
if ! curl -s --fail --connect-timeout 5 http://$TOXIPROXY_HOST:8474/proxies >/dev/null 2>&1; then
    echo "❌ Cannot connect to toxiproxy at $TOXIPROXY_HOST:8474"
    echo "💡 Make sure toxiproxy is running: docker compose up toxiproxy"
    exit 1
fi

# Check if  proxy exists
if ! curl -s --fail http://$TOXIPROXY_HOST:8474/proxies/$PROXY_TARGET >/dev/null 2>&1; then
    echo "❌ Proxy '$PROXY_TARGET' not found"
    echo "💡 Make sure to run configure.sh first to create the proxy"
    exit 1
fi

echo "✅ Toxiproxy is accessible and proxy exists"
echo ""

# Function to add toxic with error handling
add_toxic() {
    local toxic_type="$1"
    local toxic_data="$2"

    echo "🧪 Adding $toxic_type toxic..."

    # Use temporary files to capture curl output and HTTP code separately
    local temp_response=$(mktemp)
    local temp_stderr=$(mktemp)

    # Make the curl request
    local http_code
    http_code=$(curl -s -w "%{http_code}" -H "Content-Type: application/json" \
        -d "$toxic_data" \
        -o "$temp_response" \
        http://$TOXIPROXY_HOST:8474/proxies/$PROXY_TARGET/toxics 2>"$temp_stderr")

    local response_body=$(cat "$temp_response")
    local stderr_output=$(cat "$temp_stderr")

    if [ "$http_code" -eq 200 ] || [ "$http_code" -eq 201 ]; then
        echo "✅ $toxic_type toxic added successfully"
    else
        echo "❌ Failed to add $toxic_type toxic (HTTP $http_code)"
        if [ -n "$response_body" ]; then
            echo "🔍 API response: $response_body"
        fi
        if [ -n "$stderr_output" ]; then
            echo "🔍 Curl error: $stderr_output"
        fi
        echo "💡 Try restarting toxiproxy:   make restart-toxiproxy"

        # Clean up temp files
        rm -f "$temp_response" "$temp_stderr"
        return 1
    fi

    # Clean up temp files
    rm -f "$temp_response" "$temp_stderr"
}

# Add toxics with better error handling
add_toxic "timeout" "{\"type\": \"timeout\", \"toxicity\": $TIMEOUT_RATIO, \"attributes\": {\"timeout\": 30000}}" || exit 1
sleep 1
add_toxic "latency" "{\"type\": \"latency\", \"toxicity\": $LATENCY_RATIO, \"attributes\": {\"latency\": 65000}}" || exit 1
sleep 1
add_toxic "limit_data" "{\"type\": \"limit_data\", \"toxicity\": $LIMIT_DATA_RATIO, \"attributes\": {\"bytes\": 5000}}" || exit 1

echo ""
echo "📋 Final Configuration:"
curl -s "http://$TOXIPROXY_HOST:8474/proxies/$PROXY_TARGET/toxics"

echo ""
echo "✅ Standard toxics configuration completed!"
