//! Crash-safe commit flag using ping-pong slots.
//!
//! This module provides a durable flag file to track two-phase commit status
//! without relying on temp-file+rename. Instead, it uses a single file with:
//! - 4 KiB header (magic, version, active_slot, CRC)
//! - Two 4 KiB slots (sequence number, state, root hash, CRC)
//!
//! Writes alternate between slots (ping-pong) and are made durable via:
//! - O_DSYNC on Linux (eliminating extra fsync calls)
//! - sync_data() on other platforms
//!
//! The ping-pong technique is a form of double buffering where writes alternate
//! between two slots. See: <https://en.wikipedia.org/wiki/Multiple_buffering>
//!
//! Recovery handles corruption by checking CRCs and falling back to the
//! slot with the highest valid sequence number.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};


// https://github.com/bminor/glibc/blob/3a0a8eae50679d3170df7af500dde2c4c3d11c78/sysdeps/unix/sysv/linux/bits/fcntl-linux.h#L97
#[cfg(not(target_os = "linux"))]
const O_DSYNC: i32 = 0o10000;
const FLAG_FILE_NAME: &str = "commit_status.flag";

// File layout constants
const HEADER_SIZE: usize = 4096;
const SLOT_SIZE: usize = 4096;
const FILE_SIZE: usize = HEADER_SIZE + 2 * SLOT_SIZE; // 12 KiB total

// Header offsets
const MAGIC: &[u8; 8] = b"CF01\0\0\0\0";
const HEADER_VERSION_OFFSET: usize = 8;
const HEADER_ACTIVE_SLOT_OFFSET: usize = 24;
const HEADER_CRC_OFFSET: usize = 28;
// CRC covers first 28 bytes
const HEADER_CRC_INPUT_LEN: usize = 28;

// Slot offsets
const SLOT_SEQ_OFFSET: usize = 0;
const SLOT_STATE_TAG_OFFSET: usize = 8;
const SLOT_ROOT_OFFSET: usize = 16;
const SLOT_CRC_OFFSET: usize = SLOT_SIZE - 4;
// CRC covers all but last 4 bytes
const SLOT_CRC_INPUT_LEN: usize = SLOT_SIZE - 4;

// State tags
const STATE_TAG_COMPLETED: u8 = 0;
const STATE_TAG_IN_PROGRESS: u8 = 1;

/// Represents the status of a two-phase commit operation.
#[derive(Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Clone, Copy)]
pub enum CommitStatus {
    /// Indicates that the first phase of a commit is done, but the second is pending.
    /// Write root hash that has been written while in progress.
    InProgress([u8; 32]),
    /// Indicates that a commit operation is fully completed, or no operation is in progress.
    Completed,
}

/// Manages a persistent flag file to track the state of two-phase commits.
///
/// This utility helps ensure data consistency by providing a mechanism to detect
/// and recover from interruptions that might occur between the two phases of a commit.
/// It uses a ping-pong slot strategy with CRC validation for crash safety.
pub struct CommitFlag {
    file_path: PathBuf,
    /// Current sequence number (max of both slots)
    current_seq: AtomicU64,
    /// Currently active slot (0 or 1)
    active_slot: AtomicU8,
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
        let file_path = base_path.as_ref().join(FLAG_FILE_NAME);

        Self {
            file_path,
            current_seq: AtomicU64::new(0),
            active_slot: AtomicU8::new(0),
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
        match self.try_read_status() {
            Ok(status) => Ok(status),
            Err(ReadError::NotFound) => {
                // File not found, create it with Completed status
                self.initialize_file()?;
                Ok(CommitStatus::Completed)
            }
            Err(ReadError::Corrupted) => {
                tracing::warn!("Commit flag file is corrupted, defaulting to completed");
                self.initialize_file()?;
                Ok(CommitStatus::Completed)
            }
            Err(ReadError::Legacy(status)) => {
                // Legacy format detected, migrate to new format
                tracing::info!("Migrating commit flag from legacy format");
                self.initialize_file()?;
                self.write_status(status)?;
                Ok(status)
            }
            Err(ReadError::Io(e)) => {
                Err(anyhow::Error::from(e).context("Failed to read commit flag file"))
            }
        }
    }

