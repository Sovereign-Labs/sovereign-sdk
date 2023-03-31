use jmt::SimpleHasher;

use crate::{services::da::SlotData, stf::StateTransitionFunction};

pub trait RollupSpec {
    type SlotData: SlotData;
    type Stf: StateTransitionFunction;
    type Hasher: SimpleHasher;
}
