# `sov-timelock`

Timelock capability module for Sovereign SDK runtimes.

The module stores pending proposals by owner and proposal id, exposes runtime
capability methods for registering and unlocking proposals, and provides
user-callable messages for cancelling proposals and modifying cancellation
policies.

## Runtime Integration

Add `sov_timelock::Timelock` to the runtime, return it from the runtime's
timelock capability accessor, and implement `timelock_for_callmessage` for the
runtime calls that must be delayed.

```rust,ignore
use std::num::NonZeroU64;

use sov_modules_api::capabilities::{TimelockCapability, TimelockPolicy};
use sov_modules_api::Spec;

pub struct Runtime<S: Spec> {
    pub admin: my_admin_module::Admin<S>,
    pub timelock: sov_timelock::Timelock<S>,
    // Other modules...
}

impl<S: Spec> sov_modules_api::capabilities::HasCapabilities<S> for Runtime<S> {
    // Existing capability implementation omitted.

    fn timelock(&mut self) -> impl TimelockCapability<S> {
        &mut self.timelock
    }
}

impl<S: Spec> sov_modules_stf_blueprint::Runtime<S> for Runtime<S> {
    // Other associated types and methods omitted.

    fn timelock_for_callmessage(&self, call: &Self::Decodable) -> Option<TimelockPolicy> {
        match call {
            RuntimeCall::Admin(my_admin_module::CallMessage::RotateAdmin { .. }) => {
                Some(TimelockPolicy {
                    unlock_seconds_from_proposal: NonZeroU64::new(24 * 60 * 60).unwrap(),
                    expire_seconds_after_unlock_override: None,
                })
            }
            _ => None,
        }
    }
}
```

If a runtime returns a timelock policy but does not provide a concrete timelock
capability, the transaction is rejected with `TimelocksNotAvailable`. This keeps
timelocks opt-in for runtimes that do not include `sov-timelock`.

## Module-Owned Timelocks

Modules may also depend directly on `sov-timelock` and enforce their own
timelocks internally, instead of asking the runtime to match on their call
messages in `timelock_for_callmessage`. This is useful when the timelock is a
module invariant rather than a rollup-level policy. `sov-timelock` uses this
pattern for `ModifyCancellationPolicy`.

The module should encode the operation it wants to protect, wrap those bytes with
`TimelockProposalData::custom_data(module_id, bytes)`, and call
`register_or_try_unlock_proposal`. If the outcome is `Registered`, the module
returns without applying the protected action. If the outcome is `Unlocked`, the
module applies the action. The timelock module must still be part of the
runtime; the dependent module takes a module reference to it with `#[module]`.

```rust,no_run
use std::num::NonZeroU64;

use schemars::JsonSchema;
use sov_modules_api::capabilities::{
    TimelockCapability, TimelockPolicy, TimelockProposalData, TimelockProposalOutcome,
};
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    Context, CoreModuleError, Module, ModuleId, ModuleInfo, Spec, StateValue, TxState,
};

#[derive(Clone, ModuleInfo)]
pub struct ExampleModule<S: Spec> {
    #[id]
    pub id: ModuleId,

    #[state]
    pub setup_mode_terminated: StateValue<bool>,

    #[module]
    pub timelock: sov_timelock::Timelock<S>,
}

#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
pub enum CallMessage {
    TerminateSetupMode {},
}

impl<S: Spec> Module for ExampleModule<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = CallMessage;
    type Event = ();
    type Error = sov_modules_api::Error;

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        match msg {
            CallMessage::TerminateSetupMode {} => self.terminate_setup_mode(context, state),
        }
    }
}

impl<S: Spec> ExampleModule<S> {
    fn terminate_setup_mode(
        &mut self,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), sov_modules_api::Error> {
        let encoded_message = borsh::to_vec(&CallMessage::TerminateSetupMode {})
            .map_err(|error| CoreModuleError::Generic(anyhow::anyhow!(error)))?;
        let proposal_data = TimelockProposalData::custom_data(self.id, encoded_message);

        let outcome = self.timelock.register_or_try_unlock_proposal(
            context.sender(),
            proposal_data,
            TimelockPolicy {
                unlock_seconds_from_proposal: NonZeroU64::new(24 * 60 * 60).unwrap(),
                expire_seconds_after_unlock_override: None,
            },
            state,
        )?;

        match outcome {
            TimelockProposalOutcome::Registered => {}
            TimelockProposalOutcome::Unlocked => {
                self.setup_mode_terminated
                    .set(&true, state)
                    .map_err(CoreModuleError::state_write)?;
            }
        }

        Ok(())
    }
}
```

