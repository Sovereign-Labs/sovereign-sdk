//! Pins the BCS encoding of values that `sov-evm` keeps in state.
//!
//! Several of these types come straight from `revm` and `alloy`, so bumping
//! those dependencies can silently change their serialized layout. That changes
//! the state root (or breaks decoding of stored data) of existing chains.
//! If one of these tests fails after a dependency upgrade, the upgrade is state-breaking.

use alloy_consensus::{
    EthereumTxEnvelope, Header, Signed, TxEip1559, TxEip2930, TxEip4844, TxEip7702, TxLegacy,
    TxType,
};
use alloy_eips::eip2930::{AccessList, AccessListItem};
use alloy_eips::eip7702::{Authorization, SignedAuthorization};
use alloy_primitives::{
    address, b256, bytes, hex, Address, Bloom, Log, LogData, Signature, TxKind, B256, B64, U256,
};
use revm::context::BlockEnv;
use revm::context_interface::block::BlobExcessGasAndPrice;
use revm::state::{AccountInfo, Bytecode};
use sov_rollup_interface::da::Time;
use sov_state::codec::{BcsCodec, StateItemDecoder, StateItemEncoder};

use crate::db::DbAccount;
use crate::evm::eth_receipt::EthReceipt;
use crate::evm::primitive_types::{Block, PendingTransaction, TxSignedAndRecovered};
use crate::{
    ContractCreationPolicy, EvmChainSpec, EvmRuntimeConfig, Receipt, SpecId, TransactionSigned,
};

const ADDR_A: Address = address!("0x1234567890abcdef1234567890abcdef12345678");
const ADDR_B: Address = address!("0xfedcba0987654321fedcba0987654321fedcba09");
const HASH_A: B256 = b256!("0x0102030405060708091011121314151617181920212223242526272829303132");
const HASH_B: B256 = b256!("0xa1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d5");

fn bcs_hex<V: serde::Serialize>(value: &V) -> String {
    let mut out = Vec::new();
    BcsCodec.encode(value, &mut out);
    hex::encode(out)
}

/// Checks that `value` encodes to `expected_hex`, and that `expected_hex` (i.e. bytes already in state) still decodes.
fn assert_bcs_layout<V: serde::Serialize + serde::de::DeserializeOwned>(
    value: &V,
    expected_hex: &str,
    what: &str,
) {
    assert_eq!(bcs_hex(value), expected_hex, "{what}: encoding changed");
    let decoded: V = BcsCodec
        .try_decode(&hex::decode(expected_hex).unwrap())
        .unwrap_or_else(|e| panic!("{what}: stored bytes no longer decode: {e}"));
    assert_eq!(bcs_hex(&decoded), expected_hex, "{what}: decoding is lossy");
}

fn legacy_bytecode() -> Bytecode {
    // PUSH1 0x80 PUSH1 0x40 MSTORE JUMPDEST STOP
    Bytecode::new_raw(bytes!("608060405260005b00"))
}

fn header() -> Header {
    Header {
        parent_hash: HASH_A,
        ommers_hash: HASH_B,
        beneficiary: ADDR_A,
        state_root: HASH_B,
        transactions_root: HASH_A,
        receipts_root: HASH_B,
        logs_bloom: Bloom::repeat_byte(0x5a),
        difficulty: U256::from(0x1234_5678u64),
        number: 0x0102_0304,
        gas_limit: 30_000_123,
        gas_used: 21_000_456,
        timestamp: 0x6543_2100,
        extra_data: bytes!("deadbeef"),
        mix_hash: HASH_A,
        nonce: B64::from(0x0102_0304_0506_0708u64),
        base_fee_per_gas: Some(0x1234),
        withdrawals_root: Some(HASH_B),
        blob_gas_used: Some(0x2345),
        excess_blob_gas: Some(0x3456),
        parent_beacon_block_root: Some(HASH_A),
        requests_hash: Some(HASH_B),
        ..Default::default()
    }
}

fn access_list() -> AccessList {
    AccessList(vec![AccessListItem {
        address: ADDR_B,
        storage_keys: vec![HASH_A, HASH_B],
    }])
}

fn signed<T>(tx: T) -> Signed<T> {
    Signed::new_unchecked(tx, Signature::test_signature(), HASH_A)
}

