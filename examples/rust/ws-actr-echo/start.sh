#!/bin/bash
# Test script for ws-actr-echo example - 使用 WebSocket 直连通道的 Echo
#
# 演示流程：
#   1. 启动 actrix 信令服务器
#   2. 向数据库写入 realm 数据
#   3. 启动 ws-echo-server（在 9001 端口监听 WebSocket，并注册到信令）
#   4. 启动 ws-echo-client（从信令发现服务端 WebSocket 地址后直连）
#   5. 验证通过 WebSocket 通道完成 Echo RPC
#
# Usage:
#   ./start.sh              # 使用默认消息 "WsTest"
#   ./start.sh "你好世界"    # 发送自定义消息

set -e
set -o pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🔌 Testing ws-actr-echo (WebSocket Direct Connection)"
echo "    Server ↔ Client via WebSocket (no WebRTC)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Determine paths
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ACTOR_RTC_DIR="$(cd "$WORKSPACE_ROOT/../.." && pwd)"
ACTRIX_DIR="$ACTOR_RTC_DIR/actrix"
WS_ECHO_DIR="$SCRIPT_DIR"
SERVER_DIR="$WS_ECHO_DIR/server"
CLIENT_DIR="$WS_ECHO_DIR/client"
PROTO_DIR="$WS_ECHO_DIR/proto"
ACTRIX_CONFIG="$WS_ECHO_DIR/actrix-config.toml"

# WebSocket 服务端口和数据库路径
WS_LISTEN_PORT=9001
ACTRIX_PORT=8081
# 数据库路径：相对于 WORKSPACE_ROOT（actrix 的 CWD）
DB_PATH="$WORKSPACE_ROOT/ws-echo-db/actrix.db"

# Switch to workspace root and stay there
cd "$WORKSPACE_ROOT"

# Create logs directory
LOG_DIR="$WORKSPACE_ROOT/logs"
mkdir -p "$LOG_DIR"

# Ensure required CLI tools
source "$WORKSPACE_ROOT/scripts/ensure-tools.sh"

echo ""
echo "🔍 检查 Actr.toml 配置文件..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 函数：从 example.toml 复制配置文件
ensure_toml() {
    local target="$1"
    local example="$2"
    if [ ! -f "$target" ]; then
        if [ -f "$example" ]; then
            echo "📋 复制 $(basename "$example") → $(basename "$target")"
            cp "$example" "$target"
        else
            echo -e "${RED}❌ 找不到 $example${NC}" >&2
            return 1
        fi
    fi
}

ensure_toml "$SERVER_DIR/Actr.toml"     "$SERVER_DIR/Actr.example.toml"
ensure_toml "$CLIENT_DIR/Actr.toml"     "$CLIENT_DIR/Actr.example.toml"
ensure_toml "$ACTRIX_CONFIG"            "$WS_ECHO_DIR/actrix-config.example.toml"

echo ""
echo "🧰 检查必要的 CLI 工具..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ensure_cargo_bin "protoc-gen-prost"           "protoc-gen-prost"                  "$LOG_DIR"
ensure_cargo_bin "protoc-gen-actrframework"   "actr-framework-protoc-codegen"     "$LOG_DIR"
ensure_cargo_bin "actr"                       "actr-cli"                          "$LOG_DIR"

# Cleanup function
cleanup() {
    echo ""
    echo "🧹 清理进程..."

    if [ ! -z "$ACTRIX_PID" ]; then
        echo "停止 actrix (PID: $ACTRIX_PID)"
        kill $ACTRIX_PID 2>/dev/null || true
    fi

    if [ ! -z "$SERVER_PID" ]; then
        echo "停止 ws-echo-server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
    fi

    if [ ! -z "$CLIENT_PID" ]; then
        echo "停止 ws-echo-client (PID: $CLIENT_PID)"
        kill $CLIENT_PID 2>/dev/null || true
    fi

    wait 2>/dev/null || true
    echo "✅ 清理完成"
}

trap cleanup EXIT INT TERM

# ─────────────────────────────────────────────────────────────
# Step 0: 生成服务端代码（protobuf + actor glue）
# ─────────────────────────────────────────────────────────────
echo ""
echo "🛠️ 生成服务端代码 (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

ACTR_GEN_CMD=""
if command -v actr > /dev/null 2>&1; then
    ACTR_GEN_CMD="actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/debug/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/debug/actr"
elif [ -x "$ACTOR_RTC_DIR/actr/target/release/actr" ]; then
    ACTR_GEN_CMD="$ACTOR_RTC_DIR/actr/target/release/actr"
else
    echo -e "${RED}❌ 找不到 actr 工具（请确保 'actr' 在 PATH 中或已编译）${NC}"
    exit 1
fi

cd "$SERVER_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-ws-echo-server.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen (server) 失败${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ 服务端代码已生成${NC}"

# Step 0b: 生成客户端代码
echo ""
echo "🛠️ 生成客户端代码 (actr gen)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

