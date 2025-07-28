//! Implements a temporary cache that persists for the duration of the block.

use std::any::{Any, TypeId};
use std::collections::HashMap;

use sov_rollup_interface::common::SizedSafeString;

/// The maximum number of entries we expect in the temporary cache.
///
/// This is used to warn if the cache is getting too large.
const MAX_EXPECTED_CACHE_ITEMS: usize = 100;

/// The maximum number of bytes we expect in the temporary cache.
///
/// This is used to warn if the cache is getting too large.
const MAX_EXPECTED_CACHE_BYTES: usize = 10_000_000; // 10MB

type Value = Option<(Box<(dyn Any + Send + Sync + 'static)>, usize)>;

/// The result of a cache lookup.
#[derive(Debug, PartialEq, Eq)]
pub enum CacheLookup<'a, T> {
    /// The value is present in the cache
    Hit(Option<&'a T>),
    /// The value is not present in the cache
    Miss,
}

impl<'a, T> From<Option<Option<&'a T>>> for CacheLookup<'a, T> {
    fn from(value: Option<Option<&'a T>>) -> Self {
        match value {
            Some(v) => CacheLookup::Hit(v),
            None => CacheLookup::Miss,
        }
    }
}
/// A temporary cache whose values persist for at most the duration of the block.
///
/// Values in the cache are *not* visible to the API, since `Clone` bounds are
/// not required.
pub struct TempCache {
    cache: HashMap<TypeId, Value>,
    /// An estimate of the memory size of the cache. Note that `None` values are not included in this count.
    memory_size: usize,
}

impl Default for TempCache {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TempCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TempCache").finish()
    }
}

/// Returns the estimated size of the value when borsh serialized. This is used to estimate the memory size of the value in
/// a platform-independent manner. Note that a type need not actually implement `Borsh` in order to estimate its size
/// using this trait.
///
/// ## Implementation Guide
/// In borsh, integers have their expected size (u8 is 1 byte, u16 is 2 bytes, etc.), and `bool` is 1 byte which the exception
/// of usize - which is always 4 bytes regardless of the platform. The size of a struct/tuple/array/enum variant is simply the sum
/// of its field sizes, and the size of an enum is simplythe size of the active variant + 1 byte for the discriminant.
/// Finally, dynamic types (vectors, strings, maps, etc.) are simply the size of their contents plus 4 bytes for the length.
///
/// ## Example
///
/// ```rust
/// # use sov_modules_api::BorshSerializedSize;
/// struct MyStruct {
///     field1: u8,
///     field2: Vec<usize>
/// }
///
/// impl BorshSerializedSize for MyStruct {
///     fn serialized_size(&self) -> usize {
///         1 + // field1
///         4 + // vec_length
///         self.field2.len() * 4 // entries of the vec
///     }
/// }
pub trait BorshSerializedSize {
    /// The size of the value when borsh serialized, in bytes. Note that this is an estimate - as long as the size is
    /// consistent across calls and reasonably close to the real value, it's acceptable to be slightly inaccurate. However,
    /// better accuracy will yield better rollup performance and more accurate gas metering.
    fn serialized_size(&self) -> usize;
}

