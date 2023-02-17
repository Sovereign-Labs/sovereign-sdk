// AlignedVec keeps a vec whose length is guaranteed to be aligned to 4 bytes.
// This makes certain operations cheaper in zk-context (concatenation)
// TODO: Currently the implementation defaults to `stc::vec::Vec` see:
// https://github.com/Sovereign-Labs/sovereign/issues/47
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct AlignedVec {
    inner: Vec<u8>,
}

impl AlignedVec {
    // Creates a new AlignedVec whose length is aligned to 4 bytes.
    pub fn new(vector: Vec<u8>) -> Self {
        // TODO pad the vector to
        Self { inner: vector }
    }

    // Extends self with the contents of the other AlignedVec.
    pub fn extend(&mut self, other: &Self) {
        // TODO check if the standard extend method does the right thing.
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
