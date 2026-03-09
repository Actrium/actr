#!/bin/bash
# mutil-actr 并发测试脚本 - 使用 actrix 作为信令服务器
# 测试多个客户端并发向一个服务器发送消息
#
# 用法:
#   ./start.sh              # 启动 3 个并发客户端（默认）
#   ./start.sh 5            # 启动 5 个并发客户端

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 测试 mutil-actr (多客户端并发测试)"
echo "    使用 Actrix 作为信令服务器"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 获取并发客户端数量（默认 3 个）
NUM_CLIENTS=${1:-3}
echo "📊 将启动 $NUM_CLIENTS 个并发客户端"

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

# Ensure actr.toml files exist
source "$WORKSPACE_ROOT/scripts/ensure-config-toml.sh"

# Ensure actr.toml files exist for server and client
echo ""
echo "🔍 Checking actr.toml files..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_actr_toml "$ECHO_SERVER_DIR"
ensure_actr_toml "$ECHO_CLIENT_DIR"

# Ensure actrix-config.toml exists
ensure_actrix_config "$WORKSPACE_ROOT"

echo ""
echo "🧰 Checking required CLI tools..."
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

    # Kill echo server
    if [ ! -z "$SERVER_PID" ]; then
        echo "停止 mutil-actr/server (PID: $SERVER_PID)"
        kill -9 $SERVER_PID 2>/dev/null || true
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
echo "🛠️ Generating server code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_GEN_CMD=""

if command -v actr > /dev/null 2>&1; then
    ACTR_GEN_CMD="actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/debug/actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/release/actr"
else
    echo -e "${RED}❌ actr generator not found (expected 'actr' in PATH or built under $ACTOR_RTC_DIR/actr)${NC}"
    exit 1
fi

if [ ! -d "$PROTO_DIR" ]; then
    echo -e "${RED}❌ Proto directory not found at $PROTO_DIR${NC}"
    exit 1
fi

cd "$ECHO_SERVER_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-echo-server.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"

echo -e "${GREEN}✅ actr gen completed (server code refreshed)${NC}"

# Step 0b: Generate client code (protobuf + actor glue)
echo ""
echo "🛠️ Generating client code (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$ECHO_CLIENT_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-echo-client.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen failed${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"

echo -e "${GREEN}✅ actr gen completed (client code refreshed)${NC}"

# Step 1: Build and install actrix
echo ""
echo "📦 Building and installing actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Check if actrix directory exists
if [ ! -d "$ACTRIX_DIR" ]; then
    echo -e "${RED}❌ Cannot find actrix directory at $ACTRIX_DIR${NC}"
    echo "Please ensure actrix project exists at: $ACTRIX_DIR"
    exit 1
fi

# Build actrix with opentelemetry feature
echo "Building actrix from source (with opentelemetry feature)..."
cd "$ACTRIX_DIR"
cargo build --features opentelemetry 2>&1

# Check if build was successful
if [ ! -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    echo -e "${RED}❌ Failed to build actrix${NC}"
    exit 1
fi

# Copy to ~/.cargo/bin
echo "Installing actrix to ~/.cargo/bin..."
mkdir -p ~/.cargo/bin
cp "$ACTRIX_DIR/target/debug/actrix" ~/.cargo/bin/actrix
chmod +x ~/.cargo/bin/actrix

# Return to workspace root
cd "$WORKSPACE_ROOT"

# Verify actrix is available in PATH
if ! command -v actrix > /dev/null 2>&1; then
    echo -e "${RED}❌ actrix command not found in PATH after installation${NC}"
    echo "Please ensure ~/.cargo/bin is in your PATH"
    exit 1
fi

ACTRIX_CMD="actrix"
echo -e "${GREEN}✅ Actrix built and installed: $(which actrix)${NC}"

# Step 2: Start actrix
echo ""
echo "🚀 Starting actrix (signaling server)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

$ACTRIX_CMD --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "Actrix started (PID: $ACTRIX_PID)"
echo "Waiting for actrix to be ready..."

# Wait for actrix to start and verify it's listening on port 8081
MAX_WAIT=10
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ Actrix failed to start${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi
    
    # Check if port 8081 is listening (actrix WebSocket server)
    if lsof -i:8081 > /dev/null 2>&1 || nc -z localhost 8081 2>/dev/null; then
        echo -e "${GREEN}✅ Actrix is running and listening on port 8081${NC}"
        break
    fi
    
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ Actrix not listening on port 8081 after ${MAX_WAIT} seconds${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# Step 2.5: Setting up realms in actrix
echo ""
echo "🔑 Setting up realms in actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Wait a bit for supervisord gRPC service to be ready (port 50055)
sleep 2

# Build realm-setup tool if needed
if ! cargo build -p realm-setup 2>&1 | tail -5; then
    echo -e "${RED}❌ Failed to build realm-setup tool${NC}"
    exit 1
fi

# Run realm-setup with actr.toml files from server and client
REALM_SETUP_OUTPUT="$LOG_DIR/realm-setup.log"
if ! cargo run -p realm-setup -- \
    --actrix-config "$ACTRIX_CONFIG" \
    --actr-toml "$ECHO_SERVER_DIR/actr.toml" \
    --actr-toml "$ECHO_CLIENT_DIR/actr.toml" \
    > "$REALM_SETUP_OUTPUT" 2>&1; then
    echo -e "${RED}❌ Failed to setup realms in actrix${NC}"
    cat "$REALM_SETUP_OUTPUT"
    exit 1
fi

echo -e "${GREEN}✅ Realms setup completed${NC}"
cat "$REALM_SETUP_OUTPUT" | grep -E "(Created|Skipped|Found)" || true

# Step 3: Start mutil-actr/server
echo ""
echo "🚀 Starting mutil-actr/server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

RUST_LOG="${RUST_LOG:-info}" cargo run --bin mutil_server > "$LOG_DIR/mutil-actr-server.log" 2>&1 &
SERVER_PID=$!

echo "Server started (PID: $SERVER_PID)"
echo "Waiting for server to register and connect to signaling server..."

# Wait for server to start and connect
MAX_WAIT=15
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}❌ Server failed to start${NC}"
        cat "$LOG_DIR/mutil-actr-server.log"
        exit 1
    fi
    
    # Check if server has successfully connected to signaling server
    if grep -q "ActrNode 启动成功\|Echo Server 已完全启动并注册\|ActrNode started" "$LOG_DIR/mutil-actr-server.log" 2>/dev/null; then
        echo -e "${GREEN}✅ Server is running and registered${NC}"
        break
    fi
    
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  Server may not have fully registered, but continuing...${NC}"
fi

# Give server a bit more time to fully register
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
echo "  总客户端数: $NUM_CLIENTS"
echo "  成功: $SUCCESS_COUNT"
echo "  失败: $FAIL_COUNT"
echo ""

if [ $SUCCESS_COUNT -eq $NUM_CLIENTS ]; then
    echo -e "${GREEN}🎉 测试完全成功！${NC}"
    echo ""
    echo "✅ 已验证:"
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
    
    exit 0
else
    echo -e "${RED}❌ 测试失败！${NC}"
    echo ""
    exit 1
fi
