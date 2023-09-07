// AlignedVec keeps a vec whose length is guaranteed to be aligned to 4 bytes.
// This makes certain operations cheaper in zk-context (concatenation)
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

    // Creates a new AlignedVec whose length is aligned to [Self::ALIGNMENT] bytes.
    pub fn new(vector: Vec<u8>) -> Self {
        Self { inner: vector }
    }

    // Extends self with the contents of the other AlignedVec.
    pub fn extend(&mut self, other: &Self) {
        // TODO check if the standard extend method does the right thing.
        // debug_assert_eq!(
        //     self.inner.len() % Self::ALIGNMENT,
        //     0,
        //     "`AlignedVec` is expected to have well-formed chunks"
        // );
        self.inner.extend(&other.inner);
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

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
impl<'a> arbitrary::Arbitrary<'a> for AlignedVec {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        u.arbitrary().map(Self::new)
    }
}