The `CustomData` domain is separated from runtime call-message proposal data, so
module-owned proposal ids cannot collide with runtime-managed timelocks.

## Duplicate Proposals

Simultaneous duplicate proposals are not supported. Proposals are keyed by
`(owner_address, proposal_id)`, and the proposal id is derived from the encoded
call message or module-owned custom proposal data. If the same owner submits the
same timelocked call while it is already pending, the module treats it as an
attempt to unlock the existing proposal, not as a request to create another one.

Modules that need multiple simultaneous timelocks for otherwise identical
actions should make their call messages intentionally malleable, for example by
including a meaningless `nonce` or `salt` field purely to produce a distinct
proposal hash.

## Transaction UX

For a timelocked runtime call, the proposal id is the hash of the encoded
`Runtime::CallMessage`, not the hash of the raw transaction. Users submit the
same call message twice, usually in two distinct signed transactions with fresh
nonce or uniqueness metadata:

1. The first transaction registers the call as a pending proposal and does not
   dispatch the underlying call.
2. A repeat transaction before the unlock time is rejected as still locked.
3. A repeat transaction after the unlock time dispatches the underlying call and
   consumes the proposal.
4. A repeat transaction after the expiry window is rejected as expired. Expired
   proposals can be removed with `CleanExpiredProposals`.

```rust,ignore
use sov_modules_api::capabilities::{
    calculate_timelock_proposal_id, TimelockProposalData,
};
use sov_modules_api::EncodeCall;

let call = RuntimeCall::Admin(my_admin_module::CallMessage::RotateAdmin {
    new_admin,
});

let proposal_id = calculate_timelock_proposal_id::<S>(
    &TimelockProposalData::call_message(Runtime::<S>::encode(&call)),
);

submit_transaction(call.clone()); // Registers the proposal.
wait_until_unlock_time();
submit_transaction(call); // Executes and consumes the proposal.
```

The default post-unlock expiry window is two days. A runtime policy can override
that window with `expire_seconds_after_unlock_override`.

## Cancelling Proposals

The proposal owner can cancel a pending proposal by passing `address: None`.

```rust,ignore
let cancel = RuntimeCall::Timelock(sov_timelock::CallMessage::CancelProposal {
    proposal_id,
    address: None,
});

submit_transaction(cancel);
```

An authorized canceller can cancel on behalf of the owner by passing the owner's
address explicitly.

```rust,ignore
let cancel = RuntimeCall::Timelock(sov_timelock::CallMessage::CancelProposal {
    proposal_id,
    address: Some(owner_address),
});

submit_transaction(cancel);
```

Users configure that authorized canceller with `ModifyCancellationPolicy`.

```rust,ignore
let update_policy =
    RuntimeCall::Timelock(sov_timelock::CallMessage::ModifyCancellationPolicy {
        new_policy: sov_timelock::CancellationPolicy {
            authorized_canceller,
            policy_change_timelock_seconds: 60 * 60,
        },
    });

submit_transaction(update_policy);
```

If the current policy has `policy_change_timelock_seconds == 0`, the update is
applied immediately. Otherwise, the policy update is itself timelocked: submit
the same `ModifyCancellationPolicy` call once to register the update, then submit
it again after the policy-change delay to apply it. Policy update proposals use
the default expiry window.
