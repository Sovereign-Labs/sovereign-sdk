use std::io;

use crate::gas::traits::GasMeter;
use crate::{as_u32_or_panic, GasMeteringError, GasSpec, Spec};

/// `io::Read` adapter that charges gas to a `GasMeter` for every byte read.
///
/// Wraps an inner `Read` source and a meter; every `read` and `read_exact` call charges
/// `per_byte × n + per_read_bias`. If the meter exhausts mid-decode, the typed
/// `GasMeteringError` is stashed and a synthetic `io::Error` is returned. Call
/// [`MeteredReader::take_stashed_error`] after the decode bubbles the error up to
/// recover the typed gas error.
///
/// Used by `MeteredBorshDeserialize::deserialize_from_slice` to convert byte-based
/// gas charging into work-based charging that scales with the number of bytes the
/// deserializer actually pulls.
pub struct MeteredReader<'a, R: io::Read, M: GasMeter> {
    inner: R,
    meter: &'a mut M,
    per_byte: <M::Spec as Spec>::Gas,
    per_read_bias: <M::Spec as Spec>::Gas,
    stashed: Option<GasMeteringError<<M::Spec as Spec>::Gas>>,
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
            stashed: None,
        }
    }

    /// Take the stashed typed gas error if a charge failed during a read.
    ///
    /// After borsh's `deserialize_reader` returns an `io::Error`, call this to
    /// distinguish "out of gas" from "genuine IO error" — only out-of-gas charges
    /// stash here.
    pub fn take_stashed_error(&mut self) -> Option<GasMeteringError<<M::Spec as Spec>::Gas>> {
        self.stashed.take()
    }

    fn charge(&mut self, n: usize) -> io::Result<()> {
        if let Err(e) = self.meter.charge_gas(self.per_read_bias) {
            self.stashed = Some(e);
            return Err(io::Error::new(io::ErrorKind::Other, "out of gas"));
        }
        if let Err(e) = self
            .meter
            .charge_linear_gas(self.per_byte, as_u32_or_panic(n))
        {
            self.stashed = Some(e);
            return Err(io::Error::new(io::ErrorKind::Other, "out of gas"));
        }
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
