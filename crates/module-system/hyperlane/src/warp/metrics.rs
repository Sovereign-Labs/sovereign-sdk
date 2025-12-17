use sov_modules_api::Amount;
use std::io::Write;

use super::WarpRouteId;

/// Direction of the rate limiter (inbound or outbound).
#[derive(Debug, Clone, Copy)]
pub enum RateLimiterDirection {
    /// Inbound transfers (receiving from remote chains).
    Inbound,
    /// Outbound transfers (sending to remote chains).
    Outbound,
}

impl std::fmt::Display for RateLimiterDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RateLimiterDirection::Inbound => write!(f, "inbound"),
            RateLimiterDirection::Outbound => write!(f, "outbound"),
        }
    }
}

/// Core capacity metrics for a rate limiter.
#[derive(Debug)]
pub struct RateLimiterCapacityMetrics {
    /// The maximum capacity of tokens that can be transferred.
    pub max_capacity: Amount,
    /// Current available tokens that can be transferred.
    pub current_capacity: Amount,
    /// The rate at which the capacity is replenished per slot.
    pub replenishment_per_slot: Amount,
    /// Direction of the rate limiter (inbound or outbound).
    pub direction: RateLimiterDirection,
    /// The route ID for tagging specific routes.
    pub route_id: WarpRouteId,
}

impl sov_metrics::Metric for RateLimiterCapacityMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_hyperlane_rate_limiter_capacity"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // measurement name & tags
        write!(
            buffer,
            "{},direction={},route_id={}",
            self.measurement_name(),
            self.direction,
            self.route_id,
        )?;

        // fields
        write!(
            buffer,
            " max_capacity={},current_capacity={},replenishment_per_slot={}",
            self.max_capacity.0, self.current_capacity.0, self.replenishment_per_slot.0
        )?;

        Ok(())
    }
}
