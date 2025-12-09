# Actrix 服务管理文档

**版本**: v0.1.0
**最后更新**: 2025-11-03
**文档性质**: 100% 基于实际代码的准确映射

本文档记录 Actrix 服务的生命周期管理、部署、运维和监控细节。

---

## 📋 目录

- [1. 服务架构概述](#1-服务架构概述)
- [2. 服务类型和接口](#2-服务类型和接口)
- [3. ServiceManager - 服务管理器](#3-servicemanager---服务管理器)
- [4. 服务启动流程](#4-服务启动流程)
- [5. 服务配置和控制](#5-服务配置和控制)
- [6. 服务监控和健康检查](#6-服务监控和健康检查)
- [7. 优雅关闭](#7-优雅关闭)
- [8. 生产部署](#8-生产部署)
- [9. 故障排查](#9-故障排查)

---

## 1. 服务架构概述

### 1.1 服务分类

**文件**: `src/service/mod.rs:1-14`

Actrix 服务分为两大类:

```rust
/// HTTP路由服务的核心 trait - 为 axum 提供路由器
#[async_trait]
pub trait HttpRouterService: Send + Sync + Debug {
    fn info(&self) -> &ServiceInfo;
    async fn build_router(&mut self) -> Result<Router>;
    fn route_prefix(&self) -> &str;
}

/// ICE服务的核心 trait - 独立的 UDP 服务器
#[async_trait]
pub trait IceService: Send + Sync + Debug {
    fn info(&self) -> &ServiceInfo;
    async fn start(&mut self, shutdown_rx, oneshot_tx) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
}
```

**关键区别**:

| 特性       | HttpRouterService       | IceService      |
| ---------- | ----------------------- | --------------- |
| **协议**   | HTTP/HTTPS (TCP)        | UDP             |
| **服务器** | 共享单个 axum 实例      | 独立 UDP socket |
| **端口**   | 共享 (如 8443)          | 独立 (如 3478)  |
| **路由**   | URL 路径分发            | 协议内容分发    |
| **示例**   | KS, AIS, Signaling (WS) | STUN, TURN      |

### 1.2 服务类型枚举

**文件**: `src/service/mod.rs:48-56`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Display, PartialEq, Eq)]
pub enum ServiceType {
    Stun,        // STUN 服务 (ICE)
    Turn,        // TURN 服务 (ICE)
    Signaling,   // 信令服务 (HTTP/WS)
    Supervisor,  // 管理平台客户端 (HTTP)
    Ais,         // Actor Identity Service (HTTP)
    Ks,          // Key Server (HTTP)
}
```

### 1.3 当前启用的服务

根据代码实际状态:

| 服务           | 类型    | 状态     | 位掩码       | 说明           |
| -------------- | ------- | -------- | ------------ | -------------- |
| **KS**         | HTTP    | ✅ 已启用 | 16 (0b10000) | 密钥服务器     |
| **STUN**       | ICE     | ✅ 已启用 | 2 (0b00010)  | NAT 穿越       |
| **TURN**       | ICE     | ✅ 已启用 | 4 (0b00100)  | 网络中继       |
| **AIS**        | HTTP    | ✅ 已启用 | 8 (0b01000)  | Actor 身份服务 |
| **Signaling**  | HTTP/WS | ⚠️ 待重构 | 1 (0b00001)  | WebRTC 信令    |
| **Supervisor** | HTTP    | ⚠️ 可选   | -            | 管理平台客户端 |

---

## 2. 服务类型和接口

### 2.1 HttpRouterService 详解

#### 2.1.1 接口定义

**文件**: `src/service/mod.rs:85-112`

```rust
#[async_trait]
pub trait HttpRouterService: Send + Sync + Debug {
    /// 获取服务信息
    fn info(&self) -> &ServiceInfo;

    /// 获取可变的服务信息
    fn info_mut(&mut self) -> &mut ServiceInfo;

    /// 构建 axum 路由器
    async fn build_router(&mut self) -> Result<Router>;

    /// 服务启动回调（路由器已构建并启动后调用）
    async fn on_start(&mut self, base_url: Url) -> Result<()> {
        self.info_mut().set_running(base_url);
        Ok(())
    }

    /// 服务停止回调
    async fn on_stop(&mut self) -> Result<()> {
        info!("HTTP router service '{}' stopped", self.info().name);
        self.info_mut().status = ServiceStatus::Unknown;
        Ok(())
    }

    /// 获取路由前缀（如 "/ks", "/ais" 等）
    fn route_prefix(&self) -> &str;
}
```

#### 2.1.2 实现示例 - KS HTTP Service

**文件**: `src/service/http/ks.rs:20-90`

```rust
pub struct KsHttpService {
    info: ServiceInfo,
    state: Option<KSState>,
    config: ActrixConfig,
}

impl KsHttpService {
    pub fn new(config: ActrixConfig) -> Self {
        Self {
            info: ServiceInfo::new("KS", ServiceType::Ks),
            state: None,
            config,
        }
    }
}

#[async_trait]
impl HttpRouterService for KsHttpService {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn info_mut(&mut self) -> &mut ServiceInfo {
        &mut self.info
    }

    async fn build_router(&mut self) -> Result<Router> {
        let ks_config = self.config.ks.as_ref()
            .ok_or_else(|| anyhow!("KS configuration is missing"))?;

        // 创建 KSState
        let state = ks::create_ks_state(
            ks_config,
            self.config.get_actrix_shared_key()
        ).await?;

        self.state = Some(state.clone());

        // 创建路由器
        let router = ks::create_router(state);

        info!("KS HTTP service router built successfully");
        Ok(router)
    }

    fn route_prefix(&self) -> &str {
        "/ks"
    }
}
```

**路由结构**:
```
/ks
├── POST   /generate         - 生成新密钥对
├── GET    /secret/{key_id}  - 获取私钥
└── GET    /health           - 健康检查
```

#### 2.1.3 实现示例 - AIS HTTP Service

**文件**: `src/service/http/ais.rs`

```rust
pub struct AisHttpService {
    info: ServiceInfo,
    config: ActrixConfig,
}

impl AisHttpService {
    pub fn new(config: ActrixConfig) -> Self {
        Self {
            info: ServiceInfo::new(
                "AIS Service",
                ServiceType::Ais,
                Some("Actor Identity Service - ActrId 注册和凭证签发服务".to_string()),
                &config,
            ),
            config,
        }
    }
}

#[async_trait]
impl HttpRouterService for AisHttpService {
    async fn build_router(&mut self) -> Result<Router> {
        let ais_config = self.config.services.ais.as_ref()
            .ok_or_else(|| anyhow!("AIS configuration is missing"))?;

        // 创建 AIS 路由器（内部会初始化 Issuer、KS Client 等）
        let router = ais::create_ais_router(ais_config, &self.config).await?;

        info!("AIS HTTP service router built successfully");
        Ok(router)
    }

    fn route_prefix(&self) -> &str {
        "/ais"
    }
}
```

**路由结构**:
```
/ais
├── POST   /allocate  - ActrId 注册（Protobuf binary）
├── GET    /health    - 健康检查
└── GET    /info      - 服务信息
```

**关键特性**:
- **Protobuf 协议**：`/allocate` 端点使用 `application/octet-stream`
- **高性能 Snowflake**：无锁 CAS 算法，理论吞吐量 500K IDs/s
- **智能密钥管理**：自动从 KS 获取密钥，本地缓存 + 后台刷新
- **健康检查**：验证 KS 连通性 + 数据库读写 + 密钥缓存状态

**配置依赖**:
```toml
enable = 8  # ENABLE_AIS (位 3) 或与其他服务组合，如 enable = 15 (1+2+4+8)

[services.ais]
[services.ais.server]
# Note: AIS key storage file is automatically set to {sqlite_path}/keys.db
token_ttl_secs = 3600

[services.ais.dependencies.ks]
# 可选：如果不配置，自动使用本地 KS（如果启用）
endpoint = "http://localhost:50052"  # gRPC 端口
```

### 2.2 IceService 详解

#### 2.2.1 接口定义

**文件**: `src/service/mod.rs:114-142`

```rust
#[async_trait]
pub trait IceService: Send + Sync + Debug {
    /// 获取服务信息
    fn info(&self) -> &ServiceInfo;

    /// 获取可变的服务信息
    fn info_mut(&mut self) -> &mut ServiceInfo;

    /// 启动 ICE 服务
    async fn start(
        &mut self,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
        oneshot_tx: tokio::sync::oneshot::Sender<ServiceInfo>,
    ) -> Result<()>;

    /// 停止 ICE 服务
    async fn stop(&mut self) -> Result<()> {
        info!("ICE service '{}' stopped", self.info().name);
        self.info_mut().status = ServiceStatus::Unknown;
        Ok(())
    }

    /// 获取服务健康状态
    async fn health_check(&self) -> Result<bool> {
        Ok(self.info().is_running())
    }
}
```

#### 2.2.2 实现示例 - STUN Service

**文件**: `src/service/ice/stun.rs:15-100`

```rust
pub struct StunService {
    info: ServiceInfo,
    config: ActrixConfig,
}

impl StunService {
    pub fn new(config: ActrixConfig) -> Self {
        Self {
            info: ServiceInfo::new("STUN", ServiceType::Stun),
            config,
        }
    }
}

#[async_trait]
impl IceService for StunService {
    fn info(&self) -> &ServiceInfo {
        &self.info
    }

    fn info_mut(&mut self) -> &mut ServiceInfo {
        &mut self.info
    }

    async fn start(
        &mut self,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
        oneshot_tx: tokio::sync::oneshot::Sender<ServiceInfo>,
    ) -> Result<()> {
        let bind_config = &self.config.bind.ice;

        // 绑定 UDP socket
        let bind_addr = format!("{}:{}", bind_config.ip, bind_config.port);
        let socket = Arc::new(UdpSocket::bind(&bind_addr).await?);

        let actual_addr = socket.local_addr()?;
        info!("STUN service bound to {}", actual_addr);

        // 构建服务信息
        let base_url = Url::parse(&format!("stun://{}", actual_addr))?;
        self.info.set_running(base_url);

        // 发送服务信息回主线程
        oneshot_tx.send(self.info.clone())
            .map_err(|_| anyhow!("Failed to send service info"))?;

        // 启动 STUN 服务器
        stun::create_stun_server_with_shutdown(socket, shutdown_rx).await?;

        Ok(())
    }
}
```

#### 2.2.3 实现示例 - TURN Service

**文件**: `src/service/ice/turn.rs:15-120`

```rust
pub struct TurnService {
    info: ServiceInfo,
    config: ActrixConfig,
}

#[async_trait]
impl IceService for TurnService {
    async fn start(
        &mut self,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
        oneshot_tx: tokio::sync::oneshot::Sender<ServiceInfo>,
    ) -> Result<()> {
        let bind_config = &self.config.bind.ice;
        let turn_config = &self.config.turn;

        // 绑定 UDP socket
        let bind_addr = format!("{}:{}", bind_config.ip, bind_config.port);
        let socket = Arc::new(UdpSocket::bind(&bind_addr).await?);

        let actual_addr = socket.local_addr()?;
        info!("TURN service bound to {}", actual_addr);

        // 创建认证器
        let auth_handler: Arc<dyn AuthHandler + Send + Sync> =
            Arc::new(turn::Authenticator::new()?);

        // 创建 TURN 服务器
        let server = turn::create_turn_server(
            socket,
            &turn_config.advertised_ip,
            &turn_config.realm,
            auth_handler,
        ).await?;

        // 构建服务信息
        let base_url = Url::parse(&format!("turn://{}",
            turn_config.advertised_ip))?;
        self.info.set_running(base_url);

        // 发送服务信息
        oneshot_tx.send(self.info.clone())
            .map_err(|_| anyhow!("Failed to send service info"))?;

        // 等待关闭信号
        let mut shutdown_rx = shutdown_rx;
        let _ = shutdown_rx.recv().await;

        // 关闭 TURN 服务器
        turn::shutdown_turn_server(&server).await?;

        Ok(())
    }
}
```

### 2.3 ServiceInfo - 服务元数据

**文件**: `src/service/info.rs:10-80`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub service_type: ServiceType,
    pub status: ServiceStatus,
    pub base_url: Option<Url>,
    pub started_at: Option<SystemTime>,
}

impl ServiceInfo {
    pub fn new(name: &str, service_type: ServiceType) -> Self {
        Self {
            name: name.to_string(),
            service_type,
            status: ServiceStatus::Unknown,
            base_url: None,
            started_at: None,
        }
    }

    /// 设置服务为运行状态
    pub fn set_running(&mut self, base_url: Url) {
        self.status = ServiceStatus::Healthy;
        self.base_url = Some(base_url);
        self.started_at = Some(SystemTime::now());
    }

    /// 检查服务是否正在运行
    pub fn is_running(&self) -> bool {
        self.status == ServiceStatus::Healthy
    }

    /// 获取上报 URL
    pub fn report_url(&self) -> String {
        self.base_url
            .as_ref()
            .map(|u| u.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// 获取运行时长 (秒)
    pub fn uptime_secs(&self) -> Option<u64> {
        self.started_at.and_then(|started| {
            SystemTime::now()
                .duration_since(started)
                .ok()
                .map(|d| d.as_secs())
        })
    }
}
```

---

## 3. ServiceManager - 服务管理器

### 3.1 结构定义

**文件**: `src/service/manager.rs:23-31`

```rust
pub struct ServiceManager {
    services: Vec<ServiceContainer>,
    ice_handles: Vec<JoinHandle<Result<()>>>,
    http_handle: Option<JoinHandle<Result<()>>>,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    collected_service_info: Arc<RwLock<HashMap<String, ServiceInfo>>>,
    config: ActrixConfig,
}
```

**字段说明**:
- `services`: 所有待启动的服务容器
- `ice_handles`: ICE 服务的任务句柄 (用于等待)
- `http_handle`: HTTP 服务器的任务句柄
- `shutdown_tx`: 关闭信号广播器
- `collected_service_info`: 收集的服务信息 (用于注册到管理平台)
- `config`: 全局配置

### 3.2 创建服务管理器

**文件**: `src/service/manager.rs:34-45`

```rust
impl ServiceManager {
    pub fn new(config: ActrixConfig) -> Self {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(10);
        Self {
            services: Vec::new(),
            ice_handles: Vec::new(),
            http_handle: None,
            shutdown_tx,
            collected_service_info: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }
}
```

### 3.3 添加服务

**文件**: `src/service/manager.rs:47-51`

```rust
pub fn add_service(&mut self, service: ServiceContainer) {
    info!("Adding service '{}' to manager", service.info().name);
    self.services.push(service);
}
```

### 3.4 启动所有服务

**文件**: `src/service/manager.rs:126-178`

```rust
pub async fn start_all(&mut self) -> Result<()> {
    info!(
        "Starting all {} types ({}) services.",
        self.services.len(),
        self.services
            .iter()
            .map(|s| s.info().service_type.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let services = std::mem::take(&mut self.services);
    let mut http_services = Vec::new();
    let mut ice_services = Vec::new();

    // 分离 HTTP 路由服务和 ICE 服务
    for service in services {
        if service.is_http_router() {
            http_services.push(service);
        } else if service.is_ice() {
            ice_services.push(service);
        }
    }

    let notify = Arc::new(Notify::new());
    let notify_clone = notify.clone();

    // 启动 HTTP 服务器（合并所有 HTTP 路由服务）
    if !http_services.is_empty() {
        self.start_http_services(http_services, notify_clone).await?;
    }
    notify.notified().await;

    // 启动 ICE 服务
    for service in ice_services {
        self.start_ice_service(service, notify.clone()).await?;
        notify.notified().await;
    }

    // 收集服务信息
    let services = self
        .collected_service_info
        .read()
        .map_err(|e| anyhow!("Failed to read collected service info: {}", e))?
        .values()
        .cloned()
        .collect();

    // 注册到管理平台
    self.register_services(services).await?;

    Ok(())
}
```

**启动流程**:
1. 分离服务类型 (HTTP vs ICE)
2. 启动 HTTP 服务器 (合并所有 HTTP 路由)
3. 逐个启动 ICE 服务 (独立 UDP socket)
4. 收集服务信息
5. 注册到管理平台 (可选)

### 3.5 启动 HTTP 服务器

**文件**: `src/service/manager.rs:180-315`

```rust
async fn start_http_services(
    &mut self,
    mut services: Vec<ServiceContainer>,
    notify: Arc<Notify>,
) -> Result<()> {
    let is_dev = self.config.env.to_lowercase() == "dev";
    let protocol = if is_dev { "HTTP" } else { "HTTPS" };

    info!(
        "Starting {} server with {} route services (environment: {})",
        protocol,
        services.len(),
        self.config.env
    );

    let shutdown_rx = self.shutdown_tx.subscribe();

    // 确定绑定配置
    let (bind_addr, public_url, tls_config) = if is_dev {
        // 开发环境：优先使用 HTTP
        if let Some(ref http) = self.config.bind.http {
            let addr = format!("{}:{}", http.ip, http.port);
            let url = format!("http://{}", addr);
            (addr, url, None)
        } else if let Some(ref https) = self.config.bind.https {
            // 没有 HTTP 配置，使用 HTTPS
            let addr = format!("{}:{}", https.ip, https.port);
            let url = format!("https://{}", addr);
            let tls = TlsConfigurer::from_pem_files(&https.cert, &https.key)?;
            (addr, url, Some(tls))
        } else {
            return Err(anyhow!("No HTTP/HTTPS binding configured"));
        }
    } else {
        // 生产环境：强制使用 HTTPS
        if let Some(ref https) = self.config.bind.https {
            let addr = format!("{}:{}", https.ip, https.port);
            let url = format!("https://{}", addr);
            let tls = TlsConfigurer::from_pem_files(&https.cert, &https.key)?;
            (addr, url, Some(tls))
        } else {
            return Err(anyhow!("HTTPS binding required in production environment"));
        }
    };

    // 构建合并的路由器
    let mut app = Router::new();

    for service in &mut services {
        let mut http_service = service.as_http_router_mut()?;

        let router = http_service.build_router().await?;
        let prefix = http_service.route_prefix();

        info!("Nesting route '{}' at prefix '{}'", http_service.info().name, prefix);
        app = app.nest(prefix, router);

        // 调用 on_start 回调
        let base_url = Url::parse(&format!("{}{}", public_url, prefix))?;
        http_service.on_start(base_url).await?;

        // 收集服务信息
        let mut collected = self.collected_service_info.write().unwrap();
        collected.insert(
            http_service.info().name.clone(),
            http_service.info().clone()
        );
    }

    // 添加中间件
    use crate::service::trace::http_trace_layer;
    use tower_http::cors::CorsLayer;

    app = app
        .layer(http_trace_layer())  // HTTP 追踪 (OpenTelemetry)
        .layer(CorsLayer::permissive());  // CORS 支持

    info!("{} server will bind to: {}", protocol, bind_addr);

    // 启动服务器
    let handle = if let Some(tls_config) = tls_config {
        // HTTPS 服务器
        tokio::spawn(async move {
            let tls_config = RustlsConfig::from_config(Arc::new(tls_config));
            axum_server::bind_rustls(bind_addr.parse()?, tls_config)
                .serve(app.into_make_service())
                .await?;
            Ok(())
        })
    } else {
        // HTTP 服务器
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
            axum::serve(listener, app).await?;
            Ok(())
        })
    };

    self.http_handle = Some(handle);
    notify.notify_one();

    Ok(())
}
```

**关键步骤**:
1. 根据环境 (dev/prod) 决定 HTTP vs HTTPS
2. 为每个 HTTP 服务构建路由器
3. 使用 `Router::nest` 合并到主路由器
4. 添加中间件 (trace, CORS)
5. 启动 axum 服务器
6. 通知启动完成

**路由合并示例**:
```
/ks
  ├── POST /generate
  ├── GET  /secret/{key_id}
  └── GET  /health
/ais
  ├── POST /allocate
  └── GET  /validate
/signaling
  └── WS   /ws
```

### 3.6 启动 ICE 服务

**文件**: `src/service/manager.rs:317-360`

```rust
async fn start_ice_service(
    &mut self,
    mut service: ServiceContainer,
    notify: Arc<Notify>,
) -> Result<()> {
    let mut ice_service = service.as_ice_mut()?;

    info!("Starting ICE service: {}", ice_service.info().name);

    let shutdown_rx = self.shutdown_tx.subscribe();
    let (oneshot_tx, oneshot_rx) = tokio::sync::oneshot::channel();

    // 在后台任务中启动服务
    let service_name = ice_service.info().name.clone();
    let collected_info = self.collected_service_info.clone();

    let handle = tokio::spawn(async move {
        // 启动服务
        let result = ice_service.start(shutdown_rx, oneshot_tx).await;

        // 等待服务信息
        if let Ok(info) = oneshot_rx.await {
            let mut collected = collected_info.write().unwrap();
            collected.insert(info.name.clone(), info);
        }

        result
    });

    self.ice_handles.push(handle);
    notify.notify_one();

    info!("ICE service '{}' started", service_name);
    Ok(())
}
```

**启动流程**:
1. 创建 shutdown 接收器
2. 创建 oneshot 通道 (用于回传服务信息)
3. 在后台任务中启动服务
4. 收集服务信息
5. 保存任务句柄
6. 通知启动完成

### 3.7 注册到管理平台

**文件**: `src/service/manager.rs:49-74`

服务通过 SupervitClient 的 Report RPC 自动上报到管理平台：

```rust
pub async fn register_services(&self, services: Vec<ServiceInfo>) -> Result<()> {
    // 检查是否配置了管理平台
    let managed_config = match &self.config.supervisor {
        Some(config) => config,
        None => {
            warn!(
                "No management platform configured, skipping service registration for '{:?}'",
                services
            );
            return Ok(());
        }
    };

    // 服务注册通过 SupervitClient 的 Report RPC 自动完成
    info!(
        "Service registration via gRPC: {} services will be reported to {}",
        services.len(),
        managed_config
            .client
            .as_ref()
            .map(|c| c.endpoint.as_str())
            .unwrap_or("<missing endpoint>")
    );

    Ok(())
}
```

**实现方式**:

- 服务状态通过 SupervitClient 的 Unary RPC 自动上报
- 不需要手动构建和发送注册负载
- SupervitClient 在其他地方初始化并管理连接

---

## 4. 服务启动流程

### 4.1 完整启动序列

**文件**: `src/main.rs:250-350` (ApplicationLauncher::run_application)

```
1. 加载配置文件
   ├─ 查找配置文件 (config.toml 或 /etc/actor-rtc-actrix/config.toml)
   ├─ 解析 TOML
   └─ 验证配置

2. 初始化可观测性系统
   ├─ 创建日志目录
   ├─ 配置 tracing 订阅器
   ├─ (可选) 初始化 OpenTelemetry
   └─ 启动日志写入器

3. 创建 ServiceManager
   └─ 初始化关闭信号通道

4. 根据配置启用服务
   ├─ 检查 enable 位掩码
   ├─ 创建服务实例
   │   ├─ KsHttpService (if enable & 16)
   │   ├─ StunService (if enable & 2)
   │   ├─ TurnService (if enable & 4)
   │   └─ ...
   └─ 添加到 ServiceManager

5. 启动所有服务
   ├─ 分离 HTTP 和 ICE 服务
   ├─ 启动 HTTP 服务器 (合并路由)
   │   ├─ 为每个 HTTP 服务构建 Router
   │   ├─ 使用 nest() 合并路由
   │   ├─ 添加中间件 (trace, CORS)
   │   └─ 启动 axum 服务器
   ├─ 启动 ICE 服务 (逐个)
   │   ├─ 绑定 UDP socket
   │   ├─ 创建服务器实例
   │   └─ 在后台任务中运行
   └─ 注册到管理平台

6. 等待信号
   ├─ 监听 SIGINT (Ctrl+C)
   ├─ 监听 SIGTERM
   └─ 阻塞主线程

7. 优雅关闭
   ├─ 广播关闭信号
   ├─ 等待 HTTP 服务器停止
   ├─ 等待所有 ICE 服务停止
   ├─ 清理资源
   └─ 退出
```

### 4.2 实际启动代码

**文件**: `src/main.rs:280-360`

```rust
fn run_application(config_path: &PathBuf) -> Result<()> {
    // 1. 加载配置
    let config = ActrixConfig::from_file(config_path)?;

    // 2. 验证配置
    if let Err(errors) = config.validate() {
        error!("Configuration validation failed:");
        for error in errors {
            error!("  - {}", error);
        }
        return Err(Error::custom("Configuration validation failed"));
    }

    // 3. 初始化可观测性
    let _guard = Self::init_observability(config.observability_config())?;

    // 4. 创建 Tokio runtime 并运行
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        // 5. 创建全局 shutdown 广播通道 + ServiceManager
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(10);
        let mut manager = ServiceManager::new(config.clone(), shutdown_tx.clone());

        // 6. 根据配置注册服务
        if config.is_signaling_enabled() {
            manager.add_service(ServiceContainer::signaling(SignalingService::new(
                config.clone(),
            )));
        }
        if config.is_ks_enabled() {
            manager.add_service(ServiceContainer::ks(KsHttpService::new(config.clone())));
        }
        if config.is_turn_enabled() {
            manager.add_service(ServiceContainer::turn(TurnService::new(config.clone())));
        } else if config.is_stun_enabled() {
            manager.add_service(ServiceContainer::stun(StunService::new(config.clone())));
        }

        // 7. 启动所有服务并收集 JoinHandle
        let mut handles = manager.start_all().await?;

        // 8. 附加 KS gRPC 任务（同样监听 shutdown 通道）
        if config.is_ks_enabled() {
            let mut ks_grpc = KsGrpcService::new(config.clone());
            handles.push(
                ks_grpc
                    .start("127.0.0.1:50052".parse()?, shutdown_tx.clone())
                    .await?,
            );
        }

        info!("All services started successfully");

        // 9. 顺序等待所有任务；任何任务失败都会触发 shutdown 广播
        for handle in handles {
            if let Err(err) = handle.await {
                tracing::error!("background task crashed: {}", err);
                let _ = shutdown_tx.send(());
            }
        }

        // 10. 优雅关闭
        info!("Shutting down all services...");
        manager.stop_all().await?;
        info!("All services stopped gracefully");

        Ok::<_, Error>(())
    })?;

    Ok(())
}
```

---

## 5. 服务配置和控制

### 5.1 位掩码控制

**文件**: `crates/common/src/config/mod.rs:24-34`

```rust
pub struct ActrixConfig {
    /// 服务启用标志位 (位掩码)
    ///
    /// 使用二进制位掩码控制各个服务的启用状态：
    /// - 位 0 (1): Signaling 信令服务
    /// - 位 1 (2): STUN 服务
    /// - 位 2 (4): TURN 服务
    /// - 位 3 (8): AIS 身份认证服务
    /// - 位 4 (16): KS 密钥服务
    pub enable: u8,
    // ... 其他字段
}
```

**使用示例**:

```toml
# 启用所有服务
enable = 31  # 1 + 2 + 4 + 8 + 16 = 0b11111

# 仅启用 STUN + TURN
enable = 6   # 2 + 4 = 0b00110

# 仅启用 KS
enable = 16  # 0b10000

# 启用 Signaling + STUN + TURN (典型 WebRTC 部署)
enable = 7   # 1 + 2 + 4 = 0b00111
```

**检查方法**:

The bitmask (`enable`) is the **primary switch** for all services. The `services.*.enabled` fields serve as **optional secondary switches** for fine-grained control.

```rust
impl ActrixConfig {
    // Signaling uses bitmask only (no secondary switch)
    pub fn is_signaling_enabled(&self) -> bool {
        self.enable & ENABLE_SIGNALING != 0
    }

    // STUN/TURN use bitmask only
    pub fn is_stun_enabled(&self) -> bool {
        self.enable & ENABLE_STUN != 0
    }

    pub fn is_turn_enabled(&self) -> bool {
        self.enable & ENABLE_TURN != 0
    }

    // AIS uses bitmask only (no secondary switch)
    pub fn is_ais_enabled(&self) -> bool {
        self.enable & ENABLE_AIS != 0
    }

    // KS uses bitmask only (no secondary switch)
    pub fn is_ks_enabled(&self) -> bool {
        self.enable & ENABLE_KS != 0
    }
}
```

**启用逻辑**:

**所有服务 (仅使用位掩码)**:
- Bitmask not set → Service disabled
- Bitmask set → Service enabled

### 5.2 配置文件示例

**文件**: `config.example.toml`

```toml
# 服务启用控制
enable = 22  # KS (16) + TURN (4) + STUN (2)
name = "actrix-01"
env = "prod"

# SQLite 数据库存储目录
sqlite_path = "/var/lib/actrix"

# 内部服务通信密钥
actrix_shared_key = "your-strong-random-key-here"

# 日志/追踪配置
[observability]
filter_level = "info"      # RUST_LOG 覆盖时优先生效

[observability.log]
output = "file"
rotate = true
path = "/var/log/actrix"

# 位置标签
location_tag = "us-west-1"

# HTTPS 绑定
[bind.https]
ip = "0.0.0.0"
port = 8443
domain_name = "actrix.example.com"
cert = "/etc/actrix/certs/server.crt"
key = "/etc/actrix/certs/server.key"

# ICE 服务绑定
[bind.ice]
ip = "0.0.0.0"
port = 3478

# TURN 配置
[turn]
advertised_ip = "203.0.113.10"  # 公网 IP
realm = "actrix.example.com"

# KS 服务配置
[services.ks]
# Note: Service enablement is controlled by the bitmask (enable field)
# Set ENABLE_KS bit (16) in the enable field to enable this service

[services.ks.storage]
backend = "sqlite"
key_ttl_seconds = 3600

[services.ks.storage.sqlite]
path = "/var/lib/actrix/ks.db"

# OpenTelemetry 追踪
[observability.tracing]
enable = true
service_name = "actrix-prod"
endpoint = "http://localhost:4317"
```

---

## 6. 服务监控和健康检查

### 6.1 ServiceStatus 枚举

**文件**: `crates/common/src/monitoring/mod.rs:10-25`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    Unknown,      // 未知状态
    Starting,     // 正在启动
    Healthy,      // 健康运行
    Degraded,     // 降级运行
    Unhealthy,    // 不健康
    Stopping,     // 正在停止
    Stopped,      // 已停止
}

impl ServiceStatus {
    pub fn is_operational(&self) -> bool {
        matches!(self, ServiceStatus::Healthy | ServiceStatus::Degraded)
    }

    pub fn is_down(&self) -> bool {
        matches!(self, ServiceStatus::Unhealthy | ServiceStatus::Stopped)
    }
}
```

### 6.2 健康检查端点

**KS 服务健康检查**:

**文件**: `crates/ks/src/handlers.rs:240-260`

```rust
async fn health_check_handler(
    State(app_state): State<KSState>,
) -> Result<Json<HealthCheckResponse>, KsError> {
    // 检查数据库连接
    let key_count = app_state.storage.get_key_count()?;

    Ok(Json(HealthCheckResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        key_count,
    }))
}

#[derive(Serialize)]
struct HealthCheckResponse {
    status: String,
    version: String,
    key_count: usize,
}
```

**访问示例**:
```bash
curl https://actrix.example.com/ks/health

{
  "status": "healthy",
  "version": "0.1.0",
  "key_count": 42
}
```

### 6.3 服务监控指标

#### 6.3.1 TURN 认证缓存统计

**文件**: `crates/turn/src/authenticator.rs:84-88`

```rust
impl Authenticator {
    /// 获取缓存统计信息（用于监控和调试）
    pub fn cache_stats() -> (usize, usize) {
        let cache = AUTH_KEY_CACHE.lock().unwrap();
        (cache.len(), cache.cap().get())  // (当前大小, 最大容量)
    }
}
```

**监控示例**:
```rust
let (size, capacity) = Authenticator::cache_stats();
info!("TURN auth cache: {}/{} entries", size, capacity);

if size as f64 / capacity as f64 > 0.9 {
    warn!("TURN auth cache is nearly full, consider increasing capacity");
}
```

#### 6.3.2 KS 密钥统计

**文件**: `crates/ks/src/storage.rs:250-270`

```rust
impl KeyStorage {
    /// 获取密钥总数
    pub fn get_key_count(&self) -> KsResult<usize> {
        let conn = self.connection.lock().unwrap();
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM keys",
            [],
            |row| row.get(0)
        )?;
        Ok(count)
    }

    /// 获取未过期密钥数
    pub fn get_active_key_count(&self) -> KsResult<usize> {
        let conn = self.connection.lock().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM keys
             WHERE expires_at = 0 OR expires_at > ?1",
            [now],
            |row| row.get(0)
        )?;
        Ok(count)
    }
}
```

### 6.4 OpenTelemetry 追踪

**启用追踪** (需要 `opentelemetry` feature):

```toml
[observability.tracing]
enable = true
service_name = "actrix-prod-us-west-1"
endpoint = "http://jaeger:4317"  # Jaeger OTLP gRPC endpoint
```

**追踪内容**:
- ✅ 所有 HTTP 请求 (方法、URI、耗时)
- ✅ 跨服务调用链路 (W3C Trace Context)
- ✅ 错误和异常
- ✅ 自定义 span 和事件

**Jaeger UI 示例**:
```
Timeline:
├─ [200ms] HTTP POST /ks/generate
│  ├─ [50ms] KS: Generate keypair
│  ├─ [30ms] KS: Store to database
│  └─ [10ms] KS: Serialize response
└─ [100ms] HTTP GET /ks/secret/123
   ├─ [40ms] KS: Query database
   └─ [5ms] KS: Serialize response
```

---

## 7. 优雅关闭

### 7.1 关闭机制

**文件**: `src/service/manager.rs:400-450`

```rust
impl ServiceManager {
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Initiating graceful shutdown");

        // 1. 广播关闭信号
        if let Err(e) = self.shutdown_tx.send(()) {
            warn!("Failed to send shutdown signal: {}", e);
        }

        // 2. 等待 HTTP 服务器停止
        if let Some(handle) = self.http_handle.take() {
            info!("Waiting for HTTP server to stop...");
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                handle
            ).await {
                Ok(result) => {
                    if let Err(e) = result {
                        error!("HTTP server task failed: {}", e);
                    } else {
                        info!("HTTP server stopped");
                    }
                }
                Err(_) => {
                    error!("HTTP server shutdown timeout");
                }
            }
        }

        // 3. 等待所有 ICE 服务停止
        info!("Waiting for {} ICE services to stop...", self.ice_handles.len());

        for (idx, handle) in self.ice_handles.drain(..).enumerate() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                handle
            ).await {
                Ok(result) => {
                    if let Err(e) = result {
                        error!("ICE service {} task failed: {}", idx, e);
                    } else {
                        info!("ICE service {} stopped", idx);
                    }
                }
                Err(_) => {
                    error!("ICE service {} shutdown timeout", idx);
                }
            }
        }

        info!("All services shut down gracefully");
        Ok(())
    }
}
```

**关闭超时**:
- HTTP 服务器: 30 秒
- 每个 ICE 服务: 10 秒

### 7.2 信号处理

**文件**: `src/main.rs:340-360`

```rust
// 等待关闭信号
tokio::select! {
    // Ctrl+C (SIGINT)
    _ = tokio::signal::ctrl_c() => {
        info!("Received Ctrl+C (SIGINT), shutting down...");
    }

    // SIGTERM (systemd, docker stop)
    _ = async {
        let mut sigterm = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate()
        ).unwrap();
        sigterm.recv().await;
    } => {
        info!("Received SIGTERM, shutting down...");
    }
}

