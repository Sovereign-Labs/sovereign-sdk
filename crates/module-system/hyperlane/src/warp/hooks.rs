use super::metrics::{RateLimiterCapacityMetrics, RateLimiterDirection};
use super::Warp;
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint, VersionReader as _};

impl<S: Spec> BlockHooks for Warp<S> {
    type Spec = S;

    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<Self::Spec>) {
        self.emit_rate_limiter_metrics(state);
    }
}

impl<S: Spec> Warp<S> {
    fn emit_rate_limiter_metrics(&self, state: &mut StateCheckpoint<S>) {
        let route_ids = self.get_monitored_route_ids();
        let visible_slot = state.current_visible_slot_number();

        tracing::trace!(
            route_count = route_ids.len(),
            "Emitting rate limiter metrics at slot {}",
            visible_slot
        );

        for route_id in route_ids {
            tracing::trace!(%route_id, "Emitting rate limiter metrics for route");

            if let Ok(Some(route)) = self.warp_routes.get(route_id, state) {
                let route_id = *route_id;

                for &remote_domain in &route.enrolled_destinations {
                    tracing::trace!(%route_id, remote_domain, "Collecting rate limiter metrics for destination");

                    let inbound_metrics = RateLimiterCapacityMetrics {
                        max_capacity: route.inbound_rate_limiter.max_limit(),
                        current_capacity: route
                            .inbound_rate_limiter
                            .current_limit_with_replenishment(visible_slot),
                        replenishment_per_slot: route
                            .inbound_rate_limiter
                            .limit_replenishment_per_slot(),
                        direction: RateLimiterDirection::Inbound,
                        route_id,
                        remote_domain,
                    };

                    let outbound_metrics = RateLimiterCapacityMetrics {
                        max_capacity: route.outbound_rate_limiter.max_limit(),
                        current_capacity: route
                            .outbound_rate_limiter
                            .current_limit_with_replenishment(visible_slot),
                        replenishment_per_slot: route
                            .outbound_rate_limiter
                            .limit_replenishment_per_slot(),
                        direction: RateLimiterDirection::Outbound,
                        route_id,
                        remote_domain,
                    };

                    sov_metrics::track_metrics(|tracker| {
                        tracker.submit(inbound_metrics);
                        tracker.submit(outbound_metrics);
                    });
                }
            } else {
                tracing::debug!(%route_id, "Warp route not found when emitting rate limiter metrics");
            }
        }
    }
}
