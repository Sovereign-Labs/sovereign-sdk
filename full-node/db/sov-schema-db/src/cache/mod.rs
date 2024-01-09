/// CacheContainer module responsible for correct traversal of cache layers
pub mod cache_container;
/// CacheDb is main entry point into
pub mod cache_db;
/// Collection of writes in given Snapshot/Cache Layer
pub mod change_set;

/// Id of ChangeSet/Snapshot/Cache layer
pub type SnapshotId = u64;
