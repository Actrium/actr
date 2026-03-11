#!/bin/bash
set -e

# E2E test runner script

echo "🧪 Running E2E Tests..."

# Check whether the dev server is running
check_server() {
  curl -s http://localhost:5173 > /dev/null 2>&1
  return $?
}

# Start the dev server
start_server() {
  echo "📦 Starting development server..."

  # Make sure root-level dependencies are installed
  if [ ! -d "node_modules" ]; then
    echo "Installing dependencies at root..."
    pnpm install
  fi

  cd examples/hello-world

  # Start the server in the background
  pnpm run dev > /tmp/vite.log 2>&1 &
  SERVER_PID=$!
  echo "Server PID: $SERVER_PID"

  # Wait for the server to start
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

# Stop the server
stop_server() {
  if [ ! -z "$SERVER_PID" ]; then
    echo "Stopping server (PID: $SERVER_PID)..."
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
  fi
}

# Cleanup function
cleanup() {
  echo "🧹 Cleaning up..."
  stop_server
}

# Register the cleanup function
trap cleanup EXIT INT TERM

# Main logic
main() {
  # Check whether the server is already running
  if check_server; then
    echo "✅ Development server already running"
  else
    start_server || exit 1
  fi

  # Enter the test directory
  cd tests/e2e

  # Check dependencies
  if [ ! -d "node_modules" ]; then
    echo "📦 Installing test dependencies..."
    pnpm install
  fi

  # Run Puppeteer tests
  echo ""
  echo "🎭 Running Puppeteer tests..."
  pnpm test || {
    echo "❌ Puppeteer tests failed"
    exit 1
  }

  # Ask whether to run Playwright tests
  if [ "$RUN_PLAYWRIGHT" = "true" ]; then
    echo ""
    echo "🎭 Running Playwright tests..."

    # Check whether Playwright is installed
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

# Run
main
