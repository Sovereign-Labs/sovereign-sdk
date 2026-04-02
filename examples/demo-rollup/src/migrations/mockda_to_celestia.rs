use std::cmp::min;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{bail, Context};
use borsh::BorshDeserialize;
use clap::Parser;
use demo_stf::runtime::Runtime;
use demo_stf::MultiAddressEvmSolana;
use rockbound::SchemaBatch;
use serde::Serialize;
use sov_celestia_adapter::types::TmHash;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::tables::{BatchByNumber, SlotByNumber};
use sov_db::schema::types::{BatchNumber, DbBytes, StoredBatch};
use sov_db::storage_manager::NomtStorageManager;
use sov_full_node_configs::runner::from_toml_path;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{MockAddress, MockDaSpec};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{BatchSequencerReceipt, ModuleInfo, Spec, StateCheckpoint, StateWriter};
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_paymaster::{AuthorizedSequencers, PaymasterPolicy};
use sov_rollup_interface::common::SlotNumber;
use sov_sequencer_registry::KnownSequencer;
use sov_state::{
    BcsCodec, BorshCodec, Kernel, NativeStorage, Prefix, SlotKey, SlotKeyFromCodec, SlotValue,
    SlotValueFromCodec, StateItemDecoder, StateUpdate, User,
};
use sov_stf_runner::RollupConfig;

use sov_demo_rollup::{CelestiaDemoRollup, MockDemoRollup};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Offline migration tool for moving a demo rollup DB from MockDA sequencer state to Celestia sequencer state"
)]
struct Args {
    /// Path to the rollup config used by the old MockDA node.
    /// This is used to read the exact storage options so NOMT can be opened safely.
    #[arg(long)]
    rollup_config_path: PathBuf,

    /// Override `storage.path` from the rollup config.
    #[arg(long)]
    db_path: Option<PathBuf>,

    /// The new sequencer DA address on Celestia (bech32, e.g. celestia1...).
    /// Required for single-sequencer migration mode (when `--sequencer-map` is not used).
    #[arg(long)]
    new_sequencer_da_address: Option<String>,

    /// Explicit old->new DA mapping for multi-sequencer migration.
    /// Repeat this flag once per known old sequencer.
    /// Format: `<old_mockda_hex>=<new_celestia_bech32>`.
    #[arg(long, value_name = "OLD=NEW")]
    sequencer_map: Vec<SequencerMapArg>,

    /// Optional expected old MockDA sequencer address (single-sequencer mode only).
    /// If provided, it must match the single known sequencer entry in the DB.
    #[arg(long)]
    old_sequencer_da_address: Option<String>,

    /// Optional override for the sequencer rollup address (single-sequencer mode only).
    /// If omitted, preserves the old rollup address from the selected source sequencer.
    #[arg(long)]
    new_sequencer_rollup_address: Option<String>,

    /// Rollup slot at cutover.
    ///
    /// If not provided, the script will use the latest rollup slot from the DB.
    rollup_slot_at_cutover: Option<u64>,

    /// Celestia height at cutover.
    /// Required for all migrations.
    #[arg(
        long = "celestia-height-at-cutover",
        visible_alias = "cutover-celestia-height"
    )]
    celestia_height_at_cutover: u64,

    /// If present, non-`AuthorizedSequencers::All` paymaster policies are deleted.
    /// Otherwise, migration fails if any non-`All` policies are found.
    #[arg(long, default_value_t = false)]
    force_non_all_paymaster_policies: bool,

    /// Analyze and compute the post-migration root, but do not commit changes to disk.
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Optional path to write the JSON migration report.
    #[arg(long)]
    report_out: Option<PathBuf>,
}

type OldSpec = <MockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
type NewSpec = <CelestiaDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
type Hasher = <<OldSpec as Spec>::CryptoSpec as sov_modules_api::CryptoSpec>::Hasher;
type OldStorage = <OldSpec as Spec>::Storage;
type MockStorageManager = NomtStorageManager<MockDaSpec, Hasher, OldStorage>;
type OldChainState = sov_chain_state::ChainState<OldSpec>;
type OldSlotInformation = sov_chain_state::SlotInformation<OldSpec>;
type NewSlotInformation = sov_chain_state::SlotInformation<NewSpec>;
type OldBlobStorage = sov_blob_storage::BlobStorage<OldSpec>;
type OldSequencerRegistry = sov_sequencer_registry::SequencerRegistry<OldSpec>;

#[derive(Clone)]
struct KnownSequencerEntry {
    slot_key: SlotKey,
    da_address: MockAddress,
    known: KnownSequencer<OldSpec>,
}

#[derive(Clone, Debug)]
struct SequencerMapArg {
    old_da_address: MockAddress,
    new_da_address: CelestiaAddress,
}

impl FromStr for SequencerMapArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((old, new)) = value.split_once('=') else {
            return Err(format!(
                "invalid sequencer mapping '{value}', expected format OLD=NEW"
            ));
        };

        let old_da_address = MockAddress::from_str(old.trim())
            .map_err(|e| format!("invalid old MockDA address '{old}': {e}"))?;
        let new_da_address = CelestiaAddress::from_str(new.trim())
            .map_err(|e| format!("invalid new Celestia address '{new}': {e}"))?;

        Ok(Self {
            old_da_address,
            new_da_address,
        })
    }
}

#[derive(Clone)]
struct ResolvedSequencerEntry {
    old_slot_key: SlotKey,
    old_da_address: MockAddress,
    old_known: KnownSequencer<OldSpec>,
    new_da_address: CelestiaAddress,
    new_rollup_address: <NewSpec as Spec>::Address,
}

struct ResolvedSequencerPlan {
    entries: Vec<ResolvedSequencerEntry>,
    preferred_after: Option<CelestiaAddress>,
}

#[derive(Serialize)]
struct SequencerMappingReportEntry {
    old_da_address: String,
    old_rollup_address: String,
    old_balance: String,
    old_balance_state: String,
    new_da_address: String,
    new_rollup_address: String,
}

