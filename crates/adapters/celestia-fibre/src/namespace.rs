//! Fibre blob references in a rollup namespace.
//!
//! Fibre blob data lives off-chain. The namespace carries one v2 share per blob with the Fibre
//! blob version and commitment, signed by the poster. Anyone can post other blobs to the
//! namespace, so [`FibreNamespaceProof`] carries only the first share of every sequence: enough
//! to find the references and to skip everything else by its length. The node and the verifier
//! select blobs with the same function, so both agree on which blobs a block contains.

use anyhow::Context;
use celestia_types::consts::appconsts;
use celestia_types::namespace_data::{NamespaceData, NamespaceDataId};
use celestia_types::nmt::{Namespace, NamespaceProof, NamespacedHash};
use celestia_types::state::AddressTrait;
use celestia_types::{Blob, DataAvailabilityHeader, Share};
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::da::BlockHeaderTrait;

/// The only Fibre blob version this adapter reads, see `BlobID::validate` in `celestia-fibre`.
pub(crate) const FIBRE_BLOB_VERSION: u8 = 0;

/// Proof of every sequence (blob or namespace padding) a rollup namespace holds in a block.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FibreNamespaceProof {
    /// The namespace has no shares: absence proofs for the rows whose range contains it.
    Empty(NamespaceData),
    /// The namespace has shares.
    ///
    /// Shares in between are not carried. They are safe to skip because Celestia consensus only
    /// accepts squares sorted by namespace, with every sequence taking exactly the shares its
    /// length needs, and both ends of each gap are proven to be in the namespace.
    Sequences {
        /// Row of the namespace's first share.
        first_row: u16,
        /// First share of every sequence in namespace order. The position of each one follows
        /// from the length of the previous one.
        sequence_starts: Vec<ProvenShare>,
        /// Last share of the namespace, proving that no namespace share follows it.
        last_share: Box<ProvenShare>,
    },
}

/// Share with a proof of its inclusion in its row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProvenShare {
    share: Share,
    proof: NamespaceProof,
}

/// Fibre blob referenced from a v2 share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FibreRef {
    /// Signer of the `MsgPayForFibre` that posted the blob.
    pub(crate) sender: CelestiaAddress,
    pub(crate) commitment: [u8; 32],
}

/// Checks that `proof` covers every sequence of `namespace` in the block with `header`, and
/// returns the Fibre references among them in namespace order.
pub(crate) fn verify_namespace_proof(
    header: &CelestiaHeader,
    namespace: Namespace,
    proof: &FibreNamespaceProof,
) -> anyhow::Result<Vec<FibreRef>> {
    let dah = header.verified_dah()?;
    match proof {
        FibreNamespaceProof::Empty(namespace_data) => {
            namespace_data.verify(NamespaceDataId::new(namespace, header.height())?, dah)?;
            // Otherwise the prover could pick between two ways to read the same namespace.
            anyhow::ensure!(
                namespace_data
                    .rows()
                    .iter()
                    .all(|row| row.shares.is_empty()),
                "Namespace has shares, but the proof is for an empty namespace"
            );
            Ok(Vec::new())
        }
        FibreNamespaceProof::Sequences {
            first_row,
            sequence_starts,
            last_share,
        } => {
            let square = OriginalSquare::new(dah, namespace);
            let first = sequence_starts.first().context("No sequence starts")?;
            let start = u64::from(*first_row) * square.width + u64::from(first.proof.start_idx());

            let mut position = start;
            let mut refs = Vec::new();
            for (index, sequence_start) in sequence_starts.iter().enumerate() {
                let shares = square
                    .check_share(position, sequence_start)
                    .and_then(|()| sequence_shares(&sequence_start.share))
                    .with_context(|| format!("Invalid sequence start {index}"))?;
                position = position
                    .checked_add(shares)
                    .context("Share position overflow")?;
                refs.extend(start_fibre_ref(&sequence_start.share));
            }
            let end = position;

            square
                .check_share(end - 1, last_share)
                .context("Invalid last share")?;
            square.check_left_boundary(start, &first.proof)?;
            square.check_right_boundary(end, &last_share.proof)?;
            Ok(refs)
        }
    }
}

