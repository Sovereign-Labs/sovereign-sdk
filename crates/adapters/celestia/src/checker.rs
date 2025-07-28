//! Utilities to check Celestia adapter in production settings.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use rand::RngCore;
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait};
use sov_rollup_interface::node::da::{DaService, SlotData};

use crate::CelestiaService;

// Using standard hasher, because it is enough for checking uniqueness of blobs data.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Tests the full lifecycle of blobs sent through the Celestia service by submitting
/// data and verifying same data has be received.
///
/// This function performs the following steps:
/// 1. Records the head block before sending any blobs
/// 2. Sends each blob in the provided array to the Celestia DA
/// 3. Tracks each sent blob by its **bytes** hash
/// 4. Records the head block after all blobs have been sent.
/// 5. Reads all blocks produced during submission.
/// 6. Confirms that all sent blobs were received intact with matching hashes. Skips blobs from other senders.
async fn check_blobs_roundtrip(
    da_service: &CelestiaService,
    blobs: &[Vec<u8>],
) -> anyhow::Result<()> {
    let mut sent_blobs: HashMap<u64, HexHash> = HashMap::with_capacity(blobs.len());
    let sender = da_service.get_signer().await;

    let head_before = da_service.get_head_block_header().await?;
    // Padding from the previous round
    let head_before = da_service.get_block_at(head_before.height() + 1).await?;
    // Blob cannot land on the previous block.
    let start = head_before.header.height();

    for blob in blobs {
        let receipt = da_service.send_transaction(blob).await.await??;
        let bytes_hash = hash_bytes(blob);
        tracing::info!(
            "Sent blob of size {} bytes hash {} blob hash {}",
            blob.len(),
            bytes_hash,
            receipt.blob_hash
        );
        let prev_value = sent_blobs.insert(bytes_hash, receipt.blob_hash);
        if prev_value.is_some() {
            anyhow::bail!(
                "Non unique blob {} size={}, test cannot be performed",
                bytes_hash,
                blob.len()
            );
        }
    }

    let head_after = da_service.get_head_block_header().await?;

    // But blob can land in follow up blocks
    let end = head_after.height().saturating_add(2);
    for height in start..=end {
        let block = da_service.get_block_at(height).await?;

        let (received_blobs, _) = da_service.extract_relevant_blobs_with_proof(&block).await;
        tracing::info!(
            "Inspecting block {}. batch blobs: {} proof blobs: {}",
            block.header().display(),
            received_blobs.batch_blobs.len(),
            received_blobs.proof_blobs.len(),
        );

        for mut batch in received_blobs.batch_blobs {
            if batch.sender() != sender {
                continue;
            }

            tracing::info!("Received batch hash: {}, height={}", batch.hash(), height);
            let data = batch.full_data();
            let data_len = data.len();
            let bytes_hash = hash_bytes(data);
            match sent_blobs.remove(&bytes_hash) {
                None => {
                    anyhow::bail!(
                        "Received blob on height={} celestia_hash={} bytes_hash={} len={} not found in sent blobs. Bug or there's another sender colluding with the test.",
                        block.header().height(),
                        batch.hash(),
                        bytes_hash,
                        data_len,
                    );
                }
                Some(sent_blob_hash) => {
                    if sent_blob_hash.0 != *batch.hash().inner() {
                        anyhow::bail!("Blob hashes do not match for the same blob data");
                    }
                }
            }
        }
    }

    if !sent_blobs.is_empty() {
        tracing::info!("Remaining blobs: {:?}", sent_blobs);
        anyhow::bail!("Error: {} blobs were not received", sent_blobs.len());
    }

    Ok(())
}

/// Check that CelestiaService can submit and receive blobs properly.
pub async fn check_da_service(da_service: &CelestiaService, rounds: usize) -> anyhow::Result<()> {
    if rounds == 0 {
        anyhow::bail!("Cannot run test with 0 rounds");
    }
    // 478 bytes - Fills a single share exactly. No padding, no continuation.
    // 479 bytes - Triggers 2 shares. 1 byte spills into second share, most of second is padding.
    let around_1_share = 477..481;
    // 960 bytes - Exactly fills 2 shares (478 + 482).
    // Verifies clean split, no padding.
    let around_2_shares = 955..962;
    // 1440 bytes - Fills exactly 3 shares (478 + 482 + 482).
    // Checks proper continuation and full share usage.
    let around_3_shares = 1435..1445;
    //  Boundary Crossing / Subtree Edge Cases
    // 2046 bytes - Just under 5 shares. Should use 5 shares, with 100% full shares, last one mostly full.
    // 2047 bytes - Exactly fills 5 shares (478 + 482×4).
    // 2048 bytes - Just over 5-share capacity.
    // Creates sixth share with 1 byte of data + 481 bytes of padding.
    let boundary_crossing = 2040..2050;

    //  Power-of-Two Share-Length Transitions
    // (Useful for verifying padding and alignment with respect to square or NMT groupings.)
    let power_of_two_sizes = vec![
        // Fills 8 shares (1×478 + 7×482). Check that exact power-of-2 shares are handled cleanly.
        3_822, // One byte over 8 shares. The last share has 1 byte data, 481 bytes padding.
        3_823, // Fills 16 shares (1×478 + 15×482). Full subtree size.
        7_700,
        // Overflows into the 17th share. Tests last-share padding and correct subtree construction.
        7_701,
        // Exactly fills 33 shares. This size forces share alignment beyond 32-share grouping.
        15_934,
        // Crosses into the new row in 128×128 data square. Tests square alignment behavior.
        15_935,
    ];

    // Something that sequencer normally will try to send
    let large_blobs = 1_048_500..1_048_600;

    let mut important_sizes = vec![1];
    important_sizes.extend(around_1_share);
    important_sizes.extend(around_2_shares);
    important_sizes.extend(around_3_shares);
    important_sizes.extend(boundary_crossing);
    important_sizes.extend(power_of_two_sizes);
    // We want to test large blobs more
    for _ in 0..4 {
        important_sizes.extend(large_blobs.clone());
    }
    tracing::info!(
        "Going to generate {} blobs. Going to submit then in {} rounds",
        important_sizes.len(),
        rounds
    );

    for i in 0..rounds {
        let mut blobs: Vec<Vec<u8>> = Vec::with_capacity(important_sizes.len());
        let mut rng = rand::thread_rng();

        for size in &important_sizes {
            let mut blob = vec![0u8; *size];
            rng.fill_bytes(&mut blob);
            blobs.push(blob);
        }
        let blobs_cumulative_size = blobs.iter().map(|b| b.len()).sum::<usize>();
        tracing::info!(
            "Starting round {}. Going to submit {} blobs of total size {} bytes",
            i,
            blobs.len(),
            blobs_cumulative_size
        );

        check_blobs_roundtrip(da_service, &blobs).await?;
        tracing::info!("Round {} complete", i + 1);
    }

    tracing::info!("All rounds have been done!");
    Ok(())
}
