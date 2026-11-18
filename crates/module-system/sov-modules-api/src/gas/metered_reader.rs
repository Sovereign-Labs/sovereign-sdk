use std::io;

use crate::gas::traits::{GasArray, GasMeter};
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
    /// When set, reads are counted as `(reads, bytes)` instead of charged; see
    /// [`MeteredReader::charge_deferred`].
    deferred: Option<(u64, u64)>,
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
    pub(crate) fn new_with_prices(
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
            deferred: None,
        }
    }

    /// A reader that only counts what it would charge. [`MeteredReader::charge_deferred`] then
    /// charges it in two calls instead of two per read.
    pub(crate) fn new_deferred(inner: R, meter: &'a mut M) -> Self {
        Self {
            deferred: Some((0, 0)),
            ..Self::new(inner, meter)
        }
    }

    /// Charges what a reader from [`MeteredReader::new_deferred`] counted. Returns `false`, with
    /// the meter unchanged, if the meter doesn't expose its [`BasicGasMeter`](crate::BasicGasMeter)
    /// or the charge fails; the caller then decodes again with a charging reader, which reproduces
    /// the exact sequence of charges, including where it runs out of gas.
    ///
    /// Gas and its value are linear and the meter only changes on a successful charge, so when
    /// these charges succeed the meter ends in the same state as after the per-read charges.
    pub(crate) fn charge_deferred(&mut self) -> bool {
        let Some((reads, bytes)) = self.deferred else {
            return true;
        };
        let Some(snapshot) = self.meter.try_as_basic_gas_meter().cloned() else {
            return false;
        };
        let (Some(bias), Ok(bytes)) = (
            self.per_read_bias.checked_scalar_product(reads),
            u32::try_from(bytes),
        ) else {
            return false;
        };
        let charged = self.meter.charge_gas(bias).is_ok()
            && self.meter.charge_linear_gas(self.per_byte, bytes).is_ok();
        if !charged {
            if let Some(meter) = self.meter.try_as_basic_gas_meter() {
                *meter = snapshot;
            }
        }
        charged
    }

    fn charge(&mut self, n: usize) -> io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        if let Some((reads, bytes)) = &mut self.deferred {
            *reads += 1;
            *bytes += n as u64;
            return Ok(());
        }
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
        self.charge(n)?;
        Ok(n)
    }

    fn read_exact(&mut self, mut buf: &mut [u8]) -> io::Result<()> {
        let mut consumed = 0usize;
        while !buf.is_empty() {
            match self.inner.read(buf) {
                Ok(0) => {
                    let _ = self.charge(consumed);
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                Ok(n) => {
                    consumed += n;
                    buf = &mut buf[n..];
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    let _ = self.charge(consumed);
                    return Err(err);
                }
            }
        }
        self.charge(consumed)
    }
}
