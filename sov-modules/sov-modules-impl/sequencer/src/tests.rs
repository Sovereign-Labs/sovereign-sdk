use sov_modules_api::mocks::MockContext;
use sov_modules_api::{Module, ModuleInfo, PublicKey, Spec};
use sov_state::{ProverStorage, WorkingSet};

use crate::hooks::Hooks;
use crate::query;
use crate::{Sequencer, SequencerConfig};

type C = MockContext;

const SEQUENCER_DA_ADDRESS: [u8; 32] = [0; 32];
const INITIAL_BALANCE: u64 = 201;
const LOCKED_AMOUNT: u64 = 200;

struct TestSequencer {
    bank: bank::Bank<C>,
    bank_config: bank::BankConfig<C>,

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
        let query = query::QueryMessage::GetSequencerAddressAndBalance;
        let resp = self.sequencer.query(query, working_set);

        serde_json::from_slice(&resp.response).unwrap()
    }

    fn query_balance_via_bank(
        &mut self,
        working_set: &mut WorkingSet<<C as Spec>::Storage>,
    ) -> bank::query::BalanceResponse {
        let query = bank::query::QueryMessage::GetBalance {
            user_address: self.sequencer_config.seq_rollup_address.clone(),
            token_address: self.sequencer_config.coins_to_lock.token_address.clone(),
        };

        let resp = self.bank.query(query, working_set);
        serde_json::from_slice(&resp.response).unwrap()
    }
}

fn create_bank_config() -> (bank::BankConfig<C>, <C as Spec>::Address) {
    let pub_key = <C as Spec>::PublicKey::try_from("seq_pub_key").unwrap();
    let seq_address = pub_key.to_address::<<C as Spec>::Address>();

    let token_config = bank::TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances: vec![(seq_address.clone(), INITIAL_BALANCE)],
    };

    (
        bank::BankConfig {
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
        coins_to_lock: bank::Coins {
            amount: LOCKED_AMOUNT,
            token_address,
        },
    }
}

fn create_test_sequencer() -> TestSequencer {
    let bank = bank::Bank::<C>::new();
    let (bank_config, seq_rollup_address) = create_bank_config();

    let token_address = bank::create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &bank::genesis::DEPLOYER,
        bank::genesis::SALT,
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

    let hooks = Hooks::<C>::new();

    assert_eq!(
        SEQUENCER_DA_ADDRESS.to_vec(),
        hooks.next_sequencer(working_set).unwrap()
    );

    {
        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());
    }

    // Lock
    {
        hooks.lock(working_set).unwrap();

        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE - LOCKED_AMOUNT, resp.amount.unwrap());
    }

    // Reward
    {
        hooks.reward(0, working_set).unwrap();
        let resp = test_sequencer.query_balance_via_bank(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());

        let resp = test_sequencer.query_balance_via_sequencer(working_set);
        assert_eq!(INITIAL_BALANCE, resp.amount.unwrap());
    }
}
