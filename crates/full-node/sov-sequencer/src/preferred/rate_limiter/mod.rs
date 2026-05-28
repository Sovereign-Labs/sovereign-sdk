#![allow(dead_code)]
mod limiter;
mod resource;

use crate::preferred::sync_sequencer_state::comfortable_gas_limit_for_height;
pub(crate) use limiter::ResourceUsed;
use limiter::*;
use resource::*;
use sov_full_node_configs::sequencer::{Limits, SovRateLimiterConfig};
use sov_modules_api::BasicAddress;
use sov_modules_api::{CredentialId, Spec};
use sov_rollup_interface::common::RollupHeight;
use std::collections::HashMap;
use std::hash::Hash;
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

const TTL_MULTIPLIER: u32 = 5;

#[derive(Debug)]
pub(crate) struct LimiterToken<S: Spec> {
    address: S::Address,
    throttler_for_addr: Throttler<S::Gas>,

    ip_net: ipnet::IpNet,
    throttler_for_ip: Throttler<S::Gas>,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum ResourceLimitExceededError<S: Spec> {
    #[error("Resource limit exceeded for address: {address}, {reason:?}")]
    Address {
        address: S::Address,
        reason: LimitExceeded<S::Gas>,
    },
    #[error("Resource limit exceeded for IP: {ip}{}, {reason:?}", subnet_suffix(.ip_net))]
    Ip {
        ip: IpAddr,
        /// Bucket the IP resolved to: its own host network, or the configured
        /// subnet that contains it.
        ip_net: ipnet::IpNet,
        reason: LimitExceeded<S::Gas>,
    },
}

/// Renders the matched subnet for [`ResourceLimitExceededError::Ip`]: empty for a
/// host network (`/32` or `/128`, where `{ip}` already says everything) and
/// ` (subnet <net>)` when the IP was limited via a wider configured subnet.
fn subnet_suffix(ip_net: &ipnet::IpNet) -> String {
    if ip_net.prefix_len() == ip_net.max_prefix_len() {
        String::new()
    } else {
        format!(" (subnet {ip_net})")
    }
}

struct SovRateLimiterInner<S: Spec> {
    by_addr_rate_limiter: RateLimiter<S::Address, S>,
    by_ip_net_rate_limiter: RateLimiter<ipnet::IpNet, S>,
    /// Prefix lengths of the configured IP subnets, used to resolve an incoming
    /// IP to its bucket key by probing only those lengths.
    ip_subnet_prefixes: SubnetPrefixes,
}

impl<S: Spec> SovRateLimiterInner<S> {
    fn new(
        max_nb_of_concurrent_users_in_rate_limiter: u64,
        ttl: Duration,
        config: RateLimiterConfig<S>,
        addrs: HashMap<S::Address, RateLimiterConfig<S>>,
        ip_networks: HashMap<ipnet::IpNet, RateLimiterConfig<S>>,
    ) -> Self {
        // Computed before `ip_networks` is moved into the limiter below.
        let ip_subnet_prefixes = SubnetPrefixes::from_networks(ip_networks.keys());
        Self {
            by_addr_rate_limiter: RateLimiter::new(
                "limiter_by_addr",
                max_nb_of_concurrent_users_in_rate_limiter,
                ttl,
                config.clone(),
                addrs,
            ),
            by_ip_net_rate_limiter: RateLimiter::new(
                "limiter_by_ip_net",
                max_nb_of_concurrent_users_in_rate_limiter,
                ttl,
                config,
                ip_networks,
            ),
            ip_subnet_prefixes,
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

        // Overlapping subnets are rejected at startup, so an IP falls into at most
        // one configured subnet; probe the configured prefix lengths (longest
        // first) and fall back to the host network if none match.
        let ip_key = self
            .ip_subnet_prefixes
            .candidate_supernets(ip)
            .find(|ip_net| self.by_ip_net_rate_limiter.contains_special_config(ip_net))
            .unwrap_or_else(|| ip.into());
        let throttler_for_ip =
            self.by_ip_net_rate_limiter
                .allow(now, &ip_key)
                .map_err(|reason| ResourceLimitExceededError::Ip {
                    ip,
                    ip_net: ip_key,
                    reason,
                })?;

        Ok(LimiterToken {
            address,
            throttler_for_addr,
            ip_net: ip_key,
            throttler_for_ip,
        })
    }

    fn update(&mut self, token: LimiterToken<S>, resource_used: ResourceUsed<S::Gas>) {
        self.by_addr_rate_limiter
            .update(token.address, token.throttler_for_addr, resource_used);
        self.by_ip_net_rate_limiter
            .update(token.ip_net, token.throttler_for_ip, resource_used);
    }
}

fn calculate_limits<S: Spec>(
    limits: Limits,
    max_requests_per_second: u64,
    batch_execution_time_limit: Duration,
    max_batch_size_bytes: usize,
    height_for_gas_limit_computation: RollupHeight,
) -> RateLimiterConfig<S> {
    let max_gas = comfortable_gas_limit_for_height::<S>(height_for_gas_limit_computation);
    let batch_execution_time_limit_millis = batch_execution_time_limit
        .as_millis()
        .try_into()
        .expect("Batch execution time limit overflows u64 milliseconds");
    let batch_execution_time_limit_micros = batch_execution_time_limit
        .as_micros()
        .try_into()
        .expect("Batch execution time limit overflows u64 microseconds");

    let max_resources_per_batch = Resource {
        // requests-per-batch = rate (per second) × batch duration (seconds).
        // Multiply by milliseconds first, then divide by 1000, to keep sub-second precision.
        req_counter: max_requests_per_second
            .checked_mul(batch_execution_time_limit_millis)
            .expect("Overflow converting max_requests_per_second to max requests per batch")
            / 1000,
        // The `expect` is justified because we will never have batches larger than u64::MAX bytes.
        space_in_bytes: max_batch_size_bytes
            .try_into()
            .expect("Allowed batch size overflows u64::MAX"),
        execution_time_micros: batch_execution_time_limit_micros,
        gas_used: max_gas,
    };

    // A single sender is limited to 1/max_threshold_per_key_to_batch_capacity_ratio of resources of a single batch.
    let mut max_per_key = max_resources_per_batch
        .saturating_mul_by_scalar(limits.resources_per_bucket)
        .div_by_scalar(1000);
    // A non-zero rate must not silently round down to a zero request budget (which
    // would reject all traffic). `resources_per_bucket == 0` and
    // `max_requests_per_second == 0` are intentional "block everything" sentinels,
    // so leave those untouched.
    if limits.resources_per_bucket > 0 && max_requests_per_second > 0 {
        max_per_key.req_counter = max_per_key.req_counter.max(1);
    }

    // Refill per millisecond: over one batch the bucket refills `refill_rate`
    // times its capacity (`max_per_key * limits.refill_rate`), spread across
    // `batch_execution_time_limit_millis`.
    //
    // This per-ms rate is an integer. When
    // `max_per_key.req_counter * limits.refill_rate < batch_execution_time_limit_millis`
    // the request refill truncates to 0, so the request dimension acts as a hard
    // cap of `max_per_key.req_counter` that resets only when the throttler is
    // evicted (~`ttl_in_millis` after the key's last successful request) rather
    // than refilling smoothly. This surfaces only under floods of unusually cheap
    // requests; otherwise the size/execution-time limits bind first. Smooth
    // sub-token request refill is a possible follow-up.
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
    batch_execution_time_limit: Duration,
    max_batch_size_bytes: usize,
    height_for_gas_limit_computation: RollupHeight,
    v: Vec<(K, Limits)>,
) -> HashMap<K, RateLimiterConfig<S>> {
    v.into_iter()
        .map(|(key, limits)| {
            (
                key,
                calculate_limits::<S>(
                    limits,
                    max_requests_per_second,
                    batch_execution_time_limit,
                    max_batch_size_bytes,
                    height_for_gas_limit_computation,
                ),
            )
        })
        .collect()
}

fn canonicalize_ip_net(net: ipnet::IpNet) -> ipnet::IpNet {
    match net {
        ipnet::IpNet::V4(_) => net.trunc(),
        ipnet::IpNet::V6(net) => {
            if net.prefix_len() >= 96 {
                if let Some(ipv4) = net.addr().to_ipv4_mapped() {
                    return ipnet::IpNet::new(IpAddr::V4(ipv4), net.prefix_len() - 96)
                        .expect("IPv4-mapped IPv6 prefix is a valid IPv4 prefix")
                        .trunc();
                }
            }

            ipnet::IpNet::V6(net.trunc())
        }
    }
}

#[allow(clippy::type_complexity)]
fn limits<S: Spec>(
    sov_config: SovRateLimiterConfig<S::Address>,
    batch_execution_time_limit: Duration,
    max_batch_size_bytes: usize,
) -> (
    RateLimiterConfig<S>,
    HashMap<S::Address, RateLimiterConfig<S>>,
    HashMap<ipnet::IpNet, RateLimiterConfig<S>>,
) {
    let default_config = calculate_limits::<S>(
        sov_config.default_limits,
        sov_config.max_requests_per_second,
        batch_execution_time_limit,
        max_batch_size_bytes,
        sov_config.height_for_gas_limit_computation,
    );

    let addrs = to_limiter_config_map::<S::Address, S>(
        sov_config.max_requests_per_second,
        batch_execution_time_limit,
        max_batch_size_bytes,
        sov_config.height_for_gas_limit_computation,
        sov_config.address_custom_limits,
    );

    let ip_custom_limits = sov_config
        .ip_custom_limits
        .into_iter()
        .map(|(net, limits)| (canonicalize_ip_net(net), limits))
        .collect();

    let ip_networks = to_limiter_config_map::<ipnet::IpNet, S>(
        sov_config.max_requests_per_second,
        batch_execution_time_limit,
        max_batch_size_bytes,
        sov_config.height_for_gas_limit_computation,
        ip_custom_limits,
    );

    (default_config, addrs, ip_networks)
}

/// Distinct prefix lengths of the configured IP rate-limiter networks, split by
/// address family and kept **descending** (longest-prefix first). Built once at
/// startup. When a family has no configured networks, an IP of that family
/// probes nothing and resolves straight to its own host network. A configured
/// bare IP counts as a host network (`/32` or `/128`) and contributes its prefix
/// length here too, so once any entry of a family exists, every IP of that
/// family performs at least one map probe.
///
/// The invariants — per-family, deduplicated, descending, and in range for the
/// family (`v4 <= 32`, `v6 <= 128`) — are established by [`Self::from_networks`]
/// from already-validated [`ipnet::IpNet`] keys and never change afterwards.
#[derive(Debug)]
struct SubnetPrefixes {
    v4: Box<[u8]>,
    v6: Box<[u8]>,
}

impl SubnetPrefixes {
    fn from_networks<'a>(nets: impl Iterator<Item = &'a ipnet::IpNet>) -> Self {
        let mut v4 = Vec::new();
        let mut v6 = Vec::new();
        for net in nets {
            match net {
                ipnet::IpNet::V4(_) => v4.push(net.prefix_len()),
                ipnet::IpNet::V6(_) => v6.push(net.prefix_len()),
            }
        }
        // Distinct and descending (longest-prefix-match order); built once at startup.
        for prefixes in [&mut v4, &mut v6] {
            prefixes.sort_unstable();
            prefixes.dedup();
            prefixes.reverse();
        }
        Self {
            v4: v4.into(),
            v6: v6.into(),
        }
    }

