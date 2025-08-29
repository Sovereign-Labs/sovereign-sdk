use revm::{
    context::{
        result::{EVMError, HaltReason, InvalidTransaction},
        Transaction,
    },
    context_interface::{ContextTr, JournalTr},
    handler::{
        evm::FrameTr, instructions::InstructionProvider, EvmTr, FrameResult, Handler,
        PrecompileProvider,
    },
    inspector::{Inspector, InspectorEvmTr, InspectorHandler},
    interpreter::{interpreter::EthInterpreter, interpreter_action::FrameInit, InterpreterResult},
    state::EvmState,
    Database,
};

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

    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<(), Self::Error> {
        let context = evm.ctx();
        let (tx, journal) = context.tx_journal_mut();

        // Load caller's account.
        let caller_account = journal.load_account_code(tx.caller())?.data;
        let old_balance = caller_account.info.balance;

        // Touch account so we know it is changed.
        caller_account.mark_touch();

        // Bump the nonce for calls. Nonce for CREATE will be bumped in `handle_create`.
        if tx.kind().is_call() {
            // Nonce is already checked
            caller_account.info.nonce = caller_account.info.nonce.saturating_add(1);
        }

        journal.caller_accounting_journal_entry(tx.caller(), old_balance, tx.kind().is_call());

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
