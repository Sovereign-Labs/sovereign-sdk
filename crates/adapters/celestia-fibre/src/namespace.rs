//! Fibre blob references in a rollup namespace.
//!
//! Fibre blob data lives off-chain. The namespace carries one v2 share per blob with the Fibre
//! blob version and commitment, signed by the poster. The node and the verifier select blobs with
//! the same functions, so both agree on which blobs a block contains.

use celestia_types::namespace_data::{NamespaceData, NamespaceDataId};
use celestia_types::nmt::Namespace;
use celestia_types::state::AddressTrait;
use celestia_types::Blob;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::da::BlockHeaderTrait;

/// The only Fibre blob version this adapter reads, see `BlobID::validate` in `celestia-fibre`.
pub(crate) const FIBRE_BLOB_VERSION: u8 = 0;

/// Fibre blob referenced from a v2 share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FibreRef {
    /// Signer of the `MsgPayForFibre` that posted the blob.
    pub(crate) sender: CelestiaAddress,
    pub(crate) commitment: [u8; 32],
}

/// Checks that `namespace_data` holds every share of `namespace` in the block with `header`, and
/// returns the Fibre references among them in namespace order.
pub(crate) fn verify_namespace(
    header: &CelestiaHeader,
    namespace: Namespace,
    namespace_data: &NamespaceData,
) -> anyhow::Result<Vec<FibreRef>> {
    let dah = header.verified_dah()?;
    // Checks that the rows are exactly the ones whose namespace range contains `namespace`, and
    // that each row proof covers all of the namespace's shares in that row.
    namespace_data.verify(NamespaceDataId::new(namespace, header.height())?, dah)?;
    fibre_refs(namespace_data)
}

/// Fibre references in `namespace_data`, in namespace order.
///
/// Anyone can post to the rollup namespace, so blobs that aren't well-formed references to the
/// supported Fibre blob version are skipped instead of failing the block.
fn fibre_refs(namespace_data: &NamespaceData) -> anyhow::Result<Vec<FibreRef>> {
    let shares = namespace_data.rows().iter().flat_map(|row| &row.shares);
    Ok(Blob::reconstruct_all(shares)?
        .iter()
        .filter_map(fibre_ref)
        .collect())
}

