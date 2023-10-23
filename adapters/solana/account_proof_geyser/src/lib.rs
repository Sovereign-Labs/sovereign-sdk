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

fn handle_confirmed_slot(
    slot: u64,
    block_accumulator: &mut HashMap<u64, BlockInfo>,
    processed_slot_account_accumulator: &mut AccountHashAccumulator,
    processed_transaction_accumulator: &mut TransactionSigAccumulator,
    pubkeys_for_proofs: &[Pubkey],
) -> anyhow::Result<Update> {
    let Some(block) = block_accumulator.get(&slot) else {
        anyhow::bail!("block not available");
    };
    let Some(num_sigs) = processed_transaction_accumulator.get(&slot) else {
        anyhow::bail!("list of txns not available");
    };
    let Some(account_hashes_data) = processed_slot_account_accumulator.get(&slot) else {
        anyhow::bail!("account hashes not available");
    };

    let num_sigs = num_sigs.clone();
    let parent_bankhash = Hash::from_str(&block.parent_bankhash).unwrap();
    let blockhash = Hash::from_str(&block.blockhash).unwrap();

    let mut account_hashes: Vec<(Pubkey, Hash)> = account_hashes_data
        .iter()
        .map(|(k, (_, v, _))| (k.clone(), v.clone()))
        .collect();

    let (inclusion, non_inclusion_left, non_inclusion_right, non_inclusion_inner) =
        get_proof_pubkeys_required(&mut account_hashes, pubkeys_for_proofs);

    let (non_inclusion_inner_adjacent_keys, non_inclusion_inner_mapping) =
        get_keys_for_non_inclusion_inner(&non_inclusion_inner, &mut account_hashes);

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

    let (accounts_delta_hash, account_proofs) =
        calculate_root_and_proofs(&mut account_hashes, &amended_leaves);

    let bank_hash = hashv(&[
        parent_bankhash.as_ref(),
        accounts_delta_hash.as_ref(),
        &num_sigs.to_le_bytes(),
        blockhash.as_ref(),
    ]);

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

    block_accumulator.remove(&slot);
    processed_slot_account_accumulator.remove(&slot);
    processed_transaction_accumulator.remove(&slot);

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
            Ok(GeyserMessage::AccountMessage(acc)) => {
                let account_hash = hash_solana_account(
                    acc.lamports,
                    acc.owner.as_ref(),
                    acc.executable,
                    acc.rent_epoch,
                    &acc.data,
                    acc.pubkey.as_ref(),
                );

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
            Ok(GeyserMessage::TransactionMessage(txn)) => {
                let slot_num = txn.slot;
                *raw_transaction_accumulator.entry(slot_num).or_insert(0) += txn.num_sigs;
            }
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
            Ok(GeyserMessage::SlotMessage(slot_info)) => match slot_info.status {
                SlotStatus::Processed => {
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