fn transactions() -> Vec<TransactionSigned> {
    vec![
        EthereumTxEnvelope::Legacy(signed(TxLegacy {
            chain_id: Some(4321),
            nonce: 0x0102,
            gas_price: 0x0304_0506,
            gas_limit: 0x0708_0910,
            to: TxKind::Call(ADDR_B),
            value: U256::from(0x1234_5678u64),
            input: bytes!("c0ffee"),
        })),
        EthereumTxEnvelope::Eip2930(signed(TxEip2930 {
            chain_id: 4321,
            nonce: 0x0102,
            gas_price: 0x0304_0506,
            gas_limit: 0x0708_0910,
            to: TxKind::Create,
            value: U256::from(0x1234_5678u64),
            access_list: access_list(),
            input: bytes!("c0ffee"),
        })),
        EthereumTxEnvelope::Eip1559(signed(TxEip1559 {
            chain_id: 4321,
            nonce: 0x0102,
            gas_limit: 0x0708_0910,
            max_fee_per_gas: 0x0304_0506,
            max_priority_fee_per_gas: 0x0102_0304,
            to: TxKind::Call(ADDR_B),
            value: U256::from(0x1234_5678u64),
            access_list: access_list(),
            input: bytes!("c0ffee"),
        })),
        EthereumTxEnvelope::Eip4844(signed(TxEip4844 {
            chain_id: 4321,
            nonce: 0x0102,
            gas_limit: 0x0708_0910,
            max_fee_per_gas: 0x0304_0506,
            max_priority_fee_per_gas: 0x0102_0304,
            to: ADDR_B,
            value: U256::from(0x1234_5678u64),
            access_list: access_list(),
            blob_versioned_hashes: vec![HASH_B],
            max_fee_per_blob_gas: 0x0506_0708,
            input: bytes!("c0ffee"),
        })),
        EthereumTxEnvelope::Eip7702(signed(TxEip7702 {
            chain_id: 4321,
            nonce: 0x0102,
            gas_limit: 0x0708_0910,
            max_fee_per_gas: 0x0304_0506,
            max_priority_fee_per_gas: 0x0102_0304,
            to: ADDR_B,
            value: U256::from(0x1234_5678u64),
            access_list: access_list(),
            authorization_list: vec![SignedAuthorization::new_unchecked(
                Authorization {
                    chain_id: U256::from(4321),
                    address: ADDR_A,
                    nonce: 0x0203,
                },
                1,
                U256::from(0x1111_2222u64),
                U256::from(0x3333_4444u64),
            )],
            input: bytes!("c0ffee"),
        })),
    ]
}

fn receipt() -> Receipt {
    Receipt {
        receipt: EthReceipt {
            tx_type: TxType::Eip1559,
            success: true,
            cumulative_gas_used: 0x0102_0304,
            logs: vec![Log {
                address: ADDR_A,
                data: LogData::new_unchecked(vec![HASH_A, HASH_B], bytes!("abcdef")),
            }],
        },
        transaction_hash: HASH_B,
        transaction_index: 0x0203,
        block_number: 0x0102_0304,
        gas_used: 21_000_456,
        log_index_start: 0x0304,
    }
}

#[test]
fn db_account_layout_is_stable() {
    let with_code = DbAccount(AccountInfo {
        balance: U256::from(0x1234_5678_9abc_def0u64),
        nonce: 0x0102,
        code_hash: HASH_A,
        code: Some(legacy_bytecode()),
        account_id: None,
    });
    let without_code = DbAccount(AccountInfo {
        balance: U256::from(0x1234_5678_9abc_def0u64),
        nonce: 0x0102,
        code_hash: HASH_A,
        code: None,
        account_id: None,
    });

    assert_bcs_layout(&with_code, "20000000000000000000000000000000000000000000000000123456789abcdef00201000000000000200102030405060708091011121314151617181920212223242526272829303132010009608060405260005b000900000000000000136269747665633a3a6f726465723a3a4c73623008000900000000000000028000", "DbAccount with code");
    assert_bcs_layout(&without_code, "20000000000000000000000000000000000000000000000000123456789abcdef0020100000000000020010203040506070809101112131415161718192021222324252627282930313200", "DbAccount without code");
}

