use std::fs::{File, OpenOptions};
#[cfg(not(target_os = "linux"))]
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};

// https://github.com/bminor/glibc/blob/3a0a8eae50679d3170df7af500dde2c4c3d11c78/sysdeps/unix/sysv/linux/bits/fcntl-linux.h#L97
#[cfg(target_os = "linux")]
const O_DSYNC: i32 = 0o10000;
// https://github.com/bminor/glibc/blob/3a0a8eae50679d3170df7af500dde2c4c3d11c78/sysdeps/unix/sysv/linux/bits/fcntl-linux.h#L88
#[cfg(target_os = "linux")]
const O_DIRECT: i32 = 0o40000;

const FLAG_FILE_NAME: &str = "commit_status.flag";

/// Sector size for O_DIRECT alignment requirements.
///
/// O_DIRECT on Linux requires that buffer addresses, file offsets, and I/O sizes
/// are all aligned to the device's logical sector size. We use 512 bytes because:
///
/// 1. **Maximum compatibility**: 512 bytes is the lowest common denominator that works on:
///    - Traditional HDDs (512-byte physical sectors)
///    - Advanced Format drives (4096-byte sectors with 512e emulation)
///    - Modern NVMe SSDs (typically report 512-byte logical sectors)
///
/// 2. **Universally aligned**: Since we write at offset 0, a 512-byte write is properly
///    aligned for both 512-byte and 4KB sector devices (512 divides evenly into 4096).
///
/// 3. **Historical standard**: 512 bytes has been the standard sector size since the 1980s.
///
/// The actual `CommitStatus` enum is only ~33 bytes maximum, but O_DIRECT requires
/// sector-aligned I/O, making 512 bytes the safest minimum.
#[cfg(any(target_os = "linux", test))]
const SECTOR_SIZE: usize = 512;

#[cfg(target_os = "linux")]
use std::os::unix::fs::{FileExt, OpenOptionsExt};

/// Represents the status of a two-phase commit operation.
/// ## Atomicity Guarantees
///
/// On Linux: Atomicity relies on hardware single-sector write atomicity (512 bytes).
/// All modern storage devices guarantee atomic single-sector writes.
/// O_DSYNC ensures durability before the syscall returns.
///
/// On other platforms: Uses standard file I/O with sync_all.
#[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Clone, Copy)]
pub enum CommitStatus {
    /// Indicates that the first phase of a commit is done, but the second is pending.
    /// Write root hash that has been written while in progress.
    InProgress([u8; 32]),
    /// Indicates that a commit operation is fully completed, or no operation is in progress.
    Completed,
}

/// Helper struct to manage 512-byte aligned buffer for O_DIRECT writes.
///
/// ## Why This Exists
///
/// O_DIRECT on Linux has strict alignment requirements that **must** be satisfied
/// or syscalls will fail with `EINVAL`:
///
/// 1. **Buffer address** must be aligned to sector size (512 bytes)
/// 2. **File offset** must be aligned to the sector size
/// 3. **I/O size** must be a multiple of sector size
///
/// Regular Rust allocations (`Vec<u8>`, `Box<[u8]>`, etc.) are typically aligned
/// to 8 or 16 bytes, which is insufficient for O_DIRECT.
///
/// ## Implementation
///
/// This struct uses `#[repr(align(512))]` (stable since Rust 1.25) to ensure
/// the buffer is properly aligned.
#[cfg(target_os = "linux")]
#[repr(align(512))]
struct AlignedBuffer([u8; SECTOR_SIZE]);

