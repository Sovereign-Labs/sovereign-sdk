use crate::CacheValue;

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum MergeError {
    #[cfg_attr(
        feature = "std",
        error("consecutive reads are inconsistent: left read: {left:?}, right read: {right:?}")
    )]
    ReadThenRead {
        left: Option<CacheValue>,
        right: Option<CacheValue>,
    },
    #[cfg_attr(
        feature = "std",
        error("the read: {read:?} is in inconsistent with the previous write: {write:?}")
    )]
    WriteThenRead {
        write: Option<CacheValue>,
        read: Option<CacheValue>,
    },
}

/// `Access` represents a sequence of events on a particular value.
/// For example, a transaction might read a value, then take some action which causes it to be updated
/// The rules for defining causality are as follows:
/// 1. If a read is preceded by another read, check that the two reads match and discard one.
/// 2. If a read is preceded by a write, check that the value read matches the value written. Discard the read.
/// 3. Otherwise, retain the read.
/// 4. A write is retained unless it is followed by another write.
#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) enum Access {
    Read(Option<CacheValue>),
    ReadThenWrite {
        original: Option<CacheValue>,
        modified: Option<CacheValue>,
    },
    Write(Option<CacheValue>),
}

impl Access {
    pub(crate) fn last_value(&self) -> &Option<CacheValue> {
        match self {
            Access::Read(value) => value,
            Access::ReadThenWrite { modified, .. } => modified,
            Access::Write(value) => value,
        }
    }

    pub(crate) fn write_value(&mut self, new_value: Option<CacheValue>) {
        match self {
            // If we've already read this slot, turn it into a readThenWrite access
            Access::Read(original) => {
                // If we're resetting the key to its original value, we can just discard the write history
                if original == &new_value {
                    return;
                }
                // Otherwise, keep track of the original value and the new value
                *self = Access::ReadThenWrite {
                    original: original.take(),

                    modified: new_value,
                };
            }
            // For ReadThenWrite override the modified value with a new value
            Access::ReadThenWrite { original, modified } => {
                // If we're resetting the key to its original value, we can just discard the write history
                if original == &new_value {
                    *self = Access::Read(new_value)
                } else {
                    *modified = new_value
                }
            }
            // For Write override the original value with a new value
            // We can do this unconditionally, since overwriting a value with itself is a no-op
            Access::Write(value) => *value = new_value,
        }
    }

