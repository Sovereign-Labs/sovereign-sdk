use std::fmt::Debug;

mod env {
    pub fn read_unchecked<T>() -> T {
        todo!()
    }
}
pub trait DataBlob: PartialEq + Debug {
    // type Data = Vec<u8>;
    fn data(&self) -> &[u8];
    /// The metadata associated with a blob of data destined for the rollup. For example, this could be the sender address
    /// of a blob transaction on Ethereum/Celestia
    /// TODO: Consider making this "sender" instead of "metadata"
    fn sender(&self) -> &[u8];
}

pub trait StateCommitment: PartialEq + Debug + Clone {
    type Key;
    type Value;
    fn get(key: Self::Key) -> Self::Value;
    fn put(key: Self::Key, value: Self::Value) -> Self;
}
pub trait Header: PartialEq {
    type Hash;
    /// Get the block height at which this header appears
    fn height(&self) -> u64;
    /// Get the  hash of this header
    fn hash(&self) -> Self::Hash;
}

/// A proof that a blob of data is contained in a particular block
///
/// The canonical example is a merkle proof of a pay-for-data transaction on Celestia
pub trait InclusionProof {
    type SignedData: DataBlob;
    type BlockHash: PartialEq + Debug;
    type Error;
    /// Verify this inclusion proof against a blockhash.
    fn verify(self, blockhash: &Self::BlockHash) -> Result<Self::SignedData, Self::Error>;
}
/// A data availability layer
pub trait DataLayer {
    type BlockHash: PartialEq + Debug;
    type Block;
    type Header: Header<Hash = Self::BlockHash>;
    /// A proof that a particular blob of data is included in the Da::Header
    type SignedDataWithInclusionProof: InclusionProof<SignedData = Self::SignedData>;
    type SignedData: DataBlob;
    type CompletenessProof;
    type Error: Debug;
    /// Get all the data transactions from a block, with some qualifier. The qualifier could
    /// be a destination address (for Ethereum calldata), a target namespace (on chains like Celestia),
    /// or even a sender address
    type Qualifier;

    fn get_relevant_transactions(&self, block: Self::Block) -> Vec<Self::SignedData>;

    // /// verifies that a given list of (potential) rollup blocks is both accurate (all potential blocks in the list really do appear
    // /// on the DA layer) and comprehensive (no potential blocks have been excluded)
    // /// This check may be rollup-specific, but should be both lightweight and independent of the current *state* of the rollup.
    // /// The canonical example of this function is to verify that the provided list includes every data blob from
    // /// a particular Celestia namespace.
    // ///
    // /// This function accepts an additional argument, which can be used to provide auxiliary information needed to demonstrate that the
    // /// list is complete.
    // // #[risc0::method]
    // fn verify_potential_block_list(
    //     da_header: Self::Header,
    //     potential_blocks: Vec<Self::SignedDataWithInclusionProof>,
    //     completeness_proof: Self::CompletenessProof,
    // ) -> Result<Vec<Self::SignedData>, Self::Error>;
}
/// A state transition function
pub trait StateTransition {
    type Block: PartialEq + Debug;
    type StateRoot: StateCommitment;
    type SignedData: DataBlob;
    type Misbehavior;
    type Error;

    // /// Check that the blob meets the requirements to have its contents inspected in-circuit.
    // /// For example, this method might check that the sender is a registered/bonded sequencer.
    // ///
    // /// This validation should inspect the blob's metadata
    // // #[risc0::method]
    // fn validate_opaque_blob(&self, blob: &Self::SignedData, prev_state: &Self::StateRoot) -> bool;

    // /// Deserialize a valid blob into a block. Accept an optional proof of misbehavior (for example, an invalid signature)
    // /// to short-circuit the block application, returning a new stateroot to account for the slashing of the sequencer
    // fn prepare_block(
    //     blob: Self::SignedData,
    //     prev_state: &Self::StateRoot,
    //     misbehavior_hint: Option<Self::Misbehavior>,
    // ) -> Result<Self::Block, Self::StateRoot>;

    // /// Apply a block
    // fn apply_block(blk: Self::Block, prev_state: &Self::StateRoot) -> Self::StateRoot;
    fn process_bundle(&mut self, sender: &[u8], contents: &[u8]);
}

/// A succinct proof
pub trait Proof {
    type VerificationError: std::fmt::Debug;
    type MethodId;
    const MethodId: Self::MethodId;

    fn authenicated_log(&self) -> &[u8];
    fn verify(&self) -> Result<(), Self::VerificationError>;
}

pub trait ChainProof: Proof {
    type DaLayer: DataLayer<SignedData = Self::SignedData>;
    type Rollup: StateTransition<SignedData = Self::SignedData>;
    type SignedData: DataBlob;
    // returns the hash of the latest DA block
    fn da_hash(&self) -> <<Self as ChainProof>::DaLayer as DataLayer>::BlockHash;
    // returns the rollup state root
    fn state_root(&self) -> <<Self as ChainProof>::Rollup as StateTransition>::StateRoot;
}

