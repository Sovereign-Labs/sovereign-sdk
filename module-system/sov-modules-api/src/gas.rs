pub mod sov_modules_api {
    use anyhow::Result;

    use core::marker::PhantomData;

    pub trait GasUnit {
        fn value(&self, price: &Self) -> u64;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct TupleGasUnit<const N: usize>(pub [u64; N]);

    impl<const N: usize> GasUnit for TupleGasUnit<N> {
        fn value(&self, price: &Self) -> u64 {
            self.0
                .iter()
                .zip(price.0.iter().copied())
                .map(|(a, b)| a.saturating_mul(b))
                .fold(0, |a, b| a.saturating_add(b))
        }
    }

    pub trait Context {
        type GasUnit: GasUnit;
    }

    pub struct GasoMeter<GU>
    where
        GU: GasUnit,
    {
        remaining_funds: u64,
        gas_price: GU,
    }

    impl<GU> GasoMeter<GU>
    where
        GU: GasUnit,
    {
        pub fn new(remaining_funds: u64, gas_price: GU) -> Self {
            Self {
                remaining_funds,
                gas_price,
            }
        }

        pub fn charge_gas(&mut self, gas: &GU) -> Result<()> {
            let gas = gas.value(&self.gas_price);
            self.remaining_funds = self
                .remaining_funds
                .checked_sub(gas)
                .ok_or_else(|| anyhow::anyhow!("Not enough gas"))?;

            Ok(())
        }
    }

    pub struct WorkingSet<C>
    where
        C: Context,
    {
        gaso_meter: GasoMeter<C::GasUnit>,
        _context: PhantomData<C>,
    }

    impl<C: Context> WorkingSet<C> {
        pub fn new(remaining_funds: u64, gas_price: C::GasUnit) -> Self {
            Self {
                gaso_meter: GasoMeter::new(remaining_funds, gas_price),
                _context: PhantomData,
            }
        }

        pub fn charge_gas(&mut self, gas: &C::GasUnit) -> Result<()> {
            self.gaso_meter.charge_gas(gas)
        }
    }

    pub trait Module {
        type Context: Context;
        type GasConfig;
        const GAS_CONFIG: Self::GasConfig;

        fn charge_gas(
            ws: &mut WorkingSet<Self::Context>,
            price: &<Self::Context as Context>::GasUnit,
        ) -> Result<()> {
            ws.charge_gas(price)
        }
    }

    pub struct DefaultContext<GU>
    where
        GU: GasUnit,
    {
        _gas_unit: PhantomData<GU>,
    }

    impl<GU> Context for DefaultContext<GU>
    where
        GU: GasUnit,
    {
        type GasUnit = GU;
    }
}

pub mod foo {
    use anyhow::Result;

    use super::sov_modules_api::{DefaultContext, GasUnit, Module, TupleGasUnit, WorkingSet};

    pub struct FooGasConfig<GU>
    where
        GU: GasUnit,
    {
        pub complex_math_operation: GU,
        pub some_other_operation: GU,
    }

    pub struct FooModule;

    impl Module for FooModule {
        type Context = DefaultContext<TupleGasUnit<3>>;
        type GasConfig = FooGasConfig<TupleGasUnit<3>>;
        const GAS_CONFIG: Self::GasConfig = FooGasConfig {
            complex_math_operation: TupleGasUnit([1, 2, 3]),
            some_other_operation: TupleGasUnit([4, 5, 6]),
        };
    }

    impl FooModule {
        pub fn some_cool_function(
            ws: &mut WorkingSet<DefaultContext<TupleGasUnit<3>>>,
        ) -> Result<()> {
            Self::charge_gas(ws, &Self::GAS_CONFIG.complex_math_operation)
        }
    }
}

#[test]
fn it_works() {
    use foo::FooModule;
    use sov_modules_api::{TupleGasUnit, WorkingSet};

    let gas = 1_000_000;
    let price = TupleGasUnit([7, 8, 9]);
    let mut ws = WorkingSet::new(gas, price);

    FooModule::some_cool_function(&mut ws).unwrap();
}
