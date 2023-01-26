<div align="center">
  <h1> Sovereign SDK </h1>
</div>

The Sovereign SDK is a toolkit for developing zk-rollups. It provides two related sets of functionality: a generalized "full
node" implementation that is generic over an internal state transition function ("STF"), and a set of default modules
that can be used within an STF to provide common functionality. The outer "node" implementation is similar to the `tendermint`
package, except that the Sovereign node treats the consensus algorithm as a pluggable module - which allows it to support 
many different L1s with minimal changes. The set of modules is conceptually similar to the Cosmos SDK, though there are 
some differences in the implementation.

A Sovereign SDK chain defines a *logical* blockchain which is the combination of three distinct elements:

1. An L1 blockchain - which provides DA and consensus
2. A state transition function (written in Rust), which implements some "business logic" running over the
data provided by the L1
3. A zero-knowledge proof system capable of (1) recursion and (2) running arbitrary Rust code

The required functionality of each of these core components can be found in the [Rollup Interface specification](./interfaces).

Conceptually, adding a block to a Sovereign SDK happens in three steps. First, a sequencer posts a new blob of data onto
the L1 chain. As soon as the blob is finalized on chain, it is logically final. Immediately after the L1 block is finalized,
full nodes of the rollup scan through it and process all relevant data blobs in the order that they appear,
generating a new rollup state root. At this point, the block is subjectively finalized from the perspective of all full nodes.
Last but not least, prover nodes (full nodes running inside of a zkVM) perform roughly the same process as full nodes -
scanning through the DA block and processing all of the batches in order - producing proofs and posting them on chain.
(Proofs need to be posted on chain if the rollup wants to incentivize provers - otherwise, it's impossible to tell
which prover was first to process a given batch).
Once a proof for a given batch has been posted onchain, the batch is subjectively final to all nodes including light clients.

![Diagram showing batches and proofs posted on an L1](./assets/SovSDK.png)

For more information, see the [Sovereign SDK Overview](specs/overview.md).

## License

Licensed under the [Apache License, Version
2.0](./LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
