use actr_protocol::ActrId;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) const DATA_STREAM_ACTIVITY_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DataStreamDeliveryUncertainNotice {
    pub(crate) stream_id: String,
    pub(crate) session_id: u64,
    pub(crate) reason: String,
}

#[derive(Debug, Clone)]
struct ActiveDataStream {
    last_updated_at: Instant,
    notified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataStreamRecordState {
    Missing,
    Stale,
    Fresh,
}

#[derive(Debug)]
pub(crate) struct DataStreamActivityTracker {
    ttl: Duration,
    streams_by_peer: HashMap<ActrId, HashMap<String, HashMap<u64, ActiveDataStream>>>,
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

    pub(crate) fn record_state(
        &self,
        peer_id: &ActrId,
        stream_id: &str,
        session_id: u64,
        now: Instant,
    ) -> DataStreamRecordState {
        let Some(stream) = self
            .streams_by_peer
            .get(peer_id)
            .and_then(|streams| streams.get(stream_id))
            .and_then(|sessions| sessions.get(&session_id))
        else {
            return DataStreamRecordState::Missing;
        };

        let refresh_interval = self.ttl.checked_div(2).unwrap_or(self.ttl);
        if now.duration_since(stream.last_updated_at) > refresh_interval {
            DataStreamRecordState::Stale
        } else {
            DataStreamRecordState::Fresh
        }
    }

    pub(crate) fn record_stream(
        &mut self,
        peer_id: &ActrId,
        stream_id: impl Into<String>,
        session_id: u64,
        now: Instant,
    ) {
        let stream_id = stream_id.into();
        let stream = self
            .streams_by_peer
            .entry(peer_id.clone())
            .or_default()
            .entry(stream_id)
            .or_default()
            .entry(session_id)
            .or_insert_with(|| ActiveDataStream {
                last_updated_at: now,
                notified: false,
            });

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
            .filter_map(|(stream_id, sessions)| {
                let stream = sessions.get_mut(&session_id)?;
                if stream.notified {
                    return None;
                }
                stream.notified = true;

                Some(DataStreamDeliveryUncertainNotice {
                    stream_id: stream_id.clone(),
                    session_id,
                    reason: reason.clone(),
                })
            })
            .collect()
    }

    #[cfg(test)]
    fn remove_stream(&mut self, peer_id: &ActrId, stream_id: &str) {
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

    pub(crate) fn remove_stream_session(
        &mut self,
        peer_id: &ActrId,
        stream_id: &str,
        session_id: u64,
    ) {
        let should_remove_peer = if let Some(streams) = self.streams_by_peer.get_mut(peer_id) {
            let should_remove_stream = if let Some(sessions) = streams.get_mut(stream_id) {
                sessions.remove(&session_id);
                sessions.is_empty()
            } else {
                false
            };

            if should_remove_stream {
                streams.remove(stream_id);
            }

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
            streams.retain(|_, sessions| {
                sessions.retain(|_, stream| now.duration_since(stream.last_updated_at) <= ttl);
                !sessions.is_empty()
            });
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
    fn records_stream_once_per_session() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        assert_eq!(
            tracker.record_state(&peer, "stream-a", 42, now),
            DataStreamRecordState::Missing
        );
        tracker.record_stream(&peer, "stream-a", 42, now);
        assert_eq!(
            tracker.record_state(&peer, "stream-a", 42, now + Duration::from_secs(1)),
            DataStreamRecordState::Fresh
        );
        assert_eq!(
            tracker.record_state(&peer, "stream-a", 42, now + Duration::from_secs(16)),
            DataStreamRecordState::Stale
        );

        let notices = tracker.mark_delivery_uncertain(
            &peer,
            42,
            "data channel closed",
            now + Duration::from_secs(2),
        );

        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].stream_id, "stream-a");
        assert_eq!(notices[0].session_id, 42);
    }

    #[test]
    fn tracks_multiple_streams_for_same_peer() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_stream(&peer, "stream-a", 42, now);
        tracker.record_stream(&peer, "stream-b", 42, now);

        let mut notices = tracker.mark_delivery_uncertain(&peer, 42, "webrtc disconnected", now);
        notices.sort_by(|a, b| a.stream_id.cmp(&b.stream_id));

        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].stream_id, "stream-a");
        assert_eq!(notices[1].stream_id, "stream-b");
    }

    #[test]
    fn expires_streams_outside_ttl() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(5));

        tracker.record_stream(&peer, "stream-a", 42, now);

        let notices =
            tracker.mark_delivery_uncertain(&peer, 42, "late close", now + Duration::from_secs(6));

        assert!(notices.is_empty());
    }

    #[test]
    fn filters_and_deduplicates_by_session() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_stream(&peer, "stream-a", 42, now);
        let first = tracker.mark_delivery_uncertain(&peer, 42, "state disconnected", now);
        let duplicate = tracker.mark_delivery_uncertain(&peer, 42, "data channel closed", now);
        let stale_session = tracker.mark_delivery_uncertain(&peer, 43, "stale close", now);

        tracker.record_stream(&peer, "stream-a", 43, now + Duration::from_secs(1));
        let next_session = tracker.mark_delivery_uncertain(
            &peer,
            43,
            "new session failed",
            now + Duration::from_secs(2),
        );

        assert_eq!(first.len(), 1);
        assert!(duplicate.is_empty());
        assert!(stale_session.is_empty());
        assert_eq!(next_session.len(), 1);
        assert_eq!(next_session[0].session_id, 43);
    }

    #[test]
    fn keeps_same_stream_records_for_overlapping_sessions() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_stream(&peer, "stream-a", 42, now);
        tracker.record_stream(&peer, "stream-a", 43, now + Duration::from_secs(1));

        let old_session = tracker.mark_delivery_uncertain(
            &peer,
            42,
            "old data channel closed late",
            now + Duration::from_secs(2),
        );
        let new_session = tracker.mark_delivery_uncertain(
            &peer,
            43,
            "new data channel closed",
            now + Duration::from_secs(3),
        );

        assert_eq!(old_session.len(), 1);
        assert_eq!(old_session[0].session_id, 42);
        assert_eq!(new_session.len(), 1);
        assert_eq!(new_session[0].session_id, 43);
    }

    #[test]
    fn remove_stream_drops_failed_inflight_marker() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_stream(&peer, "stream-a", 42, now);
        tracker.remove_stream(&peer, "stream-a");

        let notices = tracker.mark_delivery_uncertain(&peer, 42, "late close", now);
        assert!(notices.is_empty());
    }

    #[test]
    fn remove_stream_session_keeps_other_session_marker() {
        let peer = actr_id(100);
        let now = Instant::now();
        let mut tracker = DataStreamActivityTracker::new(Duration::from_secs(30));

        tracker.record_stream(&peer, "stream-a", 42, now);
        tracker.record_stream(&peer, "stream-a", 43, now + Duration::from_secs(1));
        tracker.remove_stream_session(&peer, "stream-a", 43);

        let old_session = tracker.mark_delivery_uncertain(
            &peer,
            42,
            "old data channel closed",
            now + Duration::from_secs(2),
        );
        let removed_session = tracker.mark_delivery_uncertain(
            &peer,
            43,
            "new send failed before reaching transport",
            now + Duration::from_secs(2),
        );

        assert_eq!(old_session.len(), 1);
        assert!(removed_session.is_empty());
    }
}
