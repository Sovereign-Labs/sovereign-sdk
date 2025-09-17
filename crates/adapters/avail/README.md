# 🔵 Avail DA Adapter

> ⚠️ **Research-only prototype**  
> This code is **not audited** and may contain critical vulnerabilities.  
> Do **not** use in production.

## Avail Integration

`sov-avail-adapter`  makes [Avail](https://www.availproject.org/) compatible with the [Sovereign SDK](https://github.com/Sovereign-Labs/sovereign-sdk). and designed to work with the [Avail Rust Client](https://github.com/availproject/avail-rust) and the [Avail networks](https://docs.availproject.org/da/networks).

## How it Works

The adapter implements two key traits and required functionalities for Sovereign SDK, more details on the interface can be found in the [da](https://github.com/RISHABHAGRAWALZRA/sovereign-sdk/blob/nightly/crates/rollup-interface/specs/interfaces/da.md) Interface specification.:

- [`DaService`](https://github.com/Sovereign-Labs/sovereign-sdk/blob/main/crates/rollup-interface/src/node/da.rs)
- [`DaVerifier`](https://github.com/Sovereign-Labs/sovereign-sdk/blob/main/crates/rollup-interface/src/state_machine/da.rs)

### The `DaService` Trait

The `DaService` trait bridges the Sovereign SDK node to Avail’s DA layer.

> **Note:**  
> Some methods in this trait, such as `get_extraction_proof` and `get_proofs_at`, are **not implemented yet**. They currently return dummy values or errors as placeholders. This means that proof-based verification is **not available** at this time.

**Responsibilities**

1. **Interaction with Avail RPC:**
   - Connects to Avail nodes via HTTP and WebSocket APIs.
   - Fetches block headers, block data, and blob transactions for specific `app_id`s.
2. **Blob Extraction and Transformation:**
   - Filters and decodes blob transactions relevant to the configured `app_id` (for batch or proof blobs).
   - Verifies signatures and formats blobs for the verifier.
   - Handles timestamp and block metadata extraction.
3. **Blob Submission:**
   - Submits new blob transactions to Avail for inclusion in future blocks.
   - Handles both batch and proof blobs, signing them with the configured key.
4. **Retry and Backoff:**
   - Implements exponential backoff and retry logic for network operations, configurable via parameters.

**How It Works**

- For each block, the service fetches the block header and relevant blob transactions via Avail’s RPC.
- It decodes and validates each transaction, extracting the data and verifying the sender’s signature.
- The resulting data is packaged into the format expected by the DA verifier.
- When submitting data, the service signs the transaction and sends it to Avail, tracking submission receipts.

### The `DaVerifier` Trait

The `DaVerifier` trait is responsible for verifying a set of `BlobTransactions` fetched from an Avail DA block, ensuring these transactions are both **complete** and **correct**.

> **Warning:**  
> **This is currently a stub implementation.**  
> The `DaVerifier` always returns `Ok(())` and does **not** verify the correctness or completeness of blobs. No cryptographic or application-level checks are performed. This is a placeholder to enable integration and demo rollup operation while the actual verifier logic is under development.

**Role and Guarantees (when implemented)**

- Ensures determinism: verified data is passed to the state transition function.
- Verification logic is compiled into the rollup’s proof system, so correctness is critical.
- The verifier must reject:
  1. Modification of blob order.
  2. Tampering with sender information.
  3. Omission or duplication of blobs.
  4. Addition of unrelated blobs.

### Implementation

**Avail-specific Details**

- Avail DA blocks contain transactions that embed "blobs" of data, each associated with an `app_id` (which identifies the rollup or application).
- Each blob transaction includes:
  - The data payload (the "blob").
  - The sender’s signature, which is validated to ensure authenticity.
- The verifier (when implemented) will check:
  1. That all relevant blobs for the configured `app_id` are present and in the correct order.
  2. That each blob’s signature matches the sender’s public key.
  3. That no blobs are missing or duplicated, and no extraneous blobs are included.
  4. That the block header and transaction roots align with the Avail data model.

#### Logging and Observability

- The adapter uses the `tracing` crate for structured logging.
- Logs are emitted for block fetches, blob extraction, transaction submission, and error conditions.
- Warnings are logged for skipped or malformed blobs.

#### Retry/Backoff Policy

- Exponential backoff is implemented for all RPC/network operations.
- Configurable parameters include minimum/maximum delay, retry factor, and maximum retries.

#### Configuration

The adapter is configured using a struct with the following parameters:

| Parameter            | Description                                                         |
|----------------------|---------------------------------------------------------------------|
| `http_api_url`       | HTTP endpoint of the Avail node                                     |
| `ws_api_url`         | (Optional) WebSocket endpoint of the Avail node                     |
| `proof_app_id`       | `app_id` for proof blobs                                            |
| `batch_app_id`       | `app_id` for batch blobs                                            |
| `signer_key`         | `seed` phrase of Avail wallet for signing blob transactions                           |
| `backoff_min_delay_secs` | Minimum delay (seconds) for exponential backoff (default: 1)    |
| `backoff_max_delay_secs` | Maximum delay (seconds) for exponential backoff (default: 60)   |
| `backoff_factor`     | Exponential backoff factor (default: 2.0)                           |
| `backoff_max_times`  | Maximum number of retries (default: 3)                              |

See [`AvailDAConfig`](https://github.com/availproject/avail-sovereign-sdk/blob/avail-develop/crates/adapters/avail/src/types/config.rs) in the source for full details.

## Operator Mode
>
> **Current Demo rollup Mode:**
>
> Because the verifier logic and proof-related methods are **not yet implemented**, the rollup is currently run in **operator mode** (`operator`) in the demo-rollup setup. In this mode, no data availability or correctness proofs are checked, and the system trusts the operator to provide valid data.
>
> Once the verifier logic and proof methods are implemented, full verification can be enabled. This will allow the rollup to operate in a permissionless, trust-minimized mode with cryptographic guarantees of data availability and correctness.

## License

Licensed under the [Apache License, Version 2.0](../../../LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.

> **_NOTE:_**
This adapter was originally written by Rishabh Agrawal and reviewed by Vibhu rajeev and Avail team, and
provided as part of the Sovereign SDK under the Apache 2.0 and MIT licenses.
