//! Implements call message generation for the StorageAccessPattern module.

use std::marker::PhantomData;
use std::sync::Arc;

use http::HttpStorageAccessClient;
use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::arbitrary::{self, Arbitrary, Unstructured};
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::{CryptoSpec, PrivateKey, SafeVec, SizedSafeString, Spec};
use sov_test_modules::access_pattern::*;
use strum::VariantArray;

use crate::interface::{CallMessageGenerator, Distribution, GeneratedMessage, MessageValidity};
use crate::{
    repeatedly, AccountState, ApplyToState, ChangelogEntry, MessageOutcome, Percent, TagAction,
    Taggable,
};

/// Messages that can be sent to the access pattern module
pub const MESSAGES: &[AccessPatternDiscriminants] = AccessPatternDiscriminants::VARIANTS;

/// The state of an access pattern account
#[derive(Debug, Clone)]
pub struct AccessPatternAccount<S: Spec> {
    pub(crate) private_key: <S::CryptoSpec as CryptoSpec>::PrivateKey,

    /// The tag changes for the account
    tag_changes: Vec<TagAction<AccessPatternTag>>,
    // TODO(@theochap) add logs verification support with hooks configuration
    // pub pre_hook: Vec<HooksConfig>,
    // pub post_hook: Vec<HooksConfig>,
}

impl<S: Spec, Data> From<&AccountState<S, Data>> for AccessPatternAccount<S> {
    fn from(value: &AccountState<S, Data>) -> AccessPatternAccount<S> {
        AccessPatternAccount {
            private_key: value.private_key.clone(),

            tag_changes: Default::default(),
        }
    }
}

impl<S: Spec, Data> ApplyToState<S, Data> for AccessPatternAccount<S> {
    fn apply_to(self, _account: &mut AccountState<S, Data>) {}
}

/// Tags of the access pattern module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessPatternTag {
    /// The account is an admin of the module
    IsAdmin,
}

impl<S: Spec> Taggable for AccessPatternAccount<S> {
    type Tag = AccessPatternTag;

    fn take_tags(&mut self) -> impl IntoIterator<Item = TagAction<Self::Tag>> {
        std::mem::take(&mut self.tag_changes)
    }

    fn add_tag(&mut self, tag: Self::Tag) {
        self.tag_changes.push(TagAction::Add(tag));
    }

    fn remove_tag(&mut self, tag: Self::Tag) {
        self.tag_changes.push(TagAction::Remove(tag));
    }
}

mod harness_interface;
pub use harness_interface::*;

mod http;

/// A message generator for the `AccessPattern` module.
#[derive(Debug, Clone)]
pub struct AccessPatternMessageGenerator<S: Spec> {
    message_distribution: Distribution<AccessPatternDiscriminants>,

    /// The maximum length of the data writen to the storage.
    maximum_write_data_length: usize,

    /// The maximum begin index of the writes to the storage.
    maximum_write_begin_index: u64,

    /// The maximum size of the writes to the storage.
    maximum_write_size: u64,

    /// Max number of hooks ops per storage pattern
    maximum_hooks_ops: u64,

    /// The genesis private key of the admin of the access pattern module
    genesis_admin_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,

    phantom: PhantomData<S>,
}

impl<S: Spec> AccessPatternMessageGenerator<S> {
    /// Creates a new [`AccessPatternMessageGenerator`]
    pub fn new(
        message_distribution: Distribution<AccessPatternDiscriminants>,
        maximum_write_length: usize,
        maximum_write_begin_index: u64,
        maximum_write_size: u64,
        maximum_hooks_ops: u64,
        admin_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    ) -> Self {
        Self {
            message_distribution,
            maximum_write_data_length: maximum_write_length,
            maximum_write_begin_index,
            maximum_write_size,
            maximum_hooks_ops,
            genesis_admin_key: admin_key,
            phantom: Default::default(),
        }
    }
}

