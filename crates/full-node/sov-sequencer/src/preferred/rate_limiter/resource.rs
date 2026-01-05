use sov_modules_api::Gas;

/// Reason why the rate limit was exceeded.
#[derive(Debug, PartialEq)]
pub(crate) enum LimitExceeded<G: Gas> {
    RequestCount {
        total_accumulated: u64,
        max_allowed: u64,
    },
    Space {
        total_accumulated: u64,
        max_allowed: u64,
    },
    ExecutionTime {
        total_accumulated: u64,
        max_allowed: u64,
    },
    Gas {
        total_accumulated: G,
        max_allowed: G,
    },
    TooManyConcurrentUsers {
        nb_of_users: u64,
        max_allowed: u64,
    },
}

// Represents a resource that requires rate limiting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Resource<G: Gas> {
    pub(crate) req_counter: u64,
    pub(crate) space_in_bytes: u64,
    pub(crate) execution_time_micros: u64,
    pub(crate) gas_used: G,
}

impl<G: Gas> Resource<G> {
    pub(crate) fn zero() -> Self {
        Self {
            req_counter: 0,
            space_in_bytes: 0,
            execution_time_micros: 0,
            gas_used: Gas::zero(),
        }
    }

    pub(crate) fn max() -> Self {
        Self {
            req_counter: u64::MAX,
            space_in_bytes: u64::MAX,
            execution_time_micros: u64::MAX,
            gas_used: Gas::max(),
        }
    }

    #[must_use]
    pub(crate) fn saturating_sub(&self, tokens: &Self) -> Self {
        Self {
            req_counter: self.req_counter.saturating_sub(tokens.req_counter),
            space_in_bytes: self.space_in_bytes.saturating_sub(tokens.space_in_bytes),
            execution_time_micros: self
                .execution_time_micros
                .saturating_sub(tokens.execution_time_micros),
            gas_used: match self.gas_used.checked_sub(tokens.gas_used) {
                Some(gas) => gas,
                None => Gas::zero(),
            },
        }
    }

    #[must_use]
    pub(crate) fn checked_add(&self, other: &Self) -> Option<Self> {
        Some(Self {
            req_counter: self.req_counter.checked_add(other.req_counter)?,
            space_in_bytes: self.space_in_bytes.checked_add(other.space_in_bytes)?,
            execution_time_micros: self
                .execution_time_micros
                .checked_add(other.execution_time_micros)?,
            gas_used: self.gas_used.checked_combine(other.gas_used)?,
        })
    }

    #[must_use]
    pub(crate) fn saturating_mul_by_scalar(&self, scalar: u64) -> Self {
        let gas_used = self
            .gas_used
            .checked_scalar_product(scalar)
            .unwrap_or(G::max());

        Self {
            req_counter: self.req_counter.saturating_mul(scalar),
            space_in_bytes: self.space_in_bytes.saturating_mul(scalar),
            execution_time_micros: self.execution_time_micros.saturating_mul(scalar),
            gas_used,
        }
    }

    #[must_use]
    pub(crate) fn div_by_scalar(&self, scalar: u64) -> Self {
        let gas_used = self.gas_used.scalar_division(scalar);

        Self {
            req_counter: self.req_counter / scalar,
            space_in_bytes: self.space_in_bytes / scalar,
            execution_time_micros: self.execution_time_micros / scalar,
            gas_used,
        }
    }

