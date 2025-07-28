use std::collections::HashMap;
use std::sync::Arc;

use sov_modules_api::{CryptoSpec, PrivateKey, Spec};
use sov_test_modules::access_pattern::AccessPatternDiscriminants;
use sov_test_utils::runtime::sov_bank::CallMessageDiscriminants as BankDiscriminants;
use sov_transaction_generator::generators::access_pattern::{
    AccessPatternHarness, AccessPatternMessageGenerator,
};
use sov_transaction_generator::generators::bank::harness_interface::BankHarness;
use sov_transaction_generator::generators::bank::BankMessageGenerator;
use sov_transaction_generator::generators::basic::BasicModuleRef;
use sov_transaction_generator::{Distribution, MessageValidity, Percent};

use super::cli::BenchCLICustomArgs;
use super::{Benchmark, RT, S};

pub fn bank_benches(params: &BenchCLICustomArgs, slots: u64, seed: u128) -> Vec<Benchmark<S>> {
    let admin = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    vec![
        {
            let bench_module: BasicModuleRef<S, RT> =
                Arc::new(BankHarness::new(BankMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![BankDiscriminants::Transfer]),
                    Percent::one_hundred(),
                )));
            params.new_benchmark_with_params(
                "bank_transfers_100_percent_address_creation".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> =
                Arc::new(BankHarness::new(BankMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![BankDiscriminants::Transfer]),
                    Percent::fifty(),
                )));
            params.new_benchmark_with_params(
                "bank_transfers".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> =
                Arc::new(BankHarness::new(BankMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        BankDiscriminants::Transfer,
                        BankDiscriminants::CreateToken,
                        BankDiscriminants::Mint,
                        BankDiscriminants::Burn,
                        BankDiscriminants::Freeze,
                    ]),
                    Percent::fifty(),
                )));

            params.new_benchmark_with_params(
                "bank_messages".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
    ]
}

pub fn storage_access_benches(
    params: &BenchCLICustomArgs,
    slots: u64,
    seed: u128,
) -> Vec<Benchmark<S>> {
    let admin = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    vec![
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::WriteCells,
                        AccessPatternDiscriminants::ReadCells,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "access_pattern_rw".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::WriteCells,
                        AccessPatternDiscriminants::ReadCells,
                        AccessPatternDiscriminants::DeleteCells,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "access_pattern_rw_delete".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::ReadCells,
                        AccessPatternDiscriminants::WriteCells,
                        AccessPatternDiscriminants::WriteCustom,
                        AccessPatternDiscriminants::DeleteCells,
                        AccessPatternDiscriminants::UpdateAdmin,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "access_pattern_rw_mix".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::ReadCells,
                        AccessPatternDiscriminants::WriteCells,
                        AccessPatternDiscriminants::WriteCustom,
                        AccessPatternDiscriminants::DeleteCells,
                        AccessPatternDiscriminants::UpdateAdmin,
                        AccessPatternDiscriminants::SetHook,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "access_pattern_rw_mix_with_hooks".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
    ]
}

pub fn ops_benches(params: &BenchCLICustomArgs, slots: u64, seed: u128) -> Vec<Benchmark<S>> {
    let admin = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    vec![
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::HashBytes,
                        AccessPatternDiscriminants::HashCustom,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "bench_hash".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::StoreSerializedString,
                        AccessPatternDiscriminants::DeserializeBytesAsString,
                        AccessPatternDiscriminants::DeserializeCustomString,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "bench_deserialize".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::StoreSignature,
                        AccessPatternDiscriminants::VerifySignature,
                        AccessPatternDiscriminants::VerifyCustomSignature,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "bench_signature".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::HashBytes,
                        AccessPatternDiscriminants::HashCustom,
                        AccessPatternDiscriminants::StoreSerializedString,
                        AccessPatternDiscriminants::DeserializeBytesAsString,
                        AccessPatternDiscriminants::DeserializeCustomString,
                        AccessPatternDiscriminants::StoreSignature,
                        AccessPatternDiscriminants::VerifySignature,
                        AccessPatternDiscriminants::VerifyCustomSignature,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "bench_combined_ops".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                admin.clone(),
            )
        },
    ]
}