fn fibre_ref(blob: &Blob) -> Option<FibreRef> {
    if blob.fibre_blob_version()? != u32::from(FIBRE_BLOB_VERSION) {
        return None;
    }
    Some(FibreRef {
        sender: CelestiaAddress::try_from(blob.signer.as_ref()?.as_bytes()).ok()?,
        commitment: blob.fibre_commitment()?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use celestia_types::blob::RawBlob;
    use celestia_types::row_namespace_data::RowNamespaceData;
    use celestia_types::{ExtendedHeader, Share};

    use super::*;

    pub(crate) const BATCH_NAMESPACE: Namespace = Namespace::const_v0(*b"sov-niko-a");
    pub(crate) const PROOF_NAMESPACE: Namespace = Namespace::const_v0([1; 10]);
    /// Batch namespace holds one Fibre reference and nothing else.
    pub(crate) const FIBRE_BLOCK: &str = "block_mocha_fibre_v2_1194000";
    /// Batch namespace holds a 2.5 MB regular blob followed by one Fibre reference.
    const MIXED_BLOCK: &str = "block_mocha_mixed_v1_v2_1193993";

    pub(crate) struct Fixture {
        pub(crate) header: ExtendedHeader,
        pub(crate) batch_rows: NamespaceData,
        pub(crate) proof_rows: NamespaceData,
    }

    /// Loads a Mocha block captured by the Celestia adapter tests.
    pub(crate) fn fixture(name: &str) -> Fixture {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../celestia/test_data")
            .join(name);
        let load = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();
        Fixture {
            header: serde_json::from_str(&load("header.json")).unwrap(),
            batch_rows: serde_json::from_str(&load("rollup_batch_rows.json")).unwrap(),
            proof_rows: serde_json::from_str(&load("rollup_proof_rows.json")).unwrap(),
        }
    }

    /// The reference in the last share of the namespace, read directly from the share.
    pub(crate) fn last_share_ref(rows: &NamespaceData) -> FibreRef {
        let share = rows.rows().last().unwrap().shares.last().unwrap();
        let payload = share.payload().unwrap();
        assert_eq!(
            &payload[..4],
            &[0; 4],
            "Fixture should reference Fibre blob version 0"
        );
        FibreRef {
            sender: CelestiaAddress::try_from(share.signer().unwrap().as_bytes()).unwrap(),
            commitment: payload[4..36].try_into().unwrap(),
        }
    }

    fn verify(
        header: ExtendedHeader,
        namespace: Namespace,
        rows: &NamespaceData,
    ) -> anyhow::Result<Vec<FibreRef>> {
        verify_namespace(&header.into(), namespace, rows)
    }

    #[test]
    fn finds_fibre_reference() {
        let block = fixture(FIBRE_BLOCK);
        let expected = last_share_ref(&block.batch_rows);

        let refs = verify(block.header, BATCH_NAMESPACE, &block.batch_rows).unwrap();

        assert_eq!(refs, [expected]);
    }

    #[test]
    fn skips_regular_blob() {
        let block = fixture(MIXED_BLOCK);
        let expected = last_share_ref(&block.batch_rows);

        let refs = verify(block.header, BATCH_NAMESPACE, &block.batch_rows).unwrap();

        assert_eq!(refs, [expected]);
    }

    #[test]
    fn finds_nothing_in_absent_namespace() {
        let block = fixture(FIBRE_BLOCK);

        let refs = verify(block.header, PROOF_NAMESPACE, &block.proof_rows).unwrap();

        assert_eq!(refs, []);
    }

    #[test]
    fn rejects_missing_row() {
        let block = fixture(MIXED_BLOCK);
        let mut rows = block.batch_rows.into_inner();
        rows.pop();

        assert!(verify(block.header, BATCH_NAMESPACE, &NamespaceData::new(rows)).is_err());
    }

    fn with_last_row(
        rows: NamespaceData,
        modify: impl FnOnce(&mut RowNamespaceData),
    ) -> NamespaceData {
        let mut rows = rows.into_inner();
        modify(rows.last_mut().unwrap());
        NamespaceData::new(rows)
    }

    #[test]
    fn rejects_missing_share() {
        let block = fixture(MIXED_BLOCK);
        let rows = with_last_row(block.batch_rows, |row| {
            row.shares.pop();
        });

        assert!(verify(block.header, BATCH_NAMESPACE, &rows).is_err());
    }

    #[test]
    fn rejects_modified_share() {
        let block = fixture(FIBRE_BLOCK);
        let rows = with_last_row(block.batch_rows, |row| {
            let share = row.shares.last_mut().unwrap();
            let mut bytes = share.as_ref().to_vec();
            *bytes.last_mut().unwrap() ^= 0x01;
            *share = Share::from_raw(&bytes).unwrap();
        });

        assert!(verify(block.header, BATCH_NAMESPACE, &rows).is_err());
    }

    #[test]
    fn rejects_data_of_other_namespace() {
        let block = fixture(FIBRE_BLOCK);

        assert!(verify(block.header, PROOF_NAMESPACE, &block.batch_rows).is_err());
    }

    #[test]
    fn rejects_header_with_other_dah() {
        let mut block = fixture(FIBRE_BLOCK);
        block.header.dah = fixture(MIXED_BLOCK).header.dah;

        assert!(verify(block.header, BATCH_NAMESPACE, &block.batch_rows).is_err());
    }

    fn raw_blob(share_version: u32, data: Vec<u8>) -> Blob {
        Blob::from_raw(RawBlob {
            namespace_id: BATCH_NAMESPACE.id().to_vec(),
            namespace_version: 0,
            share_version,
            data,
            signer: vec![0xAA; 20],
        })
        .unwrap()
    }

    fn reference_data(fibre_version: u32) -> Vec<u8> {
        let mut data = fibre_version.to_be_bytes().to_vec();
        data.extend_from_slice(&[0x12; 32]);
        data
    }

    #[test]
    fn reads_v2_reference() {
        let blob = raw_blob(2, reference_data(0));

        assert_eq!(
            fibre_ref(&blob),
            Some(FibreRef {
                sender: CelestiaAddress::try_from([0xAA; 20].as_slice()).unwrap(),
                commitment: [0x12; 32],
            })
        );
    }

    #[test]
    fn skips_unsupported_fibre_version() {
        let blob = raw_blob(2, reference_data(0x0102));

        assert_eq!(fibre_ref(&blob), None);
    }

    #[test]
    fn skips_regular_blob_with_reference_layout() {
        let blob = raw_blob(1, reference_data(0));

        assert_eq!(fibre_ref(&blob), None);
    }
}
