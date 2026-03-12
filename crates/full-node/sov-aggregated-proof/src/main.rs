use sov_aggregated_proof::check_receipts;

fn main() {
    check_receipts(
        vec![
            "data/inner_0_proof.json",
            //   "data/inner_1_proof.json",
            //   "data/inner_2_proof.json",
        ],
        "data/inner_vkey.bin",
    )
    .unwrap();
}