#[derive(Serialize)]
struct MigrationReport {
    dry_run: bool,
    db_path: String,
    pre_state_root: String,
    post_state_root: String,
    db_head_rollup_slot: Option<u64>,
    rollup_slot_at_cutover: u64,
    rollup_slot_at_cutover_source: String,
    known_sequencers_removed: usize,
    known_sequencers_added: usize,
    sequencer_mappings: Vec<SequencerMappingReportEntry>,
    preferred_sequencer_before: Option<String>,
    preferred_sequencer_after: Option<String>,
    batch_receipts_scanned: usize,
    batch_receipts_rewritten: usize,
    batch_receipts_already_new: usize,
    chain_state_slots_user_reencoded: usize,
    chain_state_slots_kernel_reencoded: usize,
    deferred_blobs_removed: usize,
    sequencer_to_payer_removed: usize,
    paymaster_payers_total: usize,
    paymaster_non_all_found: usize,
    paymaster_non_all_policies_deleted: usize,
    genesis_da_height_before: u64,
    genesis_da_height_after: u64,
    notes: Vec<String>,
}

#[derive(Default)]
struct BatchReceiptMigrationStats {
    scanned: usize,
    rewritten: usize,
    already_new: usize,
}

#[derive(Clone, Copy)]
struct ModuleDiscriminants {
    chain_state: u8,
    blob_storage: u8,
    sequencer_registry: u8,
}

impl ModuleDiscriminants {
    fn from_runtime(runtime: &Runtime<OldSpec>) -> Self {
        Self {
            chain_state: runtime.chain_state.discriminant(),
            blob_storage: runtime.blob_storage.discriminant(),
            sequencer_registry: runtime.sequencer_registry.discriminant(),
        }
    }
}

struct ParsedAddressArgs {
    legacy_new_da_address: Option<CelestiaAddress>,
    requested_old_da_address: Option<MockAddress>,
    requested_new_rollup_address: Option<MultiAddressEvmSolana>,
}

struct MigrationSession {
    db_path: PathBuf,
    runtime: Runtime<OldSpec>,
    module_discriminants: ModuleDiscriminants,
    storage_manager: MockStorageManager,
    storage: OldStorage,
    ledger_db: LedgerDb,
    head_slot_number: SlotNumber,
    db_head_rollup_slot: Option<u64>,
}

struct CutoverCalibration {
    rollup_slot_at_cutover: u64,
    rollup_slot_at_cutover_source: String,
    calibrated_genesis_da_height: u64,
}

struct SequencerContext {
    plan: ResolvedSequencerPlan,
    preferred_before: Option<MockAddress>,
    preferred_slot_key: SlotKey,
}

struct PaymasterInspection {
    sequencer_to_payer_entries: Vec<(SlotKey, SlotValue)>,
    paymaster_payers_entries: Vec<(SlotKey, SlotValue)>,
    non_all_policy_payers: Vec<String>,
    deleted_non_all_paymaster_policies: Vec<SlotKey>,
}

struct BlobStorageInspection {
    deferred_blobs_entries: Vec<(SlotKey, SlotValue)>,
}

struct ChainStateSlotsInspection {
    user_slots_entries: Vec<(SlotKey, OldSlotInformation)>,
    kernel_slots_entries: Vec<(SlotKey, OldSlotInformation)>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("migration failed: {err:#}");
        std::process::exit(1);
    }
}

