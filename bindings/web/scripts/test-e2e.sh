#!/bin/bash
set -e

# E2E 测试运行脚本

echo "🧪 Running E2E Tests..."

# 检查开发服务器是否运行
check_server() {
  curl -s http://localhost:5173 > /dev/null 2>&1
  return $?
}

# 启动开发服务器
start_server() {
  echo "📦 Starting development server..."

  # 确保根目录依赖已安装
  if [ ! -d "node_modules" ]; then
    echo "Installing dependencies at root..."
    pnpm install
  fi

  cd examples/hello-world

  # 后台启动服务器
  pnpm run dev > /tmp/vite.log 2>&1 &
  SERVER_PID=$!
  echo "Server PID: $SERVER_PID"

  # 等待服务器启动
  echo "Waiting for server to start..."
  for i in {1..30}; do
    if check_server; then
      echo "✅ Server is ready"
      cd ../..
      return 0
    fi
    sleep 1
  done

  echo "❌ Server failed to start"
  cat /tmp/vite.log
  return 1
}

# 停止服务器
stop_server() {
  if [ ! -z "$SERVER_PID" ]; then
    echo "Stopping server (PID: $SERVER_PID)..."
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
  fi
}

# 清理函数
cleanup() {
  echo "🧹 Cleaning up..."
  stop_server
}

# 注册清理函数
trap cleanup EXIT INT TERM

# 主逻辑
main() {
  # 检查服务器是否已经运行
  if check_server; then
    echo "✅ Development server already running"
  else
    start_server || exit 1
  fi

  # 进入测试目录
  cd tests/e2e

  # 检查依赖
  if [ ! -d "node_modules" ]; then
    echo "📦 Installing test dependencies..."
    pnpm install
  fi

  # 运行 Puppeteer 测试
  echo ""
  echo "🎭 Running Puppeteer tests..."
  pnpm test || {
    echo "❌ Puppeteer tests failed"
    exit 1
  }

  # 询问是否运行 Playwright 测试
  if [ "$RUN_PLAYWRIGHT" = "true" ]; then
    echo ""
    echo "🎭 Running Playwright tests..."

    # 检查 Playwright 是否安装
    if ! npx playwright --version > /dev/null 2>&1; then
      echo "Installing Playwright browsers..."
      npx playwright install chromium
    fi

    pnpm run test:browser || {
      echo "❌ Playwright tests failed"
      exit 1
    }
  fi

  echo ""
  echo "✅ All E2E tests passed!"
}

# 运行
main
