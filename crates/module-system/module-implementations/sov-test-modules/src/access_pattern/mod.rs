#![deny(missing_docs)]
#![doc = include_str!("./README.md")]
use std::marker::PhantomData;

use anyhow::Context as _;
use borsh::{BorshDeserialize, BorshSerialize};
use derivative::Derivative;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    AccessoryStateMap, AccessoryStateValue, AuthenticatedTransactionData, Context, CryptoSpec,
    DaSpec, GasSpec, GenesisState, MeteredBorshDeserialize, MeteredBorshDeserializeError,
    MeteredHasher, MeteredSignature, Module, ModuleId, ModuleInfo, ModuleRestApi, SafeVec,
    SizedSafeString, Spec, StateMap, StateValue, StateVec, TxHooks, TxState,
};
use strum::{EnumDiscriminants, EnumIs, VariantArray};

/// Max length of a vector for a bench pattern call message
pub const MAX_VEC_LEN_BENCH: usize = 100_000;
/// Max length of a string for a bench pattern call message
pub const MAX_STR_LEN_BENCH: usize = 1_024;

/// A newtype struct that deserializes into a string and charges gas.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct MeteredBorshDeserializeString(pub String);

impl<S: Spec> MeteredBorshDeserialize<S> for MeteredBorshDeserializeString {
    fn bias_borsh_deserialization() -> <S as Spec>::Gas {
        <S as GasSpec>::string_bias_borsh_deserialization()
    }

    fn gas_to_charge_per_byte_borsh_deserialization() -> <S as Spec>::Gas {
        <S as GasSpec>::string_gas_to_charge_per_byte_borsh_deserialization()
    }

    fn deserialize(
        buf: &mut &[u8],
        meter: &mut impl sov_modules_api::GasMeter<Spec = S>,
    ) -> Result<
        Self,
        sov_modules_api::MeteredBorshDeserializeError<<S as sov_modules_api::GasSpec>::Gas>,
    > {
        Self::charge_gas_to_deserialize(buf, meter)?;

        <MeteredBorshDeserializeString as BorshDeserialize>::deserialize(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }

    #[cfg(feature = "native")]
    fn unmetered_deserialize(
        buf: &mut &[u8],
    ) -> Result<
        Self,
        sov_modules_api::MeteredBorshDeserializeError<<S as sov_modules_api::GasSpec>::Gas>,
    > {
        <MeteredBorshDeserializeString as BorshDeserialize>::deserialize(buf)
            .map_err(MeteredBorshDeserializeError::IOError)
    }
}

/// Call message to specify storage access patterns.
#[derive(Debug, Clone, JsonSchema, EnumDiscriminants, EnumIs, Derivative, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "AccessPatternMessages")]
#[derivative(
    PartialEq(
        bound = "<S::CryptoSpec as CryptoSpec>::Signature: PartialEq, <S::CryptoSpec as CryptoSpec>::PublicKey: PartialEq"
    ),
    Eq(
        bound = "<S::CryptoSpec as CryptoSpec>::Signature: Eq, <S::CryptoSpec as CryptoSpec>::PublicKey: Eq"
    )
)]
#[strum_discriminants(name(AccessPatternDiscriminants), derive(VariantArray, EnumIs))]
pub enum AccessPatternMessages<S: Spec> {
    /// Writes `size` bytes to the module state for every position between `begin` and `begin + size`
    WriteCells {
        /// The first index to write to
        begin: u64,
        /// The number of storage cells to write to
        num_cells: u64,
        /// The size of the data to write to storage. This is the maximum number of iterations done in
        /// a string generation loop.
        data_size: usize,
    },
    /// Like [`Self::WriteCells`] but writes a custom string.
    WriteCustom {
        /// The first index to write to
        begin: u64,
        /// The content to write to the storage. Write a string to every cell from `begin`
        content: SafeVec<SizedSafeString<MAX_STR_LEN_BENCH>, MAX_VEC_LEN_BENCH>,
    },
    /// Reads every element of the module state between `begin` and `begin + size`
    ReadCells {
        /// The first index to read from
        begin: u64,
        /// The number of storage cells to read from
        num_cells: u64,
    },
    /// Hashes the string of bytes made by the repeted filler.
    HashBytes {
        /// The filler bytes to be repeated over
        filler: u8,
        /// The size of the buffer
        size: usize,
    },
    /// Hashes the custom input buffer.
    HashCustom {
        /// The input to hash
        input: SafeVec<u8, MAX_VEC_LEN_BENCH>,
    },

