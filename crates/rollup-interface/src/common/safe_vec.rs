//! Defines a length-limited `Vec` type. This type is a heap-allocated variable-length
//! container like [`std::vec::Vec`] except that it has a max capacity
//! set at compile time.
//!
//! The API of the fallible operations  inspired by the [arrayvec](https://docs.rs/arrayvec/latest/arrayvec/struct.ArrayVec.html) crate.

use std::borrow::Cow;
use std::fmt;
// Optional items to finish in this crate:
// - Implement serde::deserialize_in_place
// - Implement Borrow and BorrowMut
// - Implement missing methods from Vec API
// - Implement additional missing traits from Vec API
use std::marker::PhantomData;
use std::ops::{Index, IndexMut, RangeBounds};
use std::slice::SliceIndex;
use std::vec::Drain;

use borsh::{BorshDeserialize, BorshSerialize};
use schemars::gen::SchemaGenerator;
use schemars::JsonSchema;
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use sov_universal_wallet::schema::{OverrideSchema, UniversalWallet as SovSchemaGenerator};
use thiserror::Error;

/// A wrapper around `Vec` which guarantees that the length of the `Vec` will never
/// exceed MAX_SIZE _elements_. If the feature `vec-compat` is disabled,
/// SafeVec also guarantees that its underying `Vec` will never request an allocation larger than
/// `MAX_SIZE`.
///
/// When the `vec-compat` feature is disabled, SafeVec guarantees that its methods will never panic because
/// the vec is out of capacity. In other words, SafeVec only panics in circumstances that would also cause a `Vec`
/// to panic, such as out-of-bounds access.
///
/// When `vec-compat` is enabled, SafeVec provides a much closer approximation to the `Vec` api at the expense of...
///  - potentially over-allocating in some cases
///  - panicking when an infallible method from the `Vec` API tries to grow the `SafeVec` beyond its max capacity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, BorshSerialize)]
pub struct SafeVec<T, const MAX_SIZE: usize> {
    contents: Vec<T>,
    phantom: PhantomData<[(); MAX_SIZE]>,
}

#[cfg(feature = "arbitrary")]
impl<'a, T: arbitrary::Arbitrary<'a>, const MAX_SIZE: usize> arbitrary::Arbitrary<'a>
    for SafeVec<T, MAX_SIZE>
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let len = std::cmp::min(u.arbitrary_len::<T>()?, MAX_SIZE);
        let mut out =
            SafeVec::try_with_capacity(len).expect("Length has been checked to be <= max size");
        for _ in 0..len {
            let _ = out.try_push(T::arbitrary(u)?);
        }
        Ok(out)
    }
}

impl<T: SovSchemaGenerator, const MAX_SIZE: usize> OverrideSchema for SafeVec<T, MAX_SIZE> {
    type Output = Vec<T>;
}

impl<T: JsonSchema, const MAX_SIZE: usize> JsonSchema for SafeVec<T, MAX_SIZE> {
    fn schema_name() -> String {
        format!("SafeVec_{}_of_{}", MAX_SIZE, T::schema_name())
    }

    fn schema_id() -> Cow<'static, str> {
        format!("SafeVec<{}, {}>", MAX_SIZE, T::schema_id()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "array",
            "items": generator.subschema_for::<T>(),
            "minItems": 0,
            "maxItems": MAX_SIZE,
        }))
        .unwrap()
    }
}

impl<T, const MAX_SIZE: usize> Default for SafeVec<T, MAX_SIZE> {
    fn default() -> Self {
        Self {
            contents: Vec::new(),
            phantom: Default::default(),
        }
    }
}

/// The error returned when a SafeVec is out of capacity. When applicable, it contains
/// the item that could not be pushed to the `SafeVec`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Error, Hash)]
#[error("Not enough capacity")]
pub struct CapacityError<T = ()>(pub T);

impl<T> CapacityError<T> {
    /// Creates a new `CapacityError`
    pub fn new(item: T) -> Self {
        CapacityError(item)
    }

    /// Replaces the contents of the capacity error with a new value
    pub fn with<U>(self, new_contents: U) -> CapacityError<U> {
        CapacityError(new_contents)
    }
}

impl<T, const MAX_SIZE: usize> From<SafeVec<T, MAX_SIZE>> for Vec<T> {
    fn from(vec: SafeVec<T, MAX_SIZE>) -> Vec<T> {
        vec.contents
    }
}

impl<T: Clone, const MAX_SIZE: usize> TryFrom<&[T]> for SafeVec<T, MAX_SIZE> {
    type Error = CapacityError;

