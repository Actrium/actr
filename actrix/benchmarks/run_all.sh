#!/bin/bash
# Actrix 完整性能基准测试套件

set -e

RESULTS_DIR="benchmarks/results/$(date +%Y%m%d_%H%M%S)"
mkdir -p "$RESULTS_DIR"

echo "🚀 Actrix 性能基准测试"
echo "结果目录: $RESULTS_DIR"
echo

# 检查服务是否运行
if ! curl -k -s https://localhost:8443/ks/health > /dev/null 2>&1; then
    echo "❌ 服务未运行，请先启动 actrix"
    exit 1
fi

echo "✅ 服务运行正常"
echo

# === 1. Health Check 基准 ===
echo "1️⃣  Health Check 基准测试..."
if command -v wrk &> /dev/null; then
    wrk -t4 -c100 -d30s --latency \
        https://localhost:8443/ks/health \
        > "$RESULTS_DIR/wrk_health.txt" 2>&1

    echo "  ✅ 完成"
    grep "Requests/sec" "$RESULTS_DIR/wrk_health.txt"
    grep "Latency" "$RESULTS_DIR/wrk_health.txt" | head -3
else
    echo "  ⚠️  wrk 未安装，跳过"
fi
echo

# === 2. Metrics Endpoint 基准 ===
echo "2️⃣  Metrics Endpoint 基准测试..."
if command -v wrk &> /dev/null; then
    wrk -t4 -c100 -d30s --latency \
        https://localhost:8443/metrics \
        > "$RESULTS_DIR/wrk_metrics.txt" 2>&1

    echo "  ✅ 完成"
    grep "Requests/sec" "$RESULTS_DIR/wrk_metrics.txt"
else
    echo "  ⚠️  wrk 未安装，跳过"
fi
echo

# === 3. KS gRPC 基准 ===
echo "3️⃣  KS gRPC 基准测试..."
if command -v ghz &> /dev/null; then
    cd crates/ks
    ghz --insecure \
        --proto proto/keyserver.proto \
        --call ks.v1.KeyServer/HealthCheck \
        -d '{"service":"ks"}' \
        -c 50 -n 10000 \
        localhost:50052 \
        > "$RESULTS_DIR/ghz_ks_health.txt" 2>&1
    cd ../..

    echo "  ✅ 完成"
    grep "Requests/sec" "$RESULTS_DIR/ghz_ks_health.txt" || true
else
    echo "  ⚠️  ghz 未安装，跳过"
    echo "     安装: go install github.com/bojand/ghz/cmd/ghz@latest"
fi
echo

# === 4. K6 负载测试 ===
echo "4️⃣  K6 负载测试..."
if command -v k6 &> /dev/null; then
    if [ -f "benchmarks/k6/load_test.js" ]; then
        k6 run benchmarks/k6/load_test.js \
            --out json="$RESULTS_DIR/k6_load.json"
        echo "  ✅ 完成"
    else
        echo "  ⚠️  K6 脚本不存在"
    fi
else
    echo "  ⚠️  k6 未安装，跳过"
    echo "     安装: https://k6.io/docs/getting-started/installation/"
fi
echo

# === 5. 生成汇总报告 ===
echo "5️⃣  生成汇总报告..."
cat > "$RESULTS_DIR/summary.txt" <<EOF
Actrix 性能基准测试报告
=====================
测试时间: $(date)
Git Commit: $(git rev-parse --short HEAD 2>/dev/null || echo "N/A")

EOF

if [ -f "$RESULTS_DIR/wrk_health.txt" ]; then
    echo "Health Check 性能:" >> "$RESULTS_DIR/summary.txt"
    grep "Requests/sec" "$RESULTS_DIR/wrk_health.txt" >> "$RESULTS_DIR/summary.txt"
    grep "99%" "$RESULTS_DIR/wrk_health.txt" >> "$RESULTS_DIR/summary.txt"
    echo "" >> "$RESULTS_DIR/summary.txt"
fi

if [ -f "$RESULTS_DIR/wrk_metrics.txt" ]; then
    echo "Metrics Endpoint 性能:" >> "$RESULTS_DIR/summary.txt"
    grep "Requests/sec" "$RESULTS_DIR/wrk_metrics.txt" >> "$RESULTS_DIR/summary.txt"
    echo "" >> "$RESULTS_DIR/summary.txt"
fi

echo "  ✅ 完成"
echo

# === 显示汇总 ===
echo "📊 测试汇总:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cat "$RESULTS_DIR/summary.txt"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo
echo "✅ 所有测试完成"
echo "详细结果: $RESULTS_DIR/"
echo
echo "查看汇总: cat $RESULTS_DIR/summary.txt"
