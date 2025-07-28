use std::fmt::Debug;

use anyhow::{bail, Result};
use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::safe_vec::CapacityError;
use sov_modules_api::{Context, DaSpec, EventEmitter, Spec, TxState};

use crate::{
    AuthorizedSequencers, Event, PayeePolicy, Payer, Paymaster, PaymasterPolicy,
    PaymasterPolicyInitializer,
};

/// The default length of a [`SafeVec`].
pub const DEFAULT_SAFE_VEC_LEN: usize = 20;
/// A [`Vec`]-like type with a reasonable limit on its length.
pub type SafeVec<T, const LEN: usize = DEFAULT_SAFE_VEC_LEN> = sov_modules_api::SafeVec<T, LEN>;

/// A list of payees and the policies that apply to them.
///
/// A type alias for [`SafeVec`] where the elements have type `(S::Address, PayeePolicy<S>)`.
#[allow(type_alias_bounds)]
pub type PayeePolicyList<S: Spec, const LEN: usize = DEFAULT_SAFE_VEC_LEN> =
    SafeVec<(S::Address, PayeePolicy<S>), LEN>;

/// Call messages for interacting with the `Paymaster` module.
///
/// ## Note:   
/// These call messages are highly unusual in that they have different effects
/// based on the address of the sequencer who places them on chain. See the docs
/// on individual variants for more information.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
pub enum CallMessage<S: Spec> {
    /// Register a new payer with the given policy. If the sequencer who places this message on chain
    /// is present in the list of `authorized_sequencers` to use the payer, the payer address for that
    /// sequencer is set to the address of the newly registered payer.
    RegisterPaymaster {
        #[allow(missing_docs)]
        policy: PaymasterPolicyInitializer<S>,
    },
    /// Set the payer address for the sequencer to the given address.
    /// This call message is highly unusual in that it executes regardless of the sender address on the rollup.
    /// Sequencers who do not wish to update their payer address should not sequence transactions containing this callmessage.
    SetPayerForSequencer {
        #[allow(missing_docs)]
        payer: S::Address,
    },
    /// Update the policy for a given payer. If the sequencer who places this message on chain
    /// is present in the list of `authorized_sequencers` to use the payer after the update, the payer address for that
    /// sequencer is set to the address of the newly registered paymaster.
    UpdatePolicy {
        #[allow(missing_docs)]
        payer: S::Address,
        #[allow(missing_docs)]
        update: PolicyUpdate<S>,
    },
}

/// An update to the policy of a single gas payer
#[derive(Debug, PartialEq, Eq, Clone, derivative::Derivative, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
#[derivative(Default(bound = ""))]
pub struct PolicyUpdate<S: Spec> {
    sequencer_update: Option<SequencerSetUpdate<S::Da>>,
    updaters_to_add: Option<SafeVec<S::Address>>,
    updaters_to_remove: Option<SafeVec<S::Address>>,
    payee_policies_to_set: Option<SafeVec<(S::Address, PayeePolicy<S>)>>,
    payee_policies_to_delete: Option<SafeVec<S::Address>>,
    default_policy: Option<PayeePolicy<S>>,
}

impl<S: Spec> PolicyUpdate<S> {
    /// Creates an empty policy update
    pub fn new() -> Self {
        Self::default()
    }

    /// Authorize all sequencers to use this payer
    #[must_use]
    pub fn allow_all_sequencers(mut self) -> Self {
        self.sequencer_update = Some(SequencerSetUpdate::AllowAll);
        self
    }

    /// Update the set of sequencers allowed to use this payer
    #[must_use]
    pub fn update_allowed_sequencers(
        mut self,
        mut sequencer_update: AllowedSequencerUpdate<S::Da>,
    ) -> Self {
        // Sort the update lists for ease of use in the SDK. This helps efficiency, but the SDK works fine without it.
        if let Some(x) = sequencer_update.to_add.as_mut() {
            x.sort();
        }
        if let Some(x) = sequencer_update.to_remove.as_mut() {
            x.sort();
        }

        self.sequencer_update = Some(SequencerSetUpdate::Update(sequencer_update));
        self
    }