#[test]
fn bytecode_layout_is_stable() {
    assert_bcs_layout(&legacy_bytecode(), "0009608060405260005b000900000000000000136269747665633a3a6f726465723a3a4c73623008000900000000000000028000", "legacy bytecode");
    assert_bcs_layout(
        &Bytecode::new_eip7702(ADDR_B),
        "0114fedcba0987654321fedcba0987654321fedcba09",
        "EIP-7702 bytecode",
    );
}

#[test]
fn block_env_layout_is_stable() {
    let block_env = BlockEnv {
        number: U256::from(0x0102_0304u64),
        beneficiary: ADDR_A,
        timestamp: U256::from(0x6543_2100u64),
        gas_limit: 30_000_123,
        basefee: 0x1234,
        difficulty: U256::from(0x1234_5678u64),
        prevrandao: Some(HASH_A),
        blob_excess_gas_and_price: Some(BlobExcessGasAndPrice {
            excess_blob_gas: 0x3456,
            blob_gasprice: 0x0102_0304_0506,
        }),
        ..Default::default()
    };

    assert_bcs_layout(&block_env, "200000000000000000000000000000000000000000000000000000000001020304141234567890abcdef1234567890abcdef12345678200000000000000000000000000000000000000000000000000000000065432100fbc3c90100000000341200000000000020000000000000000000000000000000000000000000000000000000001234567801200102030405060708091011121314151617181920212223242526272829303132015634000000000000060504030201000000000000000000000000000000000000", "BlockEnv");
}

#[test]
fn spec_id_layout_is_stable() {
    let all_known_specs = vec![
        SpecId::FRONTIER,
        SpecId::HOMESTEAD,
        SpecId::TANGERINE,
        SpecId::SPURIOUS_DRAGON,
        SpecId::BYZANTIUM,
        SpecId::PETERSBURG,
        SpecId::ISTANBUL,
        SpecId::BERLIN,
        SpecId::LONDON,
        SpecId::MERGE,
        SpecId::SHANGHAI,
        SpecId::CANCUN,
        SpecId::PRAGUE,
        SpecId::OSAKA,
        SpecId::AMSTERDAM,
    ];

    assert_bcs_layout(
        &all_known_specs,
        "0f000102030405060708090a0b0c0d0e",
        "SpecId",
    );
}

#[test]
fn evm_runtime_config_layout_is_stable() {
    let config = EvmRuntimeConfig {
        chain_spec: EvmChainSpec {
            limit_contract_code_size: Some(0x6000),
            coinbase: ADDR_A,
            block_gas_limit: 30_000_123,
            tx_gas_limit: Some(16_777_216),
            hardforks: vec![(0, SpecId::CANCUN), (0x0102, SpecId::PRAGUE)],
        },
        hardforks: vec![(0, SpecId::CANCUN), (0x0102, SpecId::PRAGUE)],
        contract_creation_policy: ContractCreationPolicy::Allowlist([ADDR_B].into()),
    };

    assert_bcs_layout(&config, "010060000000000000141234567890abcdef1234567890abcdef12345678fbc3c901000000000100000001000000000200000000000000000b02010000000000000c0200000000000000000b02010000000000000c010114fedcba0987654321fedcba0987654321fedcba09", "EvmRuntimeConfig");
}

#[test]
fn block_layout_is_stable() {
    let block = Block {
        header: header(),
        transactions: 0x0102..0x0203,
    };

    assert_bcs_layout(&block, "20010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d5141234567890abcdef1234567890abcdef1234567820a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d520010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d50120a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d580025a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a2000000000000000000000000000000000000000000000000000000000123456780403020100000000fbc3c9010000000008714001000000000021436500000000200102030405060708091011121314151617181920212223242526272829303132080102030405060708013412000000000000014523000000000000015634000000000000012001020304050607080910111213141516171819202122232425262728293031320120a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d5000004deadbeef02010000000000000302000000000000", "Block");
    assert_bcs_layout(&block.seal(), "20010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d5141234567890abcdef1234567890abcdef1234567820a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d520010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d50120a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d580025a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a2000000000000000000000000000000000000000000000000000000000123456780403020100000000fbc3c9010000000008714001000000000021436500000000200102030405060708091011121314151617181920212223242526272829303132080102030405060708013412000000000000014523000000000000015634000000000000012001020304050607080910111213141516171819202122232425262728293031320120a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d5000004deadbeef209b185dc69cb205e5498c6a116c7abfded2fbe14b18c15d02430583eb01f4028c020100000000000003020000000000000000000000000000", "SealedBlock");
}

