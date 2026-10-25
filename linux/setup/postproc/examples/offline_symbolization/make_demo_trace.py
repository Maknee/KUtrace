#!/usr/bin/env python3
"""Turn the completed demo capture into offline KUtrace inputs."""

import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("capture")
    parser.add_argument("binary")
    parser.add_argument("output_dir")
    args = parser.parse_args()

    capture = Path(args.capture).resolve()
    binary = Path(args.binary).resolve()
    output_dir = Path(args.output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)

    pid_text, address_text = capture.read_text(encoding="utf-8").split()
    pid, address = int(pid_text), int(address_text, 16)
    page_start = address & ~0xFFF

    # This map is reconstructed after exit. The demo is intentionally ET_EXEC,
    # so its virtual addresses do not depend on a live ASLR mapping.
    (output_dir / "demo.procmaps").write_text(
        f"==== /proc/{pid}/maps\n"
        f"{page_start:012x}-{page_start + 0x1000:012x} r-xp 00000000 00:00 0 {binary}\n",
        encoding="utf-8",
    )
    (output_dir / "demo.pidnames").write_text(
        f"{pid}\tpostmortem-worker\toffline-symbol-demo\n", encoding="utf-8"
    )

    trace = {
        "Comment": "Offline symbolization demo",
        "axisLabelX": "Time (sec)",
        "axisLabelY": "CPU / PID",
        "cpuModelName": "postmortem example",
        "events": [
            [0.0, 0.05, 0, pid, 0, 0x10001, 0, 0, 0, f"postmortem-worker.{pid}"],
            [0.01, 0.01, 0, pid, 0, 0x280, 0, 0, 0, f"PC={address:x}"],
            [999.0, 0, 0, 0, 0, 0, 0, 0, 0, ""],
        ],
        "extra": [],
        "flags": 0,
        "kernelVersion": "offline-only",
        "mbit_sec": 0,
        "randomid": pid,
        "savedview": [],
        "shortMulX": 1,
        "shortUnitsX": "s",
        "thousandsX": 1000,
        "title": "Postmortem symbolization demo",
        "tracebase": "offline",
        "version": 3,
    }
    (output_dir / "demo.json").write_text(
        json.dumps(trace, separators=(",", ":")) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
