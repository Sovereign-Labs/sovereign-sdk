//! SP1-specific telemetry emitted from the host.

use std::io::Write;

use sov_metrics::Metric;
use sp1_sdk::blocking::NetworkProver;
use sp1_sdk::network::B256;

/// Cycles and PGUs reported by the Succinct prover network for one fulfilled
/// proof request. Both values are optional because the network only populates
/// them once the request reaches the EXECUTED state — if we ever observe a
/// fulfilled request without them, we still want the (empty) datapoint so the
/// dashboard surfaces the gap.
#[derive(Debug)]
pub(crate) struct Sp1NetworkProvingMetric {
    /// Identifier for the proven program — `program_name` from the network when
    /// set, otherwise the hex-encoded `vk_hash`. Different ELFs (inner state
    /// transition vs. outer aggregation, or upgraded versions of either)
    /// produce different values, so this tag is what distinguishes circuits in
    /// the dashboard.
    pub program: String,
    pub cycles: Option<u64>,
    pub gas_used: Option<u64>,
}

impl Metric for Sp1NetworkProvingMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_sp1_network_proving"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},program={} cycles={},gas_used={}",
            self.measurement_name(),
            self.program,
            self.cycles.unwrap_or(0),
            self.gas_used.unwrap_or(0),
        )
    }
}

/// Best-effort: fetch the canonical cycles + PGUs the network recorded for
/// `request_id` and submit a [`Sp1NetworkProvingMetric`].
///
/// Telemetry must never fail proving, so any error from the SDK is logged and
/// swallowed.
pub(crate) fn submit_network_proving_metric(network: &NetworkProver, request_id: B256) {
    let (program, cycles, gas_used) = match network.get_proof_request(request_id) {
        Ok(Some(req)) => {
            let program = req
                .program_name
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| hex::encode(&req.vk_hash));
            (program, req.cycles, req.gas_used)
        }
        Ok(None) => {
            tracing::warn!(
                request_id = %request_id,
                "SP1 network returned no ProofRequest details; skipping cycles/gas metric"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                request_id = %request_id,
                error = ?e,
                "Failed to fetch SP1 ProofRequest details for cycles/gas metric"
            );
            return;
        }
    };

    sov_metrics::track_metrics(|tracker| {
        tracker.submit(Sp1NetworkProvingMetric {
            program,
            cycles,
            gas_used,
        });
    });
}
