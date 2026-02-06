use super::*;

fn test_timestamp() -> OffsetDateTime {
    OffsetDateTime::UNIX_EPOCH
}

const TEST_TIME_STAMP: &str = "1970-01-01T00:00:00Z";

#[test]
fn cluster_with_leader_and_followers() {
    let ts = test_timestamp();
    let info = NodeDiscovery::cluster(
        Some("node1".to_string()),
        vec![
            ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
            ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
            ("node3".to_string(), "127.0.0.1:8002".to_string(), ts),
        ],
    )
    .unwrap();

    assert_eq!(
        info.to_file_content(),
        format!(
            "leader=127.0.0.1:8000,{TEST_TIME_STAMP},node1\n\
             follower=127.0.0.1:8001,{TEST_TIME_STAMP},node2\n\
             follower=127.0.0.1:8002,{TEST_TIME_STAMP},node3"
        ),
    );
}

#[test]
fn cluster_with_only_leader_adds_leader_to_followers() {
    let ts = test_timestamp();
    let info = NodeDiscovery::cluster(
        Some("node1".to_string()),
        vec![("node1".to_string(), "127.0.0.1:8000".to_string(), ts)],
    )
    .unwrap();

    assert_eq!(
        info.to_file_content(),
        format!(
            "leader=127.0.0.1:8000,{TEST_TIME_STAMP},node1\n\
             follower=127.0.0.1:8000,{TEST_TIME_STAMP},node1"
        ),
    );
}

#[test]
fn cluster_with_no_leader() {
    let ts = test_timestamp();
    let info = NodeDiscovery::cluster(
        None,
        vec![
            ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
            ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
        ],
    )
    .unwrap();

    assert_eq!(
        info.to_file_content(),
        format!(
            "follower=127.0.0.1:8000,{TEST_TIME_STAMP},node1\n\
             follower=127.0.0.1:8001,{TEST_TIME_STAMP},node2"
        ),
    );
}

#[test]
fn cluster_empty() {
    let info = NodeDiscovery::cluster(None, vec![]).unwrap();
    assert_eq!(info.to_file_content(), "");
}

#[test]
fn cluster_errors_on_duplicate_node_id() {
    let ts = test_timestamp();
    let result = NodeDiscovery::cluster(
        None,
        vec![
            ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
            ("node1".to_string(), "127.0.0.1:8001".to_string(), ts),
        ],
    );

    let err = result.unwrap_err();
    assert!(err
        .to_string()
        .contains("Duplicate node id found in Nodes table: node1"));
}

#[test]
fn cluster_errors_when_leader_not_in_nodes() {
    let ts = test_timestamp();
    let result = NodeDiscovery::cluster(
        Some("missing_leader".to_string()),
        vec![
            ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
            ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
        ],
    );

    let err = result.unwrap_err();
    assert!(err
        .to_string()
        .contains("Leader is missing from the Nodes table."));
}

#[test]
fn leader_change_detected_when_membership_unchanged() {
    let ts = test_timestamp();
    let nodes = vec![
        ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
        ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
    ];

    // Initial state: node1 is leader
    let info1 = NodeDiscovery::cluster(Some("node1".to_string()), nodes.clone()).unwrap();

    // New state: node2 becomes leader, same membership
    let info2 = NodeDiscovery::cluster(Some("node2".to_string()), nodes).unwrap();

    // Membership should be identical
    assert_eq!(info1.members, info2.members, "Members should be unchanged");

    // But leader_id should differ.
    assert_eq!(info1.leader_id(), Some("node1".to_string()));
    assert_eq!(info2.leader_id(), Some("node2".to_string()));
}
