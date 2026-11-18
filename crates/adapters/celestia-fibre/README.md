# sov-celestia-fibre-adapter

`sov-celestia-fibre-adapter` is a Sovereign SDK DA adapter that posts rollup blobs through
[Celestia Fibre](https://github.com/celestiaorg/celestia-app) and reads them back from Fibre
validators.

## ⚠️ Important Warning

`sov-celestia-fibre-adapter` is a _research-only_ prototype.
This code has not been audited, and may contain critical vulnerabilities. Do not attempt to use in production.

## How it Works

Fibre keeps blob data off-chain with the Celestia validators. A rollup namespace only carries one
v2 share per blob, holding the Fibre blob version, the blob commitment and the signer of the
`MsgPayForFibre` that paid for it.

- **Submitting** (`DaService::send_transaction`, `send_proof`): the blob is encoded and uploaded
  to the validators, which sign a payment promise. A `MsgPayForFibre` carrying that promise is
  then broadcast and confirmed on the consensus node. Once broadcast, the tx is never re-signed:
  a new tx would carry the same payment promise, which the chain settles only once.
- **Reading** (`DaService::get_block_at`): the node fetches the header and the namespace data from
  a bridge node, selects the Fibre references among the namespace sequences, and downloads the
  referenced blobs from the validators of that block's height.
- **Verifying** (`DaVerifier`, `CelestiaFibreVerifier`):
  - **Completeness:** `FibreNamespaceProof` carries the first share of every sequence in the
    namespace, proven against the header's data root. Each sequence's length gives the position
    of the next one.
  - **Correctness:** every blob must match its reference (sender and commitment).
  - **Partial reads:** a `FibreRowsProof` authenticates the blob length and the prefix of the blob
    the rollup actually read, against the commitment. Only the rows covering the read bytes go
    into the witness.

## Configuration

`CelestiaFibreConfig` goes in the `[da]` section of the rollup config. It needs:

- `rpc_url`: a celestia-node (bridge) JSON-RPC endpoint.
- `grpc_url`: a consensus node gRPC endpoint. Fibre needs it to read blobs as well, for the
  validator set.
- `signer_private_key`: only to submit blobs. The signer needs a Fibre escrow account
  (`MsgDepositToEscrow`) with enough funds, or every shard upload fails.

Timeouts, backoff, fallback endpoints and the `SOV_CELESTIA_*` environment variables work as in
`sov-celestia-adapter`'s `CelestiaConfig`. The Fibre-specific parameters are:

- `fibre_request_timeout_secs` (default 30): timeout of one shard download or upload to a single
  validator. A validator that exceeds it is treated as failed, and the client asks another one.
- `fibre_grpc_url` and `fibre_grpc_auth_token`: a separate consensus node gRPC for Fibre's
  validator-set and validator-host queries. Default: the `grpc_url` endpoints.

See [`celestia_fibre_rollup_config.toml`](../../../examples/demo-rollup/configs/celestia_fibre_rollup_config.toml)
for an annotated example. Unlike `CelestiaConfig`, there is no `verify_on_fetch_mode`:
`get_block_at` always verifies the namespace data against the header.

## Metrics

Every attempt of a node or validator call is reported with its duration and outcome:
`sov_celestia_fibre_adapter_{get_block_header, get_head_block_header, get_namespace_data,
submit_pay_for_fibre, tx_status, fibre_download, fibre_upload}`. Fibre downloads and uploads also
report the blob size in `bytes`.
