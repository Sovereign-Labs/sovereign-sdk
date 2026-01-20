//! Docker related test-utils
use testcontainers::ContainerAsync;
use tokio::io::AsyncBufReadExt;

/// Printing logs from container (tailing the last `max_lines` lines).
pub async fn print_logs_from_container<T>(
    name: &str,
    container: &ContainerAsync<T>,
    max_lines: usize,
) where
    T: testcontainers::Image,
{
    eprintln!("[DOCKER][{name}] container stdout:");
    let mut stdout = container.stdout(false).lines();
    let mut stdout_lines = std::collections::VecDeque::new();
    while let Some(line) = stdout.next_line().await.unwrap() {
        if max_lines > 0 {
            if stdout_lines.len() == max_lines {
                stdout_lines.pop_front();
            }
            stdout_lines.push_back(line);
        }
    }
    for line in stdout_lines {
        eprintln!("[DOCKER][{name}][stdout] {line}");
    }

    eprintln!("[DOCKER][{name}] container stderr:");
    let mut stderr = container.stderr(false).lines();
    let mut stderr_lines = std::collections::VecDeque::new();
    while let Some(line) = stderr.next_line().await.unwrap() {
        if max_lines > 0 {
            if stderr_lines.len() == max_lines {
                stderr_lines.pop_front();
            }
            stderr_lines.push_back(line);
        }
    }
    for line in stderr_lines {
        eprintln!("[DOCKER][{name}][stderr] {line}");
    }
}