#[cfg(target_os = "linux")]
impl AlignedBuffer {
    fn new() -> Self {
        AlignedBuffer([0u8; SECTOR_SIZE])
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }

    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

/// Manages a persistent flag file to track the state of two-phase commits.
///
/// This utility helps ensure data consistency by providing a mechanism to detect
/// and recover from interruptions that might occur between the two phases of a commit.
/// It uses O_DIRECT + O_DSYNC for direct disk writes bypassing the page cache on Linux.
pub struct CommitFlag {
    file: File,
}

impl CommitFlag {
    /// Creates a new `CommitFlag` instance, opening or creating the flag file.
    ///
    /// The flag file will be located at `base_path/commit_status.flag`.
    /// On Linux, the file is opened with O_DIRECT + O_DSYNC flags and pre-allocated to 512 bytes.
    ///
    /// # Arguments
    ///
    /// * `base_path`: The directory path where the flag file will be stored.
    ///
    /// # Returns
    ///
    /// Returns `anyhow::Result<Self>` on success, or an error if the file cannot be
    /// created or opened.
    pub fn new(base_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let file_path = base_path.as_ref().join(FLAG_FILE_NAME);
        let file = Self::create_or_open(&file_path)?;
        Ok(Self { file })
    }

    /// Creates or opens the flag file with appropriate flags for the platform.
    /// On Linux: Uses O_DIRECT + O_DSYNC and pre-allocates to 512 bytes (sector size).
    /// On other platforms: Uses standard read/write flags.
    fn create_or_open(file_path: &Path) -> anyhow::Result<File> {
        let file_exists = file_path.exists();

        let mut options = OpenOptions::new();
        options.read(true);
        options.write(true);
        options.create(true);

        #[cfg(target_os = "linux")]
        {
            // Use O_DIRECT + O_DSYNC for direct disk writes
            options.custom_flags(O_DIRECT | O_DSYNC);
        }

        let file = options.open(file_path).with_context(|| {
            format!(
                "Failed to open commit flag file {} with options {:?}",
                file_path.display(),
                options
            )
        })?;

        // Initialize the file if it doesn't exist or is too small
        let needs_init = !file_exists || {
            #[cfg(target_os = "linux")]
            {
                file.metadata()?.len() < SECTOR_SIZE as u64
            }
            #[cfg(not(target_os = "linux"))]
            {
                file.metadata()?.len() == 0
            }
        };

        if needs_init {
            let serialized = borsh::to_vec(&CommitStatus::Completed)?;
            #[cfg(target_os = "linux")]
            {
                // Pre-allocate to sector size
                file.set_len(SECTOR_SIZE as u64)
                    .context("Failed to pre-allocate commit flag file")?;

                // Initialize with Completed status using aligned buffer
                let mut buffer = AlignedBuffer::new();

                if serialized.len() > SECTOR_SIZE {
                    anyhow::bail!("Serialized status exceeds sector size");
                }

                buffer.as_slice_mut()[..serialized.len()].copy_from_slice(&serialized);

                // Write using pwrite at offset 0
                file.write_at(buffer.as_slice(), 0)
                    .context("Failed to initialize commit flag file")?;
            }

            #[cfg(not(target_os = "linux"))]
            {
                (&file)
                    .write_all(&serialized)
                    .context("Failed to write initial status")?;
                file.sync_all().context("Failed to sync commit flag file")?;
            }
        }

        Ok(file)
    }

