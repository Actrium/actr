# Actrix 负载测试与性能基准

## 工具选择

### wrk - HTTP 基准测试
```bash
# 安装
sudo apt install wrk  # Ubuntu
brew install wrk      # macOS
```

### k6 - 现代负载测试
```bash
# 安装
brew install k6      # macOS
# https://k6.io/docs/getting-started/installation/
```

## 基准测试场景

### 1. Health Check 基准
```bash
# wrk 测试
wrk -t4 -c100 -d30s --latency https://localhost:8443/ks/health

# 预期结果:
# - RPS: > 10,000
# - P99 延迟: < 10ms
# - 错误率: 0%
```

### 2. Metrics Endpoint 基准
```bash
wrk -t4 -c100 -d30s --latency https://localhost:8443/metrics

# 预期结果:
# - RPS: > 5,000
# - P99 延迟: < 20ms
```

### 3. KS gRPC 密钥生成基准
使用 `ghz` (gRPC benchmarking tool):
```bash
# 安装 ghz
go install github.com/bojand/ghz/cmd/ghz@latest

# 测试
ghz --insecure \
  --proto crates/ks/proto/keyserver.proto \
  --call ks.v1.KeyServer/GenerateKey \
  -d '{"credential":{"timestamp":1699350000,"nonce":"test","signature":"test"}}' \
  -c 50 -n 10000 \
  localhost:50052

# 预期结果:
# - RPS: > 1,000
# - P99 延迟: < 50ms
```

### 4. AIS Token 签发基准
```bash
# 详见 benchmarks/ais_token_issuance.sh
bash benchmarks/ais_token_issuance.sh

# 预期结果:
# - RPS: > 500
# - P99 延迟: < 100ms
```

## K6 脚本

### 综合负载测试
```bash
k6 run benchmarks/k6/load_test.js
```

### 压力测试
```bash
k6 run benchmarks/k6/stress_test.js
```

### 浸泡测试（Soak Test）
```bash
k6 run benchmarks/k6/soak_test.js
```

## 性能目标

| 指标 | 目标值 | 当前值 | 状态 |
|------|--------|--------|------|
| Health Check RPS | > 10,000 | TBD | ⏳ |
| Health Check P99 | < 10ms | TBD | ⏳ |
| KS Generate Key RPS | > 1,000 | TBD | ⏳ |
| KS Generate P99 | < 50ms | TBD | ⏳ |
| AIS Token Issue RPS | > 500 | TBD | ⏳ |
| AIS Token P99 | < 100ms | TBD | ⏳ |
| TURN Allocation P99 | < 20ms | TBD | ⏳ |
| Memory Usage (idle) | < 50MB | TBD | ⏳ |
| Memory Usage (load) | < 500MB | TBD | ⏳ |

## 如何运行

### 1. 启动服务
```bash
cargo build --release
./target/release/actrix --config config.toml
```

### 2. 预热（Warmup）
```bash
bash benchmarks/warmup.sh
```

### 3. 运行基准测试
```bash
bash benchmarks/run_all.sh
```

### 4. 查看结果
```bash
cat benchmarks/results/$(date +%Y%m%d)/summary.txt
```

## 报告格式

测试结果保存在 `benchmarks/results/YYYYMMDD/`:
- `summary.txt` - 汇总结果
- `wrk_*.txt` - wrk 详细输出
- `k6_*.json` - k6 JSON 结果
- `metrics.html` - 可视化报告

## 持续监控

集成到 CI/CD:
```yaml
# .github/workflows/benchmark.yml
on:
  push:
    branches: [ main ]
  schedule:
    - cron: '0 2 * * *'  # 每天凌晨2点

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - run: bash benchmarks/run_all.sh
      - uses: benchmark-action/github-action-benchmark@v1
```

---

## Jaeger 分布式追踪

### 快速启动

```bash
# 启动 Jaeger
docker-compose -f benchmarks/jaeger-compose.yml up -d

# 查看日志
docker-compose -f benchmarks/jaeger-compose.yml logs -f

# 停止 Jaeger
docker-compose -f benchmarks/jaeger-compose.yml down
```

### 访问 Jaeger UI

启动后访问：http://localhost:16686

### 配置 Actrix

在 `config.toml` 中启用 OpenTelemetry 追踪：

```toml
[tracing]
enable = true
service_name = "actrix"
endpoint = "http://127.0.0.1:4317"
```

### 端口说明

- **4317**: OTLP gRPC 接收端口（actrix 连接）
- **16686**: Jaeger UI 端口（浏览器访问）
