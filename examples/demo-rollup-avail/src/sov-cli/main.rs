#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    demo_stf::cli::run::<
        <presence::service::DaProvider as sov_rollup_interface::services::da::DaService>::Spec,
    >()
    .await
}