    fn try_from(value: &[T]) -> std::result::Result<Self, Self::Error> {
        if value.len() > MAX_SIZE {
            return Err(CapacityError(()));
        }
        let contents = Vec::from(value);
        Ok(Self {
            contents,
            phantom: PhantomData,
        })
    }
}

impl<T, const MAX_SIZE: usize> TryFrom<Vec<T>> for SafeVec<T, MAX_SIZE> {
    type Error = CapacityError;

    fn try_from(value: Vec<T>) -> std::result::Result<Self, Self::Error> {
        if value.len() > MAX_SIZE {
            return Err(CapacityError(()));
        }
        Ok(Self {
            contents: value,
            phantom: PhantomData,
        })
    }
}

impl<T, const MAX_SIZE: usize> std::ops::Deref for SafeVec<T, MAX_SIZE> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        self.contents.deref()
    }
}

impl<T, const MAX_SIZE: usize> std::ops::DerefMut for SafeVec<T, MAX_SIZE> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.contents.deref_mut()
    }
}

impl<T, const MAX_SIZE: usize> IntoIterator for SafeVec<T, MAX_SIZE> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.contents.into_iter()
    }
}

impl<'a, T, const MAX_SIZE: usize> IntoIterator for &'a SafeVec<T, MAX_SIZE> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.contents.iter()
    }
}

impl<'a, T, const MAX_SIZE: usize> IntoIterator for &'a mut SafeVec<T, MAX_SIZE> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.contents.iter_mut()
    }
}

impl<T, I: SliceIndex<[T]>, const MAX_SIZE: usize> Index<I> for SafeVec<T, MAX_SIZE> {
    type Output = I::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        self.contents.index(index)
    }
}

impl<T, I: SliceIndex<[T]>, const MAX_SIZE: usize> IndexMut<I> for SafeVec<T, MAX_SIZE> {
    #[inline]
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        self.contents.index_mut(index)
    }
}

impl<T, const MAX_SIZE: usize> AsRef<[T]> for SafeVec<T, MAX_SIZE> {
    fn as_ref(&self) -> &[T] {
        self.contents.as_ref()
    }
}

/// Collects an iterator into a SafeVec, panicking if the required capacity exceeds MAX_SIZE
/// This method is commonly called via [`Iterator::collect()`].
///
/// Unlike  method *may* allocate more than delegates to the underlying `Vec::from_iter` implementation`
///
#[cfg(feature = "vec-compat")]
impl<T, const MAX_SIZE: usize> FromIterator<T> for SafeVec<T, MAX_SIZE> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let contents = Vec::from_iter(iter);
        contents
            .try_into()
            .expect("Tried to build a safe vec from an iter with more than MAX_SIZE items")
    }
}

/// A type alias for Result<(), CapacityError>
pub type Result<T = (), E = CapacityError> = std::result::Result<T, E>;

impl<T, const MAX_SIZE: usize> SafeVec<T, MAX_SIZE> {
    /// Clears the vector, removing all values.
    ///
    /// Note that this method has no effect on the allocated capacity of the vector.
    #[inline]
    pub fn clear(&mut self) {
        self.contents.clear();
    }

    /// Moves all the elements of `other` into `self`, leaving `other` empty.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `MAX_SIZE` elements or `isize::MAX` _bytes_.
    #[cfg(feature = "vec-compat")]
    #[inline]
    pub fn append(&mut self, other: &mut Vec<T>) {
        self.try_append(other)
            .expect("Append tried to exceed capacity of SafeVec");
    }

    /// Moves all the elements of `other` into `self`, leaving `other` empty.
    #[inline]
    pub fn try_append(&mut self, other: &mut Vec<T>) -> Result {
        self.try_reserve(other.len())?;
        self.contents.append(other);
        Ok(())
    }

