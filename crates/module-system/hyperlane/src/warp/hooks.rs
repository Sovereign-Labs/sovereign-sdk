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
        let monitored_routes = self.get_monitored_routes();
        let visible_slot = state.current_visible_slot_number();

        for monitored_route in monitored_routes {
            if let Ok(Some(route)) = self.warp_routes.get(&monitored_route.id, state) {
                let route_id = monitored_route.id;
                let route_name = &monitored_route.name;
                let decimals = route.token_source.local_decimals();

                for &remote_domain in &route.enrolled_destinations {
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
                        route_name: route_name.clone(),
                        decimals,
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
                        route_name: route_name.clone(),
                        decimals,
                    };

                    sov_metrics::track_metrics(|tracker| {
                        tracker.submit(inbound_metrics);
                        tracker.submit(outbound_metrics);
                    });
                }
            }
        }
    }
}