/// Positions in the original data square, where shares are sorted by namespace row by row.
struct OriginalSquare<'a> {
    dah: &'a DataAvailabilityHeader,
    namespace: Namespace,
    /// Number of original shares in a row (and of original rows).
    width: u64,
}

impl<'a> OriginalSquare<'a> {
    fn new(dah: &'a DataAvailabilityHeader, namespace: Namespace) -> Self {
        Self {
            dah,
            namespace,
            width: u64::from(dah.square_width() / 2),
        }
    }

    fn row_root(&self, row: u64) -> anyhow::Result<NamespacedHash> {
        u16::try_from(row)
            .ok()
            .and_then(|row| self.dah.row_root(row))
            .with_context(|| format!("Row {row} is outside the square"))
    }

    /// Checks that `proven` is the namespace share at `position`.
    fn check_share(&self, position: u64, proven: &ProvenShare) -> anyhow::Result<()> {
        let (row, column) = (position / self.width, position % self.width);
        anyhow::ensure!(
            row < self.width,
            "Share {position} is outside the original data square"
        );
        let proof = &proven.proof;
        // `verify_range` below checks that the proof covers exactly one share.
        anyhow::ensure!(
            u64::from(proof.start_idx()) == column,
            "Proof is for share {} of row {row}, expected share {column}",
            proof.start_idx()
        );
        anyhow::ensure!(
            proven.share.namespace() == self.namespace,
            "Share is from another namespace"
        );
        proof
            .verify_range(
                &self.row_root(row)?,
                std::slice::from_ref(&proven.share),
                *self.namespace,
            )
            .map_err(|error| anyhow::anyhow!("Invalid share proof: {error:?}"))
    }

    /// Checks that no namespace share comes before `start`. `proof` proves the share at `start`.
    fn check_left_boundary(&self, start: u64, proof: &NamespaceProof) -> anyhow::Result<()> {
        let (row, column) = (start / self.width, start % self.width);
        let previous_max = if column > 0 {
            proof
                .rightmost_left_sibling()
                .context("First share proof has no left sibling")?
                .max_namespace()
        } else if row > 0 {
            self.row_root(row - 1)?.max_namespace()
        } else {
            return Ok(());
        };
        anyhow::ensure!(
            previous_max < *self.namespace,
            "Namespace has shares before the first sequence start"
        );
        Ok(())
    }

    /// Checks that no namespace share comes at or after `end`. `proof` proves the share before.
    fn check_right_boundary(&self, end: u64, proof: &NamespaceProof) -> anyhow::Result<()> {
        let (row, column) = (end / self.width, end % self.width);
        // At a row end, the right sibling is the parity half of the row, so check the next row.
        let next_min = if column > 0 {
            proof
                .leftmost_right_sibling()
                .context("Last share proof has no right sibling")?
                .min_namespace()
        } else if row < self.width {
            self.row_root(row)?.min_namespace()
        } else {
            return Ok(());
        };
        anyhow::ensure!(
            next_min > *self.namespace,
            "Namespace has shares after the last sequence"
        );
        Ok(())
    }
}

/// Number of shares taken by the sequence that `share` starts.
fn sequence_shares(share: &Share) -> anyhow::Result<u64> {
    let version = share.info_byte().context("Parity share")?.version();
    let sequence_len = share
        .sequence_length()
        .context("Share doesn't start a sequence")?;
    // The share layout of future versions is unknown.
    anyhow::ensure!(
        version <= appconsts::SHARE_VERSION_TWO,
        "Unsupported share version {version}"
    );
    let mut first_share_capacity = appconsts::FIRST_SPARSE_SHARE_CONTENT_SIZE as u64;
    if share.signer().is_some() {
        first_share_capacity -= appconsts::SIGNER_SIZE as u64;
    }
    // Same as `shares_needed_for_blob` in `celestia-types`, which isn't public.
    Ok(
        match u64::from(sequence_len).checked_sub(first_share_capacity) {
            None => 1,
            Some(rest) => {
                1 + rest.div_ceil(appconsts::CONTINUATION_SPARSE_SHARE_CONTENT_SIZE as u64)
            }
        },
    )
}

