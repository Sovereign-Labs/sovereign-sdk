use crate::preferred::rate_limiter::resource::LimitExceeded;
use crate::preferred::rate_limiter::resource::Resource;
use mini_moka::unsync::Cache;
use sov_modules_api::Gas;
use sov_modules_api::Spec;
use std::hash::Hash;
use std::time::Duration;
use std::time::Instant;

/// Configuration for the rate limiter.
#[derive(Debug, Clone)]
pub(crate) struct RateLimiterConfig<S: Spec> {
    pub(crate) ttl_in_milis: u64,
    pub(crate) max_allowed_resources: TotalResources<S::Gas>,
    pub(crate) drain_rate: DrainRatePerMillis<S::Gas>,
}

/// The amount of resources used by a process that needs to be rate-limited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResourceUsed<G: Gas> {
    pub(crate) inner: Resource<G>,
}

/// Defines the [`DrainRatePerMillis`] parameter for the token bucket algorithm. The [`RateLimiter`]
/// tracks the total amount of resources consumed by each key (such as an IP address) and determines
/// whether the resource usage exceeds a predefined threshold.
///
/// Meanwhile, the resources used by a given key are also continuously reduced at the rate specified by
/// [`DrainRatePerMillis`] every millisecond.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrainRatePerMillis<G: Gas> {
    pub(crate) resource_per_ms: Resource<G>,
}

impl<G: Gas> DrainRatePerMillis<G> {
    /// We decrease the resource usage for each [`RateLimiter`] entry every millisecond.
    fn mul_by_millis(&self, since_last_refil: u64) -> TotalResources<G> {
        TotalResources {
            inner: self
                .resource_per_ms
                .saturating_mul_by_scalar(since_last_refil),
        }
    }
}

/// The total resource usage for a given key.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TotalResources<G: Gas> {
    pub(crate) inner: Resource<G>,
}

impl<G: Gas> TotalResources<G> {
    fn zero() -> Self {
        Self {
            inner: Resource::zero(),
        }
    }

    fn combine(&mut self, used: &ResourceUsed<G>) {
        self.inner = self.inner.checked_add(&used.inner).unwrap();
    }

    fn drain(&mut self, other: &Self) {
        self.inner = self.inner.saturating_sub(&other.inner)
    }

