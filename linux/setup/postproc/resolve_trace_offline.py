#!/usr/bin/env python3
"""Resolve KUtrace PC samples and PID names from offline snapshot files only.

This tool never reads /proc or /sys and never attaches to a running process.
It consumes a completed KUtrace JSON file plus optional saved sidecars.
"""

import argparse
import bisect
import collections
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


PC_USER = 0x280
PC_KERNEL = 0x281
RAW_PC_RE = re.compile(r"^(?:0x)?[0-9a-fA-F]{6,16}$")
MAP_HEADER_RE = re.compile(r"^====\s+/proc/(\d+)/maps\s*$")


def offline_path(value, label, must_exist=True):
    path = Path(value).expanduser().resolve()
    for forbidden in (Path("/proc"), Path("/sys")):
        try:
            common = os.path.commonpath((str(path), str(forbidden)))
        except ValueError:
            common = ""
        if common == str(forbidden):
            raise SystemExit(f"{label} must be an offline snapshot, not {path}")
    if must_exist and not path.exists():
        raise SystemExit(f"{label} does not exist: {path}")
    return path


def name_hash(text):
    value = 0
    for byte in text.encode("utf-8"):
        value = ((value << 3) ^ byte) & 0xFFFFFFFFFFFFFFFF
    value ^= value >> 32
    value ^= value >> 16
    return value & 0xFFFF


def raw_pc(event):
    if len(event) < 10 or not isinstance(event[9], str) or not event[9].startswith("PC="):
        return None
    value = event[9][3:]
    if not RAW_PC_RE.fullmatch(value):
        return None
    return int(value, 16)


def load_kernel_symbols(path):
    entries = []
    with path.open("r", encoding="utf-8", errors="replace") as source:
        for line in source:
            fields = line.split()
            if len(fields) < 3:
                continue
            try:
                address = int(fields[0], 16)
            except ValueError:
                continue
            if address:
                entries.append((address, fields[2]))
    entries.sort()
    return [entry[0] for entry in entries], [entry[1] for entry in entries]


def resolve_kernel(events, path):
    addresses, names = load_kernel_symbols(path)
    resolved = 0
    if not addresses:
        return resolved
    for event in events:
        if len(event) < 10 or event[5] != PC_KERNEL:
            continue
        address = raw_pc(event)
        if address is None:
            continue
        index = bisect.bisect_right(addresses, address) - 1
        if index >= 0:
            event[9] = "PC=" + names[index]
            event[6] = name_hash(names[index])
            resolved += 1
    return resolved


def load_process_maps(path):
    maps = collections.defaultdict(list)
    process_names = {}
    current_pid = None
    with path.open("r", encoding="utf-8", errors="replace") as source:
        for raw_line in source:
            line = raw_line.rstrip("\r\n")
            header = MAP_HEADER_RE.match(line)
            if header:
                current_pid = int(header.group(1))
                continue
            if current_pid is None:
                continue
            fields = line.split(None, 5)
            if len(fields) < 6 or "x" not in fields[1]:
                continue
            try:
                lo_text, hi_text = fields[0].split("-", 1)
                lo, hi, file_offset = int(lo_text, 16), int(hi_text, 16), int(fields[2], 16)
            except ValueError:
                continue
            pathname = fields[5].strip()
            if not pathname.startswith("/"):
                continue
            maps[current_pid].append((lo, hi, file_offset, pathname))
            if current_pid not in process_names and "/lib" not in pathname:
                process_names[current_pid] = Path(pathname).name
    for entries in maps.values():
        entries.sort(key=lambda entry: entry[0])
    return maps, process_names


def mapped_binary(pathname, binary_root):
    candidate = (binary_root / pathname.lstrip("/")) if binary_root else Path(pathname)
    try:
        return offline_path(candidate, "mapped binary")
    except SystemExit:
        return None


def addr2line_name(binary, offset, cache):
    key = (str(binary), offset)
    if key in cache:
        return cache[key]
    result = subprocess.run(
        ["addr2line", "-f", "-s", "-C", "-e", str(binary), hex(offset)],
        text=True, capture_output=True, check=False,
    )
    lines = result.stdout.splitlines()
    name = lines[0].split("(", 1)[0].strip() if lines else ""
    if result.returncode or not name or name.startswith("??"):
        name = None
    cache[key] = name
    return name


def resolve_user(events, maps, binary_root):
    if shutil.which("addr2line") is None:
        raise SystemExit("addr2line is required for --procmaps")
    starts = {pid: [entry[0] for entry in entries] for pid, entries in maps.items()}
    cache = {}
    resolved = 0
    for event in events:
        if len(event) < 10 or event[5] != PC_USER:
            continue
        address = raw_pc(event)
        pid = int(event[3])
        if address is None or pid not in maps:
            continue
        index = bisect.bisect_right(starts[pid], address) - 1
        if index < 0:
            continue
        lo, hi, file_offset, pathname = maps[pid][index]
        if not (lo <= address < hi):
            continue
        binary = mapped_binary(pathname, binary_root)
        if binary is None:
            continue
        symbol = addr2line_name(binary, file_offset + address - lo, cache)
        if symbol:
            event[9] = "PC=" + symbol
            event[6] = name_hash(symbol)
            resolved += 1
    return resolved