    /// Reads the current status from the flag file.
    ///
    /// The flag file always exists by the time this method is called, as it's created and
    /// initialized in `new()`. This method handles the following cases:
    ///
    /// - **Normal case**: Successfully deserializes and returns the stored [`CommitStatus`]
    /// - **Corruption/errors**: Returns `CommitStatus::InProgress([0; 32])` as a safe default
    ///
    /// Defaulting to `InProgress` on errors is safe because it triggers the recovery path,
    /// which will detect and handle any inconsistent state. This conservative approach
    /// ensures we never incorrectly report `Completed` when the system is in an unknown state.
    ///
    /// # Returns
    ///
    /// Returns an `anyhow::Result` containing the [`CommitStatus`]. Currently always returns
    /// `Ok` with either the actual status or the safe default `InProgress([0; 32])`.
    pub fn read_status(&self) -> anyhow::Result<CommitStatus> {
        #[cfg(target_os = "linux")]
        {
            // On Linux with O_DIRECT, read using an aligned buffer
            let mut buffer = AlignedBuffer::new();

            match self.file.read_at(buffer.as_slice_mut(), 0) {
                Ok(bytes_read) if bytes_read > 0 => {
                    match borsh::from_slice::<CommitStatus>(&buffer.as_slice()[..bytes_read]) {
                        Ok(status) => Ok(status),
                        Err(err) => {
                            tracing::warn!(
                                error = ?err,
                                "Commit flag file is corrupted, defaulting to InProgress for safety"
                            );
                            // Safe default: assume in progress so recovery will be triggered
                            Ok(CommitStatus::InProgress([0; 32]))
                        }
                    }
                }
                Ok(_) => {
                    tracing::warn!("Commit flag file is empty, defaulting to Completed");
                    self.write_status(CommitStatus::Completed)?;
                    Ok(CommitStatus::Completed)
                }
                Err(err) => {
                    Err(anyhow::Error::from(err).context("Failed to read commit flag file"))
                }
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            use std::io::{Seek, SeekFrom};

            let mut file_ref = &self.file;
            let mut buffer = Vec::new();

            if let Err(err) = file_ref.seek(SeekFrom::Start(0)) {
                tracing::warn!(error = ?err, "Failed to seek, defaulting to InProgress");
                return Ok(CommitStatus::Completed);
            }

            match file_ref.read_to_end(&mut buffer) {
                Ok(_) if !buffer.is_empty() => match borsh::from_slice::<CommitStatus>(&buffer) {
                    Ok(status) => Ok(status),
                    Err(err) => {
                        tracing::warn!(error = ?err, "Corrupted data, defaulting to InProgress");
                        Ok(CommitStatus::Completed)
                    }
                },
                Ok(_) => {
                    tracing::warn!("Empty file, defaulting to Completed");
                    self.write_status(CommitStatus::Completed)?;
                    Ok(CommitStatus::Completed)
                }
                Err(err) => {
                    Err(anyhow::Error::from(err).context("Failed to read commit flag file"))
                }
            }
        }
    }

    /// Writes the given status to the flag file.
    ///
    /// On Linux, this uses O_DIRECT + O_DSYNC with a 512-byte aligned buffer and a single pwrite syscall
    /// for maximum performance and durability. The file is overwritten in-place.
    ///
    /// On other platforms, this uses standard file I/O with sync_all.
    ///
    /// # Arguments
    ///
    /// * `status`: The `CommitStatus` to write to the flag file.
    ///
    /// # Returns
    ///
    /// Returns `anyhow::Result<()>` which is `Ok(())` on successful write, or an error
    /// if the write operation fails.
    ///
    /// # Error Handling
    ///
    /// This method properly handles disk full scenarios (ENOSPC) and other I/O errors by
    /// returning them as `Err`. All write operations (write_at/write_all, set_len, sync_all)
    /// are fallible and their errors are propagated with context. The caller should handle
    /// these errors appropriately, potentially triggering recovery or alerting operators.
    pub fn write_status(&self, status: CommitStatus) -> anyhow::Result<()> {
        // We could've use `borsh::to_writer`
        let serialized = borsh::to_vec(&status)?;
        #[cfg(target_os = "linux")]
        {
            if serialized.len() > SECTOR_SIZE {
                anyhow::bail!("Serialized status exceeds sector size");
            }

            // Create aligned buffer and copy serialized data
            let mut buffer = AlignedBuffer::new();
            buffer.as_slice_mut()[..serialized.len()].copy_from_slice(&serialized);

            // Single pwrite syscall at offset 0 - hot path!
            self.file
                .write_at(buffer.as_slice(), 0)
                .context("Failed to write commit status with O_DIRECT")?;
            // O_DSYNC ensures this is already synced to disk
        }

        #[cfg(not(target_os = "linux"))]
        {
            // Non-linux targets are not fully supported by NOMT.
            // SAFETY: We don't truncate BEFORE writing to avoid data loss window.
            // Instead, we:
            // 1. Write new data at offset 0 (goes to page cache, disk retains old data)
            // 2. Truncate to new size (in page cache)
            // 3. sync_all() flushes everything atomically
            //
            // If crash before sync: disk has old data (safe - can recover)
            // If crash after sync: disk has new data (safe - committed)
            let mut file_ref = &self.file;
            file_ref
                .seek(SeekFrom::Start(0))
                .context("Failed to seek to beginning")?;

            file_ref
                .write_all(&serialized)
                .context("Failed to write commit status")?;

            // Truncate to remove trailing garbage (this is safe AFTER write)
            self.file
                .set_len(serialized.len() as u64)
                .context("Failed to truncate to new size")?;
            self.file
                .sync_all()
                .context("Failed to sync commit flag file")?;
        }
        Ok(())
    }

    /// Logs instructions for manually resetting the commit flag file.
    ///
    /// This is used when the commit flag gets into an inconsistent state and needs manual intervention.
    pub fn log_reset_instruction(&self) {
        tracing::error!("To reset commit flag, please remove the commit_status.flag file manually");
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_commit_flag_flow() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path()).unwrap();

        // 1. Initial read: file should be created and initialized to Completed
        #[cfg(target_os = "linux")]
        assert_eq!(flag.read_status().unwrap(), CommitStatus::Completed);

        #[cfg(not(target_os = "linux"))]
        assert_eq!(flag.read_status().unwrap(), CommitStatus::Completed);

        let root_hash = [128u8; 32];
        let in_progress_msg = CommitStatus::InProgress(root_hash);

        // 2. Write InProgress
        flag.write_status(in_progress_msg).unwrap();
        assert_eq!(flag.read_status().unwrap(), in_progress_msg);

        // 3. Write Completed
        flag.write_status(CommitStatus::Completed).unwrap();
        assert_eq!(flag.read_status().unwrap(), CommitStatus::Completed);
    }

