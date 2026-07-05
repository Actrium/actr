//! RPC correlation - matches in-flight request_ids with waiting callers
//!
//! Shared building block for every pending-request map in the crate:
//! `HostTransport` (intra-process, no per-entry metadata) and the
//! out-of-process family (`PeerGate` / wire gates, keyed with the target
//! `ActrId` so entries can be swept when a peer disconnects).
//!
//! Registration returns an RAII [`PendingRpc`] guard: dropping the guard
//! removes the map entry, so every early-return path of a caller (send
//! failure, timeout, cancellation) cleans up without manual bookkeeping.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actr_framework::Bytes;
use actr_protocol::{ActorResult, ActrError, ErrorResponse, RpcEnvelope};
use tokio::sync::oneshot;

type PendingMap<M> = Arc<Mutex<HashMap<String, Entry<M>>>>;

/// Correlates in-flight request_ids with waiting callers.
///
/// INVARIANT: the internal `std::sync::Mutex` is only held for synchronous
/// map operations (insert/remove/len) and `oneshot::Sender::send` (which is
/// sync); it must never be held across an `.await`. Keep the mutex private
/// so all access is funneled through this module.
pub(crate) struct RpcCorrelation<M> {
    inner: PendingMap<M>,
}

struct Entry<M> {
    meta: M,
    tx: oneshot::Sender<ActorResult<Bytes>>,
}

/// Result of attempting to complete a pending request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CompleteOutcome {
    /// A waiting caller was found and woken (it may have gone away, in
    /// which case the oneshot send fails silently - same as before).
    Completed,
    /// No pending entry for this request_id (late reply after timeout,
    /// duplicate response, or an id that was never registered here).
    Orphan,
}

impl<M> Default for RpcCorrelation<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> RpcCorrelation<M> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Insert a pending entry; the returned guard removes it on Drop unless
    /// a completer consumed it first.
    ///
    /// A duplicate request_id overwrites the previous entry (the old caller
    /// observes "Response channel closed") - request ids are uuid v4, so a
    /// collision indicates a caller bug worth surfacing in the logs.
    pub(crate) fn register(&self, request_id: impl Into<String>, meta: M) -> PendingRpc<M> {
        let request_id = request_id.into();
        let (tx, rx) = oneshot::channel();
        let previous = lock_ignore_poison(&self.inner)
            .insert(request_id.clone(), Entry { meta, tx })
            .is_some();
        if previous {
            tracing::warn!(
                request_id = %request_id,
                "rpc.pending_overwritten: duplicate request_id registered; previous caller will see a closed response channel"
            );
        }
        PendingRpc {
            map: self.inner.clone(),
            request_id,
            rx: Some(rx),
        }
    }

    /// Wake the waiting caller with `result`. Returns `Orphan` when no entry
    /// exists; the caller decides how to log that (or uses
    /// [`complete_from_envelope`](Self::complete_from_envelope) for the
    /// standard inbound-response policy).
    pub(crate) fn complete(&self, request_id: &str, result: ActorResult<Bytes>) -> CompleteOutcome {
        match lock_ignore_poison(&self.inner).remove(request_id) {
            Some(entry) => {
                let _ = entry.tx.send(result);
                CompleteOutcome::Completed
            }
            None => CompleteOutcome::Orphan,
        }
    }

    /// Standard inbound-response path shared by all response readers:
    /// converts the envelope's `(payload, error)` pair via
    /// [`envelope_response_to_result`], then completes. On `Orphan` emits
    /// the crate-standard warn (`rpc.orphan_response_dropped`) with
    /// request_id / peer / route_key; `peer` is only evaluated on that
    /// drop path.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn complete_from_envelope(
        &self,
        envelope: RpcEnvelope,
        peer: impl FnOnce() -> String,
    ) -> CompleteOutcome {
        let RpcEnvelope {
            request_id,
            route_key,
            payload,
            error,
            ..
        } = envelope;
        let result = envelope_response_to_result(payload, error);
        let outcome = self.complete(&request_id, result);
        if outcome == CompleteOutcome::Orphan {
            tracing::warn!(
                request_id = %request_id,
                peer = %peer(),
                route_key = %route_key,
                "rpc.orphan_response_dropped: envelope marked Response has no pending request; dropping (late reply or peer-mislabeled request)"
            );
        }
        outcome
    }

    /// Remove-and-fail every entry whose metadata matches the predicate
    /// (peer-disconnect sweep). Returns the number of entries removed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn fail_where(
        &self,
        mut pred: impl FnMut(&M) -> bool,
        err: impl Fn() -> ActrError,
    ) -> usize {
        let mut map = lock_ignore_poison(&self.inner);
        let matching: Vec<String> = map
            .iter()
            .filter(|(_, entry)| pred(&entry.meta))
            .map(|(id, _)| id.clone())
            .collect();
        for request_id in &matching {
            if let Some(entry) = map.remove(request_id) {
                let _ = entry.tx.send(Err(err()));
            }
        }
        matching.len()
    }

    /// Number of in-flight entries.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn len(&self) -> usize {
        lock_ignore_poison(&self.inner).len()
    }
}