/// Fibre reference in the first share of a sequence.
fn start_fibre_ref(share: &Share) -> Option<FibreRef> {
    if share.info_byte()?.version() != appconsts::SHARE_VERSION_TWO {
        return None;
    }
    // A sequence that doesn't fit in its first share fails to reconstruct here. Fibre references
    // always fit, so such a sequence isn't one and is skipped like any other blob.
    let blob = Blob::reconstruct(std::iter::once(share)).ok()?;
    fibre_ref(&blob)
}

/// Anyone can post to the rollup namespace, so blobs that aren't well-formed references to the
/// supported Fibre blob version are skipped instead of failing the block.
fn fibre_ref(blob: &Blob) -> Option<FibreRef> {
    if blob.fibre_blob_version()? != u32::from(FIBRE_BLOB_VERSION) {
        return None;
    }
    Some(FibreRef {
        sender: CelestiaAddress::try_from(blob.signer.as_ref()?.as_bytes()).ok()?,
        commitment: blob.fibre_commitment()?,
    })
}

/// Builds the proof for `namespace` from all of its shares, which are checked against `header`
/// first.
#[cfg(feature = "native")]
pub(crate) fn prove_namespace(
    header: &CelestiaHeader,
    namespace: Namespace,
    namespace_data: NamespaceData,
) -> anyhow::Result<FibreNamespaceProof> {
    let dah = header.verified_dah()?;
    namespace_data.verify(NamespaceDataId::new(namespace, header.height())?, dah)?;
    if namespace_data
        .rows()
        .iter()
        .all(|row| row.shares.is_empty())
    {
        return Ok(FibreNamespaceProof::Empty(namespace_data));
    }

    // Same row selection as `NamespaceData::verify`, which has just matched the rows to these.
    // `row_contains` fails only for rows outside the square.
    let row_indexes =
        (0..dah.square_width()).filter(|&row| dah.row_contains(row, namespace).unwrap_or(false));
    let rows = row_indexes
        .zip(namespace_data.rows())
        .filter(|(_, row)| !row.shares.is_empty())
        .collect::<Vec<_>>();
    let (first_row, _) = rows[0];

    let mut sequence_starts = Vec::new();
    let mut next_start = 0u64;
    let mut index_in_namespace = 0u64;
    for (_, row) in &rows {
        for index in 0..row.shares.len() {
            if index_in_namespace == next_start {
                let proven = prove_share(row, index, namespace)?;
                next_start += sequence_shares(&proven.share)
                    .with_context(|| format!("Share {index_in_namespace} of the namespace"))?;
                sequence_starts.push(proven);
            }
            index_in_namespace += 1;
        }
    }
    let (_, last_row) = rows[rows.len() - 1];
    let last_share = Box::new(prove_share(last_row, last_row.shares.len() - 1, namespace)?);

    Ok(FibreNamespaceProof::Sequences {
        first_row,
        sequence_starts,
        last_share,
    })
}

