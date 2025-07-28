use std::fmt::Debug;

use anyhow::{ensure, Context as _, Result};
use schemars::JsonSchema;
use sov_bank::Amount;
use sov_modules_api::macros::{config_value, UniversalWallet};
use sov_modules_api::{
    Context, EventEmitter, GasMeter, HexHash, HexString, SafeVec, Spec, TxState,
};
use strum::{EnumDiscriminants, EnumIs, VariantArray};

use super::Mailbox;
use crate::crypto::{
    decode_signature, ec_recover, eth_address_from_public_key, keccak256_hash, AnnouncementHash,
    DomainHash, EthSignHash, HashKind,
};
use crate::event::Event;
use crate::traits::PostDispatchHook;
use crate::types::{Domain, EthAddress, StorageLocation, ValidatorSignature};
use crate::{Delivery, DispatchState, HyperlaneAddress, Message, Recipient, MAILBOX_ADDR};

/// The version of the hyperlane message format used by the mailbox module.
pub const MESSAGE_VERSION: u8 = 3;
/// The maximum size of a message body.
// Currently set to 8KB
pub const MAX_MESSAGE_BODY_SIZE: usize = 8192;
/// The maximum size of a message metadata.
// Currently set to 8KB
pub const MAX_MESSAGE_METADATA_SIZE: usize = 8192;

/// This enumeration represents the available call messages for interacting with the `sov-value-setter` module.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    JsonSchema,
    EnumDiscriminants,
    EnumIs,
    UniversalWallet,
)]
#[strum_discriminants(derive(VariantArray, EnumIs))]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
pub enum CallMessage<S: Spec> {
    /// Sends an outbound message to the specified recipient.
    Dispatch {
        /// The destination domain (aka "Chain ID")
        domain: Domain,
        /// The recipient address. Must implement the `handle` function - i.e. be a smart contract
        recipient: HexHash,
        /// The message body. For example, if the recipient is a warp route, this will encode the amount/type of funds being transferred
        body: HexString<SafeVec<u8, MAX_MESSAGE_BODY_SIZE>>,
        /// The "metadata" which is used to verify the message or control hooks.
        /// Can be used to set the destination gas limit for a message using
        /// [`IGPMetadata`](crate::igp::IGPMetadata)
        metadata: Option<HexString<SafeVec<u8, MAX_MESSAGE_METADATA_SIZE>>>,
        /// Selected relayer
        relayer: Option<S::Address>,
        /// A limit for the payment to relayer to cover gas needed for message delivery.
        /// If relayer demands more than this value of native gas token, dispatching message
        /// will fail. If it demands less than this, only needed amount will be paid.
        gas_payment_limit: Amount,
    },
    /// Receive an inbound message. This is called *on the desitination chain* by the relayer after
    /// a `dispatch` call has been made on the source chain.
    ///
    /// Passes the message metadata and body to the security module (ISM) for verification,
    /// then calls the recipient's `handle` function with the message body.
    Process {
        /// Metadata used to verify the message.
        metadata: HexString<SafeVec<u8, MAX_MESSAGE_BODY_SIZE>>,
        /// The serialized [`Message`] struct
        message: HexString<SafeVec<u8, MAX_MESSAGE_METADATA_SIZE>>,
    },
    /// Announce a validator and its signatures' storage.
    Announce {
        /// Address of a validator.
        validator_address: EthAddress,
        /// Location of validator's signatures.
        storage_location: StorageLocation,
        /// Signature of the announcement message for verification.
        signature: ValidatorSignature,
    },
}