    /// Add an address to the set of addresses authorized to update this policy
    #[must_use]
    pub fn add_updater(mut self, updater_to_add: S::Address) -> Self {
        retain_elts_if(&mut self.updaters_to_remove, |updater_to_remove| {
            updater_to_remove != &updater_to_add
        });
        self.updaters_to_add
            .get_or_insert(SafeVec::new())
            .try_push(updater_to_add)
            .unwrap();
        self
    }

    /// Remove an address from the set of addresses authorized to update this policy
    #[must_use]
    pub fn remove_updater(mut self, updater_to_remove: S::Address) -> Self {
        retain_elts_if(&mut self.updaters_to_add, |updater_to_add| {
            updater_to_add != &updater_to_remove
        });
        self.updaters_to_remove
            .get_or_insert(SafeVec::new())
            .try_push(updater_to_remove)
            .unwrap();
        self
    }

    /// Add a payee to this policy
    #[must_use]
    pub fn add_payee_policy(mut self, payee_to_add: S::Address, policy: PayeePolicy<S>) -> Self {
        retain_elts_if(&mut self.payee_policies_to_delete, |payee_to_delete| {
            payee_to_delete != &payee_to_add
        });
        self.payee_policies_to_set
            .get_or_insert(SafeVec::new())
            .try_push((payee_to_add, policy))
            .unwrap();
        self
    }

    /// Remove a payee from this policy
    #[must_use]
    pub fn remove_payee_policy(mut self, payee_to_delete: S::Address) -> Self {
        retain_elts_if(&mut self.payee_policies_to_set, |payee_to_add| {
            payee_to_add.0 != payee_to_delete
        });
        self.payee_policies_to_delete
            .get_or_insert(SafeVec::new())
            .try_push(payee_to_delete)
            .unwrap();
        self
    }

    /// Set the default policy for this payer
    #[must_use]
    pub fn set_default_policy(mut self, policy: PayeePolicy<S>) -> Self {
        self.default_policy = Some(policy);
        self
    }
}

/// An update to the allowed sequencer set for a gas payer.
#[derive(Debug, PartialEq, Eq, Clone, derivative::Derivative, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(bound = "Da: DaSpec", rename_all = "snake_case")]
#[schemars(bound = "Da: DaSpec", rename = "SequencerUpdate")]
pub enum SequencerSetUpdate<Da: DaSpec> {
    /// Authorizes any sequencer to use this payer.
    AllowAll,
    /// Sets the list of authorized sequencers to an explicit whitelist if it was previously `AllowAll`.
    /// Adds and removes the requested addresses from the sequencer whitelist.
    Update(AllowedSequencerUpdate<Da>),
}

/// A list of updates to the `allowed_sequencers` list for a particular payer.
#[derive(Debug, PartialEq, Eq, Clone, derivative::Derivative, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(bound = "Da: DaSpec", rename_all = "snake_case")]
#[schemars(bound = "Da: DaSpec", rename = "SequencerUpdateList")]
#[derivative(Default(bound = ""))]
pub struct AllowedSequencerUpdate<Da: DaSpec> {
    to_add: Option<SafeVec<Da::Address>>,
    to_remove: Option<SafeVec<Da::Address>>,
}

impl<Da: DaSpec> AllowedSequencerUpdate<Da> {
    /// Create an empty sequencer update list
    pub fn new() -> Self {
        Self {
            to_add: None,
            to_remove: None,
        }
    }

    /// Create a new update which removes the sequencer
    #[must_use]
    pub fn remove(address: Da::Address) -> Self {
        let mut to_remove = SafeVec::new();
        to_remove
            .try_push(address)
            .expect("Pushing to an empty safe vec is infallible");
        Self {
            to_add: None,
            to_remove: Some(to_remove),
        }
    }

    /// Create a new update which adds the given sequencer.
    #[must_use]
    pub fn add(address: Da::Address) -> Self {
        let mut to_add = SafeVec::new();
        to_add
            .try_push(address)
            .expect("Pushing to an empty safe vec is infallible");
        Self {
            to_add: Some(to_add),
            to_remove: None,
        }
    }

