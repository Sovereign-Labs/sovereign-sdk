use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};

const FLAG_FILE_NAME: &str = "commit_status.flag";

/// Represents the current state of a commit operation.
#[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Clone, Copy)]
pub enum CommitStatus {
    /// A commit of **kernel state** is in progress.
    CommittingKernelNomt,
    /// A commit of **user state** is in progress.
    CommittingUserNomt,
    /// A commit of **LedgerDB** is in progress.
    CommittingLedger,
    /// A commit of **AccessoryDB** is in progress.
    CommittingAccessory,
    /// A commit of **archival user and kernel state** in the flat-db is in progress.
    CommittingArchivalUserAndKernel,
    /// A commit of **live user and kernel state** in the flat-db is in progress.
    CommittingLiveUserAndKernel,
    /// The commit succeeded.
    Success,
}

/// Manages a persistent flag file to track the state of two-phase commits.
///
/// This utility helps ensure data consistency by providing a mechanism to detect
/// and recover from interruptions that might occur between the two phases of a commit.
/// It uses an atomic write strategy (write to temp file, sync, then rename) to update the flag.
pub struct CommitFlag {
    file_path: PathBuf,
    temp_file_path: PathBuf,
}

impl CommitFlag {
    /// Creates a new `CommitFlag` instance.
    ///
    /// The flag file will be located at `base_path/commit_status.flag`.
    ///
    /// # Arguments
    ///
    /// * `base_path`: The directory path where the flag file will be stored.
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        let base_path = base_path.as_ref();
        Self {
            file_path: base_path.join(FLAG_FILE_NAME),
            temp_file_path: base_path.join(format!("{FLAG_FILE_NAME}.tmp")),
        }
    }

    /// Reads the current status from the flag file.
    ///
    /// - If the flag file does not exist, it is created and initialized to `CommitStatus::Completed`.
    /// - If the flag file is found to be corrupted (i.e., contains unexpected data),
    ///   a warning is logged, the file is overwritten with `CommitStatus::Completed`, and `CommitStatus::Completed` is returned.
    ///
    /// # Returns
    ///
    /// Returns a `anyhow::Result` containing the `CommitStatus` on success, or an error
    /// if reading or initializing the file fails (e.g., due to I/O errors or permission issues).
    pub fn read_status(&self) -> anyhow::Result<CommitStatus> {
        match File::open(&self.file_path) {
            Ok(mut file) => match borsh::from_reader::<File, CommitStatus>(&mut file) {
                Ok(status) => Ok(status),
                Err(err) => {
                    tracing::warn!(error = ?err, "Commit flag file is corrupted, defaulting to completed");
                    self.write_status(&CommitStatus::Success)?;
                    Ok(CommitStatus::Success)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File not found, create it with COMPLETED status
                self.write_status(&CommitStatus::Success)?;
                Ok(CommitStatus::Success)
            }
            Err(e) => Err(anyhow::Error::from(e).context("Failed to read commit flag file")),
        }
    }

    /// Writes the given status to the flag file atomically.
    ///
    /// This operation is performed by first writing to a temporary file, syncing that file
    /// to disk, and then atomically renaming the temporary file to the actual flag file name.
    /// This minimizes the chance of the flag file becoming corrupted due to crashes.
    ///
    /// # Arguments
    ///
    /// * `status`: The `CommitStatus` to write to the flag file.
    ///
    /// # Returns
    ///
    /// Returns `anyhow::Result<()>` which is `Ok(())` on successful write, or an error
    /// if any step of the atomic write process fails (e.g., I/O errors, permission issues,
    /// disk full).
    pub fn write_status(&self, status: &CommitStatus) -> anyhow::Result<()> {
        let message = borsh::to_vec(&status)?;

        // Write to a temporary file first
        let mut temp_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.temp_file_path)
            .context("Failed to open temp commit flag file for writing")?;

        temp_file
            .write_all(&message)
            .context("Failed to write to temp commit flag file")?;

        temp_file
            .sync_all()
            .context("Failed to sync temp commit flag file")?;

        // Atomically rename the temporary file to the actual flag file
        std::fs::rename(&self.temp_file_path, &self.file_path)
            .context("Failed to rename temp commit flag file to actual flag file")?;

        // Attempt to sync the parent directory to ensure the rename operation is persisted on some filesystems
        if let Some(parent_dir) = self.file_path.parent() {
            if let Ok(dir_file) = File::open(parent_dir) {
                let _ = dir_file.sync_all(); // Best effort sync
            }
        }

        Ok(())
    }

    pub(crate) fn save_commit_status(&self, commit_status: &CommitStatus) -> anyhow::Result<()> {
        self.write_status(commit_status).with_context(|| {
            format!("Failed to write {commit_status:?} status after successful user commit")
        })?;

        debug_assert_eq!(&self.read_status()?, commit_status);
        Ok(())
    }
}
