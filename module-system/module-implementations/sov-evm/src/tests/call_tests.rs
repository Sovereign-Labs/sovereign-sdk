use reth_primitives::TransactionKind;
use revm::primitives::{SpecId, KECCAK_EMPTY, U256};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, Module, PrivateKey, Spec};
use sov_state::{ProverStorage, WorkingSet};

use crate::call::CallMessage;
use crate::dev_signer::DevSigner;
use crate::evm::test_helpers::SimpleStorageContract;
use crate::evm::EthAddress;
use crate::{AccountData, Evm, EvmConfig};

type C = DefaultContext;

fn create_messages(
    contract_addr: EthAddress,
    set_arg: u32,
    dev_signer: DevSigner,
) -> Vec<CallMessage> {
    println!("Addr {:?}", hex::encode(dev_signer.address));

    let mut transactions = Vec::default();
    let contract = SimpleStorageContract::new();

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
                TransactionKind::Call(contract_addr.into()),
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

    let dev_signer: DevSigner = DevSigner::new_random();
    let caller = dev_signer.address;

    let evm = Evm::<C>::default();

    let data = AccountData {
        address: caller,
        balance: U256::from(1000000000).to_le_bytes(),
        code_hash: KECCAK_EMPTY.to_fixed_bytes(),
        code: vec![],
        nonce: 0,
    };

    let config = EvmConfig {
        data: vec![data],
        spec: vec![(0, SpecId::LATEST)].into_iter().collect(),
        ..Default::default()
    };

    evm.genesis(&config, working_set).unwrap();

    let contract_addr = hex::decode("819c5497b157177315e1204f52e588b393771719")
        .unwrap()
        .try_into()
        .unwrap();

    let set_arg = 999;

    for tx in create_messages(contract_addr, set_arg, dev_signer) {
        evm.call(tx, &sender_context, working_set).unwrap();
    }

    let db_account = evm.accounts.get(&contract_addr, working_set).unwrap();
    let storage_key = &[0; 32];
    let storage_value = db_account.storage.get(storage_key, working_set).unwrap();

    assert_eq!(set_arg.to_le_bytes(), storage_value[0..4])
}