/// Performs an offline MockDA -> Celestia migration over the latest finalized DB state.
///
/// The script preserves user/application state and rewrites only DA-coupled system state:
/// - `sequencer_registry.known_sequencers` and `sequencer_registry.preferred_sequencer`
///   (MockDA addresses are replaced with Celestia addresses via CLI mapping).
/// - `chain_state.slots` (re-encoded from MockDA `SlotHash` to Celestia `SlotHash` while preserving
///   gas metadata and pre-state roots).
/// - `blob_storage.deferred_blobs` (cleared because any deferred blobs would have an incompatible DA address type after migration).
/// - `paymaster.sequencer_to_payer` (cleared because its keys are DA addresses).
/// - `paymaster.payers` entries with non-`AuthorizedSequencers::All` can be deleted
///   under `--force-non-all-paymaster-policies` because those policies may embed DA addresses.
/// - historical ledger `BatchByNumber.receipt` entries are rewritten from
///   `BatchSequencerReceipt<OldSpec>` to `BatchSequencerReceipt<NewSpec>` by remapping
///   sequencer DA addresses (MockDA -> Celestia) using the resolved sequencer mapping.
///
/// It overwrites `chain_state.genesis_da_height` using cutover calibration
/// (`celestia_height_at_cutover - rollup_slot_at_cutover`) so DA/rollup height math remains
/// consistent after cutover.
///
/// Finally, it commits rewritten state in-place at the current head version and patches ledger
/// head `StoredSlot.state_root` to the migrated root so storage/ledger roots remain aligned.
fn run() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut notes = Vec::new();
    let parsed_address_args = parse_address_args(&args)?;
    let MigrationSession {
        db_path,
        mut runtime,
        module_discriminants,
        mut storage_manager,
        storage,
        ledger_db,
        head_slot_number,
        db_head_rollup_slot,
    } = prepare_migration_session(&args, &mut notes)?;

    let cutover = resolve_cutover_calibration(&args, db_head_rollup_slot, &mut notes)?;

    let pre_state_root = storage
        .get_root_hash(head_slot_number)
        .context("failed to read pre-migration state root")?;

    let sequencer_context = build_sequencer_context(
        &storage,
        &args,
        &parsed_address_args,
        module_discriminants,
        &mut notes,
    )?;
    let preferred_after = sequencer_context
        .plan
        .preferred_after
        .as_ref()
        .map(ToString::to_string);

    let paymaster_inspection = inspect_paymaster_state(
        &storage,
        *runtime.paymaster.sequencer_to_payer.prefix(),
        *runtime.paymaster.payers.prefix(),
        args.force_non_all_paymaster_policies,
    )?;
    let chain_state_slots_inspection =
        inspect_chain_state_slots(&storage, module_discriminants, &mut notes)?;
    let blob_storage_inspection =
        inspect_blob_storage_state(&storage, module_discriminants, &mut notes)?;

    let (chain_state_genesis_da_height_key, genesis_da_height_before) =
        read_chain_state_genesis_da_height(&storage, module_discriminants)?;
    add_genesis_da_height_notes(
        cutover.calibrated_genesis_da_height,
        genesis_da_height_before,
        &mut notes,
    );

    let known_prefix = Prefix::new(
        module_discriminants.sequencer_registry,
        OldSequencerRegistry::KNOWN_SEQUENCERS_ITEM_DISCRIMINANT,
    );
    let mut checkpoint = StateCheckpoint::new(storage, &runtime.kernel(), None);
    apply_sequencer_registry_updates(
        &mut checkpoint,
        &sequencer_context.plan,
        known_prefix,
        &sequencer_context.preferred_slot_key,
    )?;
    apply_paymaster_updates(
        &mut checkpoint,
        &paymaster_inspection.sequencer_to_payer_entries,
        &paymaster_inspection.deleted_non_all_paymaster_policies,
    )?;
    apply_blob_storage_updates(
        &mut checkpoint,
        &blob_storage_inspection.deferred_blobs_entries,
    )?;
    apply_genesis_da_height_update(
        &mut checkpoint,
        &chain_state_genesis_da_height_key,
        cutover.calibrated_genesis_da_height,
    )?;
    apply_chain_state_slots_updates(
        &mut checkpoint,
        &chain_state_slots_inspection.user_slots_entries,
        &chain_state_slots_inspection.kernel_slots_entries,
    )?;

    let (next_state_root, mut state_update, accessory_delta, _witness, storage_after) =
        checkpoint.materialize_update(pre_state_root);
    state_update.add_accessory_items(accessory_delta.freeze());
    let change_set = storage_after.materialize_changes_at_version(state_update, head_slot_number);
    let (batch_receipt_ledger_patch, batch_receipt_stats) =
        make_batch_receipt_patch(&ledger_db, &sequencer_context.plan, &mut notes)?;

    let post_state_root = if args.dry_run {
        next_state_root
    } else {
        let mut ledger_change_set = make_ledger_root_patch(&ledger_db, next_state_root.as_ref())?;
        ledger_change_set.merge(batch_receipt_ledger_patch);
        storage_manager
            .commit_migration_change_set_at_head(head_slot_number, change_set, ledger_change_set)
            .context("failed to commit migration changeset at head version")?;
        let (post_storage, post_ledger_reader) = storage_manager
            .create_state_for_migration()
            .context("failed to create post-migration storage view")?;
        let post_ledger_db = LedgerDb::with_reader(post_ledger_reader)
            .context("failed to initialize post-migration ledger db view")?;
        let post_head_slot_number = assert_storage_latest_version_matches_ledger_head(
            &post_storage,
            &post_ledger_db,
            "post-migration",
        )?;
        if post_head_slot_number != head_slot_number {
            bail!(
                "post-migration head slot changed unexpectedly: expected {}, found {}",
                head_slot_number,
                post_head_slot_number
            );
        }
        assert_ledger_head_state_root_matches_storage_root(
            &post_storage,
            &post_ledger_db,
            "post-migration",
        )?;

        post_storage
            .get_root_hash(post_head_slot_number)
            .context("failed to read post-migration state root")?
    };

    let sequencer_mappings = sequencer_context
        .plan
        .entries
        .iter()
        .map(|entry| SequencerMappingReportEntry {
            old_da_address: entry.old_da_address.to_string(),
            old_rollup_address: entry.old_known.address.to_string(),
            old_balance: entry.old_known.balance.to_string(),
            old_balance_state: format!("{:?}", entry.old_known.balance_state),
            new_da_address: entry.new_da_address.to_string(),
            new_rollup_address: entry.new_rollup_address.to_string(),
        })
        .collect();

    let report = MigrationReport {
        dry_run: args.dry_run,
        db_path: db_path.display().to_string(),
        pre_state_root: pre_state_root.to_string(),
        post_state_root: post_state_root.to_string(),
        db_head_rollup_slot,
        rollup_slot_at_cutover: cutover.rollup_slot_at_cutover,
        rollup_slot_at_cutover_source: cutover.rollup_slot_at_cutover_source.clone(),
        known_sequencers_removed: sequencer_context.plan.entries.len(),
        known_sequencers_added: sequencer_context.plan.entries.len(),
        sequencer_mappings,
        preferred_sequencer_before: sequencer_context.preferred_before.map(|x| x.to_string()),
        preferred_sequencer_after: preferred_after,
        batch_receipts_scanned: batch_receipt_stats.scanned,
        batch_receipts_rewritten: batch_receipt_stats.rewritten,
        batch_receipts_already_new: batch_receipt_stats.already_new,
        chain_state_slots_user_reencoded: chain_state_slots_inspection.user_slots_entries.len(),
        chain_state_slots_kernel_reencoded: chain_state_slots_inspection.kernel_slots_entries.len(),
        deferred_blobs_removed: blob_storage_inspection.deferred_blobs_entries.len(),
        sequencer_to_payer_removed: paymaster_inspection.sequencer_to_payer_entries.len(),
        paymaster_payers_total: paymaster_inspection.paymaster_payers_entries.len(),
        paymaster_non_all_found: paymaster_inspection.non_all_policy_payers.len(),
        paymaster_non_all_policies_deleted: paymaster_inspection
            .deleted_non_all_paymaster_policies
            .len(),
        genesis_da_height_before,
        genesis_da_height_after: cutover.calibrated_genesis_da_height,
        notes,
    };

    let report_json = serde_json::to_string_pretty(&report)
        .context("failed to serialize migration report JSON")?;

    if let Some(path) = args.report_out {
        std::fs::write(&path, &report_json)
            .with_context(|| format!("failed to write migration report to {}", path.display()))?;
    }

    println!("{report_json}");

    Ok(())
}

fn parse_address_args(args: &Args) -> anyhow::Result<ParsedAddressArgs> {
    let legacy_new_da_address = args
        .new_sequencer_da_address
        .as_deref()
        .map(|value| {
            CelestiaAddress::from_str(value)
                .with_context(|| format!("invalid --new-sequencer-da-address '{value}'"))
        })
        .transpose()?;

    let requested_old_da_address = args
        .old_sequencer_da_address
        .as_deref()
        .map(|value| {
            MockAddress::from_str(value)
                .with_context(|| format!("invalid --old-sequencer-da-address '{value}'"))
        })
        .transpose()?;

    let requested_new_rollup_address = args
        .new_sequencer_rollup_address
        .as_deref()
        .map(|value| {
            MultiAddressEvmSolana::from_str(value)
                .with_context(|| format!("invalid --new-sequencer-rollup-address '{value}'"))
        })
        .transpose()?;

    Ok(ParsedAddressArgs {
        legacy_new_da_address,
        requested_old_da_address,
        requested_new_rollup_address,
    })
}

