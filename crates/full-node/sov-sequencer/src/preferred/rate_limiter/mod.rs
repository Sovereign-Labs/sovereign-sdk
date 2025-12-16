#![allow(dead_code)]
mod limiter;
mod resource;

pub(crate) use limiter::ResourceUsed;
use limiter::*;
use resource::*;
use sov_full_node_configs::sequencer::{Limits, SovRateLimiterConfig};
use sov_modules_api::BasicAddress;
use sov_modules_api::{CredentialId, Spec};
use std::collections::HashMap;
use std::hash::Hash;
use std::{net::IpAddr, time::Instant};

use crate::preferred::sync_sequencer_state::comfortable_gas_limit;

#[derive(Debug)]
pub(crate) struct LimiterToken<S: Spec> {
    address: S::Address,
    throttler_for_addr: Throttler<S::Gas>,

    ip: IpAddr,
    throttler_for_ip: Throttler<S::Gas>,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum ResourceLimitExceededError<S: Spec> {
    #[error("Resource limit exceeded for address: {address}, {reason:?}")]
    Address {
        address: S::Address,
        reason: LimitExceeded<S::Gas>,
    },
    #[error("Resource limit exceeded for IP: {ip}, {reason:?}")]
    Ip {
        ip: IpAddr,
        reason: LimitExceeded<S::Gas>,
    },
}

struct SovRateLimiterInner<S: Spec> {
    by_addr_rate_limiter: RateLimiter<S::Address, S>,
    by_ip_rate_limiter: RateLimiter<IpAddr, S>,
}

impl<S: Spec> SovRateLimiterInner<S> {
    fn new(
        ttl_in_millis: u64,
        config: RateLimiterConfig<S>,
        addrs: HashMap<S::Address, RateLimiterConfig<S>>,
        ips: HashMap<IpAddr, RateLimiterConfig<S>>,
    ) -> Self {
        Self {
            by_addr_rate_limiter: RateLimiter::new(ttl_in_millis, config.clone(), addrs),
            by_ip_rate_limiter: RateLimiter::new(ttl_in_millis, config, ips),
        }
    }

    fn allow(
        &mut self,
        ip: IpAddr,
        address: S::Address,
    ) -> Result<LimiterToken<S>, ResourceLimitExceededError<S>> {
        let now = Instant::now();
        let throttler_for_addr = match self.by_addr_rate_limiter.allow(now, &address) {
            Ok(ok) => ok,
            Err(reason) => return Err(ResourceLimitExceededError::Address { address, reason }),
        };

        let throttler_for_ip = self
            .by_ip_rate_limiter
            .allow(now, &ip)
            .map_err(|reason| ResourceLimitExceededError::Ip { ip, reason })?;

        Ok(LimiterToken {
            address,
            throttler_for_addr,
            ip,
            throttler_for_ip,
        })
    }

