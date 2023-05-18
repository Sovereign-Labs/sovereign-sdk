# Jupiter

Jupiter is a _research-only_ adapter making Celestia compatible with the Sovereign SDK. None of its code is
suitable for production use. It contains known security flaws and numerous inefficiencies.

## Celestia Integration

The current version of Jupiter runs against Celestia-node version `v0.7.1`. This is the version used on the `arabica` testnet
as of Mar 18, 2023.

## Warning

Jupiter is a research prototype. It contains known vulnerabilities and should not be used in production under any
circumstances.

## How it Works

All of Jupiter boils down to two trait implementations: [`DaVerifier`](https://github.com/Sovereign-Labs/sovereign/blob/8388dc2176940bc6a909076e5ed43feb5a87bf7a/sdk/src/state_machine/da.rs#L36) and [`DaService`](https://github.com/Sovereign-Labs/sovereign/blob/8388dc2176940bc6a909076e5ed43feb5a87bf7a/sdk/src/node/services/da.rs#L13).

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

In Celestia, checking _completeness_ of data is pretty simple. Celestia provides a a "data availability header",
containing the roots of many namespaced merkle tree. The union of the data in each of these namespaced merkle trees
is the data for this Celestia block. So, to prove compleness, we just have to iterate over these roots. At each step,
we verify a "namespace proof" showing the presence (or absence) of data from our namespace
in that row. Then, we check that the blob(s) corresponding to that data appear next in the provided list of blobs.

Checking _correctness_, is a bit more complicated. Unfortunately, Celestia does not currently provide a natural
way to associate a blob of data with its sender - so we have to be pretty creative with our solution. (Recall that the
Sovereign SDK requires blobs to be attributable to a particular sender for DOS protection). We have to read
all of the data from a special reserved namespace on Celestia which contains the Cosmos SDK transactions associated
with the current block. (We accomplish this using the same technique of iterating over the row roots that we descibed previously). Then, we associate each relevant data blob from our rollup namespace with a transaction, using the
[`share commitment`](https://github.com/celestiaorg/celestia-app/blob/main/proto/celestia/blob/v1/tx.proto#L25) field.

### The DaService Trait

The `DaService` trait is slightly more complicated than the `DaVerifier`. Thankfully, it exists entirely outside of the
rollup's state machine - so it never has to be proven in zk. This means that its perfomance is less critical, and that
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
