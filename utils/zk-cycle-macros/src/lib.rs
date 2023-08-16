#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

/// This macro is used to annotate functions that we want to track the number of riscV cycles being
/// generated inside the VM. The purpose of the this macro is to measure how many cycles a rust
/// function takes because prover time is directly proportional to the number of riscv cycles
/// generated. It does this by making use of a risc0 provided function
/// ```rust,ignore
/// risc0_zkvm::guest::env::get_cycle_count
/// ```
/// The macro essentially generates new function with the same name by wrapping the body with a get_cycle_count
/// at the beginning and end of the function, subtracting it and then emitting it out using the
/// a custom syscall that is generated when the prover is run with the `bench` feature.
/// `send_recv_slice` is used to communicate and pass a slice to the syscall that we defined.
/// The handler for the syscall can be seen in adapters/risc0/src/host.rs and adapters/risc0/src/metrics.rs
#[proc_macro_attribute]
pub fn cycle_tracker(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    match wrap_function(input) {
        Ok(ok) => ok,
        Err(err) => err.to_compile_error().into(),
    }
}

fn wrap_function(input: ItemFn) -> Result<TokenStream, syn::Error> {
    let visibility = &input.vis;
    let name = &input.sig.ident;
    let inputs = &input.sig.inputs;
    let output = &input.sig.output;
    let block = &input.block;
    let generics = &input.sig.generics;
    let where_clause = &input.sig.generics.where_clause;
    let risc0_zkvm = syn::Ident::new("risc0_zkvm", proc_macro2::Span::call_site());
    let risc0_zkvm_platform =
        syn::Ident::new("risc0_zkvm_platform", proc_macro2::Span::call_site());

    let result = quote! {
        #visibility fn #name #generics (#inputs) #output #where_clause {
            let before = #risc0_zkvm::guest::env::get_cycle_count();
            let result = (|| #block)();
            let after = #risc0_zkvm::guest::env::get_cycle_count();

            // simple serialization to avoid pulling in bincode or other libs
            let tuple = (stringify!(#name).to_string(), (after - before) as u64);
            let mut serialized = Vec::new();
            serialized.extend(tuple.0.as_bytes());
            serialized.push(0);
            let size_bytes = tuple.1.to_ne_bytes();
            serialized.extend(&size_bytes);

            // calculate the syscall name.
            let cycle_string = String::from("cycle_metrics\0");
            let metrics_syscall_name = unsafe {
                #risc0_zkvm_platform::syscall::SyscallName::from_bytes_with_nul(cycle_string.as_ptr())
            };

            #risc0_zkvm::guest::env::send_recv_slice::<u8,u8>(metrics_syscall_name, &serialized);
            result
        }
    };
    Ok(result.into())
}