    pub(crate) fn merge(&mut self, rhs: Self) -> Result<(), MergeError> {
        // Pattern matching on (`self`, rhs) is a bit cleaner, but would move the `self` inside the tuple.
        // We need the `self` later on for *self = Access.. therefore the nested solution.
        match self {
            Access::Read(left_read) => match rhs {
                Access::Read(right_read) => {
                    if left_read != &right_read {
                        Err(MergeError::ReadThenRead {
                            left: left_read.clone(),
                            right: right_read,
                        })
                    } else {
                        Ok(())
                    }
                }
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                } => {
                    if left_read != &right_original {
                        Err(MergeError::ReadThenRead {
                            left: left_read.clone(),
                            right: right_original,
                        })
                    } else {
                        *self = Access::ReadThenWrite {
                            original: right_original,
                            modified: right_modified,
                        };

                        Ok(())
                    }
                }
                Access::Write(right_write) => {
                    *self = Access::ReadThenWrite {
                        original: left_read.take(),
                        modified: right_write,
                    };
                    Ok(())
                }
            },
            Access::ReadThenWrite {
                original: left_original,
                modified: left_modified,
            } => match rhs {
                Access::Read(right_read) => {
                    if left_modified != &right_read {
                        Err(MergeError::WriteThenRead {
                            write: left_modified.clone(),
                            read: right_read,
                        })
                    } else {
                        Ok(())
                    }
                }
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                } => {
                    if left_modified != &right_original {
                        Err(MergeError::WriteThenRead {
                            write: left_modified.clone(),
                            read: right_original,
                        })
                    } else {
                        *self = Access::ReadThenWrite {
                            original: left_original.take(),
                            modified: right_modified,
                        };
                        Ok(())
                    }
                }
                Access::Write(right_write) => {
                    *self = Access::ReadThenWrite {
                        original: left_original.take(),
                        modified: right_write,
                    };
                    Ok(())
                }
            },
            Access::Write(left_write) => match rhs {
                Access::Read(right_read) => {
                    if left_write != &right_read {
                        Err(MergeError::WriteThenRead {
                            write: left_write.clone(),
                            read: right_read,
                        })
                    } else {
                        Ok(())
                    }
                }
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                } => {
                    if left_write != &right_original {
                        Err(MergeError::WriteThenRead {
                            write: left_write.clone(),
                            read: right_original,
                        })
                    } else {
                        *self = Access::Write(right_modified);
                        Ok(())
                    }
                }
                Access::Write(right_write) => {
                    *self = Access::Write(right_write);
                    Ok(())
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_util::create_value;

    #[test]
    fn test_access_read_write() {
        let original_value = create_value(1);
        let mut access = Access::Read(original_value.clone());

        // Check: Read => ReadThenWrite transition
        {
            let new_value = create_value(2);
            access.write_value(new_value.clone());

            assert_eq!(access.last_value(), &new_value);
            assert_eq!(
                access,
                Access::ReadThenWrite {
                    original: original_value.clone(),
                    modified: new_value
                }
            );
        }

        // Check: ReadThenWrite => ReadThenWrite transition
        {
            let new_value = create_value(3);
            access.write_value(new_value.clone());

            assert_eq!(access.last_value(), &new_value);
            assert_eq!(
                access,
                Access::ReadThenWrite {
                    original: original_value,
                    modified: new_value
                }
            );
        }
    }

    #[test]
    fn test_access_write() {
        let original_value = create_value(1);
        let mut access = Access::Write(original_value.clone());

        // Check: Write => Write transition
        {
            assert_eq!(access.last_value(), &original_value);
            let new_value = create_value(3);
            access.write_value(new_value.clone());
            assert_eq!(access.last_value(), &new_value);
            assert_eq!(access, Access::Write(new_value));
        }
    }

    #[test]
    fn test_access_merge() {
        let first_read = 1;
        let mut value = create_value(first_read);
        let mut left = Access::Read(value.clone());

        let last_write = 10;
        for i in 2..last_write + 1 {
            left.merge(Access::Read(value.clone())).unwrap();

            value = create_value(i);
            left.merge(Access::Write(value.clone())).unwrap();
        }

        assert_eq!(
            left,
            Access::ReadThenWrite {
                original: create_value(first_read),
                modified: create_value(last_write)
            }
        )
    }

    #[test]
    fn test_err_merge_left_read_neq_right_read() {
        let first_read = 1;
        let value = create_value(first_read);
        let left = &mut Access::Read(value.clone());

        let second_read = 2;
        let value2 = create_value(second_read);

        assert_eq!(
            left.merge(Access::Read(value2.clone())),
            Err(MergeError::ReadThenRead {
                left: value,
                right: value2,
            })
        );
    }

    #[test]
    fn test_err_merge_left_read_neq_right_orig() {
        let first_read = 1;
        let value = create_value(first_read);
        let left = &mut Access::Read(value.clone());

        let second_read = 2;
        let value2 = create_value(second_read);
        let right = Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        assert_eq!(
            left.merge(right),
            Err(MergeError::ReadThenRead {
                left: value,
                right: value2,
            })
        );
    }

    #[test]
    fn test_err_merge_left_mod_neq_right_read() {
        let first_read = 1;
        let value = create_value(first_read);

        let second_read = 2;
        let value2 = create_value(second_read);

        let left = &mut Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        let right = Access::Read(value2.clone());

        assert_eq!(
            left.merge(right),
            Err(MergeError::WriteThenRead {
                write: value,
                read: value2,
            })
        )
    }

    #[test]
    fn test_err_merge_left_mod_neq_right_orig() {
        let first_read = 1;
        let value = create_value(first_read);

        let second_read = 2;
        let value2 = create_value(second_read);

        let left = &mut Access::ReadThenWrite {
            original: value.clone(),
            modified: value2.clone(),
        };

        let right = Access::ReadThenWrite {
            original: value.clone(),
            modified: value2.clone(),
        };

        assert_eq!(
            left.merge(right),
            Err(MergeError::WriteThenRead {
                write: value2,
                read: value,
            })
        )
    }

    #[test]
    fn test_err_merge_left_right_neq_right_orig() {
        let first_read = 1;
        let value = create_value(first_read);

        let second_read = 2;
        let value2 = create_value(second_read);

        let left = &mut Access::Write(value.clone());
        let right = Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        assert_eq!(
            left.merge(right),
            Err(MergeError::WriteThenRead {
                write: value,
                read: value2,
            })
        )
    }
}
