#[derive(Debug, PartialEq, Eq)]
pub struct AlignedVec {
    inner: Vec<u8>,
}

impl AlignedVec {
    pub fn new(vector: Vec<u8>) -> Self {
        Self { inner: vector }
    }

    pub fn extend(&mut self, other: &Self) {
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
