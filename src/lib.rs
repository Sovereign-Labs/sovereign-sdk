use std::fmt::Debug;

mod env {
    pub fn read_unchecked<T>() -> T {
        todo!()
    }
}

trait Prover {
    type Chain;
    type Da: Da;
    type Stf: Stf;
    type Error;
    /// Gets a list of the potential rollup blocks at a particular DA layer block height. For example, this method could
    /// reach out to a sidecar Celestia node via RPC to get a list of all of the PFDs in the rollup namespace
    fn get_potential_block_list(
        da_height: u64,
    ) -> Result<
        (
            Vec<<<Self as Prover>::Da as Da>::BlobWithInclusionProof>,
            <<Self as Prover>::Da as Da>::CompletenessProof,
        ),
        Self::Error,
    >;

    // TODO: Be sure to call `self.stf.validate_opaque_blob` on each potential block

    /// Validate the contents of the blob without reference to the current state of the blockchain.
    /// This probably just means checking that the block deserializes and that all of the signatures are valid.
    fn check_for_misbehavior(
        blob: <<Self as Prover>::Da as Da>::Blob,
    ) -> Option<<<Self as Prover>::Stf as Stf>::Misbehavior>;
}

trait DataBlob: PartialEq + Debug {
    type Metadata;
    // type Data = Vec<u8>;
    type Data;
    fn data(&self) -> &Self::Data;
    /// The metadata associated with a blob of data destined for the rollup. For example, this could be the sender address
    /// of a blob transaction on Ethereum/Celestia
    /// TODO: Consider making this "sender" instead of "metadata"
    fn metadata(&self) -> &Self::Metadata;
}

trait StateCommitment: PartialEq + Debug + Clone {
    type Key;
    type Value;
    fn get(key: Self::Key) -> Self::Value;
    fn put(key: Self::Key, value: Self::Value) -> Self;
}
trait Header: PartialEq {
    type Hash;
    /// Get the block height at which this header appears
    fn height(&self) -> u64;
    /// Get the  hash of this header
    fn hash(&self) -> Self::Hash;
}

/// A proof that a blob of data is contained in a particular block
///
/// The canonical example is a merkle proof of a pay-for-data transaction on Celestia
trait InclusionProof {
    type Data: DataBlob;
    type BlockHash: PartialEq + Debug;
    type Error;
    /// Verify this inclusion proof against a blockhash.
    fn verify(self, blockhash: &Self::BlockHash) -> Result<Self::Data, Self::Error>;
}
/// A data availability layer
trait Da {
    type BlockHash: PartialEq + Debug;
    type Header: Header<Hash = Self::BlockHash>;
    /// A proof that a particular blob of data is included in the Da::Header
    type BlobWithInclusionProof: InclusionProof<Data = Self::Blob>;
    type Blob: DataBlob;
    type CompletenessProof;
    type Error: Debug;

    /// verifies that a given list of (potential) rollup blocks is both accurate (all potential blocks in the list really do appear
    /// on the DA layer) and comprehensive (no potential blocks have been excluded)
    /// This check may be rollup-specific, but should be both lightweight and independent of the current *state* of the rollup.
    /// The canonical example of this function is to verify that the provided list includes every data blob from
    /// a particular Celestia namespace.
    ///
    /// This function accepts an additional argument, which can be used to provide auxiliary information needed to demonstrate that the
    /// list is complete.
    // #[risc0::method]
    fn verify_potential_block_list(
        da_header: Self::Header,
        potential_blocks: Vec<Self::BlobWithInclusionProof>,
        completeness_proof: Self::CompletenessProof,
    ) -> Result<Vec<Self::Blob>, Self::Error>;
}
/// A state transition function
trait Stf {
    type Block: PartialEq + Debug;
    type StateRoot: StateCommitment;
    type DataBlob: DataBlob;
    type Misbehavior;
    type Error;

    /// Check that the blob meets the requirements to have its contents inspected in-circuit.
    /// For example, this method might check that the sender is a registered/bonded sequencer.
    ///
    /// This validation should inspect the blob's metadata
    // #[risc0::method]
    fn validate_opaque_blob(blob: &Self::DataBlob, prev_state: &Self::StateRoot) -> bool;

    /// Deserialize a valid blob into a block. Accept an optional proof of misbehavior (for example, an invalid signature)
    /// to short-circuit the block application, returning a new stateroot to account for the slashing of the sequencer
    fn prepare_block(
        blob: Self::DataBlob,
        prev_state: &Self::StateRoot,
        misbehavior_hint: Option<Self::Misbehavior>,
    ) -> Result<Self::Block, Self::StateRoot>;

    /// Apply a block
    fn apply_block(blk: Self::Block, prev_state: &Self::StateRoot) -> Self::StateRoot;
}

/// A succinct proof
pub trait Proof {
    type VerificationError: std::fmt::Debug;
    type MethodId;

