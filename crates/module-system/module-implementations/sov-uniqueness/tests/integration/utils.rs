use reth_primitives::{Address, TransactionSigned, TxKind, U256};
use reth_rpc_types::transaction::EIP1559TransactionRequest;
use reth_rpc_types::TypedTransactionRequest;
use secp256k1::rand::SeedableRng as _;
use secp256k1::{PublicKey, SecretKey};
use sov_eth_dev_signer::DevSigner;
use sov_evm::{AccountData, EthereumAuthenticator, EvmConfig, RlpEvmTransaction, SpecId};
use sov_modules_api::capabilities::{config_chain_id, TransactionAuthenticator, UniquenessData};
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_api::{EncodeCall, RawTx};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{Runtime, TestRunner, ValueSetter, ValueSetterConfig};
use sov_test_utils::{
    SimpleStorageContract, TestUser, TransactionType, TEST_DEFAULT_MAX_FEE,
    TEST_DEFAULT_MAX_PRIORITY_FEE,
};

use crate::runtime::{GenesisConfig, TestNonceRuntime, RT, S};

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

    pub fn sign(&self, tx: TypedTransactionRequest) -> (RlpEvmTransaction, TransactionSigned) {
        let signer = DevSigner::new(vec![self.0]);
        let signed_tx = signer.sign_transaction(tx, self.address()).unwrap();
        let rlp = signed_tx.envelope_encoded().to_vec();
        (RlpEvmTransaction { rlp }, signed_tx)
    }
}

pub(crate) fn generate_default_tx(
    uniqueness: UniquenessData,
    admin: &TestUser<S>,
    evm_account: &EvmAccount,
) -> TransactionType<RT, S> {
    match uniqueness {
        UniquenessData::Nonce(nonce) => {
            let contract = SimpleStorageContract::default();
            let create_contract_tx_request =
                TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
                    chain_id: config_value!("CHAIN_ID"),
                    nonce,
                    max_priority_fee_per_gas: Default::default(),
                    max_fee_per_gas: U256::from(
                        reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2,
                    ),
                    gas_limit: U256::from(1_000_000u64),
                    kind: TxKind::Create,
                    value: Default::default(),
                    input: reth_primitives::Bytes::from(contract.byte_code().to_vec()),
                    access_list: Default::default(),
                });
            let (signed_eth_tx, _) = evm_account.sign(create_contract_tx_request);
            let create_contract_tx = RawTx {
                data: borsh::to_vec(&signed_eth_tx).unwrap(),
            };
            TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(
                create_contract_tx,
            ))
        }
        UniquenessData::Generation(generation) => generate_value_setter_tx(generation, 10, admin),
    }
}

pub(crate) fn generate_value_setter_tx(
    generation: u64,
    value: u32,
    admin: &TestUser<S>,
) -> TransactionType<RT, S> {
    let runtime_msg =
        <RT as EncodeCall<ValueSetter<S>>>::to_decodable(sov_value_setter::CallMessage::SetValue {
            value,
            gas: None,
        });

    let transaction = UnsignedTransaction::new(
        runtime_msg,
        config_chain_id(),
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        generation,
        None,
    );

    let transaction = Transaction::<RT, S>::new_signed_tx(
        admin.private_key(),
        &<TestNonceRuntime<S> as Runtime<S>>::CHAIN_HASH,
        transaction,
    );
    TransactionType::PreAuthenticated(<RT as Runtime<S>>::Auth::encode_with_standard_auth(RawTx {
        data: borsh::to_vec(&transaction).unwrap(),
    }))
}

pub(crate) fn setup() -> (TestUser<S>, TestRunner<TestNonceRuntime<S>, S>, EvmAccount) {
    // Generate a genesis config, then overwrite the attester key/address with ones that
    // we know. We leave the other values untouched.
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let evm_account = EvmAccount::generate();

    let evm_config = EvmConfig {
        data: vec![AccountData {
            address: evm_account.address(),
            balance: U256::from(1000000000),
            code_hash: reth_primitives::KECCAK_EMPTY,
            code: Default::default(),
            nonce: 0,
        }],
        // SHANGHAI instead of LATEST
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
        spec: vec![(0, SpecId::SHANGHAI)].into_iter().collect(),
        ..Default::default()
    };

    // Run genesis registering the attester and sequencer we've generated.
    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
        evm_config,
    );

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestNonceRuntime::default());

    (admin, runner, evm_account)
}
