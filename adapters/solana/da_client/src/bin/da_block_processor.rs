use backoff::{future::retry, ExponentialBackoff};
use yellowstone_grpc_client::{GeyserGrpcClient, GeyserGrpcClientError};
use log::{error, info};
use futures::{future::TryFutureExt, sink::SinkExt, stream::StreamExt};
use yellowstone_grpc_proto::{
    prelude::{
        subscribe_request_filter_accounts_filter::Filter as AccountsFilterDataOneof,
        subscribe_request_filter_accounts_filter_memcmp::Data as AccountsFilterMemcmpOneof,
        subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest,
        SubscribeRequestAccountsDataSlice, SubscribeRequestFilterAccounts,
        SubscribeRequestFilterAccountsFilter, SubscribeRequestFilterAccountsFilterMemcmp,
        SubscribeRequestFilterBlocks, SubscribeRequestFilterBlocksMeta,
        SubscribeRequestFilterEntry, SubscribeRequestFilterSlots,
        SubscribeRequestFilterTransactions, SubscribeUpdateAccount, SubscribeUpdateTransaction,
    }
};
use std::{
    collections::HashMap,
    env, fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

use da_client::hash_solana_account;

type SlotsFilterMap = HashMap<String, SubscribeRequestFilterSlots>;
type AccountFilterMap = HashMap<String, SubscribeRequestFilterAccounts>;
type TransactionsFilterMap = HashMap<String, SubscribeRequestFilterTransactions>;
type EntryFilterMap = HashMap<String, SubscribeRequestFilterEntry>;
type BlocksFilterMap = HashMap<String, SubscribeRequestFilterBlocks>;
type BlocksMetaFilterMap = HashMap<String, SubscribeRequestFilterBlocksMeta>;


fn get_subscribe_request() -> SubscribeRequest {
    let mut accounts: AccountFilterMap = HashMap::new();
    let mut slots: SlotsFilterMap = HashMap::new();
    let mut transactions: TransactionsFilterMap = HashMap::new();
    let mut entry: EntryFilterMap = HashMap::new();
    let mut blocks: BlocksFilterMap = HashMap::new();
    let mut blocks_meta: BlocksMetaFilterMap = HashMap::new();
    let mut accounts_data_slice = Vec::new();

    accounts.insert(
        "client".to_owned(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![],
            filters: vec![],
        },
    );
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

fn print_account(sub_account: SubscribeUpdateAccount) {
    let slot_num = sub_account.slot;
    let account = sub_account.account.unwrap();
    let pub_key = account.pubkey.clone();
    let account_hash = hash_solana_account(
        account.lamports, &account.owner, account.executable, account.rent_epoch,
        &account.data, &pub_key);
    println!("slot:{:?}, pubkey:{:?}, hash:{:?}",slot_num,
             bs58::encode(pub_key).into_string(),
             bs58::encode(account_hash).into_string());
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let zero_attempts = Arc::new(Mutex::new(true));
    info!("Starting");

    retry(ExponentialBackoff::default(), move || {
        let zero_attempts = Arc::clone(&zero_attempts);

        async move {
            let mut zero_attempts = zero_attempts.lock().unwrap();
            if *zero_attempts {
                *zero_attempts = false;
            } else {
                info!("Retry to connect to the server");
            }

            let mut client = GeyserGrpcClient::connect_with_timeout(
                "http://127.0.0.1:10000",
                Option::<String>::None,
                None,
                Some(Duration::from_secs(10)),
                Some(Duration::from_secs(10)),
                false,
            )
                .await
                .map_err(|e| backoff::Error::transient(anyhow::Error::new(e)))?;

            let (mut subscribe_tx, mut stream) = client.subscribe()
                .await
                .map_err(|e| backoff::Error::Permanent(anyhow::Error::from(e)))?;

            subscribe_tx
                .send(get_subscribe_request())
                .await
                .map_err(|e| backoff::Error::Permanent(anyhow::Error::from(GeyserGrpcClientError::SubscribeSendError(e))))?;

            while let Some(message) = stream.next().await {
                match message {
                    Ok(msg) => {
                        #[allow(clippy::single_match)]
                        match msg.update_oneof {
                            Some(UpdateOneof::Account(account)) => {
                                print_account(account);
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
            .inspect_err(|error| error!("failed to connect: {error}"))
    })
        .await
        .map_err(Into::into)
}