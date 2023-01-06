use std::future::Future;

use crate::serial::{Deser, Serialize};

// TODO: Rename to clarify distinction with DaApp
/// A DaService is the local side of an RPC connection talking to node of the DA layer
/// It is *not* part of the logic that is zk-proven. Rather, it provides functionality
/// to allow the sovereign SDK to interact with the DA layer's RPC network.
pub trait DaService {
    /// An L1 block, possibly excluding some irrelevant information
    type FilteredBlock: SlotData;
    type Future<T>: Future<Output = Result<T, Self::Error>>;
    // /// A transaction on the L1
    // type Transaction;
    // type Address;
    type Error;

    /// Retrieve the data for the given height, waiting for it to be
    /// finalized if necessary. The block, once returned, must not be reverted
    /// without a consensus violation.
    fn get_finalized_at(height: usize) -> Self::Future<Self::FilteredBlock>;

    /// Get the block at the given height, waiting for one to be mined if necessary.
    /// The returned block may not be final, and can be reverted without a consensus violation
    fn get_block_at(height: usize) -> Self::Future<Self::FilteredBlock>;

    // TODO: Consider adding the send_transaction method
    // fn send_transaction(tx: Self::Transaction, sender: Self::Address)
}
pub trait SlotData: Serialize + Deser + PartialEq {}
