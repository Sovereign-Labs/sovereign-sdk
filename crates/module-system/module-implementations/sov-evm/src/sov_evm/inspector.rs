use revm::{
    bytecode::opcode::OpCode,
    context::ContextTr,
    interpreter::{interpreter_types::Jumps, Interpreter},
    Inspector,
};

/// An Inspector that erases the costs of storage access
#[derive(Clone, Debug, Default)]
pub struct UnmeteredStorageAccessInspector {
    /// Keep track of the last opcode executed and the remaining gas
    last_opcode_gas_remaining: Option<(OpCode, u64)>,
}

impl UnmeteredStorageAccessInspector {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<CTX> Inspector<CTX> for UnmeteredStorageAccessInspector
where
    CTX: ContextTr,
{
    fn step(&mut self, interp: &mut Interpreter, _: &mut CTX) {
        if let Some(opcode) = OpCode::new(interp.bytecode.opcode()) {
            // keep track of the last opcode executed
            self.last_opcode_gas_remaining = Some((opcode, interp.gas.remaining()));
        }
    }

    fn step_end(&mut self, interp: &mut Interpreter, _: &mut CTX) {
        if let Some((opcode, gas_remaining)) = self.last_opcode_gas_remaining.take() {
            // compute gas usage for the last opcode
            let gas_cost = gas_remaining.saturating_sub(interp.gas.remaining());
            // if storage access - revert gas changes
            if opcode == OpCode::SSTORE || opcode == OpCode::SLOAD {
                interp.gas.erase_cost(gas_cost);
            }
        }
    }
}
