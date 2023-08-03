extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::ItemFn;
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn cycle_tracker(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let r = match wrap_function(input).into() {
        Ok(ok) => ok,
        Err(err) => err.to_compile_error().into(),
    };
    r.into()
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
    let risc0_zkvm_platform = syn::Ident::new("risc0_zkvm_platform", proc_macro2::Span::call_site());

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
