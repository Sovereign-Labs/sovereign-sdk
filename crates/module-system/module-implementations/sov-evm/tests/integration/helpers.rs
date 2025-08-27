use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::TypedTransaction;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, U256};
use reth_primitives::TransactionSigned;
use secp256k1::rand::SeedableRng as _;
use secp256k1::{PublicKey, SecretKey};
use sov_eth_dev_signer::Signer;
use sov_evm::{AccountData, EvmGenesisConfig, RlpEvmTransaction, SpecId};
use sov_modules_api::{CredentialId, HexHash};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::TestUser;

use crate::runtime::{GenesisConfig, TestRuntime, RT, S};

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

pub(crate) fn setup() -> (TestRunner<RT, S>, TestUser<S>, EvmAccount, EvmAccount) {
    let evm_account = EvmAccount::generate();
    let no_balance_account = EvmAccount::generate();
    let rollup_account = TestUser::generate_with_default_balance().add_credential_id(CredentialId(
        HexHash::new(evm_account.address().into_word().into()),
    ));
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![rollup_account.clone()]);
    let mut evm_config = EvmGenesisConfig {
        accounts: vec![
            AccountData {
                address: evm_account.address(),
                balance: U256::from(1000000000),
                code_hash: KECCAK_EMPTY,
                code: Default::default(),
                nonce: 0,
            },
            AccountData {
                address: no_balance_account.address(),
                balance: U256::from(0),
                code_hash: KECCAK_EMPTY,
                code: Default::default(),
                nonce: 0,
            },
        ],
        ..Default::default()
    };
    // SHANGHAI instead of LATEST
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
    evm_config.chain_spec.hardforks = vec![(0, SpecId::SHANGHAI)];

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (runner, rollup_account, evm_account, no_balance_account)
}
