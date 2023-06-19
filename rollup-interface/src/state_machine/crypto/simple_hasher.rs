pub use jmt::SimpleHasher;

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
