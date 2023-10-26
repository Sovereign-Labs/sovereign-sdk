use alloc::vec::Vec;

/// A [`Vec`] of bytes whose length is guaranteed to be aligned to 4 bytes.
/// This makes certain operations cheaper in zk-context (namely, concatenation).
// TODO: Currently the implementation defaults to `stc::vec::Vec` see:
// https://github.com/Sovereign-Labs/sovereign-sdk/issues/47
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct AlignedVec {
    inner: Vec<u8>,
}

impl AlignedVec {
    /// The length of the chunks of the aligned vector.
    pub const ALIGNMENT: usize = 4;

    /// Creates a new [`AlignedVec`] whose length is aligned to
    /// [`AlignedVec::ALIGNMENT`] bytes.
    pub fn new(vector: Vec<u8>) -> Self {
        Self { inner: vector }
    }

    /// Extends `self` with the contents of the other [`AlignedVec`].
    pub fn extend(&mut self, other: &Self) {
        // TODO check if the standard extend method does the right thing.
        // debug_assert_eq!(
        //     self.inner.len() % Self::ALIGNMENT,
        //     0,
        //     "`AlignedVec` is expected to have well-formed chunks"
        // );
        self.inner.extend(&other.inner);
    }

    /// Consumes `self` and returns the underlying [`Vec`] of bytes.
    pub fn into_inner(self) -> Vec<u8> {
        self.inner
    }

    /// Returns the length in bytes of the prefix.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the prefix is empty, `false` otherwise.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AsRef<Vec<u8>> for AlignedVec {
    fn as_ref(&self) -> &Vec<u8> {
        &self.inner
    }
}

#[cfg(feature = "arbitrary")]
mod arbitrary_impls {
    use arbitrary::{Arbitrary, Unstructured};
    use proptest::arbitrary::any;
    use proptest::strategy::{BoxedStrategy, Strategy};

    use super::*;

    impl<'a> Arbitrary<'a> for AlignedVec {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            u.arbitrary().map(|v: Vec<u8>| {
                // we re-allocate so the capacity is also guaranteed to be aligned
                Self::new(v[..(v.len() / Self::ALIGNMENT) * Self::ALIGNMENT].to_vec())
            })
        }
    }

    impl proptest::arbitrary::Arbitrary for AlignedVec {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            any::<Vec<u8>>()
                .prop_map(|v| {
                    Self::new(v[..(v.len() / Self::ALIGNMENT) * Self::ALIGNMENT].to_vec())
                })
                .boxed()
        }
    }
}
