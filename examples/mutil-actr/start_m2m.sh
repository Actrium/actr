#!/bin/bash
# mutil-actr 多对多并发测试脚本 - 使用 actrix 作为信令服务器
# 测试多个客户端并发向多个服务器发送消息
#
# 用法:
#   ./start_mutilclient2mutilserver.sh              # 启动 3 个客户端和 2 个服务器（默认）
#   ./start_mutilclient2mutilserver.sh 5 3          # 启动 5 个客户端和 3 个服务器

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 测试 mutil-actr (多客户端对多服务器并发测试)"
echo "    使用 Actrix 作为信令服务器"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 获取并发客户端和服务器数量（默认 3 个客户端，2 个服务器）
NUM_CLIENTS=${1:-3}
NUM_SERVERS=${2:-2}
echo "📊 将启动 $NUM_CLIENTS 个并发客户端和 $NUM_SERVERS 个服务器"

# Determine paths and switch to workspace root
WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
ACTRIX_CONFIG="$(cd "$(dirname "$0")" && pwd)/actrix-config.toml"
ECHO_SERVER_DIR="$WORKSPACE_ROOT/mutil-actr/server"
ECHO_CLIENT_DIR="$WORKSPACE_ROOT/mutil-actr/client"
PROTO_DIR="$WORKSPACE_ROOT/mutil-actr/proto"

# Switch to workspace root and stay there
cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required CLI tools
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"

# Ensure Actr.toml files exist
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# Ensure Actr.toml files exist for server and client
echo ""
echo "🔍 检查 Actr.toml 文件..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$ECHO_SERVER_DIR"
ensure_actr_toml "$ECHO_CLIENT_DIR"

# Ensure actrix-config.toml exists
ensure_actrix_config "$WORKSPACE_ROOT"