cd "$CLIENT_DIR"
OUTPUT_FILE="$LOG_DIR/actr-gen-ws-echo-client.log"
$ACTR_GEN_CMD gen --input="$PROTO_DIR" --output=src/generated --clean --no-scaffold > "$OUTPUT_FILE" 2>&1 || {
    echo -e "${RED}❌ actr gen (client) 失败${NC}"
    cat "$OUTPUT_FILE"
    exit 1
}
cd "$WORKSPACE_ROOT"
echo -e "${GREEN}✅ 客户端代码已生成${NC}"

# ─────────────────────────────────────────────────────────────
# Step 1: 编译并安装 actrix
# ─────────────────────────────────────────────────────────────
echo ""
echo "📦 编译安装 actrix..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ ! -d "$ACTRIX_DIR" ]; then
    echo -e "${RED}❌ 找不到 actrix 目录: $ACTRIX_DIR${NC}"
    exit 1
fi

cd "$ACTRIX_DIR"
cargo build --features opentelemetry 2>&1

if [ ! -f "$ACTRIX_DIR/target/debug/actrix" ]; then
    echo -e "${RED}❌ actrix 编译失败${NC}"
    exit 1
fi

echo "安装 actrix 到 ~/.cargo/bin..."
mkdir -p ~/.cargo/bin
cp "$ACTRIX_DIR/target/debug/actrix" ~/.cargo/bin/actrix
chmod +x ~/.cargo/bin/actrix

cd "$WORKSPACE_ROOT"

if ! command -v actrix > /dev/null 2>&1; then
    echo -e "${RED}❌ actrix 命令安装后仍无法找到，请确认 ~/.cargo/bin 在 PATH 中${NC}"
    exit 1
fi

echo -e "${GREEN}✅ actrix 已安装: $(which actrix)${NC}"

# ─────────────────────────────────────────────────────────────
# Step 2: 启动 actrix 信令服务器
# ─────────────────────────────────────────────────────────────
echo ""
echo "🚀 启动 actrix 信令服务器..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

actrix --config "$ACTRIX_CONFIG" > "$LOG_DIR/actrix.log" 2>&1 &
ACTRIX_PID=$!

echo "actrix 已启动 (PID: $ACTRIX_PID)"
echo "等待 actrix 就绪（端口 $ACTRIX_PORT）..."

MAX_WAIT=15
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $ACTRIX_PID 2>/dev/null; then
        echo -e "${RED}❌ actrix 启动失败${NC}"
        cat "$LOG_DIR/actrix.log"
        exit 1
    fi

    if lsof -i:$ACTRIX_PORT > /dev/null 2>&1 || nc -z localhost $ACTRIX_PORT 2>/dev/null; then
        echo -e "${GREEN}✅ actrix 正在监听端口 $ACTRIX_PORT${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${RED}❌ actrix 在 ${MAX_WAIT} 秒内未监听端口 $ACTRIX_PORT${NC}"
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# ─────────────────────────────────────────────────────────────
# Step 3: 向数据库写入 realm 数据
# ─────────────────────────────────────────────────────────────
echo ""
echo "🔑 写入 realm 数据到 actrix 数据库..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 等待 actrix 创建数据库文件（可能需要几秒）
echo "等待数据库文件创建..."
COUNTER=0
while [ $COUNTER -lt 10 ]; do
    if [ -f "$DB_PATH" ]; then
        echo -e "${GREEN}✅ 数据库文件已存在: $DB_PATH${NC}"
        break
    fi
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ ! -f "$DB_PATH" ]; then
    echo -e "${RED}❌ actrix 数据库文件未创建: $DB_PATH${NC}"
    echo "actrix 日志："
    cat "$LOG_DIR/actrix.log"
    exit 1
fi

# 检查 sqlite3 是否可用
if ! command -v sqlite3 > /dev/null 2>&1; then
    echo -e "${RED}❌ 找不到 sqlite3 命令，无法写入 realm 数据${NC}"
    echo "请安装 sqlite3: brew install sqlite"
    exit 1
fi

# 插入 realm 数据（realm_id = 1001，与 Actr.toml 中配置一致）
REALM_ID=1001
sqlite3 "$DB_PATH" "INSERT OR IGNORE INTO realm (realm_id, name, status, expires_at, created_at, updated_at) VALUES ($REALM_ID, 'e2e-realm', 'Normal', NULL, strftime('%s','now'), strftime('%s','now'))"

# 验证插入成功
REALM_COUNT=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM realm WHERE realm_id = $REALM_ID")
if [ "$REALM_COUNT" -ge 1 ]; then
    echo -e "${GREEN}✅ Realm $REALM_ID 已写入数据库${NC}"
else
    echo -e "${RED}❌ Realm 插入失败${NC}"
    exit 1
fi