    /// Stores a signature to verify.
    StoreSignature {
        /// The signature to store
        sign: <S::CryptoSpec as CryptoSpec>::Signature,
        /// The associated public key
        pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
        /// The associated message
        message: SizedSafeString<MAX_STR_LEN_BENCH>,
    },
    /// Verifies the signature stored.
    VerifySignature,
    /// Verifies a custom signature, without storing it to state.
    VerifyCustomSignature {
        /// The signature to store
        sign: <S::CryptoSpec as CryptoSpec>::Signature,
        /// The associated public key
        pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
        /// The associated message
        message: SizedSafeString<MAX_STR_LEN_BENCH>,
    },

    /// Stores a string serialized as bytes.
    StoreSerializedString {
        /// The serialized string to store
        input: SafeVec<u8, MAX_VEC_LEN_BENCH>,
    },
    /// Deserializes the stored bytes into a string
    DeserializeBytesAsString,
    /// Deserializes a custom input buffer into a string without storing it to state.
    DeserializeCustomString {
        /// The serialized string to deserialize
        input: SafeVec<u8, MAX_VEC_LEN_BENCH>,
    },

    /// Deletes every element of the module state between `begin` and `begin + size`
    DeleteCells {
        /// The first index to delete from
        begin: u64,
        /// The number of storage cells to delete
        num_cells: u64,
    },
    /// Activates the pre/end-exec-hook. Adds a variable number of reads/writes for each tx.
    SetHook {
        /// The configuration of the pre-exec hooks. Set to None to disable
        pre: Option<Vec<HooksConfig>>,

        /// The configuration of the post-exec hooks. Set to None to disable
        post: Option<Vec<HooksConfig>>,
    },
    /// Updates the admin for the module.
    UpdateAdmin {
        /// New admin of the module
        new_admin: S::Address,
    },
}

/// Specifies what happens inside the pre/end-exec hook.
#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    BorshDeserialize,
    BorshSerialize,
    PartialEq,
    Eq,
    EnumDiscriminants,
    UniversalWallet,
    JsonSchema,
)]
#[strum_discriminants(derive(VariantArray))]
pub enum HooksConfig {
    /// Reads from the storage
    Read {
        /// The first index to read from
        begin: u64,
        /// The number of storage cells to read from
        size: u64,
    },
    /// Writes to the storage
    Write {
        /// The first index to write to
        begin: u64,
        /// The number of storage cells to write to
        size: u64,
        /// The size of the data to write to each storage cell
        data_size: usize,
    },
    /// Delete from the storage
    Delete {
        /// The first index to delete
        begin: u64,
        /// The number of storage cells to delete
        size: u64,
    },
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[id]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Can derive ModuleRestApi to automatically generate Rest API endpoints
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct AccessPattern<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    /// Values stored inside the module state
    #[state]
    pub values: StateMap<u64, String>,

    /// Configuration of the pre tx slot hooks
    #[state]
    pub pre_hooks: StateVec<HooksConfig>,

    /// Configuration of the post tx hooks
    #[state]
    pub post_hooks: StateVec<HooksConfig>,

    /// Last values read.
    #[state]
    pub read_values: AccessoryStateMap<u64, String>,

    /// Last value hashed.
    #[state]
    pub hashed_value: AccessoryStateValue<[u8; 32]>,

    /// Serialized bytes
    #[state]
    pub serialized_bytes: StateVec<u8>,

    /// Last value deserialized.
    #[state]
    pub deserialized_bytes: AccessoryStateValue<String>,