// 执行优雅关闭
manager.shutdown().await?;
```

---

## 8. 生产部署

### 8.1 systemd 服务

**文件**: `install/actrix.service`

```ini
[Unit]
Description=Actrix WebRTC Auxiliary Services
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=actrix
Group=actrix
WorkingDirectory=/opt/actrix
ExecStart=/opt/actrix/bin/actrix --config /etc/actrix/config.toml
Restart=on-failure
RestartSec=5s

# 安全加固
PrivateTmp=yes
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/lib/actrix /var/log/actrix

# 能力
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE

# 日志
StandardOutput=journal
StandardError=journal
SyslogIdentifier=actrix

[Install]
WantedBy=multi-user.target
```

**安装和启动**:

```bash
# 复制服务文件
sudo cp install/actrix.service /etc/systemd/system/

# 重新加载 systemd
sudo systemctl daemon-reload

# 启用开机自启
sudo systemctl enable actrix

# 启动服务
sudo systemctl start actrix

# 查看状态
sudo systemctl status actrix

# 查看日志
sudo journalctl -u actrix -f
```

### 8.2 Docker 部署

**Dockerfile**:

```dockerfile
# 构建阶段
FROM rust:1.88-bullseye as builder

WORKDIR /build
COPY . .

# 构建发布版本 (带 OpenTelemetry)
RUN cargo build --release --features opentelemetry

