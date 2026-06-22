# 地理位置负载均衡

本文档说明如何使用基于地理位置的负载均衡功能。

## 功能概述

信令服务支持基于服务实例之间地理距离的负载均衡。

**架构设计：**
- **Actor 自行提供坐标** - 各 Actor 在注册时自己提供 `geo_location`（通过 GeoIP、配置文件、GPS 等方式）
- **信令服务只做记录** - SignalingServer 存储和使用这些坐标，不需要查询 GeoIP 数据库
- **Haversine 距离计算** - 信令服务计算候选服务之间的地球表面大圆距离

**GeoIP 工具的位置：**
- GeoIP 查询工具（IP → 坐标）已移到 `actr-framework` 的 util 模块
- 各 Actor 可根据需要使用 GeoIP 或其他方式获取坐标

## 快速开始

### 1. Actor 注册时提供地理位置

**方式 1：通过 GeoIP 查询**（需要 `actr-framework` 的 GeoIP util）

```rust
use actr_protocol::{RegisterRequest, ServiceLocation};
use actr_framework::util::geoip::GeoIpService;  // 假设 GeoIP 工具在这里

// 查询本机 IP 的坐标
let geoip = GeoIpService::new("path/to/GeoLite2-City.mmdb")?;
let my_ip = local_ip_address::local_ip()?;
let (lat, lon) = geoip.lookup(my_ip).unwrap_or((0.0, 0.0));

let request = RegisterRequest {
    actr_type: my_type,
    // ... 其他字段
    geo_location: Some(ServiceLocation {
        region: "auto-detected".to_string(),
        latitude: Some(lat),
        longitude: Some(lon),
    }),
};
```

**方式 2：通过配置文件**

```toml
# actor_config.toml
[geo_location]
region = "cn-beijing"
latitude = 39.9042
longitude = 116.4074
```

```rust
// 从配置文件读取
let config: ActorConfig = toml::from_str(&config_str)?;
let request = RegisterRequest {
    actr_type: my_type,
    geo_location: Some(config.geo_location),
};
```

**方式 3：移动设备使用 GPS**

```rust
// 获取 GPS 坐标（伪代码）
let (lat, lon) = device.get_gps_coordinates();
let request = RegisterRequest {
    actr_type: my_type,
    geo_location: Some(ServiceLocation {
        region: "mobile".to_string(),
        latitude: Some(lat),
        longitude: Some(lon),
    }),
};
```

### 2. 客户端请求路由

客户端在请求路由时，指定 `NEAREST` 排序因子：

```rust
use actr_protocol::{
    RouteCandidatesRequest,
    route_candidates_request::{NodeSelectionCriteria, node_selection_criteria::NodeRankingFactor},
};

let request = RouteCandidatesRequest {
    target_type: target_type,
    criteria: Some(NodeSelectionCriteria {
        candidate_count: 3,
        ranking_factors: vec![
            NodeRankingFactor::Nearest as i32,
            NodeRankingFactor::MaximumPowerReserve as i32,
        ],
        // ... 其他字段
    }),
};
```

## 工作原理

### 架构流程

```
1. Actor 注册阶段
   Actor → 自行获取坐标 → RegisterRequest.geo_location
                              ↓
                      SignalingServer 记录坐标

2. 路由查询阶段
   客户端请求 → ServiceRegistry 过滤候选（50-200 个）
                              ↓
                   LoadBalancer 使用 Haversine 计算距离
                              ↓
                        按距离升序排序
                              ↓
                    返回最近的 N 个候选
```

### 两阶段查询策略

1. **过滤阶段** - ServiceRegistry 根据 ActrType 过滤候选（50-200 个）
2. **排序阶段** - LoadBalancer 使用 Haversine 计算距离并排序

### LoadBalancer API

```rust
use signaling::LoadBalancer;

// client_location: 客户端地理坐标 (latitude, longitude)
let client_location = Some((39.9042, 116.4074)); // 北京

let ranked = LoadBalancer::rank_candidates(
    candidates,
    criteria,
    client_id,
    client_location,  // 传入客户端坐标
);
```

## 性能特性

