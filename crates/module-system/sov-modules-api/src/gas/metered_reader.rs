use std::io;

use crate::gas::traits::GasMeter;
use crate::{as_u32_or_panic, GasSpec, Spec};

/// `io::Read` adapter that charges gas to a `GasMeter` for every byte read.
///
/// Wraps an inner `Read` source and a meter; every `read` and `read_exact` call charges
/// `per_byte × n + per_read_bias`. A failed charge wraps the typed `GasMeteringError`
/// inside the returned `io::Error` (via [`io::Error::other`]); the caller recovers it
/// with [`io::Error::downcast`].
pub struct MeteredReader<'a, R: io::Read, M: GasMeter> {
    inner: R,
    meter: &'a mut M,
    per_byte: <M::Spec as Spec>::Gas,
    per_read_bias: <M::Spec as Spec>::Gas,
}

impl<'a, R: io::Read, M: GasMeter> MeteredReader<'a, R, M> {
    /// Wrap `inner` in a metered reader using the per-byte and per-read constants
    /// from the meter's `GasSpec`.
    pub fn new(inner: R, meter: &'a mut M) -> Self {
        Self::new_with_prices(
            inner,
            meter,
            <M::Spec as GasSpec>::gas_to_charge_per_byte_borsh_read(),
            <M::Spec as GasSpec>::bias_borsh_per_read(),
        )
    }

    /// Construct a metered reader with explicit prices. Used for testing.
    pub fn new_with_prices(
        inner: R,
        meter: &'a mut M,
        per_byte: <M::Spec as Spec>::Gas,
        per_read_bias: <M::Spec as Spec>::Gas,
    ) -> Self {
        Self {
            inner,
            meter,
            per_byte,
            per_read_bias,
        }
    }

    fn charge(&mut self, n: usize) -> io::Result<()> {
        self.meter
            .charge_gas(self.per_read_bias)
            .map_err(io::Error::other)?;
        self.meter
            .charge_linear_gas(self.per_byte, as_u32_or_panic(n))
            .map_err(io::Error::other)?;
        Ok(())
    }
}

impl<R: io::Read, M: GasMeter> io::Read for MeteredReader<'_, R, M> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Charge after the inner read so partial reads / EOF / inner failures
        // don't over-charge for bytes that were not delivered.
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.charge(n)?;
        }
        Ok(n)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        // Must override the default `Read::read_exact` impl. `<&[u8] as Read>::read_exact`
        // is specialised to a direct memcpy that bypasses `Read::read`, so leaving the
        // default would silently skip metering against `&[u8]` sources — which is
        // exactly the source borsh's slice-based deserialize feeds in. The
        // `read_exact_meters_against_slice` unit test catches a missing override.
        self.inner.read_exact(buf)?;
        self.charge(buf.len())
    }
}