# 运行阶段
FROM debian:bullseye-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# 创建用户
RUN groupadd -r actrix && useradd -r -g actrix actrix

# 复制二进制文件
COPY --from=builder /build/target/release/actrix /usr/local/bin/actrix

# 创建目录
RUN mkdir -p /var/lib/actrix /var/log/actrix /etc/actrix && \
    chown -R actrix:actrix /var/lib/actrix /var/log/actrix

USER actrix
WORKDIR /var/lib/actrix

# 暴露端口
EXPOSE 8443 3478/udp

# 启动命令
ENTRYPOINT ["/usr/local/bin/actrix"]
CMD ["--config", "/etc/actrix/config.toml"]
```

**docker-compose.yml**:

```yaml
version: '3.8'

services:
  actrix:
    build: .
    container_name: actrix
    restart: unless-stopped
    ports:
      - "8443:8443"
      - "3478:3478/udp"
    volumes:
      - ./config.toml:/etc/actrix/config.toml:ro
      - ./certs:/etc/actrix/certs:ro
      - actrix-data:/var/lib/actrix
      - actrix-logs:/var/log/actrix
    environment:
      - RUST_LOG=info
    networks:
      - actrix-network

  jaeger:
    image: jaegertracing/all-in-one:latest
    container_name: actrix-jaeger
    restart: unless-stopped
    ports:
      - "16686:16686"  # Jaeger UI
      - "4317:4317"    # OTLP gRPC
      - "4318:4318"    # OTLP HTTP
    environment:
      - COLLECTOR_OTLP_ENABLED=true
    networks:
      - actrix-network

