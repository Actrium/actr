# Actrix Docker 部署指南

本文档介绍如何使用 Docker Compose 部署 Actrix 服务。

---

## 快速开始

### 1. 准备配置文件

首先使用 deploy 工具生成配置文件：

```bash
# 使用交互式向导生成配置
cargo run -p deploy -- config

# 或指定输出路径
cargo run -p deploy -- config --output config.toml
```

### 2. 生成 Docker Compose 配置

使用 deploy 工具自动生成 `docker-compose.yml`：

```bash
# 从配置文件生成 docker-compose.yml
cargo run -p deploy -- docker -c config.toml -o docker-compose.yml

# 生成并立即启动（需要 Docker 环境）
cargo run -p deploy -- docker -c config.toml --run

# 使用 docker-compose（旧版本）而非 docker compose
cargo run -p deploy -- docker -c config.toml --legacy
```

### 3. 启动服务

```bash
# 启动所有服务（后台运行）
docker compose up -d

# 查看日志
docker compose logs -f actrix

# 停止服务
docker compose down

# 停止并删除卷
docker compose down -v
```

---

## 配置说明

### 生成的 docker-compose.yml 结构

deploy 工具会根据 `config.toml` 自动生成以下配置：

```yaml
version: '3.8'

services:
  actrix:
    image: actrix:latest
    container_name: actrix
    restart: unless-stopped

    # 端口映射（根据配置自动生成）
    ports:
      - "8080:8080"           # HTTP（如果启用）
      - "8443:8443"           # HTTPS（如果启用）
      - "3478:3478/udp"       # STUN/TURN
      - "49152-65535:49152-65535/udp"  # TURN relay（如果启用）

    # 环境变量
    environment:
      - ACTRIX_KEK=${ACTRIX_KEK}  # KEK 从环境变量读取

    # 卷挂载
    volumes:
      - ./config.toml:/app/config.toml:ro
      - actrix-data:/app/data
      - actrix-certs:/app/certificates:ro

    # 网络
    networks:
      - actrix-network

    # 启动命令
    command: ["--config", "/app/config.toml"]

networks:
  actrix-network:
    driver: bridge

volumes:
  actrix-data: {}      # 持久化数据（数据库等）
  actrix-certs: {}     # TLS 证书
```

---

## 端口映射规则

deploy 工具根据 `config.toml` 自动映射端口：

| 服务 | 端口 | 协议 | 条件 |
|------|------|------|------|
| HTTP API | `bind.http.port` | TCP | 如果配置了 `bind.http` |
| HTTPS API | `bind.https.port` | TCP | 如果配置了 `bind.https` |
| STUN/TURN | `bind.ice.port` | UDP | 始终包含 |
| TURN Relay | `turn.relay_port_range` | UDP | 如果启用了 TURN 服务 |

---

## 环境变量

### KEK（密钥加密密钥）

如果使用 KEK 加密 KS 私钥，需要设置环境变量：

```bash
# 方式 1：在 .env 文件中
echo "ACTRIX_KEK=your-32-byte-hex-key" > .env

# 方式 2：export 导出
export ACTRIX_KEK=$(openssl rand -hex 32)

# 方式 3：docker-compose.yml 中直接配置
# environment:
#   - ACTRIX_KEK=your-32-byte-hex-key
```

**安全建议**：生产环境使用 Docker Secrets 或外部密钥管理服务。

---

## 卷管理

### 数据持久化

```bash
# 备份数据库
docker compose exec actrix tar czf /backup.tar.gz /app/data
docker compose cp actrix:/backup.tar.gz ./backup-$(date +%Y%m%d).tar.gz

# 恢复数据库
docker compose cp ./backup.tar.gz actrix:/backup.tar.gz
docker compose exec actrix tar xzf /backup.tar.gz -C /app
```

### 证书管理

将 TLS 证书放入 `actrix-certs` 卷或使用本地目录挂载：

```yaml
volumes:
  - ./certificates:/app/certificates:ro  # 推荐：使用本地目录
```

---

## 构建 Docker 镜像

deploy 工具只生成 `docker-compose.yml`，镜像需要单独构建。

### 使用项目根目录的 Dockerfile

```bash
# 构建镜像
docker build -t actrix:latest -f docker/Dockerfile .

# 或使用 Makefile
make docker-build
```

### 多阶段构建示例

参考 `docker/Dockerfile`：

```dockerfile
# 构建阶段
FROM rust:1.88 as builder
WORKDIR /build
COPY . .
RUN cargo build --release

# 运行阶段
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/actrix /usr/local/bin/
WORKDIR /app
ENTRYPOINT ["actrix"]
```

---

## 高级配置

### 自定义网络

```yaml
networks:
  actrix-network:
    driver: bridge
    ipam:
      config:
        - subnet: 172.28.0.0/16
```

### 健康检查

```yaml
services:
  actrix:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s
```

### 资源限制

```yaml
services:
  actrix:
    deploy:
      resources:
        limits:
          cpus: '2'
          memory: 2G
        reservations:
          cpus: '1'
          memory: 512M
```