    /// The supernets of `ip` worth probing, longest-prefix first: only the
    /// prefix lengths actually present in the config for `ip`'s address family.
    /// Empty when none are configured.
    fn candidate_supernets(&self, ip: IpAddr) -> impl Iterator<Item = ipnet::IpNet> + '_ {
        let prefixes = match ip {
            IpAddr::V4(_) => &self.v4,
            IpAddr::V6(_) => &self.v6,
        };
        prefixes.iter().map(move |&prefix| {
            // `prefix` came from a same-family config network, so it is in range.
            ipnet::IpNet::new(ip, prefix)
                .expect("config prefix length is valid for this address family")
                .trunc()
        })
    }
}

/// Panics if any two configured subnets overlap (one contains the other).
/// Overlapping subnets are forbidden: an IP inside both would have no
/// unambiguous bucket.
fn assert_no_overlapping_subnets(networks: &[ipnet::IpNet]) {
    for (i, a) in networks.iter().enumerate() {
        for b in &networks[i + 1..] {
            assert!(
                !(a.contains(b) || b.contains(a)),
                "rate limiter config contains overlapping IP subnets: {a} and {b}"
            );
        }
    }
}

impl<S: Spec> SovRateLimiter<S> {
    /// Builds the rate limiter from `config`; a `None` config disables rate
    /// limiting entirely.
    ///
    /// # Panics
    ///
    /// Panics on invalid configuration so operator mistakes surface loudly at
    /// startup rather than silently mis-limiting traffic:
    /// - `batch_execution_time_limit_millis` or `max_batch_size_bytes` is zero;
    /// - any two configured `ip_custom_limits` subnets overlap or duplicate one
    ///   another (an IP inside both would have no unambiguous bucket).
    pub(crate) fn new(
        config: Option<SovRateLimiterConfig<S::Address>>,
        batch_execution_time_limit: Duration,
        max_batch_size_bytes: usize,
    ) -> Self {
        if batch_execution_time_limit.as_millis() == 0 {
            panic!("SovRateLimiter: batch_execution_time_limit_millis must be greater than zero");
        }

        if max_batch_size_bytes == 0 {
            panic!("SovRateLimiter: max_batch_size_bytes must be greater than zero");
        }

        let inner = config.map(|sov_config| {
            // All entries older than this value are evicted from the rate limiter.
            let ttl = batch_execution_time_limit
                .checked_mul(TTL_MULTIPLIER)
                .expect("Batch execution time limit overflow when TTL multiplier applied");
            let max_nb_of_concurrent_users_in_rate_limiter =
                sov_config.max_nb_of_concurrent_users_in_rate_limiter;

            // Overlapping (and duplicate) subnets are forbidden. Check the
            // canonicalized config list before `limits` collapses duplicate
            // keys into a map.
            let configured_networks: Vec<ipnet::IpNet> = sov_config
                .ip_custom_limits
                .iter()
                .map(|(net, _)| canonicalize_ip_net(*net))
                .collect();
            assert_no_overlapping_subnets(&configured_networks);

            let (config, addrs, ips) =
                limits(sov_config, batch_execution_time_limit, max_batch_size_bytes);
            SovRateLimiterInner::new(
                max_nb_of_concurrent_users_in_rate_limiter,
                ttl,
                config,
                addrs,
                ips,
            )
        });
        Self { inner }
    }

