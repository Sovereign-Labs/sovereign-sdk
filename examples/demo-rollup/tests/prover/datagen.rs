use std::env;
use std::num::NonZero;
use std::sync::Arc;

use alloy::consensus::{TxEip1559, TypedTransaction};
use alloy::eips::eip2718::Encodable2718;
use alloy_primitives::{Address, TxKind, U256};
use demo_stf::runtime::Runtime;
use secp256k1::SecretKey;
use sov_blob_storage::PreferredBatchData;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_eth_dev_signer::Signer;
use sov_evm::{EthereumAuthenticator, RlpEvmTransaction};
use sov_evm_test_utils::LegacySimpleStorage;
use sov_mock_da::{MockAddress, MockBlock, MockDaService};
use sov_modules_api::{FullyBakedTx, RawTx};
use sov_modules_macros::config_value;
use sov_rollup_interface::node::da::DaService;
use sov_test_utils::generators::bank::BankMessageGenerator;
use sov_test_utils::generators::BlobBuildingCtx;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::MessageGenerator;

type S = super::DefaultSpec;

pub const DEFAULT_BLOCKS: u64 = 1;
const DEFAULT_TXNS_PER_BLOCK: u64 = 2;
const SENDER_PRIV_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const EVM_GAS_LIMIT: u64 = 100_000_000;

pub async fn get_blocks_from_da(mode: BlobBuildingCtx) -> anyhow::Result<Vec<MockBlock>> {
    let txns_per_block = match env::var("SOV_BENCH_TXNS_PER_BLOCK") {
        Ok(txns_per_block) => txns_per_block.parse::<u64>()?,
        Err(_) => {
            println!("SOV_BENCH_TXNS_PER_BLOCK not set, using default");
            DEFAULT_TXNS_PER_BLOCK
        }
    };

    let block_cnt = match env::var("SOV_BENCH_BLOCKS") {
        Ok(block_cnt_str) => block_cnt_str.parse::<u64>()?,
        Err(_) => {
            println!("SOV_BENCH_BLOCKS not set, using default");
            DEFAULT_BLOCKS
        }
    };

    let da_service = MockDaService::new(MockAddress::default());
    let mut blocks = vec![];

    let private_key_and_address: PrivateKeyAndAddress<S> =
        read_private_key::<S>("minter_private_key.json");

    let (create_token_message_gen, transfer_message_gen) =
        BankMessageGenerator::generate_token_and_random_transfers(
            txns_per_block,
            private_key_and_address.private_key,
        );

    let blob = create_token_message_gen.create_blobs::<Runtime<S>>(&mode);
    da_service.send_transaction(&blob).await.await??;
    let block1 = da_service.get_block_at(1).await?;
    blocks.push(block1);

    for i in 0..block_cnt {
        let blob = transfer_message_gen.create_blobs::<Runtime<S>>(&mode);
        da_service.send_transaction(&blob).await.await??;
        let blocki = da_service.get_block_at(2 + i).await?;
        blocks.push(blocki);
    }

    Ok(blocks)
}

pub async fn get_evm_blocks_from_da(mode: BlobBuildingCtx) -> anyhow::Result<Vec<MockBlock>> {
    let da_service = MockDaService::new(MockAddress::default());
    let blob = create_evm_deploy_and_call_blob(&mode)?;

    da_service.send_transaction(&blob).await.await??;
    Ok(vec![da_service.get_block_at(1).await?])
}

fn create_evm_deploy_and_call_blob(mode: &BlobBuildingCtx) -> anyhow::Result<Vec<u8>> {
    let sender = EvmAccount::from_private_key_hex(SENDER_PRIV_KEY)?;
    let contract = LegacySimpleStorage::default();
    let contract_address = sender.address().create(0);

    // Deploy, then exercise several distinct storage-access / gas paths in the same block — a fresh
    // SSTORE, a read-modify-write, and an SSTORE-to-zero (clear) — so the native↔ZK consistency
    // check covers a range of state-access + gas behaviour rather than a single write.
    let txs = vec![
        encode_evm_tx(&sender, evm_tx(TxKind::Create, contract.byte_code(), 0))?,
        encode_evm_tx(
            &sender,
            evm_tx(TxKind::Call(contract_address), contract.set(0x12345678), 1),
        )?,
        encode_evm_tx(
            &sender,
            evm_tx(TxKind::Call(contract_address), contract.inc(), 2),
        )?,
        encode_evm_tx(
            &sender,
            evm_tx(TxKind::Call(contract_address), contract.set(0), 3),
        )?,
    ];

    Ok(create_blob(txs, mode))
}

fn evm_tx(to: TxKind, input: alloy_primitives::Bytes, nonce: u64) -> TxEip1559 {
    TxEip1559 {
        chain_id: config_value!("CHAIN_ID"),
        nonce,
        gas_limit: EVM_GAS_LIMIT,
        max_fee_per_gas: config_value!("INITIAL_BASE_FEE_PER_GAS")[0] as u128 * 2,
        to,
        value: U256::ZERO,
        input,
        ..Default::default()
    }
}

fn encode_evm_tx(account: &EvmAccount, tx: TxEip1559) -> anyhow::Result<FullyBakedTx> {
    let signed_eth_tx = account.sign(TypedTransaction::Eip1559(tx))?;
    let raw_tx = RawTx::new(borsh::to_vec(&signed_eth_tx)?);
    Ok(<Runtime<S> as EthereumAuthenticator<S>>::encode_with_ethereum_auth(raw_tx))
}

fn create_blob(txs: Vec<FullyBakedTx>, mode: &BlobBuildingCtx) -> Vec<u8> {
    match mode {
        BlobBuildingCtx::Standard => borsh::to_vec(&txs).unwrap(),
        BlobBuildingCtx::Preferred {
            curr_sequence_number,
        } => {
            let batch = PreferredBatchData {
                data: Arc::new(txs),
                sequence_number: curr_sequence_number
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                visible_slots_to_advance: NonZero::new(1).unwrap(),
            };

            borsh::to_vec(&batch).unwrap()
        }
    }
}

struct EvmAccount(SecretKey);

impl EvmAccount {
    fn from_private_key_hex(hex_key: &str) -> anyhow::Result<Self> {
        let key_bytes = hex::decode(hex_key.trim_start_matches("0x"))?;
        Ok(Self(SecretKey::from_slice(&key_bytes)?))
    }

    fn address(&self) -> Address {
        Signer::new(self.0).address()
    }

    fn sign(&self, tx: TypedTransaction) -> anyhow::Result<RlpEvmTransaction> {
        let signer = Signer::new(self.0);
        let signed_tx = signer.sign_transaction(tx)?;
        Ok(RlpEvmTransaction {
            rlp: signed_tx.encoded_2718(),
        })
    }
}
