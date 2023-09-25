use lazy_static::lazy_static;
use reth_primitives::hex_literal::hex;
use reth_primitives::{
    Address, Bloom, Bytes, Header, SealedHeader, Signature, TransactionSigned, EMPTY_OMMER_ROOT,
    H256, KECCAK_EMPTY, U256,
};

use super::genesis_tests::{get_evm, TEST_CONFIG};
use crate::evm::primitive_types::{
    Block, BlockEnv, Receipt, SealedBlock, TransactionSignedAndRecovered,
};
use crate::experimental::PendingTransaction;
use crate::tests::genesis_tests::{BENEFICIARY, SEALED_GENESIS_HASH};

lazy_static! {
    pub(crate) static ref DA_ROOT_HASH: H256 = H256::from([5u8; 32]);
}

#[test]
fn begin_slot_hook_creates_pending_block() {
    let (evm, mut working_set) = get_evm(&TEST_CONFIG);
    evm.begin_slot_hook(DA_ROOT_HASH.0, &mut working_set);
    let pending_block = evm.block_env.get(&mut working_set).unwrap();
    assert_eq!(
        pending_block,
        BlockEnv {
            number: 1,
            coinbase: *BENEFICIARY,
            timestamp: TEST_CONFIG.genesis_timestamp + TEST_CONFIG.block_timestamp_delta,
            prevrandao: *DA_ROOT_HASH,
            basefee: 62u64,
            gas_limit: TEST_CONFIG.block_gas_limit,
        }
    );
}

#[test]
fn end_slot_hook_sets_head() {
    let (evm, mut working_set) = get_evm(&TEST_CONFIG);
    evm.begin_slot_hook(DA_ROOT_HASH.0, &mut working_set);

    evm.pending_transactions.push(
        &create_pending_transaction(H256::from([1u8; 32]), 1),
        &mut working_set,
    );

    evm.pending_transactions.push(
        &create_pending_transaction(H256::from([2u8; 32]), 2),
        &mut working_set,
    );

    evm.end_slot_hook(&mut working_set);
    let head = evm.head.get(&mut working_set).unwrap();
    let pending_head = evm
        .pending_head
        .get(&mut working_set.accessory_state())
        .unwrap();

    assert_eq!(head, pending_head);
    assert_eq!(
        head,
        Block {
            header: Header {
                // TODO: temp parent hash until: https://github.com/Sovereign-Labs/sovereign-sdk/issues/876
                // parent_hash: GENESIS_HASH,
                parent_hash: H256(hex!(
                    "d57423e4375c45bc114cd137146aab671dbd3f6304f05b31bdd416301b4a99f0"
                )),
                ommers_hash: EMPTY_OMMER_ROOT,
                beneficiary: TEST_CONFIG.coinbase,
                state_root: KECCAK_EMPTY,
                transactions_root: H256(hex!(
                    "30eb5f6050df7ea18ca34cf3503f4713119315a2d3c11f892c5c8920acf816f4"
                )),
                receipts_root: H256(hex!(
                    "27036187b3f5e87d4306b396cf06c806da2cc9a0fef9b07c042e3b4304e01c64"
                )),
                withdrawals_root: None,
                logs_bloom: Bloom::default(),
                difficulty: U256::ZERO,
                number: 1,
                gas_limit: TEST_CONFIG.block_gas_limit,
                gas_used: 200u64,
                timestamp: TEST_CONFIG.genesis_timestamp + TEST_CONFIG.block_timestamp_delta,
                mix_hash: *DA_ROOT_HASH,
                nonce: 0,
                base_fee_per_gas: Some(62u64),
                extra_data: Bytes::default(),
                blob_gas_used: None,
                excess_blob_gas: None,
                parent_beacon_block_root: None,
            },
            transactions: 0..2
        }
    );
}

