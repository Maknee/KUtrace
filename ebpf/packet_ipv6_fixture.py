#!/usr/bin/env python3
import socket
import struct

SOURCE = socket.inet_pton(socket.AF_INET6, "::1")
DESTINATION = SOURCE
PAYLOAD = b"KUtrace-IPv6-extension-payload!!"
assert len(PAYLOAD) == 32


def checksum(data):
    if len(data) % 2:
        data += b"\0"
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    total = (total & 0xFFFF) + (total >> 16)
    total = (total & 0xFFFF) + (total >> 16)
    return (~total) & 0xFFFF


udp_without_checksum = struct.pack("!HHHH", 12345, 23456, 40, 0)
pseudo_header = SOURCE + DESTINATION + struct.pack("!I3xB", 40, 17)
udp_checksum = checksum(pseudo_header + udp_without_checksum + PAYLOAD)
udp = struct.pack("!HHHH", 12345, 23456, 40, udp_checksum) + PAYLOAD


def ipv6(next_header, body):
    header = struct.pack(
        "!IHBB16s16s",
        0x60000000,
        len(body),
        next_header,
        64,
        SOURCE,
        DESTINATION,
    )
    return header + body


hop_by_hop = bytes([17, 0]) + b"\0" * 6
atomic_fragment = bytes([17, 0, 0, 0]) + struct.pack("!I", 0x12345678)
noninitial_fragment = bytes([17, 0, 0, 8]) + struct.pack("!I", 0x87654321)


def destination_chain(count):
    body = udp
    next_header = socket.IPPROTO_UDP
    for _ in range(count):
        body = bytes([next_header, 0]) + b"\0" * 6 + body
        next_header = 60
    return ipv6(next_header, body)


packets = [
    ipv6(0, hop_by_hop + udp),
    destination_chain(5),
    destination_chain(8),
    destination_chain(9),
    ipv6(44, atomic_fragment + udp),
    ipv6(44, noninitial_fragment + udp),
]

with socket.socket(socket.AF_INET6, socket.SOCK_RAW, socket.IPPROTO_RAW) as sender:
    for packet in packets:
        sender.sendto(packet, ("::1", 0))
