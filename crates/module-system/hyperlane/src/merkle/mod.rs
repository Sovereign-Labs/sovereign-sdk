use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_bank::Amount;
#[cfg(feature = "native")]
use sov_modules_api::prelude::tracing::instrument;
use sov_modules_api::{
    BorrowedMut, Context, DaSpec, EventEmitter, GenesisState, HexHash, HexString, Module, ModuleId,
    ModuleInfo, ModuleRestApi, Spec, StateValue, TxState,
};
use tree::MerkleTree;

use crate::traits::PostDispatchHook;
use crate::types::HookType;
use crate::Message;

#[cfg(feature = "native")]
mod api;
mod tree;

/// A helper modules which merklizes each message as it is dispatched.
// Note: In a future iteration, we may consider moving this into the Mailbox module.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct MerkleTreeHook<S: Spec> {
    /// Module identifier.
    #[id]
    pub id: ModuleId,

    /// Owners of the hooks.
    #[state]
    pub tree: StateValue<MerkleTree>,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for MerkleTreeHook<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = ();
    type Event = Event;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.tree.set(&MerkleTree::default(), state)?;
        Ok(())
    }

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
/// Events that can be emitted by the Merkle module.
#[derive(
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Clone,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Emitted when a new element is inserted into the tree.
    InsertedIntoTree {
        /// The hash of the inserted element.
        id: HexHash,
        /// The index of the inserted element.
        index: u32,
    },
}

/// Implementation of `PostDispatchHook` for the Merkle tree hooks module.
impl<S: Spec> PostDispatchHook<S> for MerkleTreeHook<S> {
    fn hook_type(
        &self,
        _addr: &S::Address,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<HookType> {
        Ok(HookType::MerkleTree)
    }

    fn supports_metadata(
        &self,
        _metadata: &HexString,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }

    // compare to https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/ff0d4af74ecc586ef0c036e37fa4cf9c2ba5050e/solidity/contracts/hooks/MerkleTreeHook.sol#L63
    #[cfg_attr(feature = "native", instrument(skip(self, _context, state)))]
    fn post_dispatch(
        &mut self,
        message_id: &HexHash,
        _message: &Message,
        _metadata: &HexString,
        _relayer: &S::Address,
        _gas_payment_limit: Amount,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let mut tree: BorrowedMut<MerkleTree, _> =
            self.tree.borrow_mut(state)?.unwrap_or(Default::default());
        let index = tree.count;

        tree.insert(*message_id, state)?;
        tree.save(state)?;

        self.emit_event(
            state,
            Event::InsertedIntoTree {
                index,
                id: *message_id,
            },
        );

        Ok(())
    }

    fn quote_dispatch(
        &self,
        _message: &Message,
        _metadata: &HexString,
        _relayer: &S::Address,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<Amount> {
        Ok(Amount::ZERO)
    }
}
