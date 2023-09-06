// Adapted from Aptos-Core.
// Modified to remove serde dependency

use rocksdb::Options;

/// Port selected RocksDB options for tuning underlying rocksdb instance of our state db.
/// The current default values are taken from Aptos. TODO: tune rocksdb for our workload.
/// see <https://github.com/facebook/rocksdb/blob/master/include/rocksdb/options.h>
/// for detailed explanations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RocksdbConfig {
    /// The maximum number of files that can be open concurrently. Defaults to 5000
    pub max_open_files: i32,
    /// Once write-ahead logs exceed this size, RocksDB will start forcing the flush of column
    /// families whose memtables are backed by the oldest live WAL file. Defaults to 1GB
    pub max_total_wal_size: u64,
    /// The maximum number of background threads, including threads for flushing and compaction. Defaults to 16.
    pub max_background_jobs: i32,
}

impl Default for RocksdbConfig {
    fn default() -> Self {
        Self {
            // Allow db to close old sst files, saving memory.
            max_open_files: 5000,
            // For now we set the max total WAL size to be 1G. This config can be useful when column
            // families are updated at non-uniform frequencies.
            max_total_wal_size: 1u64 << 30,
            // This includes threads for flushing and compaction. Rocksdb will decide the # of
            // threads to use internally.
            max_background_jobs: 16,
        }
    }
}

/// Generate [`rocksdb::Options`] corresponding to the given [`RocksdbConfig`].
pub fn gen_rocksdb_options(config: &RocksdbConfig, readonly: bool) -> Options {
    let mut db_opts = Options::default();
    db_opts.set_max_open_files(config.max_open_files);
    db_opts.set_max_total_wal_size(config.max_total_wal_size);
    db_opts.set_max_background_jobs(config.max_background_jobs);
    if !readonly {
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        db_opts.set_atomic_flush(true);
    }

    db_opts
}
