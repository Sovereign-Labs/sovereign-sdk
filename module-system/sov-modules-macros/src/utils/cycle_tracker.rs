extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::ItemFn;
use syn::FnArg;

pub fn wrap_function(input: ItemFn) -> Result<TokenStream, syn::Error> {
        let visibility = &input.vis;
        let name = &input.sig.ident;
        let inputs = &input.sig.inputs;
        let output = &input.sig.output;
        let block = &input.block;
        let risc0_zkvm = syn::Ident::new("risc0_zkvm", proc_macro2::Span::call_site());

        if let Some(self_param) = inputs.first() {
            if matches!(self_param, FnArg::Receiver(_)) {
                let result = quote! {
                #visibility fn #name (#inputs) #output {
                    let before = #risc0_zkvm::guest::env::get_cycle_count();
                    let result = (|| #block)();
                    let after = #risc0_zkvm::guest::env::get_cycle_count();
                    println!("=====> {}: {}", stringify!(#name), after - before);

                    result
                }
            };
                // println!("method: {}", result.to_string());
                Ok(result.into())
            } else {
                // function
                let result = quote! {
                #visibility fn #name (#inputs) #output {
                    let result = (|| #block)();
                    result
                }
            };
                Ok(result.into())
            }
        } else {
            // function without arguments
            let result = quote! {
            #visibility fn #name (#inputs) #output {
                let result = (|| #block)();
                result
            }
        };
            Ok(result.into())
        }


}