echo ""
echo "🧰 检查必需的 CLI 工具..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Cleanup function
cleanup() {
    echo ""
    echo "🧹 清理中..."

    # Kill actrix
    if [ ! -z "$ACTRIX_PID" ]; then
        echo "停止 actrix (PID: $ACTRIX_PID)"
        kill -9 $ACTRIX_PID 2>/dev/null || true
    fi

    # Kill all echo servers
    if [ ${#SERVER_PIDS[@]} -gt 0 ]; then
        echo "停止 ${#SERVER_PIDS[@]} 个服务器..."
        for pid in "${SERVER_PIDS[@]}"; do
            kill -9 $pid 2>/dev/null || true
        done
    fi

    # Kill all client apps
    if [ ${#CLIENT_PIDS[@]} -gt 0 ]; then
        echo "停止 ${#CLIENT_PIDS[@]} 个客户端..."
        for pid in "${CLIENT_PIDS[@]}"; do
            kill -9 $pid 2>/dev/null || true
        done
    fi

    # Wait briefly for processes to exit
    sleep 1

    echo "✅ 清理完成"
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Step 0: Generate server code (protobuf + actor glue)
echo ""
echo "🛠️ 生成服务器代码 (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_GEN_CMD=""

if command -v actr > /dev/null 2>&1; then
    ACTR_GEN_CMD="actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/debug/actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/release/actr"
else
    echo -e "${RED}❌ 未找到 actr 生成器 (期望在 PATH 中找到 'actr' 或在 $ACTOR_RTC_DIR/actr 下构建)${NC}"
    exit 1
fi

if [ ! -d "$PROTO_DIR" ]; then
    echo -e "${RED}❌ 在 $PROTO_DIR 未找到 Proto 目录${NC}"
    exit 1
fi

cd "$ECHO_SERVER_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-echo-server.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen 失败${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"

echo -e "${GREEN}✅ actr gen 完成 (服务器代码已刷新)${NC}"

# Step 0b: Generate client code (protobuf + actor glue)
echo ""
echo "🛠️ 生成客户端代码 (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$ECHO_CLIENT_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-echo-client.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen 失败${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"

echo -e "${GREEN}✅ actr gen 完成 (客户端代码已刷新)${NC}"

# Step 1: Build and install actrix
echo ""
echo "📦 构建并安装 actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if actrix directory exists
if [ ! -d "$ACTRIX_DIR" ]; then
    echo -e "${RED}❌ 在 $ACTRIX_DIR 找不到 actrix 目录${NC}"
    echo "请确保 actrix 项目存在于: $ACTRIX_DIR"
    exit 1
fi

# Build actrix with opentelemetry feature
echo "从源代码构建 actrix (使用 opentelemetry 功能)..."
cd "$ACTRIX_DIR"
cargo build --features opentelemetry 2>&1 | grep -v "^warning:" || true

# Check if build was successful
if [ ! -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    echo -e "${RED}❌ 构建 actrix 失败${NC}"
    exit 1
fi

# Copy to ~/.cargo/bin
echo "安装 actrix 到 ~/.cargo/bin..."
mkdir -p ~/.cargo/bin
cp "$ACTRIX_DIR/target/debug/actrix" ~/.cargo/bin/actrix
chmod +x ~/.cargo/bin/actrix

# Return to workspace root
cd "$WORKSPACE_ROOT"

# Verify actrix is available in PATH
if ! command -v actrix > /dev/null 2>&1; then
    echo -e "${RED}❌ 安装后在 PATH 中找不到 actrix 命令${NC}"
    echo "请确保 ~/.cargo/bin 在你的 PATH 中"
    exit 1
fi

ACTRIX_CMD="actrix"
echo -e "${GREEN}✅ Actrix 已构建并安装: $(which actrix)${NC}"

# Step 2: Start actrix
echo ""
echo "🚀 启动 actrix (信令服务器)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

$ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "Actrix 已启动 (PID: $ACTRIX_PID)"
echo "等待 actrix 准备就绪..."

# Wait for actrix to start and verify it's listening on port 8081
MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix 启动失败${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi
    
    # Check if port 8081 is listening (actrix WebSocket server)
    if lsof -i:8081 > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null; then
        echo -e "${GREEN}✅ Actrix 正在运行并监听端口 8081${NC}"
        break
    fi
    
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix 在 ${MAX_WAIT} 秒后未监听端口 8081${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# Step 2.5: Setting up realms in actrix
echo ""
echo "🔑 在 actrix 中设置 realms..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait a bit for supervisord gRPC service to be ready (port 50055)
sleep 2

# Build realm-setup tool if needed
if ! cargo build -p realm-setup 2>&1 | tail -5; then
    echo -e "${RED}❌ 构建 realm-setup 工具失败${NC}"
    exit 1
fi

# Run realm-setup with Actr.toml files from server and client
REALM_SETUP_OUTPUT="$LOG_DIR/realm-setup.log"
if ! cargo run -p realm-setup -- \
    --actrix-config "$ACTRIX_CONFIG" \
    --actr-toml "$ECHO_SERVER_DIR/Actr.toml" \
    --actr-toml "$ECHO_CLIENT_DIR/Actr.toml" \
    > "$REALM_SETUP_OUTPUT" 2>&1; then
    echo -e "${RED}❌ 在 actrix 中设置 realms 失败${NC}"
    cat "$REALM_SETUP_OUTPUT"
    exit 1
fi

echo -e "${GREEN}✅ Realms 设置完成${NC}"
cat "$REALM_SETUP_OUTPUT" | grep -E "(Created|Skipped|Found)" || true

# Step 3: 启动多个服务器
echo ""
echo "🚀 启动 $NUM_SERVERS 个服务器..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 数组存储所有服务器进程 ID
SERVER_PIDS=()

# 并发启动多个服务器
for i in $(seq 1 $NUM_SERVERS); do
    echo "启动服务器 #$i..."
    RUST_LOG="${RUST_LOG:-info}" cargo run --bin mutil_server > "$LOG_DIR/mutil-actr-server-$i.log" 2>&1 &
    SERVER_PIDS+=($!)
    # 稍微错开启动时间
    sleep 1
done

echo "已启动 ${#SERVER_PIDS[@]} 个服务器进程："
for i in "${!SERVER_PIDS[@]}"; do
    echo "  服务器 #$((i+1)): PID ${SERVER_PIDS[$i]}"
done

# 等待所有服务器注册
echo ""
echo "⏳ 等待所有服务器注册到信令服务器..."
MAX_WAIT=15
REGISTERED_COUNT=0

for i in $(seq 1 $NUM_SERVERS); do
    LOG_FILE="$LOG_DIR/mutil-actr-server-$i.log"
    COUNTER=0
    
    while [ $COUNTER -lt $MAX_WAIT ]; do
        if ! kill -0 ${SERVER_PIDS[$((i-1))]} 2>/dev/null; then
            echo -e "${RED}❌ 服务器 #$i 启动失败${NC}"
            cat "$LOG_FILE"
            exit 1
        fi
        
        if grep -q "ActrNode 启动成功\|Echo Server 已完全启动并注册\|ActrNode started" "$LOG_FILE" 2>/dev/null; then
            echo -e "${GREEN}✅ 服务器 #$i 已注册${NC}"
            REGISTERED_COUNT=$((REGISTERED_COUNT + 1))
            break
        fi
        
        sleep 1
        COUNTER=$((COUNTER + 1))
    done
    
    if [ $COUNTER -eq $MAX_WAIT ]; then
        echo -e "${YELLOW}⚠️  服务器 #$i 可能未完全注册，但继续...${NC}"
    fi
done

echo "已注册服务器数: $REGISTERED_COUNT / $NUM_SERVERS"

# 额外等待时间确保所有服务器就绪
sleep 2

# Step 4: 启动多个并发客户端
echo ""
echo "🚀 启动 $NUM_CLIENTS 个并发客户端..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 数组存储所有客户端进程 ID
CLIENT_PIDS=()

# 并发启动多个客户端
for i in $(seq 1 $NUM_CLIENTS); do
    echo "启动客户端 #$i..."
    RUST_LOG="${RUST_LOG:-info}" cargo run --bin mutil_client > "$LOG_DIR/mutil-actr-client-$i.log" 2>&1 &
    CLIENT_PIDS+=($!)
    # 稍微错开启动时间，避免同时连接
    sleep 0.5
done

echo "已启动 ${#CLIENT_PIDS[@]} 个客户端进程："
for i in "${!CLIENT_PIDS[@]}"; do
    echo "  客户端 #$((i+1)): PID ${CLIENT_PIDS[$i]}"
done

# 等待所有客户端完成（最多 20 秒）
echo ""
echo "⏳ 等待所有客户端完成..."
MAX_WAIT=20
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    # 检查是否还有客户端在运行
    RUNNING=0
    for pid in "${CLIENT_PIDS[@]}"; do
        if kill -0 $pid 2>/dev/null; then
            RUNNING=$((RUNNING + 1))
        fi
    done
    
    if [ $RUNNING -eq 0 ]; then
        echo "✅ 所有客户端已完成"
        break
    fi
    
    sleep 1
    COUNTER=$((COUNTER + 1))
done

# 强制终止仍在运行的客户端
if [ $RUNNING -gt 0 ]; then
    echo -e "${YELLOW}⚠️  $RUNNING 个客户端仍在运行，强制终止...${NC}"
    for pid in "${CLIENT_PIDS[@]}"; do
        kill $pid 2>/dev/null || true
    done
fi

# Step 5: 显示和验证输出
echo ""
echo "🔍 验证输出..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

SUCCESS_COUNT=0
FAIL_COUNT=0

for i in $(seq 1 $NUM_CLIENTS); do
    LOG_FILE="$LOG_DIR/mutil-actr-client-$i.log"
    
    # 检查响应是否成功
    if grep -q "✅ 成功！响应匹配当前客户端" "$LOG_FILE" && grep -q "✓ 验证通过" "$LOG_FILE"; then
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
        echo -e "${GREEN}✅ 客户端 #$i: 通过${NC}"
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        echo -e "${RED}❌ 客户端 #$i: 失败${NC}"
    fi
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 测试结果汇总"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  服务器数: $NUM_SERVERS"
echo "  客户端数: $NUM_CLIENTS"
echo "  成功: $SUCCESS_COUNT"
echo "  失败: $FAIL_COUNT"
echo ""

if [ $SUCCESS_COUNT -eq $NUM_CLIENTS ]; then
    echo -e "${GREEN}🎉 测试完全成功！${NC}"
    echo ""
    echo "✅ 已验证:"
    echo "   • $NUM_SERVERS 个服务器同时运行"
    echo "   • $NUM_CLIENTS 个客户端并发调用"
    echo "   • 每个客户端收到正确的响应"
    echo "   • 响应包含对应的客户端 ID"
    echo "   • 使用 actrix 作为信令服务器"
    echo ""
    
    # 显示所有成功客户端的输出
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📝 所有客户端输出:"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    for i in $(seq 1 $NUM_CLIENTS); do
        LOG_FILE="$LOG_DIR/mutil-actr-client-$i.log"
        if grep -q "✅ 成功！响应匹配当前客户端" "$LOG_FILE"; then
            echo ""
            echo "客户端 #$i:"
            cat "$LOG_FILE" | grep -A 6 "✅ 成功" || true
        fi
    done
    echo ""
    
    # 显示服务器统计信息
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "📝 服务器处理统计:"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    for i in $(seq 1 $NUM_SERVERS); do
        LOG_FILE="$LOG_DIR/mutil-actr-server-$i.log"
        if [ -f "$LOG_FILE" ]; then
            # 尝试多种模式来匹配请求
            REQUEST_COUNT=$(grep -E "Received Echo request|📨.*Echo|处理.*Echo" "$LOG_FILE" 2>/dev/null | wc -l | tr -d ' ')
            echo "服务器 #$i: 处理了 $REQUEST_COUNT 个请求"
        else
            echo "服务器 #$i: 日志文件未找到"
        fi
    done
    echo ""
    
    exit 0
else
    echo -e "${RED}❌ 测试失败！${NC}"
    echo ""
    exit 1
fi
