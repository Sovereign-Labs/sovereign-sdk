use futures::StreamExt;
use futures::TryStreamExt;
use sov_node_client::NodeClient;

fn master_client() -> NodeClient {
    let rest_url = format!("http://{}:{}", "127.0.0.1", "12346");
    NodeClient::new_unchecked(&rest_url)
}

fn replica_client() -> NodeClient {
    let rest_url = format!("http://{}:{}", "127.0.0.1", "12349");
    NodeClient::new_unchecked(&rest_url)
}

#[tokio::main]
async fn main() {
    let master = master_client();
    let replica = replica_client();

    let mut master_sub = master
        .client
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    let mut replica_sub = replica
        .client
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    println!("START");

    loop {
        let master_event = master_sub.next().await.unwrap();
        let replica_event = replica_sub.try_next().await.unwrap();

        println!("");
        println!("MASTER master_event: {:?}", master_event);
        println!("Replica event: {:?}", replica_event);
    }
}
