use revm::{
    bytecode::opcode::OpCode,
    context::ContextTr,
    interpreter::{interpreter_types::Jumps, Interpreter},
    Inspector,
};

use crate::{gas_metering_mode, GasMeteringMode};

/// An Inspector that tracks the costs of storage access
#[derive(Clone, Debug, Default)]
pub struct StorageAccessInspector {
    /// Keep track of the remaining gas before opcode execution
    last_gas_remaining: Option<u64>,
    /// Gas spent on storage access
    gas_spent_on_storage_access: u64,
}

impl StorageAccessInspector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The amount of gas spent on storage access
    pub fn gas_spent_on_storage_access(&self) -> u64 {
        self.gas_spent_on_storage_access
    }
}

impl<CTX> Inspector<CTX> for StorageAccessInspector
where
    CTX: ContextTr,
{
    fn step(&mut self, interp: &mut Interpreter, _: &mut CTX) {
        match gas_metering_mode() {
            GasMeteringMode::Rollup => {
                self.last_gas_remaining = Some(interp.gas.remaining());
            }
            GasMeteringMode::Evm => (),
        }
    }

    fn step_end(&mut self, interp: &mut Interpreter, _: &mut CTX) {
        match gas_metering_mode() {
            GasMeteringMode::Rollup => {
                let gas_remaining = self
                    .last_gas_remaining
                    .take()
                    .expect("step() is always called before step_end()");
                // if storage access - revert gas changes
                let opcode = OpCode::new(interp.bytecode.opcode());
                if opcode == Some(OpCode::SSTORE) || opcode == Some(OpCode::SLOAD) {
                    // compute gas usage for the opcode
                    let gas_cost = gas_remaining.saturating_sub(interp.gas.remaining());
                    self.gas_spent_on_storage_access += gas_cost;
                }
            }
            GasMeteringMode::Evm => (),
        }
    }
}
