//! This module defines the traits that are used by the full node to
//! instantiate and run the state transition function.
use crate::da::BlobReaderTrait;
use crate::services::batch_builder::BatchBuilder;
use crate::stf::{StateTransitionConfig, StateTransitionFunction};
use crate::zk::Zkvm;

/// A StateTransitionRunner (STR) is responsible for running the state transition function. For any particular function,
/// you might have a few different STRs, each with different runtime configs. For example, you might have a STR which takes
/// a path to a data directory as a runtime config, and another which takes a pre-built in-memory database.
///
/// Using a separate trait for initialization makes it easy to store extra data in the STR, which
/// would not fit neatly in the state transition logic itself (such as a handle to the database).
/// This way, you can easily support ancillary functions like RPC, p2p networking etc in your full node implementation
///
///
/// The StateTransitionRunner is generic over a StateTransitionConfig, and a Zkvm. The ZKvm is simply forwarded to the inner STF.
/// StateTransitionConfig is a special marker trait which has only 3 possible instantiations:  ProverConfig, NativeConfig, and ZkConfig.
/// This Config makes it easy to implement different instantiations of STR on the same struct, which are appropriate for different
/// modes of execution.
///
/// For example: might have `impl StateTransitionRunner<ProverConfig, Vm> for MyRunner` which takes a path to a data directory as a runtime config,
///
/// and a `impl StateTransitionRunner<ZkConfig, Vm> for MyRunner` which instead uses a state root as its runtime config.
///
/// TODO: Why is it called runner? It only creates. Creator, Factory: <https://github.com/Sovereign-Labs/sovereign-sdk/issues/447>
pub trait StateTransitionRunner<T: StateTransitionConfig, Vm: Zkvm, B: BlobReaderTrait> {
    /// The parameters of the state transition function which are set at runtime. For example,
    /// the runtime config might contain path to a data directory.
    type RuntimeConfig;

    /// The inner [`StateTransitionFunction`] which is being run.
    type Inner: StateTransitionFunction<Vm, B>;

    /// The [`BatchBuilder`] accepts transactions from the mempool and returns bundles of transactions
    /// on request from the full node.
    type BatchBuilder: BatchBuilder;

    // TODO: decide if `new` also requires <Self as StateTransitionFunction>::ChainParams as an argument
    /// Creates a [`StateTransitionRunner`] from the given runtime config.
    fn new(runtime_config: Self::RuntimeConfig) -> Self;

    /// Return a reference to the inner STF implementation
    fn inner(&self) -> &Self::Inner;

    /// Return a mutable reference to the inner STF implementation
    fn inner_mut(&mut self) -> &mut Self::Inner;

    /// Gives batch builder, after it has been initialized and configured
    /// Can be only called once.
    fn take_batch_builder(&mut self) -> Option<Self::BatchBuilder>;

    // /// Report if the state transition function has been initialized.
    // /// If not, node implementations should respond by running `init_chain`
    // fn has_been_initialized(&self) -> bool;
}
