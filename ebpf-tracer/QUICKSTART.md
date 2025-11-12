# Quick Start Guide

## 1. Install Prerequisites

```bash
# Install Rust nightly
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install nightly
rustup default nightly

# Add eBPF target
rustup target add bpfel-unknown-none

# Install bpf-linker
cargo install bpf-linker

# Install dependencies (Ubuntu/Debian)
sudo apt update
sudo apt install -y llvm clang linux-headers-$(uname -r) build-essential
```

## 2. Build the Tracer

```bash
cd ebpf-tracer
chmod +x build.sh
./build.sh
```

Expected output:
```
Building eBPF tracer with aya-rs...

Building eBPF kernel program...
   Compiling ebpf-tracer-ebpf v0.1.0
    Finished release [optimized] target(s) in 12.34s

Building userspace loader...
   Compiling ebpf-tracer v0.1.0
    Finished release [optimized] target(s) in 45.67s

Build complete!
Run with: sudo ./target/release/ebpf-tracer
```

## 3. Run the Tracer

```bash
# Basic run (10 seconds)
sudo ./target/release/ebpf-tracer

# Custom duration
sudo ./target/release/ebpf-tracer --duration 30

# Run until Ctrl-C
sudo ./target/release/ebpf-tracer --duration 0

# Trace only syscalls
sudo ./target/release/ebpf-tracer --no-scheduler --no-irqs
```

## 4. Run Benchmark

```bash
chmod +x benchmark.sh

# With default workload
sudo ./benchmark.sh

# With custom workload
sudo ./benchmark.sh "make -j$(nproc)"
```

## 5. Interpret Results

Example output:
```
Events: total=1523847 syscalls=1245632 sched=12453 irqs=265762

=== Final Statistics ===
Duration: 10.00s
Total events: 1523847
  Syscalls: 1245632 (81.7%)
  Scheduler: 12453 (0.8%)
  IRQs: 265762 (17.4%)

Event rate: 152385 events/sec
Syscall rate: 62282 syscalls/sec

=== Overhead Comparison ===
Estimated eBPF overhead per event: ~200-500 cycles
KUtrace overhead per event: ~5-10 cycles
Overhead multiplier: ~20-50x
```

**What this means:**
- System is doing ~62K syscalls/second
- eBPF adds ~200-500 cycles per event = ~30M cycles/sec
- On a 2.5 GHz CPU: 30M / 2.5B = **1.2% CPU overhead**
- KUtrace would add only ~0.06% for the same workload

## 6. Compare with KUtrace

If you have KUtrace installed:

```bash
# Terminal 1: Start KUtrace
cd /path/to/KUtrace
sudo insmod linux/module/kutrace_mod.ko
sudo linux/control/kutrace go myprogram
sleep 10
sudo linux/control/kutrace stop
sudo rmmod kutrace_mod

# Terminal 2: Start eBPF tracer
cd /path/to/ebpf-tracer
sudo ./target/release/ebpf-tracer --duration 10
```

Compare:
- Event counts (should be similar)
- CPU overhead (eBPF will be 20-50x higher)
- Timestamp precision (KUtrace has cycle-level, eBPF has nanosecond)

## 7. Troubleshooting

### "Permission denied" when loading eBPF
```bash
# Option 1: Run as root
sudo ./target/release/ebpf-tracer

# Option 2: Add CAP_BPF capability (kernel 5.8+)
sudo setcap cap_bpf,cap_perfmon,cap_net_admin+ep ./target/release/ebpf-tracer
./target/release/ebpf-tracer
```

### "bpf_linker not found"
```bash
cargo install bpf-linker
```

### "Failed to attach tracepoint"
```bash
# Check if tracepoints exist
ls /sys/kernel/debug/tracing/events/raw_syscalls/
ls /sys/kernel/debug/tracing/events/sched/
ls /sys/kernel/debug/tracing/events/irq/

# Mount debugfs if needed
sudo mount -t debugfs none /sys/kernel/debug
```

### Build fails with "error: linker `rust-lld` not found"
```bash
# Install LLVM
sudo apt install llvm clang lld
```

## 8. Next Steps

- Read [README.md](README.md) for detailed architecture
- Compare overhead with your actual workload
- Try different tracing configurations
- Measure impact on your application performance

## Performance Tips

1. **Reduce event rate**: Disable unnecessary tracing
   ```bash
   # Only trace syscalls
   sudo ./target/release/ebpf-tracer --no-scheduler --no-irqs
   ```

2. **Filter by PID**: Modify eBPF code to filter specific processes
   ```rust
   // In ebpf-tracer-ebpf/src/main.rs
   if pid != TARGET_PID {
       return Ok(0);
   }
   ```

3. **Use larger buffers**: Increase perf buffer size for high event rates
   ```rust
   // In src/main.rs
   let mut buf = perf_array.open(cpu, Some(65536))?; // Default: 4096
   ```

4. **Sample instead of trace all**: Add probabilistic sampling
   ```rust
   // Trace only 1 in N events
   if (bpf_get_prandom_u32() % 100) > 10 {
       return Ok(0);
   }
   ```
