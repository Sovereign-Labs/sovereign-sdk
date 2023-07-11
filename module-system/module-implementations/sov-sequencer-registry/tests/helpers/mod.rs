use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Hasher, Module, Spec};
use sov_sequencer_registry::{SequencerConfig, SequencerRegistry};
use sov_state::WorkingSet;

pub type C = DefaultContext;

pub const GENESIS_SEQUENCER_KEY: &str = "sequencer_1";
pub const GENESIS_SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
pub const ANOTHER_SEQUENCER_KEY: &str = "sequencer_2";
#[allow(dead_code)]
pub const ANOTHER_SEQUENCER_DA_ADDRESS: [u8; 32] = [2; 32];
pub const UNKNOWN_SEQUENCER_KEY: &str = "sequencer_3";
#[allow(dead_code)]
pub const UNKNOWN_SEQUENCER_DA_ADDRESS: [u8; 32] = [3; 32];
pub const LOW_FUND_KEY: &str = "zero_funds";
pub const INITIAL_BALANCE: u64 = 210;
pub const LOCKED_AMOUNT: u64 = 200;

pub struct TestSequencer {
    pub bank: sov_bank::Bank<C>,
    pub bank_config: sov_bank::BankConfig<C>,

    pub registry: SequencerRegistry<C>,
    pub sequencer_config: SequencerConfig<C>,
}

impl TestSequencer {
    pub fn genesis(&mut self, working_set: &mut WorkingSet<<C as Spec>::Storage>) {
        self.bank.genesis(&self.bank_config, working_set).unwrap();

        self.registry
            .genesis(&self.sequencer_config, working_set)
            .unwrap();
    }

    #[allow(dead_code)]
    pub fn query_balance_via_bank(
        &mut self,
        working_set: &mut WorkingSet<<C as Spec>::Storage>,
    ) -> sov_bank::query::BalanceResponse {
        self.bank.balance_of(
            self.sequencer_config.seq_rollup_address.clone(),
            self.sequencer_config.coins_to_lock.token_address.clone(),
            working_set,
        )
    }

    #[allow(dead_code)]
    pub fn query_balance(
        &mut self,
        user_address: <DefaultContext as Spec>::Address,
        working_set: &mut WorkingSet<<C as Spec>::Storage>,
    ) -> sov_bank::query::BalanceResponse {
        self.bank.balance_of(
            user_address,
            self.sequencer_config.coins_to_lock.token_address.clone(),
            working_set,
        )
    }
}

pub fn create_bank_config() -> (sov_bank::BankConfig<C>, <C as Spec>::Address) {
    let seq_address = generate_address(GENESIS_SEQUENCER_KEY);

    let token_config = sov_bank::TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances: vec![
            (seq_address.clone(), INITIAL_BALANCE),
            (generate_address(ANOTHER_SEQUENCER_KEY), INITIAL_BALANCE),
            (generate_address(UNKNOWN_SEQUENCER_KEY), INITIAL_BALANCE),
            (generate_address(LOW_FUND_KEY), 3),
        ],
    };

    (
        sov_bank::BankConfig {
            tokens: vec![token_config],
        },
        seq_address,
    )
}

pub fn create_sequencer_config(
    seq_rollup_address: <C as Spec>::Address,
    token_address: <C as Spec>::Address,
) -> SequencerConfig<C> {
    SequencerConfig {
        seq_rollup_address,
        seq_da_address: GENESIS_SEQUENCER_DA_ADDRESS.to_vec(),
        coins_to_lock: sov_bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    }
}

pub fn create_test_sequencer() -> TestSequencer {
    let bank = sov_bank::Bank::<C>::default();
    let (bank_config, seq_rollup_address) = create_bank_config();

    let token_address = sov_bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &sov_bank::genesis::DEPLOYER,
        sov_bank::genesis::SALT,
    );

    let registry = SequencerRegistry::<C>::default();
    let sequencer_config = create_sequencer_config(seq_rollup_address, token_address);

    TestSequencer {
        bank,
        bank_config,
        registry,
        sequencer_config,
    }
}

pub fn generate_address(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    Address::from(hash)
}