    fn allow(&self, max: &Self) -> Result<(), LimitExceeded<G>> {
        self.inner.err_if_exceeding(&max.inner)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Throttler<G: Gas> {
    total_resource_used: TotalResources<G>,
    last_drain: Instant,
}

impl<G: Gas> Throttler<G> {
    /// Checks whether a new request can be processed and frees resources for the throttler
    /// proportional to the amount of time that has passed since the last drain.
    fn allow_request_and_drain_resource_used(
        &mut self,
        now: Instant,
        max_allowed: &TotalResources<G>,
        drain_rate: &DrainRatePerMillis<G>,
    ) -> Result<(), LimitExceeded<G>> {
        self.drain_total_resource_used(now, drain_rate);
        self.total_resource_used.allow(max_allowed)
    }

    /// Increase the resource used by a given trottler/
    fn throttle(&mut self, resource_used: ResourceUsed<G>) {
        self.total_resource_used.combine(&resource_used);
    }

    fn drain_total_resource_used(&mut self, now: Instant, drain_rate: &DrainRatePerMillis<G>) {
        let how_much_to_drain = {
            let since_last_refil = now
                .duration_since(self.last_drain)
                .as_millis()
                .try_into()
                .expect("The throttler has not been evicted from the RateLimiter for more than u64::MAX milliseconds. This is a bug");

            self.last_drain = now;
            drain_rate.mul_by_millis(since_last_refil)
        };

        self.total_resource_used.drain(&how_much_to_drain);
    }
}

/// This is an adaptation of the token bucket algorithm with the following differences:
///
/// 1. The resource being tracked is multidimensional, not just a simple request counter.
///    (This is abstracted behind the [`Resource`] struct.)
/// 2. We do not know how many resources a request will consume until after the request
///    has been executed.
///
/// Therefore, we provide two methods:
/// a) `allow` — checks at the beginning of a request whether the key has not yet exceeded
///    its allowed resource budget.
/// b) `update` — updates the resource usage after the request has completed.
///
/// Because of this separation, it is possible that after `update` a given key will exceed its
/// resource limit. When this happens, subsequent calls to `allow` will reject new requests
/// for that key until enough time has passed for resources to drain below the threshold.
pub(crate) struct RateLimiter<K, S: Spec> {
    data: Cache<K, Throttler<S::Gas>>,
    max_allowed_resources: TotalResources<S::Gas>,
    drain_rate: DrainRatePerMillis<S::Gas>,
}

impl<K: Hash + Eq, S: Spec> RateLimiter<K, S> {
    pub(crate) fn new(config: RateLimiterConfig<S>) -> Self {
        let data = Cache::builder()
            .time_to_live(Duration::from_millis(config.ttl_in_milis))
            .build();

        Self {
            data,
            max_allowed_resources: config.max_allowed_resources,
            drain_rate: config.drain_rate,
        }
    }

    pub(crate) fn allow(
        &mut self,
        now: Instant,
        key: &K,
    ) -> Result<Throttler<S::Gas>, LimitExceeded<S::Gas>> {
        match self.data.get(key).copied() {
            Some(mut throttler) => {
                throttler.allow_request_and_drain_resource_used(
                    now,
                    &self.max_allowed_resources,
                    &self.drain_rate,
                )?;
                Ok(throttler)
            }
            None => Ok(Throttler {
                total_resource_used: TotalResources::zero(),
                last_drain: now,
            }),
        }
    }

    pub(crate) fn update(
        &mut self,
        key: K,
        mut throttler: Throttler<S::Gas>,
        resource_used: ResourceUsed<S::Gas>,
    ) {
        throttler.throttle(resource_used);
        self.data.insert(key, throttler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sov_modules_api::Spec;
    use sov_test_utils::TestSpec;
    use std::thread::sleep;

    type Gas = <TestSpec as Spec>::Gas;

    const MAX_REQ_COUNT: u64 = 10_000;
    const MAX_SPACE_IN_BYTES: u64 = 100_0000;
    const MAX_EXECUTION_TIME_MICROS: u64 = 1000_000;

    #[test]
    fn test_rate_limiter_happy_path() {
        let resource_used_per_run = small_resource_used_per_run();
        let max_allowed_resources = max_allowed_resources();
        let drain_rate = drain_rate();

        let config = RateLimiterConfig::<TestSpec> {
            ttl_in_milis: 1_000_000,
            max_allowed_resources,
            drain_rate,
        };

        let mut rollup_simulator = Simulator::new(config, resource_used_per_run);

        let addr = <TestSpec as Spec>::Address::from([1; 28]);
        let now = Instant::now();

        // After a single run, the rate limiter charged resource_used_per_run.
        {
            rollup_simulator
                .run_and_assert_limits(now, &addr, rollup_simulator.resource_used_per_run)
                .unwrap();
        }

        // After two runs, the rate limiter charged 2*resource_used_per_run.
        {
            let expected_rate_limiter_usage = rollup_simulator.resource_used_per_run.mul(2);
            rollup_simulator
                .run_and_assert_limits(now, &addr, expected_rate_limiter_usage)
                .unwrap();
        }

        // After three runs and some time passed, the rate limiter charged 3 * resource_used_per_run-time - drain_rate * time.
        {
            let time_passed_ms = 15;
            let now = now
                .checked_add(Duration::from_millis(time_passed_ms))
                .unwrap();

            let expected_rate_limiter_usage = rollup_simulator
                .resource_used_per_run
                .mul(3)
                .sub(&rollup_simulator.drained(time_passed_ms));

            rollup_simulator
                .run_and_assert_limits(now, &addr, expected_rate_limiter_usage)
                .unwrap();
        }

        // New address has fresh rate limiter throtler.
        let addr_2 = <TestSpec as Spec>::Address::from([2; 28]);
        let now = Instant::now();
        {
            rollup_simulator
                .run_and_assert_limits(now, &addr_2, rollup_simulator.resource_used_per_run)
                .unwrap();
        }
    }

    #[test]
    fn test_rate_limiter_resources_exhausted() {
        let resource_used_per_run = big_resource_used_per_run();
        let max_allowed_resources = max_allowed_resources();
        let drain_rate = drain_rate();

        let config = RateLimiterConfig::<TestSpec> {
            ttl_in_milis: 1_000_000,
            max_allowed_resources,
            drain_rate,
        };

        let mut rollup_simulator = Simulator::new(config, resource_used_per_run);

        let addr = <TestSpec as Spec>::Address::from([1; 28]);
        let now = Instant::now();

        let mut run_number = 1;

        // Make multiple runs until the rate limiting is triggered.
        let err = loop {
            let expected_rate_limiter_usage =
                rollup_simulator.resource_used_per_run.mul(run_number);

            let res =
                rollup_simulator.run_and_assert_limits(now, &addr, expected_rate_limiter_usage);

            if let Err(err) = res {
                break err;
            }
            run_number += 1;
            // The test is configured to trigger the rate limiting in less than 10 runs.
            assert!(run_number < 10, "Unable to exhaust the rate limiter");
        };

        // Rate limiting has been triggered.
        assert!(matches!(err, LimitExceeded::RequestCount { .. }));

        // Simulate a time pass of 3 ms. This will drain the total resources used in the rate limiter, but not enough to allow a new request.
        {
            let time_passed_ms = 3;
            let now = now
                .checked_add(Duration::from_millis(time_passed_ms))
                .unwrap();

            let expected_rate_limiter_usage = rollup_simulator
                .resource_used_per_run
                .mul(run_number)
                .sub(&rollup_simulator.drained(time_passed_ms));

            let err = rollup_simulator
                .run_and_assert_limits(now, &addr, expected_rate_limiter_usage)
                .unwrap_err();

            assert!(matches!(err, LimitExceeded::RequestCount { .. }));
        }

        // Simulate a time pass of 20 ms. This is enough to allow a new request.
        {
            let time_passed_ms = 20;
            let now = now
                .checked_add(Duration::from_millis(time_passed_ms))
                .unwrap();

            let expected_rate_limiter_usage = rollup_simulator
                .resource_used_per_run
                .mul(run_number)
                .sub(&rollup_simulator.drained(time_passed_ms));

            let res =
                rollup_simulator.run_and_assert_limits(now, &addr, expected_rate_limiter_usage);

            assert!(res.is_ok());
        }
    }

    #[test]
    fn test_rate_limiter_ttl() {
        let resource_used_per_run = big_resource_used_per_run();
        let max_allowed_resources = max_allowed_resources();
        let drain_rate = drain_rate();

        let config = RateLimiterConfig::<TestSpec> {
            ttl_in_milis: 2,
            max_allowed_resources,
            drain_rate,
        };

        let mut rollup_simulator = Simulator::new(config, resource_used_per_run);

        let addr = <TestSpec as Spec>::Address::from([1; 28]);
        let now = Instant::now();

        rollup_simulator
            .run_and_assert_limits(now, &addr, rollup_simulator.resource_used_per_run)
            .unwrap();

        // We configured the rate limiter with `ttl_in_millis = 2`. After 5 ms, the entry should be evicted.
        // Unfortunately, we cannot test this without using `sleep`.
        sleep(Duration::from_millis(5));

        let throttler = rollup_simulator.rate_limiter.data.get(&addr);
        assert!(throttler.is_none());
    }

    struct Simulator {
        rate_limiter: RateLimiter<<TestSpec as Spec>::Address, TestSpec>,
        resource_used_per_run: ResourceUsed<Gas>,
    }

    impl Simulator {
        fn new(
            config: RateLimiterConfig<TestSpec>,
            resource_used_per_run: ResourceUsed<Gas>,
        ) -> Self {
            let rate_limiter = RateLimiter::new(config);
            Self {
                rate_limiter,
                resource_used_per_run,
            }
        }

        fn run_and_assert_limits(
            &mut self,
            now: Instant,
            addr: &<TestSpec as Spec>::Address,
            resource_used_so_far: ResourceUsed<Gas>,
        ) -> Result<(), LimitExceeded<Gas>> {
            let throttler = self.rate_limiter.allow(now, &addr)?;

            self.rate_limiter
                .update(addr.clone(), throttler, self.resource_used_per_run);

            let throttler = self.rate_limiter.get_throtler(&addr).unwrap();

            assert_eq!(
                throttler.total_resource_used.inner,
                resource_used_so_far.inner
            );

            Ok(())
        }

        fn drained(&self, time_passed_ms: u64) -> Resource<Gas> {
            self.rate_limiter
                .drain_rate
                .resource_per_ms
                .saturating_mul_by_scalar(time_passed_ms)
        }
    }

    impl<K: Hash + Eq, S: Spec> RateLimiter<K, S> {
        fn get_throtler(&mut self, key: &K) -> Option<&Throttler<S::Gas>> {
            self.data.get(key)
        }
    }

    impl ResourceUsed<Gas> {
        fn mul(&self, n: u64) -> Self {
            Self {
                inner: self.inner.saturating_mul_by_scalar(n),
            }
        }

        fn sub(&self, resource: &Resource<Gas>) -> Self {
            Self {
                inner: self.inner.saturating_sub(&resource),
            }
        }
    }

    fn max_allowed_resources() -> TotalResources<Gas> {
        TotalResources {
            inner: Resource {
                req_counter: MAX_REQ_COUNT,
                space_in_bytes: MAX_SPACE_IN_BYTES,
                execution_time_micros: MAX_EXECUTION_TIME_MICROS,
                gas_used: Gas::from([0, 0]),
            },
        }
    }

    fn small_resource_used_per_run() -> ResourceUsed<Gas> {
        ResourceUsed {
            inner: Resource {
                req_counter: MAX_REQ_COUNT / 100,
                space_in_bytes: MAX_SPACE_IN_BYTES / 200,
                execution_time_micros: MAX_EXECUTION_TIME_MICROS / 300,
                gas_used: Gas::from([0, 0]),
            },
        }
    }

    fn big_resource_used_per_run() -> ResourceUsed<Gas> {
        ResourceUsed {
            inner: Resource {
                req_counter: MAX_REQ_COUNT / 2 + 10,
                space_in_bytes: MAX_SPACE_IN_BYTES / 3,
                execution_time_micros: MAX_EXECUTION_TIME_MICROS / 4,
                gas_used: Gas::from([0, 0]),
            },
        }
    }

    fn drain_rate() -> DrainRatePerMillis<Gas> {
        DrainRatePerMillis {
            resource_per_ms: Resource {
                req_counter: 2,
                space_in_bytes: 30,
                execution_time_micros: 400,
                gas_used: Gas::from([0, 0]),
            },
        }
    }
}
