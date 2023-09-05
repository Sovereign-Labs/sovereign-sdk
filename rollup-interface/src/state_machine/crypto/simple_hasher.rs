use digest::typenum::U32;
use digest::{Digest, FixedOutput, FixedOutputReset, OutputSizeUser, Reset, Update};

/// A [`digest::Digest`] implementation which always returns the digest
/// `[0;32]`.
pub struct NoOpHasher;

impl OutputSizeUser for NoOpHasher {
    type OutputSize = U32;
}

impl Update for NoOpHasher {
    fn update(&mut self, _data: &[u8]) {}
}

impl Reset for NoOpHasher {
    fn reset(&mut self) {}
}

impl FixedOutput for NoOpHasher {
    fn finalize_into(self, out: &mut digest::Output<Self>) {
        *out = [0u8; 32].into();
    }
}

impl FixedOutputReset for NoOpHasher {
    fn finalize_into_reset(&mut self, out: &mut digest::Output<Self>) {
        *out = [0u8; 32].into();
    }
}

impl Digest for NoOpHasher {
    fn new() -> Self {
        Self
    }

    fn new_with_prefix(_data: impl AsRef<[u8]>) -> Self {
        Self
    }

    fn update(&mut self, _data: impl AsRef<[u8]>) {}

    fn chain_update(self, _data: impl AsRef<[u8]>) -> Self {
        Self
    }

    fn finalize(self) -> digest::Output<Self> {
        [0u8; 32].into()
    }

    fn finalize_into(self, out: &mut digest::Output<Self>) {
        <Self as FixedOutput>::finalize_into(self, out)
    }

    fn finalize_reset(&mut self) -> digest::Output<Self>
    where
        Self: digest::FixedOutputReset,
    {
        [0u8; 32].into()
    }

    fn finalize_into_reset(&mut self, out: &mut digest::Output<Self>)
    where
        Self: digest::FixedOutputReset,
    {
        <Self as FixedOutputReset>::finalize_into_reset(self, out)
    }

    fn reset(&mut self)
    where
        Self: digest::Reset,
    {
        <Self as Reset>::reset(self)
    }

    fn output_size() -> usize {
        32
    }

    fn digest(_data: impl AsRef<[u8]>) -> digest::Output<Self> {
        [0u8; 32].into()
    }
}
