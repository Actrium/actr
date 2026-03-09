//! RouteTable - 路由表
//!
//! 管理 ActrId → Dest 的路由映射
//!
//! # 功能
//! - 目的地注册和查询
//! - 多路由支持（负载均衡）
//! - 路由优先级

use actr_protocol::{ActrId, ActrType};
use actr_web_common::Dest;
use parking_lot::RwLock;
use std::collections::HashMap;

/// 路由条目
#[derive(Debug, Clone)]
pub struct RouteEntry {
    /// 目标节点
    pub dest: Dest,

    /// 优先级（越小越优先）
    pub priority: u32,

    /// 权重（用于负载均衡）
    pub weight: u32,

    /// 是否可用
    pub available: bool,
}

impl RouteEntry {
    /// 创建新的路由条目
    pub fn new(dest: Dest) -> Self {
        Self {
            dest,
            priority: 100,
            weight: 100,
            available: true,
        }
    }

    /// 设置优先级
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// 设置权重
    pub fn with_weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }
}

/// 路由表
///
/// 管理 ActrId 和 ActrType 到 Dest 的映射
pub struct RouteTable {
    /// ActrId → 路由条目列表
    id_routes: RwLock<HashMap<ActrId, Vec<RouteEntry>>>,

    /// ActrType → 路由条目列表（用于服务发现）
    type_routes: RwLock<HashMap<ActrType, Vec<RouteEntry>>>,
}

impl RouteTable {
    /// 创建新的路由表
    pub fn new() -> Self {
        Self {
            id_routes: RwLock::new(HashMap::new()),
            type_routes: RwLock::new(HashMap::new()),
        }
    }

    // ========== ActrId 路由 ==========

    /// 注册 ActrId 路由
    pub fn register_id(&self, actor_id: ActrId, entry: RouteEntry) {
        let mut routes = self.id_routes.write();
        routes.entry(actor_id.clone()).or_default().push(entry);

        // 按优先级排序
        if let Some(entries) = routes.get_mut(&actor_id) {
            entries.sort_by_key(|e| e.priority);
        }

        log::debug!("[RouteTable] Registered route for ActrId: {:?}", actor_id);
    }

    /// 注销 ActrId 的所有路由
    pub fn unregister_id(&self, actor_id: &ActrId) {
        let mut routes = self.id_routes.write();
        routes.remove(actor_id);
        log::debug!(
            "[RouteTable] Unregistered all routes for ActrId: {:?}",
            actor_id
        );
    }

    /// 查询 ActrId 的最优路由
    pub fn lookup_id(&self, actor_id: &ActrId) -> Option<Dest> {
        let routes = self.id_routes.read();
        routes.get(actor_id).and_then(|entries| {
            // 返回第一个可用的路由
            entries
                .iter()
                .filter(|e| e.available)
                .next()
                .map(|e| e.dest.clone())
        })
    }

    /// 查询 ActrId 的所有可用路由
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

    // ========== ActrType 路由 ==========

    /// 注册 ActrType 路由（服务发现）
    pub fn register_type(&self, actr_type: ActrType, entry: RouteEntry) {
        let mut routes = self.type_routes.write();
        routes.entry(actr_type.clone()).or_default().push(entry);

        // 按优先级排序
        if let Some(entries) = routes.get_mut(&actr_type) {
            entries.sort_by_key(|e| e.priority);
        }

        log::debug!(
            "[RouteTable] Registered route for ActrType: {:?}",
            actr_type
        );
    }

    /// 查询 ActrType 的最优路由
    pub fn lookup_type(&self, actr_type: &ActrType) -> Option<Dest> {
        let routes = self.type_routes.read();
        routes.get(actr_type).and_then(|entries| {
            entries
                .iter()
                .filter(|e| e.available)
                .next()
                .map(|e| e.dest.clone())
        })
    }

    /// 查询 ActrType 的所有可用路由（用于负载均衡）
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

    // ========== 路由状态管理 ==========

    /// 标记路由为不可用
    pub fn mark_unavailable(&self, dest: &Dest) {
        // 更新 id_routes
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

        // 更新 type_routes
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

    /// 标记路由为可用
    pub fn mark_available(&self, dest: &Dest) {
        // 更新 id_routes
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

        // 更新 type_routes
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

    /// 获取统计信息
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

/// 路由表统计信息
#[derive(Debug, Clone)]
pub struct RouteTableStats {
    /// ActrId 路由数
    pub id_route_count: usize,

    /// ActrType 路由数
    pub type_route_count: usize,

    /// 总条目数
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
                version: "v1".to_string(),
            },
        };

        let dest = Dest::Peer("node1".to_string());

        table.register_id(actor_id.clone(), RouteEntry::new(dest.clone()));

        let result = table.lookup_id(&actor_id);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), dest);
    }
}
