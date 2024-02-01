// ----------------------------------------------------------------------------
// This file includes code adapted from the "yellowstone-grpc" project's example client:
// https://github.com/rpcpool/yellowstone-grpc/blob/master/examples/rust/src/bin/client.rs
//
// Credit to the original authors and contributors of the "yellowstone-grpc" project for their work.
// ----------------------------------------------------------------------------

use std::collections::HashMap;
use std::time::Duration;

use backoff::future::retry;
use backoff::ExponentialBackoff;
use clap::Parser;
use da_client::hash_solana_account;
use futures::future::TryFutureExt;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use log::{error, info};
use yellowstone_grpc_client::{GeyserGrpcClient, GeyserGrpcClientError};
use yellowstone_grpc_proto::prelude::subscribe_update::UpdateOneof;
use yellowstone_grpc_proto::prelude::{
    SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterBlocks,
    SubscribeRequestFilterBlocksMeta, SubscribeRequestFilterEntry, SubscribeRequestFilterSlots,
    SubscribeRequestFilterTransactions, SubscribeUpdateAccount,
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
    let slots: SlotsFilterMap = HashMap::new();
    let transactions: TransactionsFilterMap = HashMap::new();
    let entry: EntryFilterMap = HashMap::new();
    let blocks: BlocksFilterMap = HashMap::new();
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
        account.lamports,
        &account.owner,
        account.executable,
        account.rent_epoch,
        &account.data,
        &pub_key,
    );
    println!(
        "slot:{:?}, pubkey:{:?}, hash:{:?}",
        slot_num,
        bs58::encode(pub_key).into_string(),
        bs58::encode(account_hash).into_string()
    );
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    info!("Starting");

    let cli = Cli::parse();
    // optional overrides
    let grpc_url = &cli.grpc_url;
    let mut maybe_first_attempt = Some(());

    retry(ExponentialBackoff::default(), move || {
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
