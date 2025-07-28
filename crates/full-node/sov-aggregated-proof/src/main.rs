use sov_aggregated_proof::check_receipts;

fn main() {
    check_receipts(vec![
        "data/inner_0.proof",
        "data/inner_1.proof",
        "data/inner_2.proof",
        "data/inner_3.proof",
        "data/inner_4.proof",
        "data/inner_5.proof",
    ])
    .unwrap();
}