    /// Writes the given status to the flag file atomically.
    ///
    /// This operation writes to the inactive slot with a new sequence number,
    /// syncs the slot, updates the header to point to the new slot,
    /// and syncs the header.
    ///
    /// # Arguments
    ///
    /// * `status`: The `CommitStatus` to write to the flag file.
    ///
    /// # Returns
    ///
    /// Returns `anyhow::Result<()>` which is `Ok(())` on successful write, or an error
    /// if any step of the write process fails.
    pub fn write_status(&self, status: CommitStatus) -> anyhow::Result<()> {
        let mut file = self.open_for_write()?;

        let current_seq = self.current_seq.load(Ordering::Relaxed);
        let current_active_slot = self.active_slot.load(Ordering::Relaxed);
        let next_seq = current_seq + 1;
        let next_slot = 1 - current_active_slot;

        let slot_data = self.serialize_slot(next_seq, status);
        let slot_offset = HEADER_SIZE + (next_slot as usize * SLOT_SIZE);
        write_at(&mut file, &slot_data, slot_offset)?;

        #[cfg(not(target_os = "linux"))]
        file.sync_data()
            .context("Failed to sync commit flag slot data")?;

        let header_data = self.serialize_header(next_slot);
        write_at(&mut file, &header_data, 0)?;

        #[cfg(not(target_os = "linux"))]
        file.sync_data()
            .context("Failed to sync commit flag header")?;

        self.current_seq.store(next_seq, Ordering::Relaxed);
        self.active_slot.store(next_slot, Ordering::Relaxed);

        Ok(())
    }

    pub fn log_reset_instruction(&self) {
        tracing::error!(
            "To reset commit flag, please remove commit flag file: `rm {}`",
            self.file_path.display()
        );
    }

    // Internal methods

