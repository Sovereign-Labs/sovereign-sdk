use crate::runtime::{GenesisConfig, Runtime};
use crate::tx_hooks_impl::DemoAppTxHooks;
use crate::tx_verifier_impl::DemoAppTxVerifier;
use sov_app_template::AppTemplate;
use sov_modules_api::mocks::MockContext;
use sov_modules_api::Context;
#[cfg(test)]
use sov_modules_api::{PublicKey, Spec};
#[cfg(test)]
use sov_state::ProverStorage;
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
pub(crate) type C = MockContext;
pub type DemoApp<C> =
    AppTemplate<C, DemoAppTxVerifier<C>, Runtime<C>, DemoAppTxHooks<C>, GenesisConfig<C>>;

pub const SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
pub const LOCKED_AMOUNT: u64 = 200;
pub const SEQ_PUB_KEY_STR: &str = "seq_pub_key";
pub const TOKEN_NAME: &str = "Token0";

#[cfg(test)]
pub(crate) fn create_sequencer_config<C: Context>(
    seq_rollup_address: <C as Spec>::Address,
    token_address: <C as Spec>::Address,
) -> sequencer::SequencerConfig<C> {
    sequencer::SequencerConfig {
        seq_rollup_address,
        seq_da_address: SEQUENCER_DA_ADDRESS.to_vec(),
        coins_to_lock: bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    }
}

#[cfg(test)]
pub(crate) fn create_config(initial_sequencer_balance: u64) -> GenesisConfig<C> {
    let pub_key = <C as Spec>::PublicKey::try_from(SEQ_PUB_KEY_STR).unwrap();
    let seq_address = pub_key.to_address::<<C as Spec>::Address>();

    let token_config = bank::TokenConfig {
        token_name: TOKEN_NAME.to_owned(),
        address_and_balances: vec![(seq_address.clone(), initial_sequencer_balance)],
    };

    let bank_config = bank::BankConfig {
        tokens: vec![token_config],
    };

    let token_address = bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &bank::genesis::DEPLOYER,
        bank::genesis::SALT,
    );

    let sequencer_config = create_sequencer_config(seq_address, token_address);

    GenesisConfig::new(
        sequencer_config,
        bank_config,
        (),
        (),
        accounts::AccountConfig { pub_keys: vec![] },
    )
}

#[cfg(test)]
pub(crate) fn create_new_demo(
    initial_sequencer_balance: u64,
    path: impl AsRef<Path>,
) -> DemoApp<C> {
    let runtime = Runtime::new();
    let storage = ProverStorage::with_path(path).unwrap();
    let tx_hooks = DemoAppTxHooks::new();
    let tx_verifier = DemoAppTxVerifier::new();
    let genesis_config = create_config(initial_sequencer_balance);
    AppTemplate::new(storage, runtime, tx_verifier, tx_hooks, genesis_config)
}
