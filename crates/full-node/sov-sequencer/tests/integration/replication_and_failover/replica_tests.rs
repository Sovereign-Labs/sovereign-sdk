use crate::replication_and_failover::create_test_rollups;
use crate::replication_and_failover::query_value;
use crate::replication_and_failover::send_set_value_tx;
use crate::replication_and_failover::wait_for_height;

#[tokio::test(flavor = "multi_thread")]
async fn seq_with_replicas_basic_flow() {
    //sov_test_utils::initialize_logging();
    let (test_rollups, _tempdir, admin) = create_test_rollups(2).await;
    let Some(mut test_rollups) = test_rollups else {
        return;
    };

    let master = test_rollups.remove(0);
    let mut replica_1 = test_rollups.remove(0);

    let mut next_generation = 0;
    let value_to_set = 99;
    let da_service = master.da_service.clone();

    // User sends a tx to the master sequencer.
    {
        let api_client = master.api_client();
        let node_client = &master.client;

        wait_for_height(node_client, &da_service, 10).await;

        send_set_value_tx(
            api_client,
            &admin.private_key,
            next_generation,
            value_to_set,
        )
        .await
        .unwrap();

        next_generation += 1;

        let resp_value = query_value(node_client).await;
        assert_eq!(resp_value, value_to_set as u32);
    }

    // Replicas flow.
    {
        let api_client = replica_1.api_client();
        let node_client = &replica_1.client;

        wait_for_height(node_client, &da_service, 20).await;

        // Replicas cannot accept txs.
        let res = send_set_value_tx(
            api_client,
            &admin.private_key,
            next_generation,
            value_to_set,
        )
        .await
        .unwrap_err();

        let err: String = res.to_string();
        assert!(err.contains("Sequencer is replica and cannot accept transactions"));

        // Replicas see the correct value.
        let resp_value = query_value(&replica_1.client).await;
        assert_eq!(resp_value, value_to_set as u32);

        // After retrying, we still can query replica.
        replica_1 = replica_1.restart().await.unwrap();

        let resp_value = query_value(&replica_1.client).await;
        assert_eq!(resp_value, value_to_set as u32);
    }

    master.shutdown().await.unwrap();
    replica_1.shutdown().await.unwrap();
}
