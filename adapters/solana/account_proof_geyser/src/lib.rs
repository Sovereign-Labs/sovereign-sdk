pub mod config;
pub mod types;
pub mod utils;

use std::collections::HashMap;
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread;

use borsh::BorshSerialize;
use crossbeam_channel::{unbounded, Sender};
use log::error;
use solana_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions,
    ReplicaEntryInfoVersions, ReplicaTransactionInfoVersions, Result as PluginResult, SlotStatus,
};
use solana_sdk::clock::Slot;
use solana_sdk::hash::{hashv, Hash};
use solana_sdk::pubkey::Pubkey;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::types::{
    AccountHashAccumulator, AccountInfo, BankHashProof, BlockInfo, GeyserMessage, SlotInfo,
    TransactionInfo, TransactionSigAccumulator, Update,
};
use crate::utils::{
    assemble_account_delta_proof, calculate_root_and_proofs, get_keys_for_non_inclusion_inner,
    get_proof_pubkeys_required, hash_solana_account,
};

/// The primary goal of this function is to calculate the necessary proofs for accounts that we're interested in monitoring
/// against the BankHash. This includes both inclusion and non-inclusion proofs, so every confirmed slot results in an Update
/// being generated, either with inclusion proofs if the accounts we're interested in are modified,
/// or non-inclusion proofs if the accounts we're interested in are not modified.
///
/// This function also recalculates the BankHash to ensure that the proof generation is consistent.
///
/// The calculation is done based on accounts, transactions and blocks that are already in the "processed" commitment status.
/// Data that is in the "processed" commitment status is subject to change due to towerBFT and optimistic execution by solana.
/// The function "handle_processed_slot" takes care of updating data that is in the "processed" status if it changes.
/// This function looks at the latest data for a slot in the "processed" status when a slot is marked as "confirmed"
/// and uses it to generate the proofs required, organize them into an "Update" and return it to the caller
///
/// # Arguments
/// * `slot` - The slot number that has been confirmed.
/// * `block_accumulator` - A mutable reference to the hashmap maintaining block information.
/// * `processed_slot_account_accumulator` - A mutable reference to the hashmap tracking accounts in the processed state
/// * `processed_transaction_accumulator` - A mutable reference to the hashmap tracking transactions in the processed state
/// * `pubkeys_for_proofs` - A slice of public keys that we need to generate proofs for
///
/// # Steps
/// 1. If the slot we're trying to "confirm" has any data missing in the processed hashmaps, we "bail", because something has gone wrong.
/// 2. We extract the information from the processed hashmaps necessary for generating the BankHash
///     * Number of signatures in the block - we get this from the `processed_transaction_accumulator`
///     * Previous BankHash - This is part of the `block_accumulator`.
///     * Blockhash - This is the value of the last entry (PoH tick)
///     * Account Hashes for modified Accounts - This is part of the `processed_slot_account_accumulator`
/// 3. We extract the pubkeys for which we need to generate the proofs
///     * If the pubkey's account is modified in the current block, then we need an inclusion proof, so the pubkey is passed as is
///     * If the pubkey's account is not modified, we need a non-inclusion proof. The kind of non-inclusion proof we need depends on
///         * Non-inclusion Left (Our pubkey is smaller than the smallest pubkey at index 0)
///         * Non-inclusion Right (Our pubkey is larger than the largest pubkey at the last index)
///         * Non-inclusion inner (Our pubkey is in between. This means we need two adjacent pubkeys from the modified set)
/// 4. The pubkeys for which proofs are needed and the account hashes are passed into the `calculate_root_and_proofs` function. This returns the account_delta_hash (merkle root)
///     and merkle proofs for each of the pubkeys that we need
/// 5. Calculate BankHash based on hashing the information from step 4 and step 2.
/// 6. Assemble the merkle proofs using `assemble_account_delta_proof` to tag them as inclusion, non-inclusion and to provide additional data necessary for verification.
/// 7. Once a slot is confirmed and the necessary proofs are generated, the data from the `processed` hashmaps can be cleaned up
/// 8. Construct the "Update" struct which contains an update for a single confirmed block.
///
/// # Errors
/// This function returns an error if any of the required data for the given slot is not present in the accumulators.
///
/// # Returns
/// Returns an `anyhow::Result<Update>` which is Ok if the function succeeds or an error otherwise.
///
/// ```
fn handle_confirmed_slot(
    slot: u64,
    block_accumulator: &mut HashMap<u64, BlockInfo>,
    processed_slot_account_accumulator: &mut AccountHashAccumulator,
    processed_transaction_accumulator: &mut TransactionSigAccumulator,
    pubkeys_for_proofs: &[Pubkey],
) -> anyhow::Result<Update> {
    // Step 1: Bail if required information is not present
    let Some(block) = block_accumulator.get(&slot) else {
        anyhow::bail!("block not available");
    };
    let Some(num_sigs) = processed_transaction_accumulator.get(&slot) else {
        anyhow::bail!("list of txns not available");
    };
    let Some(account_hashes_data) = processed_slot_account_accumulator.get(&slot) else {
        anyhow::bail!("account hashes not available");
    };

    // Step 2: Extract necessary information for calculating Bankhash
    let num_sigs = num_sigs.clone();
    let parent_bankhash = Hash::from_str(&block.parent_bankhash).unwrap();
    let blockhash = Hash::from_str(&block.blockhash).unwrap();
    let mut account_hashes: Vec<(Pubkey, Hash)> = account_hashes_data
        .iter()
        .map(|(k, (_, v, _))| (k.clone(), v.clone()))
        .collect();

    // Step 3: Determine which Pubkeys we need the merkle proofs for
    let (inclusion, non_inclusion_left, non_inclusion_right, non_inclusion_inner) =
        get_proof_pubkeys_required(&mut account_hashes, pubkeys_for_proofs);

    let (non_inclusion_inner_adjacent_keys, non_inclusion_inner_mapping) =
        get_keys_for_non_inclusion_inner(&non_inclusion_inner, &mut account_hashes);

    // Based on the above calls, construct a list of pubkeys to pass to `calculate_root_and_proofs`
    let mut amended_leaves = inclusion.clone();

    if non_inclusion_left.len() > 0 {
        amended_leaves.push(account_hashes[0].0.clone());
    }
    if non_inclusion_right.len() > 0 {
        amended_leaves.push(account_hashes[account_hashes.len() - 1].0.clone());
    }
    if non_inclusion_inner.len() > 0 {
        amended_leaves.extend(non_inclusion_inner_adjacent_keys.iter().cloned());
    }

    // Step 4: Calculate Account Delta Hash (Merkle Root) and Merkle proofs for pubkeys
    let (accounts_delta_hash, account_proofs) =
        calculate_root_and_proofs(&mut account_hashes, &amended_leaves);

    // Step 5: Calculate BankHash based on accounts_delta_hash and information extracted in Step 2
    let bank_hash = hashv(&[
        parent_bankhash.as_ref(),
        accounts_delta_hash.as_ref(),
        &num_sigs.to_le_bytes(),
        blockhash.as_ref(),
    ]);

    // Step 6: Assembled raw merkle proofs into tagged variants for specific inclusion and non inclusion proofs
    // Include additional data that is needed to verify against the BankHash as well
    let proofs = assemble_account_delta_proof(
        &account_hashes,
        &account_hashes_data,
        &account_proofs,
        &inclusion,
        &non_inclusion_left,
        &non_inclusion_right,
        &non_inclusion_inner,
        &non_inclusion_inner_mapping,
    )
    .unwrap();

    // Step 7: Clean up data after proofs are generated
    block_accumulator.remove(&slot);
    processed_slot_account_accumulator.remove(&slot);
    processed_transaction_accumulator.remove(&slot);

    // Step 8: Return the `Update` structure which can be borsh serialized and passed to a client
    Ok(Update {
        slot,
        root: bank_hash,
        proof: BankHashProof {
            proofs,
            num_sigs,
            account_delta_root: accounts_delta_hash,
            parent_bankhash,
            blockhash,
        },
    })
}

