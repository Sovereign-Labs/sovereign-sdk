use crate::{
    call, create_token_address, genesis,
    query::{self, QueryMessage},
    Bank, BankConfig, Coins, TokenConfig,
};

use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, Module, ModuleInfo, PublicKey, Spec,
};
use sov_state::{ProverStorage, WorkingSet};

type C = MockContext;

struct TestBank {
    bank: Bank<C>,
    bank_config: BankConfig<C>,
    minter_address: <C as Spec>::Address,
    minter_context: C,
    init_token_address: <C as Spec>::Address,
    deployed_token_address: <C as Spec>::Address,
    salt: u64,
    working_set: WorkingSet<<C as Spec>::Storage>,
}

impl TestBank {
    fn genesis(&mut self) {
        self.bank
            .genesis(&self.bank_config, &mut self.working_set)
            .unwrap()
    }

    fn create_token(&mut self, initial_balance: u64, sender_context: &C) {
        let create_token = call::CallMessage::CreateToken::<C> {
            salt: self.salt,
            token_name: "Token1".to_owned(),
            initial_balance,
            minter_address: self.minter_address.clone(),
        };

        self.bank
            .call(create_token, sender_context, &mut self.working_set)
            .unwrap();
    }

    fn transfer(&mut self, amount: u64, receiver_address: <C as Spec>::Address) {
        let transfer = call::CallMessage::Transfer {
            to: receiver_address,
            coins: Coins {
                amount,
                token_address: self.deployed_token_address.clone(),
            },
        };

        self.bank
            .call(transfer, &self.minter_context, &mut self.working_set)
            .unwrap();
    }

    fn burn(&mut self, amount: u64, context: &C) {
        let burn = call::CallMessage::Burn {
            coins: Coins {
                amount,
                token_address: self.deployed_token_address.clone(),
            },
        };

        self.bank
            .call(burn, context, &mut self.working_set)
            .unwrap();
    }

    fn query_balance(&mut self, user_address: <C as Spec>::Address) -> query::BalanceResponse {
        self.query_balance_for(user_address, self.deployed_token_address.clone())
    }

    fn query_balance_for_initial_token(
        &mut self,
        user_address: <C as Spec>::Address,
    ) -> query::BalanceResponse {
        self.query_balance_for(user_address, self.init_token_address.clone())
    }

    fn query_balance_for(
        &mut self,
        user_address: <C as Spec>::Address,
        token_address: <C as Spec>::Address,
    ) -> query::BalanceResponse {
        let query = QueryMessage::GetBalance {
            user_address,
            token_address,
        };

        let resp = self.bank.query(query, &mut self.working_set);
        serde_json::from_slice(&resp.response).unwrap()
    }
}

fn create_addresses(count: usize) -> Vec<<C as sov_modules_api::Spec>::Address> {
    let mut addresses = Vec::new();
    for i in 0..count {
        let pub_key = <C as Spec>::PublicKey::try_from(format!("pub_key_{}", i)).unwrap();
        let address = pub_key.to_address::<<C as Spec>::Address>();
        addresses.push(address)
    }

    addresses
}

fn create_bank_config(addresses_count: usize, initial_balance: u64) -> BankConfig<C> {
    let address_and_balances = create_addresses(addresses_count)
        .into_iter()
        .map(|addr| (addr, initial_balance))
        .collect();

    let token_config = TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances,
    };

    BankConfig {
        tokens: vec![token_config],
    }
}

fn create_test_bank(address_count: usize, initial_balance: u64) -> (TestBank, C) {
    let bank = Bank::<C>::new();
    let working_set = WorkingSet::new(ProverStorage::temporary());

    let sender = <C as Spec>::PublicKey::try_from("pub_key_sender").unwrap();
    let sender_address = sender.to_address::<<C as Spec>::Address>();
    let sender_context = C::new(sender_address.clone());

    let minter = <C as Spec>::PublicKey::try_from("minter_sender").unwrap();
    let minter_address = minter.to_address::<<C as Spec>::Address>();
    let minter_context = C::new(minter_address.clone());

    let salt = 0;
    let token_name = "Token1".to_owned();

    let deployed_token_address =
        super::create_token_address::<C>(&token_name, sender_address.as_ref(), salt);

    let bank_config = create_bank_config(address_count, initial_balance);
    let init_token_address = create_token_address::<C>(
        &bank_config.tokens[0].token_name,
        &genesis::DEPLOYER,
        genesis::SALT,
    );
    (
        TestBank {
            bank,
            bank_config,
            minter_address,
            minter_context,
            init_token_address,
            deployed_token_address,
            salt,
            working_set,
        },
        sender_context,
    )
}

