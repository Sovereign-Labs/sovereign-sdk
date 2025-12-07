#![allow(dead_code)]
mod limiter;
mod resource;

use limiter::*;
use resource::*;
use sov_modules_api::Spec;
use std::{net::IpAddr, time::Instant};

#[derive(Debug)]
pub(crate) struct Token<S: Spec> {
    addr: S::Address,
    throttler_for_addr: Throttler<S::Gas>,

    ip: IpAddr,
    throttler_for_ip: Throttler<S::Gas>,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum ResourceLimitExceededError<S: Spec> {
    #[error("Resource limit exceeded for Address:{addr:?}, {reason:?}")]
    Address {
        addr: S::Address,
        reason: LimitExceeded<S::Gas>,
    },
    #[error("Resource limit exceeded for IP: {ip:?}, {reason:?}")]
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
    fn new(config: RateLimiterConfig<S>) -> Self {
        Self {
            by_addr_rate_limiter: RateLimiter::new(config.clone()),
            by_ip_rate_limiter: RateLimiter::new(config),
        }
    }

    fn allow(
        &mut self,
        ip: IpAddr,
        addr: S::Address,
    ) -> Result<Token<S>, ResourceLimitExceededError<S>> {
        let now = Instant::now();
        let throttler_for_addr = match self.by_addr_rate_limiter.allow(now, &addr) {
            Ok(ok) => ok,
            Err(reason) => return Err(ResourceLimitExceededError::Address { addr, reason }),
        };

        let throttler_for_ip = self
            .by_ip_rate_limiter
            .allow(now, &ip)
            .map_err(|reason| ResourceLimitExceededError::Ip { ip, reason })?;

        Ok(Token {
            addr,
            throttler_for_addr,
            ip,
            throttler_for_ip,
        })
    }

    fn update(&mut self, token: Token<S>, resource_used: ResourceUsed<S::Gas>) {
        self.by_addr_rate_limiter
            .update(token.addr, token.throttler_for_addr, resource_used);
        self.by_ip_rate_limiter
            .update(token.ip, token.throttler_for_ip, resource_used);
    }
}

pub(crate) struct SovRateLimiter<S: Spec> {
    /// If [`SovRateLimiter`] is initialized with a None config, this field is None.
    /// Not providing a config is the mechanism for disabling the rollup’s rate limiter.
    inner: Option<SovRateLimiterInner<S>>,
}

impl<S: Spec> SovRateLimiter<S> {
    pub(crate) fn new(config: Option<RateLimiterConfig<S>>) -> Self {
        let inner = config.map(|c| SovRateLimiterInner::new(c));
        Self { inner }
    }

    pub(crate) fn allow(
        &mut self,
        ip: IpAddr,
        addr: S::Address,
    ) -> Result<Option<Token<S>>, ResourceLimitExceededError<S>> {
        self.inner
            .as_mut()
            .map(|inner| inner.allow(ip, addr))
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
            ttl_in_milis: 1_000_000,
            max_allowed_resources,
            refill_rate,
        };

        let addr1 = <TestSpec as Spec>::Address::from([1; 28]);
        let addr2 = <TestSpec as Spec>::Address::from([2; 28]);
        let addr3 = <TestSpec as Spec>::Address::from([3; 28]);
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        let mut rate_limiter = SovRateLimiter::new(Some(config));

        // Update rate limiter for (ip1, addr1)
        {
            let token = rate_limiter.allow(ip1, addr1).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }

        // Update rate limiter for (ip1, addr2)
        {
            let token = rate_limiter.allow(ip1, addr2).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }

        // Update rate limiter for (ip1, addr3)
        // Different addresses were sent from the same IP, and after two requests we exceeded the limit for that IP.
        {
            let err = rate_limiter.allow(ip1, addr3).unwrap_err();
            assert!(matches!(err, ResourceLimitExceededError::Ip { .. }));
        }

        // Update the rate limiter for (ip2, addr3); this works because we're using a new IP.
        {
            let token = rate_limiter.allow(ip2, addr3).unwrap();
            rate_limiter.update(token, resource_used_per_run);
        }
    }
}
