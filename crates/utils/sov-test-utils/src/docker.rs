//! Docker related test-utils
use std::time::Duration;

use anyhow::anyhow;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use tokio::io::AsyncBufReadExt;

const PULL_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];
const PULL_MAX_ATTEMPTS: usize = PULL_RETRY_DELAYS.len() + 1;

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

/// Pulls a docker image with retries for transient pull failures.
pub async fn pull_image_with_retries<I>(image: I) -> anyhow::Result<()>
where
    I: testcontainers::Image + Clone,
{
    let image_name = image.name().to_owned();
    let image_tag = image.tag().to_owned();

    for attempt in 1..=PULL_MAX_ATTEMPTS {
        match image.clone().pull_image().await {
            Ok(_) => {
                if attempt > 1 {
                    tracing::info!(
                        attempt,
                        image = image_name,
                        tag = image_tag,
                        "Successfully pulled image after retry"
                    );
                }
                return Ok(());
            }
            Err(err) => {
                let err_text = err.to_string();
                if is_retryable_pull_error_message(&err_text) {
                    if let Some(delay) = pull_retry_delay(attempt) {
                        tracing::warn!(
                            attempt,
                            max_attempts = PULL_MAX_ATTEMPTS,
                            image = image_name,
                            tag = image_tag,
                            %err,
                            ?delay,
                            "Transient image pull failure, retrying"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                }

                return Err(anyhow!(
                    "failed to pull image {image_name}:{image_tag} after {attempt} attempt(s): \
                     {err}. Hint: verify registry connectivity or pre-pull the image before tests."
                ));
            }
        }
    }

    Err(anyhow!(
        "failed to pull image {image_name}:{image_tag} after {PULL_MAX_ATTEMPTS} attempt(s)"
    ))
}

fn pull_retry_delay(attempt: usize) -> Option<Duration> {
    PULL_RETRY_DELAYS.get(attempt.saturating_sub(1)).copied()
}

fn is_retryable_pull_error_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let is_pull_error = message.contains("pullimage") || message.contains("pull image");
    if !is_pull_error {
        return false;
    }

    [
        "requesttimeouterror",
        "timeout",
        "timed out",
        "connection reset",
        "connection refused",
        "error trying to connect",
        "temporary failure",
        "temporarily unavailable",
        "tls handshake timeout",
        "i/o error",
        "eof",
        "dns",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_timeout_errors_are_retryable() {
        let err = "Client(PullImage { descriptor: \
                   \"ghcr.io/foundry-rs/foundry:v1.3.6\", err: RequestTimeoutError })";
        assert!(is_retryable_pull_error_message(err));
    }

    #[test]
    fn pull_auth_errors_are_not_retryable() {
        let err = "Client(PullImage { descriptor: \"ghcr.io/foundry-rs/foundry:v1.3.6\", \
                   err: DockerResponseServerError { status_code: 401, message: \
                   \"unauthorized\" } })";
        assert!(!is_retryable_pull_error_message(err));
    }

    #[test]
    fn non_pull_timeouts_are_not_retryable() {
        let err = "ContainerWait(WaitError { source: RequestTimeoutError })";
        assert!(!is_retryable_pull_error_message(err));
    }

    #[test]
    fn pull_retry_delays_match_policy() {
        assert_eq!(pull_retry_delay(1), Some(Duration::from_secs(2)));
        assert_eq!(pull_retry_delay(2), Some(Duration::from_secs(5)));
        assert_eq!(pull_retry_delay(3), Some(Duration::from_secs(10)));
        assert_eq!(pull_retry_delay(4), None);
    }
}
