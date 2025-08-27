use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use reth_primitives::TransactionSigned;
use secp256k1::rand::SeedableRng as _;
use secp256k1::{PublicKey, SecretKey};
use sov_eth_dev_signer::Signer;
use sov_evm::{AccountData, EthereumAuthenticator, EvmGenesisConfig, RlpEvmTransaction, SpecId};
use sov_modules_api::macros::config_value;
use sov_modules_api::{CredentialId, HexHash, RawTx};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{SimpleStorageContract, TestUser, TransactionType};

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

pub(crate) const INITIAL_BALANCE: u128 = 1000000000;

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
                balance: U256::from(INITIAL_BALANCE),
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

pub(crate) fn create_transfer_tx(
    nonce: u64,
    from: &EvmAccount,
    to: &EvmAccount,
    value: u128,
) -> TransactionType<RT, S> {
    let tx = TxEip1559 {
        to: TxKind::Call(to.address()),
        value: U256::from(value),
        nonce,
        ..Default::default()
    };
    create_tx(from, tx)
}

pub(crate) fn create_deploy_tx(
    nonce: u64,
    contract: &SimpleStorageContract,
    account: &EvmAccount,
) -> TransactionType<RT, S> {
    let tx = TxEip1559 {
        input: Bytes::from(contract.byte_code().to_vec()),
        nonce,
        ..Default::default()
    };
    create_tx(account, tx)
}

pub(crate) fn create_set_arg_tx(
    set_arg: u32,
    nonce: u64,
    contract: &SimpleStorageContract,
    contract_addr: Address,
    account: &EvmAccount,
) -> TransactionType<RT, S> {
    let tx = TxEip1559 {
        to: TxKind::Call(contract_addr),
        input: Bytes::from(hex::decode(hex::encode(contract.set_call_data(set_arg))).unwrap()),
        nonce,
        ..Default::default()
    };
    create_tx(account, tx)
}

fn create_tx(account: &EvmAccount, tx: TxEip1559) -> TransactionType<RT, S> {
    let tx_with_defaults = TxEip1559 {
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..tx
    };
    let (signed_eth_tx, _) = account.sign(TypedTransaction::Eip1559(tx_with_defaults));
    let data = borsh::to_vec(&signed_eth_tx).unwrap();
    let raw_tx = RawTx { data };
    TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx))
}
