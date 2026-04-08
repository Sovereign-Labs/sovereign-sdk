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

## Interpreting Results

### Important: all tests run, but not all are evaluated

Hive's `ethereum/rpc-compat` simulator does not support fine-grained test filtering. When using `--profile p0`, the runner executes the **full suite** (all ~200 tests) but only evaluates pass/fail against the P0 scope defined in `p0-nonhistorical-tests.regex`. Tests outside the scope still run and their failures appear in the logs, but they do not affect the exit code.

### Summarize results with `summarize-results.py`

Use the included script to get a readable breakdown:

```bash
# P0 scoped summary (only tests in the P0 scope)
python3 examples/demo-rollup/hive/summarize-results.py <run-dir> \
    --scope examples/demo-rollup/hive/p0-nonhistorical-tests.regex

# Full suite summary (all tests)
python3 examples/demo-rollup/hive/summarize-results.py <run-dir>
```

The output has two sections:
1. **Summary** — per-method pass/fail bullets showing every test case
2. **Failure details** — for each failing test: the JSON-RPC request sent, the response received, and a diff against the expected response

### Run directory structure

Each run directory contains:

```
full-<timestamp>-<tag>/
  hive.json                  # Hive version metadata
  runner.log                 # Captured stdout/stderr from the Hive process
  <suite-id>.json            # Machine-readable test results (used by summarize-results.py)
  details/
    <suite-id>-0.log         # Combined log of all test request/response pairs
  sov-demo-rollup/
    client-<id>.log          # Container stdout/stderr from sov-demo-rollup
```

### Reading raw test logs

The result JSON stores byte offsets (`log.begin`/`log.end`) into the details log file for each test case. To extract the raw log for a specific test without the summary script:

```bash
# Find the details log
DETAILS=$(ls <run-dir>/details/*.log)

# Extract bytes for a specific test (offsets from the result JSON)
dd if="$DETAILS" bs=1 skip=<begin> count=$((end - begin)) 2>/dev/null
```

Each test entry in the details log shows:
- `>>` — the JSON-RPC request sent to the client
- `<<` — the response received
- A diff showing `--` (what the client returned) vs `++` (what the test expected)

## Manual Container Testing

You can run the container directly (without Hive) to verify the image is operational.

### 1. Build the image

```bash
bash examples/demo-rollup/hive/run-rpc-compat.sh --build-image
```

### 2. Run the container

The entrypoint expects a geth-format `/genesis.json`. You can provide a minimal one:

```bash
docker run --rm -it -p 8545:8545 -p 8551:8551 \
  -v "$(pwd)/examples/test-data/genesis/demo/mock/genesis.json:/genesis.json:ro" \
  sov-demo-rollup-hive:local
```

You should see log output like:
```
Starting sov-demo-rollup backend (mock DA + NOMT) on :8546
Waiting for backend RPC on :8546
Starting hive services (engine stub :8551 and RPC root proxy :8545)
```

### 3. Verify RPC is responding

In another terminal:

```bash
# Check chain ID
curl -s -X POST http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
```

It should return valid JSON-RPC responses with `"jsonrpc":"2.0"` and a `"result"` field.

## Debugging

### Where to look when things fail

1. **Hive run directory** — each run stores logs under `<hive-dir>/workspace/logs/full-<timestamp>-<tag>/`:
   - `hive.json` — Hive version and build metadata
   - `runner.log` — captured stdout/stderr from the Hive process
   - `*-simulator-*.log` — simulator container output (check here for RPC errors)

2. **Increase Hive log verbosity**:
   ```bash
   HIVE_LOGLEVEL=5 HIVE_SIM_LOGLEVEL=5 bash examples/demo-rollup/hive/run-rpc-compat.sh ...
   ```

3. **Run Hive directly** to see full output (from the Hive repo directory):
   ```bash
   ./hive --sim ethereum/rpc-compat --client sov-demo-rollup --loglevel 5 --docker.output
   ```
   The `--docker.output` flag includes container build logs in the output.

### Keeping containers alive after failure

By default Hive removes containers after each test. To keep them for inspection:

```bash
./hive --sim ethereum/rpc-compat --client sov-demo-rollup \
  --client.checktimelimit 1h \
  --loglevel 5
```

The long `--client.checktimelimit` gives you time to `docker exec` into a running client container:

```bash
# Find the running container
docker ps --filter "ancestor=hive/clients/sov-demo-rollup:latest"

# Shell into it
docker exec -it <container-id> bash

# Check processes
ps aux

# Check backend RPC directly
curl -s -X POST http://127.0.0.1:8546/rpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'

# Check proxy RPC
curl -s -X POST http://127.0.0.1:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
```

### Docker Desktop known issues

- **`DOCKER_HOST` not set**: Hive expects Docker at `/var/run/docker.sock`. On macOS with Docker Desktop, set `export DOCKER_HOST="unix://$HOME/.docker/run/docker.sock"` in your shell profile.
- **Container networking**: The simulator may fail with `dial tcp :8081: connection refused` if Docker Desktop's container-to-container networking has issues with IP resolution. Try restarting Docker Desktop or upgrading Hive.
- **io_uring denied**: If NOMT crashes with `io_uring` errors, your Docker seccomp profile needs to allow `io_uring_setup`, `io_uring_enter`, and `io_uring_register` syscalls.

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
| `mock_rollup_config.toml`      | Rollup config for NOMT-backed mock DA                    |
| `summarize-results.py`         | Human-readable test result summary with failure details  |
| `p0-nonhistorical-tests.regex` | P0 scope: test name patterns for the phase-1 gate        |
