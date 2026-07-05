use super::*;

struct TestFactory;

#[async_trait]
impl WireBuilder for TestFactory {
    async fn create_connections(&self, _dest: &Dest) -> NetworkResult<Vec<Arc<dyn WireHandle>>> {
        // Test factory: returns empty list (real usage requires actual connections)
        Ok(vec![])
    }
}

fn create_test_factory() -> Arc<dyn WireBuilder> {
    Arc::new(TestFactory)
}

#[tokio::test]
async fn test_transport_manager_creation() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id.clone(), factory);

    assert_eq!(mgr.dest_count().await, 0);
    assert_eq!(mgr.local_id(), &local_id);
}

#[tokio::test]
async fn test_list_dests() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id, factory);

    let dests = mgr.list_dests().await;
    assert_eq!(dests.len(), 0);
}

#[tokio::test]
async fn test_has_dest() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id, factory);

    let dest = Dest::host();
    assert!(!mgr.has_dest(&dest).await);
}

#[tokio::test]
async fn close_transport_if_current_replaced_instance_does_not_mark_closing() {
    let local_id = ActrId::default();
    let factory = create_test_factory();
    let mgr = PeerTransport::new(local_id, factory);
    let dest = Dest::host();

    let old_transport = Arc::new(
        DestTransport::new(dest.clone(), vec![])
            .await
            .expect("old transport should be created"),
    );
    let current_transport = Arc::new(
        DestTransport::new(dest.clone(), vec![])
            .await
            .expect("current transport should be created"),
    );
    let old_ref = DestTransportRef::new(&old_transport, None);

    mgr.transports
        .write()
        .await
        .insert(dest.clone(), Either::Right(current_transport));

    let closed = mgr
        .close_transport_if_current(&dest, &old_ref)
        .await
        .expect("stale instance close should not fail");

    assert!(!closed);
    assert_eq!(mgr.dest_count().await, 1);
    assert!(
        !mgr.is_closing(&dest).await,
        "stale no-op close must not mark the replacement transport closing"
    );
}