/// Narrows the proof of a row's namespace shares to the share at `index`.
#[cfg(feature = "native")]
fn prove_share(
    row: &celestia_types::row_namespace_data::RowNamespaceData,
    index: usize,
    namespace: Namespace,
) -> anyhow::Result<ProvenShare> {
    let proof = row
        .proof
        .narrow_range(&row.shares[..index], &row.shares[index + 1..], *namespace)
        .map_err(|error| anyhow::anyhow!("Failed to narrow the row proof: {error:?}"))?;
    Ok(ProvenShare {
        share: row.shares[index].clone(),
        proof: proof.into(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use celestia_types::blob::RawBlob;
    use celestia_types::{ExtendedDataSquare, ExtendedHeader};

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

    impl Fixture {
        pub(crate) fn celestia_header(&self) -> CelestiaHeader {
            self.header.clone().into()
        }

        pub(crate) fn prove(
            &self,
            namespace: Namespace,
            rows: &NamespaceData,
        ) -> FibreNamespaceProof {
            prove_namespace(&self.celestia_header(), namespace, rows.clone()).unwrap()
        }
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
        block: &Fixture,
        namespace: Namespace,
        proof: &FibreNamespaceProof,
    ) -> anyhow::Result<Vec<FibreRef>> {
        verify_namespace_proof(&block.celestia_header(), namespace, proof)
    }

    fn root_cause(result: anyhow::Result<Vec<FibreRef>>) -> String {
        result.unwrap_err().root_cause().to_string()
    }

    /// Mixed block's batch namespace proof, with `modify` applied to its parts.
    fn modified_mixed_proof(
        modify: impl FnOnce(&mut u16, &mut Vec<ProvenShare>, &mut ProvenShare),
    ) -> (Fixture, FibreNamespaceProof) {
        let block = fixture(MIXED_BLOCK);
        let FibreNamespaceProof::Sequences {
            mut first_row,
            mut sequence_starts,
            mut last_share,
        } = block.prove(BATCH_NAMESPACE, &block.batch_rows)
        else {
            panic!("Mixed block should have shares in the batch namespace");
        };
        modify(&mut first_row, &mut sequence_starts, &mut last_share);
        let proof = FibreNamespaceProof::Sequences {
            first_row,
            sequence_starts,
            last_share,
        };
        (block, proof)
    }

    #[test]
    fn finds_fibre_reference() {
        let block = fixture(FIBRE_BLOCK);
        let proof = block.prove(BATCH_NAMESPACE, &block.batch_rows);

        let refs = verify(&block, BATCH_NAMESPACE, &proof).unwrap();

        assert_eq!(refs, [last_share_ref(&block.batch_rows)]);
    }

    #[test]
    fn skips_regular_blob() {
        let block = fixture(MIXED_BLOCK);
        let proof = block.prove(BATCH_NAMESPACE, &block.batch_rows);

        let refs = verify(&block, BATCH_NAMESPACE, &proof).unwrap();

        assert_eq!(refs, [last_share_ref(&block.batch_rows)]);
    }

    #[test]
    fn carries_only_sequence_starts() {
        let (_, proof) = modified_mixed_proof(|_, _, _| {});
        let FibreNamespaceProof::Sequences {
            sequence_starts, ..
        } = proof
        else {
            unreachable!()
        };

        assert_eq!(
            sequence_starts.len(),
            2,
            "Proof should carry the regular blob's first share and the Fibre reference"
        );
    }

    #[test]
    fn finds_nothing_in_absent_namespace() {
        let block = fixture(FIBRE_BLOCK);
        let proof = block.prove(PROOF_NAMESPACE, &block.proof_rows);

        let refs = verify(&block, PROOF_NAMESPACE, &proof).unwrap();

        assert_eq!(refs, []);
    }

    /// Fixtures of the Celestia adapter with namespace padding, parity boundaries, several rows
    /// and other layouts. Each one's namespace is read from its shares.
    #[test]
    fn verifies_namespaces_of_celestia_fixtures() {
        let fixtures = [
            FIBRE_BLOCK,
            MIXED_BLOCK,
            "block_from_testnet",
            "block_mocha_multi_candidate_rows_10261831",
            "block_mocha_no_shares",
            "block_mocha_with_tail_padding",
            "block_with_batch_and_proof",
            "block_with_medium_size_single_rollup_batch_data",
            "block_with_mixed_v0_v1_batches",
            "block_with_mixed_v0_v1_multi_v1_parity_boundary",
            "block_with_namespace_padding",
            "block_with_parity_boundary_followed_by_namespace",
            "block_with_preceding_blobs_from_different_namespaces",
            "block_with_rollup_batch_data",
            "block_with_rollup_proof_data",
            "block_with_several_large_rollup_batches",
            "block_with_several_medium_rollup_batches",
            "block_with_several_small_rollup_batches",
            "block_without_rollup_batch_data",
        ];
        for name in fixtures {
            let block = fixture(name);
            for rows in [&block.batch_rows, &block.proof_rows] {
                let Some(share) = rows.rows().iter().flat_map(|row| &row.shares).next() else {
                    continue;
                };
                let namespace = share.namespace();
                let proof = block.prove(namespace, rows);

                verify(&block, namespace, &proof)
                    .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            }
        }
    }

    #[test]
    fn rejects_namespace_data_with_shares() {
        let block = fixture(FIBRE_BLOCK);
        let proof = FibreNamespaceProof::Empty(block.batch_rows.clone());

        assert_eq!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
            "Namespace has shares, but the proof is for an empty namespace"
        );
    }

    #[test]
    fn rejects_proof_without_sequence_starts() {
        let (block, proof) = modified_mixed_proof(|_, starts, _| starts.clear());

        assert_eq!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
            "No sequence starts"
        );
    }

    /// The only row where the remaining proof lines up is the Fibre reference's row, and there
    /// the regular blob before it is in the way.
    #[test]
    fn rejects_missing_first_sequence_in_any_row() {
        let (block, proof) = modified_mixed_proof(|_, starts, _| {
            starts.remove(0);
        });
        let FibreNamespaceProof::Sequences {
            sequence_starts,
            last_share,
            ..
        } = proof
        else {
            unreachable!()
        };
        let width = block.header.dah.square_width() / 2;

        let errors = (0..width)
            .map(|first_row| {
                let proof = FibreNamespaceProof::Sequences {
                    first_row,
                    sequence_starts: sequence_starts.clone(),
                    last_share: last_share.clone(),
                };
                root_cause(verify(&block, BATCH_NAMESPACE, &proof))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            errors
                .iter()
                .filter(|error| *error == "Namespace has shares before the first sequence start")
                .count(),
            1,
            "Errors: {errors:?}"
        );
    }

    #[test]
    fn rejects_missing_last_sequence() {
        let block = fixture(MIXED_BLOCK);
        let rows = block.batch_rows.rows();
        // Last share of the regular blob, right before the Fibre reference.
        let last_row = rows.last().unwrap();
        let blob_end = if last_row.shares.len() > 1 {
            prove_share(last_row, last_row.shares.len() - 2, BATCH_NAMESPACE).unwrap()
        } else {
            let row = &rows[rows.len() - 2];
            prove_share(row, row.shares.len() - 1, BATCH_NAMESPACE).unwrap()
        };
        let (_, proof) = modified_mixed_proof(|_, starts, last_share| {
            starts.pop();
            *last_share = blob_end;
        });

        assert_eq!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
            "Namespace has shares after the last sequence"
        );
    }

    /// The Fibre reference is the real last share, but it's not where the proof ends.
    #[test]
    fn rejects_last_share_from_other_position() {
        let (block, proof) = modified_mixed_proof(|_, starts, last_share| {
            *last_share = starts.pop().unwrap();
        });

        assert!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)).starts_with("Proof is for share"),
            "Last share should be checked at the namespace end"
        );
    }

    #[test]
    fn rejects_first_row_outside_original_square() {
        let width = fixture(MIXED_BLOCK).header.dah.square_width() / 2;
        let (block, proof) = modified_mixed_proof(|first_row, _, _| *first_row = width);

        assert!(root_cause(verify(&block, BATCH_NAMESPACE, &proof))
            .ends_with("is outside the original data square"));
    }

    #[test]
    fn rejects_extra_sequence_start() {
        let (block, proof) = modified_mixed_proof(|_, starts, _| starts.push(starts[1].clone()));

        assert!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)).starts_with("Proof is for share"),
            "Last share should no longer be at the namespace end"
        );
    }

    #[test]
    fn rejects_sequence_starts_out_of_order() {
        let (block, proof) = modified_mixed_proof(|_, starts, _| starts.swap(0, 1));

        assert_eq!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
            "Invalid share proof: InvalidRoot"
        );
    }

    #[test]
    fn rejects_modified_sequence_start() {
        let (block, proof) = modified_mixed_proof(|_, starts, _| {
            let share = &mut starts[1].share;
            let mut bytes = share.as_ref().to_vec();
            *bytes.last_mut().unwrap() ^= 0x01;
            *share = Share::from_raw(&bytes).unwrap();
        });

        assert_eq!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
            "Invalid share proof: InvalidRoot"
        );
    }

    #[test]
    fn rejects_wrong_first_row() {
        let (block, proof) = modified_mixed_proof(|first_row, _, _| *first_row += 1);

        assert_eq!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
            "Invalid share proof: InvalidRoot"
        );
    }

    #[test]
    fn rejects_proof_of_other_namespace() {
        let (block, proof) = modified_mixed_proof(|_, _, _| {});

        assert_eq!(
            root_cause(verify(&block, PROOF_NAMESPACE, &proof)),
            "Share is from another namespace"
        );
    }

    #[test]
    fn rejects_header_with_other_dah() {
        let (mut block, proof) = modified_mixed_proof(|_, _, _| {});
        block.header.dah = fixture(FIBRE_BLOCK).header.dah;

        assert_eq!(
            root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
            "Data root hash doesn't match computed one"
        );
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
    fn reads_v2_reference_from_its_share() {
        let shares = raw_blob(2, reference_data(0)).to_shares().unwrap();

        assert_eq!(
            start_fibre_ref(&shares[0]),
            fibre_ref(&raw_blob(2, reference_data(0)))
        );
    }

    #[test]
    fn skips_unsupported_fibre_version() {
        let blob = raw_blob(2, reference_data(0x0102));

        assert_eq!(fibre_ref(&blob), None);
    }

    #[test]
    fn skips_regular_blob_with_reference_layout() {
        let shares = raw_blob(1, reference_data(0)).to_shares().unwrap();

        assert_eq!(start_fibre_ref(&shares[0]), None);
    }

    #[test]
    fn counts_shares_of_multi_share_blob() {
        // 3 shares only because the signer takes room in the first share.
        let shares = raw_blob(1, vec![0x12; 950]).to_shares().unwrap();
        assert_eq!(shares.len(), 3, "Blob should take 3 shares");

        assert_eq!(sequence_shares(&shares[0]).unwrap(), 3);
    }

    mod synthetic_square {
        //! 4x4 original squares for layouts the captured blocks don't have.

        use super::*;

        /// Namespaces sorted before and after [`BATCH_NAMESPACE`].
        const BEFORE: Namespace = PROOF_NAMESPACE;
        const AFTER: Namespace = Namespace::const_v0([0xF0; 10]);

        fn single_share_blob(namespace: Namespace) -> Share {
            let shares = Blob::new(namespace, vec![0x12; 100], None)
                .unwrap()
                .to_shares()
                .unwrap();
            assert_eq!(shares.len(), 1);
            shares[0].clone()
        }

        fn padding(namespace: Namespace) -> Share {
            let mut bytes = namespace.as_bytes().to_vec();
            // Share version 0, sequence start, sequence length 0.
            bytes.extend_from_slice(&[0x01, 0, 0, 0, 0]);
            bytes.resize(appconsts::SHARE_SIZE, 0);
            Share::from_raw(&bytes).unwrap()
        }

        fn fibre_reference() -> Share {
            raw_blob(2, reference_data(0)).to_shares().unwrap()[0].clone()
        }

        /// Square with `shares` first and tail padding after, under the Fibre block's header
        /// changed to commit to it. Returns the block and the batch namespace's data.
        fn square(shares: Vec<Share>) -> (Fixture, NamespaceData) {
            let mut ods = shares;
            ods.resize(16, padding(Namespace::TAIL_PADDING));
            let eds =
                ExtendedDataSquare::from_ods(ods.iter().map(Share::to_vec).collect()).unwrap();
            let dah = celestia_types::DataAvailabilityHeader::from_eds(&eds);
            let mut block = fixture(FIBRE_BLOCK);
            block.header.header.data_hash = Some(dah.hash());
            block.header.dah = dah;
            let height = block.celestia_header().height();
            let rows = eds
                .get_namespace_data(BATCH_NAMESPACE, &block.header.dah, height)
                .unwrap()
                .into_iter()
                .map(|(_, row)| row)
                .collect();
            (block, NamespaceData::new(rows))
        }

        /// Proof of `rows` with `modify` applied to its sequence starts and last share.
        fn modified_proof(
            block: &Fixture,
            rows: &NamespaceData,
            modify: impl FnOnce(&mut Vec<ProvenShare>, &mut ProvenShare),
        ) -> FibreNamespaceProof {
            let FibreNamespaceProof::Sequences {
                first_row,
                mut sequence_starts,
                mut last_share,
            } = block.prove(BATCH_NAMESPACE, rows)
            else {
                panic!("Square should have shares in the batch namespace");
            };
            modify(&mut sequence_starts, &mut last_share);
            FibreNamespaceProof::Sequences {
                first_row,
                sequence_starts,
                last_share,
            }
        }

        #[test]
        fn skips_namespace_padding() {
            let (block, rows) = square(vec![
                single_share_blob(BEFORE),
                single_share_blob(BATCH_NAMESPACE),
                padding(BATCH_NAMESPACE),
                fibre_reference(),
                single_share_blob(AFTER),
            ]);
            let proof = block.prove(BATCH_NAMESPACE, &rows);

            let refs = verify(&block, BATCH_NAMESPACE, &proof).unwrap();

            assert_eq!(refs, [start_fibre_ref(&fibre_reference()).unwrap()]);
        }

        #[test]
        fn accepts_namespace_starting_a_row() {
            let (block, rows) = square(vec![
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                fibre_reference(),
                single_share_blob(AFTER),
            ]);
            let proof = block.prove(BATCH_NAMESPACE, &rows);

            verify(&block, BATCH_NAMESPACE, &proof).unwrap();
        }

        /// The skipped blob ends the previous row, so the proof starts at a row start.
        #[test]
        fn rejects_skipped_first_blob_in_previous_row() {
            let (block, rows) = square(vec![
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                single_share_blob(BATCH_NAMESPACE),
                fibre_reference(),
                single_share_blob(AFTER),
            ]);
            let proof = match modified_proof(&block, &rows, |starts, _| {
                starts.remove(0);
            }) {
                FibreNamespaceProof::Sequences {
                    sequence_starts,
                    last_share,
                    ..
                } => FibreNamespaceProof::Sequences {
                    first_row: 1,
                    sequence_starts,
                    last_share,
                },
                FibreNamespaceProof::Empty(_) => unreachable!(),
            };

            assert_eq!(
                root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
                "Namespace has shares before the first sequence start"
            );
        }

        #[test]
        fn accepts_namespace_ending_a_row() {
            let (block, rows) = square(vec![
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                fibre_reference(),
                single_share_blob(AFTER),
            ]);
            let proof = block.prove(BATCH_NAMESPACE, &rows);

            verify(&block, BATCH_NAMESPACE, &proof).unwrap();
        }

        /// The skipped blob starts the next row, so the proof ends at a row end.
        #[test]
        fn rejects_skipped_last_blob_in_next_row() {
            let (block, rows) = square(vec![
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                single_share_blob(BEFORE),
                fibre_reference(),
                single_share_blob(BATCH_NAMESPACE),
                single_share_blob(AFTER),
            ]);
            let proof = modified_proof(&block, &rows, |starts, last_share| {
                starts.pop();
                *last_share = starts[0].clone();
            });

            assert_eq!(
                root_cause(verify(&block, BATCH_NAMESPACE, &proof)),
                "Namespace has shares after the last sequence"
            );
        }

        #[test]
        fn accepts_namespace_ending_the_square() {
            let mut shares = vec![single_share_blob(BEFORE)];
            shares.extend(std::iter::repeat_n(single_share_blob(BATCH_NAMESPACE), 14));
            shares.push(fibre_reference());
            let (block, rows) = square(shares);
            let proof = block.prove(BATCH_NAMESPACE, &rows);

            let refs = verify(&block, BATCH_NAMESPACE, &proof).unwrap();

            assert_eq!(refs, [start_fibre_ref(&fibre_reference()).unwrap()]);
        }
    }
}
