use anyhow::Context;
use demo_stf::runtime::Runtime as DemoRuntime;
use sov_accounts::{Account, Accounts};
use sov_bank::config_gas_token_id;
use sov_chain_state::ChainState;
use sov_db::config::RollupDbConfig;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::tables::SlotByNumber;
use sov_db::storage_manager::NomtStorageManager;
use sov_demo_rollup::MockDemoRollup;
use sov_migrations::MigrationStorage as _;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{
    AccessoryStateValue, CredentialId, CryptoSpec, ModuleInfo, OperatingMode, Spec,
    StateCheckpoint, StateMap, StateWriter,
};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::{
    BorshCodec, Kernel, NativeStorage as _, Prefix, SlotKey, SlotValue, StateUpdate as _,
};
use sov_test_utils::test_rollup::{read_private_key, RollupBuilder};

use crate::test_helpers::{build_transfer_token_tx, test_genesis_source, DemoRollupSpec};

type Rollup = MockDemoRollup<Native>;
type Hasher = <<DemoRollupSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher;
type DemoStorageManager =
    NomtStorageManager<<DemoRollupSpec as Spec>::Da, Hasher, <DemoRollupSpec as Spec>::Storage>;
type DemoLedgerChangeSet = <DemoStorageManager as HierarchicalStorageManager<
    <DemoRollupSpec as Spec>::Da,
>>::LedgerChangeSet;

struct KernelRoundtrip {
    key: SlotKey,
    value: SlotValue,
}

#[tokio::test(flavor = "multi_thread")]
async fn v1_migration_rewrites_live_demo_state() -> anyhow::Result<()> {
    let credential_id = credential_id(0x11);
    let account_address = account_address(0x22);

    let test_rollup = RollupBuilder::<Rollup>::new(
        test_genesis_source(OperatingMode::Operator),
        BlockProducingConfig::Manual,
        0,
    )
    .set_persistent_da()
    .start()
    .await?;

    test_rollup.wait_for_node_synced().await?;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await?;
    let tx_signer = read_private_key::<DemoRollupSpec>("tx_signer_private_key.json");
    let tx = build_transfer_token_tx(
        &tx_signer.private_key,
        config_gas_token_id(),
        account_address,
        1,
        0,
    );
    test_rollup.send_tx_to_sequencer(&tx).await?;
    let initial_height = test_rollup.height().await.get();
    test_rollup.force_close_batch().await?;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_height(initial_height + 1).await;
    test_rollup.wait_for_node_synced().await?;

    let storage_config = test_rollup.rollup_config.storage.clone();
    let builder = test_rollup.shutdown().await?;

    prepare_v0_state_with_legacy_account(storage_config.clone(), credential_id, account_address)?;

    let mut runtime = DemoRuntime::<DemoRollupSpec>::default();
    let runtime_inner = &mut *runtime;
    let report = sov_migrations::v1::run_with_options::<DemoRollupSpec, Hasher>(
        sov_migrations::MigrationOptions {
            storage: storage_config.clone(),
            dry_run: false,
        },
        &mut runtime_inner.accounts,
        &mut runtime_inner.chain_state,
    )?;

    assert!(!report.dry_run);
    assert!(report.head_rollup_slot > 0);
    assert_ne!(report.pre_state_root, report.post_state_root);
    assert_eq!(
        report.migration.from_state_version,
        sov_migrations::v1::SOURCE_STATE_VERSION
    );
    assert_eq!(
        report.migration.to_state_version,
        sov_migrations::v1::TARGET_STATE_VERSION
    );
    assert_eq!(report.migration.accounts.entries_migrated, 1);

    assert_migrated_state(storage_config, credential_id, account_address)?;

    let restarted_rollup = builder.start().await?;
    restarted_rollup.wait_for_node_synced().await?;
    restarted_rollup.shutdown().await?;

    Ok(())
}

fn prepare_v0_state_with_legacy_account(
    storage_config: RollupDbConfig,
    credential_id: CredentialId,
    address: <DemoRollupSpec as Spec>::Address,
) -> anyhow::Result<()> {
    rewrite_head(storage_config, |runtime, state| {
        chain_state_state_version_value(&runtime.chain_state)
            .set(&sov_migrations::v1::SOURCE_STATE_VERSION, state)?;
        legacy_accounts_map(&runtime.accounts).set(
            &credential_id,
            &Account { addr: address },
            state,
        )?;
        Ok(())
    })
}

