use proc_macro2::{Ident, Span, TokenStream};
use sov_chain_config::{
    encode_chain_config_record, parse_chain_config_value, ChainConfig, CHAIN_DATA_ELF_SECTION_NAME,
    CHAIN_DATA_MACHO_SECTION_NAME,
};

use crate::manifest::Manifest;

#[derive(Clone)]
pub struct EmbedChainConfigInput {
    pub name: Ident,
}

impl syn::parse::Parse for EmbedChainConfigInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = input.parse()?;

        Ok(Self { name })
    }
}

pub fn expand_embed_chain_config(input: &EmbedChainConfigInput) -> syn::Result<TokenStream> {
    let manifest = Manifest::read_constants(&input.name)?;
    let config = parse_chain_config_value(manifest.toml_value().clone())
        .map_err(|error| syn::Error::new(input.name.span(), error))?;
    let record = encode_chain_config(&config, input.name.span())?;

    let len = record.len();
    let bytes = syn::LitByteStr::new(&record, Span::call_site());
    let name = &input.name;
    let elf_section = syn::LitStr::new(CHAIN_DATA_ELF_SECTION_NAME, Span::call_site());
    let macho_section = syn::LitStr::new(CHAIN_DATA_MACHO_SECTION_NAME, Span::call_site());

    // On targets without a supported patchable-section format the record is embedded as a plain
    // static: the build-time chain configuration still works, it just cannot be patched
    // post-link.
    Ok(quote::quote!(
        #[used]
        #[cfg_attr(any(target_os = "linux", target_os = "zkvm"), unsafe(link_section = #elf_section))]
        #[cfg_attr(target_os = "macos", unsafe(link_section = #macho_section))]
        static #name: [u8; #len] = *#bytes;
    ))
}

fn encode_chain_config(config: &ChainConfig, span: Span) -> syn::Result<Vec<u8>> {
    encode_chain_config_record(config)
        .map(|record| record.to_vec())
        .map_err(|error| syn::Error::new(span, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sov_chain_config::{
        ChainHashOverride, BINARY_SECTION_HEADER_LEN, CHAIN_CONFIG_RECORD_SIZE,
    };

    fn config(overrides: Vec<ChainHashOverride>) -> ChainConfig {
        ChainConfig {
            chain_id: 4321,
            chain_name: "TestChain".to_owned(),
            chain_hash_overrides: overrides,
        }
    }

    fn hash_override(start_height: u64, end_height: u64) -> ChainHashOverride {
        ChainHashOverride {
            start_height,
            end_height,
            chain_hash: [0x11; 32],
            grace_period: 10,
        }
    }

    #[test]
    fn record_has_fixed_size_and_zero_padding() {
        let record =
            encode_chain_config(&config(vec![hash_override(0, 100)]), Span::call_site()).unwrap();
        assert_eq!(record.len(), CHAIN_CONFIG_RECORD_SIZE);
        let payload_len = u32::from_le_bytes(record[24..28].try_into().unwrap()) as usize;
        assert!(record[BINARY_SECTION_HEADER_LEN + payload_len..]
            .iter()
            .all(|byte| *byte == 0));
    }

    #[test]
    fn rejects_non_contiguous_overrides() {
        let config = config(vec![hash_override(0, 100), hash_override(101, 200)]);
        assert!(encode_chain_config(&config, Span::call_site()).is_err());
    }
}
