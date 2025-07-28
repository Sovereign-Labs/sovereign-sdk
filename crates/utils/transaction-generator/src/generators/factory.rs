//! Implements call message generation for the most widely used modules such that
//! the generator can be plugged into any [`Runtime`] implementation.

use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use sov_modules_api::prelude::arbitrary;
use sov_modules_api::{DispatchCall, Spec};
use sov_modules_stf_blueprint::Runtime;

use crate::interface::{GeneratedMessage, MessageValidity};
use crate::state::State;
use crate::{Distribution, HarnessModule};

/// The minimal amount of randomness needed to generate a call message. This value should be high enough
/// to be able to generate a random call message.
const MINIMUM_RANDOMNESS_CALL_MESSAGE_GEN: u64 = 11_000;

/// Generates call messages for the modules passed as inputs.
///
/// Each instance has its own state, which is some subset of the world state. Callers
/// may instantiate multiple generators and run them in parallel so long as the initial
/// states of the generators are fully disjoin.
#[derive(Debug, Clone, Default)]
pub struct CallMessageFactory<
    S: Spec,
    RT,
    Tag: Clone + Eq + Hash + Debug,
    ChangelogEntry,
    Acct = (),
> {
    phantom: PhantomData<(S, RT, Tag, ChangelogEntry, Acct)>,
}

impl<S: Spec, RT, Tag: Clone + Eq + Hash + Debug, ChangelogEntry: crate::ChangelogEntry, Acct>
    CallMessageFactory<S, RT, Tag, ChangelogEntry, Acct>
{
    /// Instantiate a new [`CallMessageFactory`] with the given
    /// subset of state.
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<
        RT: Runtime<S>,
        S: Spec,
        Tag: Clone + Eq + Hash + Debug + 'static,
        ChangelogEntry: crate::ChangelogEntry,
        BonusAcctData: Debug + Clone + Default + Send + Sync + 'static,
    > CallMessageFactory<S, RT, Tag, ChangelogEntry, BonusAcctData>
{
    /// Generate call messages needed to properly setup the generator.
    #[allow(clippy::type_complexity)]
    pub fn generate_setup_messages(
        &self,
        modules: &Vec<Arc<dyn HarnessModule<S, RT, Tag, ChangelogEntry, BonusAcctData>>>,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut State<S, Tag, BonusAcctData>,
    ) -> arbitrary::Result<Vec<GeneratedMessage<S, <RT as DispatchCall>::Decodable, ChangelogEntry>>>
    {
        let mut messages: Vec<
            GeneratedMessage<S, <RT as DispatchCall>::Decodable, ChangelogEntry>,
        > = vec![];

        for module in modules {
            messages.append(&mut module.generate_setup_messages(u, generator_state)?);
        }

        Ok(messages)
    }

    /// Generates a call message for the modules supported by this generator.
    #[allow(clippy::type_complexity)]
    pub fn generate_call_message(
        &self,
        modules: &Distribution<Arc<dyn HarnessModule<S, RT, Tag, ChangelogEntry, BonusAcctData>>>,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut State<S, Tag, BonusAcctData>,
        validity: MessageValidity,
    ) -> arbitrary::Result<GeneratedMessage<S, <RT as DispatchCall>::Decodable, ChangelogEntry>>
    {
        if u.len() < <u64 as TryInto<usize>>::try_into(MINIMUM_RANDOMNESS_CALL_MESSAGE_GEN).unwrap()
        {
            return Err(arbitrary::Error::NotEnoughData);
        }

        let module = modules.select_value(u)?;

        let GeneratedMessage {
            message,
            sender,
            outcome: changes,
        } = module.generate_call_message(u, generator_state, validity)?;

        Ok(GeneratedMessage {
            message,
            sender,
            outcome: changes,
        })
    }
}