#[test]
fn end_slot_hook_moves_transactions_and_receipts() {
    let (evm, mut working_set) = get_evm(&TEST_CONFIG);
    evm.begin_slot_hook(DA_ROOT_HASH.0, &mut working_set);

    let tx1 = create_pending_transaction(H256::from([1u8; 32]), 1);
    evm.pending_transactions.push(&tx1, &mut working_set);

    let tx2 = create_pending_transaction(H256::from([2u8; 32]), 2);
    evm.pending_transactions.push(&tx2, &mut working_set);

    evm.end_slot_hook(&mut working_set);

    let tx1_hash = tx1.transaction.signed_transaction.hash;
    let tx2_hash = tx2.transaction.signed_transaction.hash;

    assert_eq!(
        evm.transactions
            .iter(&mut working_set.accessory_state())
            .collect::<Vec<_>>(),
        [tx1.transaction, tx2.transaction]
    );

    assert_eq!(
        evm.receipts
            .iter(&mut working_set.accessory_state())
            .collect::<Vec<_>>(),
        [tx1.receipt, tx2.receipt]
    );

    assert_eq!(
        evm.transaction_hashes
            .get(&tx1_hash, &mut working_set.accessory_state())
            .unwrap(),
        0
    );

    assert_eq!(
        evm.transaction_hashes
            .get(&tx2_hash, &mut working_set.accessory_state())
            .unwrap(),
        1
    );

    assert_eq!(evm.pending_transactions.len(&mut working_set), 0);
}

fn create_pending_transaction(hash: H256, index: u64) -> PendingTransaction {
    PendingTransaction {
        transaction: TransactionSignedAndRecovered {
            signer: Address::from([1u8; 20]),
            signed_transaction: TransactionSigned {
                hash,
                signature: Signature::default(),
                transaction: reth_primitives::Transaction::Eip1559(reth_primitives::TxEip1559 {
                    chain_id: 1u64,
                    nonce: 1u64,
                    gas_limit: 1000u64,
                    max_fee_per_gas: 2000u64 as u128,
                    max_priority_fee_per_gas: 3000u64 as u128,
                    to: reth_primitives::TransactionKind::Call(Address::from([3u8; 20])),
                    value: 4000u64 as u128,
                    access_list: reth_primitives::AccessList::default(),
                    input: Bytes::from([4u8; 20]),
                }),
            },
            block_number: 1,
        },
        receipt: Receipt {
            receipt: reth_primitives::Receipt {
                tx_type: reth_primitives::TxType::EIP1559,
                success: true,
                cumulative_gas_used: 100u64 * index,
                logs: vec![],
            },
            gas_used: 100u64,
            log_index_start: 0,
            error: None,
        },
    }
}

#[test]
fn finalize_hook_creates_final_block() {
    let (evm, mut working_set) = get_evm(&TEST_CONFIG);
    evm.begin_slot_hook(DA_ROOT_HASH.0, &mut working_set);
    evm.pending_transactions.push(
        &create_pending_transaction(H256::from([1u8; 32]), 1),
        &mut working_set,
    );
    evm.pending_transactions.push(
        &create_pending_transaction(H256::from([2u8; 32]), 2),
        &mut working_set,
    );
    evm.end_slot_hook(&mut working_set);

    let mut accessory_state = working_set.accessory_state();
    let root_hash = [99u8; 32].into();
    evm.finalize_hook(&root_hash, &mut accessory_state);

    assert_eq!(evm.blocks.len(&mut accessory_state), 2);

    let block = evm.blocks.get(1usize, &mut accessory_state).unwrap();

    assert_eq!(
        block,
        SealedBlock {
            header: SealedHeader {
                header: Header {
                    parent_hash: SEALED_GENESIS_HASH,
                    ommers_hash: EMPTY_OMMER_ROOT,
                    beneficiary: TEST_CONFIG.coinbase,
                    state_root: H256::from(root_hash.0),
                    transactions_root: H256(hex!(
                        "30eb5f6050df7ea18ca34cf3503f4713119315a2d3c11f892c5c8920acf816f4"
                    )),
                    receipts_root: H256(hex!(
                        "27036187b3f5e87d4306b396cf06c806da2cc9a0fef9b07c042e3b4304e01c64"
                    )),
                    withdrawals_root: None,
                    logs_bloom: Bloom::default(),
                    difficulty: U256::ZERO,
                    number: 1,
                    gas_limit: 30000000,
                    gas_used: 200,
                    timestamp: 52,
                    mix_hash: H256(hex!(
                        "0505050505050505050505050505050505050505050505050505050505050505"
                    )),
                    nonce: 0,
                    base_fee_per_gas: Some(62),
                    extra_data: Bytes::default(),
                    blob_gas_used: None,
                    excess_blob_gas: None,
                    parent_beacon_block_root: None,
                },
                hash: H256(hex!(
                    "0da4e80c5cbd00d9538cb0215d069bfee5be5b59ae4da00244f9b8db429e6889"
                )),
            },
            transactions: 0..2
        }
    );

    assert_eq!(
        evm.block_hashes
            .get(&block.header.hash, &mut accessory_state)
            .unwrap(),
        1u64
    );

    assert_eq!(evm.pending_head.get(&mut accessory_state), None);
}
