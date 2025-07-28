#!/usr/bin/env bash

set -euo pipefail

# Check if Docker CLI is working
if ! docker info > /dev/null 2>&1; then
    echo "Docker is not running or not installed. Not possible to run observability stack."
    exit 0
fi

# Set up paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OBSERVABILITY_DIR="$PROJECT_ROOT/docker/observability"
OBSERVABILITY_REPO="https://github.com/Sovereign-Labs/sov-observability.git"

# Create observability directory if it doesn't exist
mkdir -p "$OBSERVABILITY_DIR"

# Clone or update the observability repository
if [ -d "$OBSERVABILITY_DIR/.git" ]; then
    echo "Updating sov-observability repository..."
    cd "$OBSERVABILITY_DIR"
    git pull origin main
else
    echo "Cloning sov-observability repository..."
    cd "$PROJECT_ROOT/docker"
    rm -rf observability
    git clone "$OBSERVABILITY_REPO" observability
fi

# Execute make start from the observability directory
cd "$OBSERVABILITY_DIR"
make start