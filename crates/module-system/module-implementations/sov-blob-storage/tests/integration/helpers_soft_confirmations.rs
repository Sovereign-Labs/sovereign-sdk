use sov_kernels::soft_confirmations::SoftConfirmationsKernel;
use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::macros::config_value;
use sov_modules_api::{Amount, CryptoSpec, Gas, GasSpec, GetGasPrice, RawTx, Spec};
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::SequencerRegistry;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::{
    generate_zk_runtime_with_kernel, AsUser, BatchType, SequencerInfo, SoftConfirmationBlobInfo,
    TestSequencer, TransactionType, TEST_DEFAULT_USER_STAKE,
};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

use crate::{
    assert_blobs_are_correctly_received_helper, HashMap, SequenceInfo, SlotConfigInfo, TestData,
    TestRunner, S,
};

pub type SoftConfRT = SoftConfBlobStorageRuntime<S>;
generate_zk_runtime_with_kernel!(kernel_type: SoftConfirmationsKernel<'a, S>, SoftConfBlobStorageRuntime <= value_setter: ValueSetter<S>);

/// Sets up a test runtime and returns a [`TestData`] struct. Does not register the regular sequencer.
pub fn setup_soft_confirmation_kernel() -> (TestData<S>, TestRunner<SoftConfRT>) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(2);
    let preferred_sequencer = genesis_config.initial_sequencer.clone();
    let user_account = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let regular_sequencer = genesis_config.additional_accounts()[1].clone();
    let regular_sequencer_da_address = MockAddress::new([42; 32]);

    let user_stake = <S as Spec>::Gas::from(TEST_DEFAULT_USER_STAKE);
    let user_stake_value = user_stake
        .checked_value(&S::initial_base_fee_per_gas())
        .unwrap();

    let regular_sequencer = TestSequencer {
        user_info: regular_sequencer,
        da_address: regular_sequencer_da_address,
        bond: user_stake_value,
    };

    // Run genesis registering the attester and sequencer we've generated.
    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: user_account.address(),
        },
    );

    let runner = TestRunner::<SoftConfRT>::new_with_genesis(
        genesis.into_genesis_params(),
        SoftConfRT::default(),
    );

    (
        TestData {
            user: user_account,
            preferred_sequencer,
            regular_sequencer,
        },
        runner,
    )
}

/// Sets up a test runtime and returns a [`TestData`] struct. Registers the regular sequencer
/// Note: with this setup, the first available sequence number is 1. This is because the
/// sequencer number 0 is used to register the non-preferred sequencer.
pub fn setup_with_registration_soft_confirmation_kernel() -> (TestData<S>, TestRunner<SoftConfRT>) {
    let (test_data, mut runner) = setup_soft_confirmation_kernel();

    let regular_sequencer = &test_data.regular_sequencer;
    let regular_sequencer_da_address = regular_sequencer.da_address;

    let user_stake_value = runner.query_visible_state(|state| {
        <S as Spec>::Gas::from(config_value!("MAX_SEQUENCER_EXEC_GAS_PER_TX"))
            .value(state.gas_price())
            .checked_mul(Amount::new(10))
            .unwrap()
    });

    // We currently have to manually build the soft-confirmation blob
    // There is an issue to fix that: `https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1330`
    let mut nonces = runner.nonces().clone();
    let blob = TestRunner::<SoftConfRT>::soft_confirmation_batches_to_blobs(
        vec![SoftConfirmationBlobInfo {
            batch_type: BatchType(vec![regular_sequencer
                .create_plain_message::<SoftConfRT, SequencerRegistry<S>>(
                    sov_sequencer_registry::CallMessage::Register {
                        da_address: regular_sequencer_da_address,
                        amount: user_stake_value,
                    },
                )]),
            sequencer_address: test_data.preferred_sequencer.da_address,
            sequencer_info: SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        }],
        &mut nonces,
    );

    runner.execute::<RelevantBlobs<MockBlob>>(blob);

    (test_data, runner)
}

/// Builds a [`RelevantBlobs`] struct from a list of [`BlobConfigInfo`]s.
/// This struct populates the batches with simple [`ValueSetter`] messages. One
/// can specify special sequencer addresses for each batch.
pub fn build_soft_confirmation_blobs(
    slot_info: &SlotConfigInfo<(TestSequencer<S>, SequencerInfo)>,
    nonces: &mut HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    batch_size: usize,
) -> RelevantBlobs<MockBlob> {
    let mut batches = Vec::new();

    for (sequencer, additional_info) in slot_info {
        batches.push(SoftConfirmationBlobInfo {
            batch_type: BatchType(vec![TransactionType::PreSigned(RawTx::new(vec![
                1;
                batch_size
            ]))]),
            sequencer_address: sequencer.da_address,
            sequencer_info: additional_info.clone(),
        });
    }

    TestRunner::<SoftConfRT>::soft_confirmation_batches_to_blobs(batches, nonces)
}

/// This helper method asserts that given slots to send and an expected order of receipts, the
/// [`TestRunner`] will emit the receipts in the expected order.
///
/// The `receive_order` parameter is the list of indexes of the batches that we expect to receive.
///
/// Example: If we have the following situation:
/// - Slot 1: Send [ (Blob 0, sequencer A, Preferred { slots_to_advance: 1, sequence_number: 0 }), (Blob 1, sequencer B, Regular), (Blob 2, sequencer B, Regular) ] | Receive [ Blob 0 ]
/// - Slot 2: Send [ (Blob 3, sequencer A, Preferred { slots_to_advance: 1, sequence_number: 1 }), (Blob 4, sequencer B, Regular) ] | Receive [ Blob 3 ]
/// - Slot 3: Send [] | Receive [ Blob 1, Blob 2 ]
/// - Slot 4: Send [] | Receive [ Blob 4 ]
///
/// Then the `receive_order` parameter should be [ [0], [3], [1, 2], [4] ].
///
/// The `visible_slot_heights_increases` parameter indicates the visible slot heights that we expect to advance.
/// In the situation above: we would have [1, 1, 0, 0] for the `visible_slot_heights_increases` parameter.
pub fn assert_blobs_are_correctly_received_soft_confirmation(
    sending_order: Vec<Vec<(TestSequencer<S>, SequencerInfo)>>,
    receive_order: Vec<Vec<SequenceInfo>>,
    visible_slot_heights_increases: Vec<u64>,
    runner: &mut TestRunner<SoftConfRT>,
) {
    let mut nonces = HashMap::new();

    let slots_to_send = sending_order
        .iter()
        .map(|blobs_slot_info| build_soft_confirmation_blobs(blobs_slot_info, &mut nonces, 0))
        .collect::<Vec<_>>();

    assert_blobs_are_correctly_received_helper(
        slots_to_send,
        receive_order,
        visible_slot_heights_increases,
        runner,
    );
}
