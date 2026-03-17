//! Docker related test-utils
use std::time::Duration;

use anyhow::anyhow;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use tokio::io::AsyncBufReadExt;

/// Retry delays for image pull failures. Base (non-transient) errors use only
/// the first [`BASE_RETRY_COUNT`] entries; known transient errors use all of them.
const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(20),
];
/// How many retries a non-transient pull error gets (→ 3 attempts total).
const BASE_RETRY_COUNT: usize = 2;

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

/// Pulls a docker image with retries for pull failures. All errors are retried,
/// with known transient errors receiving more retry attempts.
pub async fn pull_image_with_retries<I>(image: I) -> anyhow::Result<()>
where
    I: testcontainers::Image + Clone,
{
    let image_name = image.name().to_owned();
    let image_tag = image.tag().to_owned();

    for attempt in 1..=RETRY_DELAYS.len() + 1 {
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
                let delays: &[Duration] = if is_retryable_pull_error_message(&err_text) {
                    &RETRY_DELAYS
                } else {
                    &RETRY_DELAYS[..BASE_RETRY_COUNT]
                };

                if let Some(&delay) = delays.get(attempt.saturating_sub(1)) {
                    tracing::warn!(
                        attempt,
                        max_attempts = delays.len() + 1,
                        image = image_name,
                        tag = image_tag,
                        %err,
                        ?delay,
                        "Image pull failure, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }

                return Err(anyhow!(
                    "failed to pull image {image_name}:{image_tag} after {attempt} attempt(s): \
                     {err}. Hint: verify registry connectivity or pre-pull the image before tests."
                ));
            }
        }
    }

    unreachable!("loop always returns on Ok or final Err")
}

fn is_retryable_pull_error_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let is_pull_error = message.contains("pullimage")
        || message.contains("pull image")
        || message.contains("pull the image");
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
    fn pull_the_image_timeout_errors_are_retryable() {
        let err = "failed to pull the image \
                   'ghcr.io/ross-weir/hyperlane-agent:integration-lander-1', \
                   error: Timeout error";
        assert!(is_retryable_pull_error_message(err));
    }

    #[test]
    fn retry_delays_and_base_limit() {
        assert_eq!(
            RETRY_DELAYS.len(),
            4,
            "transient errors get 4 retries → 5 attempts"
        );
        assert_eq!(
            BASE_RETRY_COUNT, 2,
            "base errors get 2 retries → 3 attempts"
        );
        assert_eq!(RETRY_DELAYS[0], Duration::from_secs(2));
        assert_eq!(RETRY_DELAYS[1], Duration::from_secs(5));
        assert_eq!(RETRY_DELAYS[2], Duration::from_secs(10));
        assert_eq!(RETRY_DELAYS[3], Duration::from_secs(20));
    }
}
