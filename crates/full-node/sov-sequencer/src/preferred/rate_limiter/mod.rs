#![allow(dead_code)]
mod limiter;
mod resource;

pub(crate) use limiter::ResourceUsed;
use limiter::*;
use resource::*;
use sov_full_node_configs::sequencer::SovRateLimiterConfig;
use sov_modules_api::{CredentialId, Spec};
use std::hash::Hash;
use std::{collections::HashMap, net::IpAddr, time::Instant};

use crate::preferred::sync_sequencer_state::comfortable_gas_limit;

#[derive(Debug)]
pub(crate) struct Token<S: Spec> {
    credential_id: CredentialId,
    throttler_for_credential: Throttler<S::Gas>,

    ip: IpAddr,
    throttler_for_ip: Throttler<S::Gas>,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum ResourceLimitExceededError<S: Spec> {
    #[error("Resource limit exceeded for Credential_ID: {credential_id:?}, {reason:?}")]
    CredentialId {
        credential_id: CredentialId,
        reason: LimitExceeded<S::Gas>,
    },
    #[error("Resource limit exceeded for IP: {ip:?}, {reason:?}")]
    Ip {
        ip: IpAddr,
        reason: LimitExceeded<S::Gas>,
    },
}

#[derive(Default)]
pub struct SpecialKeys {
    pub ips: Vec<(IpAddr, SovRateLimiterConfig)>,
    pub credentials: Vec<(CredentialId, SovRateLimiterConfig)>,
}

struct SovRateLimiterInner<S: Spec> {
    by_credential_rate_limiter: RateLimiter<CredentialId, S>,
    by_ip_rate_limiter: RateLimiter<IpAddr, S>,
}

impl<S: Spec> SovRateLimiterInner<S> {
    fn new(
        config: RateLimiterConfig<S>,
        credentials: HashMap<CredentialId, RateLimiterConfig<S>>,
        ips: HashMap<IpAddr, RateLimiterConfig<S>>,
    ) -> Self {
        Self {
            by_credential_rate_limiter: RateLimiter::new(config.clone()),
            by_ip_rate_limiter: RateLimiter::new(config),
        }
    }

    fn allow(
        &mut self,
        ip: IpAddr,
        credential_id: CredentialId,
    ) -> Result<Token<S>, ResourceLimitExceededError<S>> {
        let now = Instant::now();
        let throttler_for_credential =
            match self.by_credential_rate_limiter.allow(now, &credential_id) {
                Ok(ok) => ok,
                Err(reason) => {
                    return Err(ResourceLimitExceededError::CredentialId {
                        credential_id,
                        reason,
                    })
                }
            };

        let throttler_for_ip = self
            .by_ip_rate_limiter
            .allow(now, &ip)
            .map_err(|reason| ResourceLimitExceededError::Ip { ip, reason })?;

        Ok(Token {
            credential_id,
            throttler_for_credential,
            ip,
            throttler_for_ip,
        })
    }