def clean_thread_name(name, pid):
    suffix = "." + str(pid)
    if name.endswith(suffix):
        name = name[:-len(suffix)]
    return name.strip()


def infer_pid_names(events, process_names):
    counts = collections.defaultdict(collections.Counter)
    seen_pids = set()
    for event in events:
        if len(event) < 10:
            continue
        pid = int(event[3])
        if pid <= 0:
            continue
        seen_pids.add(pid)
        event_number, event_name = int(event[5]), event[9]
        is_user_span = (event_number & 0xF0000) == 0x10000 and event_number != 0x10000
        if is_user_span and isinstance(event_name, str) and not event_name.startswith("PC="):
            thread = clean_thread_name(event_name, pid)
            if thread and not thread.startswith("-"):
                counts[pid][thread] += 1
    result = {}
    for pid in sorted(seen_pids):
        thread = counts[pid].most_common(1)[0][0] if counts[pid] else ""
        process = process_names.get(pid, "")
        if thread or process:
            result[str(pid)] = {"thread": thread, "process": process, "source": "trace+offline-maps"}
    return result


def load_pid_name_overrides(path):
    if path.suffix.lower() == ".json":
        with path.open("r", encoding="utf-8") as source:
            raw = json.load(source)
        return {str(key): value for key, value in raw.items()}
    result = {}
    with path.open("r", encoding="utf-8", errors="replace") as source:
        for line_number, line in enumerate(source, 1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            fields = line.split("\t") if "\t" in line else line.split(None, 2)
            try:
                pid = str(int(fields[0]))
            except (ValueError, IndexError):
                raise SystemExit(f"invalid PID-name line {line_number}: {line}")
            result[pid] = {"thread": fields[1] if len(fields) > 1 else "",
                           "process": fields[2] if len(fields) > 2 else "",
                           "source": "offline-pid-names"}
    return result


def main():
    parser = argparse.ArgumentParser(
        description="Resolve a completed KUtrace JSON using offline snapshots only.",
        epilog="PID-name text format: PID<TAB>THREAD_NAME<TAB>PROCESS_NAME",
    )
    parser.add_argument("trace_json")
    parser.add_argument("-o", "--output")
    parser.add_argument("--kallsyms", help="saved kallsyms snapshot")
    parser.add_argument("--procmaps", help="saved multi-PID maps snapshot")
    parser.add_argument("--pid-names", help="saved JSON/TSV PID-name mapping")
    parser.add_argument("--binary-root", help="offline root containing mapped binaries")
    args = parser.parse_args()

    source_path = offline_path(args.trace_json, "trace JSON")
    output_value = args.output if args.output else source_path.with_name(source_path.stem + "_resolved.json")
    output_path = offline_path(output_value, "output", must_exist=False)
    kallsyms = offline_path(args.kallsyms, "kallsyms") if args.kallsyms else None
    procmaps = offline_path(args.procmaps, "procmaps") if args.procmaps else None
    pid_names_path = offline_path(args.pid_names, "PID names") if args.pid_names else None
    binary_root = offline_path(args.binary_root, "binary root") if args.binary_root else None

    with source_path.open("r", encoding="utf-8") as source:
        trace = json.load(source)
    events = trace.get("events")
    if not isinstance(events, list):
        raise SystemExit("input does not contain an events array")

    kernel_count = resolve_kernel(events, kallsyms) if kallsyms else 0
    maps, process_names = load_process_maps(procmaps) if procmaps else ({}, {})
    user_count = resolve_user(events, maps, binary_root) if procmaps else 0
    pid_names = infer_pid_names(events, process_names)
    if pid_names_path:
        pid_names.update(load_pid_name_overrides(pid_names_path))
    trace["pidNames"] = pid_names
    trace["offlineResolution"] = {
        "kernelSamples": kernel_count,
        "userSamples": user_count,
        "pidNames": len(pid_names),
        "kallsyms": kallsyms.name if kallsyms else None,
        "procmaps": procmaps.name if procmaps else None,
        "pidNameFile": pid_names_path.name if pid_names_path else None,
        "liveProcessReads": False,
    }

    output_path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=output_path.name + ".", dir=output_path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(trace, output, separators=(",", ":"))
            output.write("\n")
        os.replace(temp_name, output_path)
    finally:
        if os.path.exists(temp_name):
            os.unlink(temp_name)
    print(f"wrote {output_path}")
    print(f"resolved kernel={kernel_count} user={user_count} pid_names={len(pid_names)}")


if __name__ == "__main__":
    main()
