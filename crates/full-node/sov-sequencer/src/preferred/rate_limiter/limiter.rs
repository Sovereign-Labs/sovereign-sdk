use crate::preferred::rate_limiter::resource::LimitExceeded;
use crate::preferred::rate_limiter::resource::Resource;
use mini_moka::sync::Cache;
use sov_modules_api::Gas;
use sov_modules_api::Spec;
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::time::Duration;
use std::time::Instant;

/// Configuration for the rate limiter.
#[derive(Debug, Clone)]
pub(crate) struct RateLimiterConfig<S: Spec> {
    pub(crate) max_allowed_resources: TotalResources<S::Gas>,
    pub(crate) refill_rate: RefillRatePerMillis<S::Gas>,
}

/// The amount of resources used by a request that needs to be rate-limited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResourceUsed<G: Gas> {
    pub(crate) inner: Resource<G>,
}

impl<G: Gas> ResourceUsed<G> {
    pub(crate) fn zero() -> Self {
        Self {
            inner: Resource::zero(),
        }
    }

    pub(crate) fn new(
        req_counter: u64,
        tx_size_in_bytes: usize,
        execution_time_micros: u64,
        gas_used: G,
    ) -> Self {
        Self {
            inner: Resource {
                req_counter,
                // This cast is safe because a single transaction can never use more than u64::MAX bytes.
                space_in_bytes: tx_size_in_bytes
                    .try_into()
                    .expect("Alowed transactions size overflows u64::MAX"),
                execution_time_micros,
                gas_used,
            },
        }
    }
}

/// [`RefillRatePerMillis`] determines how quickly used resources are refilled.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RefillRatePerMillis<G: Gas> {
    pub(crate) token_resource_per_ms: Resource<G>,
}

impl<G: Gas> RefillRatePerMillis<G> {
    /// The total amount refilled is calculated as token_resource_per_ms multiplied by the time elapsed since the last refill.
    fn mul_by_millis(&self, since_last_refill: u64) -> Resource<G> {
        self.token_resource_per_ms
            .saturating_mul_by_scalar(since_last_refill)
    }
}

/// The total resource usage for a given key (see [`Throttler`] bellow]).
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

    #[must_use]
    fn combine(&self, used: &ResourceUsed<G>) -> Option<Self> {
        Some(Self {
            inner: self.inner.checked_add(&used.inner)?,
        })
    }

    #[must_use]
    fn refill_tokens(&self, how_much_to_refill: &Resource<G>) -> Self {
        // In [`Throttler`] we track the resources used by a given key.
        // Therefore, refilling means decreasing the amount of resources used.
        Self {
            inner: self.inner.saturating_sub(how_much_to_refill),
        }
    }

    fn allow(&self, max: &Self) -> Result<(), LimitExceeded<G>> {
        self.inner.err_if_exceeding(&max.inner)
    }
}

/// Tracks the resources consumed for a given key (for example, an IP address),
/// as well as the last time `total_resource_used` was refilled.
///
/// The Throttler grants permission for requests as long as `total_resource_used`
/// remains below `max_allowed_resources`. Once it exceeds that threshold,
/// the Throttler stops granting permissions until `total_resource_used` drops
/// back below `max_allowed_resources`.
///
/// This decrease happens due to a constant stream of resource tokens that
/// continuously reduce `total_resource_used`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Throttler<G: Gas> {
    total_resource_used: TotalResources<G>,
    last_refill: Instant,
}

