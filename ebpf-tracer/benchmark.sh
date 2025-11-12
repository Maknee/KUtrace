#!/bin/bash
set -e

echo "eBPF vs KUtrace Overhead Benchmark"
echo "===================================="
echo ""

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo "Error: Must run as root (sudo ./benchmark.sh)"
    exit 1
fi

# Check if the tracer is built
if [ ! -f "target/release/ebpf-tracer" ]; then
    echo "Tracer not found. Building..."
    ./build.sh
fi

# Benchmark parameters
DURATION=10
WORKLOAD_CMD=${1:-"stress-ng --cpu 1 --timeout ${DURATION}s"}

echo "Benchmark configuration:"
echo "  Duration: ${DURATION} seconds"
echo "  Workload: ${WORKLOAD_CMD}"
echo ""

# Function to get CPU usage
get_cpu_usage() {
    top -bn2 -d1 | grep "Cpu(s)" | tail -1 | awk '{print $2}' | cut -d'%' -f1
}

# Baseline test (no tracing)
echo "=== Test 1: Baseline (no tracing) ==="
echo "Starting workload..."
START_TIME=$(date +%s.%N)
eval "${WORKLOAD_CMD}" &
WORKLOAD_PID=$!
sleep 1
BASELINE_CPU=$(get_cpu_usage)
wait $WORKLOAD_PID 2>/dev/null || true
END_TIME=$(date +%s.%N)
BASELINE_DURATION=$(echo "$END_TIME - $START_TIME" | bc)
echo "  CPU usage: ${BASELINE_CPU}%"
echo "  Duration: ${BASELINE_DURATION}s"
echo ""

sleep 2

# eBPF tracer test
echo "=== Test 2: With eBPF tracer ==="
echo "Starting eBPF tracer..."
./target/release/ebpf-tracer --duration ${DURATION} > /tmp/ebpf_trace.log 2>&1 &
TRACER_PID=$!
sleep 2  # Let tracer attach

echo "Starting workload..."
START_TIME=$(date +%s.%N)
eval "${WORKLOAD_CMD}" &
WORKLOAD_PID=$!
sleep 1
EBPF_CPU=$(get_cpu_usage)
wait $WORKLOAD_PID 2>/dev/null || true
END_TIME=$(date +%s.%N)
EBPF_DURATION=$(echo "$END_TIME - $START_TIME" | bc)
wait $TRACER_PID 2>/dev/null || true
echo "  CPU usage: ${EBPF_CPU}%"
echo "  Duration: ${EBPF_DURATION}s"
echo ""

# Extract event statistics from tracer
echo "=== Tracer Statistics ==="
tail -20 /tmp/ebpf_trace.log
echo ""

# Calculate overhead
echo "=== Overhead Comparison ==="
CPU_OVERHEAD=$(echo "$EBPF_CPU - $BASELINE_CPU" | bc)
DURATION_OVERHEAD=$(echo "($EBPF_DURATION - $BASELINE_DURATION) / $BASELINE_DURATION * 100" | bc -l)
echo "CPU overhead: ${CPU_OVERHEAD}% (absolute)"
printf "Duration overhead: %.2f%% (relative)\n" $DURATION_OVERHEAD
echo ""

echo "=== Theoretical Comparison ==="
echo "eBPF overhead per event: ~200-500 cycles"
echo "KUtrace overhead per event: ~5-10 cycles"
echo "Theoretical multiplier: ~20-50x"
echo ""
echo "Note: Actual overhead depends on:"
echo "  - Event rate (syscalls/sec)"
echo "  - CPU frequency"
echo "  - System load"
echo "  - Kernel version"
echo ""
echo "To test with custom workload:"
echo "  sudo ./benchmark.sh \"your-command-here\""