# ─────────────────────────────────────────────────────────────
# Step 4: 编译二进制文件
# ─────────────────────────────────────────────────────────────
echo ""
echo "🔨 编译二进制文件..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if ! cargo build --bin ws-echo-server --bin ws-echo-client-app 2>&1; then
    echo -e "${RED}❌ 编译失败${NC}"
    exit 1
fi

echo -e "${GREEN}✅ 编译成功${NC}"

# ─────────────────────────────────────────────────────────────
# Step 5: 启动 ws-echo-server
# ─────────────────────────────────────────────────────────────
echo ""
echo "🚀 启动 ws-echo-server..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  - 将在端口 $WS_LISTEN_PORT 监听 WebSocket 连接"
echo "  - 并注册 ws://127.0.0.1:$WS_LISTEN_PORT 到信令服务器"

RUST_LOG="${RUST_LOG:-info}" cargo run --bin ws-echo-server > "$LOG_DIR/ws-echo-server.log" 2>&1 &
SERVER_PID=$!

echo "ws-echo-server 已启动 (PID: $SERVER_PID)"
echo "等待服务端注册到信令服务器..."

MAX_WAIT=20
COUNTER=0
while [ $COUNTER -lt $MAX_WAIT ]; do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}❌ ws-echo-server 启动失败${NC}"
        cat "$LOG_DIR/ws-echo-server.log"
        exit 1
    fi

    if grep -q "WS Echo Server 已完全启动\|ActrNode 启动成功\|WebSocket 地址已上报" "$LOG_DIR/ws-echo-server.log" 2>/dev/null; then
        echo -e "${GREEN}✅ ws-echo-server 已启动并注册${NC}"
        break
    fi

    sleep 1
    COUNTER=$((COUNTER + 1))
done

if [ $COUNTER -eq $MAX_WAIT ]; then
    echo -e "${YELLOW}⚠️  服务端可能尚未完全注册，继续...${NC}"
fi

# 额外等待确保 WebSocket 端口就绪
sleep 2

# 验证服务端 WebSocket 端口是否在监听
if lsof -i:$WS_LISTEN_PORT > /dev/null 2>&1 || nc -z localhost $WS_LISTEN_PORT 2>/dev/null; then
    echo -e "${GREEN}✅ WebSocket 端口 $WS_LISTEN_PORT 已就绪${NC}"
else
    echo -e "${YELLOW}⚠️  WebSocket 端口 $WS_LISTEN_PORT 尚未就绪（可能仍在初始化）${NC}"
fi

# ─────────────────────────────────────────────────────────────
# Step 6: 运行 ws-echo-client
# ─────────────────────────────────────────────────────────────
echo ""
echo "🚀 运行 ws-echo-client..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ -n "$1" ]; then
    TEST_INPUT="$1"
else
    TEST_INPUT="WsTest"
fi

echo "发送测试消息: \"$TEST_INPUT\""

(
    sleep 3
    echo "$TEST_INPUT"
    sleep 3
    echo "quit"
) | RUST_LOG="${RUST_LOG:-info}" cargo run --bin ws-echo-client-app > "$LOG_DIR/ws-echo-client.log" 2>&1 &
CLIENT_PID=$!

# 等待客户端完成（最多 15 秒）
COUNTER=0
while kill -0 $CLIENT_PID 2>/dev/null && [ $COUNTER -lt 15 ]; do
    sleep 1
    COUNTER=$((COUNTER + 1))
done

if kill -0 $CLIENT_PID 2>/dev/null; then
    echo -e "${YELLOW}⚠️  客户端超时，强制终止...${NC}"
    kill $CLIENT_PID 2>/dev/null || true
fi

# ─────────────────────────────────────────────────────────────
# Step 7: 验证输出
# ─────────────────────────────────────────────────────────────
echo ""
echo "🔍 验证输出..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 检查客户端是否收到服务端的 WS-Echo 响应
if grep -q "\[收到回复\].*WS-Echo: $TEST_INPUT" "$LOG_DIR/ws-echo-client.log"; then
    echo -e "${GREEN}✅ 测试通过：收到服务端 WebSocket Echo 响应${NC}"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🎉 ws-actr-echo 测试完成！"
    echo ""
    echo "📋 客户端日志摘要："
    grep -E "收到回复|WebSocket|发现|连接" "$LOG_DIR/ws-echo-client.log" | tail -10 || true
    echo ""
    echo "📋 服务端日志摘要："
    grep -E "\[WS\]|WebSocket|已注册|启动成功" "$LOG_DIR/ws-echo-server.log" | tail -10 || true
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    exit 0
else
    echo -e "${RED}❌ 测试失败：未找到预期的 WebSocket Echo 响应${NC}"
    echo ""
    echo "📋 客户端日志（最后 30 行）："
    tail -30 "$LOG_DIR/ws-echo-client.log"
    echo ""
    echo "📋 服务端日志（最后 30 行）："
    tail -30 "$LOG_DIR/ws-echo-server.log"
    exit 1
fi
