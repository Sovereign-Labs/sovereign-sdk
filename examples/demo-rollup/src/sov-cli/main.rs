#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    demo_stf::cli::run::<
        <celestia::CelestiaService as sov_rollup_interface::services::da::DaService>::Spec,
    >()
    .await
}