fn prepare_migration_session(
    args: &Args,
    notes: &mut Vec<String>,
) -> anyhow::Result<MigrationSession> {
    let mut storage_config = load_storage_config(args)?;
    apply_storage_defaults_and_overrides(&mut storage_config, notes);
    let db_path = storage_config.path.clone();

    let runtime = Runtime::<OldSpec>::default();
    let module_discriminants = ModuleDiscriminants::from_runtime(&runtime);
    let storage_manager = MockStorageManager::new(storage_config, false)
        .with_context(|| format!("failed to open storage manager at {}", db_path.display()))?;

    let (storage, ledger_reader) = storage_manager
        .create_state_for_migration()
        .context("failed to create migration storage view")?;
    let ledger_db =
        LedgerDb::with_reader(ledger_reader).context("failed to initialize migration ledger db")?;
    let head_slot_number =
        assert_storage_latest_version_matches_ledger_head(&storage, &ledger_db, "pre-migration")?;
    assert_ledger_head_state_root_matches_storage_root(&storage, &ledger_db, "pre-migration")?;

    notes.push(format!(
        "using DB head rollup slot {head_slot_number} as migration base",
    ));

    Ok(MigrationSession {
        db_path,
        runtime,
        module_discriminants,
        storage_manager,
        storage,
        ledger_db,
        head_slot_number,
        db_head_rollup_slot: Some(head_slot_number.get()),
    })
}

fn resolve_cutover_calibration(
    args: &Args,
    db_head_rollup_slot: Option<u64>,
    notes: &mut Vec<String>,
) -> anyhow::Result<CutoverCalibration> {
    let celestia_height_at_cutover = args.celestia_height_at_cutover;

    let (rollup_slot_at_cutover, rollup_slot_at_cutover_source) =
        if let Some(explicit_slot) = args.rollup_slot_at_cutover {
            (explicit_slot, "explicit-arg".to_string())
        } else {
            let resolved_slot = db_head_rollup_slot
                .ok_or_else(|| anyhow::anyhow!("DB head slot is unavailable"))?;
            notes.push(format!(
                "resolved cutover rollup slot from DB head slot: {resolved_slot}"
            ));
            (resolved_slot, "db-head".to_string())
        };

    let calibrated_genesis_da_height = celestia_height_at_cutover
        .checked_sub(rollup_slot_at_cutover)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "invalid cutover heights: --celestia-height-at-cutover ({celestia_height_at_cutover}) \
                 is smaller than cutover rollup slot ({rollup_slot_at_cutover})"
            )
        })?;
    notes.push(format!(
        "calibrated chain_state.genesis_da_height to {calibrated_genesis_da_height} \
         from celestia_height_at_cutover={celestia_height_at_cutover} \
         and rollup_slot_at_cutover={rollup_slot_at_cutover}"
    ));

    Ok(CutoverCalibration {
        rollup_slot_at_cutover,
        rollup_slot_at_cutover_source,
        calibrated_genesis_da_height,
    })
}

fn build_sequencer_context<S: NativeStorage>(
    storage: &S,
    args: &Args,
    parsed_address_args: &ParsedAddressArgs,
    module_discriminants: ModuleDiscriminants,
    notes: &mut Vec<String>,
) -> anyhow::Result<SequencerContext> {
    let known_entries =
        load_known_sequencer_entries(storage, module_discriminants.sequencer_registry)?;

    let preferred_prefix = Prefix::new(
        module_discriminants.sequencer_registry,
        OldSequencerRegistry::PREFERRED_SEQUENCER_ITEM_DISCRIMINANT,
    );
    let preferred_slot_key = SlotKey::singleton(&preferred_prefix);
    let preferred_before = storage
        .get_unbound::<User>(preferred_slot_key.clone())
        .map(|value| {
            decode_borsh::<MockAddress>(
                value.value(),
                "sequencer_registry.preferred_sequencer value",
            )
        })
        .transpose()?;

    let plan = resolve_sequencer_plan(
        &known_entries,
        preferred_before,
        &args.sequencer_map,
        parsed_address_args.legacy_new_da_address,
        parsed_address_args.requested_old_da_address,
        parsed_address_args.requested_new_rollup_address,
    )?;

    notes.push(format!(
        "loaded {} known sequencer entries from sequencer_registry",
        known_entries.len()
    ));

    Ok(SequencerContext {
        plan,
        preferred_before,
        preferred_slot_key,
    })
}

fn inspect_paymaster_state<S: NativeStorage>(
    storage: &S,
    sequencer_to_payer_prefix: Prefix,
    paymaster_payers_prefix: Prefix,
    force_non_all_paymaster_policies: bool,
) -> anyhow::Result<PaymasterInspection> {
    let sequencer_to_payer_entries = collect_user_entries(storage, sequencer_to_payer_prefix)?;
    let paymaster_payers_entries = collect_user_entries(storage, paymaster_payers_prefix)?;

    let mut non_all_policy_payers = Vec::new();
    let mut deleted_non_all_paymaster_policies = Vec::new();
    for (slot_key, slot_value) in &paymaster_payers_entries {
        let payer = decode_borsh::<MultiAddressEvmSolana>(
            slot_key.without_prefix(),
            "paymaster.payers key",
        )?;
        let policy =
            decode_borsh::<PaymasterPolicy<OldSpec>>(slot_value.value(), "paymaster.payers value")?;

        if !matches!(policy.authorized_sequencers, AuthorizedSequencers::All) {
            non_all_policy_payers.push(payer.to_string());
            if force_non_all_paymaster_policies {
                deleted_non_all_paymaster_policies.push(slot_key.clone());
            }
        }
    }

    if !non_all_policy_payers.is_empty() && !force_non_all_paymaster_policies {
        bail!(
            "found {} paymaster policies with non-`AuthorizedSequencers::All` (payers: [{}]). \
             re-run with --force-non-all-paymaster-policies to delete these policies",
            non_all_policy_payers.len(),
            non_all_policy_payers.join(", ")
        );
    }

    Ok(PaymasterInspection {
        sequencer_to_payer_entries,
        paymaster_payers_entries,
        non_all_policy_payers,
        deleted_non_all_paymaster_policies,
    })
}

