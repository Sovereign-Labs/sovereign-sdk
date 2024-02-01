use std::str::FromStr;

use anyhow::Result;
use avail_subxt::api::runtime_types::da_control::pallet::Call as DaCall;
use avail_subxt::api::runtime_types::sp_core::bounded::bounded_vec::BoundedVec;
use avail_subxt::api::{self};
use avail_subxt::avail::AppUncheckedExtrinsic;
use avail_subxt::primitives::AvailExtrinsicParams;
use avail_subxt::{build_client, AvailConfig, Call};
use sp_core::crypto::Pair as PairTrait;
use sp_keyring::sr25519::sr25519::{self, Pair};
use structopt::StructOpt;
use subxt::tx::PairSigner;

#[derive(Debug)]
struct HexData(Vec<u8>);

impl FromStr for HexData {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode(s).map(HexData)
    }
}

#[derive(Debug, StructOpt)]
struct Opts {
    /// The WebSocket address of the target the Avail Node,
    #[structopt(name = "ws_uri", long, default_value = "ws://127.0.0.1:9944")]
    pub ws: String,

    /// Check whether the Client you are using is aligned with the statically generated codegen.
    #[structopt(name = "validate_codegen", short = "c", long)]
    pub validate_codegen: bool,

    #[structopt(
        name = "seed",
        long,
        default_value = "rose label choose orphan garlic upset scout payment first have boil stamp"
    )]
    pub seed: String,

    #[structopt(name = "app_id", long, default_value = "0")]
    pub app_id: u32,

    #[structopt(name = "tx_blob", long, default_value = "example")]
    pub tx_blob: HexData,
}

/// This example submits an Avail data extrinsic, then retrieves the block containing the
/// extrinsic and matches the data.
#[async_std::main]
async fn main() -> Result<()> {
    let args = Opts::from_args();

    let pair = Pair::from_phrase(&args.seed, None).unwrap();
    let signer = PairSigner::<AvailConfig, sr25519::Pair>::new(pair.0.clone());

    let client = build_client(args.ws, args.validate_codegen).await?;
    let example_data = args.tx_blob.0;

    let data_transfer = api::tx()
        .data_availability()
        .submit_data(BoundedVec(example_data.clone()));
    let extrinsic_params = AvailExtrinsicParams::new_with_app_id(args.app_id.into());

    let h = client
        .tx()
        .sign_and_submit_then_watch(&data_transfer, &signer, extrinsic_params)
        .await?
        .wait_for_finalized_success()
        .await?;

    println!("receipt hash{:#?}", h.extrinsic_hash());

    let submitted_block = client.rpc().block(Some(h.block_hash())).await?.unwrap();

    let matched_xt = submitted_block
        .block
        .extrinsics
        .into_iter()
        .filter_map(|chain_block_ext| {
            AppUncheckedExtrinsic::try_from(chain_block_ext)
                .map(|ext| ext.function)
                .ok()
        })
        .find(|call| match call {
            Call::DataAvailability(DaCall::submit_data { data }) => data.0 == example_data,
            _ => false,
        });

    assert!(matched_xt.is_some(), "Submitted data not found");

    Ok(())
}
