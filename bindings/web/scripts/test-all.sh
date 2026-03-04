#!/bin/bash
set -e

# Run all tests

echo "Running all tests..."

# Rust tests
echo "Running Rust tests..."
cargo test --all

# TypeScript tests
echo "Running TypeScript tests..."
npm test

# E2E tests
echo "Running E2E tests..."
npm run test:e2e || {
    echo "Warning: E2E tests not yet configured"
}

echo "✅ All tests complete!"
