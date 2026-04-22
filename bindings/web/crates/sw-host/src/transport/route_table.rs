//! RouteTable - route registry.
//!
//! Manages routing mappings from `ActrId` and `ActrType` to `Dest`.
//!
//! # Features
//! - Destination registration and lookup
//! - Multi-route support for load balancing
//! - Route priorities

use actr_protocol::{ActrId, ActrType};
use actr_web_common::Dest;
use parking_lot::RwLock;
use std::collections::HashMap;

/// Route entry.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    /// Destination node.
    pub dest: Dest,

    /// Priority, where smaller values are preferred.
    pub priority: u32,

    /// Weight used for load balancing.
    pub weight: u32,

    /// Whether the route is currently available.
    pub available: bool,
}

impl RouteEntry {
    /// Create a new route entry.
    pub fn new(dest: Dest) -> Self {
        Self {
            dest,
            priority: 100,
            weight: 100,
            available: true,
        }
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Set the weight.
    pub fn with_weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }
}

/// Route table.
///
/// Manages mappings from `ActrId` and `ActrType` to `Dest`.
pub struct RouteTable {
    /// `ActrId -> route entries`.
    id_routes: RwLock<HashMap<ActrId, Vec<RouteEntry>>>,

    /// `ActrType -> route entries`, used for service discovery.
    type_routes: RwLock<HashMap<ActrType, Vec<RouteEntry>>>,
}

impl RouteTable {
    /// Create a new route table.
    pub fn new() -> Self {
        Self {
            id_routes: RwLock::new(HashMap::new()),
            type_routes: RwLock::new(HashMap::new()),
        }
    }

    // ========== ActrId Routes ==========

    /// Register a route for an ActrId.
    pub fn register_id(&self, actor_id: ActrId, entry: RouteEntry) {
        let mut routes = self.id_routes.write();
        routes.entry(actor_id.clone()).or_default().push(entry);

        // Sort by priority.
        if let Some(entries) = routes.get_mut(&actor_id) {
            entries.sort_by_key(|e| e.priority);
        }

        log::debug!("[RouteTable] Registered route for ActrId: {:?}", actor_id);
    }

    /// Unregister all routes for an ActrId.
    pub fn unregister_id(&self, actor_id: &ActrId) {
        let mut routes = self.id_routes.write();
        routes.remove(actor_id);
        log::debug!(
            "[RouteTable] Unregistered all routes for ActrId: {:?}",
            actor_id
        );
    }

    /// Look up the best route for an ActrId.
    pub fn lookup_id(&self, actor_id: &ActrId) -> Option<Dest> {
        let routes = self.id_routes.read();
        routes.get(actor_id).and_then(|entries| {
            // Return the first available route.
            entries.iter().find(|e| e.available).map(|e| e.dest.clone())
        })
    }

    /// Look up all available routes for an ActrId.
    pub fn lookup_id_all(&self, actor_id: &ActrId) -> Vec<Dest> {
        let routes = self.id_routes.read();
        routes
            .get(actor_id)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.available)
                    .map(|e| e.dest.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    // ========== ActrType Routes ==========

    /// Register a route for an ActrType, typically for service discovery.
    pub fn register_type(&self, actr_type: ActrType, entry: RouteEntry) {
        let mut routes = self.type_routes.write();
        routes.entry(actr_type.clone()).or_default().push(entry);

        // Sort by priority.
        if let Some(entries) = routes.get_mut(&actr_type) {
            entries.sort_by_key(|e| e.priority);
        }

        log::debug!(
            "[RouteTable] Registered route for ActrType: {:?}",
            actr_type
        );
    }

    /// Look up the best route for an ActrType.
    pub fn lookup_type(&self, actr_type: &ActrType) -> Option<Dest> {
        let routes = self.type_routes.read();
        routes
            .get(actr_type)
            .and_then(|entries| entries.iter().find(|e| e.available).map(|e| e.dest.clone()))
    }

    /// Look up all available routes for an ActrType for load balancing.
    pub fn lookup_type_all(&self, actr_type: &ActrType) -> Vec<Dest> {
        let routes = self.type_routes.read();
        routes
            .get(actr_type)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.available)
                    .map(|e| e.dest.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    // ========== Route State Management ==========

    /// Mark a route as unavailable.
    pub fn mark_unavailable(&self, dest: &Dest) {
        // Update id_routes.
        {
            let mut routes = self.id_routes.write();
            for entries in routes.values_mut() {
                for entry in entries.iter_mut() {
                    if &entry.dest == dest {
                        entry.available = false;
                    }
                }
            }
        }

        // Update type_routes.
        {
            let mut routes = self.type_routes.write();
            for entries in routes.values_mut() {
                for entry in entries.iter_mut() {
                    if &entry.dest == dest {
                        entry.available = false;
                    }
                }
            }
        }

        log::info!("[RouteTable] Marked dest as unavailable: {:?}", dest);
    }

    /// Mark a route as available.
    pub fn mark_available(&self, dest: &Dest) {
        // Update id_routes.
        {
            let mut routes = self.id_routes.write();
            for entries in routes.values_mut() {
                for entry in entries.iter_mut() {
                    if &entry.dest == dest {
                        entry.available = true;
                    }
                }
            }
        }

        // Update type_routes.
        {
            let mut routes = self.type_routes.write();
            for entries in routes.values_mut() {
                for entry in entries.iter_mut() {
                    if &entry.dest == dest {
                        entry.available = true;
                    }
                }
            }
        }

        log::info!("[RouteTable] Marked dest as available: {:?}", dest);
    }

    /// Return routing statistics.
    pub fn stats(&self) -> RouteTableStats {
        let id_routes = self.id_routes.read();
        let type_routes = self.type_routes.read();

        RouteTableStats {
            id_route_count: id_routes.len(),
            type_route_count: type_routes.len(),
            total_entries: id_routes.values().map(|v| v.len()).sum::<usize>()
                + type_routes.values().map(|v| v.len()).sum::<usize>(),
        }
    }
}

impl Default for RouteTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Route-table statistics.
#[derive(Debug, Clone)]
pub struct RouteTableStats {
    /// Number of `ActrId` routes.
    pub id_route_count: usize,

    /// Number of `ActrType` routes.
    pub type_route_count: usize,

    /// Total number of entries.
    pub total_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_table_creation() {
        let table = RouteTable::new();
        let stats = table.stats();
        assert_eq!(stats.id_route_count, 0);
        assert_eq!(stats.type_route_count, 0);
    }

    #[test]
    fn test_route_registration_and_lookup() {
        let table = RouteTable::new();

        let actor_id = ActrId {
            realm: actr_protocol::Realm { realm_id: 1 },
            serial_number: 123,
            r#type: actr_protocol::ActrType {
                manufacturer: "test".to_string(),
                name: "node".to_string(),
                version: "1.0.0".to_string(),
            },
        };

        let dest = Dest::Peer("node1".to_string());

        table.register_id(actor_id.clone(), RouteEntry::new(dest.clone()));

        let result = table.lookup_id(&actor_id);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), dest);
    }
}