impl<G: Gas> Throttler<G> {
    /// Checks whether a new request can be processed and frees resources for the throttler
    /// proportional to the amount of time that has passed since the last refill.
    fn allow_request_and_refill_resource_used(
        &self,
        now: Instant,
        max_allowed_resources: &TotalResources<G>,
        refill_rate: &RefillRatePerMillis<G>,
    ) -> Result<Self, LimitExceeded<G>> {
        let how_much_to_fill = {
            let since_last_refil = now
                .duration_since(self.last_refill)
                .as_millis()
                .try_into()
                .expect("The throttler has not been evicted from the RateLimiter for more than u64::MAX milliseconds. This is a bug");

            refill_rate.mul_by_millis(since_last_refil)
        };

        let total_resource_used_after_refill =
            self.total_resource_used.refill_tokens(&how_much_to_fill);

        total_resource_used_after_refill.allow(max_allowed_resources)?;

        Ok(Throttler {
            total_resource_used: total_resource_used_after_refill,
            last_refill: now,
        })
    }

    /// Increase the resource used by a given throttler.
    #[must_use]
    fn throttle<K: Debug>(&self, resource_used: ResourceUsed<G>, key: &K) -> Self {
        let used = match self.total_resource_used.combine(&resource_used) {
            Some(used) => used,
            None => {
                // On overflow we issue an error but we won't panic.
                tracing::error!(
                    "Throttler for {:?} overflowed: Initial total resource used {:?}, resource increase {:?}",
                    key,
                    self.total_resource_used,
                    resource_used
                );
                TotalResources {
                    inner: Resource::max(),
                }
            }
        };

        Self {
            total_resource_used: used,
            last_refill: self.last_refill,
        }
    }
}

/// This is an adaptation of the token bucket algorithm with the following differences:
///
/// 1. The resource being tracked is multidimensional, not just a simple request counter.
///    (This is abstracted behind the [`Resource`] struct.)
/// 2. A request’s resource usage is unknown until execution finishes.
///
/// Therefore, we provide two methods:
/// a) `allow` — checks at the beginning of a request whether the key has not yet exceeded
///    its allowed resource budget.
/// b) `update` — updates the resource usage after the request has completed.
///
/// Due to this separation, it is possible for a key to exceed its resource limit *after*
/// `update` is called. When this happens, subsequent calls to `allow` will reject new
/// requests for that key until enough time has passed for resources to drain below
/// the allowed threshold.
///
/// More details about `allow`:
///  - Determines how much time has passed since the last refill.
///  - Calculates  `tokens_to_refill = refill_rate * time_since_last_refill``.
///  - Each token decreases `total_resource_used`.
///  - If `total_resource_used` is below `max_allowed_total_resource_used`, the request is allowed.
///  
/// EXAMPLE: (All requests are for the same key (IP address)):
///
/// Suppose we are only interested in `execution_time_micros`.
///
/// MAX_TOTAL_ALLOWED_EXECUTION_TIME_MICROS = 10 micros  
/// REFILL_RATE = 2micros/ms
///
/// Request1 consumes  8 micros  
/// Request2 consumes 15 micros  
/// Request3 consumes  3 micros
///
/// After Request 1:
///   total_resource_used = 8 micros
///
/// TIME_PASSED = 1 ms  
/// Request2 arrives
///
/// allow:
///   total_resource_used = 8 micros - REFILL_RATE * TIME_PASSED | 6 micros  
///   6 micros <= MAX_TOTAL_ALLOWED_EXECUTION_TIME_MICROS → Request2 is allowed
///
/// After Request2:
/// update:
///   total_resource_used = 6 micros + 15 micros = 21 mmicros
///
/// TIME_PASSED_SINCE_REQ2 = 1 ms  
/// Request3 arrives
///
/// allow:
///   total_resource_used = 21 ms - REFILL_RATE * TIME_PASSED_SINCE_REQ2 | 19 micros  
///   19 micros > MAX_TOTAL_ALLOWED_EXECUTION_TIME_MICROS → Request3 is *not* allowed
///
/// TIME_PASSED_SINCE_REQ2 = 20 ms  
/// Request 3 arrives again
///
/// allow:
///   total_resource_used = 19 micros - REFILL_RATE * TIME_PASSED_SINCE_REQ2 | 0 micros  // using saturating_sub  
///   0 micros <= MAX_TOTAL_ALLOWED_EXECUTION_TIME_MICROS → Request 3 is allowed
///
/// After Request 3:
/// update:
///   total_resource_used = 3 micros
pub(crate) struct RateLimiter<K, S: Spec> {
    data: Cache<K, Throttler<S::Gas>>,
    default_config: RateLimiterConfig<S>,
    special_configs: HashMap<K, RateLimiterConfig<S>>,
}