impl TempCache {
    /// Creates a new temporary cache.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            memory_size: 0,
        }
    }

    /// Gets a value from the cache.
    pub fn get<T: 'static + Send + Sync>(&self) -> CacheLookup<'_, T> {
        self.cache
            .get(&TypeId::of::<T>())
            .map(|v| {
                v.as_ref().map(|v| {
                    v.0.downcast_ref::<T>()
                        .expect("Invalid type in type map. This is a bug!")
                })
            })
            .into()
    }

    /// Sets a value in the cache.
    pub fn set<T: 'static + Send + Sync + BorshSerializedSize>(&mut self, value: T) {
        let type_id = TypeId::of::<T>();
        let size = value.serialized_size();
        let boxed = Box::new(value);
        let prev = self.cache.insert(type_id, Some((boxed, size)));
        if let Some(Some((_prev, prev_size))) = prev {
            self.memory_size -= prev_size;
        }
        if self.cache.len() > MAX_EXPECTED_CACHE_ITEMS {
            tracing::warn!(
                "Temporary value cache is getting large! This may result in degraded performance."
            );
        }
        self.memory_size += size;
        if self.memory_size > MAX_EXPECTED_CACHE_BYTES {
            tracing::warn!(
                "Temporary value cache is getting large! This may result in degraded performance."
            );
        }
    }

    /// Deletes a value from the cache.
    pub fn delete<T: 'static + Send + Sync>(&mut self) {
        let prev = self.cache.insert(TypeId::of::<T>(), None);
        if let Some(Some((_prev, size))) = prev {
            self.memory_size -= size;
        }
    }

    pub fn update_with(&mut self, other: Self) {
        for (key, value) in other.cache.into_iter() {
            let new_size = value.as_ref().map(|v| v.1).unwrap_or(0);
            if let Some(Some((_prev, prev_size))) = self.cache.insert(key, value) {
                self.memory_size -= prev_size;
            };
            self.memory_size += new_size;
        }
    }

    /// Prunes all `None` values from the cache.
    pub fn prune(&mut self) {
        self.cache.retain(|_, value| value.is_some());
    }
}

macro_rules! impl_borsh_serialized_size {
    ($(($t:ident, $size:expr)),*) => {
        $(
            impl BorshSerializedSize for $t {
                fn serialized_size(&self) -> usize {
                    $size
                }
            }
        )*
    };
}

impl_borsh_serialized_size!(
    (u8, 1),
    (u16, 2),
    (u32, 4),
    (u64, 8),
    (u128, 16),
    (bool, 1),
    (usize, 4),
    (i8, 1),
    (i16, 2),
    (i32, 4),
    (i64, 8),
    (i128, 16),
    (f32, 4),
    (f64, 8)
);

impl<const N: usize> BorshSerializedSize for [u8; N] {
    fn serialized_size(&self) -> usize {
        N
    }
}

impl BorshSerializedSize for Vec<u8> {
    fn serialized_size(&self) -> usize {
        self.len() + 4
    }
}

impl BorshSerializedSize for String {
    fn serialized_size(&self) -> usize {
        self.len() + 4
    }
}

impl BorshSerializedSize for &str {
    fn serialized_size(&self) -> usize {
        self.len() + 4
    }
}

impl<const N: usize> BorshSerializedSize for SizedSafeString<N> {
    fn serialized_size(&self) -> usize {
        self.len() + 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_cache() {
        let mut cache = TempCache::new();

        cache.set(1u8);
        assert_eq!(cache.get::<u8>(), CacheLookup::Hit(Some(&1u8)));
        assert_eq!(cache.memory_size, 1);

        cache.set(2u8);
        assert_eq!(cache.get::<u8>(), CacheLookup::Hit(Some(&2u8)));
        assert_eq!(cache.memory_size, 1);

        cache.set(3u16);
        assert_eq!(cache.get::<u16>(), CacheLookup::Hit(Some(&3u16)));
        assert_eq!(cache.memory_size, 3);

        cache.delete::<u8>();
        assert_eq!(cache.get::<u8>(), CacheLookup::Hit(None));
        assert_eq!(cache.memory_size, 2);

        assert_eq!(cache.get::<u16>(), CacheLookup::Hit(Some(&3u16)));
        cache.prune();
        assert_eq!(cache.get::<u8>(), CacheLookup::Miss);
        cache.set(11u32);

        let mut other = TempCache::new();
        other.set(4u8);
        other.set(5u64);
        other.delete::<u16>();

        cache.update_with(other);
        assert_eq!(cache.get::<u8>(), CacheLookup::Hit(Some(&4u8)));
        assert_eq!(cache.get::<u64>(), CacheLookup::Hit(Some(&5u64)));
        assert_eq!(cache.get::<u16>(), CacheLookup::Hit(None));
        assert_eq!(cache.get::<u32>(), CacheLookup::Hit(Some(&11u32)));
        assert_eq!(cache.memory_size, 13);
    }
}
