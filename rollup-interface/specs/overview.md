<div align="center">
  <h1> Sovereign SDK </h1>
</div>

## Overview and Core APIs

A Sovereign SDK chain defines a _logical_ blockchain which is the combination of three distinct elements:

1. An L1 blockchain - which provides data availability (DA) and consensus
2. A state transition function (written in Rust), which implements some "business logic" running over the
   data provided by the L1
3. A zero-knowledge proof system capable of (1) recursion and (2) running arbitrary Rust code

The required functionality of each of these core components can be found in the [Rollup Interface specification](./interfaces).

Conceptually, adding a block to a Sovereign SDK chain happens in three steps. First, a sequencer posts a new blob of data onto
the L1 chain. As soon as the blob is finalized on L1, it is logically final. Immediately after the L1 block is finalized,
full nodes of the rollup scan through it and process all relevant data blobs in the order that they appear,
generating a new rollup state root. At this point, the block is subjectively finalized from the perspective of all full nodes.
Last but not least, prover nodes (full nodes running inside a zkVM) perform roughly the same process as full nodes -
scanning through the DA block and processing all of the batches in order - producing proofs and posting them on chain.
(Proofs need to be posted on chain if the rollup wants to incentivize provers - otherwise, it's impossible to tell
which prover was first to process a given batch).
Once a proof for a given batch has been posted on chain, the batch is subjectively final to all nodes including light clients.

![Diagram showing batches and proofs posted on an L1](./assets/SovSDK.png)

## Glossary

- DA chain: Short for Data Availability chain. The Layer 1 blockchain underlying a Sovereign SDK rollup.
- Slot: a "block" in the Data Availability layer. May contain many batches of rollup transactions.
- Header: An overloaded term that may refer to (1) a block header of the _logical_ chain defined by the SDK,
  (2) a block header of the underlying L1 ("Data Availability") chain or (3) a batch header.
- Batch: a group of 1 or more rollup transactions which are submitted as a single data blob on the DA chain.
- Batch Header: A summary of a given batch, posted on the L1 alongside the transactions. Rollups may define this header
  to contain any relevant information, but may also choose to omit it entirely.
- JMT: Jellyfish Merkle Tree - an optimized sparse merkle tree invented by Diem and used in many modern blockchains.
