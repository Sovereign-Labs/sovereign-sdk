# sov-celestia-adapter

`sov-celestia-adapter` is an adapter making [Celestia](https://docs.celestia.org/) compatible with the Sovereign SDK.

## ⚠️ Important Warning

`sov-celestia-adapter` is a _research-only_ prototype.
This code has not been audited, and may contain critical vulnerabilities. Do not attempt to use in production.

## Celestia Integration

The current version of `sov-celestia-adapter` runs against celestia-node version `v0.23.1`.
This is the version used on the `mocha` testnet as of Jul 3rd, 2025.

To set up Celestia nodes, please refer to the official documentation provided by Celestia, for example: 
[how to run a light node](https://docs.celestia.org/how-to-guides/light-node)


## How it Works

All of `sov-celestia-adapter` boils down to two trait implementations: 
 - [`DaVerifier`](https://github.com/Sovereign-Labs/sovereign-sdk/blob/545c93c6adfe650d19604cf3413e013a62889f46/crates/rollup-interface/src/state_machine/da.rs#L56)
 - [`DaService`](https://github.com/Sovereign-Labs/sovereign-sdk/blob/545c93c6adfe650d19604cf3413e013a62889f46/crates/rollup-interface/src/node/da.rs#L112)

### The `DaVerifier` Trait

The `DaVerifier` trait is responsible for verifying a set of `BlobTransactions` fetched from a Data Availability (DA) layer block, 
ensuring these transactions are both **complete** and **correct**.

- Once deployed in a rollup environment, verified data is passed to the state transition function. Thus, strict determinism must be maintained.
- The verification logic within this trait is eventually compiled into your rollup SNARK (proof system). Hence, correctness must be thoroughly validated (updating SNARK logic later can be challenging).
- Performance considerations are critical when designing a verifier due to potentially high computational costs during proof verification.

At a bare minimum, you should ensure that the verifier rejects...

1. Any modification to the blobs' original order,
2. Tampering with the sender information of the blobs,
3. Omission of any blob,
4. Duplication of any blob,
5. Addition of extra blobs.

We recommend, but don't mandate, that verifier logic is implemented with Rust's `no_std` attribute. 
Doing so maintains compatibility with a broad range of zero-knowledge (ZK) proof systems becoming available. 
However, certain Rust-compatible SNARKs such as Risc0 support limited `std` features; 
thus, using `no_std` is optional if compatibility is only required for such proof systems.

**`sov-celestia-adapter`'s DA Verifier Internals**

Blobs submitted to Celestia are integrated into Celestia’s data structure called the [`ExtendedDataSquare`](https://celestiaorg.github.io/celestia-app/specs/data_square_layout.html)

Each blob submitted to Celestia is divided into [`Sparse Shares`](https://celestiaorg.github.io/celestia-app/specs/shares.html#overview) and included within the data square under its own unique namespace.

For details on how an `ExtendedDataSquare` is structured, refer to Celestia's-[data square layout specification](https://celestiaorg.github.io/celestia-app/specs/data_square_layout.html) and [data structures specification](https://celestiaorg.github.io/celestia-app/specs/data_structures.html).

Celestia distributes data through an [`ExtendedHeader`](https://celestiaorg.github.io/celestia-app/specs/data_structures.html#header) within each block, 
containing the `DataAvailabilityHeader`. 
This header includes Merkle roots representing each row and column in the data square. 
These roots can subsequently be validated against computed roots derived from [Namespaced Merkle Tree (NMT) proofs](https://github.com/celestiaorg/nmt/blob/ca7cd2f2b0b0e18c9fc2f8e3c8b07756bbff0d88/docs/spec/nmt.md).

#### Checking _completeness_ of the data

To retrieve data for a specific block, use the [`share.GetNamespaceData`](https://node-rpc-docs.celestia.org/?version=v0.21.7#share.GetNamespaceData) RPC call.

Celestia node then responds with shares for the requested namespace (which identifies the rollup), 
along with proofs of inclusion or absence. The response is organized as an ordered list of rows, where each row can contain:

- All shares relevant to the rollup’s namespace along with their inclusion proofs, or
- An empty row paired with an absence proof, indicating no relevant rollup data for this block.

**Verifying Completeness**

Completeness verification in Celestia ensures that all transactions belonging to the rollup for the block are obtained. 
Specifically, this means validating the namespaces of sibling nodes adjacent to the rollup’s namespace in the data square. 
A correct verification confirms:

- The namespace immediately left (preceding shares) is strictly lower than the rollup's namespace.
- The namespace immediately right (following shares) is strictly higher than the rollup's namespace.
- All blobs within the rollup’s namespace are contiguous, leaving no gaps or omissions. But irrelevant shares of some blobs are not verified.

This verification can either:
- Iterate over all rows containing rollup-relevant data and use [`NmtProof::verify_complete_namespace`](https://github.com/Sovereign-Labs/nmt-rs/blob/7b73324b92c8c43f9caa124ad4e5510be01c221d/src/nmt_proof.rs#L50), 
  which entails passing all namespace shares to the verifier—a potentially costly choice that exposes you to DoS (Denial of Service) risks due to verifying unnecessary shares.
- Alternatively, perform targeted checks asserting namespace order and continuity as described above, significantly reducing verification overhead.

Subsequently, Merkle roots computed from provided proofs must match those from the block's `DataAvailabilityHeader`. 
The same verification logic also applies efficiently to blocks containing no rollup data, namely confirming the correctness of absence proofs.

#### Checking _correctness_ of the data

The current version of the adapter relies on sparsed shares V1 which support authored blobs as described in [CIP-21](https://cips.celestia.org/cip-021.html).

To check correctness, a verifier compares each Blob with given prove and checks the following conditions:

1. The blob proof range starts at the beginning of namespace or immediately after the previous blob
2. The first share of the blob has a signer address matching the blob sender and this share is the correct version
3. For each byte read by the rollup, the verifier checks that this data matches the share included in the proof
4. The merkle proof for each share verifies against the corresponding row_root from the `DataAvailabilityHeader`.

If all proofs and all blobs were verified successfully, then the data is correct.

Note, that if the rollup skipped a blob entirely, the verifier will only validate the first share

### The DaService Trait

The `DaService` trait is slightly more complicated than the `DaVerifier`. 
Thankfully, it exists entirely outside of the rollup's state machine — so it never has to be proven in ZK context. 
This means that its performance is less critical, and that upgrading it in response to a vulnerability is much easier.

The job of the `DaService` is to allow the Sovereign SDK's node software to communicate with a DA layer. 
It has two related responsibilities. 
The first is to interact with DA layer nodes via RPC - retrieving data for the rollup as it becomes available. 
The second is to process that data into the form expected by the `DaVerifier`. 
For example, almost all DA layers  provide data in JSON format via RPC - but, 
parsing JSON in a zk-SNARK would be horribly inefficient. 
So, the `DaService` is responsible for both querying the RPC service and transforming its responses into a more useful format.

**sov-celestia-adapter's DA Service**

`sov-celestia-adapter`'s DA service currently communicates with a local Celestia node via JSON-RPC. 
Each time a Celestia block is created, 
the DA service makes a series of RPC requests to obtain all of the relevant share data. 
Then, it packages that data into the format expected by the DA verifier and returns.

## License

Licensed under the [Apache License, Version 2.0](../../../LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