    /// Add permissions for the requested sequencer
    pub fn add_sequencer(
        mut self,
        sequencer_to_add: Da::Address,
    ) -> Result<Self, CapacityError<Self>> {
        retain_elts_if(&mut self.to_remove, |seq_to_remove| {
            seq_to_remove != &sequencer_to_add
        });
        match self
            .to_remove
            .get_or_insert(SafeVec::new())
            .try_push(sequencer_to_add)
        {
            Ok(()) => Ok(self),
            Err(_) => Err(CapacityError::new(self)),
        }
    }

    /// Remove permissions for the requested sequencer
    pub fn remove_sequencer(
        mut self,
        sequencer_to_remove: Da::Address,
    ) -> Result<Self, CapacityError<Self>> {
        retain_elts_if(&mut self.to_add, |seq_to_add| {
            seq_to_add != &sequencer_to_remove
        });
        match self
            .to_remove
            .get_or_insert(SafeVec::new())
            .try_push(sequencer_to_remove)
        {
            Ok(()) => Ok(self),
            Err(_) => Err(CapacityError::new(self)),
        }
    }
}

fn retain_elts_if<T>(collection: &mut Option<SafeVec<T>>, f: impl FnMut(&T) -> bool) {
    if let Some(contents) = collection {
        contents.retain(f);
        if contents.is_empty() {
            *collection = None;
        }
    }
}

impl<S: Spec> Paymaster<S> {
    /// Registers a new paymaster with the given policy. If the paymaster is willing to pay for txs from the
    /// current sequencer, then the paymaster is set as the payer for this sequencer.
    pub(crate) fn register_paymaster(
        &mut self,
        policy: PaymasterPolicyInitializer<S>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        self.do_registration(
            context.sender(),
            std::iter::once(context.sequencer_da_address()),
            policy,
            state,
        )
    }

    /// Registers a new paymaster with the given policy. Sets the provided sequencer addresses
    /// to use that paymaster, if the paymaster policy permits it.
    pub(crate) fn do_registration<'a>(
        &mut self,
        new_payer: &S::Address,
        sequencer_addresses_to_register: impl Iterator<Item = &'a <S::Da as DaSpec>::Address>,
        policy: PaymasterPolicyInitializer<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        if self.payers.get(new_payer, state)?.is_some() {
            tracing::debug!(payer = %new_payer, "Payer already exists. Reverting registration tx.");
            bail!("{} is already registered as a payer. Use `UpdatePolicy` if you wish to change its configuration.", new_payer);
        }

        self.emit_event(
            state,
            Event::<S>::RegisteredPaymaster {
                address: new_payer.clone(),
            },
        );
        self.emit_event(
            state,
            Event::SetDefaultPayeePolicy {
                payer: new_payer.clone(),
                policy: policy.default_payee_policy.clone(),
            },
        );

        self.add_payee_policies(new_payer, policy.payees, state)?;

        // Store the paymaster policy
        let policy = PaymasterPolicy {
            authorized_sequencers: policy.authorized_sequencers,
            authorized_updaters: policy.authorized_updaters,
            default_payee_policy: policy.default_payee_policy,
        };
        self.payers.set(new_payer, &policy, state)?;

        for sequencer in sequencer_addresses_to_register {
            if policy.authorized_sequencers.covers(sequencer) {
                self.sequencer_to_payer.set(sequencer, new_payer, state)?;
                self.emit_event(
                    state,
                    Event::<S>::SetPayerForSequencer {
                        payer: new_payer.clone(),
                        sequencer: sequencer.clone(),
                    },
                );
            } else {
                tracing::debug!(sequencer = %sequencer, payer = %new_payer, "Attempt to register sequencer to use paymaster failed because payer policy does not permit it.");
            }
        }

