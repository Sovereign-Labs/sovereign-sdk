use anyhow::Context;
use std::path::Path;
use tokio::io::AsyncWriteExt;

/// Atomically writes content to a file.
///
/// Uses write-to-temp-then-rename pattern to ensure the file is never
/// partially written. The data is synced to disk before renaming.
pub async fn write_to_file_atomically(path: &Path, content: &str) -> anyhow::Result<()> {
    let dir = path.parent().context("Path has no parent directory")?;

    // Create temp file in same directory to ensure same filesystem for atomic rename.
    let temp_path = dir.join(".tmp");

    // Write content to temp file.
    let mut file = tokio::fs::File::create(&temp_path)
        .await
        .with_context(|| format!("Failed to create temp file at {temp_path:?}"))?;

    file.write_all(content.as_bytes())
        .await
        .with_context(|| format!("Failed to write to temp file at {temp_path:?}"))?;

    // Sync to disk before renaming.
    file.sync_all()
        .await
        .with_context(|| format!("Failed to sync temp file at {temp_path:?}"))?;

    // Atomic rename.
    tokio::fs::rename(&temp_path, path)
        .await
        .with_context(|| format!("Failed to rename {temp_path:?} to {path:?}"))?;

    Ok(())
}