pub trait ExecutionProof: Proof {
    type Rollup: StateTransition;
    type DaLayer: DataLayer;
    fn blobs_applied(&self) -> &[<<Self as ExecutionProof>::DaLayer as DataLayer>::SignedData];
    fn pre_state_root(&self) -> <<Self as ExecutionProof>::Rollup as StateTransition>::StateRoot;
    // returns the state root after applying
    fn post_state_root(&self) -> <<Self as ExecutionProof>::Rollup as StateTransition>::StateRoot;
}

pub enum VerificationType<P: Proof> {
    /// A computation which is done immediately
    Inline,
    /// A computation which has already been proven by some other process and should be aggregated into the current execution
    PreProcessed(P),
}

pub struct Rollup<Da: DataLayer, Stf: StateTransition> {
    pub stf: Stf,
    pub da: Da,
}

// pub trait Rollup {
//     fn on_da_block(&mut self);
// }
// #[cfg(feature = "prover")]

impl<Da: DataLayer, Stf: StateTransition> Rollup<Da, Stf> {
    fn on_da_block(&mut self, block: <Da as DataLayer>::Block) {
        let transactions = self.da.get_relevant_transactions(block);
        for tx in transactions {
            self.stf.process_bundle(tx.sender(), tx.data())
        }
    }
}

// // Verifies that prev_head.da_hash is Da_header.prev_hash
// //  the set of sequencers, potentially using the previous rollup state
// // and returns an array of rollup blocks from those sequencers
// // #[Risc0::magic]
// fn extend_da_chain(
//     prev_head: Self::ChainProof,
//     header: <<Self as Chain>::DaLayer as DataLayer>::Header,
// ) -> Result<Vec<Self::DataBlob>, <<Self as Chain>::DaLayer as Da>::Error> {
//     prev_head.verify().expect("proof must be valid");
//     assert_eq!(prev_head.da_hash(), header.hash());
//     // hint: get_potential_block_list(header.height())?;
//     let (potential_blocks, completeness_proof) = env::read_unchecked();
//     let blocks = Self::DaLayer::verify_potential_block_list(
//         header,
//         potential_blocks,
//         completeness_proof,
//     )?;

//     return Ok(blocks);
// }

// // #[risc0::method]
// fn process_blob(
//     blob: Self::DataBlob,
//     current_root: <Self::Stf as StateTransition>::StateRoot,
// ) -> <Self::Stf as Stf>::StateRoot {
//     if Self::Rollup::validate_opaque_blob(&blob, &current_root) {
//         let misbehavior_hint = env::read_unchecked();
//         match Self::Rollup::prepare_block(blob, &current_root, misbehavior_hint)
//             .map(|block| Self::Rollup::apply_block(block, &current_root))
//         {
//             Ok(root) => root,
//             Err(root) => root,
//         }
//     } else {
//         current_root
//     }
// }

// /// Verify the application of the state transition function to every blob in the provided array, starting from the
// /// pre-state root.
// // #[risc0::method]
// fn verify_stf(
//     blobs: Vec<Self::DataBlob>,
//     prev_root: <Self::Stf as StateTransition>::StateRoot,
// ) -> <Self::Stf as Stf>::StateRoot {
//     let mut current_root = prev_root;
//     let mut blobs = blobs.into_iter();
//     while blobs.len() != 0 {
//         let computation_type: VerificationType<Self::ExecutionProof> = env::read_unchecked();
//         match computation_type {
//             VerificationType::Inline => {
//                 current_root = Self::process_blob(
//                     blobs
//                         .next()
//                         .expect("Next blob must exist because of check at top of loop"),
//                     current_root,
//                 )
//             }
//             VerificationType::PreProcessed(proof) => {
//                 assert_eq!(proof.pre_state_root(), current_root);
//                 for blob in proof.blobs_applied() {
//                     assert_eq!(Some(blob), blobs.next().as_ref())
//                 }
//                 proof.verify().expect("proof must be valid");
//                 current_root = proof.post_state_root();
//             }
//         }
//     }
//     current_root
// }

// ///
// ///
// // #[risc0::method]
// fn extend_chain(
//     prev_head: Self::ChainProof,
//     da_header: <Self::DaLayer as DataLayer>::Header,
// ) -> (
//     <Self::DaLayer as Da>::BlockHash,
//     <Self::Stf as Stf>::StateRoot,
// ) {
//     let prev_root = prev_head.state_root();
//     let da_hash = da_header.hash();
//     let rollup_blocks =
//         Self::extend_da_chain(prev_head, da_header).expect("Prover must be honest");
//     let output_root = Self::verify_stf(rollup_blocks, prev_root);
//     return (da_hash, output_root);
// }