    pub(crate) fn err_if_exceeding(&self, other: &Self) -> Result<(), LimitExceeded<G>> {
        // Error if the total accumulated is greater than or equal to the max allowed. 
        // Using >= instead of > to ensure that a rate limit of zero prevents all requests.
        if self.req_counter >= other.req_counter {
            return Err(LimitExceeded::RequestCount {
                total_accumulated: self.req_counter,
                max_allowed: other.req_counter,
            });
        }

        if self.space_in_bytes >= other.space_in_bytes {
            return Err(LimitExceeded::Space {
                total_accumulated: self.space_in_bytes,
                max_allowed: other.space_in_bytes,
            });
        }

        if self.execution_time_micros >= other.execution_time_micros {
            return Err(LimitExceeded::ExecutionTime {
                total_accumulated: self.execution_time_micros,
                max_allowed: other.execution_time_micros,
            });
        }

        // Gas is not included until all gas-related issues are resolved:
        // - proper constants
        // - proper gas estimation

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use sov_modules_api::Spec;
    use sov_test_utils::TestSpec;

    type Gas = <TestSpec as Spec>::Gas;

    impl Resource<Gas> {
        fn new(
            req_counter: u64,
            space_in_bytes: u64,
            execution_time_micros: u64,
            gas_used: Gas,
        ) -> Self {
            Self {
                req_counter,
                space_in_bytes,
                execution_time_micros,
                gas_used,
            }
        }

        fn from(n: u64) -> Self {
            Self {
                req_counter: n,
                space_in_bytes: n,
                execution_time_micros: n,
                gas_used: Gas::from([n, n]),
            }
        }
    }

    #[test]
    fn test_resouce_saturating_sub() {
        {
            let zero: Resource<_> = Resource::<Gas>::zero();
            let r1 = Resource::from(1);
            let r2 = Resource::from(2);

            assert_eq!(r1.saturating_sub(&r2), zero);
        }

        {
            let r1 = Resource::from(5);
            let r2 = Resource::from(2);

            assert_eq!(r1.saturating_sub(&r2), Resource::from(3));
        }
    }

    #[test]
    fn test_resouce_addition() {
        let r1 = Resource::from(1);
        {
            let r2 = Resource::from(2);
            assert_eq!(r1.checked_add(&r2), Some(Resource::from(3)));
        }

        {
            let max = Resource::from(u64::MAX);
            assert_eq!(r1.checked_add(&max), None);
        }
    }

    #[test]
    fn test_resouce_multiplication() {
        let r1 = Resource::from(3);

        assert_eq!(r1.saturating_mul_by_scalar(2), Resource::from(6));

        assert_eq!(
            r1.saturating_mul_by_scalar(u64::MAX),
            Resource::from(u64::MAX)
        );
    }

    #[test]
    fn test_resouce_exceeding() {
        let r1 = Resource::from(1);
        let r2 = Resource::from(2);

        assert!(r1.err_if_exceeding(&r2).is_ok());
        assert!(r2.err_if_exceeding(&r1).is_err());

        {
            let r1 = Resource::new(5, 5, 7, Gas::from([8, 9]));
            let r2 = Resource { ..r1 };
            assert!(r2.err_if_exceeding(&r1).is_ok());
        }

        {
            let r1 = Resource::new(100, 101, 102, Gas::from([103, 104]));
            let r2 = Resource {
                req_counter: 1,
                ..r1
            };
            assert_eq!(
                r1.err_if_exceeding(&r2),
                Err(LimitExceeded::RequestCount {
                    total_accumulated: { r1.req_counter },
                    max_allowed: { r2.req_counter },
                })
            );
        }

        {
            let r1 = Resource::new(100, 101, 102, Gas::from([103, 104]));
            let r2 = Resource {
                space_in_bytes: 1,
                ..r1
            };
            assert_eq!(
                r1.err_if_exceeding(&r2),
                Err(LimitExceeded::Space {
                    total_accumulated: { r1.space_in_bytes },
                    max_allowed: { r2.space_in_bytes },
                })
            );
        }

        {
            let r1 = Resource::new(100, 101, 102, Gas::from([103, 104]));
            let r2 = Resource {
                execution_time_micros: 1,
                ..r1
            };
            assert_eq!(
                r1.err_if_exceeding(&r2),
                Err(LimitExceeded::ExecutionTime {
                    total_accumulated: { r1.execution_time_micros },
                    max_allowed: { r2.execution_time_micros },
                })
            );
        }
    }
}