/// A complete description of any possible state change created by the [`AccessPatternMessageGenerator`].
/// ## TODO(@theochap)
/// For now, the transaction generator doesn't know about past generations,
/// Also, add change log entries for values read/hashed/deserialized/signatures. This
/// requires tracking the previous changes made to the state (either in the logs or as part of the account).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessPatternChangeLogEntry<S: Spec> {
    /// The value array was updated
    ValueUpdated {
        /// The index where the value was updated
        item: u64,
        /// The new content of the cell. If set to None, the value has been deleted.
        content: Option<String>,
    },
    /// The pre exec hooks were updated
    PreHooksUpdated {
        /// The new values of the begin slot hook
        values: Option<Vec<HooksConfig>>,
    },
    /// The post exec hooks were updated
    PostHooksUpdated {
        /// The new values of the post slot hook
        values: Option<Vec<HooksConfig>>,
    },
    /// The module admin has been updated
    AdminUpdated {
        /// New admin of the module
        new_admin: S::Address,
    },
}

/// Discriminants used to distinguish between two access pattern change log entries
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum AccessPatternChangeLogDiscriminant {
    /// The value array was updated
    ValueUpdated {
        /// The index where the value was updated
        item: u64,
    },
    /// The pre exec hooks were updated
    PreHooksUpdated,
    /// The post exec hooks were updated
    PostHooksUpdated,
    /// The module admin has been updated
    AdminUpdated,
}

#[async_trait]
impl<S: Spec> ChangelogEntry for AccessPatternChangeLogEntry<S> {
    type ClientConfig = HttpStorageAccessClient;

    type Discriminant = AccessPatternChangeLogDiscriminant;

    async fn assert_state(
        &self,
        rollup_state_accessor: Arc<Self::ClientConfig>,
    ) -> Result<(), anyhow::Error> {
        match self {
            AccessPatternChangeLogEntry::ValueUpdated { item, content } => {
                let value = rollup_state_accessor.get_value(*item).await;

                assert_eq!(&value, content);
            }
            AccessPatternChangeLogEntry::PreHooksUpdated { values } => {
                if let Some(begin_hooks) = values {
                    for (i, hook) in begin_hooks.iter().enumerate() {
                        let stored_begin_hooks =
                            rollup_state_accessor.get_begin_hook(i as u64).await;

                        assert_eq!(stored_begin_hooks, Some(hook).copied());
                    }
                }
            }
            AccessPatternChangeLogEntry::PostHooksUpdated { values } => {
                if let Some(end_hooks) = values {
                    for (i, hook) in end_hooks.iter().enumerate() {
                        let stored_end_hooks = rollup_state_accessor.get_end_hook(i as u64).await;

                        assert_eq!(stored_end_hooks, Some(hook).copied());
                    }
                }
            }
            AccessPatternChangeLogEntry::AdminUpdated { new_admin } => {
                let http_admin = rollup_state_accessor.get_admin::<S>().await;

                assert_eq!(http_admin, Some(new_admin.clone()));
            }
        }

        Ok(())
    }

    fn as_discriminant(&self) -> Self::Discriminant {
        match self {
            AccessPatternChangeLogEntry::ValueUpdated { item, .. } => {
                AccessPatternChangeLogDiscriminant::ValueUpdated { item: *item }
            }
            AccessPatternChangeLogEntry::PreHooksUpdated { .. } => {
                AccessPatternChangeLogDiscriminant::PreHooksUpdated
            }
            AccessPatternChangeLogEntry::PostHooksUpdated { .. } => {
                AccessPatternChangeLogDiscriminant::PostHooksUpdated
            }
            AccessPatternChangeLogEntry::AdminUpdated { .. } => {
                AccessPatternChangeLogDiscriminant::AdminUpdated
            }
        }
    }
}

#[async_trait]
impl<S: Spec> CallMessageGenerator<S> for AccessPatternMessageGenerator<S> {
    type Module = AccessPattern<S>;

    type AccountView = AccessPatternAccount<S>;

    type ChangelogEntry = AccessPatternChangeLogEntry<S>;