/// RAII guard for one pending request.
///
/// Dropping the guard removes the map entry, so callers that bail out
/// before (or while) waiting never leak an entry. Completion via
/// [`RpcCorrelation::complete`] removes the entry first; the guard's later
/// Drop is then a no-op.
pub(crate) struct PendingRpc<M> {
    map: PendingMap<M>,
    request_id: String,
    rx: Option<oneshot::Receiver<ActorResult<Bytes>>>,
}

impl<M> PendingRpc<M> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Wait for the response with a deadline. Timeout expiry yields
    /// `Err(ActrError::TimedOut)`; a dropped sender (entry overwritten or
    /// swept without a result) yields `Err(Unavailable("Response channel
    /// closed"))`. The entry is removed on every path (by the completer or
    /// by this guard's Drop).
    pub(crate) async fn wait(mut self, timeout: Duration) -> ActorResult<Bytes> {
        let rx = self
            .rx
            .take()
            .expect("PendingRpc receiver is consumed exactly once");
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ActrError::Unavailable("Response channel closed".into())),
            Err(_) => Err(ActrError::TimedOut),
        }
    }

    /// Wait for the response without an internal deadline - for callers
    /// that own a combined send+wait budget (e.g. `PeerGate` wraps this in
    /// its own `tokio::time::timeout` so it can close the transport on
    /// expiry).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn recv(mut self) -> ActorResult<Bytes> {
        let rx = self
            .rx
            .take()
            .expect("PendingRpc receiver is consumed exactly once");
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(ActrError::Unavailable("Response channel closed".into())),
        }
    }

    /// Detach the receiver from the guard: the entry then stays in the map
    /// until completed or swept. Test-only - in production this reintroduces
    /// the leak the guard exists to prevent.
    #[cfg(feature = "test-utils")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn into_receiver(mut self) -> oneshot::Receiver<ActorResult<Bytes>> {
        let rx = self
            .rx
            .take()
            .expect("PendingRpc receiver is consumed exactly once");
        // Forget the guard so its Drop does not remove the live entry.
        std::mem::forget(self);
        rx
    }
}

impl<M> Drop for PendingRpc<M> {
    fn drop(&mut self) {
        lock_ignore_poison(&self.map).remove(&self.request_id);
    }
}

/// Centralized `(payload, error)` -> `ActorResult` conversion for inbound
/// response envelopes, reconstructing the precise `ActrError` variant from
/// the wire code.
pub(crate) fn envelope_response_to_result(
    payload: Option<Bytes>,
    error: Option<ErrorResponse>,
) -> ActorResult<Bytes> {
    match (payload, error) {
        (Some(payload), None) => Ok(payload),
        (None, Some(error)) => Err(crate::lifecycle::node::wire_code_to_actr_error(
            error.code,
            error.message,
        )),
        _ => Err(ActrError::DecodeFailure(
            "Invalid RpcEnvelope: payload and error fields inconsistent".to_string(),
        )),
    }
}

