# Celestia Namespace Stats

Utility to scan a Celestia namespace over a block range and emit CSV stats for debugging and offline analysis.

## What it outputs

CSV columns:

- `height`
- `blob_count`
- `total_blob_size_bytes`
- `blob_sizes_bytes` (semicolon-delimited list of blob sizes in that block)

## Usage

From repo root:

```bash
cargo run --package celestia-namespace-stats -- \
  --rpc-url "wss://your-endpoint.example" \
  --namespace "sov-test" \
  --start-height 100000 \
  --end-height 100200 \
  --output /tmp/celestia_namespace_stats.csv
```

If your namespace is in hex, prefix with `0x`:

```bash
--namespace 0x736f762d74657374
```

If your provider requires an auth header token:

```bash
--rpc-auth-token "$SOV_CELESTIA_RPC_AUTH_TOKEN"
```

Or set env var `SOV_CELESTIA_RPC_AUTH_TOKEN`.
