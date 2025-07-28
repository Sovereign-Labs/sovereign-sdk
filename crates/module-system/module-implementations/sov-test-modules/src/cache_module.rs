/// A module for testing gas charges
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::macros::UniversalWallet;
// use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
use sov_modules_api::{
    BorshSerializedSize, Context, DaSpec, EventEmitter, GenesisState, Module, ModuleId, ModuleInfo,
    ModuleRestApi, SafeString, Spec, StateValue, TxState,
};

/// A message to test and set a value
#[derive(
    Clone,
    BorshSerialize,
    BorshDeserialize,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub enum CallMessage {
    /// Tests and sets a u8 value
    TestAndSetU8(TestAndSet<u8>),
    /// Tests and sets a u16 value
    TestAndSetU16(TestAndSet<u16>),
    /// Tests and sets a string value
    TestAndSetString(TestAndSet<SafeString>),
    /// Sets a value, but then reverts the tx resulting an a no-op
    SetAndRevertString(Option<SafeString>),
    /// Tests and set a value, then conditionally undoes the changes without reverting.
    TestSetAndMaybeUndo {
        cache_value: TestAndSet<SafeString>,
        state_value: u64,
        undo: bool,
    },
}

/// A message to set a value
#[derive(
    Clone,
    BorshSerialize,
    BorshDeserialize,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub struct TestAndSet<T> {
    pub new_value: Option<T>,
    pub expected_value: Option<T>,
}

impl<T: std::fmt::Debug + PartialEq + Eq + Send + Sync + BorshSerializedSize + 'static>
    TestAndSet<T>
{
    pub fn run<S: Spec>(self, state: &mut impl TxState<S>) -> Result<(), anyhow::Error> {
        let current_value = state.get_cached::<T>();
        if current_value != self.expected_value.as_ref() {
            anyhow::bail!(
                "Wrong value: expected {:?}, got {:?}",
                self.expected_value,
                current_value
            );
        }
        match self.new_value {
            Some(new_value) => {
                state.put_cached(new_value);
            }
            None => {
                state.delete_cached::<T>();
            }
        }
        Ok(())
    }
}

/// A module for testing the block-level cache.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct CacheAndRevertTester<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    #[state]
    pub value: StateValue<u64>,

    #[phantom]
    _phantom: std::marker::PhantomData<S>,
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    BorshSerialize,
    BorshDeserialize,
)]
pub enum Event {
    SetValue(u64),
}

impl<S: Spec> Module for CacheAndRevertTester<S> {
    type Spec = S;

    type Config = ();

    type CallMessage = CallMessage;

    type Event = Event;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::TestAndSetU8(msg) => msg.run(state),
            CallMessage::TestAndSetU16(msg) => msg.run(state),
            CallMessage::TestAndSetString(msg) => msg.run(state),
            CallMessage::SetAndRevertString(msg) => {
                match msg {
                    Some(msg) => state.put_cached(msg),
                    None => state.delete_cached::<String>(),
                }
                Err(anyhow::anyhow!("Reverting"))
            }
            CallMessage::TestSetAndMaybeUndo {
                cache_value,
                state_value,
                undo,
            } => {
                let mut state_wrapped = state.to_revertable();
                let state = &mut state_wrapped;
                cache_value.run(state)?;
                self.value
                    .set(&state_value, state)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                self.emit_event(state, Event::SetValue(state_value));
                if !undo {
                    state_wrapped.commit();
                }
                Ok(())
            }
        }
    }
}
