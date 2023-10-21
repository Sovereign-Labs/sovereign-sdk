use std::fmt::{Debug, Formatter};
use solana_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfoVersions, ReplicaBlockInfoVersions,
    ReplicaEntryInfoVersions, ReplicaTransactionInfoVersions, Result as PluginResult,
    ReplicaBlockInfoV2,
    SlotStatus};
use crossbeam_channel::{select, Receiver, Sender, unbounded};
use solana_sdk::clock::Slot;
use solana_sdk::hash::{hashv, Hash};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::str::FromStr;
use std::thread;
use lru::LruCache;
use std::collections::{HashMap, HashSet};
use std::ptr::addr_of_mut;
use log::{error, info};
use blake3::traits::digest::Digest;
use solana_runtime::accounts_hash::AccountsHasher;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc};

type AccountHashAccumulator = HashMap<u64, HashMap<Pubkey, (u64, Hash)>>;
type TransactionSigAccumulator = HashMap<u64, u64>;

/// Util helper function to calculate the hash of a solana account
/// https://github.com/solana-labs/solana/blob/v1.16.15/runtime/src/accounts_db.rs#L6076-L6118
/// We can see as we make the code more resilient to see if we can also make
/// the structures match and use the function from solana-sdk, but currently it seems a bit more
/// complicated and lower priority, since getting a stable version working is top priority
pub fn hash_solana_account(
    lamports: u64,
    owner: &[u8],
    executable: bool,
    rent_epoch: u64,
    data: &[u8],
    pubkey: &[u8],
) -> [u8; 32] {
    if lamports == 0 {
        return [08; 32];
    }
    let mut hasher = blake3::Hasher::new();

    hasher.update(&lamports.to_le_bytes());
    hasher.update(&rent_epoch.to_le_bytes());
    hasher.update(data);

    if executable {
        hasher.update(&[1u8; 1]);
    } else {
        hasher.update(&[0u8; 1]);
    }
    hasher.update(owner.as_ref());
    hasher.update(pubkey.as_ref());

    hasher.finalize().into()
}

pub fn calculate_root(pubkey_hash_vec: Vec<(Pubkey, Hash)>) -> Hash {
    AccountsHasher::accumulate_account_hashes(pubkey_hash_vec)
}


#[derive(Debug, Clone)]
pub struct AccountInfo {
    /// The Pubkey for the account
    pub pubkey: Pubkey,

    /// The lamports for the account
    pub lamports: u64,

    /// The Pubkey of the owner program account
    pub owner: Pubkey,

    /// This account's data contains a loaded program (and is now read-only)
    pub executable: bool,

    /// The epoch at which this account will next owe rent
    pub rent_epoch: u64,

    /// The data held in this account.
    pub data: Vec<u8>,

    /// A global monotonically increasing atomic number, which can be used
    /// to tell the order of the account update. For example, when an
    /// account is updated in the same slot multiple times, the update
    /// with higher write_version should supersede the one with lower
    /// write_version.
    pub write_version: u64,

    /// Slot number for this update
    pub slot: u64,
}

#[derive(Debug, Clone)]
pub struct TransactionInfo {
    pub slot: u64,
    pub num_sigs: u64,
}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub slot: u64,
    pub parent_bankhash: String,
    pub blockhash: String,
    pub executed_transaction_count: u64
}