fn inspect_blob_storage_state<S: NativeStorage>(
    storage: &S,
    module_discriminants: ModuleDiscriminants,
    notes: &mut Vec<String>,
) -> anyhow::Result<BlobStorageInspection> {
    let deferred_blobs_prefix = Prefix::new(
        module_discriminants.blob_storage,
        OldBlobStorage::DEFERRED_BLOBS_ITEM_DISCRIMINANT,
    );
    let deferred_blobs_entries = collect_kernel_entries(storage, deferred_blobs_prefix)?;
    notes.push(format!(
        "loaded {} blob_storage.deferred_blobs entries from kernel state",
        deferred_blobs_entries.len()
    ));

    Ok(BlobStorageInspection {
        deferred_blobs_entries,
    })
}

fn inspect_chain_state_slots<S: NativeStorage>(
    storage: &S,
    module_discriminants: ModuleDiscriminants,
    notes: &mut Vec<String>,
) -> anyhow::Result<ChainStateSlotsInspection> {
    let user_slots_entries = collect_user_entries(
        storage,
        Prefix::new(
            module_discriminants.chain_state,
            OldChainState::SLOTS_ITEM_DISCRIMINANT,
        ),
    )?;
    let user_slots_entries = decode_old_slot_information_entries(
        user_slots_entries,
        "chain_state.slots (user namespace) value",
    )?;

    let kernel_slots_entries = collect_kernel_entries(
        storage,
        Prefix::new(
            module_discriminants.chain_state,
            OldChainState::SLOTS_ITEM_DISCRIMINANT,
        ),
    )?;
    let kernel_slots_entries = decode_old_slot_information_entries(
        kernel_slots_entries,
        "chain_state.slots (kernel namespace) value",
    )?;

    notes.push(format!(
        "loaded {} chain_state.slots user entries and {} kernel entries",
        user_slots_entries.len(),
        kernel_slots_entries.len()
    ));

    Ok(ChainStateSlotsInspection {
        user_slots_entries,
        kernel_slots_entries,
    })
}

fn decode_old_slot_information_entries(
    entries: Vec<(SlotKey, SlotValue)>,
    field: &str,
) -> anyhow::Result<Vec<(SlotKey, OldSlotInformation)>> {
    entries
        .into_iter()
        .map(|(slot_key, slot_value)| {
            let decoded = BcsCodec
                .try_decode(slot_value.value())
                .with_context(|| format!("failed to decode {field} at key {slot_key}"))?;
            Ok((slot_key, decoded))
        })
        .collect()
}

fn read_chain_state_genesis_da_height<S: NativeStorage>(
    storage: &S,
    module_discriminants: ModuleDiscriminants,
) -> anyhow::Result<(SlotKey, u64)> {
    let chain_state_genesis_da_height_key = SlotKey::singleton(&Prefix::new(
        module_discriminants.chain_state,
        OldChainState::GENESIS_DA_HEIGHT_ITEM_DISCRIMINANT,
    ));

    let genesis_da_height_before = storage
        .get_unbound::<User>(chain_state_genesis_da_height_key.clone())
        .map(|slot_value| {
            decode_borsh::<u64>(
                slot_value.value(),
                "chain_state.genesis_da_height state value",
            )
        })
        .transpose()?
        .expect("Genesis DA height must have been set at genesis!");

    Ok((chain_state_genesis_da_height_key, genesis_da_height_before))
}

fn add_genesis_da_height_notes(
    calibrated_genesis_da_height: u64,
    genesis_da_height_before: u64,
    notes: &mut Vec<String>,
) {
    let before = genesis_da_height_before;
    let after = calibrated_genesis_da_height;
    if before == after {
        notes.push(format!(
            "chain_state.genesis_da_height remains unchanged at {after}"
        ));
    } else {
        notes.push(format!(
            "chain_state.genesis_da_height will be updated from {before} to {after}"
        ));
    }
}

fn apply_sequencer_registry_updates<WS>(
    checkpoint: &mut WS,
    plan: &ResolvedSequencerPlan,
    known_prefix: Prefix,
    preferred_slot_key: &SlotKey,
) -> anyhow::Result<()>
where
    WS: StateWriter<Kernel> + StateWriter<User>,
{
    for entry in &plan.entries {
        StateWriter::<Kernel>::delete(checkpoint, &entry.old_slot_key).with_context(|| {
            format!(
                "failed to delete old known sequencer entry for {}",
                entry.old_da_address
            )
        })?;
    }

    for entry in &plan.entries {
        let migrated = KnownSequencer::<NewSpec> {
            address: entry.new_rollup_address,
            balance: entry.old_known.balance,
            balance_state: entry.old_known.balance_state.clone(),
        };
        let new_slot_key = SlotKey::new(&known_prefix, &entry.new_da_address, &BorshCodec);
        let new_slot_value = SlotValue::new(&migrated, &BorshCodec);
        StateWriter::<Kernel>::set(checkpoint, &new_slot_key, new_slot_value).with_context(
            || {
                format!(
                    "failed to write migrated known sequencer entry for {}",
                    entry.new_da_address
                )
            },
        )?;
    }

    StateWriter::<User>::delete(checkpoint, preferred_slot_key)
        .context("failed to clear old preferred sequencer value")?;
    if let Some(preferred_after) = &plan.preferred_after {
        StateWriter::<User>::set(
            checkpoint,
            preferred_slot_key,
            SlotValue::new(preferred_after, &BorshCodec),
        )
        .with_context(|| format!("failed to set preferred sequencer to {preferred_after}"))?;
    }

    Ok(())
}

