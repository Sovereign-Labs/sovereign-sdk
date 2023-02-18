use crate::CacheValue;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum MergeError {
    #[error("consecutive reads are inconsistent: left read: {left:?}, right read: {right:?}")]
    ReadThenRead {
        left: Option<CacheValue>,
        right: Option<CacheValue>,
    },
    #[error("the read: {read:?} is in inconsistent with the previous write: {write:?}")]
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
                *self = Access::ReadThenWrite {
                    original: original.take(),

                    modified: new_value,
                };
            }
            // For ReadThenWrite override the modified value with a new value
            Access::ReadThenWrite { modified, .. } => *modified = new_value,
            // For Write override the original value with a new value
            Access::Write(value) => *value = new_value,
        }
    }

    pub(crate) fn merge(self, rhs: Self) -> Result<Self, MergeError> {
        match (self, rhs) {
            (Access::Read(left_read), Access::Read(right_read)) => {
                if left_read != right_read {
                    Err(MergeError::ReadThenRead {
                        left: left_read,
                        right: right_read,
                    })
                } else {
                    Ok(Access::Read(left_read))
                }
            }
            (
                Access::Read(left_read),
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                },
            ) => {
                if left_read != right_original {
                    Err(MergeError::ReadThenRead {
                        left: left_read,
                        right: right_original,
                    })
                } else {
                    Ok(Access::ReadThenWrite {
                        original: right_original,
                        modified: right_modified,
                    })
                }
            }
            (Access::Read(left_read), Access::Write(right_write)) => Ok(Access::ReadThenWrite {
                original: left_read,
                modified: right_write,
            }),
            (
                Access::ReadThenWrite {
                    original: left_original,
                    modified: left_modified,
                },
                Access::Read(right_read),
            ) => {
                if left_modified != right_read {
                    Err(MergeError::WriteThenRead {
                        write: left_modified,
                        read: right_read,
                    })
                } else {
                    Ok(Access::ReadThenWrite {
                        original: left_original,
                        modified: left_modified,
                    })
                }
            }
            (
                Access::ReadThenWrite {
                    original: left_original,
                    modified: left_modified,
                },
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                },
            ) => {
                if left_modified != right_original {
                    Err(MergeError::WriteThenRead {
                        write: left_modified,
                        read: right_original,
                    })
                } else {
                    Ok(Access::ReadThenWrite {
                        original: left_original,
                        modified: right_modified,
                    })
                }
            }
            (
                Access::ReadThenWrite {
                    original: left_original,
                    ..
                },
                Access::Write(right_write),
            ) => Ok(Access::ReadThenWrite {
                original: left_original,
                modified: right_write,
            }),
            (Access::Write(left_write), Access::Read(right_read)) => {
                if left_write != right_read {
                    Err(MergeError::WriteThenRead {
                        write: left_write,
                        read: right_read,
                    })
                } else {
                    Ok(Access::Write(left_write))
                }
            }
            (
                Access::Write(left_write),
                Access::ReadThenWrite {
                    original: right_original,
                    modified: right_modified,
                },
            ) => {
                if left_write != right_original {
                    Err(MergeError::WriteThenRead {
                        write: left_write,
                        read: right_original,
                    })
                } else {
                    Ok(Access::Write(right_modified))
                }
            }
            (Access::Write(_), Access::Write(right_write)) => Ok(Access::Write(right_write)),
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
            left = left.merge(Access::Read(value.clone())).unwrap();

            value = create_value(i);
            left = left.merge(Access::Write(value.clone())).unwrap();
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
        let left = Access::Read(value.clone());

        let second_read = 2;
        let value2 = create_value(second_read);

        assert_eq!(left.merge(Access::Read(value2.clone())),
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
        let left = Access::Read(value.clone());

        let second_read = 2;
        let value2 = create_value(second_read);
        let right = Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        assert_eq!(left.merge(right),
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

        let left = Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };


        let right = Access::Read(value2.clone());

        assert_eq!(left.merge(right),
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

        let left = Access::ReadThenWrite {
            original: value.clone(),
            modified: value2.clone(),
        };

        let right = Access::ReadThenWrite {
            original: value.clone(),
            modified: value2.clone(),
        };

        assert_eq!(left.merge(right),
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

        let left = Access::Write(value.clone());
        let right = Access::ReadThenWrite {
            original: value2.clone(),
            modified: value.clone(),
        };

        assert_eq!(left.merge(right),
                   Err(MergeError::WriteThenRead {
                       write: value,
                       read: value2,
                   })
        )
    }
}
