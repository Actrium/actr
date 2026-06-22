# Actrix 性能基准线

**最后更新**: 2025-11-07
**测试环境**: 未建立
**硬件**: TBD

---

## 目标值（Target）

| 服务 | 操作 | RPS目标 | P99延迟目标 | 内存目标 |
|------|------|---------|-------------|----------|
| HTTP | Health Check | > 10,000 | < 10ms | - |
| HTTP | Metrics | > 5,000 | < 20ms | - |
| KS gRPC | GenerateKey | > 1,000 | < 50ms | < 100MB |
| KS gRPC | GetSecretKey | > 2,000 | < 30ms | - |
| AIS HTTP | Token Issue | > 500 | < 100ms | < 200MB |
| AIS HTTP | Token Validate | > 1,000 | < 50ms | - |
| TURN UDP | Allocation | > 10,000 | < 20ms | < 500MB |

---

## 当前基准（Current Baseline）

### 2025-11-07 - 初始基准（待建立）

**环境**:
- CPU: TBD
- RAM: TBD
- OS: TBD
- Rust: TBD

**结果**:
```
⏳ 尚未运行基准测试

运行命令: bash benchmarks/run_all.sh
```

---

## 历史趋势

### 版本对比

| 版本 | Date | Health RPS | KS Gen RPS | AIS Issue RPS | 备注 |
|------|------|------------|------------|---------------|------|
| 0.1.0 | 2025-11-07 | TBD | TBD | TBD | 初始版本 |
| - | - | - | - | - | - |

---

## 性能回归检测

### CI 集成

```yaml
# .github/workflows/benchmark.yml
name: Benchmark
on:
  push:
    branches: [ main ]
  pull_request:

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo build --release
      - run: bash benchmarks/run_all.sh
      - name: Compare with baseline
        run: |
          python3 benchmarks/compare.py \
            benchmarks/results/latest/summary.json \
            benchmarks/BASELINE.json
```

### 回归阈值

- RPS 下降 > 20% → ⚠️ 警告
- P99 延迟增加 > 30% → ⚠️ 警告
- 内存增加 > 50% → ⚠️ 警告
- 错误率 > 1% → ❌ 失败

---

## 优化历史

### 已实施的优化

1. **2025-11-07** - 添加 Prometheus metrics (预期性能影响 < 5%)

### 计划中的优化

- [ ] 数据库连接池优化
- [ ] gRPC 客户端连接复用
- [ ] Token 缓存策略改进
- [ ] 内存分配器切换（jemalloc）

---

## 测试方法

### 1. 单次运行
```bash
bash benchmarks/run_all.sh
```

### 2. 压力测试
```bash
# 运行 10 分钟高负载
k6 run --duration 10m --vus 500 benchmarks/k6/stress_test.js
```

### 3. 浸泡测试（Soak Test）
```bash
# 运行 24 小时稳定负载
k6 run --duration 24h --vus 100 benchmarks/k6/soak_test.js
```

---

## 性能调优清单

### OS 级别
- [ ] 增加文件描述符限制 (`ulimit -n 65535`)
- [ ] 调整 TCP 参数 (`net.ipv4.tcp_tw_reuse=1`)
- [ ] 禁用透明大页 (`echo never > /sys/kernel/mm/transparent_hugepage/enabled`)

### 应用级别
- [ ] 启用 release 模式编译
- [ ] 配置合适的线程池大小
- [ ] 启用 CPU 亲和性绑定
- [ ] 使用 jemalloc 替换默认分配器

### 数据库级别
- [ ] 配置连接池 (min=5, max=50)
- [ ] 启用 WAL 模式（SQLite）
- [ ] 定期 VACUUM

---

## 参考资料

- [wrk 使用指南](https://github.com/wg/wrk)
- [k6 文档](https://k6.io/docs/)
- [ghz (gRPC benchmark)](https://ghz.sh/)
- [Rust 性能优化](https://nnethercote.github.io/perf-book/)