    fn authenicated_log(&self) -> &[u8];
    fn verify(&self) -> Result<(), Self::VerificationError>;
}

trait ChainProof: Proof {
    type DaLayer: Da<Blob = Self::DataBlob>;
    type Rollup: Stf<DataBlob = Self::DataBlob>;
    type DataBlob: DataBlob;
    // returns the hash of the latest DA block
    fn da_hash(&self) -> <<Self as ChainProof>::DaLayer as Da>::BlockHash;
    // returns the rollup state root
    fn state_root(&self) -> <<Self as ChainProof>::Rollup as Stf>::StateRoot;
}

trait ExecutionProof: Proof {
    type Rollup: Stf;
    type DaLayer: Da;
    fn blobs_applied(&self) -> &[<<Self as ExecutionProof>::DaLayer as Da>::Blob];
    fn pre_state_root(&self) -> <<Self as ExecutionProof>::Rollup as Stf>::StateRoot;
    // returns the state root after applying
    fn post_state_root(&self) -> <<Self as ExecutionProof>::Rollup as Stf>::StateRoot;
}

pub enum VerificationType<P: Proof> {
    /// A computation which is done immediately
    Inline,
    /// A computation which has already been proven by some other process and should be aggregated into the current execution
    PreProcessed(P),
}

trait Chain {
    type DataBlob: DataBlob;
    type DaLayer: Da<Blob = Self::DataBlob>;
    type Rollup: Stf<DataBlob = Self::DataBlob>;
    type ChainProof: ChainProof<DaLayer = Self::DaLayer, Rollup = Self::Rollup>;
    type ExecutionProof: ExecutionProof<Rollup = Self::Rollup, DaLayer = Self::DaLayer>;
    // Verifies that prev_head.da_hash is Da_header.prev_hash
    // calculates the set of sequencers, potentially using the previous rollup state
    // and returns an array of rollup blocks from those sequencers
    // #[Risc0::magic]
    fn extend_da_chain(
        prev_head: Self::ChainProof,
        header: <<Self as Chain>::DaLayer as Da>::Header,
    ) -> Result<Vec<Self::DataBlob>, <<Self as Chain>::DaLayer as Da>::Error> {
        prev_head.verify().expect("proof must be valid");
        assert_eq!(prev_head.da_hash(), header.hash());
        // hint: get_potential_block_list(header.height())?;
        let (potential_blocks, completeness_proof) = env::read_unchecked();
        let blocks = Self::DaLayer::verify_potential_block_list(
            header,
            potential_blocks,
            completeness_proof,
        )?;

        return Ok(blocks);
    }

    // #[risc0::method]
    fn process_blob(
        blob: Self::DataBlob,
        current_root: <Self::Rollup as Stf>::StateRoot,
    ) -> <Self::Rollup as Stf>::StateRoot {
        if Self::Rollup::validate_opaque_blob(&blob, &current_root) {
            let misbehavior_hint = env::read_unchecked();
            match Self::Rollup::prepare_block(blob, &current_root, misbehavior_hint)
                .map(|block| Self::Rollup::apply_block(block, &current_root))
            {
                Ok(root) => root,
                Err(root) => root,
            }
        } else {
            current_root
        }
    }

    /// Verify the application of the state transition function to every blob in the provided array, starting from the
    /// pre-state root.
    // #[risc0::method]
    fn verify_stf(
        blobs: Vec<Self::DataBlob>,
        prev_root: <Self::Rollup as Stf>::StateRoot,
    ) -> <Self::Rollup as Stf>::StateRoot {
        let mut current_root = prev_root;
        let mut blobs = blobs.into_iter();
        while blobs.len() != 0 {
            let computation_type: VerificationType<Self::ExecutionProof> = env::read_unchecked();
            match computation_type {
                VerificationType::Inline => {
                    current_root = Self::process_blob(
                        blobs
                            .next()
                            .expect("Next blob must exist because of check at top of loop"),
                        current_root,
                    )
                }
                VerificationType::PreProcessed(proof) => {
                    assert_eq!(proof.pre_state_root(), current_root);
                    for blob in proof.blobs_applied() {
                        assert_eq!(Some(blob), blobs.next().as_ref())
                    }
                    proof.verify().expect("proof must be valid");
                    current_root = proof.post_state_root();
                }
            }
        }
        current_root
    }

    ///
    ///
    // #[risc0::method]
    fn extend_chain(
        prev_head: Self::ChainProof,
        da_header: <Self::DaLayer as Da>::Header,
    ) -> (
        <Self::DaLayer as Da>::BlockHash,
        <Self::Rollup as Stf>::StateRoot,
    ) {
        let prev_root = prev_head.state_root();
        let da_hash = da_header.hash();
        let rollup_blocks =
            Self::extend_da_chain(prev_head, da_header).expect("Prover must be honest");
        let output_root = Self::verify_stf(rollup_blocks, prev_root);
        return (da_hash, output_root);
    }
}
