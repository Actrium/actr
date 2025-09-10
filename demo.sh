#!/bin/bash

# Actor-RTC Framework 一键演示脚本
# 完整构建、测试 Echo 功能

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m'

# 配置
SIGNALING_PORT=8081
SIGNALING_URL="ws://localhost:${SIGNALING_PORT}"
ECHO_ACTOR_ID=1001
CLIENT_ACTOR_ID=2001

# PID 和日志文件
WORK_DIR=$(pwd)
PID_DIR="${WORK_DIR}/.demo_pids"
LOG_DIR="${WORK_DIR}/.demo_logs"

SIGNALING_PID_FILE="${PID_DIR}/signaling.pid"
ECHO_ACTOR_PID_FILE="${PID_DIR}/echo.pid"
CLIENT_ACTOR_PID_FILE="${PID_DIR}/client.pid"

SIGNALING_LOG="${LOG_DIR}/signaling.log"
ECHO_ACTOR_LOG="${LOG_DIR}/echo.log"
CLIENT_ACTOR_LOG="${LOG_DIR}/client.log"

log_info() {
    echo -e "${GREEN}✓ $1${NC}"
}

log_step() {
    echo -e "\n${BLUE}📋 $1${NC}"
}

log_warn() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

log_error() {
    echo -e "${RED}❌ $1${NC}"
}

# 清理函数
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up processes...${NC}"
    
    # 停止所有后台进程
    for pid_file in "${SIGNALING_PID_FILE}" "${ECHO_ACTOR_PID_FILE}" "${CLIENT_ACTOR_PID_FILE}"; do
        if [[ -f "$pid_file" ]]; then
            PID=$(cat "$pid_file")
            if kill -0 $PID 2>/dev/null; then
                kill $PID 2>/dev/null || true
                log_info "Stopped process $PID"
            fi
            rm -f "$pid_file"
        fi
    done
    
    # 额外清理可能的进程
    pkill -f "signaling-server" 2>/dev/null || true
    pkill -f "echo-demo" 2>/dev/null || true
    pkill -f "echo-client" 2>/dev/null || true
    
    log_info "Cleanup completed"
}

# 设置退出时清理
trap cleanup EXIT INT TERM

# 创建目录
setup_dirs() {
    mkdir -p "$PID_DIR" "$LOG_DIR"
    log_info "Created work directories"
}

# 构建项目
build_project() {
    log_step "构建 Actor-RTC 项目"
    
    # 首先构建 protoc 插件
    log_info "Building protoc-gen-actorframework plugin..."
    cargo build --bin protoc-gen-actorframework
    
    # 然后构建协议
    log_info "Building shared-protocols..."
    cargo build -p shared-protocols
    
    # 构建框架
    log_info "Building actor-rtc-framework..."
    cargo build -p actor-rtc-framework
    
    # 最后构建应用
    log_info "Building signaling server..."
    cargo build --bin signaling-server
    
    log_info "Building echo demo..."
    cargo build --bin echo-demo
    
    log_info "Building echo client..."
    cargo build --bin echo-client
    
    log_info "All components built successfully"
}

# 启动信令服务器
start_signaling() {
    log_step "启动信令服务器"
    
    SIGNALING_ADDR="0.0.0.0:${SIGNALING_PORT}" nohup ./target/debug/signaling-server > "$SIGNALING_LOG" 2>&1 &
    SIGNALING_PID=$!
    echo $SIGNALING_PID > "$SIGNALING_PID_FILE"
    
    # 等待启动
    sleep 1
    if kill -0 $SIGNALING_PID 2>/dev/null; then
        log_info "Signaling server started (PID: $SIGNALING_PID)"
    else
        log_error "Failed to start signaling server"
        cat "$SIGNALING_LOG"
        exit 1
    fi
}

# 启动 Echo Actor
start_echo_actor() {
    log_step "启动 Echo Actor"
    
    SIGNALING_URL="$SIGNALING_URL" ACTOR_ID="$ECHO_ACTOR_ID" \
        nohup ./target/debug/echo-demo > "$ECHO_ACTOR_LOG" 2>&1 &
    ECHO_PID=$!
    echo $ECHO_PID > "$ECHO_ACTOR_PID_FILE"
    
    # 等待启动和注册
    sleep 2
    if kill -0 $ECHO_PID 2>/dev/null; then
        log_info "Echo Actor started (PID: $ECHO_PID)"
        
        # 检查是否注册成功
        if grep -q "注册为 Actor $ECHO_ACTOR_ID" "$SIGNALING_LOG" 2>/dev/null; then
            log_info "Echo Actor registered successfully"
        else
            log_warn "Echo Actor registration status unclear"
        fi
    else
        log_error "Failed to start Echo Actor"
        cat "$ECHO_ACTOR_LOG"
        exit 1
    fi
}