    fn try_read_status(&self) -> Result<CommitStatus, ReadError> {
        let mut file = File::open(&self.file_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ReadError::NotFound
            } else {
                ReadError::Io(e)
            }
        })?;

        // Check file size to detect legacy format
        let metadata = file.metadata().map_err(ReadError::Io)?;
        if metadata.len() < FILE_SIZE as u64 {
            // Might be legacy format, try parsing
            return self.try_read_legacy(&mut file);
        }

        // Read header
        let mut header = vec![0u8; HEADER_SIZE];
        file.read_exact(&mut header).map_err(ReadError::Io)?;

        // Try to parse header
        if let Some(active_slot) = self.parse_header(&header) {
            // Header is valid, read the active slot
            if let Some((seq, status)) = self.read_and_parse_slot(&mut file, active_slot)? {
                self.current_seq.store(seq, Ordering::Relaxed);
                self.active_slot.store(active_slot, Ordering::Relaxed);
                return Ok(status);
            }
        }

        // Header invalid or active slot invalid, try both slots
        let slot0 = self.read_and_parse_slot(&mut file, 0)?;
        let slot1 = self.read_and_parse_slot(&mut file, 1)?;

        match (slot0, slot1) {
            (Some((seq0, status0)), Some((seq1, status1))) => {
                // Both valid, pick higher sequence
                if seq1 > seq0 {
                    self.current_seq.store(seq1, Ordering::Relaxed);
                    self.active_slot.store(1, Ordering::Relaxed);
                    Ok(status1)
                } else {
                    self.current_seq.store(seq0, Ordering::Relaxed);
                    self.active_slot.store(0, Ordering::Relaxed);
                    Ok(status0)
                }
            }
            (Some((seq, status)), None) => {
                self.current_seq.store(seq, Ordering::Relaxed);
                self.active_slot.store(0, Ordering::Relaxed);
                Ok(status)
            }
            (None, Some((seq, status))) => {
                self.current_seq.store(seq, Ordering::Relaxed);
                self.active_slot.store(1, Ordering::Relaxed);
                Ok(status)
            }
            (None, None) => {
                // Both slots corrupted, try legacy format as last resort
                self.try_read_legacy(&mut file)
            }
        }
    }

    fn try_read_legacy(&self, file: &mut File) -> Result<CommitStatus, ReadError> {
        file.seek(SeekFrom::Start(0)).map_err(ReadError::Io)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data).map_err(ReadError::Io)?;

        if data.is_empty() {
            return Err(ReadError::Corrupted);
        }

        match borsh::from_slice::<CommitStatus>(&data) {
            Ok(status) => Err(ReadError::Legacy(status)),
            Err(_) => Err(ReadError::Corrupted),
        }
    }

    fn parse_header(&self, header: &[u8]) -> Option<u8> {
        // Check magic
        if &header[0..8] != MAGIC {
            return None;
        }

        // Verify header CRC
        let expected_crc = u32::from_le_bytes(
            header[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4]
                .try_into()
                .ok()?,
        );
        let actual_crc = crc32fast::hash(&header[0..HEADER_CRC_INPUT_LEN]);
        if actual_crc != expected_crc {
            return None;
        }

        // Extract active slot
        let active_slot = header[HEADER_ACTIVE_SLOT_OFFSET];
        if active_slot > 1 {
            return None;
        }

        Some(active_slot)
    }

    fn read_and_parse_slot(
        &self,
        file: &mut File,
        slot: u8,
    ) -> Result<Option<(u64, CommitStatus)>, ReadError> {
        let offset = HEADER_SIZE + (slot as usize * SLOT_SIZE);
        let mut slot_data = vec![0u8; SLOT_SIZE];

        file.seek(SeekFrom::Start(offset as u64))
            .map_err(ReadError::Io)?;
        file.read_exact(&mut slot_data).map_err(ReadError::Io)?;

        Ok(self.parse_slot(&slot_data))
    }

    fn parse_slot(&self, slot_data: &[u8]) -> Option<(u64, CommitStatus)> {
        // Verify slot CRC
        let expected_crc = u32::from_le_bytes(
            slot_data[SLOT_CRC_OFFSET..SLOT_CRC_OFFSET + 4]
                .try_into()
                .ok()?,
        );
        let actual_crc = crc32fast::hash(&slot_data[0..SLOT_CRC_INPUT_LEN]);
        if actual_crc != expected_crc {
            return None;
        }

        // Parse slot fields
        let seq = u64::from_le_bytes(
            slot_data[SLOT_SEQ_OFFSET..SLOT_SEQ_OFFSET + 8]
                .try_into()
                .ok()?,
        );
        let state_tag = slot_data[SLOT_STATE_TAG_OFFSET];

        let status = match state_tag {
            STATE_TAG_COMPLETED => CommitStatus::Completed,
            STATE_TAG_IN_PROGRESS => {
                let root: [u8; 32] = slot_data[SLOT_ROOT_OFFSET..SLOT_ROOT_OFFSET + 32]
                    .try_into()
                    .ok()?;
                CommitStatus::InProgress(root)
            }
            _ => return None,
        };

        Some((seq, status))
    }

    fn serialize_header(&self, active_slot: u8) -> Vec<u8> {
        let mut header = vec![0u8; HEADER_SIZE];

        // Magic
        header[0..8].copy_from_slice(MAGIC);

        // Version
        header[HEADER_VERSION_OFFSET..HEADER_VERSION_OFFSET + 4]
            .copy_from_slice(&1u32.to_le_bytes());

        // Reserved (already zeroed)

        // Active slot
        header[HEADER_ACTIVE_SLOT_OFFSET] = active_slot;

        // Compute and write CRC
        let crc = crc32fast::hash(&header[0..HEADER_CRC_INPUT_LEN]);
        header[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());

        header
    }

    fn serialize_slot(&self, seq: u64, status: CommitStatus) -> Vec<u8> {
        let mut slot = vec![0u8; SLOT_SIZE];

        // Sequence number
        slot[SLOT_SEQ_OFFSET..SLOT_SEQ_OFFSET + 8].copy_from_slice(&seq.to_le_bytes());

        // State tag and root
        match status {
            CommitStatus::Completed => {
                slot[SLOT_STATE_TAG_OFFSET] = STATE_TAG_COMPLETED;
                // Root remains zeroed
            }
            CommitStatus::InProgress(root) => {
                slot[SLOT_STATE_TAG_OFFSET] = STATE_TAG_IN_PROGRESS;
                slot[SLOT_ROOT_OFFSET..SLOT_ROOT_OFFSET + 32].copy_from_slice(&root);
            }
        }

        // Compute and write CRC
        let crc = crc32fast::hash(&slot[0..SLOT_CRC_INPUT_LEN]);
        slot[SLOT_CRC_OFFSET..SLOT_CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());

        slot
    }

    fn initialize_file(&self) -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.file_path)
            .context("Failed to create commit flag file")?;

        // Write header with active_slot=0
        let header = self.serialize_header(0);
        file.write_all(&header)
            .context("Failed to write commit flag header")?;

        // Write two empty Completed slots
        let slot = self.serialize_slot(0, CommitStatus::Completed);
        file.write_all(&slot)
            .context("Failed to write commit flag slot 0")?;
        file.write_all(&slot)
            .context("Failed to write commit flag slot 1")?;

        file.sync_data()
            .context("Failed to sync commit flag file")?;

        self.current_seq.store(0, Ordering::Relaxed);
        self.active_slot.store(0, Ordering::Relaxed);

        Ok(())
    }

    fn open_for_write(&self) -> anyhow::Result<File> {
        let mut options = OpenOptions::new();
        options.write(true);

        #[cfg(target_os = "linux")]
        let options = {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(O_DSYNC)
        };

        options
            .open(&self.file_path)
            .context("Failed to open commit flag file for writing")
    }
}

