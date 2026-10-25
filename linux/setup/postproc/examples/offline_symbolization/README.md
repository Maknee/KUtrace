# Postmortem symbolization example

This project proves that KUtrace user-PC symbol lookup can run after the traced
process has exited. The resolver never attaches to the target and never reads
`/proc` or `/sys`.

Run:

```bash
./run_demo.sh
```

The script:

1. builds and runs a short-lived target that records a sampled address;
2. waits for and verifies the target's exit;
3. constructs the completed trace and offline metadata;
4. invokes `resolve_trace_offline.py`;
5. asserts that the PC became `PC=demo_hot_function`; and
6. creates `build/demo_resolved.html` with the fast viewer.

The target is built as a non-PIE executable so the example can reconstruct its
mapping from the binary after exit, with no live process inspection. Production
PIE executables and shared libraries use ASLR, so their load mappings must be
saved as capture metadata before those mappings disappear. Symbol lookup itself
still happens later, against saved maps and offline binaries.
