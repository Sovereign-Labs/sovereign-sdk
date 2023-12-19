use reth_primitives::{Address, Bytes, TransactionKind};
use revm::primitives::{SpecId, KECCAK_EMPTY, U256};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::utils::generate_address;
use sov_modules_api::{Context, Module, StateMapAccessor, StateValueAccessor, StateVecAccessor};

use crate::call::CallMessage;
use crate::evm::primitive_types::Receipt;
use crate::smart_contracts::SimpleStorageContract;
use crate::tests::genesis_tests::get_evm;
use crate::tests::test_signer::TestSigner;
use crate::{AccountData, EvmConfig};
type C = DefaultContext;

#[test]
fn call_test() {
    let dev_signer: TestSigner = TestSigner::new_random();
    let config = EvmConfig {
        data: vec![AccountData {
            address: dev_signer.address(),
            balance: U256::from(1000000000),
            code_hash: KECCAK_EMPTY,
            code: Bytes::default(),
            nonce: 0,
        }],
        // SHANGAI instead of LATEST
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
        spec: vec![(0, SpecId::SHANGHAI)].into_iter().collect(),
        ..Default::default()
    };

    let (evm, mut working_set) = get_evm(&config);

    let contract_addr: Address = Address::from_slice(
        hex::decode("819c5497b157177315e1204f52e588b393771719")
            .unwrap()
            .as_slice(),
    );

    evm.begin_slot_hook([5u8; 32], &[10u8; 32].into(), &mut working_set);

    let set_arg = 999;
    {
        let sender_address = generate_address::<C>("sender");
        let sequencer_address = generate_address::<C>("sequencer");
        let context = C::new(sender_address, sequencer_address, 1);

        let messages = vec![
            create_contract_message(&dev_signer, 0),
            set_arg_message(contract_addr, &dev_signer, 1, set_arg),
        ];
        for tx in messages {
            evm.call(tx, &context, &mut working_set).unwrap();
        }
    }
    evm.end_slot_hook(&mut working_set);

    let db_account = evm.accounts.get(&contract_addr, &mut working_set).unwrap();
    let storage_value = db_account
        .storage
        .get(&U256::ZERO, &mut working_set)
        .unwrap();

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

    evm.begin_slot_hook([5u8; 32], &[10u8; 32].into(), working_set);
    {
        let sender_address = generate_address::<C>("sender");
        let sequencer_address = generate_address::<C>("sequencer");
        let context = C::new(sender_address, sequencer_address, 1);
        let message = create_contract_message(&dev_signer, 0);
        evm.call(message, &context, working_set).unwrap();
    }

    // assert no pending transaction
    let pending_txs = evm.pending_transactions.iter(working_set);
    assert_eq!(pending_txs.len(), 0);

    evm.end_slot_hook(working_set);

    // Assert block does not have any transaction
    let block = evm
        .pending_head
        .get(&mut working_set.accessory_state())
        .unwrap();
    assert_eq!(block.transactions.start, 0);
    assert_eq!(block.transactions.end, 0);
}

fn create_contract_message(dev_signer: &TestSigner, nonce: u64) -> CallMessage {
    let contract = SimpleStorageContract::default();
    let signed_tx = dev_signer
        .sign_default_transaction(
            TransactionKind::Create,
            contract.byte_code().to_vec(),
            nonce,
        )
        .unwrap();
    CallMessage { tx: signed_tx }
}

fn set_arg_message(
    contract_addr: Address,
    dev_signer: &TestSigner,
    nonce: u64,
    set_arg: u32,
) -> CallMessage {
    let contract = SimpleStorageContract::default();
    let signed_tx = dev_signer
        .sign_default_transaction(
            TransactionKind::Call(contract_addr),
            hex::decode(hex::encode(&contract.set_call_data(set_arg))).unwrap(),
            nonce,
        )
        .unwrap();

    CallMessage { tx: signed_tx }
}
