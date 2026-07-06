use actr_protocol::ActrId;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub(crate) const DATA_CHUNK_ACTIVITY_TTL: Duration = Duration::from_secs(30);

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
        Self::new(DATA_CHUNK_ACTIVITY_TTL)
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
#[path = "data_stream_activity_tests.rs"]
mod tests;
