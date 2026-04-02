# Hive RPC-Compat Tests

Runs Ethereum [Hive](https://github.com/ethereum/hive) `ethereum/rpc-compat` conformance tests against `sov-demo-rollup` to validate JSON-RPC and EVM behavior.

## Prerequisites

- **Docker** — **io_uring syscalls must be allowed in Docker's seccomp profile (required by NOMT storage)**
- **Go** (to build Hive)
- **jq** (for result parsing)

## One-Time Setup

### How Hive works

Hive is a Docker-based test harness. It has two key concepts:

- **Clients** — the software being tested. Each client is a Docker image that Hive builds from a Dockerfile it finds under `<hive-repo>/clients/<client-name>/`. During a test, Hive launches a container from this image, injects a genesis file, and sends JSON-RPC requests to it.
- **Simulators** — test suites that define what requests to send and what responses to expect. We use the built-in `ethereum/rpc-compat` simulator.

Our setup pre-builds the `sov-demo-rollup` Docker image (via `run-rpc-compat.sh --build-image`) and registers a stub Dockerfile in Hive's `clients/` directory that simply references the pre-built image. This avoids duplicating the build logic inside the Hive repo.

### 1. Clone and build Hive

```bash
cd ~/workspace
git clone https://github.com/ethereum/hive.git
cd hive
go build .
```

### 2. Register the client in Hive

Create a stub Dockerfile so Hive can discover our client:

```bash
mkdir -p ~/workspace/hive/clients/sov-demo-rollup
cat > ~/workspace/hive/clients/sov-demo-rollup/Dockerfile <<'EOF'
FROM sov-demo-rollup-hive:local
EOF
```

## Running Tests

All commands are run from the SDK root.

### Build image + run P0 gate (27-test smoke suite)

```bash
bash examples/demo-rollup/hive/run-rpc-compat.sh \
  --build-image \
  --profile p0 \
  --tag p0-nonhistorical \
  --exit-on-fail
```

### Run full rpc-compat baseline

```bash
bash examples/demo-rollup/hive/run-rpc-compat.sh \
  --build-image \
  --profile full \
  --tag full-baseline
```

### Key flags

| Flag              | Description                           | Default                |
|-------------------|---------------------------------------|------------------------|
| `--build-image`   | Build the Docker image before running | off                    |
| `--profile`       | `full`, `p0`, or `p0-nonhistorical`   | `full`                 |
| `--tag`           | Suffix for the results directory      | `run`                  |
| `--exit-on-fail`  | Exit non-zero on test failures        | off                    |
| `--hive-dir`      | Path to Hive repo                     | `$HOME/workspace/hive` |
| `--image-tag`     | Docker image tag                      | `local`                |
| `--chain-id`      | EVM chain ID baked into the image     | `3503995874084926`     |
| `--limit`         | Override Hive `--sim.limit` regex     | (none)                 |
| `--check-timeout` | Per-test timeout                      | `10m`                  |

## Results

Results are stored under `<hive-dir>/workspace/logs/full-<timestamp>-<tag>/`.

The script prints:
- **Full-suite totals**: total/pass/fail counts
- **Top failing method buckets**: grouped by RPC method
- **Profile-scoped totals** (when using `--profile p0`): pass/fail for just the P0 test set

## How It Works

1. `run-rpc-compat.sh` builds a Docker image containing `sov-demo-rollup` and `sov-hive-genesis-adapter`
2. Hive launches the container and injects a geth-format `genesis.json`
3. Inside the container, `entrypoint.sh`:
   - Converts genesis to Sovereign format via `sov-hive-genesis-adapter`
   - Starts `sov-demo-rollup` with mock DA on port 8546
   - Starts `hive_services.py` which runs an RPC proxy (8545) and Engine API stub (8551)
4. Hive's `ethereum/rpc-compat` simulator sends JSON-RPC requests to port 8545
5. Results are collected and summarized

## File Overview

| File                           | Purpose                                                  |
|--------------------------------|----------------------------------------------------------|
| `Dockerfile`                   | Multi-stage build for the Hive client image              |
| `entrypoint.sh`                | Container startup: genesis conversion, backend, services |
| `run-rpc-compat.sh`            | Host-side orchestrator for building and running Hive     |
| `hive_services.py`             | RPC proxy (8545) + Engine API stub (8551)                |
| `wait_for_rpc.py`              | Readiness probe for backend RPC                          |
| `enode.sh`                     | Stub for Hive's peer discovery protocol                  |
| `mock_nomt_rollup_config.toml` | Rollup config for NOMT-backed mock DA                    |