    /// Tries to reserve capacity for at least `additional` more elements to be inserted
    /// in the given `SafeVec`. The collection may reserve more space to speculatively avoid
    /// frequent reallocations. After calling `try_reserve`, capacity will be
    /// greater than or equal to `self.len() + additional` if it returns
    /// `Ok(())`. Does nothing if capacity is already sufficient. This method
    /// preserves the contents even if an error occurs.
    ///
    /// # Errors
    ///
    /// If the capacity overflows, or the allocator reports a failure, then an error
    /// is returned.
    pub fn try_reserve(&mut self, items_to_add: usize) -> Result<(), CapacityError> {
        let new_len = self.len().saturating_add(items_to_add);
        // If we would exceed max size, return an error immediately
        if new_len > MAX_SIZE {
            return Err(CapacityError(()));
        }

        // If we already have capacity, return early
        if new_len <= self.capacity() {
            return Ok(());
        }

        // If the request less than doubles the capacity, double it. Otherwise, increase to exactly the requested size.
        // If that amount would exceed MAX_SIZE, allocate max size instead. We know this covers at least items_to_add
        // because of the check above
        let amount_to_reserve = std::cmp::max(
            new_len,
            std::cmp::min(MAX_SIZE, self.capacity() * 2),
        ).checked_sub(self.len()).expect("Unexpected underflow: tried to reserve negative capacity after checking that capacity was positive. This is a bug");
        self.contents
            .try_reserve_exact(amount_to_reserve)
            .map_err(|_| CapacityError(()))?;
        Ok(())
    }