    fn update(&mut self, token: Token<S>, resource_used: ResourceUsed<S::Gas>) {
        self.by_credential_rate_limiter.update(
            token.credential_id,
            token.throttler_for_credential,
            resource_used,
        );
        self.by_ip_rate_limiter
            .update(token.ip, token.throttler_for_ip, resource_used);
    }
}

const TTL_MULTIPLIER: u64 = 20;

// A single user is limited to 1/PER_KEY = 0.5% of resources of a single batch.
const PER_KEY: u64 = 200;

fn clculate_limits<S: Spec>(
    sov_config: SovRateLimiterConfig,
    batch_execution_time_limit_millis: u64,
    max_batch_size_bytes: usize,
) -> RateLimiterConfig<S> {
    // All entries older than this value are evicted from the rate limiter.
    let ttl_in_millis = batch_execution_time_limit_millis * TTL_MULTIPLIER;
    let max_gas = comfortable_gas_limit::<S>();

    let max_resources_per_batch = Resource {
        req_counter: sov_config.max_requests_per_batch,
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

    // A single sender is limited to 1/PER_KEY = 0.5% of resources of a single batch.
    let max_per_key = max_resources_per_batch.div_by_scalar(PER_KEY);

    // The refill rate is defined as 0.1% of max_per_key. After one second, the system refills max_per_key tokens.
    let refill_rate = max_per_key
        .saturating_mul_by_scalar(sov_config.refill_rate)
        .div_by_scalar(1000);

    RateLimiterConfig {
        ttl_in_millis,
        max_allowed_resources: TotalResources { inner: max_per_key },
        refill_rate: RefillRatePerMillis {
            token_resource_per_ms: refill_rate,
        },
    }
}

fn to_limiter_config_map<K: Eq + Hash, S: Spec>(
    batch_execution_time_limit_millis: u64,
    max_batch_size_bytes: usize,
    v: Vec<(K, SovRateLimiterConfig)>,
) -> HashMap<K, RateLimiterConfig<S>> {
    v.into_iter()
        .map(|x| {
            (
                x.0,
                clculate_limits::<S>(x.1, batch_execution_time_limit_millis, max_batch_size_bytes),
            )
        })
        .collect()
}

fn limits<S: Spec>(
    sov_config: SovRateLimiterConfig,
    batch_execution_time_limit_millis: u64,
    max_batch_size_bytes: usize,
    special_keys: SpecialKeys,
) -> (
    RateLimiterConfig<S>,
    HashMap<CredentialId, RateLimiterConfig<S>>,
    HashMap<IpAddr, RateLimiterConfig<S>>,
) {
    let default_config = clculate_limits::<S>(
        sov_config,
        batch_execution_time_limit_millis,
        max_batch_size_bytes,
    );

    let credentials = to_limiter_config_map::<CredentialId, S>(
        batch_execution_time_limit_millis,
        max_batch_size_bytes,
        special_keys.credentials,
    );

    let ips = to_limiter_config_map::<IpAddr, S>(
        batch_execution_time_limit_millis,
        max_batch_size_bytes,
        special_keys.ips,
    );

    (default_config, credentials, ips)
}

pub(crate) struct SovRateLimiter<S: Spec> {
    /// If [`SovRateLimiter`] is initialized with a None config, this field is None.
    /// Not providing a config is the mechanism for disabling the rollup’s rate limiter.
    inner: Option<SovRateLimiterInner<S>>,
}

impl<S: Spec> SovRateLimiter<S> {
    pub(crate) fn new(
        config: Option<SovRateLimiterConfig>,
        batch_execution_time_limit_millis: u64,
        max_batch_size_bytes: usize,
        special_keys: SpecialKeys,
    ) -> Self {
        let inner = config.map(|sov_config| {
            let (config, credentials, ips) = limits(
                sov_config,
                batch_execution_time_limit_millis,
                max_batch_size_bytes,
                special_keys,
            );
            SovRateLimiterInner::new(config, credentials, ips)
        });
        Self { inner }
    }

    pub(crate) fn allow(
        &mut self,
        ip: IpAddr,
        credential_id: CredentialId,
    ) -> Result<Option<Token<S>>, ResourceLimitExceededError<S>> {
        self.inner
            .as_mut()
            .map(|inner| inner.allow(ip, credential_id))
            .transpose()
    }

    pub(crate) fn update(&mut self, token: Option<Token<S>>, resource_used: ResourceUsed<S::Gas>) {
        if let Some(token) = token {
            let inner = self.inner.as_mut().expect("The impossible happened: SovRateLimiter is unavailable even though a Some(Token) was provided.");
            inner.update(token, resource_used);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sov_test_utils::TestSpec;
    use std::net::Ipv4Addr;

    type Gas = <TestSpec as Spec>::Gas;

    #[test]
    fn test_clculate_limits() {
        let sov_config = SovRateLimiterConfig {
            max_requests_per_batch: 234000000,
            refill_rate: 1,
        };

        let rate_limiter_config = clculate_limits::<TestSpec>(sov_config, 1_000_000_000, 1000000);

        let max_allowed_resources_per_key = rate_limiter_config.max_allowed_resources;

        // In one ms we refill max_allowed_resources_per_key tokens.
        let refilled = rate_limiter_config
            .refill_rate
            .token_resource_per_ms
            .saturating_mul_by_scalar(1000);

        assert_eq!(max_allowed_resources_per_key.inner, refilled);
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
            ttl_in_millis: 1_000_000,
            max_allowed_resources,
            refill_rate,
        };

        let cred1 = CredentialId::from_bytes([1; 32]);
        let cred2 = CredentialId::from_bytes([2; 32]);
        let cred3 = CredentialId::from_bytes([3; 32]);
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        let mut rate_limiter = SovRateLimiter::new_for_test(Some(config));

        // Update rate limiter for (ip1, cred1)
        {
            let token = rate_limiter.allow(ip1, cred1).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }

        // Update rate limiter for (ip1, cred2)
        {
            let token = rate_limiter.allow(ip1, cred2).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }

        // Update rate limiter for (ip1, cred3)
        // Different addresses were sent from the same IP, and after two requests we exceeded the limit for that IP.
        {
            let err = rate_limiter.allow(ip1, cred3).unwrap_err();
            assert!(matches!(err, ResourceLimitExceededError::Ip { .. }));
        }

        // Update the rate limiter for (ip2, cred3); this works because we're using a new IP.
        {
            let token = rate_limiter.allow(ip2, cred3).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }
    }

    impl<S: Spec> SovRateLimiter<S> {
        fn new_for_test(config: Option<RateLimiterConfig<S>>) -> Self {
            let inner = config.map(|c| SovRateLimiterInner::new(c));
            Self { inner }
        }
    }
}
