# sov-celestia-fibre-adapter

Celestia adapter that reads and posts rollup blobs through [Fibre](https://github.com/celestiaorg/lumina) (share version 2).

## How it works

A Fibre blob is one Celestia share: a version byte and a 32-byte commitment. The payload lives off-chain. The sequencer and the prover both post through Fibre, so both signers need a funded Fibre escrow. Blob bytes are posted and read verbatim. There is no compression envelope.

## Trust model

The pointer share is covered by Celestia consensus. The verifier checks a SHA-256 Merkle range proof for the rows that hold the bytes the rollup actually read, and checks that `sha256(row_root ‖ rlc_root)` equals the pointer commitment. It does not re-check the Reed-Solomon or RLC relations. Validators checked those before signing the payment promise, and the chain checked them before it wrote the pointer share.

## Retention

Downloads come from validator shards. This adapter has no archive. A block older than shard retention cannot be fetched until the SDK has state sync.

## Pointer versions

Only blob version 0 with a 36-byte pointer is supported. Any other share-version-2 pointer fails the download and cannot be proven as skipped, so the rollup stops on that block until the adapter learns the new version.

## Configuration

`CelestiaFibreConfig` accepts the same RPC and gRPC fields as `CelestiaConfig`, plus:

| Field | Default |
| --- | --- |
| `fibre_grpc_url` | `grpc_url` |
| `fibre_grpc_auth_token` | `grpc_auth_token` |
| `download_timeout_secs` | 120 |
| `verify_on_fetch` | `off` |

`signer_private_key` is required to submit. Without it the service is read-only.

The demo rollup selects this adapter with `--da-layer celestia-fibre` and feature `celestia_fibre_da`. See `examples/demo-rollup/configs/celestia_fibre_rollup_config.toml`.