    /// Constructs a new, empty `SafeVec` with at least the specified capacity.
    ///
    /// The vector will be able to hold at least `capacity` elements without
    /// reallocating. This method is allowed to allocate for more elements than
    /// `capacity`. If `capacity` is 0, the vector will not allocate.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` _bytes_ or `MAX_SIZE`.
    #[inline]
    #[must_use]
    #[cfg(feature = "vec-compat")]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::try_with_capacity(capacity).expect("with_capacity tried to exceed MAX_SIZE")
    }

    /// Constructs a new, empty `SafeVec` with at least the specified capacity.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` _bytes_
    #[inline]
    pub fn try_with_capacity(capacity: usize) -> Result<Self> {
        let mut output = Self::new();
        output.try_reserve(capacity)?;
        Ok(output)
    }

    /// Tries to reserve the minimum capacity for at least `additional`
    /// elements to be inserted in the given `SafeVec`. Unlike [`try_reserve`],
    /// this will not deliberately over-allocate to speculatively avoid frequent
    /// allocations. After calling `try_reserve_exact`, capacity will be greater
    /// than or equal to `self.len() + additional` if it returns `Ok(())`.
    /// Does nothing if the capacity is already sufficient.
    ///
    /// Note that the allocator may give the collection more space than it
    /// requests. Therefore, capacity can not be relied upon to be precisely
    /// minimal. Prefer [`try_reserve`] if future insertions are expected.
    ///
    /// [`try_reserve`]: SafeVec::try_reserve
    ///
    /// # Errors
    ///
    /// If the capacity overflows, or the allocator reports a failure, then an error
    /// is returned.
    pub fn try_reserve_exact(&mut self, additional: usize) -> Result {
        if self.len().saturating_add(additional) > MAX_SIZE {
            return Err(CapacityError(()));
        }
        self.contents
            .try_reserve_exact(additional)
            .map_err(|_| CapacityError(()))?;
        Ok(())
    }

    #[cfg(feature = "vec-compat")]
    fn remaining_max_capacity(&self) -> Option<usize> {
        MAX_SIZE.checked_sub(self.len())
    }

    /// Returns the total number of elements the vector can hold without reallocating.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.contents.capacity()
    }

    /// Returns the number of additional elements the vector can hold without reallocating.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.capacity() - self.len()
    }

    /// Removes all but the first of consecutive elements in the vector satisfying a given equality
    /// relation.
    ///
    /// The `same_bucket` function is passed references to two elements from the vector and
    /// must determine if the elements compare equal. The elements are passed in opposite order
    /// from their order in the slice, so if `same_bucket(a, b)` returns `true`, `a` is removed.
    ///
    /// If the vector is sorted, this removes all duplicates.
    pub fn dedup_by<F>(&mut self, same_bucket: F)
    where
        F: FnMut(&mut T, &mut T) -> bool,
    {
        self.contents.dedup_by(same_bucket);
    }

    /// Removes all but the first of consecutive elements in the vector that resolve to the same
    /// key.
    ///
    /// If the vector is sorted, this removes all duplicates.
    #[inline]
    pub fn dedup_by_key<F, K>(&mut self, key: F)
    where
        F: FnMut(&mut T) -> K,
        K: PartialEq,
    {
        self.contents.dedup_by_key(key);
    }

    /// Removes the specified range from the vector in bulk, returning all
    /// removed elements as an iterator. If the iterator is dropped before
    /// being fully consumed, it drops the remaining removed elements.
    ///
    /// The returned iterator keeps a mutable borrow on the vector to optimize
    /// its implementation.
    ///
    /// # Panics
    ///
    /// Panics if the starting point is greater than the end point or if
    /// the end point is greater than the length of the vector.
    ///
    /// # Leaking
    ///
    /// If the returned iterator goes out of scope without being dropped (due to
    /// [`std::mem::forget`], for example), the vector may have lost and leaked
    /// elements arbitrarily, including elements outside the range.
    pub fn drain<R>(&mut self, range: R) -> Drain<'_, T>
    where
        R: RangeBounds<usize>,
    {
        self.contents.drain(range)
    }

    /// Inserts an element at position `index` within the vector, shifting all
    /// elements after it to the right.
    ///
    /// # Panics
    ///
    /// Panics if `index > len` or `len + 1 > MAX_SIZE`
    ///
    /// # Time complexity
    ///
    /// Takes *O*(`len`) time. All items after the insertion index must be
    /// shifted to the right. In the worst case, all elements are shifted when
    /// the insertion index is 0.
    #[cfg(feature = "vec-compat")]
    pub fn insert(&mut self, index: usize, element: T) {
        self.try_insert(index, element)
            .map_err(|_| ())
            .expect("Inserting caused safe vec to exceed its max capacity");
    }

    /// Tries to insert an element at position `index` within the vector, shifting all
    /// elements after it to the right. Returns the unused item on failure.
    ///
    /// # Time complexity
    ///
    /// Takes *O*(`len`) time. All items after the insertion index must be
    /// shifted to the right. In the worst case, all elements are shifted when
    /// the insertion index is 0.
    pub fn try_insert(&mut self, index: usize, element: T) -> Result<(), CapacityError<T>> {
        if self.try_reserve(1).is_err() || index > self.len() {
            return Err(CapacityError(element));
        }
        self.contents.insert(index, element);
        Ok(())
    }

    /// Appends an element to the back of a collection.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` _bytes_ or if the new length
    /// would exceed `MAX_SIZE`.
    ///
    /// # Time complexity
    ///
    /// Takes amortized *O*(1) time. If the vector's length would exceed its
    /// capacity after the push, *O*(*capacity*) time is taken to copy the
    /// vector's elements to a larger allocation. This expensive operation is
    /// offset by the *capacity* *O*(1) insertions it allows.
    #[inline]
    #[cfg(feature = "vec-compat")]
    pub fn push(&mut self, value: T) {
        self.try_push(value)
            .map_err(|_| ())
            .expect("Push exceeded safe vec capacity");
    }

    /// Appends an element to the back of a collection.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` _bytes_
    ///
    /// # Time complexity
    ///
    /// Takes amortized *O*(1) time. If the vector's length would exceed its
    /// capacity after the push, *O*(*capacity*) time is taken to copy the
    /// vector's elements to a larger allocation. This expensive operation is
    /// offset by the *capacity* *O*(1) insertions it allows.
    #[inline]
    pub fn try_push(&mut self, value: T) -> Result<(), CapacityError<T>> {
        // Use if let instead of map_err to avoid moving value
        if let Err(e) = self.try_reserve(1) {
            return Err(e.with(value));
        }
        self.push_unchecked(value);
        Ok(())
    }

    /// Appends an element to the back of a collection without checking capacity.
    /// This method could violate the `SafeVec` invariant if used incorrectly!
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` _bytes_.
    #[inline]
    fn push_unchecked(&mut self, item: T) {
        self.contents.push(item);
    }

    /// Constructs a new, empty `SafeVec<T, MAX_SIZE>`.
    ///
    /// The vector will not allocate until elements are pushed onto it.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            contents: Vec::new(),
            phantom: PhantomData,
        }
    }

    /// A convenince function to return the max size of a `SafeVec`
    pub const fn max_size(&self) -> usize {
        MAX_SIZE
    }

    /// Retains only the elements specified by the predicate.
    ///
    /// In other words, remove all elements `e` for which `f(&e)` returns `false`.
    /// This method operates in place, visiting each element exactly once in the
    /// original order, and preserves the order of the retained elements.
    ///
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.contents.retain(|elem| f(elem));
    }

    /// Shortens the vector, keeping the first `len` elements and dropping
    /// the rest.
    ///
    /// If `len` is greater or equal to the vector's current length, this has
    /// no effect.
    ///
    /// The `drain` method can emulate `truncate`, but causes the excess
    /// elements to be returned instead of dropped.
    ///
    /// Note that this method has no effect on the allocated capacity
    /// of the vector.
    pub fn truncate(&mut self, len: usize) {
        self.contents.truncate(len);
    }
}

impl<T: PartialEq, const MAX_SIZE: usize> SafeVec<T, MAX_SIZE> {
    /// Removes consecutive repeated elements in the vector according to the
    /// [`PartialEq`] trait implementation.
    ///
    /// If the vector is sorted, this removes all duplicates.
    pub fn dedup(&mut self) {
        self.contents.dedup();
    }
}

#[cfg(feature = "vec-compat")]
impl<T, const N: usize> Extend<T> for SafeVec<T, N> {
    /// Extend the `SafeVec` with the contents of an iterator.
    ///
    /// # Panics
    ///
    /// Panics if extending the vector exceeds its capacity.
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        let mut iter = iter.into_iter();
        self.try_reserve(iter.size_hint().0)
            .expect("extend tried to exceed max size of SafeVec");
        for _ in 0..self.remaining_max_capacity().unwrap() {
            if let Some(item) = iter.next() {
                self.push_unchecked(item);
            } else {
                return;
            }
        }
        assert!(
            iter.next().is_none(),
            "extend tried to exceed max size of SafeVec"
        );
    }
}

impl<T: Clone, const N: usize> SafeVec<T, N> {
    /// Clones and appends all elements in a slice to the `SafeVec`.
    ///
    /// Iterates over the slice `other`, clones each element, and then appends
    /// it to this `SafeVec`. The `other` slice is traversed in-order.
    ///
    /// Note that this function is same as `extend` except that it is
    /// specialized to work with slices instead. If and when Rust gets
    /// specialization this function will likely be deprecated (but still
    /// available).
    ///
    /// # Panics
    ///
    /// Panics if the resulting vector would exceed MAX_SIZE
    #[cfg(feature = "vec-compat")]
    pub fn extend_from_slice(&mut self, other: &[T]) {
        self.try_extend_from_slice(other)
            .expect("extend_from_slice tried to exceed the max size of a SafeVec");
    }

    /// Clones and appends all elements in a slice to the `SafeVec`.
    ///
    /// Iterates over the slice `other`, clones each element, and then appends
    /// it to this `SafeVec`. The `other` slice is traversed in-order.
    ///
    /// Note that this function is same as `extend` except that it is
    /// specialized to work with slices instead. If and when Rust gets
    /// specialization this function will likely be deprecated (but still
    /// available).
    pub fn try_extend_from_slice(&mut self, other: &[T]) -> Result {
        self.try_reserve(other.len())?;
        self.contents.extend_from_slice(other);
        Ok(())
    }

    /// Copies elements from `src` range to the end of the vector.
    ///
    /// # Panics
    ///
    /// Panics if the starting point is greater than the end point, if
    /// the end point is greater than the length of the vector, or if
    /// extending the vector would cause it to exceed its max capacity
    #[cfg(feature = "vec-compat")]
    pub fn extend_from_within<R>(&mut self, src: R)
    where
        R: RangeBounds<usize>,
    {
        self.try_extend_from_within(src)
            .expect("extend_from_within exceeded the max size of a safe vec");
    }

    /// Copies elements from `src` range to the end of the vector.
    ///
    /// # Panics
    ///
    /// Panics if the starting point is greater than the end point, if
    /// the end point is greater than the length of the vector
    pub fn try_extend_from_within<R>(&mut self, src: R) -> Result
    where
        R: RangeBounds<usize>,
    {
        let mut to_add = 0;
        let start = match src.start_bound() {
            std::ops::Bound::Included(&start) => {
                to_add += 1;
                start
            }
            std::ops::Bound::Excluded(&start) => start,
            std::ops::Bound::Unbounded => 0,
        };
        let end = match src.end_bound() {
            std::ops::Bound::Included(&end) => {
                to_add += 1;
                end
            }
            std::ops::Bound::Excluded(&end) => end,
            std::ops::Bound::Unbounded => self.len(),
        };
        let needed_elements = end.checked_sub(start).expect(
            "try_extend_from_slice: Invalid range; start must be greater than or equal to end",
        );
        needed_elements
            .checked_add(to_add)
            .ok_or(CapacityError(()))?;
        self.try_reserve(needed_elements)?;
        self.contents.extend_from_within(src);
        Ok(())
    }

    /// Extend the `SafeVec` with the contents of an iterator.
    pub fn try_extend<I: IntoIterator<Item = T>>(&mut self, iter: I) -> Result {
        let iter = iter.into_iter();
        self.try_reserve(iter.size_hint().0)?;
        for item in iter {
            self.try_push(item).map_err(|_| CapacityError(()))?;
        }
        Ok(())
    }
}

impl<T: Serialize, const N: usize> serde::Serialize for SafeVec<T, N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Delegate serialization to the inner Vec
        self.contents.serialize(serializer)
    }
}

fn cautious_num_elts<T>() -> usize {
    const KILOBYTE: usize = 1024;
    (32 * KILOBYTE) / std::mem::size_of::<T>()
}

fn cautious_size_hint<T>(size_hint: usize) -> usize {
    std::cmp::min(cautious_num_elts::<T>(), size_hint)
}

const ERROR_ZST_FORBIDDEN: &str = "Collections of zero-sized types are not allowed by borsh due to denial-of-service concerns on deserialization.";

impl<T: BorshDeserialize, const MAX_SIZE: usize> BorshDeserialize for SafeVec<T, MAX_SIZE> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        if std::mem::size_of::<T>() == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                ERROR_ZST_FORBIDDEN,
            ));
        }

        let len = u32::deserialize_reader(reader)? as usize;
        if len > MAX_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unexpected length of input",
            ));
        }

        let mut output = SafeVec::<T, MAX_SIZE>::try_with_capacity(cautious_size_hint::<T>(len))
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::OutOfMemory,
                    "Out of memory! Unable to allocate capacity",
                )
            })?;

        let mut remaining_iters = len;
        while remaining_iters > 0 {
            for _ in 0..std::cmp::min(output.remaining_capacity(), remaining_iters - 1) {
                output.push_unchecked(T::deserialize_reader(reader)?);
                remaining_iters -= 1;
            }
            output
                .try_push(T::deserialize_reader(reader)?)
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::OutOfMemory,
                        "Out of memory! Unable to allocate capacity",
                    )
                })?;
            remaining_iters -= 1;
        }
        Ok(output)
    }
}

// Custom deserialization implementation
impl<'de, T: Deserialize<'de>, const N: usize> serde::Deserialize<'de> for SafeVec<T, N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct VecVisitor<T, const N: usize> {
            marker: PhantomData<(T, [(); N])>,
        }

        impl<'de, T, const N: usize> Visitor<'de> for VecVisitor<T, N>
        where
            T: Deserialize<'de>,
        {
            type Value = SafeVec<T, N>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sequence")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
                A::Error: serde::de::Error,
            {
                let mut output = SafeVec::<T, N>::new();
                let max_iters = if let Some(size_hint) = seq.size_hint() {
                    if size_hint > N {
                        return Err(<A::Error as serde::de::Error>::invalid_length(
                            size_hint,
                            &format!("at most {N} items").as_str(),
                        ));
                    }
                    output
                        .try_reserve(cautious_size_hint::<T>(size_hint))
                        .map_err(|_e| {
                            <A::Error as serde::de::Error>::invalid_length(
                                size_hint,
                                &format!(
                                    "to be able to allocate capacity for {} items. This is an OOM error which does not necessarily a problem with your particular input - it could be that other parts of the system are using too much memory",
                                    cautious_size_hint::<T>(size_hint)
                                )
                                .as_str(),
                            )
                        })?;
                    size_hint
                } else {
                    N
                };

                let mut max_remaining_iters = max_iters;
                while max_remaining_iters > 0 {
                    for _ in 0..std::cmp::min(output.remaining_capacity(), max_remaining_iters - 1)
                    {
                        if let Some(value) = seq.next_element()? {
                            max_remaining_iters -= 1;
                            output.push_unchecked(value);
                        } else {
                            return Ok(output);
                        }
                    }
                    if let Some(value) = seq.next_element()? {
                        max_remaining_iters -= 1;
                        output.try_push(value).map_err(|_e| {
                            <A::Error as serde::de::Error>::invalid_length(
                                max_iters - max_remaining_iters,
                                &"to be able to allocate capacity for all items.  This is an OOM error which does not necessarily a problem with your particular input - it could be that other parts of the system are using too much memory",
                            )
                        })?;
                    } else {
                        return Ok(output);
                    }
                }
                let overflowing_item: Option<T> = seq.next_element()?;
                if overflowing_item.is_some() {
                    return Err(serde::de::Error::invalid_length(
                        N + 1,
                        &format!("expected at most {N} items").as_str(),
                    ));
                }

                Ok(output)
            }
        }

        let visitor = VecVisitor {
            marker: PhantomData,
        };
        deserializer.deserialize_seq(visitor)
    }
}
