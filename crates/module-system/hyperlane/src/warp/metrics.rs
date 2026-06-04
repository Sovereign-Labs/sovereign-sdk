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
    /// The remote domain for tagging specific destinations.
    pub remote_domain: u32,
    /// Operator-supplied human-readable name of the route.
    pub route_name: String,
    /// Decimals of the route's local token, used to scale the raw capacities.
    pub decimals: u8,
}

impl sov_metrics::Metric for RateLimiterCapacityMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_hyperlane_rate_limiter_capacity"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // measurement name & tags
        write!(
            buffer,
            "{},direction={},route_id={},remote_domain={},route_name={}",
            self.measurement_name(),
            self.direction,
            self.route_id,
            self.remote_domain,
            escape_tag(&self.route_name),
        )?;

        write!(
            buffer,
            " max_capacity={},current_capacity={},replenishment_per_slot={}",
            scale(self.max_capacity, self.decimals),
            scale(self.current_capacity, self.decimals),
            scale(self.replenishment_per_slot, self.decimals),
        )?;

        Ok(())
    }
}

/// Scales a raw token amount to whole-token units using the token's decimals,
/// rendered as a decimal string (e.g. `742500000` with 6 decimals -> `742.5`).
fn scale(amount: Amount, decimals: u8) -> String {
    let decimals = usize::from(decimals);
    if decimals == 0 {
        return amount.0.to_string();
    }

    let digits = format!("{:0>width$}", amount.0, width = decimals + 1);
    let point = digits.len() - decimals;
    let whole = &digits[..point];
    let frac = digits[point..].trim_end_matches('0');

    if frac.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    }
}

/// escapes a string for use as an InfluxDB line-protocol tag value.
fn escape_tag(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace('=', "\\=")
        .replace(' ', "\\ ")
}

#[cfg(test)]
mod tests {
    use sov_metrics::Metric as _;

    use super::*;

    fn sample(route_name: &str, decimals: u8) -> RateLimiterCapacityMetrics {
        RateLimiterCapacityMetrics {
            max_capacity: Amount(1_000_000_000),
            current_capacity: Amount(742_500_000),
            replenishment_per_slot: Amount(11_500),
            direction: RateLimiterDirection::Inbound,
            route_id: WarpRouteId::from([0xab; 32]),
            remote_domain: 0x1234,
            route_name: route_name.to_string(),
            decimals,
        }
    }

    fn serialize(metrics: &RateLimiterCapacityMetrics) -> String {
        let mut buffer = Vec::new();
        metrics.serialize_for_telegraf(&mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn scales_raw_amounts_by_token_decimals() {
        let line = serialize(&sample("USDC", 6));

        assert!(
            line.contains("max_capacity=1000,current_capacity=742.5,replenishment_per_slot=0.0115"),
            "amounts should be scaled to whole tokens: {line}"
        );
    }

    #[test]
    fn serializes_fractional_scaled_amount() {
        let metrics = RateLimiterCapacityMetrics {
            max_capacity: Amount(1_234_567_891),
            ..sample("USDC", 8)
        };

        let line = serialize(&metrics);

        assert!(
            line.contains("max_capacity=12.34567891"),
            "non-whole amounts should keep their fractional digits: {line}"
        );
    }

    #[test]
    fn escapes_route_name_tag() {
        let line = serialize(&sample("USDC eth,v2", 0));

        assert!(
            line.contains(r"route_name=USDC\ eth\,v2"),
            "spaces and commas in tag values must be escaped: {line}"
        );
    }
}
