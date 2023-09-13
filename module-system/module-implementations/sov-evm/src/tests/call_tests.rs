use reth_primitives::{Address, TransactionKind};
use revm::primitives::{SpecId, KECCAK_EMPTY, U256};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, Module, PrivateKey, Spec};
use sov_state::{ProverStorage, WorkingSet};

use crate::call::CallMessage;
use crate::smart_contracts::SimpleStorageContract;
use crate::tests::dev_signer::TestSigner;
use crate::{AccountData, Evm, EvmConfig};
type C = DefaultContext;

fn create_messages(
    contract_addr: Address,
    set_arg: u32,
    dev_signer: TestSigner,
) -> Vec<CallMessage> {
    let mut transactions = Vec::default();
    let contract = SimpleStorageContract::default();

    // Contract creation.
    {
        let signed_tx = dev_signer
            .sign_default_transaction(TransactionKind::Create, contract.byte_code().to_vec(), 0)
            .unwrap();

        transactions.push(CallMessage { tx: signed_tx });
    }

    // Update contract state.
    {
        let signed_tx = dev_signer
            .sign_default_transaction(
                TransactionKind::Call(contract_addr),
                hex::decode(hex::encode(&contract.set_call_data(set_arg))).unwrap(),
                1,
            )
            .unwrap();

        transactions.push(CallMessage { tx: signed_tx });
    }

    transactions
}

#[test]
fn evm_test() {
    use sov_modules_api::PublicKey;
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    let priv_key = DefaultPrivateKey::generate();

    let sender = priv_key.pub_key();
    let sender_addr = sender.to_address::<<C as Spec>::Address>();
    let sender_context = C::new(sender_addr);

    let dev_signer: TestSigner = TestSigner::new_random();
    let caller = dev_signer.address();

    let evm = Evm::<C>::default();

    let data = AccountData {
        address: caller,
        balance: U256::from(1000000000),
        code_hash: KECCAK_EMPTY,
        code: vec![],
        nonce: 0,
    };

    let config = EvmConfig {
        data: vec![data],
        spec: vec![(0, SpecId::LATEST)].into_iter().collect(),
        ..Default::default()
    };

    evm.genesis(&config, working_set).unwrap();

    let contract_addr: Address = Address::from_slice(
        hex::decode("819c5497b157177315e1204f52e588b393771719")
            .unwrap()
            .as_slice(),
    );

    let set_arg = 999;

    for tx in create_messages(contract_addr, set_arg, dev_signer) {
        evm.call(tx, &sender_context, working_set).unwrap();
    }

    let db_account = evm.accounts.get(&contract_addr, working_set).unwrap();
    let storage_value = db_account.storage.get(&U256::ZERO, working_set).unwrap();

    assert_eq!(U256::from(set_arg), storage_value)
}