volumes:
  actrix-data:
  actrix-logs:

networks:
  actrix-network:
    driver: bridge
```

**启动**:

```bash
# 构建并启动
docker-compose up -d

# 查看日志
docker-compose logs -f actrix

# 停止
docker-compose down
```

### 8.3 目录结构

**生产环境标准布局**:

```
/opt/actrix/
├── bin/
│   └── actrix                    # 主二进制文件
└── lib/                          # 共享库 (如果需要)

/etc/actrix/
├── config.toml                   # 主配置文件
├── certs/
│   ├── server.crt                # TLS 证书
│   └── server.key                # TLS 私钥
└── secrets/                      # 敏感信息 (可选)

/var/lib/actrix/
├── actrix.db                     # 主数据库
├── ks.db                         # KS 密钥数据库
└── nonce.db                      # Nonce 存储

/var/log/actrix/
├── actrix.log                    # 主日志文件
├── actrix.log.2025-11-01         # 轮转的日志
└── actrix.log.2025-11-02

/var/run/
└── actrix.pid                    # PID 文件
```

---

## 9. 故障排查

### 9.1 常见问题

#### 问题 1: 服务无法启动

**症状**:
```
ERROR Failed to start services: Address already in use
```

**排查**:
```bash
# 检查端口占用
sudo lsof -i :8443
sudo lsof -i :3478

