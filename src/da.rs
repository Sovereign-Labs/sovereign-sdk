use crate::{Header, InclusionProof};
use core::fmt::Debug;

pub trait Blob: PartialEq + Debug {
    type Metadata;
    // type Data = Vec<u8>;
    type Data;
    fn data(&self) -> &Self::Data;
    /// The metadata associated with a blob of data destined for the rollup. For example, this could be the sender address
    /// of a blob transaction on Ethereum/Celestia
    /// TODO: Consider making this "sender" instead of "metadata"
    fn metadata(&self) -> &Self::Metadata;
}

/// A data availability layer
pub trait Da {
    type BlockHash: PartialEq + Debug;
    type Header: Header<Hash = Self::BlockHash>;
    type Block;
    /// A proof that a particular blob of data is included in the Da::Header
    type SignedDataWithInclusionProof: InclusionProof<SignedData = Self::SignedData>;
    type SignedData: Blob;
    type CompletenessProof;
    type Error: Debug;
    type Qualifier;

   
    fn get_transactions_by_destination(block: Self::Block, qualifier: Qualifier) -> Vec<SignedData>;

//     /// verifies that a given list of (potential) rollup blocks is both accurate (all potential blocks in the list really do appear
//     /// on the DA layer) and comprehensive (no potential blocks have been excluded)
//     /// This check may be rollup-specific, but should be both lightweight and independent of the current *state* of the rollup.
//     /// The canonical example of this function is to verify that the provided list includes every data blob from
//     /// a particular Celestia namespace.
//     ///
//     /// This function accepts an additional argument, which can be used to provide auxiliary information needed to demonstrate that the
//     /// list is complete.
//     // #[risc0::method]
//     fn verify_potential_block_list(
//         da_header: Self::Header,
//         potential_blocks: Vec<Self::SignedDataWithInclusionProof>,
//         completeness_proof: Self::CompletenessProof,
//     ) -> Result<Vec<Self::SignedData>, Self::Error>;
// }
