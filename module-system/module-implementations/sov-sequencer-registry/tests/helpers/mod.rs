use jsonrpsee::core::RpcResult;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::digest::Digest;
use sov_modules_api::{Address, Module, Spec, WorkingSet};
use sov_rollup_interface::mocks::{MockAddress, MockDaSpec};
use sov_sequencer_registry::{SequencerConfig, SequencerRegistry};

pub type C = DefaultContext;
pub type Da = MockDaSpec;

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

    pub registry: SequencerRegistry<C, Da>,
    pub sequencer_config: SequencerConfig<C, Da>,
}

impl TestSequencer {
    pub fn genesis(&mut self, working_set: &mut WorkingSet<C>) {
        self.bank.genesis(&self.bank_config, working_set).unwrap();

        self.registry
            .genesis(&self.sequencer_config, working_set)
            .unwrap();
    }

    #[allow(dead_code)]
    pub fn query_balance_via_bank(
        &mut self,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<sov_bank::BalanceResponse> {
        self.bank.balance_of(
            self.sequencer_config.seq_rollup_address,
            self.sequencer_config.coins_to_lock.token_address,
            working_set,
        )
    }

    #[allow(dead_code)]
    pub fn query_balance(
        &mut self,
        user_address: <DefaultContext as Spec>::Address,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<sov_bank::BalanceResponse> {
        self.bank.balance_of(
            user_address,
            self.sequencer_config.coins_to_lock.token_address,
            working_set,
        )
    }
}

pub fn create_bank_config() -> (sov_bank::BankConfig<C>, <C as Spec>::Address) {
    let seq_address = generate_address(GENESIS_SEQUENCER_KEY);

    let token_config = sov_bank::TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances: vec![
            (seq_address, INITIAL_BALANCE),
            (generate_address(ANOTHER_SEQUENCER_KEY), INITIAL_BALANCE),
            (generate_address(UNKNOWN_SEQUENCER_KEY), INITIAL_BALANCE),
            (generate_address(LOW_FUND_KEY), 3),
        ],
        authorized_minters: vec![],
        salt: 8,
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
) -> SequencerConfig<C, Da> {
    SequencerConfig {
        seq_rollup_address,
        seq_da_address: MockAddress::from(GENESIS_SEQUENCER_DA_ADDRESS),
        coins_to_lock: sov_bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
        is_preferred_sequencer: false,
    }
}

pub fn create_test_sequencer() -> TestSequencer {
    let bank = sov_bank::Bank::<C>::default();
    let (bank_config, seq_rollup_address) = create_bank_config();

    let token_address = sov_bank::get_genesis_token_address::<C>(
        &bank_config.tokens[0].token_name,
        bank_config.tokens[0].salt,
    );

    let registry = SequencerRegistry::<C, Da>::default();
    let sequencer_config = create_sequencer_config(seq_rollup_address, token_address);

    TestSequencer {
        bank,
        bank_config,
        registry,
        sequencer_config,
    }
}

pub fn generate_address(key: &str) -> <C as Spec>::Address {
    let hash: [u8; 32] = <C as Spec>::Hasher::digest(key.as_bytes()).into();
    Address::from(hash)
}
