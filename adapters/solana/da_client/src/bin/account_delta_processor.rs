// ----------------------------------------------------------------------------
// This file includes code adapted from the "yellowstone-grpc" project's example client:
// https://github.com/rpcpool/yellowstone-grpc/blob/master/examples/rust/src/bin/client.rs
//
// Credit to the original authors and contributors of the "yellowstone-grpc" project for their work.
// ----------------------------------------------------------------------------

use std::collections::HashMap;
use std::str::FromStr;
use std::thread;
use std::time::Duration;

use backoff::future::retry;
use backoff::ExponentialBackoff;
use clap::Parser;
use crossbeam_channel::{select, unbounded};
use da_client::{calculate_root, hash_solana_account};
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use log::{error, info};
use solana_sdk::hash::{hashv, Hash};
use solana_sdk::pubkey::Pubkey;
use yellowstone_grpc_client::{GeyserGrpcClient, GeyserGrpcClientError};
use yellowstone_grpc_proto::prelude::subscribe_update::UpdateOneof;
use yellowstone_grpc_proto::prelude::{
    SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterBlocks,
    SubscribeRequestFilterBlocksMeta, SubscribeRequestFilterEntry, SubscribeRequestFilterSlots,
    SubscribeRequestFilterTransactions, SubscribeUpdateAccount, SubscribeUpdateBlock,
};

type SlotsFilterMap = HashMap<String, SubscribeRequestFilterSlots>;
type AccountFilterMap = HashMap<String, SubscribeRequestFilterAccounts>;
type TransactionsFilterMap = HashMap<String, SubscribeRequestFilterTransactions>;
type EntryFilterMap = HashMap<String, SubscribeRequestFilterEntry>;
type BlocksFilterMap = HashMap<String, SubscribeRequestFilterBlocks>;
type BlocksMetaFilterMap = HashMap<String, SubscribeRequestFilterBlocksMeta>;

const DEFAULT_GRPC_URL: &str = "http://127.0.0.1:10000";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, default_value_t=DEFAULT_GRPC_URL.to_string())]
    /// URL for solana RPC
    grpc_url: String,
}

fn get_subscribe_request() -> SubscribeRequest {
    let mut accounts: AccountFilterMap = HashMap::new();
    let mut slots: SlotsFilterMap = HashMap::new();
    let transactions: TransactionsFilterMap = HashMap::new();
    let entry: EntryFilterMap = HashMap::new();
    let mut blocks: BlocksFilterMap = HashMap::new();
    let blocks_meta: BlocksMetaFilterMap = HashMap::new();
    let accounts_data_slice = Vec::new();

    accounts.insert(
        "client".to_owned(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![],
            filters: vec![],
        },
    );
    blocks.insert(
        "client".to_owned(),
        SubscribeRequestFilterBlocks {
            account_include: vec![],
            include_transactions: Some(true),
            include_accounts: Some(false),
            include_entries: Some(false),
        },
    );
    slots.insert("client".to_owned(), SubscribeRequestFilterSlots {});

    SubscribeRequest {
        slots,
        accounts,
        transactions,
        entry,
        blocks,
        blocks_meta,
        commitment: Some(1),
        accounts_data_slice,
    }
}

struct BlockInfoForBankHash {
    pub blockhash: Hash,
    pub parent_bankhash: Hash,
    pub num_sigs: u64,
    pub updated_account_count: u64,
}

