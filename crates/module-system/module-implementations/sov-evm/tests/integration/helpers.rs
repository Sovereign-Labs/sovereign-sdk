use crate::runtime::{GenesisConfig, TestRuntime, RT, S};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::crypto::secp256k1::public_key_to_address;
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::B256;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use secp256k1::rand::SeedableRng as _;
use secp256k1::{PublicKey, SecretKey};
use sov_address::EthereumAddress;
use sov_address::MultiAddress;
use sov_eth_dev_signer::Signer;
use sov_evm::{
    AccountData, EthereumAuthenticator, EvmGenesisConfig, RlpEvmTransaction, SpecId,
    TransactionSigned,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::runtime::{genesis::optimistic::HighLevelOptimisticGenesisConfig, TestRunner};
use sov_test_utils::{SimpleStorage, TransactionType, TEST_DEFAULT_USER_BALANCE};
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
        public_key_to_address(self.public_key())
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

    evm_config.chain_spec.hardforks = vec![(0, SpecId::CANCUN)];

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

pub(crate) fn create_transfer_tx(
    nonce: u64,
    from: &EvmAccount,
    to: &EvmAccount,
    value: u128,
) -> TxWithNonceAndHash {
    let tx = TxEip1559 {
        to: TxKind::Call(to.address()),
        value: U256::from(value),
        nonce,
        ..Default::default()
    };
    create_tx(from, tx)
}

#[derive(Clone)]
pub(crate) struct TxWithNonceAndHash {
    pub(crate) nonce: u64,
    pub(crate) hash: B256,
    pub(crate) tx: TransactionType<RT, S>,
}

pub(crate) fn create_deploy_tx(
    nonce: u64,
    contract: &SimpleStorage,
    account: &EvmAccount,
) -> TxWithNonceAndHash {
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
    contract: &SimpleStorage,
    contract_addr: Address,
    account: &EvmAccount,
) -> TxWithNonceAndHash {
    let tx = TxEip1559 {
        to: TxKind::Call(contract_addr),
        input: Bytes::from(hex::decode(hex::encode(contract.set(set_arg))).unwrap()),
        nonce,
        ..Default::default()
    };
    create_tx(account, tx)
}

pub(crate) fn create_inc_tx(
    nonce: u64,
    contract: &SimpleStorage,
    contract_addr: Address,
    account: &EvmAccount,
) -> TxWithNonceAndHash {
    let tx = TxEip1559 {
        to: TxKind::Call(contract_addr),
        input: Bytes::from(hex::decode(hex::encode(contract.inc())).unwrap()),
        nonce,
        ..Default::default()
    };
    create_tx(account, tx)
}

pub(crate) fn create_emit_logs(
    nonce: u64,
    contract: &SimpleStorage,
    contract_addr: Address,
    account: &EvmAccount,
    topic: u32,
    nb_of_logs: u32,
) -> TxWithNonceAndHash {
    let tx = TxEip1559 {
        to: TxKind::Call(contract_addr),
        input: Bytes::from(
            hex::decode(hex::encode(contract.emit_logs(topic, nb_of_logs))).unwrap(),
        ),
        nonce,
        ..Default::default()
    };
    create_tx(account, tx)
}

fn create_tx(account: &EvmAccount, tx: TxEip1559) -> TxWithNonceAndHash {
    let tx_with_defaults = TxEip1559 {
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..tx
    };
    let (signed_eth_tx, tx_env) = account.sign(TypedTransaction::Eip1559(tx_with_defaults));
    let data = borsh::to_vec(&signed_eth_tx).unwrap();
    let raw_tx = RawTx { data };

    TxWithNonceAndHash {
        nonce: tx.nonce,
        hash: *tx_env.hash(),
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
    }
}
