use core::fmt;

use anyhow::Result;

/// A gas unit that provides scalar conversion from complex, multi-dimensional types.
pub trait GasUnit: fmt::Debug + Clone {
    /// A zeroed instance of the unit.
    const ZEROED: Self;

    /// Creates a unit from a multi-dimensional unit with arbitrary dimension.
    fn from_arbitrary_dimensions(dimensions: &[u64]) -> Self;

    /// Converts the unit into a scalar value, given a price.
    fn value(&self, price: &Self) -> u64;
}

/// A multi-dimensional gas unit.
pub type TupleGasUnit<const N: usize> = [u64; N];

impl<const N: usize> GasUnit for TupleGasUnit<N> {
    const ZEROED: Self = [0; N];

    fn from_arbitrary_dimensions(dimensions: &[u64]) -> Self {
        // as demonstrated on the link below, the compiler can easily optimize the conversion as if
        // it is a transparent type.
        //
        // https://rust.godbolt.org/z/rPhaxnPEY
        let mut unit = Self::ZEROED;
        unit.iter_mut()
            .zip(dimensions.iter().copied())
            .for_each(|(a, b)| *a = b);
        unit
    }

    fn value(&self, price: &Self) -> u64 {
        self.iter()
            .zip(price.iter().copied())
            .map(|(a, b)| a.saturating_mul(b))
            .fold(0, |a, b| a.saturating_add(b))
    }
}

/// A gas meter.
pub struct GasMeter<GU>
where
    GU: GasUnit,
{
    remaining_funds: u64,
    gas_price: GU,
}

impl<GU> Default for GasMeter<GU>
where
    GU: GasUnit,
{
    fn default() -> Self {
        Self {
            remaining_funds: 0,
            gas_price: GU::ZEROED,
        }
    }
}

impl<GU> GasMeter<GU>
where
    GU: GasUnit,
{
    /// Creates a new instance of the gas meter with the provided price.
    pub fn new(remaining_funds: u64, gas_price: GU) -> Self {
        Self {
            remaining_funds,
            gas_price,
        }
    }

    /// Returns the remaining gas funds.
    pub const fn remaining_funds(&self) -> u64 {
        self.remaining_funds
    }

    /// Deducts the provided gas unit from the remaining funds, computing the scalar value of the
    /// funds from the price of the instance.
    pub fn charge_gas(&mut self, gas: &GU) -> Result<()> {
        let gas = gas.value(&self.gas_price);
        self.remaining_funds = self
            .remaining_funds
            .checked_sub(gas)
            .ok_or_else(|| anyhow::anyhow!("Not enough gas"))?;

        Ok(())
    }
}