    pub(crate) fn allow(
        &mut self,
        ip: IpAddr,
        address: S::Address,
    ) -> Result<Option<LimiterToken<S>>, ResourceLimitExceededError<S>> {
        // Canonicalize IPv4-mapped IPv6 (`::ffff:a.b.c.d`) to IPv4 so a client
        // can't dodge a configured IPv4 subnet limit by sending the mapped form.
        let ip = ip.to_canonical();
        let res = self
            .inner
            .as_mut()
            .map(|inner| inner.allow(ip, address))
            .transpose();

        if let Err(err) = &res {
            tracing::debug!(ip = %ip, address = %address, %err, "Transaction not allowed.");
        }

        res
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

#[derive(Clone, Copy, Debug)]
pub struct IpAndCredentialId<Address: BasicAddress> {
    pub address: Address,
    pub credential_id: CredentialId,
    pub ip_addr: IpAddr,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Limits;
    use sov_test_utils::TestSpec;
    use std::collections::HashMap;
    use std::net::Ipv4Addr;

    type Gas = <TestSpec as Spec>::Gas;

    fn test_addr(byte: u8) -> <TestSpec as Spec>::Address {
        <TestSpec as Spec>::Address::from([byte; 28])
    }

    fn req_count_config(max_req_count: u64) -> RateLimiterConfig<TestSpec> {
        let mut max_allowed_resources = Resource::max();
        max_allowed_resources.req_counter = max_req_count;

        RateLimiterConfig::<TestSpec> {
            max_allowed_resources: TotalResources {
                inner: max_allowed_resources,
            },
            refill_rate: RefillRatePerMillis {
                token_resource_per_ms: Resource::zero(),
            },
        }
    }

    fn one_request() -> ResourceUsed<Gas> {
        ResourceUsed {
            inner: Resource {
                req_counter: 1,
                ..Resource::zero()
            },
        }
    }

    fn rate_limiter_with_ip_networks(
        default_config: RateLimiterConfig<TestSpec>,
        ip_networks: &[&str],
    ) -> SovRateLimiter<TestSpec> {
        let ip_networks: HashMap<ipnet::IpNet, RateLimiterConfig<TestSpec>> = ip_networks
            .iter()
            .map(|ip_net| (ip_net.parse().unwrap(), default_config.clone()))
            .collect();

        SovRateLimiter::new_for_test_with_ip_networks(
            1000,
            Duration::from_millis(1_000_000),
            default_config,
            ip_networks,
        )
    }

    #[test]
    fn test_calculate_limits() {
        let limits = Limits {
            resources_per_bucket: 5,
            refill_rate: 1,
        };

        let max_batch_exec_time = 6000;
        let rate_limiter_config = calculate_limits::<TestSpec>(
            limits,
            10000,
            Duration::from_millis(max_batch_exec_time),
            6000000,
            RollupHeight::GENESIS,
        );

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
    fn calculate_limits_scales_req_counter_to_requests_per_batch() {
        // 1000 req/s over a 6 s batch = 6000 requests/batch; resources_per_bucket = 10
        // (1% of a batch) => 60 requests per key.
        let config = calculate_limits::<TestSpec>(
            Limits {
                resources_per_bucket: 10,
                refill_rate: 0,
            },
            1000,
            Duration::from_millis(6000),
            6_000_000,
            RollupHeight::GENESIS,
        );
        assert_eq!(config.max_allowed_resources.inner.req_counter, 60);
    }

    #[test]
    fn calculate_limits_floors_req_counter_for_subsecond_batch() {
        // 1000 req/s over a 100 ms batch = 100 requests/batch; resources_per_bucket = 5
        // would round the per-key budget to 0 (100 * 5 / 1000). The guard floors it at 1.
        let config = calculate_limits::<TestSpec>(
            Limits {
                resources_per_bucket: 5,
                refill_rate: 0,
            },
            1000,
            Duration::from_millis(100),
            6_000_000,
            RollupHeight::GENESIS,
        );
        assert_eq!(config.max_allowed_resources.inner.req_counter, 1);
    }

    #[test]
    fn calculate_limits_keeps_zero_budget_for_zero_resources_per_bucket() {
        // `resources_per_bucket == 0` is the intentional "block everything" sentinel:
        // the guard must not floor it up to 1.
        let config = calculate_limits::<TestSpec>(
            Limits {
                resources_per_bucket: 0,
                refill_rate: 0,
            },
            1000,
            Duration::from_millis(100),
            6_000_000,
            RollupHeight::GENESIS,
        );
        assert_eq!(config.max_allowed_resources.inner.req_counter, 0);
    }

    #[test]
    fn calculate_limits_keeps_zero_budget_for_zero_max_requests_per_second() {
        // `max_requests_per_second == 0` is the intentional "block everything" sentinel:
        // the guard must not floor it up to 1, even when `resources_per_bucket > 0`.
        let config = calculate_limits::<TestSpec>(
            Limits {
                resources_per_bucket: 5,
                refill_rate: 0,
            },
            0,
            Duration::from_millis(100),
            6_000_000,
            RollupHeight::GENESIS,
        );
        assert_eq!(config.max_allowed_resources.inner.req_counter, 0);
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

        let mut rate_limiter =
            SovRateLimiter::new_for_test(1000, Duration::from_millis(1_000_000), Some(config));

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
        fn new_for_test(
            max_nb_of_concurrent_users: u64,
            ttl: Duration,
            config: Option<RateLimiterConfig<S>>,
        ) -> Self {
            let inner = config.map(|c| {
                SovRateLimiterInner::new(
                    max_nb_of_concurrent_users,
                    ttl,
                    c,
                    Default::default(),
                    Default::default(),
                )
            });
            Self { inner }
        }

        fn new_for_test_with_ip_networks(
            max_nb_of_concurrent_users: u64,
            ttl: Duration,
            config: RateLimiterConfig<S>,
            ip_networks: HashMap<ipnet::IpNet, RateLimiterConfig<S>>,
        ) -> Self {
            Self {
                inner: Some(SovRateLimiterInner::new(
                    max_nb_of_concurrent_users,
                    ttl,
                    config,
                    Default::default(),
                    ip_networks,
                )),
            }
        }
    }

    #[test]
    fn allow_falls_back_to_host_bucket_with_subnets_configured() {
        let mut rate_limiter = rate_limiter_with_ip_networks(req_count_config(2), &["10.0.0.0/24"]);
        let resource_used = one_request();
        let fallback_ip: IpAddr = "192.0.2.10".parse().unwrap();

        for byte in [1, 2] {
            let token = rate_limiter.allow(fallback_ip, test_addr(byte)).unwrap();
            rate_limiter.update(token, resource_used);
        }

        let err = rate_limiter.allow(fallback_ip, test_addr(3)).unwrap_err();
        let ResourceLimitExceededError::Ip { ip_net, .. } = err else {
            panic!("expected IP rate-limit error");
        };
        assert_eq!(ip_net, "192.0.2.10/32".parse().unwrap());

        let token = rate_limiter
            .allow("192.0.2.11".parse().unwrap(), test_addr(4))
            .unwrap()
            .unwrap();
        assert_eq!(token.ip_net, "192.0.2.11/32".parse().unwrap());

        let token = rate_limiter
            .allow("10.0.0.7".parse().unwrap(), test_addr(5))
            .unwrap()
            .unwrap();
        assert_eq!(token.ip_net, "10.0.0.0/24".parse().unwrap());
    }

    #[test]
    fn allow_matches_configured_subnet_after_longer_prefix_miss() {
        let mut rate_limiter =
            rate_limiter_with_ip_networks(req_count_config(2), &["10.0.0.0/24", "172.16.0.0/16"]);

        let token = rate_limiter
            .allow("172.16.2.3".parse().unwrap(), test_addr(1))
            .unwrap()
            .unwrap();
        assert_eq!(token.ip_net, "172.16.0.0/16".parse().unwrap());

        let token = rate_limiter
            .allow("10.0.0.42".parse().unwrap(), test_addr(2))
            .unwrap()
            .unwrap();
        assert_eq!(token.ip_net, "10.0.0.0/24".parse().unwrap());
    }

    #[test]
    fn subnet_suffix_renders_only_wider_subnets() {
        assert_eq!(subnet_suffix(&"10.0.0.1/32".parse().unwrap()), "");
        assert_eq!(subnet_suffix(&"2001:db8::1/128".parse().unwrap()), "");
        assert_eq!(
            subnet_suffix(&"10.0.0.0/24".parse().unwrap()),
            " (subnet 10.0.0.0/24)"
        );
        assert_eq!(
            subnet_suffix(&"2001:db8::/64".parse().unwrap()),
            " (subnet 2001:db8::/64)"
        );
    }

    #[test]
    #[should_panic(expected = "overlapping IP subnets")]
    fn overlapping_subnet_config_panics() {
        let config = SovRateLimiterConfig {
            max_requests_per_second: 1000,
            max_nb_of_concurrent_users_in_rate_limiter: 1000,
            default_limits: Limits {
                resources_per_bucket: 5,
                refill_rate: 1,
            },
            height_for_gas_limit_computation: RollupHeight::GENESIS,
            address_custom_limits: Vec::default(),
            ip_custom_limits: vec![
                (
                    "10.0.0.0/8".parse().unwrap(),
                    Limits {
                        resources_per_bucket: 5,
                        refill_rate: 1,
                    },
                ),
                (
                    "10.0.0.0/24".parse().unwrap(),
                    Limits {
                        resources_per_bucket: 5,
                        refill_rate: 1,
                    },
                ),
            ],
        };
        let _ =
            SovRateLimiter::<TestSpec>::new(Some(config), Duration::from_millis(3000), 1_000_000);
    }

    #[test]
    #[should_panic(expected = "overlapping IP subnets")]
    fn overlapping_ipv4_mapped_ipv6_subnet_config_panics() {
        let limits = Limits {
            resources_per_bucket: 5,
            refill_rate: 1,
        };
        let config = SovRateLimiterConfig {
            max_requests_per_second: 1000,
            max_nb_of_concurrent_users_in_rate_limiter: 1000,
            default_limits: limits,
            height_for_gas_limit_computation: RollupHeight::GENESIS,
            address_custom_limits: Vec::default(),
            ip_custom_limits: vec![
                ("10.0.0.0/24".parse().unwrap(), limits),
                ("::ffff:10.0.0.0/120".parse().unwrap(), limits),
            ],
        };
        let _ =
            SovRateLimiter::<TestSpec>::new(Some(config), Duration::from_millis(3000), 1_000_000);
    }

    #[test]
    fn candidate_supernets_probes_only_configured_prefix_lengths() {
        let prefixes = SubnetPrefixes::from_networks(
            [
                "10.0.0.0/24".parse::<ipnet::IpNet>().unwrap(),
                "172.16.0.0/16".parse::<ipnet::IpNet>().unwrap(),
            ]
            .iter(),
        );

        let ip: IpAddr = "10.0.5.7".parse().unwrap();
        let candidates: Vec<ipnet::IpNet> = prefixes.candidate_supernets(ip).collect();

        // Only the configured prefix lengths (24, 16), longest first, truncated.
        assert_eq!(
            candidates,
            vec![
                "10.0.5.0/24".parse().unwrap(),
                "10.0.0.0/16".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn candidate_supernets_are_empty_for_other_family() {
        let prefixes = SubnetPrefixes::from_networks(
            ["2001:db8::/32".parse::<ipnet::IpNet>().unwrap()].iter(),
        );

        let ipv4: IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(prefixes.candidate_supernets(ipv4).count(), 0);
    }

    #[test]
    fn allow_canonicalizes_ipv4_mapped_ipv6() {
        // Generous limits so the request is allowed, and we can read back the bucket key.
        let config = RateLimiterConfig::<TestSpec> {
            max_allowed_resources: TotalResources {
                inner: Resource {
                    req_counter: 100000,
                    space_in_bytes: 5000,
                    execution_time_micros: 100000,
                    gas_used: Gas::from([0, 0]),
                },
            },
            refill_rate: RefillRatePerMillis {
                token_resource_per_ms: Resource {
                    req_counter: 0,
                    space_in_bytes: 0,
                    execution_time_micros: 0,
                    gas_used: Gas::from([0, 0]),
                },
            },
        };
        let mut rate_limiter =
            SovRateLimiter::new_for_test(1000, Duration::from_millis(1_000_000), Some(config));
        let addr = <TestSpec as Spec>::Address::from([1; 28]);

        // `::ffff:10.0.0.5` must be charged to the IPv4 host network, not a V6 /128,
        // so it cannot dodge an IPv4 subnet limit by arriving in mapped form.
        let mapped: IpAddr = "::ffff:10.0.0.5".parse().unwrap();
        let token = rate_limiter.allow(mapped, addr).unwrap().unwrap();
        assert_eq!(token.ip_net, "10.0.0.5/32".parse::<ipnet::IpNet>().unwrap());
    }

    #[test]
    fn programmatic_noncanonical_ip_custom_limit_is_normalized() {
        let config = SovRateLimiterConfig {
            max_requests_per_second: 1000,
            max_nb_of_concurrent_users_in_rate_limiter: 1000,
            default_limits: Limits {
                resources_per_bucket: 5,
                refill_rate: 0,
            },
            height_for_gas_limit_computation: RollupHeight::GENESIS,
            address_custom_limits: Vec::default(),
            ip_custom_limits: vec![(
                "10.0.0.5/24".parse().unwrap(),
                Limits {
                    resources_per_bucket: 0,
                    refill_rate: 0,
                },
            )],
        };
        let mut rate_limiter =
            SovRateLimiter::<TestSpec>::new(Some(config), Duration::from_millis(3000), 1_000_000);
        let addr = <TestSpec as Spec>::Address::from([1; 28]);
        let ip: IpAddr = "10.0.0.42".parse().unwrap();

        let err = rate_limiter.allow(ip, addr).unwrap_err();
        let ResourceLimitExceededError::Ip { ip_net, .. } = err else {
            panic!("expected IP rate-limit error");
        };
        assert_eq!(ip_net, "10.0.0.0/24".parse::<ipnet::IpNet>().unwrap());
    }

    #[test]
    fn configured_ipv4_mapped_ipv6_subnet_is_normalized() {
        let config = SovRateLimiterConfig {
            max_requests_per_second: 1000,
            max_nb_of_concurrent_users_in_rate_limiter: 1000,
            default_limits: Limits {
                resources_per_bucket: 5,
                refill_rate: 0,
            },
            height_for_gas_limit_computation: RollupHeight::GENESIS,
            address_custom_limits: Vec::default(),
            ip_custom_limits: vec![(
                "::ffff:10.0.0.42/120".parse().unwrap(),
                Limits {
                    resources_per_bucket: 0,
                    refill_rate: 0,
                },
            )],
        };
        let mut rate_limiter =
            SovRateLimiter::<TestSpec>::new(Some(config), Duration::from_millis(3000), 1_000_000);
        let addr = <TestSpec as Spec>::Address::from([1; 28]);
        let ip: IpAddr = "10.0.0.5".parse().unwrap();

        let err = rate_limiter.allow(ip, addr).unwrap_err();
        let ResourceLimitExceededError::Ip { ip_net, .. } = err else {
            panic!("expected IP rate-limit error");
        };
        assert_eq!(ip_net, "10.0.0.0/24".parse::<ipnet::IpNet>().unwrap());
    }

    #[test]
    fn address_and_ip_throttlers_charge_independently() {
        // Tight custom limits on both a specific address and an IP subnet, with a
        // loose default. A single admission charges both throttlers, so either
        // side can later reject — verified by exhausting one side at a time.
        let limited_addr = test_addr(7);
        let subnet: ipnet::IpNet = "10.0.0.0/24".parse().unwrap();
        let tight = Limits {
            resources_per_bucket: 1,
            refill_rate: 0,
        };
        let loose = Limits {
            resources_per_bucket: 1000,
            refill_rate: 0,
        };
        let config = SovRateLimiterConfig {
            max_requests_per_second: 1000,
            max_nb_of_concurrent_users_in_rate_limiter: 1000,
            default_limits: loose,
            height_for_gas_limit_computation: RollupHeight::GENESIS,
            address_custom_limits: vec![(limited_addr, tight)],
            ip_custom_limits: vec![(subnet, tight)],
        };
        let mut rate_limiter =
            SovRateLimiter::<TestSpec>::new(Some(config), Duration::from_millis(1000), 1_000_000);
        let ip_in_subnet: IpAddr = "10.0.0.1".parse().unwrap();

        // Both throttlers have budget; the admission charges both.
        let token = rate_limiter.allow(ip_in_subnet, limited_addr).unwrap();
        rate_limiter.update(token, one_request());

        // Same custom address: address throttler is exhausted; the address-side
        // rejects.
        let err = rate_limiter.allow(ip_in_subnet, limited_addr).unwrap_err();
        let ResourceLimitExceededError::Address { address, .. } = err else {
            panic!("expected Address rejection, got {err:?}");
        };
        assert_eq!(address, limited_addr);

        // Different address (loose default) but same subnet: the address-side is
        // fresh, so the IP throttler is the one that rejects — proving it was
        // charged by the first admission alongside the address throttler.
        let err = rate_limiter.allow(ip_in_subnet, test_addr(8)).unwrap_err();
        let ResourceLimitExceededError::Ip { ip_net, .. } = err else {
            panic!("expected Ip rejection, got {err:?}");
        };
        assert_eq!(ip_net, subnet);
    }
}
