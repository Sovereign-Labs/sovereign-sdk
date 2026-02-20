# Hive rpc-compat Bring-Up for `sov-demo-rollup` (Correctness First)

## Scope
- Simulator: `ethereum/rpc-compat`
- Target binary: `sov-demo-rollup`
- Runtime mode: `--da-layer mock --storage nomt`
- Priority: JSON-RPC and EVM correctness deltas, not throughput.

## Current Status
Latest full result:
- `workspace/logs/full-20260220-130435-plan-impl/1771589325-0814d0020d77913d63eecdbb7fcb198c.json`

Current baseline from that run:
- Total: `200`
- Pass: `84`
- Fail: `116`

Method coverage snapshot:
- RPC method groups exercised: `30`
- Fully passing groups: `19`
- Partially passing groups: `8`
- Zero-pass groups: `3` (`eth_blobBaseFee`, `eth_getProof`, `eth_simulateV1`)

Largest remaining fail buckets:
- `eth_simulateV1`: `91`
- `eth_getLogs`: `6`
- `eth_getTransactionReceipt`: `6`
- `eth_getBlockReceipts`: `4`
- `eth_getProof`: `3`

## What Is Implemented
- Hive eth1 client lifecycle support:
  - JSON-RPC HTTP exposed on `:8545`
  - `/genesis.json` consumed at startup
  - `/hive-bin/enode.sh` present (stub)
  - `/version.txt` generated at build time
- Engine API startup compatibility:
  - `engine_stub.py` serves minimal setup methods on `:8551`
- Genesis translation:
  - now handled by Rust binary `sov-hive-genesis-adapter`
  - geth alloc balances are applied to `bank.json`
  - geth alloc EVM state (`code`, `nonce`, `storage`) is applied to `evm.json`
  - chain ID is written to `chain_id.txt`
  - `/chain.rlp` is used for time-fork activation mapping only
- Chain ID correctness guard:
  - image compiles `CHAIN_ID` from `HIVE_CHAIN_ID`
  - runtime startup fails fast if `chain_id.txt` mismatches compiled chain ID

## Intentional Stubs / Non-Goals (Current)
- Full historical chain import from `/chain.rlp` and `/blocks/*.rlp` into canonical runtime state is not implemented.
- `eth_simulateV1` is not implemented yet.
- `eth_getProof` remains unsupported.
- `eth_blobBaseFee` is intentionally unsupported in this rollup profile.

## NOMT on Other Machines (Custom Seccomp, Recommended)
NOMT requires `io_uring` syscalls inside containers.

### 1) Build a custom Docker seccomp profile
Use Docker default seccomp as base, then allow `io_uring_*`:

```bash
sudo cp /usr/share/docker/seccomp.json /etc/docker/seccomp-nomt.json
sudo jq '.syscalls += [{"names":["io_uring_setup","io_uring_enter","io_uring_register"],"action":"SCMP_ACT_ALLOW"}]' \
  /etc/docker/seccomp-nomt.json | sudo tee /etc/docker/seccomp-nomt.json >/dev/null
```

If `/usr/share/docker/seccomp.json` is missing, fetch Docker's default profile and apply the same `jq` patch.

### 2) Configure Docker daemon to use the profile
`/etc/docker/daemon.json`:

```json
{
  "seccomp-profile": "/etc/docker/seccomp-nomt.json"
}
```

If you already have daemon settings, merge this key into the existing JSON.

Restart Docker:

```bash
sudo systemctl restart docker
```

### 3) Verify `io_uring` is allowed in containers

```bash
docker run --rm --entrypoint python3 hive/clients/sov-demo-rollup:latest \
  -c "import ctypes,os;libc=ctypes.CDLL(None,use_errno=True);fd=libc.syscall(425,2,ctypes.create_string_buffer(256));err=ctypes.get_errno();print('fd',fd,'errno',err,'msg',os.strerror(err) if err else '')"
```

Expected healthy output: `fd` non-negative and `errno 0`.

## Build and Run
Build client image:

```bash
cd /home/nikolai/workspace/sovereign-sdk

docker build \
  --build-arg HIVE_CHAIN_ID=3503995874084926 \
  -f examples/demo-rollup/hive/Dockerfile \
  -t sov-demo-rollup-hive:local \
  .
```

Run full rpc-compat from Hive checkout:

```bash
hive \
  --sim ethereum/rpc-compat \
  --client sov-demo-rollup \
  --loglevel 2 \
  --sim.loglevel 2 \
  --client.checktimelimit 10m
```

## Vanilla Hive Compatibility
- No Hive source patches are required.
- Local integration is done through standard Hive client packaging (`clients/sov-demo-rollup/` and client YAML).

## Next Fix Order
1. Restore deterministic receipt/log behavior for fixture history paths (`eth_getLogs`, `eth_getTransactionReceipt`, `eth_getBlockReceipts`).
2. Decide scope for `eth_getProof` (implement vs intentionally unsupported with documented expectation).
3. Implement `eth_simulateV1` (largest remaining bucket).
