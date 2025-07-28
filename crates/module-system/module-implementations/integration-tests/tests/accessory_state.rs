use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    AccessoryStateValue, Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, Spec,
    StateAccessor, TxState,
};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_optimistic_runtime, get_gas_used, AsUser};

type S = sov_test_utils::TestSpec;

#[derive(
    Debug,
    PartialEq,
    Eq,
    Clone,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
#[serde(rename_all = "snake_case")]
pub enum CallMessage {
    SetAccessoryValue(u32),
    Nop(u32),
}

#[derive(ModuleInfo, Clone)]
pub struct TestAccessoryModule<S: Spec> {
    #[id]
    id: ModuleId,

    #[state]
    accessory_state: AccessoryStateValue<u32>,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for TestAccessoryModule<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = CallMessage;
    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,
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
            CallMessage::SetAccessoryValue(value) => {
                let unmetered_state = &mut state.to_unmetered();

                self.accessory_state
                    .set(&value, unmetered_state)
                    .unwrap_infallible();

                Ok(())
            }
            CallMessage::Nop(_) => Ok(()),
        }
    }
}

generate_optimistic_runtime!(TestAccessoryRuntime <= accessory_module: TestAccessoryModule<S>);
type RT = TestAccessoryRuntime<S>;

/// Check that:
/// 1. Accessory state does not change normal state root hash.
/// 2. Accessory state is reverted together with normal state.
///
/// Changes are returned explicitly by storage trait.
#[test]
fn test_accessory_value_setter() {
    // Generate a genesis config, then overwrite the attester key/address with ones that
    // we know. We leave the other values untouched.
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let user = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    // Run genesis registering the attester and sequencer we've generated.
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), ());

    let mut runner = TestRunner::<_, _>::new_with_genesis(
        genesis.into_genesis_params(),
        TestAccessoryRuntime::default(),
    );

    let (result_with_update, _, _) = runner.simulate(
        user.create_plain_message::<RT, TestAccessoryModule<S>>(CallMessage::SetAccessoryValue(42)),
    );

    let gas_consumed_with_update =
        get_gas_used(&result_with_update.batch_receipts[0].tx_receipts[0]);

    let (result_without_update, _, _) = runner
        .simulate(user.create_plain_message::<RT, TestAccessoryModule<S>>(CallMessage::Nop(42)));

    let gas_consumed_without_update =
        get_gas_used(&result_without_update.batch_receipts[0].tx_receipts[0]);

    assert_eq!(
        gas_consumed_with_update, gas_consumed_without_update,
        "Gas consumption has been changed by accessory writes"
    );

    // TODO: this test used to check root_hash_with_update and root_hash_without_update and
    // assert_eq! on them.
    // This is no longer possible since the switch from nonces to generations, as the tx hash is
    // stored inside sov_uniqueness thus altering the state root hash anyway.
    // See https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2189
}