    type Tag = AccessPatternTag;

    /// We need to generate a setup message by updating the genesis admin address.
    fn generate_setup_messages(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
    ) -> arbitrary::Result<
        Vec<crate::interface::GeneratedMessage<S, AccessPatternMessages<S>, Self::ChangelogEntry>>,
    > {
        let (new_admin, mut new_admin_acct) = generator_state.generate_account(u)?;

        new_admin_acct.add_tag(AccessPatternTag::IsAdmin);
        generator_state.update_account(&new_admin, new_admin_acct);

        Ok(vec![GeneratedMessage {
            message: AccessPatternMessages::UpdateAdmin {
                new_admin: new_admin.clone(),
            },
            sender: self.genesis_admin_key.clone(),
            outcome: MessageOutcome::Successful {
                changes: vec![AccessPatternChangeLogEntry::AdminUpdated { new_admin }],
            },
        }])
    }

    fn generate_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
        validity: crate::interface::MessageValidity,
    ) -> sov_modules_api::prelude::arbitrary::Result<
        crate::interface::GeneratedMessage<S, AccessPatternMessages<S>, Self::ChangelogEntry>,
    > {
        match validity {
            MessageValidity::Valid => self.generate_valid_call_message(u, generator_state),
            MessageValidity::Invalid => self.generate_invalid_call_message(u, generator_state),
        }
    }
}

