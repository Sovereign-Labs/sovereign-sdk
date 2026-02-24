//! Docker related test-utils
use testcontainers::ContainerAsync;
use tokio::io::AsyncBufReadExt;

/// Printing logs from container
pub async fn print_logs_from_container<T>(name: &str, container: &ContainerAsync<T>)
where
    T: testcontainers::Image,
{
    let _span = tracing::info_span!("docker_log", name = name).entered();
    eprintln!("[{name}] container stdout:");
    let mut stdout = container.stdout(false).lines();
    while let Some(line) = stdout.next_line().await.unwrap() {
        tracing::info!("stdout: {line}");
        eprintln!("[{name}] stdout: {line}");
    }

    eprintln!("[{name}] container stderr:");
    let mut stderr = container.stderr(false).lines();
    while let Some(line) = stderr.next_line().await.unwrap() {
        tracing::info!("stderr: {line}");
        eprintln!("[{name}] stderr: {line}");
    }
}
