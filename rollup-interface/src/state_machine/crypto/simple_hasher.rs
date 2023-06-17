use digest::{generic_array::GenericArray, Digest, OutputSizeUser};

/// A minimal trait representing a hash function. We implement our own
/// rather than relying on `Digest` for broader compatibility.
pub trait SimpleHasher: Sized {
    /// Creates a new hasher with default state.
    fn new() -> Self;
    /// Ingests the provided data, updating the hasher's state.
    fn update(&mut self, data: &[u8]);
    /// Consumes the hasher state to produce a digest.
    fn finalize(self) -> [u8; 32];
    /// Returns the digest of the provided data.
    fn hash(data: impl AsRef<[u8]>) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(data.as_ref());
        hasher.finalize()
    }
}

/// A SimpleHasher implementation which always returns the digest [0;32]
pub struct NoOpHasher;
impl SimpleHasher for NoOpHasher {
    fn new() -> Self {
        Self
    }

    fn update(&mut self, _data: &[u8]) {}

    fn finalize(self) -> [u8; 32] {
        [0u8; 32]
    }
}

// Blanekt implement SimpleHasher for the rust-crypto hashers
impl<T: Digest> SimpleHasher for T
where
    [u8; 32]: From<GenericArray<u8, <T as OutputSizeUser>::OutputSize>>,
{
    fn new() -> Self {
        <T as Digest>::new()
    }

    fn update(&mut self, data: &[u8]) {
        self.update(data)
    }

    fn finalize(self) -> [u8; 32] {
        self.finalize().into()
    }
}
