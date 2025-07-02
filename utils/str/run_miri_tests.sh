#!/bin/bash

# Script to run tests with Miri for memory safety verification
# Make sure to install Miri first: rustup component add miri

echo "Running basic tests..."
cargo test

echo ""
echo "Running tests with Miri for memory safety verification..."
echo "This may take a while as Miri performs thorough memory safety checks..."

# Run all tests with Miri
cargo +nightly miri test

# Run specific memory safety tests
echo ""
echo "Running memory safety specific tests with Miri..."
cargo +nightly miri test test_memory_safety

echo ""
echo "Memory safety testing complete!"
echo ""
echo "If all tests pass, your code is memory safe according to Miri's analysis."
echo "Miri checks for:"
echo "  - Use after free"
echo "  - Double free"
echo "  - Memory leaks"
echo "  - Invalid pointer dereferences"
echo "  - Data races (if using concurrency)"
echo "  - Undefined behavior"
