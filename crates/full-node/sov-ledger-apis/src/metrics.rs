use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use sov_metrics::Metric;
use sov_rollup_interface::node::ledger_api::QueryMode;

const LEDGER_AGGREGATED_PROOFS_WS_ROUTE: &str = "/ledger/aggregated-proofs/latest/ws";
const LEDGER_SLOT_EVENTS_WS_ROUTE: &str = "/ledger/slots/latest/events/ws";
const LEDGER_SLOTS_LATEST_WS_ROUTE: &str = "/ledger/slots/latest/ws";
const LEDGER_SLOTS_FINALIZED_WS_ROUTE: &str = "/ledger/slots/finalized/ws";
const NO_QUERY_MODE_LABEL: &str = "none";

static LEDGER_AGGREGATED_PROOFS_WS_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static LEDGER_SLOT_EVENTS_WS_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static LEDGER_SLOTS_LATEST_WS_COMPACT_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static LEDGER_SLOTS_LATEST_WS_STANDARD_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static LEDGER_SLOTS_LATEST_WS_FULL_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static LEDGER_SLOTS_FINALIZED_WS_COMPACT_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static LEDGER_SLOTS_FINALIZED_WS_STANDARD_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
static LEDGER_SLOTS_FINALIZED_WS_FULL_CONNECTIONS: AtomicU64 = AtomicU64::new(0);

pub(crate) struct LedgerWsConnectionGuard {
    route: &'static str,
    query_mode: &'static str,
    counter: &'static AtomicU64,
}

impl LedgerWsConnectionGuard {
    fn new(route: &'static str, query_mode: &'static str, counter: &'static AtomicU64) -> Self {
        let active_connections = counter.fetch_add(1, Ordering::Relaxed) + 1;
        track_ledger_ws_connections(route, query_mode, active_connections);
        Self {
            route,
            query_mode,
            counter,
        }
    }

    pub(crate) fn aggregated_proofs() -> Self {
        Self::new(
            LEDGER_AGGREGATED_PROOFS_WS_ROUTE,
            NO_QUERY_MODE_LABEL,
            &LEDGER_AGGREGATED_PROOFS_WS_CONNECTIONS,
        )
    }

    pub(crate) fn slot_events() -> Self {
        Self::new(
            LEDGER_SLOT_EVENTS_WS_ROUTE,
            NO_QUERY_MODE_LABEL,
            &LEDGER_SLOT_EVENTS_WS_CONNECTIONS,
        )
    }

    pub(crate) fn head(query_mode: QueryMode) -> Self {
        Self::new(
            LEDGER_SLOTS_LATEST_WS_ROUTE,
            query_mode_label(query_mode),
            head_ws_counter(query_mode),
        )
    }

    pub(crate) fn finalized(query_mode: QueryMode) -> Self {
        Self::new(
            LEDGER_SLOTS_FINALIZED_WS_ROUTE,
            query_mode_label(query_mode),
            finalized_ws_counter(query_mode),
        )
    }
}

impl Drop for LedgerWsConnectionGuard {
    fn drop(&mut self) {
        let previous = self.counter.fetch_sub(1, Ordering::Relaxed);
        let active_connections = previous.saturating_sub(1);
        track_ledger_ws_connections(self.route, self.query_mode, active_connections);
    }
}

#[derive(Debug)]
struct FinalizedWsReplayMetrics {
    query_mode: &'static str,
    replayed_slots: u64,
    missed_slots: u64,
}

impl Metric for FinalizedWsReplayMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_ledger_finalized_ws_replay"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},query_mode={} replayed_slots={},missed_slots={}",
            self.measurement_name(),
            self.query_mode,
            self.replayed_slots,
            self.missed_slots,
        )
    }
}

fn query_mode_label(query_mode: QueryMode) -> &'static str {
    match query_mode {
        QueryMode::Compact => "compact",
        QueryMode::Standard => "standard",
        QueryMode::Full => "full",
    }
}

pub(crate) fn track_finalized_ws_replay(query_mode: QueryMode, replayed_slots: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(FinalizedWsReplayMetrics {
            query_mode: query_mode_label(query_mode),
            replayed_slots,
            missed_slots: replayed_slots.saturating_sub(1),
        });
    });
}

#[derive(Debug)]
struct LedgerWsConnectionsMetric {
    route: &'static str,
    query_mode: &'static str,
    active_connections: u64,
}

impl Metric for LedgerWsConnectionsMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_ledger_ws_connections"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},route={},query_mode={} active_connections={}",
            self.measurement_name(),
            self.route,
            self.query_mode,
            self.active_connections,
        )
    }
}

fn track_ledger_ws_connections(
    route: &'static str,
    query_mode: &'static str,
    active_connections: u64,
) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(LedgerWsConnectionsMetric {
            route,
            query_mode,
            active_connections,
        });
    });
}

fn head_ws_counter(query_mode: QueryMode) -> &'static AtomicU64 {
    match query_mode {
        QueryMode::Compact => &LEDGER_SLOTS_LATEST_WS_COMPACT_CONNECTIONS,
        QueryMode::Standard => &LEDGER_SLOTS_LATEST_WS_STANDARD_CONNECTIONS,
        QueryMode::Full => &LEDGER_SLOTS_LATEST_WS_FULL_CONNECTIONS,
    }
}

fn finalized_ws_counter(query_mode: QueryMode) -> &'static AtomicU64 {
    match query_mode {
        QueryMode::Compact => &LEDGER_SLOTS_FINALIZED_WS_COMPACT_CONNECTIONS,
        QueryMode::Standard => &LEDGER_SLOTS_FINALIZED_WS_STANDARD_CONNECTIONS,
        QueryMode::Full => &LEDGER_SLOTS_FINALIZED_WS_FULL_CONNECTIONS,
    }
}
