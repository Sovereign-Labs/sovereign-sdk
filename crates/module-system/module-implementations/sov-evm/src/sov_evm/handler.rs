use revm::{
    context::{
        result::{EVMError, HaltReason, InvalidTransaction},
        Transaction,
    },
    context_interface::{journaled_state::account::JournaledAccountTr, ContextTr, JournalTr},
    handler::{
        evm::FrameTr, instructions::InstructionProvider, EvmTr, FrameResult, Handler,
        PrecompileProvider,
    },
    inspector::{Inspector, InspectorEvmTr, InspectorHandler},
    interpreter::{
        interpreter::EthInterpreter, interpreter_action::FrameInit, InitialAndFloorGas,
        InterpreterResult,
    },
    state::EvmState,
    Database,
};

use crate::{gas_metering_mode, GasMeteringMode};

#[derive(Debug)]
pub struct SovHandler<EVM>(core::marker::PhantomData<EVM>);

impl<EVM> Default for SovHandler<EVM> {
    fn default() -> Self {
        Self(core::marker::PhantomData)
    }
}

impl<EVM> Handler for SovHandler<EVM>
where
    EVM: EvmTr<
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
        Frame: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
    >,
{
    type Evm = EVM;
    type Error = EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>;
    type HaltReason = HaltReason;

    #[inline]
    fn validate(&self, evm: &mut Self::Evm) -> Result<InitialAndFloorGas, Self::Error> {
        self.validate_env(evm)?;
        let mut init_and_floor_gas = match gas_metering_mode() {
            GasMeteringMode::Rollup => InitialAndFloorGas::new(0, 0), // We disable charging initial gas
            GasMeteringMode::Evm => self.validate_initial_tx_gas(evm)?,
        };
        self.validate_against_state_and_deduct_caller(evm, &mut init_and_floor_gas)?;
        Ok(init_and_floor_gas)
    }

    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
        _init_and_floor_gas: &mut InitialAndFloorGas,
    ) -> Result<(), Self::Error> {
        let context = evm.ctx();
        let (tx, journal) = context.tx_journal_mut();

        // Load caller's account.
        let mut caller = journal.load_account_with_code_mut(tx.caller())?.data;

        // Bump the nonce for calls. Nonce for CREATE will be bumped in `handle_create`.
        if tx.kind().is_call() {
            caller.bump_nonce();
        }

        // Touch account so we know it is changed.
        caller.touch();

        Ok(())
    }

    fn reward_beneficiary(
        &self,
        _evm: &mut Self::Evm,
        _exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn reimburse_caller(
        &self,
        _evm: &mut Self::Evm,
        _exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<EVM> InspectorHandler for SovHandler<EVM>
where
    EVM: InspectorEvmTr<
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type IT = EthInterpreter;
}
