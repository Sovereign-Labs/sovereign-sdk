pub mod sov_modules_api {
    use anyhow::Result;

    use core::marker::PhantomData;

    pub trait GasUnit {
        //: serde::Serialize + serde::de::DeserializeOwned {
        fn value(&self, price: &Self) -> u64;
    }

    pub trait Context {
        type GasUnit: GasUnit;
    }

    pub struct GasoMeter<GasUnit> {
        remaining_funds: u64,
        gas_price: GasUnit,
    }

    impl<GU: GasUnit> GasoMeter<GU> {
        pub fn new(remaining_funds: u64, gas_price: GU) -> Self {
            Self {
                remaining_funds,
                gas_price,
            }
        }

        pub fn charge_gas(&mut self, gas: &GU) -> Result<()> {
            // TODO add checks
            self.remaining_funds = self
                .remaining_funds
                .saturating_sub(gas.value(&self.gas_price));

            Ok(())
        }
    }
    pub struct WorkingSet<C: Context> {
        gaso_meter: GasoMeter<C::GasUnit>,
        _context: PhantomData<C>,
    }

    impl<C: Context> WorkingSet<C> {
        pub fn new(gaso_meter: GasoMeter<C::GasUnit>) -> Self {
            Self {
                gaso_meter,
                _context: PhantomData,
            }
        }

        pub fn charge_gas(&mut self, gas: &C::GasUnit) -> Result<()> {
            self.gaso_meter.charge_gas(gas)
        }
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    pub struct TupleGasUnit {
        pub a: u64,
        pub b: u64,
        pub c: u64,
    }

    impl GasUnit for TupleGasUnit {
        fn value(&self, price: &Self) -> u64 {
            self.a.saturating_mul(price.a).saturating_add(
                self.b
                    .saturating_mul(price.b)
                    .saturating_add(self.c.saturating_mul(price.c)),
            )
        }
    }

    pub struct DefaultContext {}

    impl Context for DefaultContext {
        type GasUnit = TupleGasUnit;
    }

    // The `GasConfig` will have the whole implementation encapsulated by macros.

    pub trait GasConfig {
        const MANIFEST: Self;
    }

    pub trait Module {
        type Context: Context;
        type GasConfig: GasConfig;
    }
}

pub mod foo {
    use std::marker::PhantomData;

    use anyhow::Result;

    use super::sov_modules_api::{Context, GasConfig, GasUnit, Module, WorkingSet};

    //#[derive(serde::Serialize, serde::Deserialize)]
    pub struct FooGasConfig<GS> {
        complex_math_operation: GS,
        another_operation: GS,
    }

    impl<GS: GasUnit> GasConfig for FooGasConfig<GS> {
        const MANIFEST: Self = {
            Self {
                complex_math_operation: todo!(),
                another_operation: todo!(),
            }
        };
    }

    // then we define a custom module

    pub struct FooModule<C: Context> {
        _context: PhantomData<C>,
        gas_config: FooGasConfig<C::GasUnit>,
    }

    impl<C: Context> Module for FooModule<C> {
        type Context = C;
        type GasConfig = FooGasConfig<C::GasUnit>;
    }

    // we can charge gas using our custom unit to define the price

    impl<C: Context> FooModule<C> {
        pub fn new() -> Self {
            Self {
                _context: PhantomData::<C>,
                gas_config: GasConfig::MANIFEST,
            }
        }

        pub fn some_cool_function(&self, ws: &mut WorkingSet<C>) -> Result<()> {
            //.. some code
            ws.charge_gas(&self.gas_config.complex_math_operation)?;
            //.. more code
            Ok(())
        }
    }
}

#[test]
fn it_works() {
    use foo::FooModule;
    use sov_modules_api::{DefaultContext, GasoMeter, WorkingSet};

    let funds = 1_000_000;
    let price = TupleGasUnit { a: 1, b: 2, c: 3 };
    let gs = GasoMeter::new(funds, price);

    let mut ws = WorkingSet::<DefaultContext>::new(gs);

    let foo = FooModule::new();

    foo.some_cool_function(&mut ws).unwrap();
}
