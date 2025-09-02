use revm::{
    bytecode::opcode::OpCode,
    context::ContextTr,
    interpreter::{interpreter_types::Jumps, Interpreter},
    Inspector,
};

/// An Inspector that erases the costs of storage access
#[derive(Clone, Debug, Default)]
pub struct UnmeteredStorageAccessInspector {
    /// Keep track of the remaining gas before opcode execution
    last_gas_remaining: Option<u64>,
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
        self.last_gas_remaining = Some(interp.gas.remaining());
    }

    fn step_end(&mut self, interp: &mut Interpreter, _: &mut CTX) {
        let gas_remaining = self
            .last_gas_remaining
            .take()
            .expect("step() is always called before step_end()");
        // if storage access - revert gas changes
        let opcode = OpCode::new(interp.bytecode.opcode());
        if opcode == Some(OpCode::SSTORE) || opcode == Some(OpCode::SLOAD) {
            // compute gas usage for the opcode
            let gas_cost = gas_remaining.saturating_sub(interp.gas.remaining());
            interp.gas.erase_cost(gas_cost);
        }
    }
}
