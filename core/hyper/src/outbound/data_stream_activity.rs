use actr_protocol::ActrId;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub(crate) const DATA_STREAM_ACTIVITY_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataStreamDeliveryUncertainNotice {
    pub(crate) peer_id: ActrId,
    pub(crate) stream_id: String,
    pub(crate) last_sent_seq: u64,
    pub(crate) session_id: u64,
    pub(crate) reason: String,
}

#[derive(Debug, Clone)]
struct ActiveDataStream {
    last_sent_seq: u64,
    last_updated_at: Instant,
    notified_sessions: HashSet<u64>,
}

#[derive(Debug)]
pub(crate) struct DataStreamActivityTracker {
    ttl: Duration,
    streams_by_peer: HashMap<ActrId, HashMap<String, ActiveDataStream>>,
}

impl Default for DataStreamActivityTracker {
    fn default() -> Self {
        Self::new(DATA_STREAM_ACTIVITY_TTL)
    }
}

impl DataStreamActivityTracker {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            streams_by_peer: HashMap::new(),
        }
    }

    pub(crate) fn record_sent(
        &mut self,
        peer_id: &ActrId,
        stream_id: impl Into<String>,
        sequence: u64,
        now: Instant,
    ) {
        let stream_id = stream_id.into();
        let stream = self
            .streams_by_peer
            .entry(peer_id.clone())
            .or_default()
            .entry(stream_id)
            .or_insert_with(|| ActiveDataStream {
                last_sent_seq: sequence,
                last_updated_at: now,
                notified_sessions: HashSet::new(),
            });

        stream.last_sent_seq = sequence;
        stream.last_updated_at = now;
    }

    pub(crate) fn mark_delivery_uncertain(
        &mut self,
        peer_id: &ActrId,
        session_id: u64,
        reason: impl Into<String>,
        now: Instant,
    ) -> Vec<DataStreamDeliveryUncertainNotice> {
        self.prune_expired(now);

        let reason = reason.into();
        let Some(streams) = self.streams_by_peer.get_mut(peer_id) else {
            return Vec::new();
        };

        streams
            .iter_mut()
            .filter_map(|(stream_id, stream)| {
                if !stream.notified_sessions.insert(session_id) {
                    return None;
                }

                Some(DataStreamDeliveryUncertainNotice {
                    peer_id: peer_id.clone(),
                    stream_id: stream_id.clone(),
                    last_sent_seq: stream.last_sent_seq,
                    session_id,
                    reason: reason.clone(),
                })
            })
            .collect()
    }

    pub(crate) fn remove_stream(&mut self, peer_id: &ActrId, stream_id: &str) {
        let should_remove_peer = if let Some(streams) = self.streams_by_peer.get_mut(peer_id) {
            streams.remove(stream_id);
            streams.is_empty()
        } else {
            false
        };

        if should_remove_peer {
            self.streams_by_peer.remove(peer_id);
        }
    }

    fn prune_expired(&mut self, now: Instant) {
        let ttl = self.ttl;
        self.streams_by_peer.retain(|_, streams| {
            streams.retain(|_, stream| now.duration_since(stream.last_updated_at) <= ttl);
            !streams.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::{ActrType, Realm};

    fn actr_id(serial_number: u64) -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "node".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    #[test]
    fn records_last_sent_sequence_per_stream() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_sent(&peer, "stream-a", 1, now);
        tracker.record_sent(&peer, "stream-a", 7, now + Duration::from_secs(1));

        let notices = tracker.mark_delivery_uncertain(
            &peer,
            42,
            "data channel closed",
            now + Duration::from_secs(2),
        );

        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].stream_id, "stream-a");
        assert_eq!(notices[0].last_sent_seq, 7);
        assert_eq!(notices[0].session_id, 42);
    }

    #[test]
    fn tracks_multiple_streams_for_same_peer() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_sent(&peer, "stream-a", 3, now);
        tracker.record_sent(&peer, "stream-b", 9, now);

        let mut notices = tracker.mark_delivery_uncertain(&peer, 42, "webrtc disconnected", now);
        notices.sort_by(|a, b| a.stream_id.cmp(&b.stream_id));

        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].stream_id, "stream-a");
        assert_eq!(notices[0].last_sent_seq, 3);
        assert_eq!(notices[1].stream_id, "stream-b");
        assert_eq!(notices[1].last_sent_seq, 9);
    }

    #[test]
    fn expires_streams_outside_ttl() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(5));

        tracker.record_sent(&peer, "stream-a", 1, now);

        let notices =
            tracker.mark_delivery_uncertain(&peer, 42, "late close", now + Duration::from_secs(6));

        assert!(notices.is_empty());
    }

    #[test]
    fn deduplicates_by_peer_stream_and_session() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_sent(&peer, "stream-a", 1, now);
        let first = tracker.mark_delivery_uncertain(&peer, 42, "state disconnected", now);
        let duplicate = tracker.mark_delivery_uncertain(&peer, 42, "data channel closed", now);
        let next_session = tracker.mark_delivery_uncertain(&peer, 43, "new session failed", now);

        assert_eq!(first.len(), 1);
        assert!(duplicate.is_empty());
        assert_eq!(next_session.len(), 1);
        assert_eq!(next_session[0].session_id, 43);
    }

    #[test]
    fn remove_stream_drops_failed_inflight_marker() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_sent(&peer, "stream-a", 1, now);
        tracker.remove_stream(&peer, "stream-a");

        let notices = tracker.mark_delivery_uncertain(&peer, 42, "late close", now);
        assert!(notices.is_empty());
    }
}
