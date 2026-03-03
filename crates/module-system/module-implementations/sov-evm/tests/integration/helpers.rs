use crate::runtime::{GenesisConfig, TestRuntime, RT, S};
use alloy_consensus::crypto::secp256k1::public_key_to_address;
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::B256;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use secp256k1::rand::SeedableRng as _;
use secp256k1::{PublicKey, SecretKey};
use sov_address::{EthereumAddress, MultiAddress};
use sov_eth_dev_signer::Signer;
use sov_evm::ContractCreationPolicy;
use sov_evm::EvmChainSpec;
use sov_evm::{
    AccountData, EthereumAuthenticator, EvmGenesisConfig, RlpEvmTransaction, SpecId,
    TransactionSigned,
};
use sov_evm_test_utils::LegacySimpleStorage;
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::runtime::{genesis::optimistic::HighLevelOptimisticGenesisConfig, TestRunner};
use sov_test_utils::{TestUser, TransactionType, TEST_DEFAULT_USER_BALANCE};

/// Sets the block height after which max fee check becomes active.
pub(crate) fn set_max_fee_check_height(height: u64) {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_EVM_MAX_FEE_CHECK_HEIGHT",
        height.to_string(),
    );
}

/// Sets the block height after which receipt effective gas price is derived from actual charged fee.
pub(crate) fn set_receipt_actual_fee_height(height: u64) {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_EVM_RECEIPT_ACTUAL_FEE_HEIGHT",
        height.to_string(),
    );
}

pub(crate) struct EvmAccount(SecretKey);

impl EvmAccount {
    pub fn generate() -> Self {
        let mut rng = secp256k1::rand::rngs::StdRng::from_entropy();
        let secret_key = SecretKey::new(&mut rng);
        Self(secret_key)
    }

    pub fn secret_key(&self) -> SecretKey {
        self.0
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

/// Setup with EVM accounts and an admin TestUser for config updates.
/// Returns (runner, funded_account, no_balance_account, admin).
pub(crate) fn setup() -> (TestRunner<RT, S>, EvmAccount, EvmAccount, TestUser<S>) {
    let evm_account = EvmAccount::generate();
    let no_balance_account = EvmAccount::generate();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let accounts = vec![
        AccountData::empty_with_address(evm_account.address()),
        AccountData::empty_with_address(no_balance_account.address()),
    ];

    let evm_config = EvmGenesisConfig {
        accounts,
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)],
        },
        contract_creation_policy: ContractCreationPolicy::Everyone,
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (runner, evm_account, no_balance_account, admin)
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

pub(crate) fn create_transfer_tx_with_fee_params(
    nonce: u64,
    from: &EvmAccount,
    to: &EvmAccount,
    value: u128,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
) -> TxWithNonceAndHash {
    let tx = TxEip1559 {
        to: TxKind::Call(to.address()),
        value: U256::from(value),
        nonce,
        max_fee_per_gas,
        max_priority_fee_per_gas,
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
    contract: &LegacySimpleStorage,
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
    contract: &LegacySimpleStorage,
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
    contract: &LegacySimpleStorage,
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
    contract: &LegacySimpleStorage,
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

/// Create a transfer transaction with a specific max_fee_per_gas.
pub(crate) fn create_transfer_tx_with_max_fee(
    nonce: u64,
    from: &EvmAccount,
    to: Address,
    max_fee_per_gas: u128,
) -> TransactionType<RT, S> {
    let tx = TxEip1559 {
        to: TxKind::Call(to),
        value: U256::from(1),
        nonce,
        gas_limit: 1_000_000,
        max_fee_per_gas,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };
    let (signed_eth_tx, _) = from.sign(TypedTransaction::Eip1559(tx));
    let data = borsh::to_vec(&signed_eth_tx).unwrap();
    let raw_tx = RawTx { data };
    TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx))
}