fn apply_paymaster_updates<WS>(
    checkpoint: &mut WS,
    sequencer_to_payer_entries: &[(SlotKey, SlotValue)],
    deleted_non_all_paymaster_policies: &[SlotKey],
) -> anyhow::Result<()>
where
    WS: StateWriter<User>,
{
    for (slot_key, _) in sequencer_to_payer_entries {
        StateWriter::<User>::delete(checkpoint, slot_key)
            .context("failed to delete paymaster.sequencer_to_payer entry")?;
    }

    for slot_key in deleted_non_all_paymaster_policies {
        StateWriter::<User>::delete(checkpoint, slot_key)
            .context("failed to delete non-`All` paymaster policy")?;
    }

    Ok(())
}

fn apply_blob_storage_updates<WS>(
    checkpoint: &mut WS,
    deferred_blobs_entries: &[(SlotKey, SlotValue)],
) -> anyhow::Result<()>
where
    WS: StateWriter<Kernel>,
{
    for (slot_key, _) in deferred_blobs_entries {
        StateWriter::<Kernel>::delete(checkpoint, slot_key)
            .context("failed to delete blob_storage.deferred_blobs entry")?;
    }

    Ok(())
}

fn apply_genesis_da_height_update<WS>(
    checkpoint: &mut WS,
    chain_state_genesis_da_height_key: &SlotKey,
    calibrated_genesis_da_height: u64,
) -> anyhow::Result<()>
where
    WS: StateWriter<User>,
{
    StateWriter::<User>::set(
        checkpoint,
        chain_state_genesis_da_height_key,
        SlotValue::new(&calibrated_genesis_da_height, &BorshCodec),
    )
    .with_context(|| {
        format!("failed to set chain_state.genesis_da_height to {calibrated_genesis_da_height}")
    })?;

    Ok(())
}

fn apply_chain_state_slots_updates<WS>(
    checkpoint: &mut WS,
    user_slots_entries: &[(SlotKey, OldSlotInformation)],
    kernel_slots_entries: &[(SlotKey, OldSlotInformation)],
) -> anyhow::Result<()>
where
    WS: StateWriter<User> + StateWriter<Kernel>,
{
    for (slot_key, old_slot_info) in user_slots_entries {
        let migrated = migrate_slot_information(old_slot_info);
        StateWriter::<User>::set(checkpoint, slot_key, SlotValue::new(&migrated, &BcsCodec))
            .with_context(|| {
                format!("failed to rewrite chain_state.slots user entry {slot_key}")
            })?;
    }

    for (slot_key, old_slot_info) in kernel_slots_entries {
        let migrated = migrate_slot_information(old_slot_info);
        StateWriter::<Kernel>::set(checkpoint, slot_key, SlotValue::new(&migrated, &BcsCodec))
            .with_context(|| {
                format!("failed to rewrite chain_state.slots kernel entry {slot_key}")
            })?;
    }

    Ok(())
}

fn migrate_slot_information(old_slot_info: &OldSlotInformation) -> NewSlotInformation {
    let new_slot_hash = TmHash::from(old_slot_info.slot_hash().0);

    NewSlotInformation::new(
        new_slot_hash,
        old_slot_info.gas_info().clone(),
        *old_slot_info.prev_state_root(),
    )
}

fn load_storage_config(args: &Args) -> anyhow::Result<RollupDbConfig> {
    let rollup_config: RollupConfig<MultiAddressEvmSolana, StorableMockDaService> =
        from_toml_path(&args.rollup_config_path).with_context(|| {
            format!(
                "failed to read rollup config from {}",
                args.rollup_config_path.display()
            )
        })?;

    let mut storage = rollup_config.storage;
    if let Some(db_path_override) = &args.db_path {
        storage.path = db_path_override.clone();
    }

    Ok(storage)
}

fn apply_storage_defaults_and_overrides(config: &mut RollupDbConfig, notes: &mut Vec<String>) {
    if config.user_commit_concurrency.is_none() {
        config.user_commit_concurrency = Some(4);
        notes.push("storage.user_commit_concurrency missing; defaulted to 4".to_string());
    }
    if config.user_hashtable_buckets.is_none() {
        config.user_hashtable_buckets = Some(15_000_000);
        notes.push("storage.user_hashtable_buckets missing; defaulted to 15000000".to_string());
    }
    if config.user_preallocate_ht.is_none() {
        config.user_preallocate_ht = Some(false);
        notes.push("storage.user_preallocate_ht missing; defaulted to false".to_string());
    }
    if config.kernel_commit_concurrency.is_none() {
        config.kernel_commit_concurrency = Some(2);
        notes.push("storage.kernel_commit_concurrency missing; defaulted to 2".to_string());
    }
    if config.kernel_preallocate_ht.is_none() {
        config.kernel_preallocate_ht = Some(false);
        notes.push("storage.kernel_preallocate_ht missing; defaulted to false".to_string());
    }
    if config.pruner_versions_to_keep.is_none() {
        config.pruner_versions_to_keep = Some(20);
        notes.push("storage.pruner_versions_to_keep missing; defaulted to 20".to_string());
    }
}

fn decode_borsh<T: BorshDeserialize>(bytes: &[u8], field: &str) -> anyhow::Result<T> {
    borsh::from_slice(bytes)
        .with_context(|| format!("failed to decode borsh for {field} ({} bytes)", bytes.len()))
}

fn collect_kernel_entries<S: NativeStorage>(
    storage: &S,
    prefix: Prefix,
) -> anyhow::Result<Vec<(SlotKey, SlotValue)>> {
    let maybe_iter = storage
        .maybe_iter_kernel_values_with_prefix(SlotKey::singleton(&prefix))
        .context("kernel prefix iteration failed")?;

    let iter = maybe_iter
        .ok_or_else(|| anyhow::anyhow!("kernel prefix iteration is unavailable on this backend"))?;

    Ok(iter.collect())
}

fn collect_user_entries<S: NativeStorage>(
    storage: &S,
    prefix: Prefix,
) -> anyhow::Result<Vec<(SlotKey, SlotValue)>> {
    let maybe_iter = storage
        .maybe_iter_user_values_with_prefix(SlotKey::singleton(&prefix))
        .context("user prefix iteration failed")?;

    let iter = maybe_iter
        .ok_or_else(|| anyhow::anyhow!("user prefix iteration is unavailable on this backend"))?;

    Ok(iter.collect())
}

