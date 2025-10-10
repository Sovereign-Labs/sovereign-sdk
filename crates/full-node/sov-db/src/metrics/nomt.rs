use std::io::Write;

use nomt::hasher::BinaryHasher;
use nomt::Nomt;
use sov_metrics::Metric;
use sov_rollup_interface::reexports::digest;

#[derive(Debug)]
pub struct NomtDbMetric {
    pub db: &'static str,
    pub hash_table_capacity: usize,
    pub hash_table_occupied: usize,
    pub page_requests: u64,
    pub page_cache_misses: u64,
    pub avg_page_fetch_time_ns: Option<u64>,
    pub avg_value_fetch_time_ns: Option<u64>,
}

impl NomtDbMetric {
    pub fn new<H>(db: &'static str, nomt: &Nomt<BinaryHasher<H>>) -> Self
    where
        H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    {
        let hash_table_utilization = nomt.hash_table_utilization();
        if hash_table_utilization.occupancy_rate() > 0.9 {
            tracing::warn!(
                %db,
                rate = hash_table_utilization.occupancy_rate(),
                "Occupancy rate for NOMT hashtable is too high. Please update buckets size and resync database");
        }
        let metrics = nomt.metrics();
        Self {
            db,
            hash_table_capacity: hash_table_utilization.capacity,
            hash_table_occupied: hash_table_utilization.occupied,
            page_requests: metrics.get_page_requests(),
            page_cache_misses: metrics.get_page_cache_misses(),
            avg_page_fetch_time_ns: metrics.get_page_fetch_time(),
            avg_value_fetch_time_ns: metrics.get_value_fetch_time(),
        }
    }
}

impl Metric for NomtDbMetric {
    fn measurement_name(&self) -> &'static str {
        "nomt_db_stats"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // DB as tag, rest as fields
        write!(
            buffer,
            "{},db={} ht_capacity={},ht_occupied={},page_requests={},page_cache_misses={}",
            self.measurement_name(),
            self.db,
            self.hash_table_capacity,
            self.hash_table_occupied,
            self.page_requests,
            self.page_cache_misses,
        )?;
        if let Some(avg_page_fetch_time) = &self.avg_page_fetch_time_ns {
            write!(buffer, ",avg_page_fetch_time_ns={avg_page_fetch_time}")?;
        }
        if let Some(avg_value_fetch_time) = &self.avg_value_fetch_time_ns {
            write!(buffer, ",avg_value_fetch_time_ns={avg_value_fetch_time}")?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct NomtBeginSessionMetric {
    pub db: &'static str,
    pub overlays: usize,
    pub init_time: std::time::Duration,
}

impl Metric for NomtBeginSessionMetric {
    fn measurement_name(&self) -> &'static str {
        "nomt_begin_session"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // DB as tag, rest as fields
        write!(
            buffer,
            "{},db={} overlays={},init_time_us={}",
            self.measurement_name(),
            self.db,
            self.overlays,
            self.init_time.as_micros(),
        )
    }
}

#[derive(Debug)]
pub struct StorageManagerFinalizationMetric {
    pub da_height: u64,
    pub preparation_time: std::time::Duration,
    pub commit_time: std::time::Duration,
    pub pruning_commit_time: Option<std::time::Duration>,
}

impl Metric for StorageManagerFinalizationMetric {
    fn measurement_name(&self) -> &'static str {
        "storage_manager_finalization"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},da_height={} prep_time_us={},commit_time_us={}",
            self.measurement_name(),
            self.da_height,
            self.preparation_time.as_micros(),
            self.commit_time.as_micros(),
        )?;

        if let Some(pruning_time) = &self.pruning_commit_time {
            write!(
                buffer,
                ",pruning_commit_time_us={}",
                pruning_time.as_micros()
            )?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct PrunerMetric {
    pub db: &'static str,
    pub keys_inspected: usize,
    pub keys_to_prune: usize,
    pub time: std::time::Duration,
}

impl Metric for PrunerMetric {
    fn measurement_name(&self) -> &'static str {
        "pruner"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},db={} keys_inspected={},keys_to_prune={},time_us={}",
            self.measurement_name(),
            self.db,
            self.keys_inspected,
            self.keys_to_prune,
            self.time.as_micros(),
        )
    }
}

#[derive(Debug)]
pub struct MerklizedCommitMetric {
    pub flag_prepare: std::time::Duration,
    pub write_attempts_kernel: usize,
    // How much time in total it took to write overlay
    pub write_kernel: std::time::Duration,
    pub flag_mid: std::time::Duration,
    // How many attempts it took to write
    pub write_attempts_user: usize,
    pub write_user: std::time::Duration,
    pub flag_finish: std::time::Duration,
    pub total: std::time::Duration,
}

#[derive(Debug)]
pub struct FlatStateCommitMetric {
    pub prepare: std::time::Duration,
    pub write: std::time::Duration,
}

#[derive(Debug)]
pub struct CommitDetailedMetric {
    pub merklized_commit: MerklizedCommitMetric,
    pub merklized_commit_from_caller: std::time::Duration,
    pub flat: FlatStateCommitMetric,
    pub accessory_commit: std::time::Duration,
    pub ledger_commit: std::time::Duration,
}

impl Metric for CommitDetailedMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_nomt_commit_detailed"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} merklized_flag_prepare_us={},write_attempts_kernel={},write_kernel_us={},write_attempts_user={},write_user_us={},flag_finish_us={},merklized_total_us={},merklized_from_caller_us={},flat_prepare_us={},flat_write_us={},accessory_commit_us={},ledger_commit_us={}",
            self.measurement_name(),
            self.merklized_commit_from_caller.as_micros(),
            self.merklized_commit.flag_prepare.as_micros(),
            self.merklized_commit.write_attempts_kernel,
            self.merklized_commit.write_kernel.as_micros(),
            self.merklized_commit.write_attempts_user,
            self.merklized_commit.write_user.as_micros(),
            self.merklized_commit.flag_finish.as_micros(),
            self.merklized_commit.total.as_micros(),
            self.flat.prepare.as_micros(),
            self.flat.write.as_micros(),
            self.accessory_commit.as_micros(),
            self.ledger_commit.as_micros(),
        )
    }
}