#[test]
fn test_bank_happy_path() {
    let (mut test_bank, sender_context) = create_test_bank(5, 10_000);

    // Genesis
    {
        test_bank.genesis();
        let (addr, balance) = test_bank.bank_config.tokens[0].address_and_balances[0].clone();
        let query_response = test_bank.query_balance_for_initial_token(addr);

        assert_eq!(
            query_response.amount,
            Some(balance),
            "Bank has not been deployed correctly"
        );
    }

    // Create token
    let initial_balance = 100;
    {
        test_bank.create_token(initial_balance, &sender_context);
        let query_response = test_bank.query_balance(test_bank.minter_address.clone());
        assert_eq!(query_response.amount, Some(initial_balance));
    }

    // Transfer coins
    let amount = 22;
    let receiver = MockPublicKey::try_from("pub_key_receiver").unwrap();
    let receiver_address = receiver.to_address::<<C as Spec>::Address>();
    let receiver_context = C::new(receiver_address.clone());
    {
        test_bank.transfer(amount, receiver_address.clone());

        let query_response = test_bank.query_balance(test_bank.minter_address.clone());
        assert_eq!(query_response.amount, Some(initial_balance - amount));
    }

    // Burn coins
    {
        let query_response = test_bank.query_balance(receiver_address.clone());
        assert_eq!(query_response.amount, Some(amount));

        let burn_amount = 22;
        test_bank.burn(burn_amount, &receiver_context);

        let query_response = test_bank.query_balance(receiver_address);
        assert_eq!(query_response.amount, Some(amount - burn_amount));
    }
}

#[test]
fn test_bank_edge_cases() {
    let (mut test_bank, _) = create_test_bank(3, 10_0000);
    test_bank.genesis();

    // Not enough balance
    {}

    // Sender does not exist
    {
        let amount = 22;
        let unknown_sender = MockPublicKey::try_from("pub_key_unknown_receiver").unwrap();
        let unknown_sender_address = unknown_sender.to_address::<<C as Spec>::Address>();
        let unknown_sender_context = C::new(unknown_sender_address);

        let (receiver_address, _) = test_bank.bank_config.tokens[0].address_and_balances[0].clone();

        let transfer = call::CallMessage::Transfer {
            to: receiver_address.clone(),
            coins: Coins {
                amount,
                token_address: test_bank.deployed_token_address.clone(),
            },
        };

        let query_response = test_bank.query_balance(receiver_address.clone());
        let balance_before = query_response.amount;

        let result = test_bank.bank.call(
            transfer,
            &unknown_sender_context,
            &mut test_bank.working_set,
        );
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert!(error
            .to_string()
            .contains("Value not found for prefix: \"bank/Bank/tokens/\" and: storage key"));

        let query_response = test_bank.query_balance(receiver_address);

        assert_eq!(query_response.amount, balance_before);
    }

    // Receiver does not exist

    // Sender does not have enough of token A, but enough token B
}

#[test]
fn not_enough_balance() {
    let initial_balance = 100;
    let (mut test_bank, sender_context) = create_test_bank(2, initial_balance);
    test_bank.genesis();

    let receiver = MockPublicKey::try_from("pub_key_receiver").unwrap();
    let receiver_address = receiver.to_address::<<C as Spec>::Address>();
    {
        let transfer = call::CallMessage::Transfer {
            to: receiver_address,
            coins: Coins {
                amount: initial_balance + 1,
                token_address: test_bank.init_token_address.clone(),
            },
        };

        let result = test_bank
            .bank
            .call(transfer, &sender_context, &mut test_bank.working_set);

        assert!(result.is_err());
        let error = result.err().unwrap();
        assert_eq!("Not enough balance", error.to_string());
    }
}

#[test]
fn integer_overflow() {
    let bank = Bank::<C>::new();
    let mut working_set = WorkingSet::new(ProverStorage::temporary());

    let bank_config = create_bank_config(2, u64::MAX - 1);

    let genesis_result = bank.genesis(&bank_config, &mut working_set);
    assert!(genesis_result.is_err());

    assert_eq!(
        "Total supply overflow",
        genesis_result.unwrap_err().to_string()
    );
}