impl<S: Spec> AccessPatternMessageGenerator<S> {
    /// Generates an invalid call message for the access pattern module
    pub fn generate_invalid_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = AccessPatternAccount<S>,
            Tag: From<AccessPatternTag>,
        >,
    ) -> arbitrary::Result<
        GeneratedMessage<S, AccessPatternMessages<S>, AccessPatternChangeLogEntry<S>>,
    > {
        let message_type = self.message_distribution.select_value(u)?;

        repeatedly!(
            let (_address, account) = generator_state.get_or_generate(Percent::fifty(), u)?;
            until: !generator_state.has_tag(&_address, AccessPatternTag::IsAdmin.into()),
            on_failure: panic!("Impossible to get a non-admin account, when there should only be one admin!")
        );

        let message = match message_type {
            AccessPatternDiscriminants::WriteCells => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let num_cells = u.int_in_range(0..=self.maximum_write_size)?;
                let data_size = u.int_in_range(0..=self.maximum_write_data_length)?;
                AccessPatternMessages::WriteCells {
                    begin,
                    num_cells,
                    data_size,
                }
            }
            AccessPatternDiscriminants::WriteCustom => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let size = u.int_in_range(0..=self.maximum_write_size)?;

                let mut contents = vec![];

                for _ in begin..(begin.saturating_add(size)) {
                    let buf_size = u.int_in_range(0..=self.maximum_write_data_length)?;
                    let mut buf = vec![0_u8; buf_size];

                    u.fill_buffer(&mut buf)?;

                    let content = hex::encode(buf);

                    contents.push(TryFrom::<String>::try_from(content.clone()).unwrap());
                }

                AccessPatternMessages::WriteCustom {
                    begin,
                    content: TryFrom::<Vec<SizedSafeString<MAX_STR_LEN_BENCH>>>::try_from(contents)
                        .unwrap(),
                }
            }
            AccessPatternDiscriminants::ReadCells => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let num_cells = u.int_in_range(0..=self.maximum_write_size)?;

                AccessPatternMessages::ReadCells { begin, num_cells }
            }
            AccessPatternDiscriminants::DeleteCells => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let num_cells = u.int_in_range(0..=self.maximum_write_size)?;

                AccessPatternMessages::DeleteCells { begin, num_cells }
            }
            AccessPatternDiscriminants::SetHook => AccessPatternMessages::SetHook {
                pre: None,
                post: None,
            },
            AccessPatternDiscriminants::HashCustom => AccessPatternMessages::HashCustom {
                input: SafeVec::new(),
            },
            AccessPatternDiscriminants::HashBytes => {
                let filler = u.int_in_range(0..=(u8::MAX - 1))?;
                let size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                AccessPatternMessages::HashBytes { filler, size }
            }
            AccessPatternDiscriminants::DeserializeBytesAsString => {
                AccessPatternMessages::DeserializeBytesAsString
            }
            AccessPatternDiscriminants::DeserializeCustomString => {
                AccessPatternMessages::DeserializeCustomString {
                    input: SafeVec::new(),
                }
            }
            AccessPatternDiscriminants::StoreSerializedString => {
                AccessPatternMessages::StoreSerializedString {
                    input: SafeVec::new(),
                }
            }
            AccessPatternDiscriminants::VerifySignature => AccessPatternMessages::VerifySignature,
            AccessPatternDiscriminants::VerifyCustomSignature => {
                let string_size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                let message = (0..string_size)
                    .map(|_| rand_ascii_char(u))
                    .collect::<Result<String, _>>()?;

                let sign = account.private_key.sign(message.as_ref());

                AccessPatternMessages::VerifyCustomSignature {
                    sign,
                    pub_key: account.private_key.pub_key(),
                    message: TryFrom::<String>::try_from(message).unwrap(),
                }
            }
            AccessPatternDiscriminants::StoreSignature => {
                let string_size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                let message = (0..string_size)
                    .map(|_| rand_ascii_char(u))
                    .collect::<Result<String, _>>()?;

                let sign = account.private_key.sign(message.as_ref());

                AccessPatternMessages::VerifyCustomSignature {
                    sign,
                    pub_key: account.private_key.pub_key(),
                    message: TryFrom::<String>::try_from(message).unwrap(),
                }
            }
            AccessPatternDiscriminants::UpdateAdmin => {
                let (new_admin, _) = generator_state.generate_account(u)?;

                AccessPatternMessages::UpdateAdmin { new_admin }
            }
        };

        Ok(GeneratedMessage {
            message,
            sender: account.private_key,
            outcome: MessageOutcome::Reverted,
        })
    }

    /// Generates valid call messages for the access pattern module
    // TODO(@theochap): this method does not accurately generate logs if the pre/post tx hooks are set.
    // Indeed, state values can be updated by these hooks which causes the logs state to be incorrect.
    // This is temporary tech debt - the implementation is straighforward (it requires using the `Data`) extension
    // of the `AccountState` - but we are postponing it for now.
    pub fn generate_valid_call_message(
        &self,
        u: &mut sov_modules_api::prelude::arbitrary::Unstructured<'_>,
        generator_state: &mut impl crate::interface::GeneratorState<
            S,
            AccountView = AccessPatternAccount<S>,
            Tag: From<AccessPatternTag>,
        >,
    ) -> arbitrary::Result<
        GeneratedMessage<S, AccessPatternMessages<S>, AccessPatternChangeLogEntry<S>>,
    > {
        let message_type = self.message_distribution.select_value(u)?;

        let (sender_addr, mut sender_acct) = generator_state
            .get_random_existing_account_with_tag(AccessPatternTag::IsAdmin.into(), u)?
            .unwrap_or_else(|| panic!("No admin for the access pattern module!"));

        match message_type {
            AccessPatternDiscriminants::WriteCells => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let num_cells = u.int_in_range(0..=self.maximum_write_size)?;
                let data_size = u.int_in_range(0..=self.maximum_write_data_length)?;

                let changes = (begin..(begin.saturating_add(num_cells)))
                    .map(|i| AccessPatternChangeLogEntry::ValueUpdated {
                        item: i,
                        content: Some(i.to_string().repeat(data_size)),
                    })
                    .collect::<Vec<_>>();

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::WriteCells {
                        begin,
                        num_cells,
                        data_size,
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes },
                })
            }
            AccessPatternDiscriminants::WriteCustom => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let size = u.int_in_range(0..=self.maximum_write_size)?;

                let mut contents = vec![];
                let mut changes = vec![];

                for i in begin..(begin.saturating_add(size)) {
                    let buf_size = u.int_in_range(0..=self.maximum_write_data_length)?;
                    let mut buf = vec![0_u8; buf_size];

                    u.fill_buffer(&mut buf)?;

                    let content = hex::encode(buf);

                    contents.push(TryFrom::<String>::try_from(content.clone()).unwrap());
                    changes.push(AccessPatternChangeLogEntry::ValueUpdated {
                        item: i,
                        content: Some(content),
                    });
                }

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::WriteCustom {
                        begin,
                        content: TryFrom::<Vec<SizedSafeString<MAX_STR_LEN_BENCH>>>::try_from(
                            contents,
                        )
                        .unwrap(),
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes },
                })
            }
            AccessPatternDiscriminants::ReadCells => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let num_cells = u.int_in_range(0..=self.maximum_write_size)?;

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::ReadCells { begin, num_cells },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes: vec![] },
                })
            }
            AccessPatternDiscriminants::DeleteCells => {
                let begin = u.int_in_range(0..=self.maximum_write_begin_index)?;
                let num_cells = u.int_in_range(0..=self.maximum_write_size)?;

                let changes = (begin..(begin.saturating_add(num_cells)))
                    .map(|i| AccessPatternChangeLogEntry::ValueUpdated {
                        item: i,
                        content: None,
                    })
                    .collect::<Vec<_>>();

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::DeleteCells { begin, num_cells },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes },
                })
            }
            AccessPatternDiscriminants::SetHook => {
                let get_arbitrary_hook: &mut dyn FnMut(
                    &mut arbitrary::Unstructured<'_>,
                )
                    -> arbitrary::Result<Vec<HooksConfig>> =
                    &mut (|u: &mut arbitrary::Unstructured<'_>| {
                        let num_hooks = u.int_in_range(0..=self.maximum_hooks_ops)?;

                        let mut hooks = Vec::with_capacity(num_hooks as usize);

                        for _ in 0..num_hooks {
                            let hook = match u.choose(HooksConfigDiscriminants::VARIANTS)? {
                                HooksConfigDiscriminants::Read => {
                                    let begin =
                                        u.int_in_range(0..=self.maximum_write_begin_index)?;
                                    let size = u.int_in_range(0..=self.maximum_write_size)?;

                                    HooksConfig::Read { begin, size }
                                }
                                HooksConfigDiscriminants::Write => {
                                    let begin =
                                        u.int_in_range(0..=self.maximum_write_begin_index)?;
                                    let size = u.int_in_range(0..=self.maximum_write_size)?;
                                    let data_size =
                                        u.int_in_range(0..=self.maximum_write_data_length)?;

                                    HooksConfig::Write {
                                        begin,
                                        size,
                                        data_size,
                                    }
                                }
                                HooksConfigDiscriminants::Delete => {
                                    let begin =
                                        u.int_in_range(0..=self.maximum_write_begin_index)?;
                                    let size = u.int_in_range(0..=self.maximum_write_size)?;

                                    HooksConfig::Delete { begin, size }
                                }
                            };

                            hooks.push(hook);
                        }

                        Ok(hooks)
                    });

                // Selects if we set the pre-hooks
                let pre = if bool::arbitrary(u)? {
                    Some(get_arbitrary_hook(u)?)
                } else {
                    None
                };

                let post = if bool::arbitrary(u)? {
                    Some(get_arbitrary_hook(u)?)
                } else {
                    None
                };

                let changes = vec![
                    AccessPatternChangeLogEntry::PreHooksUpdated {
                        values: pre.clone(),
                    },
                    AccessPatternChangeLogEntry::PostHooksUpdated {
                        values: post.clone(),
                    },
                ];

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::SetHook { pre, post },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes },
                })
            }
            AccessPatternDiscriminants::HashCustom => {
                let num_bytes = u.int_in_range(0..=self.maximum_write_size)?;
                let hash = (0..num_bytes)
                    .map(|_| u8::arbitrary(u))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::HashCustom {
                        input: TryFrom::<Vec<u8>>::try_from(hash).unwrap(),
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes: vec![] },
                })
            }
            AccessPatternDiscriminants::HashBytes => {
                let filler = u.int_in_range(0..=(u8::MAX - 1))?;
                let size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::HashBytes { filler, size },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes: vec![] },
                })
            }
            AccessPatternDiscriminants::DeserializeCustomString => {
                let string_size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                let input = (0..string_size)
                    .map(|_| rand_ascii_char(u))
                    .collect::<Result<String, _>>()?;

                let serialized_string = borsh::to_vec(&MeteredBorshDeserializeString(input))
                    .expect("Impossible to serialize string");

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::DeserializeCustomString {
                        input: TryFrom::<Vec<u8>>::try_from(serialized_string).unwrap(),
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes: vec![] },
                })
            }
            AccessPatternDiscriminants::StoreSerializedString => {
                let string_size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                let input = (0..string_size)
                    .map(|_| rand_ascii_char(u))
                    .collect::<Result<String, _>>()?;

                let serialized_string = borsh::to_vec(&MeteredBorshDeserializeString(input))
                    .expect("Impossible to serialize string");

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::StoreSerializedString {
                        input: TryFrom::<Vec<u8>>::try_from(serialized_string).unwrap(),
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes: vec![] },
                })
            }
            AccessPatternDiscriminants::DeserializeBytesAsString => Ok(GeneratedMessage {
                message: AccessPatternMessages::DeserializeBytesAsString,
                sender: sender_acct.private_key,
                outcome: MessageOutcome::Successful { changes: vec![] },
            }),
            AccessPatternDiscriminants::VerifyCustomSignature => {
                let string_size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                let message = (0..string_size)
                    .map(|_| rand_ascii_char(u))
                    .collect::<Result<String, _>>()?;

                let sign = sender_acct.private_key.sign(message.as_ref());

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::VerifyCustomSignature {
                        sign,
                        pub_key: sender_acct.private_key.pub_key(),
                        message: TryFrom::<String>::try_from(message).unwrap(),
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes: vec![] },
                })
            }
            AccessPatternDiscriminants::StoreSignature => {
                let string_size = u.int_in_range(0..=self.maximum_write_size)? as usize;

                let message = (0..string_size)
                    .map(|_| rand_ascii_char(u))
                    .collect::<Result<String, _>>()?;

                let sign = sender_acct.private_key.sign(message.as_ref());

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::StoreSignature {
                        sign,
                        pub_key: sender_acct.private_key.pub_key(),
                        message: TryFrom::<String>::try_from(message).unwrap(),
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful { changes: vec![] },
                })
            }
            AccessPatternDiscriminants::VerifySignature => Ok(GeneratedMessage {
                message: AccessPatternMessages::VerifySignature,
                sender: sender_acct.private_key,
                outcome: MessageOutcome::Successful { changes: vec![] },
            }),
            AccessPatternDiscriminants::UpdateAdmin => {
                let (new_admin, mut new_admin_account) = generator_state.generate_account(u)?;

                sender_acct.remove_tag(AccessPatternTag::IsAdmin);
                new_admin_account.add_tag(AccessPatternTag::IsAdmin);

                generator_state.update_account(&sender_addr, sender_acct.clone());
                generator_state.update_account(&new_admin, new_admin_account);

                Ok(GeneratedMessage {
                    message: AccessPatternMessages::UpdateAdmin {
                        new_admin: new_admin.clone(),
                    },
                    sender: sender_acct.private_key,
                    outcome: MessageOutcome::Successful {
                        changes: vec![AccessPatternChangeLogEntry::AdminUpdated { new_admin }],
                    },
                })
            }
        }
    }
}

/// Generates a random ASCII character
fn rand_ascii_char(u: &mut Unstructured<'_>) -> Result<char, arbitrary::Error> {
    Ok(u.int_in_range(32..=126)? as u8 as char)
}
