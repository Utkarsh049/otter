#!/usr/bin/env bash
set -euo pipefail

echo "=========================================="
echo "      Running Otter Test Suite            "
echo "=========================================="

echo "--> Running all tests sequentially..."
cargo test -- --test-threads=1

echo "=========================================="
echo "    All tests passed successfully!        "
echo "=========================================="