impl<'a> From<&'a ReplicaBlockInfoV2<'a>> for BlockInfo {
    fn from(block: &'a ReplicaBlockInfoV2<'a>) -> Self {
        Self {
            slot: block.slot,
            parent_bankhash: block.parent_blockhash.to_string(),
            blockhash: block.blockhash.to_string(),
            executed_transaction_count: block.executed_transaction_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub slot: u64,
    pub status: SlotStatus
}

#[derive(Debug, Clone)]
enum GeyserMessage {
    AccountMessage(AccountInfo),
    BlockMessage(BlockInfo),
    TransactionMessage(TransactionInfo),
    SlotMessage(SlotInfo)
}

fn handle_confirmed_slot(slot: u64,
                         block_accumulator: &mut HashMap<u64, BlockInfo>,
                         processed_slot_account_accumulator: &mut AccountHashAccumulator,
                         processed_transaction_accumulator: &mut TransactionSigAccumulator) -> anyhow::Result<()> {
    let Some(block) = block_accumulator.get(&slot) else {
        anyhow::bail!("block not available");
    };
    let Some(num_sigs) = processed_transaction_accumulator.get(&slot) else {
        anyhow::bail!("list of txns not available");
    };
    let Some(account_hashes) = processed_slot_account_accumulator.get(&slot) else {
        anyhow::bail!("account hashes not available");
    };

    let parent_bankhash = Hash::from_str(&block.parent_bankhash).unwrap();
    let blockhash = Hash::from_str(&block.blockhash).unwrap();

    let accounts_delta_hash = calculate_root(account_hashes.iter().map(|(k, (version, v))| (k.clone(), v.clone())).collect());
    let bank_hash = hashv(&[
        parent_bankhash.as_ref(),
        accounts_delta_hash.as_ref(),
        &num_sigs.to_le_bytes(),
        blockhash.as_ref()
    ]);

    info!("=====> CALCULATED: {:?}: {:?} ", slot, bank_hash);
    info!("=====> GEYSER DIRECT: {:?}: {:?} ", slot-1, parent_bankhash);

    block_accumulator.remove(&slot);
    processed_slot_account_accumulator.remove(&slot);
    processed_transaction_accumulator.remove(&slot);

    Ok(())
}

fn handle_processed_slot(slot: u64,
                         raw_slot_account_accumulator: &mut AccountHashAccumulator,
                         processed_slot_account_accumulator: &mut AccountHashAccumulator,
                         raw_transaction_accumulator: &mut TransactionSigAccumulator,
                         processed_transaction_accumulator: &mut TransactionSigAccumulator)
                         -> anyhow::Result<()> {
    transfer_slot(slot, raw_slot_account_accumulator, processed_slot_account_accumulator);
    transfer_slot(slot, raw_transaction_accumulator, processed_transaction_accumulator);
    Ok(())
}

fn transfer_slot<V>(
    slot: u64,
    raw: &mut HashMap<u64, V>,
    processed: &mut HashMap<u64, V>,

) {
    if let Some(entry) = raw.remove(&slot) {
        processed.insert(slot, entry);
    }
}

fn process_messages(
    geyser_receiver: crossbeam::channel::Receiver<GeyserMessage>
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

                let slot_entry = raw_slot_account_accumulator.entry(slot).or_insert_with(HashMap::new);

                let account_entry = slot_entry.entry(acc.pubkey).or_insert_with(|| (0, Hash::default()));

                if write_version > account_entry.0 {
                    *account_entry = (write_version, Hash::from(account_hash));
                }
            }
            Ok(GeyserMessage::TransactionMessage(txn)) => {
                let slot_num = txn.slot;
                // let inner_map = raw_transaction_accumulator.entry(slot_num).or_default();
                // inner_map.entry(txn.identifier.clone()).or_insert(txn);
                *raw_transaction_accumulator.entry(slot_num).or_insert(0) += txn.num_sigs;
            }
            Ok(GeyserMessage::BlockMessage(block)) => {
                let slot = block.slot;
                block_accumulator.insert(slot, BlockInfo {
                    slot,
                    parent_bankhash: block.parent_bankhash,
                    blockhash: block.blockhash,
                    executed_transaction_count: block.executed_transaction_count
                });
            }
            Ok(GeyserMessage::SlotMessage(slot_info)) => {
                match slot_info.status {
                    SlotStatus::Processed => {
                        handle_processed_slot(slot_info.slot,
                                              &mut raw_slot_account_accumulator,
                                              &mut processed_slot_account_accumulator,
                                              &mut raw_transaction_accumulator,
                                              &mut processed_transaction_accumulator);
                    }
                    SlotStatus::Confirmed => {
                        handle_confirmed_slot(slot_info.slot,
                                              &mut block_accumulator ,
                                              &mut processed_slot_account_accumulator ,
                                              &mut processed_transaction_accumulator);
                    }
                    _ => {}
                }
            }
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
        self.geyser_sender.send(message);
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

    fn on_load(&mut self, _config_file: &str) -> PluginResult<()> {
        solana_logger::setup_with_default("error");
        let (geyser_sender, geyser_receiver) = unbounded();

        thread::spawn(move || {
            process_messages(
                geyser_receiver
            );
        });

        self.inner = Some(PluginInner {
            startup_status: AtomicU8::new(0),
            geyser_sender
        });


        Ok(())
    }

    fn on_unload(&mut self) {
        if let Some(inner) = self.inner.take() {
            drop(inner.geyser_sender);
        }
    }

    fn update_account(&self, account: ReplicaAccountInfoVersions, slot: Slot, _is_startup: bool) -> PluginResult<()> {
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

    fn update_slot_status(&self, slot: Slot, parent: Option<u64>, status: SlotStatus) -> PluginResult<()> {
        let inner = self.inner.as_ref().expect("initialized");
        if inner.startup_status.load(Ordering::SeqCst) == STARTUP_END_OF_RECEIVED
            && status == SlotStatus::Processed
        {
            inner
                .startup_status
                .fetch_or(STARTUP_PROCESSED_RECEIVED, Ordering::SeqCst);
        }

        self.with_inner(|inner| {
            let message = GeyserMessage::SlotMessage(SlotInfo{ slot, status });
            inner.send_message(message);
            Ok(())
        })
    }

    fn notify_transaction(&self, transaction: ReplicaTransactionInfoVersions<'_>, slot: Slot) -> PluginResult<()> {
        self.with_inner(|inner| {
            let transaction = match transaction {
                ReplicaTransactionInfoVersions::V0_0_2(t) => t,
                _ => {
                    unreachable!("Only ReplicaTransactionInfoVersions::V0_0_2 is supported")
                }
            };

            let message = GeyserMessage::TransactionMessage(TransactionInfo { slot,
                num_sigs: transaction.transaction.signatures().len() as u64 });
            inner.send_message(message);
            Ok(())
        })
    }

    fn notify_entry(&self, entry: ReplicaEntryInfoVersions) -> PluginResult<()> {
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
