#!/bin/bash
set -e

echo "Building eBPF tracer with aya-rs..."
echo ""

# Check if bpf-linker is installed
if ! command -v bpf-linker &> /dev/null; then
    echo "bpf-linker not found. Installing..."
    cargo install bpf-linker
fi

# Build eBPF program (kernel side)
echo "Building eBPF kernel program..."
cd ebpf-tracer-ebpf
cargo build --release --target=bpfel-unknown-none -Z build-std=core
cd ..

# Build userspace loader
echo "Building userspace loader..."
cargo build --release

echo ""
echo "Build complete!"
echo "Run with: sudo ./target/release/ebpf-tracer"
