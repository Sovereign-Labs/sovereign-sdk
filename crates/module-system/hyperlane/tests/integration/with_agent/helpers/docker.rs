use std::collections::VecDeque;
use std::time::Duration;
use testcontainers::core::client::docker_client_instance;
use testcontainers::core::ExecResult;
use tokio::io::AsyncBufReadExt;
use tokio::time::Instant;

pub async fn print_logs_from_exec_result(
    name: &str,
    result: &mut ExecResult,
    timeout: Duration,
    max_lines: usize,
) {
    let exit_code = result.exit_code().await.unwrap();
    eprintln!("[DOCKER][{name}] exit code: {exit_code:?}");
    let stdout_lines = read_lines_with_timeout(result.stdout().lines(), timeout, max_lines).await;
    eprintln!("[DOCKER][{name}] stdout:");
    for line in stdout_lines {
        eprintln!("[DOCKER][{name}][stdout] {line}");
    }
    eprintln!("[DOCKER][{name}] ---- end stdout ----");
    let stderr_lines = read_lines_with_timeout(result.stderr().lines(), timeout, max_lines).await;
    eprintln!("[DOCKER][{name}] stderr:");
    for line in stderr_lines {
        eprintln!("[DOCKER][{name}][stderr] {line}");
    }
    eprintln!("[DOCKER][{name}] ---- end stderr ----");
}

async fn read_lines_with_timeout<R>(
    mut lines: tokio::io::Lines<R>,
    timeout: Duration,
    max_lines: usize,
) -> Vec<String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let deadline = Instant::now() + timeout;
    let mut collected = VecDeque::new();
    let mut truncated = false;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                if max_lines > 0 {
                    if collected.len() == max_lines {
                        collected.pop_front();
                        truncated = true;
                    }
                    collected.push_back(line);
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(err)) => {
                eprintln!("[DOCKER][logs] Failed reading logs: {err}");
                break;
            }
            Err(_) => break,
        }
    }

    if truncated {
        collected.push_front(format!(
            "... truncated to last {max_lines} lines due to size/time limit ..."
        ));
    }

    collected.into_iter().collect()
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
