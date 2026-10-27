//! Untrusted storage for downloaded Fibre blobs.
//!
//! Validators drop shards after `shard_retention`, so a node that must sync older blocks needs
//! its own copy. Implementations return whatever bytes they hold: callers MUST check the
//! commitment of every blob returned by [`FibreArchive::get`] before using it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use celestia_client::api::fibre::BlobID;

use crate::fibre::{FIBRE_HEADER_LEN, FIBRE_MAX_BLOB_SIZE};

const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

/// A store of Fibre blob data keyed by [`BlobID`].
#[async_trait]
pub trait FibreArchive: Send + Sync {
    /// Returns the stored bytes, or `None` if this archive does not have the blob.
    async fn get(&self, id: &BlobID) -> anyhow::Result<Option<Bytes>>;

    /// Stores the bytes of a blob.
    async fn put(&self, id: &BlobID, data: &[u8]) -> anyhow::Result<()>;
}

fn id_hex(id: &BlobID) -> String {
    hex::encode(id.as_bytes())
}

/// Archive backed by a directory, one file per blob named by the hex [`BlobID`].
pub struct FsArchive {
    dir: PathBuf,
}

impl FsArchive {
    /// Creates an archive rooted at `dir`; the directory is created on first write.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

#[async_trait]
impl FibreArchive for FsArchive {
    async fn get(&self, id: &BlobID) -> anyhow::Result<Option<Bytes>> {
        let path = self.dir.join(id_hex(id));
        match tokio::fs::read(&path).await {
            Ok(data) => Ok(Some(data.into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::Error::new(e).context(format!("reading {}", path.display()))),
        }
    }

    async fn put(&self, id: &BlobID, data: &[u8]) -> anyhow::Result<()> {
        static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

        tokio::fs::create_dir_all(&self.dir).await?;
        let name = id_hex(id);
        // Unique per call so concurrent writers of the same blob never share a temp file.
        let tmp = self.dir.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let dest = self.dir.join(&name);
        tokio::fs::write(&tmp, data).await?;
        if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
            // Best effort: the rename error is the one worth reporting.
            if let Err(cleanup) = tokio::fs::remove_file(&tmp).await {
                tracing::warn!(
                    "Failed to remove temp archive file {}: {cleanup}",
                    tmp.display()
                );
            }
            return Err(anyhow::Error::new(e).context(format!("renaming into {}", dest.display())));
        }
        Ok(())
    }
}

/// Read-only archive that fetches `GET {url}/{blob_id_hex}` from each configured mirror in order.
pub struct HttpFallbackArchive {
    urls: Vec<String>,
    client: reqwest::Client,
}

impl HttpFallbackArchive {
    /// Creates an archive over the given base URLs.
    pub fn new(urls: Vec<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?;
        let urls = urls
            .into_iter()
            .map(|u| u.trim_end_matches('/').to_string())
            .collect();
        Ok(Self { urls, client })
    }

    async fn get_from(&self, base: &str, name: &str) -> anyhow::Result<Option<Bytes>> {
        let max_len = (FIBRE_MAX_BLOB_SIZE - FIBRE_HEADER_LEN) as u64;
        let response = self.client.get(format!("{base}/{name}")).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status()?;
        anyhow::ensure!(
            response.content_length().is_none_or(|len| len <= max_len),
            "archive response from {base} exceeds the maximum Fibre blob size"
        );
        let data = response.bytes().await?;
        anyhow::ensure!(
            data.len() as u64 <= max_len,
            "archive response from {base} exceeds the maximum Fibre blob size"
        );
        Ok(Some(data))
    }
}

#[async_trait]
impl FibreArchive for HttpFallbackArchive {
    async fn get(&self, id: &BlobID) -> anyhow::Result<Option<Bytes>> {
        let name = id_hex(id);
        for base in &self.urls {
            match self.get_from(base, &name).await {
                Ok(Some(data)) => return Ok(Some(data)),
                Ok(None) => {}
                // A broken mirror must not hide a healthy one behind it.
                Err(e) => tracing::warn!("Fibre archive mirror {base} failed for {name}: {e:#}"),
            }
        }
        Ok(None)
    }