    fn update(&mut self, token: LimiterToken<S>, resource_used: ResourceUsed<S::Gas>) {
        self.by_addr_rate_limiter
            .update(token.address, token.throttler_for_addr, resource_used);
        self.by_ip_rate_limiter
            .update(token.ip, token.throttler_for_ip, resource_used);
    }
}

const TTL_MULTIPLIER: u64 = 20;

fn clculate_limits<S: Spec>(
    limits: Limits,
    max_requests_per_second: u64,
    batch_execution_time_limit_millis: u64,
    max_batch_size_bytes: usize,
) -> RateLimiterConfig<S> {
    let max_gas = comfortable_gas_limit::<S>();

    let max_resources_per_batch = Resource {
        req_counter: max_requests_per_second
            .checked_mul(batch_execution_time_limit_millis)
            .expect("Unable to covert max_requests_per_second to req_counter"),
        // The expect is justified because we will never have batches larger than u64::MAX bytes.
        space_in_bytes: max_batch_size_bytes
            .try_into()
            .expect("Alowed batch size overflows u64::MAX"),
        // The expect is justified because batches will never take longer than u64::MAX microseconds.
        execution_time_micros: batch_execution_time_limit_millis.checked_mul(1000).expect(
            "Converting batch_execution_time_limit_millis to microseconds would overflow u64::MAX.",
        ),
        gas_used: max_gas,
    };

    // A single sender is limited to 1/max_threshold_per_key_to_batch_capacity_ratio of resources of a single batch.
    let max_per_key = max_resources_per_batch
        .saturating_mul_by_scalar(limits.max_user_bursts_per_batch)
        .div_by_scalar(1000);

    // The refill rate is defined as 0.1% of max_per_key. After one second, the system refills max_per_key tokens.
    let refill_rate = max_per_key
        .saturating_mul_by_scalar(limits.refill_rate)
        .div_by_scalar(batch_execution_time_limit_millis);

    RateLimiterConfig {
        max_allowed_resources: TotalResources { inner: max_per_key },
        refill_rate: RefillRatePerMillis {
            token_resource_per_ms: refill_rate,
        },
    }
}

pub(crate) struct SovRateLimiter<S: Spec> {
    /// If [`SovRateLimiter`] is initialized with a None config, this field is None.
    /// Not providing a config is the mechanism for disabling the rollup’s rate limiter.
    inner: Option<SovRateLimiterInner<S>>,
}

fn to_limiter_config_map<K: Eq + Hash, S: Spec>(
    max_requests_per_second: u64,
    batch_execution_time_limit_millis: u64,
    max_batch_size_bytes: usize,
    v: Vec<(K, Limits)>,
) -> HashMap<K, RateLimiterConfig<S>> {
    v.into_iter()
        .map(|(key, limits)| {
            (
                key,
                clculate_limits::<S>(
                    limits,
                    max_requests_per_second,
                    batch_execution_time_limit_millis,
                    max_batch_size_bytes,
                ),
            )
        })
        .collect()
}

#[allow(clippy::type_complexity)]
fn limits<S: Spec>(
    sov_config: SovRateLimiterConfig<S::Address>,
    batch_execution_time_limit_millis: u64,
    max_batch_size_bytes: usize,
) -> (
    RateLimiterConfig<S>,
    HashMap<S::Address, RateLimiterConfig<S>>,
    HashMap<IpAddr, RateLimiterConfig<S>>,
) {
    let default_config = clculate_limits::<S>(
        sov_config.default_limits,
        sov_config.max_requests_per_second,
        batch_execution_time_limit_millis,
        max_batch_size_bytes,
    );

    let addrs = to_limiter_config_map::<S::Address, S>(
        batch_execution_time_limit_millis,
        sov_config.max_requests_per_second,
        max_batch_size_bytes,
        sov_config.address_custom_limits,
    );

    let ips = to_limiter_config_map::<IpAddr, S>(
        batch_execution_time_limit_millis,
        sov_config.max_requests_per_second,
        max_batch_size_bytes,
        sov_config.ip_custom_limits,
    );

    (default_config, addrs, ips)
}

impl<S: Spec> SovRateLimiter<S> {
    pub(crate) fn new(
        config: Option<SovRateLimiterConfig<S::Address>>,
        batch_execution_time_limit_millis: u64,
        max_batch_size_bytes: usize,
    ) -> Self {
        let inner = config.map(|sov_config| {
            // All entries older than this value are evicted from the rate limiter.
            let ttl_in_millis = batch_execution_time_limit_millis * TTL_MULTIPLIER;

            let (config, addrs, ips) = limits(
                sov_config,
                batch_execution_time_limit_millis,
                max_batch_size_bytes,
            );
            SovRateLimiterInner::new(ttl_in_millis, config, addrs, ips)
        });
        Self { inner }
    }

