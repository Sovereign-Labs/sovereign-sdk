use crate::runtime::{GenesisConfig, TestRuntime, RT, S};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::TypedTransaction;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::Address;
use reth_primitives::TransactionSigned;
use secp256k1::rand::SeedableRng as _;
use secp256k1::{PublicKey, SecretKey};
use sov_address::EthereumAddress;
use sov_address::MultiAddress;
use sov_eth_dev_signer::Signer;
use sov_evm::{AccountData, EvmGenesisConfig, RlpEvmTransaction, SpecId};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::TEST_DEFAULT_USER_BALANCE;

pub(crate) struct EvmAccount(SecretKey);

impl EvmAccount {
    pub fn generate() -> Self {
        let mut rng = secp256k1::rand::rngs::StdRng::from_entropy();
        let secret_key = SecretKey::new(&mut rng);
        Self(secret_key)
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey::from_secret_key(secp256k1::SECP256K1, &self.0)
    }

    pub fn address(&self) -> Address {
        reth_primitives::public_key_to_address(self.public_key())
    }

    pub fn sign(&self, tx: TypedTransaction) -> (RlpEvmTransaction, TransactionSigned) {
        let signer = Signer::new(self.0);
        let signed_tx = signer.sign_transaction(tx).unwrap();
        let rlp = signed_tx.encoded_2718();
        (RlpEvmTransaction { rlp }, signed_tx)
    }
}

pub(crate) fn setup() -> (TestRunner<RT, S>, EvmAccount, EvmAccount) {
    let evm_account = EvmAccount::generate();
    let no_balance_account = EvmAccount::generate();

    let genesis_config = HighLevelOptimisticGenesisConfig::generate();

    let mut evm_config = EvmGenesisConfig {
        accounts: vec![
            AccountData {
                address: evm_account.address(),
                code_hash: KECCAK_EMPTY,
                code: Default::default(),
            },
            AccountData {
                address: no_balance_account.address(),
                code_hash: KECCAK_EMPTY,
                code: Default::default(),
            },
        ],
        ..Default::default()
    };
    // SHANGHAI instead of LATEST
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
    evm_config.chain_spec.hardforks = vec![(0, SpecId::SHANGHAI)];

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (runner, evm_account, no_balance_account)
}