#[test]
fn transaction_layout_is_stable() {
    let expected = [
        "141234567890abcdef1234567890abcdef1234567820840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca900585652025e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d108000000000000000000010800000000000010e102010000000000000605040300000000000000000000000010090807000000000114fedcba0987654321fedcba0987654321fedcba0920000000000000000000000000000000000000000000000000000000001234567803c0ffee0403020100000000",
        "141234567890abcdef1234567890abcdef1234567820840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca900585652025e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d108000000000000000001e1100000000000000201000000000000060504030000000000000000000000001009080700000000002000000000000000000000000000000000000000000000000000000000123456780114fedcba0987654321fedcba0987654321fedcba090220010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d503c0ffee0403020100000000",
        "141234567890abcdef1234567890abcdef1234567820840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca900585652025e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d108000000000000000002e1100000000000000201000000000000100908070000000006050403000000000000000000000000040302010000000000000000000000000114fedcba0987654321fedcba0987654321fedcba092000000000000000000000000000000000000000000000000000000000123456780114fedcba0987654321fedcba0987654321fedcba090220010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d503c0ffee0403020100000000",
        "141234567890abcdef1234567890abcdef1234567820840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca900585652025e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1080000000000000000030800000000000010e10800000000000001020800000000070809101000000000000000000000000003040506100000000000000000000000000102030414fedcba0987654321fedcba0987654321fedcba092000000000000000000000000000000000000000000000000000000000123456780114fedcba0987654321fedcba0987654321fedcba090220010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d50120a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d5100000000000000000000000000506070803c0ffee0403020100000000",
        "141234567890abcdef1234567890abcdef1234567820840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca900585652025e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d108000000000000000004e11000000000000002010000000000001009080700000000060504030000000000000000000000000403020100000000000000000000000014fedcba0987654321fedcba0987654321fedcba092000000000000000000000000000000000000000000000000000000000123456780114fedcba0987654321fedcba0987654321fedcba090220010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d5012000000000000000000000000000000000000000000000000000000000000010e1141234567890abcdef1234567890abcdef12345678080000000000000203010120000000000000000000000000000000000000000000000000000000001111222220000000000000000000000000000000000000000000000000000000003333444403c0ffee0403020100000000",
    ];

    for (tx, expected_hex) in transactions().into_iter().zip(expected) {
        let tx_type = tx.tx_type();
        let stored = TxSignedAndRecovered {
            signer: ADDR_A,
            signed_transaction: tx,
            block_number: 0x0102_0304,
        };
        assert_bcs_layout(&stored, expected_hex, &format!("{tx_type:?} transaction"));
    }
}

#[test]
fn receipt_layout_is_stable() {
    let time = Time::from_millis(0x0102_0304_0506);

    assert_bcs_layout(&(receipt(), time), "010201040302010000000001141234567890abcdef1234567890abcdef123456780220010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d503abcdef20a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d503020000000000000403020100000000087140010000000004030000000000000605040302010000", "Receipt with time");
}

#[test]
fn pending_transaction_layout_is_stable() {
    let pending = PendingTransaction::new(
        TxSignedAndRecovered {
            signer: ADDR_A,
            signed_transaction: transactions().remove(2),
            block_number: 0x0102_0304,
        },
        receipt(),
        Time::from_millis(0x0102_0304_0506),
    );

    assert_bcs_layout(&pending, "141234567890abcdef1234567890abcdef1234567820840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca900585652025e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d108000000000000000002e1100000000000000201000000000000100908070000000006050403000000000000000000000000040302010000000000000000000000000114fedcba0987654321fedcba0987654321fedcba092000000000000000000000000000000000000000000000000000000000123456780114fedcba0987654321fedcba0987654321fedcba090220010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d503c0ffee0403020100000000010201040302010000000001141234567890abcdef1234567890abcdef123456780220010203040506070809101112131415161718192021222324252627282930313220a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d503abcdef20a1a2a3a4a5a6a7a8a9b1b2b3b4b5b6b7b8b9c1c2c3c4c5c6c7c8c9d1d2d3d4d503020000000000000403020100000000087140010000000004030000000000000605040302010000", "PendingTransaction");
}