### 多实例部署

```yaml
services:
  actrix-1:
    <<: *actrix-common
    container_name: actrix-1
    ports:
      - "8443:8443"
      - "3478:3478/udp"

  actrix-2:
    <<: *actrix-common
    container_name: actrix-2
    ports:
      - "8444:8443"
      - "3479:3478/udp"
```

---

## 故障排查

### 查看日志

```bash
# 实时日志
docker compose logs -f actrix

# 最近 100 行
docker compose logs --tail=100 actrix

# 只看错误
docker compose logs actrix 2>&1 | grep -i error
```

### 进入容器调试

```bash
# 进入运行中的容器
docker compose exec actrix /bin/bash

# 检查配置
docker compose exec actrix cat /app/config.toml

# 检查进程
docker compose exec actrix ps aux | grep actrix
```

### 端口占用问题

```bash
# 检查端口占用
docker compose ps
netstat -tulnp | grep -E '3478|8443'

# 修改端口映射
# 编辑 docker-compose.yml，改为：
ports:
  - "18443:8443"  # 主机端口改为 18443
```

### 重新生成配置

```bash
# 删除旧配置
rm docker-compose.yml

# 重新生成
cargo run -p deploy -- docker -c config.toml
```

---

## 生产部署建议

### 1. 安全加固

```yaml
services:
  actrix:
    # 非 root 用户运行
    user: "1000:1000"

    # 只读根文件系统
    read_only: true

    # 临时目录
    tmpfs:
      - /tmp

    # 限制能力
    cap_drop:
      - ALL
    cap_add:
      - NET_BIND_SERVICE  # 只保留绑定端口能力

    # 安全选项
    security_opt:
      - no-new-privileges:true
```

### 2. 日志配置

```yaml
services:
  actrix:
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"
```

### 3. 自动重启策略

```yaml
services:
  actrix:
    restart: unless-stopped  # 推荐
    # 或者
    # restart: always
    # restart: on-failure:3
```

### 4. 使用 Docker Secrets

```yaml
secrets:
  actrix_kek:
    file: ./secrets/kek.txt

services:
  actrix:
    secrets:
      - actrix_kek
```

### 5. 监控集成

```yaml
services:
  actrix:
    labels:
      - "prometheus.scrape=true"
      - "prometheus.port=8080"
      - "prometheus.path=/metrics"
```

---

## 命令行参考

### deploy docker 命令选项

```bash
# 基本用法
deploy docker [OPTIONS]

选项:
  -c, --config <CONFIG>   配置文件路径 [默认: config.toml]
  -o, --output <OUTPUT>   输出文件路径 [默认: docker-compose.yml]
      --run               生成后自动执行 docker compose up -d
      --legacy            使用 docker-compose 命令（旧版本）
      --debug             启用调试模式
  -h, --help              显示帮助信息
```

### 示例

```bash
# 1. 生成配置
deploy docker

# 2. 指定配置文件
deploy docker -c /etc/actrix/config.toml

# 3. 指定输出路径
deploy docker -o production-compose.yml

# 4. 生成并立即启动
deploy docker --run

# 5. 使用 docker-compose（旧版）
deploy docker --legacy --run
```

---

## 与其他部署方式对比

| 特性 | Docker Compose | Systemd | Kubernetes |
|------|---------------|---------|------------|
| 易用性 | ⭐⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐ |
| 隔离性 | ⭐⭐⭐⭐ | ⭐⭐ | ⭐⭐⭐⭐⭐ |
| 可扩展性 | ⭐⭐⭐ | ⭐⭐ | ⭐⭐⭐⭐⭐ |
| 资源占用 | ⭐⭐⭐ | ⭐⭐⭐⭐⭐ | ⭐⭐ |
| 适用场景 | 开发/测试/小规模 | 生产/单机 | 大规模/集群 |

---

## 相关文档

- [配置指南](./CONFIGURATION.md) - 详细配置说明
- [部署指南](../install/README.md) - Systemd 部署方式
- [Dockerfile](../docker/Dockerfile) - Docker 镜像构建
- [API 文档](./API.md) - HTTP API 接口

---

## 常见问题

**Q: 为什么端口范围很大（49152-65535）？**
A: 这是 TURN relay 端口范围，用于 NAT 穿透。可以在 `config.toml` 中调整 `turn.relay_port_range`。

**Q: 如何更新配置？**
A: 修改 `config.toml` 后重启容器：`docker compose restart actrix`

**Q: 数据库在哪里？**
A: SQLite 数据库存储在 `actrix-data` 卷中的 `/app/data/database.db`。

**Q: 如何切换到 PostgreSQL？**
A: 目前仅支持 SQLite。PostgreSQL 支持在开发中。

**Q: 可以使用 Docker Swarm 吗？**
A: 可以，但需要调整 `deploy:` 配置。参考 Docker Swarm 文档。

---

**最后更新**: 2025-11-07
**版本**: v0.1.0