fn assert_migrated_state(
    storage_config: RollupDbConfig,
    credential_id: CredentialId,
    address: <DemoRollupSpec as Spec>::Address,
) -> anyhow::Result<()> {
    let storage_manager = DemoStorageManager::new(storage_config, false)
        .context("failed to open post-migration storage manager")?;
    let (storage, _ledger_reader) = storage_manager
        .create_state_for_migration()
        .context("failed to create post-migration storage view")?;
    let mut runtime = DemoRuntime::<DemoRollupSpec>::default();
    let mut checkpoint = StateCheckpoint::new(storage, &runtime.kernel(), None);

    let state_version = runtime
        .chain_state
        .state_version(&mut checkpoint.accessory_state())
        .context("failed to read migrated state_version")?;
    assert_eq!(state_version, sov_migrations::v1::TARGET_STATE_VERSION);
    assert!(runtime.accounts.is_explicitly_authorized(
        &address,
        &credential_id,
        &mut checkpoint
    )?);
    assert!(legacy_accounts_map(&runtime.accounts)
        .get(&credential_id, &mut checkpoint)?
        .is_none());

    Ok(())
}

fn rewrite_head(
    storage_config: RollupDbConfig,
    apply: impl FnOnce(
        &mut DemoRuntime<DemoRollupSpec>,
        &mut StateCheckpoint<DemoRollupSpec>,
    ) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let db_path = storage_config.path.clone();
    let mut storage_manager = DemoStorageManager::new(storage_config, false)
        .with_context(|| format!("failed to open storage manager at {}", db_path.display()))?;
    let (storage, ledger_reader) = storage_manager
        .create_state_for_migration()
        .context("failed to create migration setup storage view")?;
    let ledger_db =
        LedgerDb::with_reader(ledger_reader).context("failed to initialize migration ledger db")?;
    let (head_slot_number, mut head_slot) = ledger_db
        .get_head_slot()?
        .context("ledger has no head slot; cannot rewrite state")?;
    let pre_state_root = storage
        .get_root_hash(head_slot_number)
        .context("failed to read pre-rewrite state root")?;

    anyhow::ensure!(
        storage.latest_version() == head_slot_number,
        "storage latest version does not match ledger head slot"
    );
    anyhow::ensure!(
        head_slot.state_root.as_ref() == pre_state_root.as_ref(),
        "ledger head state root does not match storage root"
    );

    let mut runtime = DemoRuntime::<DemoRollupSpec>::default();
    let kernel_roundtrip = kernel_roundtrip(&runtime.chain_state, &storage)?;
    let mut checkpoint = StateCheckpoint::new(storage, &runtime.kernel(), None);
    apply(&mut runtime, &mut checkpoint)?;
    StateWriter::<Kernel>::set(
        &mut checkpoint,
        &kernel_roundtrip.key,
        kernel_roundtrip.value,
    )
    .context("failed to round-trip kernel state during setup rewrite")?;

    let (next_state_root, mut state_update, accessory_delta, _witness, storage_after) =
        checkpoint.materialize_update(pre_state_root);
    state_update.add_accessory_items(accessory_delta.freeze());
    let change_set =
        storage_after.materialize_migration_changes_at_version(state_update, head_slot_number);

    head_slot.state_root = next_state_root.as_ref().to_vec().into();
    let mut ledger_change_set: DemoLedgerChangeSet = Default::default();
    ledger_change_set.put::<SlotByNumber>(&head_slot_number, &head_slot)?;
    storage_manager
        .commit_migration_change_set_at_head(head_slot_number, change_set, ledger_change_set)
        .context("failed to commit rewritten state")?;

    Ok(())
}

fn legacy_accounts_map(
    accounts: &Accounts<DemoRollupSpec>,
) -> StateMap<CredentialId, Account<DemoRollupSpec>> {
    StateMap::with_codec(
        Prefix::new(
            accounts.discriminant(),
            Accounts::<DemoRollupSpec>::ACCOUNTS_ITEM_DISCRIMINANT,
        ),
        BorshCodec,
    )
}

fn chain_state_state_version_value(
    chain_state: &ChainState<DemoRollupSpec>,
) -> AccessoryStateValue<u64> {
    AccessoryStateValue::with_codec(
        Prefix::new(
            chain_state.discriminant(),
            ChainState::<DemoRollupSpec>::STATE_VERSION_ITEM_DISCRIMINANT,
        ),
        BorshCodec,
    )
}

fn kernel_roundtrip(
    chain_state: &ChainState<DemoRollupSpec>,
    storage: &<DemoRollupSpec as Spec>::Storage,
) -> anyhow::Result<KernelRoundtrip> {
    let key = SlotKey::singleton(&Prefix::new(
        chain_state.discriminant(),
        ChainState::<DemoRollupSpec>::TRUE_SLOT_NUMBER_ITEM_DISCRIMINANT,
    ));
    let value = storage.get_unbound::<Kernel>(key.clone()).ok_or_else(|| {
        anyhow::anyhow!("kernel chain_state.true_slot_number is unset; cannot rewrite state")
    })?;

    Ok(KernelRoundtrip { key, value })
}

fn credential_id(byte: u8) -> CredentialId {
    [byte; 32].into()
}

fn account_address(byte: u8) -> <DemoRollupSpec as Spec>::Address {
    credential_id(byte).into()
}
