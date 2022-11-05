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
            Vec<(
                <<Self as Prover>::Da as Da>::Blob,
                <<Self as Prover>::Da as Da>::InclusionProof,
            )>,
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

trait DataBlob {
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

    fn height(&self) -> u64;
    fn hash(&self) -> Self::Hash;
}
/// A data availability layer
trait Da {
    type BlockHash: PartialEq + Debug;
    type Header: Header<Hash = Self::BlockHash>;
    /// A proof that a particular blob of data is included in the Da::Header
    type InclusionProof;
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
        potential_blocks: Vec<(Self::Blob, Self::InclusionProof)>,
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
    fn validate_opaque_blob(blob: &Self::DataBlob, prev_state: Self::StateRoot) -> bool;

    /// Deserialize a valid blob into a block. Accept an optional proof of misbehavior (for example, an invalid signature)
    /// to short-circuit the block application, returning a new stateroot to account for the slashing of the sequencer
    fn prepare_block(
        blob: Self::DataBlob,
        prev_state: Self::StateRoot,
        misbehavior_hint: Option<Self::Misbehavior>,
    ) -> Result<Self::Block, Self::StateRoot>;

    /// Apply a block
    fn apply_block(blk: Self::Block, prev_state: Self::StateRoot) -> Self::StateRoot;
}

pub trait Proof {
    type VerificationError: std::fmt::Debug;
    type MethodId;

    fn get_log(&self) -> &[u8];
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
    fn blocks_applied(&self) -> &[<<Self as ExecutionProof>::Rollup as Stf>::Block];
    fn pre_state_root(&self) -> <<Self as ExecutionProof>::Rollup as Stf>::StateRoot;
    // returns the state root after applying
    fn post_state_root(&self) -> <<Self as ExecutionProof>::Rollup as Stf>::StateRoot;
}

trait ChainProver {}

trait Chain {
    type DataBlob: DataBlob;
    type DaLayer: Da<Blob = Self::DataBlob>;
    type Rollup: Stf<DataBlob = Self::DataBlob>;
    type ChainProof: ChainProof<DaLayer = Self::DaLayer, Rollup = Self::Rollup>;
    type ExecutionProof: ExecutionProof<Rollup = Self::Rollup>;
    // Verifies that prev_head.da_hash is Da_header.prev_hash
    // calculates the set of sequencers, potentially using the previous rollup state
    // and returns an array of rollup blocks from those sequencers
    // #[Risc0::magic]
    fn extend_da_chain(
        prev_head: Self::ChainProof,
        header: <<Self as Chain>::DaLayer as Da>::Header,
        rollup_namespace_data: &[<<Self as Chain>::DaLayer as Da>::InclusionProof],
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
        let latest_state_commitment = prev_head.state_root();
        let filtered_blocks = blocks
            .into_iter()
            .filter(|blob| {
                Self::Rollup::validate_opaque_blob(blob, latest_state_commitment.clone())
            })
            .collect();

        return Ok(filtered_blocks);
    }

    fn apply_rollup_blocks(
        blks: Vec<<Self::Rollup as Stf>::Block>,
        prev_root: <Self::Rollup as Stf>::StateRoot,
    ) -> <Self::Rollup as Stf>::StateRoot {
        let mut root = prev_root;
        for blk in blks.into_iter() {
            root = <Self::Rollup as Stf>::apply_block(blk, root)
        }
        return root;
    }

    fn execute_or_verify_stf(
        blobs: Vec<Self::DataBlob>,
        prev_root: <Self::Rollup as Stf>::StateRoot,
    ) -> <Self::Rollup as Stf>::StateRoot {
        let mut current_root = prev_root;
        let mut blocks = Vec::new();
        // Deserialize each blob. If deserializing fails, slash the sequencer and update the current state root
        for blob in blobs.into_iter() {
            let misbehavior_hint = env::read_unchecked();
            match Self::Rollup::prepare_block(blob, current_root.clone(), misbehavior_hint) {
                Ok(blk) => blocks.push(blk),
                Err(root) => current_root = root,
            };
        }

        let pf: Option<Self::ExecutionProof> = env::read_unchecked();
        if let Some(proof) = pf {
            assert_eq!(proof.blocks_applied(), blocks);
            assert_eq!(proof.pre_state_root(), current_root);
            proof.verify().expect("proof must be valid");
            return proof.post_state_root();
        }
        return Self::apply_rollup_blocks(blocks, current_root);
    }

    ///
    ///
    // #[risc0::method]
    fn extend_chain(
        prev_head: Self::ChainProof,
        da_header: <Self::DaLayer as Da>::Header,
        rollup_namespace_data: &[<Self::DaLayer as Da>::InclusionProof],
    ) -> (
        <Self::DaLayer as Da>::BlockHash,
        <Self::Rollup as Stf>::StateRoot,
    ) {
        let prev_root = prev_head.state_root();
        let da_hash = da_header.hash();
        let rollup_blocks = Self::extend_da_chain(prev_head, da_header, rollup_namespace_data)
            .expect("Prover must be honest");
        let output_root = Self::execute_or_verify_stf(rollup_blocks, prev_root);
        return (da_hash, output_root);
    }
}
