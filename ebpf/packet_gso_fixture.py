#!/usr/bin/env python3
import hashlib
import json
import socket
import struct
import threading

PORT = 34567
SEGMENT_SIZE = 64
SEGMENTS = [
    hashlib.sha256(f"kutrace-gso-{index}".encode()).digest() + bytes([index]) * 32
    for index in range(4)
]


def payload_hash(payload):
    value = 0
    for offset in range(0, 32, 4):
        value ^= struct.unpack_from("<I", payload, offset)[0]
    return value


received = []
gro_size = 0


def receive():
    global gro_size
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as receiver:
        receiver.setsockopt(socket.IPPROTO_UDP, 104, 1)
        receiver.bind(("127.0.0.1", PORT))
        receiver.settimeout(2)
        payload, ancillary, _, _ = receiver.recvmsg(1024, 1024)
        for level, kind, value in ancillary:
            if level == socket.IPPROTO_UDP and kind == 104:
                gro_size = struct.unpack("I", value)[0]
        received.extend(
            payload[offset : offset + gro_size]
            for offset in range(0, len(payload), gro_size)
        )


thread = threading.Thread(target=receive)
thread.start()
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sender:
    sent = sender.sendmsg(
        [b"".join(SEGMENTS)],
        [(socket.IPPROTO_UDP, 103, struct.pack("H", SEGMENT_SIZE))],
        0,
        ("127.0.0.1", PORT),
    )
thread.join()

assert sent == len(SEGMENTS) * SEGMENT_SIZE
assert gro_size == SEGMENT_SIZE
assert received == SEGMENTS
print(
    json.dumps(
        {
            "segments": len(SEGMENTS),
            "segment_size": SEGMENT_SIZE,
            "gro_size": gro_size,
            "hashes": [payload_hash(segment) for segment in SEGMENTS],
        }
    )
)
