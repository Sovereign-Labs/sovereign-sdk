use std::time::Duration;
use testcontainers::core::client::docker_client_instance;
use testcontainers::core::ExecResult;
use testcontainers::ContainerAsync;
use tokio::io::AsyncBufReadExt;

pub async fn print_logs_from_container<T>(name: &str, container: &ContainerAsync<T>)
where
    T: testcontainers::Image,
{
    let _span = tracing::info_span!("docker_log", name = name).entered();
    let mut stdout = container.stdout(false).lines();
    while let Some(line) = stdout.next_line().await.unwrap() {
        tracing::info!("stdout: {line}");
    }

    let mut stderr = container.stderr(false).lines();
    while let Some(line) = stderr.next_line().await.unwrap() {
        tracing::info!("stderr: {line}");
    }
}

pub async fn print_logs_from_exec_result(name: &str, result: &mut ExecResult, timeout: Duration) {
    let _span = tracing::info_span!("docker_exec_log", name = name).entered();
    let exit_code = result.exit_code().await.unwrap();
    tracing::info!("Exit code  {exit_code:?}");
    let _ = tokio::time::timeout(timeout, async {
        tracing::info!("Printing stdout");
        let mut stdout = result.stdout().lines();
        while let Some(line) = stdout.next_line().await.unwrap() {
            tracing::info!("stdout: {line}");
        }
    })
    .await;
    tracing::info!("---- Done printing stdout");
    let _ = tokio::time::timeout(timeout, async {
        tracing::info!("Printing stderr");
        let mut stderr = result.stderr().lines();
        while let Some(line) = stderr.next_line().await.unwrap() {
            tracing::info!("stderr: {line}");
        }
    })
    .await;
    tracing::info!("---- Done printing stderr");
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub async fn get_docker_gateway_ip() -> String {
    let bridge_info = docker_client_instance()
        .await
        .unwrap()
        .inspect_network(
            "bridge",
            None::<testcontainers::bollard::query_parameters::InspectNetworkOptions>,
        )
        .await
        .unwrap();
    bridge_info
        .ipam
        .expect("no IPAM driver found")
        .config
        .expect("IPAM has no configuration")
        .into_iter()
        .find_map(|conf| conf.gateway)
        .expect("No gateway config in IPAM")
}