fn load_known_sequencer_entries<S: NativeStorage>(
    storage: &S,
    sequencer_registry_module_discriminant: u8,
) -> anyhow::Result<Vec<KnownSequencerEntry>> {
    let prefix = Prefix::new(
        sequencer_registry_module_discriminant,
        OldSequencerRegistry::KNOWN_SEQUENCERS_ITEM_DISCRIMINANT,
    );
    let entries = collect_kernel_entries(storage, prefix)?;

    entries
        .into_iter()
        .map(|(slot_key, slot_value)| {
            let da_address =
                decode_borsh::<MockAddress>(slot_key.without_prefix(), "known sequencer key")?;
            let known = decode_borsh::<KnownSequencer<OldSpec>>(
                slot_value.value(),
                "known sequencer value",
            )?;
            Ok(KnownSequencerEntry {
                slot_key,
                da_address,
                known,
            })
        })
        .collect()
}

fn resolve_sequencer_plan(
    entries: &[KnownSequencerEntry],
    preferred_before: Option<MockAddress>,
    explicit_mappings: &[SequencerMapArg],
    legacy_new_da_address: Option<CelestiaAddress>,
    requested_old_da_address: Option<MockAddress>,
    requested_new_rollup_address: Option<MultiAddressEvmSolana>,
) -> anyhow::Result<ResolvedSequencerPlan> {
    if explicit_mappings.is_empty() {
        let Some(new_da_address) = legacy_new_da_address else {
            bail!("--new-sequencer-da-address is required when --sequencer-map is not provided");
        };
        let selected = select_source_sequencer(entries, requested_old_da_address)?;
        let new_rollup_address = requested_new_rollup_address.unwrap_or(selected.known.address);

        let preferred_after = match preferred_before {
            Some(preferred) if preferred != selected.da_address => {
                bail!(
                    "preferred_sequencer {} is not the selected source sequencer {}; \
                     use --sequencer-map to migrate multi-sequencer state safely",
                    preferred,
                    selected.da_address
                );
            }
            _ => Some(new_da_address),
        };

        return Ok(ResolvedSequencerPlan {
            entries: vec![ResolvedSequencerEntry {
                old_slot_key: selected.slot_key,
                old_da_address: selected.da_address,
                old_known: selected.known,
                new_da_address,
                new_rollup_address,
            }],
            preferred_after,
        });
    }

    if legacy_new_da_address.is_some()
        || requested_old_da_address.is_some()
        || requested_new_rollup_address.is_some()
    {
        bail!(
            "--sequencer-map cannot be combined with --new-sequencer-da-address, \
             --old-sequencer-da-address, or --new-sequencer-rollup-address"
        );
    }

    for (idx, mapping) in explicit_mappings.iter().enumerate() {
        if explicit_mappings[..idx]
            .iter()
            .any(|m| m.old_da_address == mapping.old_da_address)
        {
            bail!(
                "duplicate old sequencer address in --sequencer-map: {}",
                mapping.old_da_address
            );
        }
        if explicit_mappings[..idx]
            .iter()
            .any(|m| m.new_da_address == mapping.new_da_address)
        {
            bail!(
                "duplicate new sequencer address in --sequencer-map: {}",
                mapping.new_da_address
            );
        }
    }

    if explicit_mappings.len() != entries.len() {
        let known_addresses = entries
            .iter()
            .map(|entry| entry.da_address.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "--sequencer-map count ({}) does not match number of known sequencers in DB ({}). \
             known sequencer addresses: [{}]",
            explicit_mappings.len(),
            entries.len(),
            known_addresses
        );
    }

    for mapping in explicit_mappings {
        if entries
            .iter()
            .all(|entry| entry.da_address != mapping.old_da_address)
        {
            bail!(
                "--sequencer-map references unknown old sequencer address: {}",
                mapping.old_da_address
            );
        }
    }

    let mut resolved_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(mapping) = explicit_mappings
            .iter()
            .find(|m| m.old_da_address == entry.da_address)
        else {
            bail!(
                "missing --sequencer-map for old sequencer address {}",
                entry.da_address
            );
        };
        resolved_entries.push(ResolvedSequencerEntry {
            old_slot_key: entry.slot_key.clone(),
            old_da_address: entry.da_address,
            old_known: entry.known.clone(),
            new_da_address: mapping.new_da_address,
            new_rollup_address: entry.known.address,
        });
    }

    let preferred_after = if let Some(old_preferred) = preferred_before {
        let Some(mapping) = explicit_mappings
            .iter()
            .find(|m| m.old_da_address == old_preferred)
        else {
            bail!(
                "preferred_sequencer {} has no corresponding --sequencer-map entry",
                old_preferred
            );
        };
        Some(mapping.new_da_address)
    } else {
        None
    };

    Ok(ResolvedSequencerPlan {
        entries: resolved_entries,
        preferred_after,
    })
}

fn select_source_sequencer(
    entries: &[KnownSequencerEntry],
    requested_old_da_address: Option<MockAddress>,
) -> anyhow::Result<KnownSequencerEntry> {
    if entries.len() > 1 {
        let available = entries
            .iter()
            .map(|entry| entry.da_address.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "multiple old known sequencers found [{}]. pass explicit --sequencer-map OLD=NEW \
             entries for every old sequencer",
            available
        );
    }

    let Some(only) = entries.first() else {
        bail!("no old known sequencers found");
    };

    if let Some(requested) = requested_old_da_address {
        if only.da_address != requested {
            bail!(
                "requested --old-sequencer-da-address {} does not match the single known sequencer {}",
                requested,
                only.da_address
            );
        }
    }

    Ok(only.clone())
}

fn make_ledger_root_patch(
    ledger_db: &LedgerDb,
    new_state_root: &[u8],
) -> anyhow::Result<SchemaBatch> {
    let (head_slot_number, mut head_slot) = ledger_db
        .get_head_slot()?
        .ok_or_else(|| anyhow::anyhow!("ledger has no head slot; cannot patch state root"))?;

    head_slot.state_root = new_state_root.to_vec().into();

    let mut batch = SchemaBatch::new();
    batch.put::<SlotByNumber>(&head_slot_number, &head_slot)?;
    Ok(batch)
}

