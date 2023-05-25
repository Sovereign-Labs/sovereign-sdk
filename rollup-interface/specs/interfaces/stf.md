# State transition Function

## Overview
The state transition function serves as the "business logic" for a rollup. It specifies the methods that will be automatically invoked by the rollup during different stages of block processing. Implementations of this trait can be integrated with any ZKVM and DA Layer resulting in a fully functional rollup.

The Sovereign SDK guarantees that all relevant transactions will be delivered to the STF for processing
exactly once, in the order that they appear on the DA layer. The STF is responsible for implementing its own metering and billing (to prevent spam), and for maintaining a "consensus set" (a list of addresses who are allowed to post transactions).

The SDK also allows (and expects) the STF to process any proofs that are posted onto the DA layer to
allow honest provers to be rewarded for their work, and to allow adaptive gas pricing depending on prover throughput.

## Required Types
```rust
    /// Root of state merkle tree
    type StateRoot;
    /// The initial state of the rollup.
    type InitialState;
    /// The contents of a transaction receipt. This is the data that is persisted in the database
    type TxReceiptContents: Serialize + DeserializeOwned + Clone;
    /// The contents of a batch receipt. This is the data that is persisted in the database
    type BatchReceiptContents: Serialize + DeserializeOwned + Clone;
    /// Witness is a data that is produced during actual batch execution
    /// or validated together with proof during verification
    type Witness: Default + Serialize;
    /// A proof that the sequencer has misbehaved. For example, this could be a merkle proof of a transaction
    /// with an invalid signature
    type MisbehaviorProof;
```

## Required Methods:
```rust
    /// Perform one-time initialization for the genesis block.
    fn init_chain(&mut self, params: Self::InitialState);

    /// Called at the beginning of each DA-layer block - whether or not that block contains any
    /// data relevant to the rollup.
    /// If slot is started in Node context, default witness should be provided
    /// if slot is tarted in Zero Knowledge context, witness from execution should be provided
    fn begin_slot(&mut self, witness: Self::Witness);

    /// Apply a blob/batch of transactions to the rollup, slashing the sequencer who proposed the blob on failure.
    /// The concrete blob type is defined by the DA layer implementation, which is why we use a generic here instead
    /// of an associated type.
    fn apply_blob(
        &mut self,
        blob: impl BlobTransactionTrait,
        misbehavior_hint: Option<Self::MisbehaviorProof>,
    ) -> BatchReceipt<Self::BatchReceiptContents, Self::TxReceiptContents>;

    /// Called once at the *end* of each DA layer block (i.e. after all rollup blob have been processed)
    /// Commits state changes to the database
    fn end_slot(
        &mut self,
    ) -> (
        Self::StateRoot,
        Self::Witness,
        Vec<ConsensusSetUpdate<OpaqueAddress>>,
    );
```

