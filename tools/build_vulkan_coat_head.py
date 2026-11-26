"""Build the embedded Vulkan coat head from a bundled ONNX and a pinned pnnx conversion.

Usage: python tools/build_vulkan_coat_head.py SOURCE.onnx HEAD.param HEAD.bin OUTPUT.xz
Requires NumPy. Each reference is a byte range in SOURCE.onnx. The resulting
weights must reconstruct byte for byte before the tool emits the asset lock.
"""

import hashlib
import json
import lzma
from pathlib import Path
import struct
import sys

import numpy as np


def varint(data, offset):
    value = 0
    for shift in range(0, 70, 7):
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
    raise ValueError("invalid protobuf varint")


def fields(data, start, end):
    offset = start
    while offset < end:
        tag, offset = varint(data, offset)
        number, wire = divmod(tag, 8)
        if wire == 2:
            length, offset = varint(data, offset)
            yield number, (offset, offset + length)
            offset += length
        elif wire == 0:
            value, offset = varint(data, offset)
            yield number, value
        elif wire in (1, 5):
            offset += {1: 8, 5: 4}[wire]
        else:
            raise ValueError(f"unsupported protobuf wire type {wire}")
    if offset != end:
        raise ValueError("protobuf field extends beyond parent message")


def ncnn_half(raw):
    bits = np.frombuffer(raw, dtype="<u4")
    exponent = ((bits >> 23) & 255).astype(np.int32) - 112
    sign = (bits >> 16) & 0x8000
    fraction = bits & 0x7fffff
    rounded = (exponent << 10) + ((fraction + 0x1000) >> 13)
    half = np.where(exponent <= 0, 0, np.where(exponent >= 31, 0x7c00, rounded))
    nan = ((exponent == 143) & (fraction != 0)) * 0x200
    return (sign | half | nan).astype("<u2").tobytes()


def encode(mode, raw):
    if mode == 1:
        return raw
    if mode == 2:
        return np.frombuffer(raw, dtype="<f4").astype("<f2").tobytes()
    return ncnn_half(raw)


def patch(source, target):
    graph, = (value for field, value in fields(source, 0, len(source)) if field == 7)
    matches = []
    for field, initializer in fields(source, *graph):
        if field != 5:
            continue
        values = dict(fields(source, *initializer))
        if values.get(2) != 1 or not isinstance(raw_range := values.get(9), tuple):
            continue
        start, end = raw_range
        if end - start < 64:
            continue
        for mode in (1, 2, 3):
            candidate = encode(mode, source[start:end])
            position = target.find(candidate)
            if position >= 0:
                matches.append((position, len(candidate), start, end - start, mode))
    data = bytearray()
    cursor = 0
    for position, length, start, count, mode in sorted(matches, key=lambda row: (row[0], -row[1])):
        if position < cursor:
            continue
        if position > cursor:
            data.extend(struct.pack("<BII", 0, 0, position - cursor))
            data.extend(target[cursor:position])
        data.extend(struct.pack("<BII", mode, start, count))
        cursor = position + length
    if cursor < len(target):
        data.extend(struct.pack("<BII", 0, 0, len(target) - cursor))
        data.extend(target[cursor:])
    return bytes(data)


def rebuild(source, delta):
    output = bytearray()
    position = 0
    while position < len(delta):
        mode, start, count = struct.unpack_from("<BII", delta, position)
        position += 9
        if mode == 0:
            output.extend(delta[position:position + count])
            position += count
        else:
            output.extend(encode(mode, source[start:start + count]))
    return bytes(output)


def main():
    if len(sys.argv) != 5:
        raise SystemExit(__doc__)
    source, param, weights = (Path(path).read_bytes() for path in sys.argv[1:4])
    delta = patch(source, weights)
    if rebuild(source, delta) != weights:
        raise ValueError("reconstruction does not match original ncnn weights")
    expanded = param + delta
    bundle = lzma.compress(expanded, preset=6)
    Path(sys.argv[4]).write_bytes(bundle)
    print(json.dumps({
        name: {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        for name, data in (("source", source), ("bundle", bundle), ("expanded", expanded),
                           ("param", param), ("bin", weights))
    }, indent=2))


if __name__ == "__main__":
    main()