fn handle_processed_slot(
    slot: u64,
    raw_slot_account_accumulator: &mut AccountHashAccumulator,
    processed_slot_account_accumulator: &mut AccountHashAccumulator,
    raw_transaction_accumulator: &mut TransactionSigAccumulator,
    processed_transaction_accumulator: &mut TransactionSigAccumulator,
) -> anyhow::Result<()> {
    transfer_slot(
        slot,
        raw_slot_account_accumulator,
        processed_slot_account_accumulator,
    );
    transfer_slot(
        slot,
        raw_transaction_accumulator,
        processed_transaction_accumulator,
    );
    Ok(())
}

fn transfer_slot<V>(slot: u64, raw: &mut HashMap<u64, V>, processed: &mut HashMap<u64, V>) {
    if let Some(entry) = raw.remove(&slot) {
        processed.insert(slot, entry);
    }
}

/// The goal of this function is to process messages passed through the geyser interface by a solana full node.
/// The message types are variants of the `GeyserMessage` enum.
///  ```
/// pub enum GeyserMessage {
///     AccountMessage(AccountInfo),
///     BlockMessage(BlockInfo),
///     TransactionMessage(TransactionInfo),
///     SlotMessage(SlotInfo),
/// }
/// ```
/// * AccountMessage: indicates a specific account is updated for a slot number (can be called multiple times for the same slot)
/// * BlockMessage: indicates a block is updated
/// * TransactionMessage: indicates a transaction is "executed" for a specific slot number.
/// * SlotMessage: indicates a change in the status of a slot. we're interested in "processed" and "confirmed"
///
///  Logic for handling each message
/// * AccountMessage
///     -> we update `raw_slot_account_accumulator` when an account update is received for a specific slot.
///     -> The hash of the account is also calculated in a way consistent with how solana calculates it
///     -> Versioning is also handled since the same account can be modified multiple times in the same slot (by multiple transactions)
/// * TransactionMessage
///     -> We only need the number of signatures in the block and since this is not directly provided, we accumulate the number of signatures
///        for each transaction for each slot into `raw_transaction_accumulator`
/// * BlockMessage
///     -> We update the `block_accumulator` hashmap with the latest info for each block.
/// * SlotMessage
///     -> When a slot is "processed", we move the information from `raw_slot_account_accumulator` and `raw_transaction_accumulator` to
///     `processed_slot_account_accumulator` and `processed_transaction_accumulator`
///         -> This is done by calling `handle_processed_slot` which moves the information to the corresponding hashmaps
///         -> Key thing to note here is that if a slot is re-processed, then the hashmap is updated so it always has the latest information
///     -> When a slot is "confirmed", we call `handle_confirmed_slot` with the latest information from `processed`
///
/// NOTE: This processing is single threaded because we rely on ordering of events. If a slot is re-processed, then
/// we expect messages in the following order
/// 1. the transactions in the new block for that slot are sent as  `TransactionMessage`
/// 2. The new account values are sent as `AccountMessage`
/// 3. `SlotMessage` is sent with processed status again.
///
/// The above 3 steps would ensure that the `processed_slot_account_accumulator` and `processed_transaction_accumulator` would contain
/// the latest values needed in preparation for when the `confirmed` message is received for a slot
///
fn process_messages(
    geyser_receiver: crossbeam::channel::Receiver<GeyserMessage>,
    tx: broadcast::Sender<Update>,
    pubkeys_for_proofs: Vec<Pubkey>,
) {
    let mut raw_slot_account_accumulator: AccountHashAccumulator = HashMap::new();
    let mut processed_slot_account_accumulator: AccountHashAccumulator = HashMap::new();

    let mut raw_transaction_accumulator: TransactionSigAccumulator = HashMap::new();
    let mut processed_transaction_accumulator: TransactionSigAccumulator = HashMap::new();

    let mut block_accumulator: HashMap<u64, BlockInfo> = HashMap::new();

    loop {
        match geyser_receiver.recv() {
            // Handle account update
            Ok(GeyserMessage::AccountMessage(acc)) => {
                let account_hash = hash_solana_account(
                    acc.lamports,
                    acc.owner.as_ref(),
                    acc.executable,
                    acc.rent_epoch,
                    &acc.data,
                    acc.pubkey.as_ref(),
                );

                // Overwrite an account if it already exists
                // Overwrite an older version with a newer version of the account data (if account is modified multiple times in the same slot)
                let write_version = acc.write_version;
                let slot = acc.slot;

                let slot_entry = raw_slot_account_accumulator
                    .entry(slot)
                    .or_insert_with(HashMap::new);

                let account_entry = slot_entry
                    .entry(acc.pubkey)
                    .or_insert_with(|| (0, Hash::default(), AccountInfo::default()));

                if write_version > account_entry.0 {
                    *account_entry = (write_version, Hash::from(account_hash), acc);
                }
            }
            // Handle transaction message. We only require the number of signatures for the purpose of calculating the BankHash
            Ok(GeyserMessage::TransactionMessage(txn)) => {
                let slot_num = txn.slot;
                *raw_transaction_accumulator.entry(slot_num).or_insert(0) += txn.num_sigs;
            }
            // Handle Block updates
            Ok(GeyserMessage::BlockMessage(block)) => {
                let slot = block.slot;
                block_accumulator.insert(
                    slot,
                    BlockInfo {
                        slot,
                        parent_bankhash: block.parent_bankhash,
                        blockhash: block.blockhash,
                        executed_transaction_count: block.executed_transaction_count,
                    },
                );
            }
            // Handle `processed` and `confirmed` slot messages.
            // `handle_processed_slot` moves from "working" hashmaps to "processed" hashmaps
            // `handle_confirmed_slot` gets the necessary proofs when a slot is "confirmed"
            Ok(GeyserMessage::SlotMessage(slot_info)) => match slot_info.status {
                SlotStatus::Processed => {
                    // handle a slot being processed.
                    // move data from raw -> processed
                    if let Err(e) = handle_processed_slot(
                        slot_info.slot,
                        &mut raw_slot_account_accumulator,
                        &mut processed_slot_account_accumulator,
                        &mut raw_transaction_accumulator,
                        &mut processed_transaction_accumulator,
                    ) {
                        error!(
                            "Error when handling processed slot {}: {:?}",
                            slot_info.slot, e
                        );
                    }
                }
                SlotStatus::Confirmed => {
                    // handle a slot being confirmed
                    // use latest information in "processed" hashmaps and generate required proofs
                    // cleanup the processed hashmaps
                    match handle_confirmed_slot(
                        slot_info.slot,
                        &mut block_accumulator,
                        &mut processed_slot_account_accumulator,
                        &mut processed_transaction_accumulator,
                        &pubkeys_for_proofs,
                    ) {
                        Ok(update) => {
                            if let Err(e) = tx.send(update) {
                                error!(
                                    "No subscribers to receive the update {}: {:?}",
                                    slot_info.slot, e
                                );
                            }
                        }
                        Err(err) => {
                            error!("{:?}", err);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

const STARTUP_END_OF_RECEIVED: u8 = 1 << 0;
const STARTUP_PROCESSED_RECEIVED: u8 = 1 << 1;

#[derive(Debug)]
pub struct PluginInner {
    startup_status: AtomicU8,
    geyser_sender: Sender<GeyserMessage>,
}

impl PluginInner {
    fn send_message(&self, message: GeyserMessage) {
        if let Err(e) = self.geyser_sender.send(message) {
            error!("error when sending message to geyser {:?}", e);
        }
    }
}

#[derive(Debug, Default)]
pub struct Plugin {
    inner: Option<PluginInner>,
}

impl Plugin {
    fn with_inner<F>(&self, f: F) -> PluginResult<()>
    where
        F: FnOnce(&PluginInner) -> PluginResult<()>,
    {
        // Before processed slot after end of startup message we will fail to construct full block
        let inner = self.inner.as_ref().expect("initialized");
        if inner.startup_status.load(Ordering::SeqCst)
            == STARTUP_END_OF_RECEIVED | STARTUP_PROCESSED_RECEIVED
        {
            f(inner)
        } else {
            Ok(())
        }
    }
}

impl GeyserPlugin for Plugin {
    fn name(&self) -> &'static str {
        "AccountProofGeyserPlugin"
    }

    fn on_load(&mut self, config_file: &str) -> PluginResult<()> {
        let config = Config::load_from_file(config_file)
            .map_err(|e| GeyserPluginError::ConfigFileReadError { msg: e.to_string() })?;
        solana_logger::setup_with_default("error");
        let (geyser_sender, geyser_receiver) = unbounded();
        let pubkeys_for_proofs: Vec<Pubkey> = config
            .account_list
            .iter()
            .map(|x| Pubkey::from_str(x).unwrap())
            .collect();

        let (tx, _rx) = broadcast::channel(32);
        let tx_process_messages = tx.clone();

        thread::spawn(move || {
            process_messages(geyser_receiver, tx_process_messages, pubkeys_for_proofs);
        });

        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let listener = TcpListener::bind(&config.bind_address).await.unwrap();
                loop {
                    let (mut socket, _) = match listener.accept().await {
                        Ok(connection) => connection,
                        Err(e) => {
                            error!("Failed to accept connection: {:?}", e);
                            continue;
                        }
                    };
                    let mut rx = tx.subscribe();
                    tokio::spawn(async move {
                        loop {
                            match rx.recv().await {
                                Ok(update) => {
                                    let data = update.try_to_vec().unwrap();
                                    let _ = socket.write_all(&data).await;
                                }
                                Err(_) => {}
                            }
                        }
                    });
                }
            });
        });

        self.inner = Some(PluginInner {
            startup_status: AtomicU8::new(0),
            geyser_sender,
        });

        Ok(())
    }

    fn on_unload(&mut self) {
        if let Some(inner) = self.inner.take() {
            drop(inner.geyser_sender);
        }
    }

    fn update_account(
        &self,
        account: ReplicaAccountInfoVersions,
        slot: Slot,
        _is_startup: bool,
    ) -> PluginResult<()> {
        self.with_inner(|inner| {
            let account = match account {
                ReplicaAccountInfoVersions::V0_0_3(a) => a,
                _ => {
                    unreachable!("Only ReplicaAccountInfoVersions::V0_0_3 is supported")
                }
            };
            let pubkey = Pubkey::try_from(account.pubkey).unwrap();
            let owner = Pubkey::try_from(account.owner).unwrap();

            let message = GeyserMessage::AccountMessage(AccountInfo {
                pubkey,
                lamports: account.lamports,
                owner,
                executable: account.executable,
                rent_epoch: account.rent_epoch,
                data: account.data.to_vec(),
                write_version: account.write_version,
                slot,
            });
            inner.send_message(message);
            Ok(())
        })
    }

    fn notify_end_of_startup(&self) -> PluginResult<()> {
        let inner = self.inner.as_ref().expect("initialized");
        inner
            .startup_status
            .fetch_or(STARTUP_END_OF_RECEIVED, Ordering::SeqCst);
        Ok(())
    }

    fn update_slot_status(
        &self,
        slot: Slot,
        _parent: Option<u64>,
        status: SlotStatus,
    ) -> PluginResult<()> {
        let inner = self.inner.as_ref().expect("initialized");
        if inner.startup_status.load(Ordering::SeqCst) == STARTUP_END_OF_RECEIVED
            && status == SlotStatus::Processed
        {
            inner
                .startup_status
                .fetch_or(STARTUP_PROCESSED_RECEIVED, Ordering::SeqCst);
        }

        self.with_inner(|inner| {
            let message = GeyserMessage::SlotMessage(SlotInfo { slot, status });
            inner.send_message(message);
            Ok(())
        })
    }

    fn notify_transaction(
        &self,
        transaction: ReplicaTransactionInfoVersions<'_>,
        slot: Slot,
    ) -> PluginResult<()> {
        self.with_inner(|inner| {
            let transaction = match transaction {
                ReplicaTransactionInfoVersions::V0_0_2(t) => t,
                _ => {
                    unreachable!("Only ReplicaTransactionInfoVersions::V0_0_2 is supported")
                }
            };

            let message = GeyserMessage::TransactionMessage(TransactionInfo {
                slot,
                num_sigs: transaction.transaction.signatures().len() as u64,
            });
            inner.send_message(message);
            Ok(())
        })
    }

    fn notify_entry(&self, _entry: ReplicaEntryInfoVersions) -> PluginResult<()> {
        Ok(())
    }

    fn notify_block_metadata(&self, blockinfo: ReplicaBlockInfoVersions<'_>) -> PluginResult<()> {
        self.with_inner(|inner| {
            let blockinfo = match blockinfo {
                ReplicaBlockInfoVersions::V0_0_2(info) => info,
                _ => {
                    unreachable!("Only ReplicaBlockInfoVersions::V0_0_1 is supported")
                }
            };

            let message = GeyserMessage::BlockMessage((blockinfo).into());
            inner.send_message(message);

            Ok(())
        })
    }

    fn account_data_notifications_enabled(&self) -> bool {
        true
    }

    fn transaction_notifications_enabled(&self) -> bool {
        true
    }

    fn entry_notifications_enabled(&self) -> bool {
        false
    }
}

#[no_mangle]
#[allow(improper_ctypes_definitions)]
/// # Safety
/// This function returns the Plugin pointer as trait GeyserPlugin.
pub unsafe extern "C" fn _create_plugin() -> *mut dyn GeyserPlugin {
    let plugin = Plugin::default();
    let plugin: Box<dyn GeyserPlugin> = Box::new(plugin);
    Box::into_raw(plugin)
}
