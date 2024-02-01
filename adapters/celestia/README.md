# Jupiter

Jupiter is a _research-only_ adapter making Celestia compatible with the Sovereign SDK. None of its code is
suitable for production use. It contains known security flaws and numerous inefficiencies.

## Celestia Integration

The current version of Jupiter runs against Celestia-node version `v0.11.0-rc15`.
This is the version used on the `arabica` and `mocha` testnets as of Oct 16, 2023.

## Warning

Jupiter is a research prototype. It contains known vulnerabilities and should not be used in production under any
circumstances.

## How it Works

All of Jupiter boils down to two trait implementations: [`DaVerifier`](https://github.com/Sovereign-Labs/sovereign-sdk/blob/8388dc2176940bc6a909076e5ed43feb5a87bf7a/sdk/src/state_machine/da.rs#L36) and [`DaService`](https://github.com/Sovereign-Labs/sovereign-sdk/blob/8388dc2176940bc6a909076e5ed43feb5a87bf7a/sdk/src/node/services/da.rs#L13).

### The DaVerifier Trait

The DaVerifier trait is the simpler of the two core traits. Its job is to take a list of BlobTransactions from a DA layer block
and verify that the list is _complete_ and _correct_. Once deployed in a rollup, the data verified by this trait
will be passed to the state transition function, so non-determinism should be strictly avoided.

The logic inside this trait will get compiled down into your rollup's proof system, so it's important to gain a high
degree of confidence in its correctness (upgrading SNARKs is hard!) and think carefully about performance.

At a bare minimum, you should ensure that the verifier rejects...

1. If the order of the blobs in an otherwise valid input is changed
1. If the sender of any of the blobs is tampered with
1. If any blob is omitted
1. If any blob is duplicated
1. If any extra blobs are added

We also recommend (but don't require) that any logic in the `DaVerifier` be able to build with `no_std`.
This maximizes your odds of being compatible with new zk proof systems as they become available. However,
it's worth noting that some Rust-compatible SNARKs (including Risc0) support limited versions of `std`. If you only care
about compatibility with these proof systems, then `no_std` isn't a requirement.

**Jupiter's DA Verifier**

Blobs submitted to Celestia are processed and composed into the `ExtendedDataSquare`. Submitting the blobs to the Celestia
results in two different additions in the data square.

First, the cosmos `Tx` is created which contains the `MsgPayForBlobs`
message. This message contains the address of the `signer`, namespaces of all the blobs included and their commitments.
This cosmos transaction is then appended to other transactions appearing in given block. All the transactions are then
splitted into [`Compact Shares`](https://github.com/celestiaorg/celestia-app/blob/main/specs/src/specs/shares.md#transaction-shares)
and included in the data square under the [`PAY_FOR_BLOB_NAMESPACE`](https://github.com/celestiaorg/celestia-app/blob/main/specs/src/specs/namespace.md).

Second, each submitted blob is split into the [`Sparse Shares`](https://github.com/celestiaorg/celestia-app/blob/main/specs/src/specs/shares.md#share-format)
and also included in the data square, each blob under it's own namespace.

The layout and structure of the `ExtendedDataSquare` is explained in [data square layout spec](https://github.com/celestiaorg/celestia-app/blob/main/specs/src/specs/data_square_layout.md#data-square-layout)
and in the [data structures spec](https://github.com/celestiaorg/celestia-app/blob/main/specs/src/specs/data_structures.md#arranging-available-data-into-shares).
Celestia distributes the [`DataAvailabilityHeader`](https://github.com/celestiaorg/celestia-app/blob/main/specs/src/specs/data_structures.md#availabledataheader)
in block's `ExtendedHeader` which have all the merkle roots for each row and column of the data square.
Those can be later compared with the computed row roots from the NMT proofs.

#### Checking _completeness_ of the data

In order to acquire the data for a given block, the `share.GetSharesByNamespace` RPC call is used. In return, celestia-node
provides all the shares for the given namespace (rollup's namespace) together with the proofs. The shares are returned as a
list of rows, in order, each row having only the relevant shares and the proof of inclusion of those shares or a single empty
row and the proof of absence of rollup's data.

Data is complete when it includes all the transactions belonging to the rollup in a given block.
Checking _completeness_ of the data in celestia is done by verifying that the namespaces of the siblings of rollup's data shares
are respectively lower and higher than rollup's namespace. This can be done using [`NmtProof::verify_complete_namespace`](https://github.com/Sovereign-Labs/nmt-rs/blob/master/src/nmt_proof.rs#L38)
for each row in data square that hold's rollup's data. Merkle roots computed using the proofs should be equal to the roots
obtained from the `DataAvailabilityHeader` header of this block.
As for the empty blocks (not containing rollup's data) we get an empty row with an absence proof, the same logic
applies for proving that there is no rollup's data.

#### Checking _correctness_ of the data

Checking _correctness_, is a bit more complicated. Unfortunately, Celestia does not currently provide a natural
way to associate a blob of data with its sender - so we have to be pretty creative with our solution. (Recall that the
Sovereign SDK requires blobs to be attributable to a particular sender for DOS protection). We have to read
all of the data from a special reserved namespace on Celestia which contains the Cosmos SDK transactions associated
with the current block. The transactions are serialized using `protobuf` and encoded into data square in
[compact share format](https://github.com/celestiaorg/celestia-app/blob/main/specs/src/specs/shares.md#transaction-shares).

In order to prove that, we use a proofs called `EtxProof` which consist of the merkle proofs for all the shares contaniing transaction
as well the offset to the beginning of the cosmos transaction in first of those shares.

To venify them, we first iterate over rollup's blobs re-created from _completeness_ verification. We associate each blob
with its `EtxProof`. Then we verify that the etx proof holds the contiguous range of shares and verify the merkle proofs
of it's shares with corresponding row_roots from `DataAvailabilityHeader`.
If that process succeeds, we can extract the cosmos transaction data from the given proof. We need to check if the
transaction offset provided in proof is indeed a start of a new transaction, and then we extract transaction data from
the contiguous shares according to the compact share format. Then we acquire the `MsgPayForBlobs` message from it
deserializing cosmos transaction and then it's data.
Then, we can finally verify the sender and commitment of the rollup's blob. As the `MsgPayForBlobs` can hold metadata
about many blobs, we then iterate over them. When we encounter the rollup's namespace, we take next provided rollup transaction
and check if it's sender is equal to the rollup tx's sender. Then we verify if current's blob data is equal to the verified rollup
tx's data and if recomputed blob's commitment is matching the commitment from the current metadata.
(note: I think this has a flaw. If any `MsgPayForBlobs` holds metadata for more than a single rollup's blob, then we will try to verify
next transaction with the old blob. To fix that, we could only iterate over the `EtxProof`s and take both next blob and next transaction
when we encounter metadata for rollup's namespace. This shouldn't be an issue now as in `DaService` we only submit a single blob at a time
but can become an issue when that's changed).
If all proofs and all blobs were verified successfully, that means the data is correct.

### The DaService Trait

The `DaService` trait is slightly more complicated than the `DaVerifier`. Thankfully, it exists entirely outside of the
rollup's state machine - so it never has to be proven in zk. This means that its performance is less critical, and that
upgrading it in response to a vulnerability is much easier.

The job of the `DAService` is to allow the Sovereign SDK's node software to communicate with a DA layer. It has two related
responsibilities. The first is to interact with DA layer nodes via RPC - retrieving data for the rollup as it becomes
available. The second is to process that data into the form expected by the `DaVerifier`. For example, almost all DA layers
provide data in JSON format via RPC - but, parsing JSON in a zk-SNARK would be horribly inefficient. So, the `DaService`
is responsible for both querying the RPC service and transforming its responses into a more useful format.

**Jupiter's DA Service**
Jupiter's DA service currently communicates with a local Celestia node via JSON-RPC. Each time a Celestia block is
created, the DA service makes a series of RPC requests to obtain all of the relevant share data. Then, it packages
that data into the format expected by the DA verifier and returns.

## License

Licensed under the [Apache License, Version
2.0](./LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
