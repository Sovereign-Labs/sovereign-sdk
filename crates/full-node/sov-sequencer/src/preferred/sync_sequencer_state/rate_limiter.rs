#![allow(dead_code)]
use sov_modules_api::GasArray;
use sov_modules_api::{CredentialId, Spec};
use std::collections::BTreeSet;
use std::hash::Hash;
use std::net::IpAddr;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

enum ResourceLimitExceededError {}

struct ResourceUsed<S: Spec> {
    req_counter: usize,
    space_in_bytes: usize,
    execution_time_micros: u64,
    gas_used: S::Gas,
}

impl<S: Spec> ResourceUsed<S> {
    fn zero() -> Self {
        Self {
            req_counter: todo!(),
            space_in_bytes: todo!(),
            execution_time_micros: todo!(),
            gas_used: todo!(),
        }
    }

    fn combine(&mut self, other: &Self) {
        self.req_counter += other.req_counter;
        self.space_in_bytes += other.space_in_bytes;
        self.execution_time_micros += other.execution_time_micros;
        self.gas_used = self.gas_used.checked_combine(other.gas_used).unwrap();
    }

    fn allow(&self, max: &Self) -> Result<(), ResourceLimitExceededError> {
        if self.req_counter > max.req_counter {
            todo!()
        }

        if self.space_in_bytes > max.space_in_bytes {
            todo!()
        }

        if self.execution_time_micros > max.execution_time_micros {
            todo!()
        }

        //if !self.gas_used.dim_is_less_than(other.gas_used) {}

        Ok(())
    }
}

struct Throttler<S: Spec> {
    _phantopm: std::marker::PhantomData<S>,
    window_start: Instant,
    total_resource_used: ResourceUsed<S>,
}

impl<S: Spec> Throttler<S> {
    pub fn new() -> Self {
        Self {
            _phantopm: std::marker::PhantomData,
            window_start: Instant::now(),
            total_resource_used: ResourceUsed::zero(),
        }
    }

    fn throttle(
        &mut self,
        window: &Duration,
        used_resources: &ResourceUsed<S>,
        max_resources: &ResourceUsed<S>,
        now: Instant,
    ) -> Result<(), ResourceLimitExceededError> {
        if &now.duration_since(self.window_start) >= window {
            self.window_start = now;
            self.total_resource_used = ResourceUsed::zero();
        }

        // Q should we self.total_resource_used.combine(used_resources); before allow
        self.total_resource_used.allow(max_resources)?;
        self.total_resource_used.combine(used_resources);

        Ok(())
    }
}

struct Entry<S: Spec> {
    throttler: Throttler<S>,
    expire_at: Instant,
}

pub struct RateLimiter<K, S: Spec> {
    per_key: HashMap<K, Entry<S>>,

    // Throttler params
    max_resources: ResourceUsed<S>,
    window: Duration,

    // If we want to make it work outside sync section use moka-rs
    ttl: Duration,
    queue: BTreeSet<(Instant, K)>,
    // TODO Do we need this?
    // min_prune_interval: Duration,
    // last_prune: Instant,
}
use std::collections::hash_map;

impl<K: Ord + Hash + Clone, S: Spec> RateLimiter<K, S> {
    fn new(max_resources: ResourceUsed<S>, window: Duration, ttl: Duration) -> Self {
        Self {
            max_resources,
            window,
            ttl,
            per_key: HashMap::new(),
            queue: BTreeSet::new(),
        }
    }

    pub fn prune(&mut self, now: Instant) {
        loop {
            let first = match self.queue.pop_first() {
                Some(entry) => entry,
                None => return,
            };

            if first.0 > now {
                self.queue.insert(first);
                // earliest expiry is in the future → nothing to do
                return;
            }

            let (exp, key) = first;
            self.queue.remove(&(exp.clone(), key.clone()));

            match self.per_key.get(&key) {
                Some(entry) => {
                    assert_eq!(entry.expire_at, exp);
                    self.per_key.remove(&key);
                    return;
                }
                _ => {
                    panic!("TODO")
                }
            }
        }
    }

    fn allow(
        &mut self,
        key: K,
        used_resources: &ResourceUsed<S>,
    ) -> Result<(), ResourceLimitExceededError> {
        let now = Instant::now();
        self.prune(now);
        let new_exp = now + self.ttl;

        let entry = match self.per_key.entry(key.clone()) {
            hash_map::Entry::Occupied(occupied) => {
                let old_exp = occupied.get().expire_at;
                self.queue.remove(&(old_exp, key.clone()));
                occupied.into_mut()
            }
            hash_map::Entry::Vacant(vacant) => vacant.insert(Entry {
                throttler: Throttler::new(),
                expire_at: new_exp,
            }),
        };

        let allowed =
            entry
                .throttler
                .throttle(&self.window, used_resources, &self.max_resources, now);

        self.queue.insert((new_exp, key));

        allowed
    }
}

struct RollupRateLimiter<S: Spec> {
    by_credential: RateLimiter<CredentialId, S>,
    by_ip: RateLimiter<IpAddr, S>,
}

impl<S: Spec> RollupRateLimiter<S> {
    fn new() -> Self {
        todo!()
    }

    fn check_if_allowed(
        &mut self,
        credential_id: CredentialId,
        ip: IpAddr,
        used_resources: &ResourceUsed<S>,
    ) -> Result<(), ResourceLimitExceededError> {
        self.by_ip.allow(ip, used_resources)?;
        self.by_credential.allow(credential_id, used_resources)?;

        Ok(())
    }
}
