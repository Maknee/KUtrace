# Offline symbol and process-name resolution

`resolve_trace_offline.py` enriches a completed KUtrace JSON without reading a
running process. It never reads `/proc` or `/sys` and rejects sidecar arguments
that point there.

```bash
./resolve_trace_offline.py recording.json \
  --kallsyms recording.kallsyms \
  --procmaps recording.procmaps \
  --pid-names recording.pidnames \
  --binary-root /path/to/offline-root \
  --output recording_resolved.json
```

Every sidecar is optional:

- `kallsyms` resolves kernel PC samples from a previously saved symbol table.
- `procmaps` resolves user PC samples from a previously saved multi-PID maps
  file. `binary-root` remaps absolute executable paths into an offline root.
- `pidnames` overrides inferred names. Each text line is
  `PID<TAB>THREAD_NAME<TAB>PROCESS_NAME`; JSON mappings are also accepted.

The output includes `pidNames` for the viewer and an `offlineResolution` record
showing which offline inputs were used and how many values were resolved.

For the complete trace-to-HTML flow, place any sidecars next to `recording.trace`
and run:

```bash
./postproc3_sym.sh recording "Trace title"
```

That script automatically discovers `recording.kallsyms`,
`recording.procmaps`, and `recording.pidnames`. It performs no live process
inspection.

## Runnable postmortem example

The example project runs a short-lived target, verifies that it has exited, and
only then resolves its recorded PC and produces a self-contained HTML viewer:

```bash
cd examples/offline_symbolization
./run_demo.sh
```

It intentionally uses a non-PIE executable, allowing its mapping to be
reconstructed after exit without reading `/proc/<pid>`. For production PIE
executables and shared libraries, save their ASLR mappings as capture metadata;
the expensive symbol lookup remains an offline postprocessing step.
