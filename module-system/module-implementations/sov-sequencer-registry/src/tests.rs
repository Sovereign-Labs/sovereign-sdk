use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::ApplyBlobTxHooks;
use sov_modules_api::Hasher;
use sov_modules_api::{Address, Module, ModuleInfo, Spec};
use sov_state::{ProverStorage, WorkingSet};

use crate::query;
use crate::{Sequencer, SequencerConfig};

type C = DefaultContext;

const SEQUENCER_DA_ADDRESS: [u8; 32] = [0; 32];
const INITIAL_BALANCE: u64 = 201;
const LOCKED_AMOUNT: u64 = 200;

struct TestSequencer {
    bank: sov_bank::Bank<C>,
    bank_config: sov_bank::BankConfig<C>,

    sequencer: Sequencer<C>,
    sequencer_config: SequencerConfig<C>,
}

impl TestSequencer {
    fn geneses(&mut self, working_set: &mut WorkingSet<<C as Spec>::Storage>) {
        self.bank.genesis(&self.bank_config, working_set).unwrap();

        self.sequencer
            .genesis(&self.sequencer_config, working_set)
            .unwrap();
    }
    fn query_balance_via_sequencer(
        &self,
        working_set: &mut WorkingSet<<C as Spec>::Storage>,
    ) -> query::SequencerAndBalanceResponse {
        self.sequencer.sequencer_address_and_balance(working_set)
    }

    fn query_balance_via_bank(
        &mut self,
        working_set: &mut WorkingSet<<C as Spec>::Storage>,
    ) -> sov_bank::query::BalanceResponse {
        self.bank.balance_of(
            self.sequencer_config.seq_rollup_address.clone(),
            self.sequencer_config.coins_to_lock.token_address.clone(),
            working_set,
        )
    }
}

fn create_bank_config() -> (sov_bank::BankConfig<C>, <C as Spec>::Address) {
    let seq_address = generate_address("seq_pub_key");

    let token_config = sov_bank::TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances: vec![(seq_address.clone(), INITIAL_BALANCE)],
    };

    (
        sov_bank::BankConfig {
            tokens: vec![token_config],
        },
        seq_address,
    )
}

fn create_sequencer_config(
    seq_rollup_address: <C as Spec>::Address,
    token_address: <C as Spec>::Address,
) -> SequencerConfig<C> {
    SequencerConfig {
        seq_rollup_address,
        seq_da_address: SEQUENCER_DA_ADDRESS.to_vec(),
        coins_to_lock: sov_bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    }
}

fn create_test_sequencer() -> TestSequencer {
    let bank = sov_bank::Bank::<C>::new();
    let (bank_config, seq_rollup_address) = create_bank_config();

    let token_address = sov_bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &sov_bank::genesis::DEPLOYER,
        sov_bank::genesis::SALT,
    );

    let sequencer = Sequencer::<C>::new();
    let sequencer_config = create_sequencer_config(seq_rollup_address, token_address);

    TestSequencer {
        bank,
        bank_config,
        sequencer,
        sequencer_config,
    }
}

#[test]
fn test_sequencer() {
    let mut test_sequencer = create_test_sequencer();
    let working_set = &mut WorkingSet::new(ProverStorage::temporary());
    test_sequencer.geneses(working_set);

    {
        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE, resp.data.unwrap().balance);
    }

    // Lock
    {
        test_sequencer
            .sequencer
            .enter_apply_blob(&SEQUENCER_DA_ADDRESS, working_set)
            .unwrap();

        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, resp.data.unwrap().balance);
    }

    // Reward
    {
        test_sequencer
            .sequencer
            .exit_apply_blob(0, working_set)
            .unwrap();
        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE, resp.data.unwrap().balance);
    }
}

pub fn generate_address(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    Address::from(hash)
}
