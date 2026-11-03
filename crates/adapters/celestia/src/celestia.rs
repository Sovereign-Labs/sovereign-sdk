pub use sov_celestia_common::header::{
    celestia_tm_version, CelestiaHeader, Sha2Hash, TendermintHeader, GENESIS_PLACEHOLDER_HASH,
};

#[cfg(test)]
mod tests {
    use celestia_types::ExtendedHeader;

    use crate::test_helper::files::{
        with_rollup_batch_data, with_rollup_proof_data, without_rollup_batch_data,
    };
    use crate::test_helper::serialization::test_block_serialization;
    use sov_celestia_common::header::{CelestiaHeader, CompactHeader};

    const HEADER_JSON_RESPONSES: &[&str] = &[
        include_str!("../test_data/block_with_rollup_batch_data/prev_header.json"),
        include_str!("../test_data/block_with_rollup_batch_data/header.json"),
    ];

    #[test]
    fn test_compact_header_serde() {
        for header_json in HEADER_JSON_RESPONSES {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();

            let header: CompactHeader = original_header.header.into();

            let serialized_header = postcard::to_stdvec(&header).unwrap();
            let deserialized_header: CompactHeader =
                postcard::from_bytes(&serialized_header).unwrap();
            assert_eq!(deserialized_header, header);
        }
    }

    #[test]
    fn test_compact_header_hash() {
        let expected_hashes = [
            "f45a8d5efb8b64a7f1ddbdc5f24f29163ec24007dde9df62b966f2504b347fcc",
            "ff623b7e8a9075cc40fa32c64348632d80d3178e56fdef4e778ebb0a3f94f328",
        ];
        for (header_json, expected_hash) in HEADER_JSON_RESPONSES.iter().zip(expected_hashes.iter())
        {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();

            let tm_header = original_header.header.clone();
            let compact_header: CompactHeader = original_header.header.into();

            assert_eq!(tm_header.hash(), compact_header.hash());
            assert_eq!(
                expected_hash,
                &hex::encode(compact_header.hash().as_bytes()),
            );

            assert_eq!(tm_header.hash(), compact_header.hash(),);
        }
    }

    #[test]
    fn test_zkvm_serde_celestia_header() {
        // regression https://github.com/eigerco/celestia-tendermint-rs/pull/12
        for header_json in HEADER_JSON_RESPONSES {
            let original_header: ExtendedHeader = serde_json::from_str(header_json).unwrap();
            let cel_header =
                CelestiaHeader::new(original_header.dah, original_header.header.into());

            let serialized = risc0_zkvm::serde::to_vec(&cel_header).unwrap();
            let deserialized = risc0_zkvm::serde::from_slice(&serialized).unwrap();

            assert_eq!(cel_header, deserialized);
        }
    }

    #[tokio::test]
    async fn test_zkvm_serde_block_with_batch_serialization() {
        let block = with_rollup_batch_data::filtered_block();

        test_block_serialization(block).await;
    }

    #[tokio::test]
    async fn test_zkvm_serde_block_without_batch_serialization() {
        let block = without_rollup_batch_data::filtered_block();

        test_block_serialization(block).await;
    }

    #[tokio::test]
    async fn test_zkvm_serde_block_with_proof_serialization() {
        let block = with_rollup_proof_data::filtered_block();
        test_block_serialization(block).await;
    }
}
