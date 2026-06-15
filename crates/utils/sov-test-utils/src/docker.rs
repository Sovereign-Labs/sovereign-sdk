//! Docker related test-utils
use std::time::Duration;

use testcontainers::core::client::docker_client_instance;
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

/// Best-effort pre-pull of a Docker image with retries.
///
/// This is an optimization, not a guarantee: it warms the local cache and
/// surfaces explicit retries before the caller invokes `start()`. It never
/// returns an error — `start()` remains the actual gate, and any genuine
/// problem (image truly missing, daemon down, etc.) is reported there.
///
/// Behavior:
/// - **Cache hit** (image already present locally): logs and returns
///   immediately, with no registry call. Avoids burning ghcr.io's
///   anonymous-pull rate budget on no-ops.
/// - **Cache miss + pull succeeds**: logs and returns.
/// - **Cache miss + transient registry error**: backs off and retries, up to
///   `RETRY_DELAYS.len()` times for known-transient errors and
///   `BASE_RETRY_COUNT` times otherwise.
/// - **Cache miss + auth error (401/403/access denied)**: logs an error
///   pointing at registry auth (since we already verified the image is not
///   cached) and returns. Caller's `start()` will fail next, and the log
///   tells you the real reason.
/// - **Cache miss + retries exhausted**: logs an error and returns. Same
///   contract as above — caller's `start()` is the real failure surface.
///
/// Cache check uses bollard's `inspect_image(name:tag)`, which resolves a
/// local image by tag. For the pinned versioned tags this repo uses
/// (`v1.3.6`, `integration-lander-1`, etc.), this is effectively certain.
/// For floating tags it could be stale, but no caller in this repo uses one.
pub async fn prepull_image_best_effort<I>(image: I)
where
    I: testcontainers::Image + Clone,
{
    let image_name = image.name().to_owned();
    let image_tag = image.tag().to_owned();
    let image_ref = format!("{image_name}:{image_tag}");

    if is_image_cached_locally(&image_ref).await {
        tracing::info!(
            image = image_ref,
            "Image already cached locally, skipping pre-pull"
        );
        return;
    }

    for attempt in 1..=RETRY_DELAYS.len() + 1 {
        match image.clone().pull_image().await {
            Ok(_) => {
                if attempt > 1 {
                    tracing::info!(
                        attempt,
                        image = image_ref,
                        "Successfully pulled image after retry"
                    );
                }
                return;
            }
            Err(err) => {
                let err_text = err.to_string().to_ascii_lowercase();

                if is_registry_auth_error(&err_text) {
                    tracing::error!(
                        %err,
                        image = image_ref,
                        "Registry auth blocked AND image not cached locally — \
                         container start will fail. Hint: ghcr.io anonymous \
                         rate limit, or genuine auth issue."
                    );
                    return;
                }

                let delays: &[Duration] = if is_retryable_pull_error_message(&err_text) {
                    &RETRY_DELAYS
                } else {
                    &RETRY_DELAYS[..BASE_RETRY_COUNT]
                };

                if let Some(&delay) = delays.get(attempt.saturating_sub(1)) {
                    tracing::warn!(
                        attempt,
                        max_attempts = delays.len() + 1,
                        image = image_ref,
                        %err,
                        ?delay,
                        "Image pull failure, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }

                tracing::error!(
                    attempts = attempt,
                    image = image_ref,
                    %err,
                    "Failed to pre-pull image after all retries — \
                     container start will likely fail."
                );
                return;
            }
        }
    }
}

async fn is_image_cached_locally(image_ref: &str) -> bool {
    let Ok(docker) = docker_client_instance().await else {
        return false;
    };
    docker.inspect_image(image_ref).await.is_ok()
}

fn is_registry_auth_error(lowered_message: &str) -> bool {
    lowered_message.contains("status code 401")
        || lowered_message.contains("status code 403")
        || lowered_message.contains("access denied")
}

fn is_retryable_pull_error_message(lowered_message: &str) -> bool {
    let is_pull_error = lowered_message.contains("pullimage")
        || lowered_message.contains("pull image")
        || lowered_message.contains("pull the image");
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
    .any(|pattern| lowered_message.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower(s: &str) -> String {
        s.to_ascii_lowercase()
    }

    #[test]
    fn pull_timeout_errors_are_retryable() {
        let err = "Client(PullImage { descriptor: \
                   \"ghcr.io/foundry-rs/foundry:v1.3.6\", err: RequestTimeoutError })";
        assert!(is_retryable_pull_error_message(&lower(err)));
    }

    #[test]
    fn pull_auth_errors_are_not_retryable() {
        let err = "Client(PullImage { descriptor: \"ghcr.io/foundry-rs/foundry:v1.3.6\", \
                   err: DockerResponseServerError { status_code: 401, message: \
                   \"unauthorized\" } })";
        assert!(!is_retryable_pull_error_message(&lower(err)));
    }

    #[test]
    fn registry_auth_errors_detected() {
        // bollard's Display format: "Docker responded with status code N: ..."
        assert!(is_registry_auth_error(&lower(
            "failed to pull the image 'ghcr.io/foo/bar:tag', \
             error: Docker responded with status code 401: unauthorized"
        )));
        assert!(is_registry_auth_error(&lower(
            "Docker responded with status code 403: forbidden"
        )));
        assert!(is_registry_auth_error(&lower(
            "pull access denied for ghcr.io/foo/bar, \
             repository does not exist or may require 'docker login'"
        )));
        // Unrelated "denied" text must not match.
        assert!(!is_registry_auth_error(&lower(
            "permission denied (os error 13)"
        )));
        assert!(!is_registry_auth_error(&lower(
            "Docker responded with status code 500: internal error"
        )));
    }

    #[test]
    fn non_pull_timeouts_are_not_retryable() {
        let err = "ContainerWait(WaitError { source: RequestTimeoutError })";
        assert!(!is_retryable_pull_error_message(&lower(err)));
    }

    #[test]
    fn pull_the_image_timeout_errors_are_retryable() {
        let err = "failed to pull the image \
                   'ghcr.io/ross-weir/hyperlane-agent:integration-lander-1', \
                   error: Timeout error";
        assert!(is_retryable_pull_error_message(&lower(err)));
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