/// The maps guarded here hold plain sender handles; a panic while holding
/// the lock cannot leave them logically inconsistent, so recover the guard
/// instead of propagating the poison (mandatory in `Drop`, where a second
/// panic would abort).
fn lock_ignore_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn guard_drop_removes_entry() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let pending = correlation.register("req-1", ());
        assert_eq!(pending.request_id(), "req-1");
        assert_eq!(correlation.len(), 1);
        drop(pending);
        assert_eq!(correlation.len(), 0);
        assert_eq!(
            correlation.complete("req-1", Ok(Bytes::from_static(b"late"))),
            CompleteOutcome::Orphan
        );
    }

    #[tokio::test]
    async fn complete_before_drop_wins() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let pending = correlation.register("req-1", ());
        assert_eq!(
            correlation.complete("req-1", Ok(Bytes::from_static(b"resp"))),
            CompleteOutcome::Completed
        );
        assert_eq!(correlation.len(), 0);
        let result = pending.wait(Duration::from_secs(1)).await;
        assert_eq!(result.unwrap(), Bytes::from_static(b"resp"));
        assert_eq!(correlation.len(), 0);
    }

    #[tokio::test]
    async fn wait_timeout_returns_timed_out_and_empties_map() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let pending = correlation.register("req-1", ());
        let err = pending.wait(Duration::from_millis(10)).await.unwrap_err();
        assert!(matches!(err, ActrError::TimedOut), "got {err:?}");
        assert_eq!(correlation.len(), 0);
    }

    #[tokio::test]
    async fn recv_resolves_on_complete() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let pending = correlation.register("req-1", ());
        assert_eq!(
            correlation.complete("req-1", Err(ActrError::NotFound("x".into()))),
            CompleteOutcome::Completed
        );
        let err = pending.recv().await.unwrap_err();
        assert!(matches!(err, ActrError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn orphan_complete_returns_orphan() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        assert_eq!(
            correlation.complete("never-registered", Ok(Bytes::new())),
            CompleteOutcome::Orphan
        );
    }

    #[tokio::test]
    async fn duplicate_register_overwrites_previous_entry() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let first = correlation.register("req-1", ());
        let second = correlation.register("req-1", ());
        assert_eq!(correlation.len(), 1);

        // The first caller's sender was dropped by the overwrite.
        let err = first.recv().await.unwrap_err();
        assert!(matches!(err, ActrError::Unavailable(_)), "got {err:?}");
        // The first guard's Drop already ran (recv consumed it) and removed
        // the entry by id; the overwrite-then-stale-drop race is accepted
        // for uuid v4 ids and observable via the overwrite warn.
        assert_eq!(correlation.len(), 0);
        drop(second);
    }

    #[tokio::test]
    async fn fail_where_removes_only_matching_metas() {
        let correlation: RpcCorrelation<u8> = RpcCorrelation::new();
        let pending_a = correlation.register("req-a", 1);
        let pending_b = correlation.register("req-b", 2);
        let pending_c = correlation.register("req-c", 1);
        assert_eq!(correlation.len(), 3);

        let removed = correlation.fail_where(
            |meta| *meta == 1,
            || ActrError::Unavailable("Connection closed".into()),
        );
        assert_eq!(removed, 2);
        assert_eq!(correlation.len(), 1);

        let err = pending_a.recv().await.unwrap_err();
        assert!(matches!(err, ActrError::Unavailable(_)), "got {err:?}");
        let err = pending_c.recv().await.unwrap_err();
        assert!(matches!(err, ActrError::Unavailable(_)), "got {err:?}");

        // Non-matching entry still completes normally.
        assert_eq!(
            correlation.complete("req-b", Ok(Bytes::from_static(b"ok"))),
            CompleteOutcome::Completed
        );
        assert_eq!(pending_b.recv().await.unwrap(), Bytes::from_static(b"ok"));
    }

    #[tokio::test]
    async fn complete_from_envelope_resolves_pending() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let pending = correlation.register("req-1", ());
        let envelope = RpcEnvelope {
            request_id: "req-1".to_string(),
            route_key: "echo".to_string(),
            payload: Some(Bytes::from_static(b"resp")),
            ..Default::default()
        };
        let outcome = correlation.complete_from_envelope(envelope, || {
            panic!("peer must not be computed on the completed path")
        });
        assert_eq!(outcome, CompleteOutcome::Completed);
        let result = pending.wait(Duration::from_secs(1)).await;
        assert_eq!(result.unwrap(), Bytes::from_static(b"resp"));
    }

    #[tokio::test]
    async fn complete_from_envelope_orphan_computes_peer() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let envelope = RpcEnvelope {
            request_id: "req-unknown".to_string(),
            route_key: "echo".to_string(),
            payload: Some(Bytes::from_static(b"resp")),
            ..Default::default()
        };
        let outcome = correlation.complete_from_envelope(envelope, || "peer-1".to_string());
        assert_eq!(outcome, CompleteOutcome::Orphan);
    }

    #[cfg(feature = "test-utils")]
    #[tokio::test]
    async fn into_receiver_detaches_guard_and_keeps_entry() {
        let correlation: RpcCorrelation<()> = RpcCorrelation::new();
        let rx = correlation.register("req-1", ()).into_receiver();
        // Guard consumed, but the entry must stay until completed/swept.
        assert_eq!(correlation.len(), 1);
        assert_eq!(
            correlation.complete("req-1", Ok(Bytes::from_static(b"resp"))),
            CompleteOutcome::Completed
        );
        assert_eq!(rx.await.unwrap().unwrap(), Bytes::from_static(b"resp"));
        assert_eq!(correlation.len(), 0);
    }

    #[test]
    fn envelope_response_to_result_maps_all_cases() {
        // Success payload.
        let ok = envelope_response_to_result(Some(Bytes::from_static(b"ok")), None);
        assert_eq!(ok.unwrap(), Bytes::from_static(b"ok"));

        // Wire error code reconstructs the precise variant (10002 = TimedOut).
        let err = envelope_response_to_result(
            None,
            Some(ErrorResponse {
                code: 10002,
                message: "deadline".to_string(),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, ActrError::TimedOut), "got {err:?}");

        // Both present and both absent are inconsistent envelopes.
        let err = envelope_response_to_result(
            Some(Bytes::from_static(b"x")),
            Some(ErrorResponse {
                code: 10001,
                message: "boom".to_string(),
            }),
        )
        .unwrap_err();
        assert!(matches!(err, ActrError::DecodeFailure(_)), "got {err:?}");
        let err = envelope_response_to_result(None, None).unwrap_err();
        assert!(matches!(err, ActrError::DecodeFailure(_)), "got {err:?}");
    }
}