pub fn mix_accesses(params: &BenchCLICustomArgs, slots: u64, seed: u128) -> Vec<Benchmark<S>> {
    let value_setter_admin = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    vec![
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::ReadCells,
                        AccessPatternDiscriminants::WriteCells,
                        AccessPatternDiscriminants::WriteCustom,
                        AccessPatternDiscriminants::DeleteCells,
                        AccessPatternDiscriminants::UpdateAdmin,
                        AccessPatternDiscriminants::SetHook,
                        AccessPatternDiscriminants::HashBytes,
                        AccessPatternDiscriminants::HashCustom,
                        AccessPatternDiscriminants::StoreSerializedString,
                        AccessPatternDiscriminants::DeserializeBytesAsString,
                        AccessPatternDiscriminants::DeserializeCustomString,
                        AccessPatternDiscriminants::StoreSignature,
                        AccessPatternDiscriminants::VerifySignature,
                        AccessPatternDiscriminants::VerifyCustomSignature,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    value_setter_admin.clone(),
                ),
            ));

            params.new_benchmark_with_params(
                "bench_complete_patterns".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                value_setter_admin.clone(),
            )
        },
        {
            let bench_module: BasicModuleRef<S, RT> = Arc::new(AccessPatternHarness::new(
                AccessPatternMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        AccessPatternDiscriminants::ReadCells,
                        AccessPatternDiscriminants::WriteCells,
                        AccessPatternDiscriminants::WriteCustom,
                        AccessPatternDiscriminants::DeleteCells,
                        AccessPatternDiscriminants::UpdateAdmin,
                        AccessPatternDiscriminants::SetHook,
                        AccessPatternDiscriminants::HashBytes,
                        AccessPatternDiscriminants::HashCustom,
                        AccessPatternDiscriminants::StoreSerializedString,
                        AccessPatternDiscriminants::DeserializeBytesAsString,
                        AccessPatternDiscriminants::DeserializeCustomString,
                        AccessPatternDiscriminants::StoreSignature,
                        AccessPatternDiscriminants::VerifySignature,
                        AccessPatternDiscriminants::VerifyCustomSignature,
                    ]),
                    params.pattern_maximum_write_data_length,
                    params.pattern_maximum_write_begin_index,
                    params.pattern_maximum_write_size,
                    params.pattern_maximum_hooks_ops,
                    value_setter_admin.clone(),
                ),
            ));

            let bank_module: BasicModuleRef<S, RT> =
                Arc::new(BankHarness::new(BankMessageGenerator::new(
                    Distribution::with_equiprobable_values(vec![
                        BankDiscriminants::Transfer,
                        BankDiscriminants::CreateToken,
                        BankDiscriminants::Mint,
                        BankDiscriminants::Burn,
                        BankDiscriminants::Freeze,
                    ]),
                    Percent::fifty(),
                )));

            params.new_benchmark_with_params(
                "bench_complete_patterns_and_bank".to_string(),
                slots,
                seed,
                Distribution::with_equiprobable_values(vec![bench_module, bank_module]),
                Distribution::with_equiprobable_values(vec![
                    MessageValidity::Valid,
                    // TODO(@theochap) - reactivate once we have a way to send invalid transactions
                    // to DA through sequencers.
                    // MessageValidity::Invalid,
                ]),
                value_setter_admin.clone(),
            )
        },
    ]
}

/// Returns all the benchmarks as a map from a benchmark set name to the associated benchmarks.
pub fn all_benches(
    params: &BenchCLICustomArgs,
    slots: u64,
    seed: u128,
) -> HashMap<String, Vec<Benchmark<S>>> {
    HashMap::from([
        (
            "bank_benches".to_string(),
            bank_benches(params, slots, seed),
        ),
        (
            "storage_access_benches".to_string(),
            storage_access_benches(params, slots, seed),
        ),
        ("ops_benches".to_string(), ops_benches(params, slots, seed)),
        (
            "mix_accesses".to_string(),
            mix_accesses(params, slots, seed),
        ),
    ])
}