# 运行 Echo Client 测试
run_echo_test() {
    log_step "运行 Echo 客户端测试"
    
    SIGNALING_URL="$SIGNALING_URL" ACTOR_ID="$CLIENT_ACTOR_ID" TARGET_ACTOR_ID="$ECHO_ACTOR_ID" \
        timeout 15s ./target/debug/echo-client > "$CLIENT_ACTOR_LOG" 2>&1 &
    CLIENT_PID=$!
    echo $CLIENT_PID > "$CLIENT_ACTOR_PID_FILE"
    
    log_info "Echo Client started (PID: $CLIENT_PID)"
    
    # 等待测试完成
    wait $CLIENT_PID 2>/dev/null || true
    
    log_info "Echo test completed"
}

# 显示测试结果
show_results() {
    log_step "显示测试结果"
    
    echo -e "\n${PURPLE}=== 信令服务器日志 ===${NC}"
    tail -5 "$SIGNALING_LOG" 2>/dev/null || echo "No signaling logs"
    
    echo -e "\n${PURPLE}=== Echo Actor 日志 ===${NC}"
    tail -5 "$ECHO_ACTOR_LOG" 2>/dev/null || echo "No echo actor logs"
    
    echo -e "\n${PURPLE}=== Echo Client 日志 ===${NC}"
    cat "$CLIENT_ACTOR_LOG" 2>/dev/null || echo "No echo client logs"
    
    # 统计结果
    echo -e "\n${CYAN}📊 测试统计:${NC}"
    
    registered_actors=$(grep -c "注册为 Actor" "$SIGNALING_LOG" 2>/dev/null || echo "0")
    echo "- 注册的 Actor 数量: $registered_actors"
    
    if grep -q "Starting echo test sequence" "$CLIENT_ACTOR_LOG" 2>/dev/null; then
        log_info "Echo 测试序列已启动"
    fi
    
    echo_requests=$(grep -c "Sending echo request" "$CLIENT_ACTOR_LOG" 2>/dev/null || echo "0" | head -1 | tr -d '\n')
    echo "- 发送的 Echo 请求: $echo_requests"
    
    if grep -q "Starting echo test sequence" "$CLIENT_ACTOR_LOG" 2>/dev/null; then
        log_info "Echo 测试序列已启动"
    fi
    
    if grep -q "Echo client demo completed" "$CLIENT_ACTOR_LOG" 2>/dev/null; then
        log_info "Echo 客户端测试完成"
    fi
    
    # 确保变量是干净的数字
    registered_count=$(echo "$registered_actors" | head -1 | tr -d '\n' | grep -o '[0-9]*' || echo "0")
    request_count=$(echo "$echo_requests" | head -1 | tr -d '\n' | grep -o '[0-9]*' || echo "0")
    
    if [[ $registered_count -eq 2 ]] && [[ $request_count -gt 0 ]]; then
        echo -e "\n${GREEN}🎉 演示成功完成！${NC}"
        echo -e "${CYAN}✅ 两个 Actor 成功注册并通信${NC}"
    else
        echo -e "\n${YELLOW}⚠️  演示部分完成，请检查日志${NC}"
        if [[ $registered_count -ne 2 ]]; then
            echo -e "${YELLOW}   - Actor 注册数量不正确 (期望: 2, 实际: $registered_count)${NC}"
        fi
        if [[ $request_count -eq 0 ]]; then
            echo -e "${YELLOW}   - 未检测到 Echo 请求${NC}"
        fi
    fi
}

# 主函数
main() {
    echo -e "${PURPLE}🚀 Actor-RTC Framework 一键演示${NC}"
    echo -e "${CYAN}   完整构建和测试 Echo 功能${NC}\n"
    
    # 清理之前的进程
    cleanup 2>/dev/null || true
    
    setup_dirs
    build_project
    start_signaling
    start_echo_actor
    run_echo_test
    show_results
    
    echo -e "\n${BLUE}📁 日志文件位置:${NC}"
    echo "- 信令服务器: $SIGNALING_LOG"
    echo "- Echo Actor: $ECHO_ACTOR_LOG"
    echo "- Echo Client: $CLIENT_ACTOR_LOG"
    
    echo -e "\n${GREEN}演示完成！${NC}"
}

# 显示帮助
show_help() {
    echo "Actor-RTC Framework 一键演示脚本"
    echo ""
    echo "用法: $0 [选项]"
    echo ""
    echo "选项:"
    echo "  help, -h, --help    显示此帮助信息"
    echo "  clean               清理构建文件和日志"
    echo ""
    echo "默认行为: 构建项目并运行完整的 Echo 演示"
}

# 清理选项
clean_project() {
    log_step "清理项目"
    
    cleanup 2>/dev/null || true
    
    if [[ -d "$PID_DIR" ]]; then
        rm -rf "$PID_DIR"
        log_info "Removed PID directory"
    fi
    
    if [[ -d "$LOG_DIR" ]]; then
        rm -rf "$LOG_DIR"
        log_info "Removed log directory"
    fi
    
    cargo clean
    log_info "Cargo clean completed"
    
    log_info "Project cleaned"
}

# 处理参数
case "${1:-demo}" in
    "help"|"-h"|"--help")
        show_help
        ;;
    "clean")
        clean_project
        ;;
    "demo"|"")
        main
        ;;
    *)
        echo "Unknown option: $1"
        echo "Use '$0 help' for usage information"
        exit 1
        ;;
esac