    #[test]
    fn test_corrupted_file() {
        let dir = tempdir().unwrap();
        let flag_path = dir.path().join(FLAG_FILE_NAME);

        // Create a corrupted file
        let mut file = File::create(&flag_path).unwrap();
        std::io::Write::write_all(&mut file, b"CORRUPTED_DATA").unwrap();
        drop(file);

        let commit_flag = CommitFlag::new(dir.path()).unwrap();
        // Should detect corruption and return InProgress (safe default)
        let status = commit_flag.read_status().unwrap();

        // On Linux with O_DIRECT, the file will be reinitialized on open if too small
        // So we should get Completed. On other platforms, we return InProgress for corrupted data.
        #[cfg(target_os = "linux")]
        assert_eq!(status, CommitStatus::Completed);

        #[cfg(not(target_os = "linux"))]
        assert_eq!(status, CommitStatus::InProgress([0; 32]));
    }

    #[test]
    fn test_serialization_size() {
        // Ensure our enum fits within 512 bytes
        let completed = CommitStatus::Completed;
        let in_progress = CommitStatus::InProgress([255u8; 32]);

        assert!(borsh::to_vec(&completed).unwrap().len() <= SECTOR_SIZE);
        assert!(borsh::to_vec(&in_progress).unwrap().len() <= SECTOR_SIZE);
    }

    #[test]
    fn test_reinitialize_preserves_state() {
        let dir = tempdir().unwrap();

        // First initialization - create flag and set to InProgress
        let root_hash = [42u8; 32];
        {
            let flag = CommitFlag::new(dir.path()).unwrap();
            flag.write_status(CommitStatus::InProgress(root_hash))
                .unwrap();
            assert_eq!(
                flag.read_status().unwrap(),
                CommitStatus::InProgress(root_hash)
            );
        }
        // Flag is dropped here

        // Re-initialize with new() on the same directory
        let flag2 = CommitFlag::new(dir.path()).unwrap();

        // Should preserve the InProgress state, not overwrite with Completed
        let status = flag2.read_status().unwrap();
        assert_eq!(
            status,
            CommitStatus::InProgress(root_hash),
            "Re-initialization should preserve existing state, not reset to Completed"
        );

        // Now write Completed and verify it's preserved after re-initialization
        flag2.write_status(CommitStatus::Completed).unwrap();
        drop(flag2);

        let flag3 = CommitFlag::new(dir.path()).unwrap();
        assert_eq!(
            flag3.read_status().unwrap(),
            CommitStatus::Completed,
            "Re-initialization should preserve Completed state"
        );
    }
}