impl<S: Spec, R: Recipient<S>> Mailbox<S, R>
where
    S::Address: HyperlaneAddress,
{
    /// Dispatches (aka "sends") a message to the specified recipient.
    // Compare with https://github.com/eigerco/hyperlane-monorepo/blob/b68fe264b3585ecd9d95a5ec2ec2d7defbe907d2/solidity/contracts/Mailbox.sol#L276
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch(
        &mut self,
        destination_domain: Domain,
        recipient_address: HexHash,
        sender: HexHash,
        message_body: HexString,
        metadata: Option<HexString>,
        relayer: Option<S::Address>,
        gas_payment_limit: Amount,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<HexString> {
        let mut dispatch_state = self
            .dispatch_state
            .borrow_mut(state)?
            .unwrap_or(DispatchState {
                nonce: 0,
                last_dispatched_id: HexHash::new([0; 32]),
            });
        let message = Message {
            version: MESSAGE_VERSION,
            nonce: dispatch_state.nonce,
            origin_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
            sender,
            dest_domain: destination_domain,
            recipient: recipient_address,
            body: message_body,
        };
        let message_id = message.id(state)?;

        dispatch_state.nonce += 1;
        dispatch_state.last_dispatched_id = message_id;
        dispatch_state.save(state)?;

        let message_hex: HexString = message.encode();
        self.emit_event(
            state,
            Event::Dispatch {
                sender: message.sender,
                destination_domain,
                recipient_address,
                message: message_hex.clone(),
            },
        );
        self.emit_event(state, Event::DispatchId { id: message_id });

        let metadata = metadata.unwrap_or_else(|| HexString::new(vec![]));

        let relayer = self.with_default_relayer(relayer, &recipient_address, state)?;

        self.merkle_tree_hook.post_dispatch(
            &message_id,
            &message,
            &metadata,
            &relayer,
            gas_payment_limit,
            context,
            state,
        )?;

        self.interchain_gas_paymaster.post_dispatch(
            &message_id,
            &message,
            &metadata,
            &relayer,
            gas_payment_limit,
            context,
            state,
        )?;

        Ok(message_hex)
    }

    /// Processes an incoming message.
    // Compare with https://github.com/eigerco/hyperlane-monorepo/blob/b68fe264b3585ecd9d95a5ec2ec2d7defbe907d2/solidity/contracts/Mailbox.sol#L202
    pub(crate) fn process(
        &mut self,
        metadata: HexString,
        message: HexString,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let message_id = keccak256_hash(&message.0, state)?;
        let message = Message::decode(message.as_ref())
            .context(format!("Failed to decode message {message_id}"))?;
        // Ensure message version is correct
        anyhow::ensure!(
            message.version == MESSAGE_VERSION,
            "Invalid message version: {} had version {} but only {} is supported",
            message_id,
            message.version,
            MESSAGE_VERSION,
        );
        let bridge_domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");

        // Ensure the message is intended for us
        anyhow::ensure!(
            message.dest_domain == bridge_domain,
            "Invalid message destination domain. Message ID: {} had domain: {} but only {} is supported",
            message_id,
            message.dest_domain,
            bridge_domain
        );

        // Check if message has already been processed. If not, mark it as processed.
        let delivery = self.deliveries.borrow(&message_id, state)?;
        if delivery.is_some() {
            return Err(anyhow::anyhow!("Message {} already processed", message_id));
        }
        self.deliveries.set(
            &message_id,
            &Delivery {
                sender: message.sender,
                block_number: state.current_visible_slot_number().get(),
            },
            state,
        )?;

        self.emit_event(
            state,
            Event::Process {
                origin_domain: message.origin_domain,
                sender_address: message.sender,
                recipient_address: message.recipient,
            },
        );
        self.emit_event(state, Event::ProcessId { id: message_id });

        // Try and get ISM from recipient registry, if not found, use default ISM
        let ism = match self.recipients.ism(&message.recipient, state)? {
            Some(ism) => ism,
            None => self.recipients.default_ism(state)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "No dedicated or default ISM found for recipient {}",
                    message.recipient
                )
            })?,
        };

        ism.verify(context, &message, &metadata, state)?;
        self.recipients.handle(
            message.origin_domain,
            message.sender,
            &message.recipient,
            message.body,
            state,
        )?;

        Ok(())
    }

    pub(crate) fn announce(
        &mut self,
        validator_address: EthAddress,
        storage_location: StorageLocation,
        signature: ValidatorSignature,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let mut storage_locations = self
            .validators
            .borrow_mut(&validator_address, state)?
            .unwrap_or(Vec::new());

        ensure!(
            !storage_locations.contains(&storage_location),
            "Validator {validator_address} already announced location {storage_location}"
        );

        validate_validator_announcement(&validator_address, &storage_location, signature, state)?;

        storage_locations.push(storage_location.clone());
        storage_locations.save(state)?;

        // pass the announcement to recipient
        self.recipients
            .handle_validator_announce(&validator_address, &storage_location, state)?;

        self.emit_event(
            state,
            Event::ValidatorAnnouncement {
                address: validator_address,
                storage_location,
            },
        );

        Ok(())
    }
}

fn validate_validator_announcement<S: Spec>(
    validator_address: &EthAddress,
    location: &StorageLocation,
    signature: ValidatorSignature,
    gas_meter: &mut impl GasMeter<Spec = S>,
) -> Result<()> {
    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");

    let domain_hash = DomainHash::new(
        domain,
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        gas_meter,
    )?;
    let announcement_hash = AnnouncementHash::new(domain_hash, location, gas_meter)?;
    let digest = EthSignHash::new(announcement_hash.0, gas_meter)?;

    let signature = decode_signature(&signature.0)?;
    let pub_key = ec_recover(digest.0, &signature, gas_meter)?;
    let eth_addr = eth_address_from_public_key(pub_key, gas_meter)?;

    ensure!(
        validator_address == &eth_addr,
        "Recovered address doesn't match announced address"
    );
    Ok(())
}
