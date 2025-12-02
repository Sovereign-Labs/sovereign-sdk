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

pub struct ResourceUsed<S: Spec> {
    pub req_counter: usize,
    pub space_in_bytes: usize,
    pub execution_time_micros: u64,
    pub gas_used: S::Gas,
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

    fn clear(&mut self) {
        *self = Self::zero()
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

    fn allow(&mut self, max_resources: &ResourceUsed<S>) -> Result<(), ResourceLimitExceededError> {
        self.total_resource_used.allow(max_resources)?;
        Ok(())
    }

    fn update(&mut self, used_resources: &ResourceUsed<S>, window: &Duration, now: Instant) {
        if &now.duration_since(self.window_start) >= window {
            self.window_start = now;
            self.total_resource_used.clear();
        }

        self.total_resource_used.combine(used_resources);
    }
}

struct Entry<S: Spec> {
    throttler: Throttler<S>,
    expire_at: Instant,
}

// TOCKEN BUCKET

pub struct RateLimiter<K, S: Spec> {
    per_key: HashMap<K, Entry<S>>,

    window: Duration,

    // If we want to make it work outside sync section use mini_moka or mini_moka::unsync::Cache
    ttl: Duration,
    queue: BTreeSet<(Instant, K)>,
    // TODO Do we need this?
    // min_prune_interval: Duration,
    // last_prune: Instant,
}

impl<K: Ord + Hash + Clone, S: Spec> RateLimiter<K, S> {
    fn new(window: Duration, ttl: Duration) -> Self {
        Self {
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

    fn remove(&mut self, key: &K) -> Option<Entry<S>> {
        self.per_key.remove(key)
    }

    fn update(&mut self, key: K, entry: Option<Entry<S>>, resource_used: &ResourceUsed<S>) {
        let now = Instant::now();
        let expire_at = now + self.ttl;
        self.prune(now);

        match entry {
            Some(mut e) => {
                self.queue.remove(&(e.expire_at, key.clone()));
                e.throttler.update(resource_used, &self.window, now);
                e.expire_at = expire_at;
                self.per_key.insert(key.clone(), e);
            }
            None => {
                let mut throttler = Throttler::new();
                throttler.update(resource_used, &self.window, now);
                self.per_key.insert(
                    key.clone(),
                    Entry {
                        throttler: throttler,
                        expire_at: expire_at,
                    },
                );
            }
        }

        self.queue.insert((now + self.ttl, key));
    }
}

pub struct CacheEntry<S: Spec> {
    entry_for_ip: Option<Entry<S>>,
    entry_for_credential: Option<Entry<S>>,
    ip: IpAddr,
    credential_id: CredentialId,
}

// TODO Addr/IP whitelist?
struct RollupRateLimiter<S: Spec> {
    by_credential: RateLimiter<CredentialId, S>,
    by_ip: RateLimiter<IpAddr, S>,
    max_resources: ResourceUsed<S>,
    foo: HashMap<CredentialId, ResourceUsed>,
}

impl<S: Spec> RollupRateLimiter<S> {
    fn new() -> Self {
        todo!()
    }

    fn allow(
        &mut self,
        credential_id: CredentialId,
        ip: IpAddr,
    ) -> Result<CacheEntry<S>, ResourceLimitExceededError> {
        let mut entry_for_ip = self.by_ip.remove(&ip);
        let mut entry_for_credential = self.by_credential.remove(&credential_id);

        match (&mut entry_for_ip, &mut entry_for_credential) {
            (Some(e1), Some(e2)) => {
                let r1 = e1.throttler.allow(&self.max_resources);
                let r2 = e2.throttler.allow(&self.max_resources);

                if r1.is_err() || r2.is_err() {
                    self.by_ip.per_key.insert(ip, entry_for_ip);
                    self.by_credential
                        .per_key
                        .insert(credential_id, entry_for_credential);
                    todo!()
                }
            }
            (None, None) => {}
            _ => {
                todo!()
            }
        };

        Ok(CacheEntry {
            entry_for_ip,
            entry_for_credential,
            ip,
            credential_id,
        })
    }

    fn update(&mut self, e: CacheEntry<S>, used_resources: &ResourceUsed<S>) {
        self.by_ip.update(e.ip, e.entry_for_ip, used_resources);
        self.by_credential
            .update(e.credential_id, e.entry_for_credential, used_resources);
    }
}