fn process_block(
    slot_accumulator: &mut HashMap<u64, HashMap<Pubkey, Hash>>,
    block_info: &mut HashMap<u64, BlockInfoForBankHash>,
) -> anyhow::Result<()> {
    let mut to_remove: Vec<u64> = Vec::new();

    for (pending_slotnum, pending_block) in block_info.iter() {
        let acc_hashes = match slot_accumulator.get(pending_slotnum) {
            Some(hashes) => hashes,
            None => continue,
        };
        if (acc_hashes.len() as u64) != pending_block.updated_account_count {
            continue;
        }
        let accounts_delta_hash = calculate_root(
            acc_hashes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        let bank_hash = hashv(&[
            pending_block.parent_bankhash.as_ref(),
            accounts_delta_hash.as_ref(),
            &pending_block.num_sigs.to_le_bytes(),
            pending_block.blockhash.as_ref(),
        ]);
        info!("CALCULATED: {:?}: {:?} ", pending_slotnum, bank_hash);
        info!(
            "FROM GEYSER: {:?}: {:?} ",
            pending_slotnum - 1,
            pending_block.parent_bankhash
        );
        to_remove.push(*pending_slotnum);
    }
    for slotnum in to_remove {
        block_info.remove(&slotnum);
        slot_accumulator.remove(&slotnum);
    }

    Ok(())
}

fn generate_proofs(
    r_account: crossbeam::channel::Receiver<SubscribeUpdateAccount>,
    r_block: crossbeam::channel::Receiver<SubscribeUpdateBlock>,
) {
    let mut slot_accumulator: HashMap<u64, HashMap<Pubkey, Hash>> = HashMap::new();
    let mut block_info: HashMap<u64, BlockInfoForBankHash> = HashMap::new();

    loop {
        select! {
            recv(r_account) -> sub_account_msg => {
                let sub_account = sub_account_msg.unwrap();
                let slot_num = sub_account.slot;
                let account = sub_account.account.unwrap();
                let pub_key = account.pubkey.clone();
                let account_hash = hash_solana_account(
                    account.lamports,
                    &account.owner,
                    account.executable,
                    account.rent_epoch,
                    &account.data,
                    &pub_key,
                );
                let account_map = slot_accumulator.entry(slot_num).or_default();
                account_map.insert(Pubkey::try_from(pub_key.clone()).unwrap(), Hash::from(account_hash));
            }
            recv(r_block) -> sub_account_block => {
                let sub_block = sub_account_block.unwrap();
                let slot_num = sub_block.slot;
                let blockhash = sub_block.blockhash;
                let updated_account_count = sub_block.updated_account_count;
                let parent_bankhash = sub_block.parent_blockhash;
                let num_sigs: usize = sub_block.transactions.iter()
                    .filter_map(|t| t.transaction.as_ref())
                    .map(|transaction| transaction.signatures.len())
                    .sum();
                block_info.insert(slot_num, BlockInfoForBankHash {
                        blockhash: Hash::from_str(&blockhash).unwrap(),
                        parent_bankhash: Hash::from_str(&parent_bankhash).unwrap(),
                        num_sigs: num_sigs as u64,
                        updated_account_count
                });
                process_block(&mut slot_accumulator, &mut block_info);
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    info!("Starting");

    let cli = Cli::parse();
    // optional overrides
    let grpc_url = &cli.grpc_url;
    let mut maybe_first_attempt = Some(());

    let (s_account, r_account) = unbounded::<SubscribeUpdateAccount>();
    let (s_block, r_block) = unbounded::<SubscribeUpdateBlock>();

    thread::spawn(move || {
        generate_proofs(r_account, r_block);
    });

    retry(ExponentialBackoff::default(), move || {
        let sender_account = s_account.clone();
        let sender_block = s_block.clone();

        async move {
            if maybe_first_attempt.take().is_none() {
                info!("Retry to connect to the server");
            }
            let mut client = GeyserGrpcClient::connect_with_timeout(
                grpc_url.to_string(),
                Option::<String>::None,
                None,
                Some(Duration::from_secs(10)),
                Some(Duration::from_secs(10)),
                false,
            )
            .await
            .map_err(|e| backoff::Error::transient(anyhow::Error::new(e)))?;

            let (mut subscribe_tx, mut stream) = client
                .subscribe()
                .await
                .map_err(|e| backoff::Error::Permanent(anyhow::Error::from(e)))?;

            subscribe_tx
                .send(get_subscribe_request())
                .await
                .map_err(|e| {
                    backoff::Error::Permanent(anyhow::Error::from(
                        GeyserGrpcClientError::SubscribeSendError(e),
                    ))
                })?;

            while let Some(message) = stream.next().await {
                match message {
                    Ok(msg) =>
                    {
                        #[allow(clippy::single_match)]
                        match msg.update_oneof {
                            Some(UpdateOneof::Account(account)) => {
                                sender_account.send(account).unwrap();
                                continue;
                            }
                            Some(UpdateOneof::Block(block)) => {
                                sender_block.send(block).unwrap();
                                continue;
                            }
                            _ => {}
                        }
                    }
                    Err(error) => {
                        error!("error: {error:?}");
                        break;
                    }
                }
            }
            Ok::<(), backoff::Error<anyhow::Error>>(())
        }
    })
    .await
    .map_err(Into::into)
}
