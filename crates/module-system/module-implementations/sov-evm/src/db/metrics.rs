#[cfg(feature = "native")]
use std::time::Duration;

use alloy_primitives::{Address, StorageValue, B256};
use derive_more::{Deref, DerefMut};
use revm::{
    primitives::StorageKey,
    state::{AccountInfo, Bytecode},
    Database,
};
#[cfg(feature = "native")]
use sov_metrics::Metric;
#[cfg(feature = "native")]
use std::io::Write;

#[cfg(feature = "native")]
#[derive(Debug, Default, Clone, Copy)]
pub struct DbAccess {
    duration: Duration,
    count: usize,
}

#[cfg(feature = "native")]
impl DbAccess {
    pub fn push(&mut self, t: Duration) {
        self.duration += t;
        self.count += 1;
    }
}

#[cfg(feature = "native")]
#[derive(Debug, Default, Clone, Copy)]
pub struct DbMetrics {
    account: DbAccess,
    code: DbAccess,
    storage: DbAccess,
    block_hash: DbAccess,
}

#[cfg(feature = "native")]
impl Metric for DbMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_evm_db_metrics"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let fields: &[(&str, DbAccess)] = &[
            ("account", self.account),
            ("code", self.code),
            ("storage", self.storage),
            ("block_hash", self.block_hash),
        ];
        write!(buffer, "{}", self.measurement_name())?;
        for (i, (name, val)) in fields.iter().enumerate() {
            let sep = if i == 0 { ' ' } else { ',' };
            write!(
                buffer,
                "{sep}{name}={},{name}_count={}",
                val.duration.as_micros(),
                val.count
            )?;
        }
        Ok(())
    }
}

#[cfg(feature = "native")]
impl DbMetrics {
    fn basic(&mut self, t: Duration) {
        self.account.push(t);
    }
    fn code_by_hash(&mut self, t: Duration) {
        self.code.push(t);
    }
    fn storage(&mut self, t: Duration) {
        self.storage.push(t);
    }
    fn block_hash(&mut self, t: Duration) {
        self.block_hash.push(t);
    }
}

#[derive(Deref, DerefMut)]
pub struct MetricsDb<ExtDB> {
    #[cfg(feature = "native")]
    metrics: DbMetrics,
    #[deref]
    #[deref_mut]
    db: ExtDB,
}

impl<ExtDb> MetricsDb<ExtDb> {
    pub fn new(db: ExtDb) -> Self {
        Self {
            #[cfg(feature = "native")]
            metrics: Default::default(),
            db,
        }
    }

    #[cfg(feature = "native")]
    pub fn metrics(&self) -> DbMetrics {
        self.metrics
    }
}

#[macro_export]
/// Times wrapped DB call and writes to DB metrics
macro_rules! time {
    ($self:ident . db . $name:ident ( $($args:expr),* $(,)? )) => {{
        #[cfg(feature = "native")]
        let __t0 = ::std::time::Instant::now();
        let __ret = $self.db.$name($($args),*);
        #[cfg(feature = "native")]
        $self.metrics.$name(__t0.elapsed());
        __ret
    }};
}

impl<ExtDB: Database> Database for MetricsDb<ExtDB> {
    type Error = ExtDB::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        time!(self.db.basic(address))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        time!(self.db.code_by_hash(code_hash))
    }

    fn storage(
        &mut self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        time!(self.db.storage(address, index))
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        time!(self.db.block_hash(number))
    }
}
