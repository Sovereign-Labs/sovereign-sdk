// Adapted from Aptos-Core.
// Modified to remove serde dependency

use rocksdb::Options;

/// Port selected RocksDB options for tuning underlying rocksdb instance of AptosDB.
/// see <https://github.com/facebook/rocksdb/blob/master/include/rocksdb/options.h>
/// for detailed explanations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RocksdbConfig {
    pub max_open_files: i32,
    pub max_total_wal_size: u64,
    pub max_background_jobs: i32,
    pub block_cache_size: u64,
    pub block_size: u64,
    pub cache_index_and_filter_blocks: bool,
}

impl Default for RocksdbConfig {
    fn default() -> Self {
        Self {
            // Allow db to close old sst files, saving memory.
            max_open_files: 5000,
            // For now we set the max total WAL size to be 1G. This config can be useful when column
            // families are updated at non-uniform frequencies.
            max_total_wal_size: 1u64 << 30,
            // This includes threads for flashing and compaction. Rocksdb will decide the # of
            // threads to use internally.
            max_background_jobs: 16,
            // Default block cache size is 8MB,
            block_cache_size: 8 * (1u64 << 20),
            // Default block cache size is 4KB,
            block_size: 4 * (1u64 << 10),
            // Whether cache index and filter blocks into block cache.
            cache_index_and_filter_blocks: false,
        }
    }
}

pub fn gen_rocksdb_options(config: &RocksdbConfig, readonly: bool) -> Options {
    let mut db_opts = Options::default();
    db_opts.set_max_open_files(config.max_open_files);
    db_opts.set_max_total_wal_size(config.max_total_wal_size);
    db_opts.set_max_background_jobs(config.max_background_jobs);
    if !readonly {
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
    }

    db_opts
}
