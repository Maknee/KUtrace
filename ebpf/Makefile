.PHONY: build build-ebpf bench-profiler-overhead bench-ui bench-ui-scale bench-uprobe check-arm64 test test-ui verify-arm64-live verify-live-differential smoke

build:
	cargo build --release --workspace

build-ebpf:
	cd kutrace-ebpf && RUSTC_BOOTSTRAP=1 cargo build --release \
		--target bpfel-unknown-none -Z build-std=core

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

bench-uprobe: build build-ebpf
	./bench_uprobe_overhead.sh

bench-profiler-overhead: build build-ebpf
	./bench_profiler_comparison.sh

verify-live-differential: build build-ebpf
	./verify_live_differential.sh

verify-arm64-live: build
	cd kutrace-ebpf && AYA_BPF_TARGET_ARCH=aarch64 \
		RUSTC_BOOTSTRAP=1 cargo build --release \
		--target bpfel-unknown-none -Z build-std=core
	./verify_arm64_live.sh

smoke: build build-ebpf
	@echo "Run: sudo target/release/kutrace-collector --ebpf kutrace-ebpf/target/bpfel-unknown-none/release/kutrace-ebpf"
