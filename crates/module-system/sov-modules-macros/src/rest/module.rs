use darling::{ast, util, FromDeriveInput, FromField};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident};

use crate::common::{join_doc_comments, str_to_url_segment, wrap_in_new_scope};
use crate::module_info::parsing::{ModuleField, ModuleFieldAttribute, StructDef};

pub fn derive(tokens: &DeriveInput) -> syn::Result<TokenStream> {
    // This `proc-macro` partially relies on parsing logic provided by
    // `StructDef`.
    let module_struct_def = StructDef::parse(tokens)?;
    let rest_api_input = InputStruct::from_derive_input(tokens)?;

    let state_fields = ParsedStateField::parse(&module_struct_def, &rest_api_input);

    let state_item_exprs = state_fields
        .iter()
        .map(|f| {
            let ident = &f.ident;
            let ty = &f.ty;
            let state_name = format!("{ident}");
            let description = description_code(&f.rest_api_field.doc, &f.rest_api_field.attrs)?;

            Ok(quote! {
                StateItemInfo {
                    r#type: <#ty as GetStateItemInfo>::STATE_ITEM_KIND,
                    name: #state_name.to_string(),
                    description: #description,
                    namespace: <#ty as GetStateItemInfo>::NAMESPACE.into(),
                    prefix: Prefix(self.#ident.prefix().clone()),
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    let map_of_state_item_exprs = quote! {
        <Vec<StateItemInfo>>::into_iter(vec![#(#state_item_exprs),*])
            .map(|s| (s.name.clone(), s))
            .collect()
    };

    let router_nest_ops = state_fields
        .iter()
        .zip(state_item_exprs.iter())
        .map(|(f, state_item_expr)| {
            let ty = &f.ty;
            let path = format!("/state/{}", str_to_url_segment(&f.ident));
            let must_include = f.rest_api_field.include;

            // Items marked with `include` MUST be included in the final router.
            let inclusion_check = if must_include {
                quote! { StateItemRestApiExists::exists(&state_impl); }
            } else {
                quote! {}
            };

            quote! {
                router = {
                    let state_impl = StateItemRestApiImpl::<Self, #ty> {
                        api_state: api_state.clone(),
                        state_item_info: #state_item_expr,
                        phantom: PhantomData::<#ty>::default(),
                    };

                    #inclusion_check

                    let state_item_router = (&state_impl).state_item_rest_api();
                    router.nest(#path, state_item_router)
                };
            }
        })
        .collect::<Vec<_>>();

    let state_field_specs = state_fields
        .iter()
        .zip(state_item_exprs.iter())
        .map(|(f, state_item_expr)| {
            let ty = &f.ty;
            let item_name_path = format!("/state/{}", str_to_url_segment(&f.ident));
            let module_name = rest_api_input.ident.to_string();

            quote! {
                {
                    let state_impl = StateItemOpenApiSpecImpl::<#ty> {
                        state_item_info: #state_item_expr,
                        phantom: PhantomData::<#ty>::default(),
                    };

                    let mut item_spec = (&state_impl).state_item_open_api(#module_name);

                    let old_paths = std::mem::take(&mut item_spec.paths);

                    for (rel_path, path_item) in old_paths.paths {
                        let item_path = format!("{}{}", #item_name_path, rel_path);
                        item_spec.paths.paths.insert(item_path, path_item);
                    }

                    module_spec.merge(item_spec);
                };
            }
        })
        .collect::<Vec<_>>();

    let module_ty = &rest_api_input.ident;
    let description = description_code(&rest_api_input.doc, &rest_api_input.attrs)?;

    let StructDef {
        impl_generics,
        type_generics,
        where_clause,
        ..
    } = module_struct_def;

    let code = wrap_in_new_scope(&quote! {
        use ::sov_modules_api::rest::utils::*;
        use ::sov_modules_api::rest::__private::*;
        use ::sov_modules_api::rest::__private::state::*;
        use ::sov_modules_api::rest::__private::openapi::*;
        use ::sov_modules_api::rest::*;
        use ::sov_modules_api::prelude::*;
        use ::sov_modules_api::{CompileTimeNamespace, Module, ModuleInfo};

        use axum::http::StatusCode;
        use axum::Json;
        use axum::routing::get;

        use std::marker::PhantomData;
        use std::sync::Arc;
        use std::vec::Vec;
        use std::result::Result;
        use std::option::Option;

        #[automatically_derived]
        impl #impl_generics HasRestApi<<Self as ModuleInfo>::Spec> for #module_ty #type_generics #where_clause {
            fn rest_api(&self, api_state: ApiState<<Self as ModuleInfo>::Spec>) -> axum::Router<()> {
                let mut state_item_routers: Vec<axum::Router<()>> = vec![];
                let base_impl = ModuleRestApiBaseImpl::<Self> {
                    module: Arc::new(Self::default()),
                    description: #description,
                    state_items: #map_of_state_item_exprs,
                };

                let mut router: axum::Router<()> = (&base_impl).rest_api(api_state.clone());

                #(#router_nest_ops)*

                let custom_router = (self).custom_rest_api(api_state);
                router = router.nest("/", custom_router);

                router
            }

            fn openapi_spec(&self) -> Option<::sov_modules_api::prelude::utoipa::openapi::OpenApi> {
                let mut module_spec = ::sov_modules_api::prelude::utoipa::openapi::OpenApi::default();

                #(#state_field_specs)*

                if let Some(custom_spec) = (self).custom_openapi_spec() {
                    module_spec.merge(custom_spec);
                }

                Some(module_spec)
            }

        }
    });

    Ok(quote! {
        // The following code is related to REST APIs, `axum` and other things
        // that MUST NOT be compiled into zkVM code.
        ::sov_modules_api::native_only!(#code);
    })
}

fn description_code(
    override_docs: &[String],
    attrs: &[syn::Attribute],
) -> syn::Result<TokenStream> {
    let description = if override_docs.is_empty() {
        join_doc_comments(attrs)?
    } else {
        Some(override_docs.join("\n"))
    };

    if let Some(desc) = description {
        Ok(quote! { Some(#desc.to_owned()) })
    } else {
        Ok(quote! { None })
    }
}

#[derive(Clone, derive_more::Deref)]
pub struct ParsedStateField {
    #[deref]
    pub module_field: ModuleField,
    pub rest_api_field: InputField,
}

impl ParsedStateField {
    /// Iterates over a module's `struct` fields, and accumulates all state item
    /// fields in a [`Vec`]. Items marked with `skip` are ignored.
    pub fn parse<'a>(
        module_info_input: &'a StructDef,
        rest_api_input: &'a InputStruct,
    ) -> Vec<Self> {
        module_info_input
            .fields
            .iter()
            .zip(rest_api_input.fields().iter())
            .map(|(f_a, f_b)| {
                // It'd be very painful if our input sources didn't agree on the
                // order of fields or their names, so let's check just to be sure.
                assert_eq!(Some(&f_a.ident), f_b.ident.as_ref());
                ParsedStateField {
                    module_field: f_a.clone(),
                    rest_api_field: f_b.clone(),
                }
            })
            // TODO(@neysofu): instead of just filtering non-state fields, we should
            // also check that they contain no `rest_api` attributes, as that
            // would indicate incorrect usage of the macro.
            .filter(|f| {
                // We're only interested in state fields.
                matches!(f.module_field.attr, ModuleFieldAttribute::State { .. })
            })
            .filter(|f| {
                // Fields explicitly marked with `skip` are ignored.
                !f.rest_api_field.skip
            })
            .collect()
    }
}

#[derive(Debug, FromDeriveInput)]
#[darling(attributes(rest_api), supports(struct_named), forward_attrs(doc))]
pub struct InputStruct {
    pub ident: Ident,
    pub attrs: Vec<syn::Attribute>,
    pub data: ast::Data<util::Ignored, InputField>,
    #[darling(multiple)]
    pub doc: Vec<String>,
}

impl InputStruct {
    pub fn fields(&self) -> &[InputField] {
        match &self.data {
            ast::Data::Struct(s) => s.fields.as_slice(),
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, FromField)]
#[darling(attributes(rest_api), forward_attrs(doc))]
pub struct InputField {
    pub attrs: Vec<syn::Attribute>,
    pub ident: Option<Ident>,
    #[darling(default)]
    pub skip: bool,
    #[darling(default)]
    pub include: bool,
    #[darling(multiple)]
    pub doc: Vec<String>,
}