fn make_batch_receipt_patch(
    ledger_db: &LedgerDb,
    sequencer_plan: &ResolvedSequencerPlan,
    notes: &mut Vec<String>,
) -> anyhow::Result<(SchemaBatch, BatchReceiptMigrationStats)> {
    const BATCH_SCAN_CHUNK_SIZE: u64 = 10_000;
    let mut patch = SchemaBatch::new();
    let mut stats = BatchReceiptMigrationStats::default();
    let mut unmapped_old_da_addresses: BTreeMap<MockAddress, usize> = BTreeMap::new();

    let old_to_new = sequencer_plan
        .entries
        .iter()
        .map(|entry| (entry.old_da_address, entry.new_da_address))
        .collect::<BTreeMap<_, _>>();

    let reader = ledger_db.clone_reader();
    let Some((last_batch_number, _)) = reader.get_largest::<BatchByNumber>()? else {
        notes.push("ledger contains no batches; skipping batch receipt migration".to_string());
        return Ok((patch, stats));
    };
    let last_batch_number = last_batch_number.0;

    let mut batch_start = 0u64;
    while batch_start <= last_batch_number {
        let batch_end_inclusive = min(
            batch_start.saturating_add(BATCH_SCAN_CHUNK_SIZE - 1),
            last_batch_number,
        );
        let range_end_exclusive = batch_end_inclusive.saturating_add(1);
        let batches = reader.collect_in_range::<BatchByNumber, _>(
            BatchNumber(batch_start)..BatchNumber(range_end_exclusive),
        )?;

        for (batch_number, batch) in batches {
            stats.scanned = stats.scanned.saturating_add(1);
            match migrate_stored_batch_receipt(batch, &old_to_new, &mut unmapped_old_da_addresses)?
            {
                BatchReceiptMigrationOutcome::AlreadyNew => {
                    stats.already_new = stats.already_new.saturating_add(1);
                }
                BatchReceiptMigrationOutcome::NeedsRewrite(rewritten_batch) => {
                    patch.put::<BatchByNumber>(&batch_number, &rewritten_batch)?;
                    stats.rewritten = stats.rewritten.saturating_add(1);
                }
                BatchReceiptMigrationOutcome::UnmappedOldAddress => {}
            }
        }

        if batch_end_inclusive == last_batch_number {
            break;
        }
        batch_start = batch_end_inclusive.saturating_add(1);
    }

    if !unmapped_old_da_addresses.is_empty() {
        let missing = unmapped_old_da_addresses
            .iter()
            .map(|(addr, count)| format!("{addr} ({count} batch receipts)"))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "found historical batch receipts with old DA addresses not present in the migration mapping: [{}]. \
             provide --sequencer-map entries for all historical sequencer DA addresses",
            missing
        );
    }

    notes.push(format!(
        "scanned {} historical batch receipts; rewrote {}; already-migrated {}",
        stats.scanned, stats.rewritten, stats.already_new
    ));

    Ok((patch, stats))
}

enum BatchReceiptMigrationOutcome {
    AlreadyNew,
    NeedsRewrite(StoredBatch),
    UnmappedOldAddress,
}

fn migrate_stored_batch_receipt(
    mut batch: StoredBatch,
    old_to_new: &BTreeMap<MockAddress, CelestiaAddress>,
    unmapped_old_da_addresses: &mut BTreeMap<MockAddress, usize>,
) -> anyhow::Result<BatchReceiptMigrationOutcome> {
    if bincode::deserialize::<BatchSequencerReceipt<NewSpec>>(batch.receipt.as_ref()).is_ok() {
        return Ok(BatchReceiptMigrationOutcome::AlreadyNew);
    }

    let old_receipt =
        bincode::deserialize::<BatchSequencerReceipt<OldSpec>>(batch.receipt.as_ref())
            .with_context(|| {
                format!(
                "failed to decode batch receipt as either NewSpec or OldSpec for batch hash 0x{}",
                hex::encode(batch.hash)
            )
            })?;

    let Some(new_da_address) = old_to_new.get(&old_receipt.da_address) else {
        let count = unmapped_old_da_addresses
            .entry(old_receipt.da_address)
            .or_insert(0);
        *count = count.saturating_add(1);
        return Ok(BatchReceiptMigrationOutcome::UnmappedOldAddress);
    };

    let new_receipt = BatchSequencerReceipt::<NewSpec> {
        da_address: *new_da_address,
        gas_price: old_receipt.gas_price,
        gas_used: old_receipt.gas_used,
        outcome: old_receipt.outcome,
    };

    batch.receipt = DbBytes::new(
        bincode::serialize(&new_receipt).context("failed to encode migrated batch receipt")?,
    );
    Ok(BatchReceiptMigrationOutcome::NeedsRewrite(batch))
}

fn assert_storage_latest_version_matches_ledger_head<S: NativeStorage>(
    storage: &S,
    ledger_db: &LedgerDb,
    phase: &str,
) -> anyhow::Result<SlotNumber> {
    let (head_slot_number, _head_slot) = ledger_db
        .get_head_slot()?
        .ok_or_else(|| anyhow::anyhow!("ledger has no head slot; cannot migrate an empty DB"))?;
    let storage_latest_version = storage.latest_version();
    if storage_latest_version != head_slot_number {
        bail!(
            "{phase} invariant failed: storage.latest_version ({}) != ledger head slot ({})",
            storage_latest_version,
            head_slot_number
        );
    }

    Ok(head_slot_number)
}

fn assert_ledger_head_state_root_matches_storage_root<S: NativeStorage>(
    storage: &S,
    ledger_db: &LedgerDb,
    phase: &str,
) -> anyhow::Result<()> {
    let (head_slot_number, head_slot) = ledger_db
        .get_head_slot()?
        .ok_or_else(|| anyhow::anyhow!("ledger has no head slot; cannot migrate an empty DB"))?;
    let storage_root = storage
        .get_root_hash(head_slot_number)
        .context("failed to read storage root at ledger head slot")?;
    if head_slot.state_root.as_ref() != storage_root.as_ref() {
        bail!(
            "{phase} invariant failed: ledger head state_root does not match storage root at slot {}",
            head_slot_number
        );
    }

    Ok(())
}
