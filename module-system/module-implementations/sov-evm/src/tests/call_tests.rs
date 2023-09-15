use reth_primitives::{Address, TransactionKind};
use revm::primitives::{SpecId, KECCAK_EMPTY, U256};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::{Context, Module, PrivateKey, PublicKey, Spec};

use crate::call::CallMessage;
use crate::evm::primitive_types::Receipt;
use crate::smart_contracts::SimpleStorageContract;
use crate::tests::dev_signer::TestSigner;
use crate::tests::genesis_tests::get_evm;
use crate::{AccountData, EvmConfig};
type C = DefaultContext;

fn create_messages(
    contract_addr: Address,
    set_arg: u32,
    dev_signer: TestSigner,
    create_contract: bool,
) -> Vec<CallMessage> {
    let mut transactions = Vec::default();
    let contract = SimpleStorageContract::default();
    let mut nonce = 0;

    // Contract creation.
    if create_contract {
        let signed_tx = dev_signer
            .sign_default_transaction(TransactionKind::Create, contract.byte_code().to_vec(), 0)
            .unwrap();

        transactions.push(CallMessage { tx: signed_tx });
        nonce += 1;
    }

    // Update contract state.
    {
        let signed_tx = dev_signer
            .sign_default_transaction(
                TransactionKind::Call(contract_addr),
                hex::decode(hex::encode(&contract.set_call_data(set_arg))).unwrap(),
                nonce,
            )
            .unwrap();

        transactions.push(CallMessage { tx: signed_tx });
    }

    transactions
}

#[test]
fn evm_test() {
    let dev_signer: TestSigner = TestSigner::new_random();

    let config = EvmConfig {
        data: vec![AccountData {
            address: dev_signer.address(),
            balance: U256::from(1000000000),
            code_hash: KECCAK_EMPTY,
            code: vec![],
            nonce: 0,
        }],
        spec: vec![(0, SpecId::LATEST)].into_iter().collect(),
        ..Default::default()
    };

    let (evm, mut working_set) = get_evm(&config);
    let working_set = &mut working_set;

    let contract_addr: Address = Address::from_slice(
        hex::decode("819c5497b157177315e1204f52e588b393771719")
            .unwrap()
            .as_slice(),
    );

    evm.begin_slot_hook([5u8; 32], working_set);

    let set_arg = 999;
    let sender_context = C::new(
        DefaultPrivateKey::generate()
            .pub_key()
            .to_address::<<C as Spec>::Address>(),
    );

    for tx in create_messages(contract_addr, set_arg, dev_signer, true) {
        evm.call(tx, &sender_context, working_set).unwrap();
    }

    evm.end_slot_hook(working_set);

    let db_account = evm.accounts.get(&contract_addr, working_set).unwrap();
    let storage_value = db_account.storage.get(&U256::ZERO, working_set).unwrap();

    assert_eq!(U256::from(set_arg), storage_value);
    assert_eq!(
        evm.receipts
            .iter(&mut working_set.accessory_state())
            .collect::<Vec<_>>(),
        [
            Receipt {
                receipt: reth_primitives::Receipt {
                    tx_type: reth_primitives::TxType::EIP1559,
                    success: true,
                    cumulative_gas_used: 132943,
                    logs: vec![]
                },
                gas_used: 132943,
                log_index_start: 0,
                error: None
            },
            Receipt {
                receipt: reth_primitives::Receipt {
                    tx_type: reth_primitives::TxType::EIP1559,
                    success: true,
                    cumulative_gas_used: 176673,
                    logs: vec![]
                },
                gas_used: 43730,
                log_index_start: 0,
                error: None
            }
        ]
    )
}

#[test]
fn failed_transaction_test() {
    let dev_signer: TestSigner = TestSigner::new_random();

    let (evm, mut working_set) = get_evm(&EvmConfig::default());
    let working_set = &mut working_set;

    let contract_addr: Address = Address::from_slice(
        hex::decode("819c5497b157177315e1204f52e588b393771719")
            .unwrap()
            .as_slice(),
    );

    evm.begin_slot_hook([5u8; 32], working_set);

    let set_arg = 999;
    let sender_context = C::new(
        DefaultPrivateKey::generate()
            .pub_key()
            .to_address::<<C as Spec>::Address>(),
    );

    for tx in create_messages(contract_addr, set_arg, dev_signer, false) {
        evm.call(tx, &sender_context, working_set).unwrap();
    }

    evm.end_slot_hook(working_set);

    assert_eq!(
        evm.receipts
            .iter(&mut working_set.accessory_state())
            .collect::<Vec<_>>(),
        [Receipt {
            receipt: reth_primitives::Receipt {
                tx_type: reth_primitives::TxType::EIP1559,
                success: false,
                cumulative_gas_used: 0,
                logs: vec![]
            },
            gas_used: 0,
            log_index_start: 0,
            error: Some(revm::primitives::EVMError::Transaction(
                revm::primitives::InvalidTransaction::LackOfFundForGasLimit {
                    gas_limit: U256::from(0xd59f80),
                    balance: U256::ZERO
                }
            ))
        }]
    )
}