    async fn put(&self, _id: &BlobID, _data: &[u8]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Tries each archive in order on `get`; writes to every archive on `put`.
pub struct ChainedArchive(Vec<Arc<dyn FibreArchive>>);

impl ChainedArchive {
    /// Creates a chain; earlier archives take priority on reads.
    pub fn new(archives: Vec<Arc<dyn FibreArchive>>) -> Self {
        Self(archives)
    }
}

#[async_trait]
impl FibreArchive for ChainedArchive {
    async fn get(&self, id: &BlobID) -> anyhow::Result<Option<Bytes>> {
        for archive in &self.0 {
            match archive.get(id).await {
                Ok(Some(data)) => return Ok(Some(data)),
                Ok(None) => {}
                Err(e) => tracing::warn!("Fibre archive lookup of {} failed: {e:#}", id_hex(id)),
            }
        }
        Ok(None)
    }

    async fn put(&self, id: &BlobID, data: &[u8]) -> anyhow::Result<()> {
        let mut first_error = None;
        for archive in &self.0 {
            if let Err(e) = archive.put(id, data).await {
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fibre::encode_for_proofs;

    fn blob_with_id(seed: u8) -> (BlobID, Vec<u8>) {
        let data: Vec<u8> = (0..1000u32)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect();
        let encoded = encode_for_proofs(&data).unwrap();
        (BlobID::new(0, encoded.commitment), data)
    }

    struct FailingArchive;

    #[async_trait]
    impl FibreArchive for FailingArchive {
        async fn get(&self, _id: &BlobID) -> anyhow::Result<Option<Bytes>> {
            anyhow::bail!("get failed")
        }
        async fn put(&self, _id: &BlobID, _data: &[u8]) -> anyhow::Result<()> {
            anyhow::bail!("put failed")
        }
    }

    #[tokio::test]
    async fn fs_archive_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let archive = FsArchive::new(dir.path().join("nested"));
        let (id, data) = blob_with_id(1);

        archive.put(&id, &data).await.unwrap();

        assert_eq!(archive.get(&id).await.unwrap(), Some(Bytes::from(data)));
    }

    #[tokio::test]
    async fn fs_archive_misses_unknown_blob() {
        let dir = tempfile::tempdir().unwrap();
        let archive = FsArchive::new(dir.path().to_path_buf());
        let (id, _) = blob_with_id(1);

        assert_eq!(archive.get(&id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn fs_archive_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let archive = FsArchive::new(dir.path().to_path_buf());
        let (id, data) = blob_with_id(1);

        archive.put(&id, &data).await.unwrap();

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![id_hex(&id)]);
    }

    #[tokio::test]
    async fn corrupted_archive_file_fails_commitment_check() {
        let dir = tempfile::tempdir().unwrap();
        let archive = FsArchive::new(dir.path().to_path_buf());
        let (id, mut data) = blob_with_id(1);
        archive.put(&id, &data).await.unwrap();

        data[500] ^= 0xff;
        std::fs::write(dir.path().join(id_hex(&id)), &data).unwrap();

        let returned = archive.get(&id).await.unwrap().unwrap();
        assert_ne!(
            encode_for_proofs(&returned).unwrap().commitment,
            id.commitment(),
            "corrupted archive bytes must not match the blob's commitment"
        );
    }

    #[tokio::test]
    async fn chained_archive_falls_through_misses_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let empty = Arc::new(FsArchive::new(dir.path().join("empty")));
        let full = Arc::new(FsArchive::new(dir.path().join("full")));
        let (id, data) = blob_with_id(1);
        full.put(&id, &data).await.unwrap();
        let chain = ChainedArchive::new(vec![Arc::new(FailingArchive), empty, full]);

        assert_eq!(chain.get(&id).await.unwrap(), Some(Bytes::from(data)));
    }

    #[tokio::test]
    async fn chained_archive_puts_to_all_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let fs = Arc::new(FsArchive::new(dir.path().to_path_buf()));
        let chain = ChainedArchive::new(vec![Arc::new(FailingArchive), fs.clone()]);
        let (id, data) = blob_with_id(1);

        assert!(chain.put(&id, &data).await.is_err());

        assert_eq!(fs.get(&id).await.unwrap(), Some(Bytes::from(data)));
    }
}
