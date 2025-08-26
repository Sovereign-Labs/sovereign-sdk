use delegate::delegate;
use revm::{
    context::{ContextError, ContextSetters, ContextTr, Evm, FrameStack},
    handler::{
        evm::FrameTr, instructions::EthInstructions, EthFrame, EthPrecompiles, EvmTr,
        FrameInitOrResult, ItemOrResult,
    },
    inspector::{InspectorEvmTr, JournalExt},
    interpreter::interpreter::EthInterpreter,
    Database, Inspector,
};

/// TODO
#[derive(Debug)]
pub struct SovEvm<CTX, INSP>(
    pub  Evm<
        CTX,
        INSP,
        EthInstructions<EthInterpreter, CTX>,
        EthPrecompiles,
        EthFrame<EthInterpreter>,
    >,
);

impl<CTX: ContextTr, INSP> SovEvm<CTX, INSP> {
    /// TODO
    pub fn new(ctx: CTX, inspector: INSP) -> Self {
        Self(Evm {
            ctx,
            inspector,
            instruction: EthInstructions::new_mainnet(),
            precompiles: EthPrecompiles::default(),
            frame_stack: FrameStack::new(),
        })
    }
}

impl<CTX: ContextTr, INSP> EvmTr for SovEvm<CTX, INSP>
where
    CTX: ContextTr,
{
    type Context = CTX;
    type Instructions = EthInstructions<EthInterpreter, CTX>;
    type Precompiles = EthPrecompiles;
    type Frame = EthFrame<EthInterpreter>;

    delegate! {
        to self.0 {
            fn ctx(&mut self) -> &mut Self::Context;
            fn ctx_ref(&self) -> &Self::Context;
            fn ctx_instructions(&mut self) -> (&mut Self::Context, &mut Self::Instructions);
            fn ctx_precompiles(&mut self) -> (&mut Self::Context, &mut Self::Precompiles);
            fn frame_stack(&mut self) -> &mut FrameStack<Self::Frame>;
            fn frame_init(
                &mut self,
                frame_input: <Self::Frame as FrameTr>::FrameInit,
            ) -> Result<
                ItemOrResult<&mut Self::Frame, <Self::Frame as FrameTr>::FrameResult>,
                ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
            >;
            fn frame_run(
                &mut self,
            ) -> Result<
                FrameInitOrResult<Self::Frame>,
                ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
            >;
            fn frame_return_result(
                &mut self,
                frame_result: <Self::Frame as FrameTr>::FrameResult,
            ) -> Result<
                Option<<Self::Frame as FrameTr>::FrameResult>,
                ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
            >;
        }
    }
}

impl<CTX: ContextTr, INSP> InspectorEvmTr for SovEvm<CTX, INSP>
where
    CTX: ContextSetters<Journal: JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
{
    type Inspector = INSP;

    delegate! {
        to self.0 {
            fn inspector(&mut self) -> &mut Self::Inspector;
            fn ctx_inspector(&mut self) -> (&mut Self::Context, &mut Self::Inspector);
            fn ctx_inspector_frame(
                &mut self,
            ) -> (&mut Self::Context, &mut Self::Inspector, &mut Self::Frame);
            fn ctx_inspector_frame_instructions(
                &mut self,
            ) -> (
                &mut Self::Context,
                &mut Self::Inspector,
                &mut Self::Frame,
                &mut Self::Instructions,
            );
        }
    }
}
