#!/usr/bin/env bash

set -euo pipefail

# Set up paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OBSERVABILITY_DIR="$PROJECT_ROOT/docker/observability"

# Check if observability repository exists
if [ ! -d "$OBSERVABILITY_DIR/.git" ]; then
    echo "Error: Observability repository not found at $OBSERVABILITY_DIR"
    echo "Please run start_observability.sh first to clone the repository."
    exit 1
fi

# Execute make stop from the observability directory
echo "Stopping observability services..."
cd "$OBSERVABILITY_DIR"
make stop