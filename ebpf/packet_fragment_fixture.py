#!/usr/bin/env python3
import json
import socket
import struct
import time

IPV4_PAYLOAD = b"KUtrace-IPv4-fragment-payload-0123456789"
IPV6_PAYLOAD = b"KUtrace-IPv6-fragment-payload-ABCDEFGHIJ"
IPV6_DEST_PAYLOAD = b"Destination-extension-payload-0123456789"[:40]
IPV6_AH_PAYLOAD = b"Authentication-header-payload-abcdefghij"
IPV6_LONG_DEST_PAYLOAD = b"Seven-destination-headers-payload-0123456789"[:40]
IPV6_OVERBOUND_PAYLOAD = b"Eight-destination-header-payload-abcdefghij"[:40]
assert len(IPV4_PAYLOAD) == 40
assert len(IPV6_PAYLOAD) == 40
assert len(IPV6_DEST_PAYLOAD) == 40
assert len(IPV6_AH_PAYLOAD) == 40
assert len(IPV6_LONG_DEST_PAYLOAD) == 40
assert len(IPV6_OVERBOUND_PAYLOAD) == 40


def checksum(data):
    if len(data) & 1:
        data += b"\0"
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    total = (total & 0xFFFF) + (total >> 16)
    total = (total & 0xFFFF) + (total >> 16)
    return (~total) & 0xFFFF


def payload_hash(payload):
    return struct.unpack("<8I", payload[:32])[0] ^ struct.unpack("<8I", payload[:32])[1] ^ struct.unpack("<8I", payload[:32])[2] ^ struct.unpack("<8I", payload[:32])[3] ^ struct.unpack("<8I", payload[:32])[4] ^ struct.unpack("<8I", payload[:32])[5] ^ struct.unpack("<8I", payload[:32])[6] ^ struct.unpack("<8I", payload[:32])[7]


def ipv4_header(length, identification, fragment):
    source = socket.inet_aton("127.0.0.1")
    header = struct.pack(
        "!BBHHHBBH4s4s",
        0x45,
        0,
        20 + length,
        identification,
        fragment,
        64,
        socket.IPPROTO_UDP,
        0,
        source,
        source,
    )
    return header[:10] + struct.pack("!H", checksum(header)) + header[12:]


def send_ipv4():
    udp = struct.pack("!HHHH", 31001, 31002, 48, 0) + IPV4_PAYLOAD
    pieces = [(16, True, udp[16:32]), (32, False, udp[32:]), (0, True, udp[:16])]
    with socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_RAW) as sender:
        sender.setsockopt(socket.IPPROTO_IP, socket.IP_HDRINCL, 1)
        for offset, more, data in pieces:
            fragment = (0x2000 if more else 0) | (offset // 8)
            sender.sendto(ipv4_header(len(data), 0x4B55, fragment) + data, ("127.0.0.1", 0))
            time.sleep(0.01)


def send_ipv6():
    source = socket.inet_pton(socket.AF_INET6, "::1")
    udp_zero = struct.pack("!HHHH", 32001, 32002, 48, 0)
    pseudo = source + source + struct.pack("!I3xB", 48, socket.IPPROTO_UDP)
    udp = struct.pack("!HHHH", 32001, 32002, 48, checksum(pseudo + udp_zero + IPV6_PAYLOAD))
    udp += IPV6_PAYLOAD
    pieces = [(16, True, udp[16:32]), (32, False, udp[32:]), (0, True, udp[:16])]
    with socket.socket(socket.AF_INET6, socket.SOCK_RAW, socket.IPPROTO_RAW) as sender:
        for offset, more, data in pieces:
            fragment = struct.pack("!BBHI", socket.IPPROTO_UDP, 0, offset | int(more), 0x4B555452)
            header = struct.pack("!IHBB16s16s", 6 << 28, 8 + len(data), 44, 64, source, source)
            sender.sendto(header + fragment + data, ("::1", 0))
            time.sleep(0.01)


def send_ipv6_post_fragment(payload, source_port, identification, next_header, prefix, pieces):
    source = socket.inet_pton(socket.AF_INET6, "::1")
    udp_zero = struct.pack("!HHHH", source_port, source_port + 1, 48, 0)
    pseudo = source + source + struct.pack("!I3xB", 48, socket.IPPROTO_UDP)
    udp = struct.pack(
        "!HHHH",
        source_port,
        source_port + 1,
        48,
        checksum(pseudo + udp_zero + payload),
    ) + payload
    fragmentable = prefix + udp
    with socket.socket(socket.AF_INET6, socket.SOCK_RAW, socket.IPPROTO_RAW) as sender:
        for offset, more, end in pieces:
            data = fragmentable[offset:end]
            fragment = struct.pack("!BBHI", next_header, 0, offset | int(more), identification)
            header = struct.pack("!IHBB16s16s", 6 << 28, 8 + len(data), 44, 64, source, source)
            sender.sendto(header + fragment + data, ("::1", 0))
            time.sleep(0.01)


def destination_prefix(count):
    return b"".join(
        struct.pack("!BB6s", 60 if index + 1 < count else socket.IPPROTO_UDP, 0, b"\0" * 6)
        for index in range(count)
    )


send_ipv4()
send_ipv6()
destination = struct.pack("!BB6s", socket.IPPROTO_UDP, 0, b"\0" * 6)
send_ipv6_post_fragment(
    IPV6_DEST_PAYLOAD,
    33001,
    0x4B555453,
    60,
    destination,
    [(16, True, 32), (32, False, 56), (0, True, 16)],
)
authentication = struct.pack("!BBHII", socket.IPPROTO_UDP, 1, 0, 0x12345678, 1)
send_ipv6_post_fragment(
    IPV6_AH_PAYLOAD,
    34001,
    0x4B555454,
    51,
    authentication,
    [(24, True, 40), (40, False, 60), (0, True, 24)],
)
send_ipv6_post_fragment(
    IPV6_LONG_DEST_PAYLOAD,
    35001,
    0x4B555455,
    60,
    destination_prefix(7),
    [(64, True, 80), (80, False, 104), (0, True, 64)],
)
send_ipv6_post_fragment(
    IPV6_OVERBOUND_PAYLOAD,
    36001,
    0x4B555456,
    60,
    destination_prefix(8),
    [(72, True, 88), (88, False, 112), (0, True, 72)],
)
print(json.dumps({
    "ipv4_hash": payload_hash(IPV4_PAYLOAD),
    "ipv6_hash": payload_hash(IPV6_PAYLOAD),
    "ipv6_destination_hash": payload_hash(IPV6_DEST_PAYLOAD),
    "ipv6_ah_hash": payload_hash(IPV6_AH_PAYLOAD),
    "ipv6_long_destination_hash": payload_hash(IPV6_LONG_DEST_PAYLOAD),
    "ipv6_overbound_hash": payload_hash(IPV6_OVERBOUND_PAYLOAD),
}))