- **查询延迟** - 内存计算，< 1ms（50-200 个候选）
- **精度** - 取决于 Actor 提供的坐标精度（GeoIP 城市级 50-100km，GPS 可达米级）
- **扩展性** - 支持数万服务实例
- **灵活性** - Actor 可选择最适合的坐标获取方式

## 降级行为

1. **无客户端坐标** - 使用简单优先级排序（有 geo_location 的优先）
2. **无服务坐标** - 该服务排在最后
3. **坐标无效** - 视为无坐标处理

## 故障排查

### Actor 未提供坐标

**现象：** 服务注册成功但 geo_location 为 None

**排查步骤：**
1. 检查 RegisterRequest 是否包含 geo_location
2. 检查 Actor 的坐标获取逻辑（GeoIP/配置文件/GPS）
3. 查看信令服务日志

### 距离计算结果不符合预期

**可能原因：**
1. Actor 提供的坐标不准确
2. 使用 GeoIP 时精度限制（城市级，50-100km 误差）
3. 客户端坐标未传递到 LoadBalancer

**解决方法：**
- 验证 Actor 提供的坐标正确性
- 考虑使用更精确的坐标来源（GPS、配置文件）
- 检查 LoadBalancer 调用时是否传递了 client_location

### 排序未生效

**检查点：**
1. 确认 RouteCandidatesRequest 包含 `NodeRankingFactor::Nearest`
2. 确认候选服务有 geo_location
3. 确认传递了有效的 client_location 给 LoadBalancer

## 示例

### Actor 端完整示例

```rust
use actr_protocol::{RegisterRequest, ServiceLocation, ActrType, Realm};

// 从配置文件读取坐标
#[derive(Deserialize)]
struct ActorConfig {
    geo_location: ServiceLocation,
}

let config: ActorConfig = toml::from_str(r#"
    [geo_location]
    region = "cn-beijing"
    latitude = 39.9042
    longitude = 116.4074
"#)?;

// 注册到信令服务
let request = RegisterRequest {
    realm: Realm { realm_id: 1 },
    actr_type: ActrType {
        manufacturer: "my-company".to_string(),
        name: "my-service".to_string(),
    },
    geo_location: Some(config.geo_location),
    // ... 其他字段
};

// 发送注册请求
client.register(request).await?;
```

### 测试用例

```rust
#[test]
fn test_geographic_routing() {
    // 客户端位置：北京
    let client_location = Some((39.9042, 116.4074));

    // 候选服务：上海、深圳、北京
    let candidates = vec![
        create_service_with_location("shanghai", 31.2304, 121.4737),
        create_service_with_location("shenzhen", 22.5431, 114.0579),
        create_service_with_location("beijing", 39.9042, 116.4074),
    ];

    let criteria = NodeSelectionCriteria {
        candidate_count: 3,
        ranking_factors: vec![NodeRankingFactor::Nearest as i32],
        // ...
    };

    let ranked = LoadBalancer::rank_candidates(
        candidates,
        Some(&criteria),
        None,
        client_location,
    );

    // 排序结果：北京(0km) < 上海(~1067km) < 深圳(~1943km)
    assert_eq!(ranked[0].name, "beijing");
    assert_eq!(ranked[1].name, "shanghai");
    assert_eq!(ranked[2].name, "shenzhen");
}
```

## 最佳实践

1. **坐标提供** - 根据场景选择合适的坐标来源：
   - 服务器：配置文件或 GeoIP（使用 actr-framework util）
   - 移动设备：GPS（精确到米级）
   - 边缘节点：部署时配置
2. **多因子组合** - 结合 `NEAREST` 和 `MAXIMUM_POWER_RESERVE` 等因子
3. **服务分区** - 在同一区域部署多个服务实例以提高可用性
4. **监控指标** - 记录地理距离分布，优化服务部署
5. **坐标验证** - Actor 启动时验证坐标有效性（-90 ≤ lat ≤ 90，-180 ≤ lon ≤ 180）

## 参考

- Haversine 公式: https://en.wikipedia.org/wiki/Haversine_formula
- LoadBalancer 文档: `crates/signaling/src/load_balancer.rs`
- GeoIP 工具: `actr-framework/util/geoip`（TODO: 待实现）
