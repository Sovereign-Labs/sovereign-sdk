# sov-celestia-common

Shared Celestia code used by `sov-celestia-adapter` and `sov-celestia-fibre-adapter`.

The guest-safe half covers header hashing, `CelestiaAddress`, namespace share iteration, and the
namespace walk (`verify_namespace`) that checks continuity, skipped shares, and the right-hand
boundary. Each adapter supplies the predicate for which share versions it reads and the function
that authenticates one of those blobs.

The `native` feature adds the Celestia RPC client (`CelestiaNode`), endpoint configuration,
inclusion-proof construction, API metrics, and fixture loading.

Blob-kind-specific code stays in the adapters: v1 share authentication and the compression
envelope in `sov-celestia-adapter`, v2 pointer shares and Fibre row proofs in
`sov-celestia-fibre-adapter`.