    pub(crate) fn allow(
        &mut self,
        ip: IpAddr,
        address: S::Address,
    ) -> Result<Option<LimiterToken<S>>, ResourceLimitExceededError<S>> {
        self.inner
            .as_mut()
            .map(|inner| inner.allow(ip, address))
            .transpose()
    }

    pub(crate) fn update(
        &mut self,
        token: Option<LimiterToken<S>>,
        resource_used: ResourceUsed<S::Gas>,
    ) {
        if let Some(token) = token {
            let inner = self.inner.as_mut().expect("The impossible happened: SovRateLimiter is unavailable even though a Some(Token) was provided.");
            inner.update(token, resource_used);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Limits;
    use sov_test_utils::TestSpec;
    use std::net::Ipv4Addr;

    type Gas = <TestSpec as Spec>::Gas;

    #[test]
    fn test_clculate_limits() {
        let limits = Limits {
            max_user_bursts_per_batch: 5,
            refill_rate: 1,
        };

        let max_batch_exec_time = 6000;
        let rate_limiter_config =
            clculate_limits::<TestSpec>(limits, 10000, max_batch_exec_time, 6000000);

        let max_allowed_resources_per_key = rate_limiter_config.max_allowed_resources;

        // In one ms we refill max_allowed_resources_per_key tokens.
        let refilled = rate_limiter_config
            .refill_rate
            .token_resource_per_ms
            .saturating_mul_by_scalar(max_batch_exec_time);

        assert_eq!(
            max_allowed_resources_per_key.inner.space_in_bytes,
            refilled.space_in_bytes
        );
    }

    #[test]
    fn test_sov_test_limiter() {
        let resource_used_per_run = ResourceUsed {
            inner: Resource {
                req_counter: 10,
                space_in_bytes: 3000,
                execution_time_micros: 30,
                gas_used: Gas::from([0, 0]),
            },
        };

        // This is enough to make 2 requests.
        let max_allowed_resources = TotalResources {
            inner: Resource {
                req_counter: 100000,
                space_in_bytes: 5000,
                execution_time_micros: 100000,

                gas_used: Gas::from([0, 0]),
            },
        };

        let refill_rate = RefillRatePerMillis {
            token_resource_per_ms: Resource {
                req_counter: 1,
                space_in_bytes: 1,
                execution_time_micros: 1,
                gas_used: Gas::from([0, 0]),
            },
        };

        let config = RateLimiterConfig::<TestSpec> {
            max_allowed_resources,
            refill_rate,
        };

        let addr1 = <TestSpec as Spec>::Address::from([1; 28]);
        let addr2 = <TestSpec as Spec>::Address::from([2; 28]);
        let addr3 = <TestSpec as Spec>::Address::from([3; 28]);
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        let mut rate_limiter = SovRateLimiter::new_for_test(1_000_000, Some(config));

        // Update rate limiter for (ip1, cred1)
        {
            let token = rate_limiter.allow(ip1, addr1).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }

        // Update rate limiter for (ip1, cred2)
        {
            let token = rate_limiter.allow(ip1, addr2).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }

        // Update rate limiter for (ip1, cred3)
        // Different addresses were sent from the same IP, and after two requests we exceeded the limit for that IP.
        {
            let err = rate_limiter.allow(ip1, addr3).unwrap_err();
            assert!(matches!(err, ResourceLimitExceededError::Ip { .. }));
        }

        // Update the rate limiter for (ip2, cred3); this works because we're using a new IP.
        {
            let token = rate_limiter.allow(ip2, addr3).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }
    }

    impl<S: Spec> SovRateLimiter<S> {
        fn new_for_test(ttl_in_millis: u64, config: Option<RateLimiterConfig<S>>) -> Self {
            let inner = config.map(|c| {
                SovRateLimiterInner::new(ttl_in_millis, c, Default::default(), Default::default())
            });
            Self { inner }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IpAndCredentialId<Address: BasicAddress> {
    pub default_address: Address,
    pub credential_id: CredentialId,
    pub ip_addr: IpAddr,
}
