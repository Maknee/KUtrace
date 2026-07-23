.PHONY: build build-ui build-ebpf build-ebpf-default build-ebpf-stack verify-ebpf-artifacts bench-kprobe bench-overhead bench-profiler-overhead bench-resources bench-ui bench-ui-scale bench-uprobe bench-usdt check-arm64 test test-ui verify-arm64-live verify-dynamic-probes verify-kprobe verify-library-probe verify-live-differential smoke

build: build-ui
	cargo build --release --workspace

build-ui:
	cd kutrace-ui-web && NO_COLOR=false trunk build --release

build-ebpf: build-ebpf-default build-ebpf-stack verify-ebpf-artifacts

build-ebpf-default:
	cd kutrace-ebpf && RUSTC_BOOTSTRAP=1 cargo build --release \
		--target bpfel-unknown-none -Z build-std=core

build-ebpf-stack:
	cd kutrace-ebpf && CARGO_TARGET_DIR=target/stack-traces RUSTC_BOOTSTRAP=1 \
		cargo build --release --features stack-traces \
		--target bpfel-unknown-none -Z build-std=core

verify-ebpf-artifacts: build-ebpf-default build-ebpf-stack
	@! readelf -sW kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf | \
		grep -Eq 'USER_STACKS|KERNEL_STACKS|STACK_DROPPED'
	@readelf -sW kutrace-ebpf/target/stack-traces/bpfel-unknown-none/release/kutrace-ebpf | \
		grep -q USER_STACKS
	@readelf -sW kutrace-ebpf/target/stack-traces/bpfel-unknown-none/release/kutrace-ebpf | \
		grep -q KERNEL_STACKS
	@readelf -sW kutrace-ebpf/target/stack-traces/bpfel-unknown-none/release/kutrace-ebpf | \
		grep -q STACK_DROPPED

check-arm64:
	cargo clippy --target aarch64-unknown-linux-gnu \
		-p kutrace-common -p kutrace-client -p kutrace-collector -p kutrace-transform \
		--all-targets -- -D warnings
	cd kutrace-ebpf && AYA_BPF_TARGET_ARCH=aarch64 CARGO_TARGET_DIR=target-aarch64 \
		cargo clippy --release -- -D warnings
	cd kutrace-ebpf && AYA_BPF_TARGET_ARCH=aarch64 CARGO_TARGET_DIR=target-aarch64 \
		RUSTC_BOOTSTRAP=1 cargo build --release \
		--target bpfel-unknown-none -Z build-std=core

test:
	cargo test --workspace

test-ui: build
	cd kutrace-ui/browser && npm ci && npm test

bench-ui: build
	./bench_ui_import.sh

bench-ui-scale: build
	KUTRACE_UI_BENCH_EVENTS=10000000 \
	KUTRACE_UI_BENCH_NAME_CARDINALITY=100000 \
	KUTRACE_UI_BENCH_MAX_SECONDS=120 \
	KUTRACE_UI_BENCH_MAX_RSS_KIB=65536 \
	./bench_ui_import.sh

bench-uprobe: build build-ebpf-default
	./bench_uprobe_overhead.sh

bench-kprobe: build build-ebpf-default
	./bench_kprobe_overhead.sh

bench-usdt: build build-ebpf-default
	./bench_usdt_overhead.sh

bench-overhead: build build-ebpf-default
	./bench_overhead.sh

bench-resources: build build-ebpf
	./bench_resource_usage.sh

bench-profiler-overhead: build build-ebpf-default
	./bench_profiler_comparison.sh

verify-dynamic-probes: build build-ebpf-default
	./verify_uprobe.sh
	./verify_library_probe.sh
	./verify_usdt.sh
	./verify_kprobe.sh

verify-kprobe: build build-ebpf-default
	./verify_kprobe.sh

verify-library-probe: build build-ebpf-default
	./verify_library_probe.sh

verify-live-differential: build build-ebpf-default
	./verify_live_differential.sh

verify-arm64-live: build
	cd kutrace-ebpf && AYA_BPF_TARGET_ARCH=aarch64 \
		RUSTC_BOOTSTRAP=1 cargo build --release \
		--target bpfel-unknown-none -Z build-std=core
	./verify_arm64_live.sh

smoke: build build-ebpf-default
	@echo "Run: sudo target/release/kutrace-collector --ebpf kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
