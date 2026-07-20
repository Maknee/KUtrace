#!/usr/bin/env python3
import ctypes
import json
import os
import signal
import threading
import time


start = threading.Event()
ready = threading.Event()
signal.signal(signal.SIGUSR1, lambda _signum, _frame: start.set())
print(json.dumps({"pid": os.getpid()}), flush=True)

result = {}
early_thread = os.environ.get("KUTRACE_THREAD_BEFORE_ATTACH") == "1"


def worker():
    result["tid"] = threading.get_native_id()
    ctypes.CDLL(None).prctl(15, b"kut-new-thread", 0, 0, 0)
    ready.set()
    if early_thread and not start.wait(10):
        result["error"] = "timed out waiting for collector attachment"
        return
    print(json.dumps(result), flush=True)
    deadline = time.monotonic() + 0.75
    while time.monotonic() < deadline:
        os.getpid()
        time.sleep(0.0001)


if not early_thread and not start.wait(10):
    raise SystemExit("timed out waiting for collector attachment")
thread = threading.Thread(target=worker)
thread.start()
if early_thread:
    if not ready.wait(10):
        raise SystemExit("timed out creating fixture thread")
    print(json.dumps({"ready": True, **result}), flush=True)
thread.join()
if "error" in result:
    raise SystemExit(result["error"])
print(json.dumps({"complete": True, **result}), flush=True)
