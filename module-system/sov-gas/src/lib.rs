pub trait GasUnit: Copy {
    type Price;
    fn value(&self, p: &Self::Price) -> u64;
}

pub struct StdGasConfig<G: GasUnit> {
    pub set_gas: G,
    pub get_gas: G,
    pub hash_gas: G,
    pub sig_check_gas: G,
}

#[derive(Copy, Clone)]
pub struct Gas2D {
    pub native: u64,
    pub zk: u64,
}

pub struct Price2D {}

impl GasUnit for Gas2D {
    type Price = Price2D;
    fn value(&self, p: &Price2D) -> u64 {
        todo!()
    }
}