impl<K: Hash + Eq + Debug + Send + Sync + 'static, S: Spec> RateLimiter<K, S> {
    pub(crate) fn new(
        ttl_in_millis: u64,
        default_config: RateLimiterConfig<S>,
        special_configs: HashMap<K, RateLimiterConfig<S>>,
    ) -> Self {
        let data = Cache::builder()
            .time_to_live(Duration::from_millis(ttl_in_millis))
            .build();

        Self {
            data,
            default_config,
            special_configs,
        }
    }

    pub(crate) fn allow(
        &mut self,
        now: Instant,
        key: &K,
    ) -> Result<Throttler<S::Gas>, LimitExceeded<S::Gas>> {
        match self.data.get(key) {
            Some(throttler) => {
                let config = self.get_config(key);
                Ok(throttler.allow_request_and_refill_resource_used(
                    now,
                    &config.max_allowed_resources,
                    &config.refill_rate,
                )?)
            }
            None => Ok(Throttler {
                total_resource_used: TotalResources::zero(),
                last_refill: now,
            }),
        }
    }

    pub(crate) fn update(
        &mut self,
        key: K,
        throttler: Throttler<S::Gas>,
        resource_used: ResourceUsed<S::Gas>,
    ) -> Throttler<S::Gas> {
        let throttler = throttler.throttle(resource_used, &key);
        self.data.insert(key, throttler);
        throttler
    }

    fn get_config(&self, k: &K) -> &RateLimiterConfig<S> {
        self.special_configs.get(k).unwrap_or(&self.default_config)
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
    const MAX_EXECUTION_TIME_MICROS: u64 = 1_000_000;
    const TTL_IN_MILLIS: u64 = 1_000_000;

    #[test]
    fn test_rate_limiter_happy_path() {
        let resource_used_per_run = small_resource_used_per_run();
        let max_allowed_resources = max_allowed_resources();
        let refill_rate = refill_rate();

        let config = RateLimiterConfig::<TestSpec> {
            max_allowed_resources,
            refill_rate,
        };

        let mut rollup_simulator = Simulator::new(
            TTL_IN_MILLIS,
            config,
            resource_used_per_run,
            Default::default(),
        );

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

        // After three runs and some time passed, the rate limiter charged 3 * resource_used_per_run-time - refill_rate * time.
        {
            let time_passed_ms = 15;
            let now = now
                .checked_add(Duration::from_millis(time_passed_ms))
                .unwrap();

            let expected_rate_limiter_usage = rollup_simulator
                .resource_used_per_run
                .mul(3)
                .refill(&rollup_simulator.resource_tokens_to_refill(time_passed_ms, addr));

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
    fn test_rate_limiter_happy_path_sepcial_address() {
        let resource_used_per_run = small_resource_used_per_run();

        let default_config = RateLimiterConfig::<TestSpec> {
            max_allowed_resources: TotalResources::zero(),
            refill_rate: RefillRatePerMillis::zero(),
        };

        let addr = <TestSpec as Spec>::Address::from([1; 28]);
        let special_addr = <TestSpec as Spec>::Address::from([2; 28]);

        let special_keys = HashMap::from([(
            special_addr,
            RateLimiterConfig::<TestSpec> {
                max_allowed_resources: max_allowed_resources(),
                refill_rate: RefillRatePerMillis::zero(),
            },
        )]);

        let mut rollup_simulator = Simulator::new(
            TTL_IN_MILLIS,
            default_config,
            resource_used_per_run,
            special_keys,
        );

        let now = Instant::now();

        // After a single run, the rate limiter charged resource_used_per_run.
        {
            rollup_simulator
                .run_and_assert_limits(now, &addr, rollup_simulator.resource_used_per_run)
                .unwrap();

            rollup_simulator
                .run_and_assert_limits(now, &special_addr, rollup_simulator.resource_used_per_run)
                .unwrap();
        }

        // After two runs, the standard addr is rate limitied but special_addr has higher limits.
        {
            let expected_rate_limiter_usage = rollup_simulator.resource_used_per_run.mul(2);
            rollup_simulator
                .run_and_assert_limits(now, &addr, expected_rate_limiter_usage)
                .unwrap_err();

            rollup_simulator
                .run_and_assert_limits(now, &special_addr, expected_rate_limiter_usage)
                .unwrap();
        }
    }

    #[test]
    fn test_rate_limiter_resources_exhausted() {
        let resource_used_per_run = big_resource_used_per_run();
        let max_allowed_resources = max_allowed_resources();
        let refill_rate = refill_rate();

        let config = RateLimiterConfig::<TestSpec> {
            max_allowed_resources,
            refill_rate,
        };

        let mut rollup_simulator = Simulator::new(
            TTL_IN_MILLIS,
            config,
            resource_used_per_run,
            Default::default(),
        );

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
                .refill(&rollup_simulator.resource_tokens_to_refill(time_passed_ms, addr));

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
                .refill(&rollup_simulator.resource_tokens_to_refill(time_passed_ms, addr));

            let res =
                rollup_simulator.run_and_assert_limits(now, &addr, expected_rate_limiter_usage);

            assert!(res.is_ok());
        }
    }

    #[test]
    fn test_rate_limiter_ttl() {
        let resource_used_per_run = big_resource_used_per_run();
        let max_allowed_resources = max_allowed_resources();
        let refill_rate = refill_rate();

        let config = RateLimiterConfig::<TestSpec> {
            max_allowed_resources,
            refill_rate,
        };

        let mut rollup_simulator =
            Simulator::new(2, config, resource_used_per_run, Default::default());

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
            ttl_in_millis: u64,
            config: RateLimiterConfig<TestSpec>,
            resource_used_per_run: ResourceUsed<Gas>,
            special_configs: HashMap<<TestSpec as Spec>::Address, RateLimiterConfig<TestSpec>>,
        ) -> Self {
            let rate_limiter = RateLimiter::new(ttl_in_millis, config, special_configs);
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
            let throttler = self.rate_limiter.allow(now, addr)?;

            let throttler = self
                .rate_limiter
                .update(*addr, throttler, self.resource_used_per_run);

            assert_eq!(
                throttler.total_resource_used.inner,
                resource_used_so_far.inner
            );

            Ok(())
        }

        fn resource_tokens_to_refill(
            &self,
            time_passed_ms: u64,
            addr: <TestSpec as Spec>::Address,
        ) -> Resource<Gas> {
            self.rate_limiter
                .get_config(&addr)
                .refill_rate
                .token_resource_per_ms
                .saturating_mul_by_scalar(time_passed_ms)
        }
    }

    impl ResourceUsed<Gas> {
        fn mul(&self, n: u64) -> Self {
            Self {
                inner: self.inner.saturating_mul_by_scalar(n),
            }
        }

        fn refill(&self, resource: &Resource<Gas>) -> Self {
            Self {
                inner: self.inner.saturating_sub(resource),
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

    fn refill_rate() -> RefillRatePerMillis<Gas> {
        RefillRatePerMillis {
            token_resource_per_ms: Resource {
                req_counter: 2,
                space_in_bytes: 30,
                execution_time_micros: 400,
                gas_used: Gas::from([0, 0]),
            },
        }
    }

    impl RefillRatePerMillis<Gas> {
        fn zero() -> Self {
            Self {
                token_resource_per_ms: Resource::zero(),
            }
        }
    }
}