enum ReadError {
    NotFound,
    Corrupted,
    Legacy(CommitStatus),
    Io(std::io::Error),
}

/// Write data at a specific offset in the file.
fn write_at(file: &mut File, data: &[u8], offset: usize) -> anyhow::Result<()> {
    file.seek(SeekFrom::Start(offset as u64))
        .context("Failed to seek in commit flag file")?;
    file.write_all(data)
        .context("Failed to write to commit flag file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_init_creates_new_layout() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initial read should create file and return Completed
        assert_eq!(flag.read_status().unwrap(), CommitStatus::Completed);

        // Verify file exists and has correct size
        let file_path = dir.path().join(FLAG_FILE_NAME);
        assert!(file_path.exists());
        let metadata = std::fs::metadata(&file_path).unwrap();
        assert_eq!(metadata.len(), FILE_SIZE as u64);
    }

    #[test]
    fn test_roundtrip_in_progress() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initialize
        assert_eq!(flag.read_status().unwrap(), CommitStatus::Completed);

        // Write InProgress
        let root_hash = [128u8; 32];
        let in_progress = CommitStatus::InProgress(root_hash);
        flag.write_status(in_progress).unwrap();

        // Read back
        let flag2 = CommitFlag::new(dir.path());
        assert_eq!(flag2.read_status().unwrap(), in_progress);
    }

    #[test]
    fn test_roundtrip_completed() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initialize
        assert_eq!(flag.read_status().unwrap(), CommitStatus::Completed);

        // Write InProgress
        let root_hash = [42u8; 32];
        flag.write_status(CommitStatus::InProgress(root_hash))
            .unwrap();

        // Write Completed
        flag.write_status(CommitStatus::Completed).unwrap();

        // Read back
        let flag2 = CommitFlag::new(dir.path());
        assert_eq!(flag2.read_status().unwrap(), CommitStatus::Completed);
    }

    #[test]
    fn test_crash_after_slot_write() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initialize
        flag.read_status().unwrap();

        // Simulate: write InProgress to slot 1, but don't update header
        let root_hash = [99u8; 32];
        let slot_data = flag.serialize_slot(1, CommitStatus::InProgress(root_hash));

        let file_path = dir.path().join(FLAG_FILE_NAME);
        let mut file = OpenOptions::new().write(true).open(&file_path).unwrap();
        write_at(&mut file, &slot_data, HEADER_SIZE + SLOT_SIZE).unwrap();
        file.sync_data().unwrap();
        drop(file);

        // Read should recover and return Completed (header still points to slot 0)
        let flag2 = CommitFlag::new(dir.path());
        assert_eq!(flag2.read_status().unwrap(), CommitStatus::Completed);
    }

    #[test]
    fn test_crash_after_header_update() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initialize and write InProgress
        flag.read_status().unwrap();
        let root_hash = [77u8; 32];
        flag.write_status(CommitStatus::InProgress(root_hash))
            .unwrap();

        // Now write Completed to slot 0, update header, but simulate crash before final sync
        let slot_data = flag.serialize_slot(2, CommitStatus::Completed);
        let header_data = flag.serialize_header(0);

        let file_path = dir.path().join(FLAG_FILE_NAME);
        let mut file = OpenOptions::new().write(true).open(&file_path).unwrap();
        write_at(&mut file, &slot_data, HEADER_SIZE).unwrap();
        write_at(&mut file, &header_data, 0).unwrap();
        // Don't sync, simulating crash
        drop(file);

        // Read should return the last durable state
        // On most systems this will be Completed if writes went through
        let flag2 = CommitFlag::new(dir.path());
        let status = flag2.read_status().unwrap();
        // Should be either Completed (if writes persisted) or InProgress (if they didn't)
        assert!(status == CommitStatus::Completed || status == CommitStatus::InProgress(root_hash));
    }

    #[test]
    fn test_legacy_migration_in_progress() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join(FLAG_FILE_NAME);

        // Write legacy Borsh format
        let root_hash = [55u8; 32];
        let legacy_status = CommitStatus::InProgress(root_hash);
        let legacy_data = borsh::to_vec(&legacy_status).unwrap();
        std::fs::write(&file_path, legacy_data).unwrap();

        // Open should migrate and preserve status
        let flag = CommitFlag::new(dir.path());
        assert_eq!(flag.read_status().unwrap(), legacy_status);

        // Verify new format is in place
        let metadata = std::fs::metadata(&file_path).unwrap();
        assert_eq!(metadata.len(), FILE_SIZE as u64);

        // Reading again should work
        let flag2 = CommitFlag::new(dir.path());
        assert_eq!(flag2.read_status().unwrap(), legacy_status);
    }

    #[test]
    fn test_legacy_migration_completed() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join(FLAG_FILE_NAME);

        // Write legacy Borsh format
        let legacy_status = CommitStatus::Completed;
        let legacy_data = borsh::to_vec(&legacy_status).unwrap();
        std::fs::write(&file_path, legacy_data).unwrap();

        // Open should migrate
        let flag = CommitFlag::new(dir.path());
        assert_eq!(flag.read_status().unwrap(), CommitStatus::Completed);

        // Verify new format
        let metadata = std::fs::metadata(&file_path).unwrap();
        assert_eq!(metadata.len(), FILE_SIZE as u64);
    }

    #[test]
    fn test_no_temp_files_on_hot_path() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initialize
        flag.read_status().unwrap();

        // Write InProgress
        let root_hash = [111u8; 32];
        flag.write_status(CommitStatus::InProgress(root_hash))
            .unwrap();

        // Write Completed
        flag.write_status(CommitStatus::Completed).unwrap();

        // Check that no .tmp files exist in directory
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], FLAG_FILE_NAME);
    }

    #[test]
    fn test_corrupted_header_recovers_from_slots() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initialize and write InProgress
        flag.read_status().unwrap();
        let root_hash = [200u8; 32];
        flag.write_status(CommitStatus::InProgress(root_hash))
            .unwrap();

        // Corrupt the header
        let file_path = dir.path().join(FLAG_FILE_NAME);
        let mut file = OpenOptions::new().write(true).open(&file_path).unwrap();
        file.write_all(b"CORRUPTED_HEADER").unwrap();
        drop(file);

        // Read should recover from slot with highest seq
        let flag2 = CommitFlag::new(dir.path());
        assert_eq!(
            flag2.read_status().unwrap(),
            CommitStatus::InProgress(root_hash)
        );
    }

    #[test]
    fn test_ping_pong_slots() {
        let dir = tempdir().unwrap();
        let flag = CommitFlag::new(dir.path());

        // Initialize
        flag.read_status().unwrap();
        assert_eq!(flag.active_slot.load(Ordering::Relaxed), 0);
        assert_eq!(flag.current_seq.load(Ordering::Relaxed), 0);

        // First write should go to slot 1
        flag.write_status(CommitStatus::InProgress([1u8; 32]))
            .unwrap();
        assert_eq!(flag.active_slot.load(Ordering::Relaxed), 1);
        assert_eq!(flag.current_seq.load(Ordering::Relaxed), 1);

        // Second write should go to slot 0
        flag.write_status(CommitStatus::Completed).unwrap();
        assert_eq!(flag.active_slot.load(Ordering::Relaxed), 0);
        assert_eq!(flag.current_seq.load(Ordering::Relaxed), 2);

        // Third write should go to slot 1
        flag.write_status(CommitStatus::InProgress([2u8; 32]))
            .unwrap();
        assert_eq!(flag.active_slot.load(Ordering::Relaxed), 1);
        assert_eq!(flag.current_seq.load(Ordering::Relaxed), 3);
    }
}