        Ok(())
    }

    /// Sets the payer address for the sequencer who sends sequences this call message to the given address
    /// if that payer's policy allows it.
    pub(crate) fn set_payer_for_sequencer(
        &mut self,
        payer: S::Address,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let policy = self
            .payers
            .get(&payer, state)?
            .ok_or(anyhow::anyhow!("Paymaster {} is not registered", payer))?;

        if !policy
            .authorized_sequencers
            .covers(context.sequencer_da_address())
        {
            return Err(anyhow::anyhow!(
                "Sequencer {} is not authorized to use paymaster {}",
                context.sequencer_da_address(),
                payer
            ));
        }
        self.sequencer_to_payer
            .set(context.sequencer_da_address(), &payer, state)?;

        self.emit_event(
            state,
            Event::<S>::SetPayerForSequencer {
                payer,
                sequencer: context.sequencer_da_address().clone(),
            },
        );

        Ok(())
    }

    /// Update the policy of the payer
    pub(crate) fn update_policy_if_authorized(
        &mut self,
        policy_update: PolicyUpdate<S>,
        context: &Context<S>,
        payer: &S::Address,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let Some(mut policy) = self.payers.get(payer, state)? else {
            bail!("{} is not a registered payer", payer);
        };
        if !policy.authorized_updaters.contains(context.sender()) {
            bail!(
                "{} is not an authorized updater for payer {}",
                context.sender(),
                payer
            );
        }

        let PolicyUpdate {
            sequencer_update,
            updaters_to_add,
            updaters_to_remove,
            payee_policies_to_set,
            payee_policies_to_delete,
            default_policy,
        } = policy_update;

        self.update_allowed_sequencers(payer, sequencer_update, &mut policy, context, state)?;
        self.remove_allowed_updaters(updaters_to_remove, &mut policy);
        self.add_allowed_updaters(payer, updaters_to_add, &mut policy)?;
        self.remove_payee_policies(payer, payee_policies_to_delete, state)?;
        if let Some(policies_to_add) = payee_policies_to_set {
            self.add_payee_policies(payer, policies_to_add, state)?;
        }
        if let Some(default_policy) = default_policy {
            self.emit_event(
                state,
                Event::SetDefaultPayeePolicy {
                    payer: payer.clone(),
                    policy: default_policy.clone(),
                },
            );
            policy.default_payee_policy = default_policy;
        }

        self.payers.set(payer, &policy, state)?;
        Ok(())
    }

    fn add_payee_policies<const N: usize>(
        &mut self,
        payer: &S::Address,
        to_add: SafeVec<(S::Address, PayeePolicy<S>), N>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        for (address, policy) in to_add {
            self.policies
                .set(&(Payer(payer), &address), &policy, state)?;
            self.emit_event(
                state,
                Event::AddedPayeePolicy {
                    payer: payer.clone(),
                    payee: address,
                    policy,
                },
            );
        }
        Ok(())
    }

    fn remove_payee_policies(
        &self,
        payer: &S::Address,
        to_remove: Option<SafeVec<S::Address>>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let Some(to_remove) = to_remove else {
            return Ok(());
        };
        for payee in to_remove {
            if self
                .policies
                .remove(&(Payer(payer), &payee), state)?
                .is_some()
            {
                self.emit_event(
                    state,
                    Event::RemovedPayeePolicy {
                        payer: payer.clone(),
                        payee,
                    },
                );
            }
        }
        Ok(())
    }

    fn remove_allowed_updaters(
        &self,
        to_remove: Option<SafeVec<S::Address>>,
        policy: &mut PaymasterPolicy<S>,
    ) {
        let authorized_updaters = &mut policy.authorized_updaters;
        if let Some(to_remove) = to_remove {
            remove_elts_from_list(to_remove, authorized_updaters);
        }
    }

    fn add_allowed_updaters(
        &self,
        payer: &S::Address,
        to_add: Option<SafeVec<S::Address>>,
        policy: &mut PaymasterPolicy<S>,
    ) -> Result<()> {
        let authorized_updaters = &mut policy.authorized_updaters;
        if let Some(to_add) = to_add {
            authorized_updaters.try_extend(to_add).map_err(|_| anyhow::anyhow!(
                "Attempted to add too many updaters to policy for payer {}. Only {} updaters are allowed and the policy already has {}.", payer, DEFAULT_SAFE_VEC_LEN, authorized_updaters.len()
            ))?;
        }
        Ok(())
    }

    fn update_allowed_sequencers(
        &mut self,
        payer: &S::Address,
        update: Option<SequencerSetUpdate<S::Da>>,
        policy: &mut PaymasterPolicy<S>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let Some(update) = update else { return Ok(()) };

        match update {
            SequencerSetUpdate::AllowAll => {
                policy.authorized_sequencers = AuthorizedSequencers::All;
            }

            SequencerSetUpdate::Update(sequencer_update_list) => {
                let to_remove = sequencer_update_list.to_remove.unwrap_or_default();
                for seq_address_to_remove in &to_remove {
                    // Only remove the sequencer address if it is currently mapped to this payer
                    if let Some(current_payer) = self
                        .sequencer_to_payer
                        .remove(seq_address_to_remove, state)?
                    {
                        if current_payer == *payer {
                            self.emit_event(
                                state,
                                Event::<S>::RemovedPayerForSequencer {
                                    sequencer: seq_address_to_remove.clone(),
                                    payer: payer.clone(),
                                },
                            );
                        } else {
                            bail!(
                                "Cannot remove sequencer {} from payer {}: sequencer is currently mapped to payer {}",
                                seq_address_to_remove,
                                payer,
                                current_payer
                            );
                        }
                    } else {
                        bail!(
                            "Cannot remove sequencer {} from payer {}: sequencer is not currently mapped to any payer",
                            seq_address_to_remove,
                            payer
                        );
                    }
                }
                let sequencers_to_add = sequencer_update_list.to_add.unwrap_or_default();
                match &mut policy.authorized_sequencers {
                    // If all sequencers were authorized previously, scope the permissions down to just
                    // the provided list
                    AuthorizedSequencers::All => {
                        policy.authorized_sequencers =
                            AuthorizedSequencers::Some(sequencers_to_add);
                    }
                    AuthorizedSequencers::Some(existing_list) => {
                        remove_elts_from_list(to_remove, existing_list);
                        if sequencers_to_add.contains(context.sequencer_da_address()) {
                            self.sequencer_to_payer.set(
                                context.sequencer_da_address(),
                                payer,
                                state,
                            )?;
                            self.emit_event(
                                state,
                                Event::<S>::SetPayerForSequencer {
                                    sequencer: context.sequencer_da_address().clone(),
                                    payer: payer.clone(),
                                },
                            );
                        }
                        existing_list.try_extend(sequencers_to_add).map_err(|_| anyhow::anyhow!("Attempted to add too many sequencers to policy for payer {}. Only {} sequencers are allowed and the policy already has {}.", payer, DEFAULT_SAFE_VEC_LEN, existing_list.len()))?;
                    }
                }
            }
        }

        Ok(())
    }
}

// Remove elements from a list in place in O(n log n) time
fn remove_elts_from_list<Item: Eq + Ord + Clone>(
    mut to_remove: SafeVec<Item>,
    original: &mut SafeVec<Item>,
) {
    to_remove.sort();
    original.sort();

    let mut to_remove = to_remove.iter().peekable();
    let mut next_idx_to_fill = 0;
    for idx in 0..original.len() {
        let item = &original[idx];
        // Since both lists are sorted, if the first item in the list to remove is less than the next item in our original list
        // it's irrelevant. Advance the `to_remove` list until we get to relevant items.
        while to_remove.peek().is_some_and(|contents| *contents < item) {
            to_remove.next();
        }
        // If the next item in our original list is present in `to_remove`, we want to remove it.
        // We do that by simply skipping the `push` at the end of the loop.
        if to_remove.peek().is_some_and(|contents| *contents == item) {
            continue;
        }
        if idx != next_idx_to_fill {
            original[next_idx_to_fill] = item.clone();
        }
        next_idx_to_fill += 1;
    }
    let output_len = next_idx_to_fill;
    original.truncate(output_len);
}