    /// A signature stored along the associated public key and message.
    #[state]
    #[allow(clippy::type_complexity)]
    pub signature_stored: StateValue<(
        <S::CryptoSpec as CryptoSpec>::Signature,
        <S::CryptoSpec as CryptoSpec>::PublicKey,
        String,
    )>,

    /// The last verified message signed.
    #[state]
    pub last_verified_message: AccessoryStateValue<String>,

    /// Admin of the module. Can set values, hooks.
    #[state]
    pub admin: StateValue<S::Address>,

    #[phantom]
    phantom: PhantomData<S>,
}

/// The genesis config of the access pattern module
#[derive(Debug, Clone, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
pub struct AccessPatternGenesisConfig<S: Spec> {
    /// Admin user at genesis
    pub admin: S::Address,
}

impl<S: Spec> Module for AccessPattern<S> {
    type Spec = S;

    type Config = AccessPatternGenesisConfig<S>;

    type CallMessage = AccessPatternMessages<S>;

    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        // The initialization logic
        self.admin.set(&config.admin, state).map_err(Into::into)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let admin = self
            .admin
            .get(state)
            .map_err(Into::into)?
            .expect("Admin should be set at genesis");

        if context.sender() != &admin {
            return Err(anyhow::anyhow!(
                "The transaction sender is not an admin of the access patterns module. Sender {}",
                context.sender()
            ));
        }

        self.inner_call(msg, state)
    }
}

impl<S: Spec> AccessPattern<S> {
    fn inner_call(
        &mut self,
        msg: AccessPatternMessages<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            AccessPatternMessages::WriteCells {
                begin,
                num_cells: size,
                data_size,
            } => {
                for i in begin..(begin.saturating_add(size)) {
                    self.values
                        .set(&i, &i.to_string().repeat(data_size), state)?;
                }
            }
            AccessPatternMessages::WriteCustom { begin, content } => {
                for i in begin..(begin.saturating_add(content.len() as u64)) {
                    self.values.set(
                        &i,
                        &content[i.saturating_sub(begin) as usize].to_string(),
                        state,
                    )?;
                }
            }
            AccessPatternMessages::ReadCells {
                begin,
                num_cells: size,
            } => {
                for i in begin..(begin.saturating_add(size)) {
                    let value = self.values.get(&i, state)?;

                    if let Some(value) = value {
                        self.read_values.set(&i, &value, state)?;
                    }
                }
            }
            AccessPatternMessages::DeleteCells {
                begin,
                num_cells: size,
            } => {
                for i in begin..(begin.saturating_add(size)) {
                    self.values.delete(&i, state)?;
                }
            }
            AccessPatternMessages::SetHook { pre, post: end } => {
                self.pre_hooks.clear(state)?;
                self.post_hooks.clear(state)?;

                if let Some(pre_hooks) = pre {
                    for hook in pre_hooks {
                        self.pre_hooks.push(&hook, state)?;
                    }
                }

                if let Some(post_hooks) = end {
                    for hook in post_hooks {
                        self.post_hooks.push(&hook, state)?;
                    }
                }
            }
            AccessPatternMessages::UpdateAdmin { new_admin } => {
                // Update the admin
                self.admin.set(&new_admin, state)?;
            }
            AccessPatternMessages::HashBytes { filler, size } => {
                let buf = vec![filler; size];
                let hash =
                    MeteredHasher::<_, <S::CryptoSpec as CryptoSpec>::Hasher>::digest(&buf, state)?;
                self.hashed_value.set(&hash, state)?;
            }
            AccessPatternMessages::HashCustom { input } => {
                let hash = MeteredHasher::<_, <S::CryptoSpec as CryptoSpec>::Hasher>::digest(
                    &input, state,
                )?;
                self.hashed_value.set(&hash, state)?;
            }
            AccessPatternMessages::DeserializeBytesAsString => {
                // We just exist if we don't have bytes to deserialize
                if self.serialized_bytes.len(state)? == 0 {
                    tracing::warn!(module = "access-pattern", "no bytes to deserialize");
                    return Ok(());
                }

                let serialized_bytes = self
                    .serialized_bytes
                    .iter(state)?
                    .collect::<Result<Vec<_>, _>>()?;

                let deserialized_string: MeteredBorshDeserializeString =
                    MeteredBorshDeserialize::deserialize(&mut serialized_bytes.as_ref(), state)
                        .with_context(|| {
                            "access-pattern: Impossible to deserialize the input bytes to string"
                        })?;

                self.deserialized_bytes.set(&deserialized_string.0, state)?;
            }
            AccessPatternMessages::DeserializeCustomString { input } => {
                let deserialized_string: MeteredBorshDeserializeString =
                    MeteredBorshDeserialize::deserialize(&mut input.as_ref(), state).with_context(
                        || "access-pattern: Impossible to deserialize the input bytes to string",
                    )?;

                self.deserialized_bytes.set(&deserialized_string.0, state)?;
            }
            AccessPatternMessages::StoreSerializedString { input } => {
                self.serialized_bytes.clear(state)?;

                input
                    .iter()
                    .map(|byte| self.serialized_bytes.push(byte, state))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            AccessPatternMessages::StoreSignature {
                sign,
                pub_key,
                message,
            } => {
                self.signature_stored
                    .set(&(sign, pub_key, message.to_string()), state)?;
            }
            AccessPatternMessages::VerifySignature => {
                let Some((sign, pub_key, message)) = self.signature_stored.get(state)? else {
                    // We just return if we have no signature stored.
                    tracing::warn!(module = "access-pattern", "no bytes to verify");
                    return Ok(());
                };

                MeteredSignature::new::<S>(sign)
                    .verify(&pub_key, message.as_ref(), state)
                    .with_context(|| "access-pattern: Error when verifying signature")?;

                self.last_verified_message.set(&message, state)?;
            }
            AccessPatternMessages::VerifyCustomSignature {
                sign,
                pub_key,
                message,
            } => {
                MeteredSignature::new::<S>(sign)
                    .verify(&pub_key, message.as_ref(), state)
                    .with_context(|| "access-pattern: Error when verifying signature")?;

                self.last_verified_message
                    .set(&message.to_string(), state)?;
            }
        }

        Ok(())
    }

