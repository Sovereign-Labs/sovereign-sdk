use sov_aggregated_proof::check_receipts;

fn main() {
    check_receipts(vec![
        "data/inner_0.proof",
        "data/inner_1.proof",
        "data/inner_2.proof",
    ])
    .unwrap();
}
