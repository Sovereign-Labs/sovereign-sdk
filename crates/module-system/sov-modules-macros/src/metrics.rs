use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::ToTokens;
use syn::{parse2, parse_quote, Block, Ident, ItemFn};

#[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
pub mod gas_estimation {
    use quote::quote;

    use super::*;

    /// Wrap a block with benchmarking. Fills the correct cycle counter based on the target and vendor.
    pub fn const_tracker(ident: &Ident, block: &Block, tagged_inputs: &[Ident]) -> Block {
        let inputs_iter = tagged_inputs
            .iter()
            .map(|ident| quote! { (stringify!(#ident).to_string(), #ident.to_string()) })
            .collect::<Vec<_>>();

        let const_tracker_block: Box<Block> = parse_quote! {
            {
                let inputs = vec![ #(#inputs_iter,)* ];

                let closure = move |constants_usage_before: ::sov_metrics::GasConstantTracker| {
                    let result = (|| #block)();
                    let constants_usage_after: ::sov_metrics::GasConstantTracker =
                        ::sov_metrics::GAS_CONSTANTS.with(|gas_constants| gas_constants.borrow().clone());

                    let constants_usage_diff = constants_usage_after.diff(constants_usage_before);

                    constants_usage_diff.report_gas_constants_usage(stringify!(#ident), inputs);

                    result
                };

                if let Ok(constants_usage_before) =
                    ::sov_metrics::GAS_CONSTANTS.try_with(|gas_constants| gas_constants.borrow().clone()) {
                        closure(constants_usage_before)
                } else {
                    ::sov_metrics::GAS_CONSTANTS.sync_scope(
                        std::cell::RefCell::new(::sov_metrics::GasConstantTracker::default()), || {
                            closure(::sov_metrics::GasConstantTracker::default())
                        }
                    )
                }
            }
        };

        parse_quote!({
            #const_tracker_block
        })
    }
}

#[cfg(feature = "bench")]
pub mod zk {
    use quote::quote;

    use super::*;

    /// Wrap a block with benchmarking. Fills the correct cycle counter based on the target and vendor.
    pub fn guest_metrics(ident: &Ident, block: &Block, tagged_inputs: &[Ident]) -> Block {
        let risc0_zk_block = metrics_inner_risc0(ident, block, tagged_inputs);
        let sp1_zk_block = metrics_inner_sp1(ident, block, tagged_inputs);

        parse_quote!({
            #[cfg(all(target_os = "zkvm", target_vendor = "succinct"))]
            {
                return #sp1_zk_block;
            }
            #[cfg(all(target_os = "zkvm", target_vendor = "risc0"))]
            {
                return #risc0_zk_block;
            }
            #[cfg(not(target_os = "zkvm"))]
            {
                return #block;
            }
        })
    }

    fn metrics_inner_risc0(ident: &Ident, block: &Block, tagged_inputs: &[Ident]) -> Block {
        let inputs_iter = tagged_inputs
            .iter()
            .map(|ident| quote! { (stringify!(#ident).to_string(), #ident.to_string()) })
            .collect::<Vec<_>>();

        parse_quote! {
            {
                let inputs = vec![ #(#inputs_iter,)* ];


                let memory_before = ::sov_metrics::cycle_utils::risc0::get_available_heap();

                let cycles_before = ::sov_metrics::cycle_utils::risc0::get_cycle_count();
                let result = (|| #block)();
                let cycles_after = ::sov_metrics::cycle_utils::risc0::get_cycle_count();

                let memory_after = ::sov_metrics::cycle_utils::risc0::get_available_heap();

                let cycles = cycles_after.saturating_sub(cycles_before);

                let memory_diff = ::sov_metrics::cycle_utils::MemoryInfo {
                    free: memory_after.free,
                    used: memory_after.used.saturating_sub(memory_before.used)
                };

                ::sov_metrics::cycle_utils::risc0::report_cycle_count(
                    ::sov_metrics::cycle_utils::CycleMetric {
                        name: stringify!(#ident).to_string(),
                        metadata: inputs,
                        count: cycles,
                        memory: memory_diff,
                    }
                );

                result
            }
        }
    }

    fn metrics_inner_sp1(ident: &Ident, block: &Block, tagged_inputs: &[Ident]) -> Block {
        let inputs_iter = tagged_inputs
            .iter()
            .map(|ident| quote! { (stringify!(#ident).to_string(), #ident.to_string()) })
            .collect::<Vec<_>>();

        parse_quote!({
           {
                let inputs = vec![ #(#inputs_iter,)* ];

                let memory_before = ::sov_metrics::cycle_utils::sp1::get_available_heap();
                let before = ::sov_metrics::cycle_utils::sp1::get_cycle_count();
                let result = (move || #block)();
                let after = ::sov_metrics::cycle_utils::sp1::get_cycle_count();
                let memory_after = ::sov_metrics::cycle_utils::sp1::get_available_heap();

                let memory_diff = ::sov_metrics::cycle_utils::MemoryInfo {
                    free: memory_after.free,
                    used: memory_after.used.saturating_sub(memory_before.used)
                };

                ::sov_metrics::cycle_utils::sp1::report_cycle_count(
                    ::sov_metrics::cycle_utils::CycleMetric {
                        name: stringify!(#ident).to_string(),
                        metadata: inputs,
                        count: after - before,
                        memory: memory_diff,
                    });
                result
            }
        })
    }
}

/// Wrap a function, where `f` wraps a block with (benchmarking) code.
pub fn wrap_function_with<F>(
    f: F,
    input: TokenStream,
    mut tag_attr: HashSet<Ident>,
) -> Result<TokenStream, syn::Error>
where
    F: Fn(&Ident, &Block, &[Ident]) -> Block,
{
    let mut input = parse2::<ItemFn>(input.into())?;
    let ItemFn {
        sig: syn::Signature { ident, inputs, .. },
        block,
        ..
    } = input.clone();

    // We are collecting the function inputs idents that have been tagged in the macro arguments.
    // For that we are looping over the function arguments and checking that the associated identifier
    // is part of the set of macro arguments.
    let tagged_inputs = inputs
        .into_iter()
        .filter_map(|i| match i {
            syn::FnArg::Typed(syn::PatType { pat, .. }) => match *pat {
                syn::Pat::Ident(pat_ident) if tag_attr.remove(&pat_ident.ident) => {
                    Some(pat_ident.ident)
                }
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();

    if !tag_attr.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "The following attributes were not found in the function signature: {}",
                tag_attr
                    .into_iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    input.block = Box::new(f(&ident, &block, &tagged_inputs));

    // Add allow attribute to suppress clippy warnings on generated code
    input
        .attrs
        .push(syn::parse_quote!(#[allow(clippy::unused_unit, unexpected_cfgs)]));

    Ok(input.to_token_stream().into())
}