    fn inner_hook(&mut self, hook: HooksConfig, state: &mut impl TxState<S>) -> anyhow::Result<()> {
        match hook {
            HooksConfig::Read { begin, size } => {
                for i in begin..(begin.saturating_add(size)) {
                    self.values.get(&i, state)?;
                }
            }
            HooksConfig::Write {
                begin,
                size,
                data_size,
            } => {
                for i in begin..(begin.saturating_add(size)) {
                    self.values
                        .set(&i, &i.to_string().repeat(data_size), state)?;
                }
            }
            HooksConfig::Delete { begin, size } => {
                for i in begin..(begin.saturating_add(size)) {
                    self.values.delete(&i, state)?;
                }
            }
        }

        Ok(())
    }
}

impl<S: Spec> TxHooks for AccessPattern<S> {
    type Spec = S;

    fn pre_dispatch_tx_hook<T: TxState<Self::Spec>>(
        &mut self,
        _tx: &sov_modules_api::AuthenticatedTransactionData<Self::Spec>,
        state: &mut T,
    ) -> anyhow::Result<()> {
        let curr_len = self.pre_hooks.len(state)?;

        for i in 0..curr_len {
            if let Some(hook) = self.pre_hooks.get(i, state)? {
                self.inner_hook(hook, state)?;
            }
        }

        Ok(())
    }

    fn post_dispatch_tx_hook<T: TxState<Self::Spec>>(
        &mut self,
        _tx: &AuthenticatedTransactionData<Self::Spec>,
        _ctx: &Context<Self::Spec>,
        state: &mut T,
    ) -> anyhow::Result<()> {
        let curr_len = self.post_hooks.len(state)?;

        for i in 0..curr_len {
            if let Some(hook) = self.post_hooks.get(i, state)? {
                self.inner_hook(hook, state)?;
            }
        }

        Ok(())
    }
}