# 检查配置文件
actrix --config /etc/actrix/config.toml test

# 查看详细日志
RUST_LOG=debug actrix --config config.toml
```

#### 问题 2: 数据库权限错误

**症状**:
```
ERROR Database error: unable to open database file
```

**修复**:
```bash
# 检查文件权限
ls -la /var/lib/actrix/*.db

# 修复权限
sudo chown actrix:actrix /var/lib/actrix/*.db
sudo chmod 600 /var/lib/actrix/*.db

# 检查目录权限
sudo chmod 750 /var/lib/actrix
```

#### 问题 3: TLS 证书错误

**症状**:
```
ERROR Failed to load TLS certificate: No such file or directory
```

**修复**:
```bash
# 检查证书文件存在
ls -la /etc/actrix/certs/

# 检查证书有效期
openssl x509 -in /etc/actrix/certs/server.crt -noout -dates

# 测试证书和私钥匹配
openssl x509 -noout -modulus -in server.crt | openssl md5
openssl rsa -noout -modulus -in server.key | openssl md5
# 两个 MD5 值应该相同
```

#### 问题 4: OpenTelemetry 连接失败

**症状**:
```
WARN Failed to export traces: connection refused
```

**排查**:
```bash
# 检查 Jaeger 是否运行
docker ps | grep jaeger

# 测试连接
curl http://localhost:4318/v1/traces

# 检查配置
grep -A 3 "\[tracing\]" /etc/actrix/config.toml
```

### 9.2 性能问题

#### 问题 1: TURN 认证缓慢

**排查**:
```rust
// 在代码中添加日志
let (size, capacity) = Authenticator::cache_stats();
info!("TURN auth cache: {}/{} ({}%)",
      size, capacity, (size * 100) / capacity);
```

**优化**:
```rust
// crates/turn/src/authenticator.rs:24
// 增加缓存容量
let capacity = NonZeroUsize::new(5000).unwrap();  // 从 1000 增加到 5000
```

#### 问题 2: KS 密钥查询缓慢

**排查**:
```bash
# 检查数据库大小
ls -lh /var/lib/actrix/ks.db

# 检查过期密钥数量
sqlite3 /var/lib/actrix/ks.db "
  SELECT COUNT(*) FROM keys
  WHERE expires_at > 0 AND expires_at < strftime('%s', 'now');
"
```

**修复**:
```bash
# 手动清理过期密钥
sqlite3 /var/lib/actrix/ks.db "
  DELETE FROM keys
  WHERE expires_at > 0 AND expires_at < strftime('%s', 'now');
"

# 重建索引
sqlite3 /var/lib/actrix/ks.db "
  REINDEX idx_keys_expires_at;
"
```

### 9.3 日志分析

#### 启动成功的日志序列

```
INFO Starting Actrix v0.1.0
INFO Instance: actrix-01, Environment: prod
INFO Adding service 'KS' to manager
INFO Adding service 'STUN' to manager
INFO Adding service 'TURN' to manager
INFO Starting all 3 types (Ks, Stun, Turn) services
INFO Starting HTTPS server with 1 route services (environment: prod)
INFO Nesting route 'KS' at prefix '/ks'
INFO HTTPS server will bind to: 0.0.0.0:8443
INFO Starting ICE service: STUN
INFO STUN service bound to 0.0.0.0:3478
INFO Starting ICE service: TURN
INFO TURN service bound to 0.0.0.0:3478
INFO All services started successfully
```

#### 关键日志模式

```bash
# 查找错误
journalctl -u actrix | grep ERROR

# 查找认证失败
journalctl -u actrix | grep "Authentication error"

# 查找数据库错误
journalctl -u actrix | grep "Database error"

# 统计请求数
journalctl -u actrix | grep "HTTP POST /ks/generate" | wc -l
```

---

## 🎯 总结

本文档覆盖了 Actrix 服务管理的所有方面:

- ✅ **服务架构** - HTTP vs ICE,trait 设计
- ✅ **ServiceManager** - 服务启动、关闭、监控
- ✅ **配置控制** - 位掩码、配置文件
- ✅ **生产部署** - systemd, Docker, 目录结构
- ✅ **故障排查** - 常见问题和解决方案

**相关文档**:
- [ARCHITECTURE.md](./ARCHITECTURE.md) - 整体架构
- [CRATES.md](./CRATES.md) - Crate 详细文档
- [API.md](./API.md) - API 参考 (待创建)
- [CONFIGURATION.md](./CONFIGURATION.md) - 配置参考 (待更新)
- [install/README.md](../install/README.md) - 部署指南

**最后验证时间**: 2025-11-03
**代码版本**: v0.1.0+enhancements
