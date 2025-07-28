use rest_api_load_testing::Requests;

#[tokio::main]
async fn main() {
    let requests = Requests::new(
        "http://localhost:12346",
        vec!["ledger/slots/latest".to_string()],
    );
    let summary = rest_api_load_testing::start(requests).await;
    summary.print_summary();
}
