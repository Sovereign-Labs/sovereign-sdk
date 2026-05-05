//! SP1-specific telemetry emitted from the host.

use std::io::Write;

use sov_metrics::{Metric, ZkCircuit};
use sp1_sdk::blocking::NetworkProver;
use sp1_sdk::network::B256;

/// Cycles and PGUs reported by the Succinct prover network for one fulfilled
/// proof request. Both values are optional because the network only populates
/// them once the request reaches the EXECUTED state — if we ever observe a
/// fulfilled request without them, we still want the (empty) datapoint so the
/// dashboard surfaces the gap.
#[derive(Debug)]
pub(crate) struct Sp1ProvingMetric {
    pub circuit: ZkCircuit,
    pub cycles: Option<u64>,
    pub gas_used: Option<u64>,
}

impl Metric for Sp1ProvingMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_sp1_proving"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},circuit={:?} cycles={},gas_used={}",
            self.measurement_name(),
            self.circuit,
            self.cycles.unwrap_or(0),
            self.gas_used.unwrap_or(0),
        )
    }
}

/// Best-effort: fetch the canonical cycles + PGUs the network recorded for
/// `request_id` and submit a [`Sp1ProvingMetric`].
///
/// Telemetry must never fail proving. Any error or missing-details response is
/// silently dropped — the occasional missing datapoint is acceptable.
pub(crate) fn submit_proving_metric(network: &NetworkProver, request_id: B256, circuit: ZkCircuit) {
    // Best-effort metric: drop on RPC failure or missing details.
    let Ok(Some(req)) = network.get_proof_request(request_id) else {
        return;
    };

    sov_metrics::track_metrics(|tracker| {
        tracker.submit(Sp1ProvingMetric {
            circuit,
            cycles: req.cycles,
            gas_used: req.gas_used,
        });
    });
}
