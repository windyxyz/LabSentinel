"""Pack or restore the LS-086 Windows ncnn probe assets with byte-for-byte verification."""

import argparse
import hashlib
from io import BytesIO
import json
import lzma
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile, ZipInfo


MAX_BUNDLE = 15_000_000
MAX_DLL = 32 * 1024 * 1024
MAX_MODEL = 8 * 1024 * 1024
DLL = "ncnn.dll"
PACKED_DLL = DLL + ".xz"
MODEL_FILES = (
    "spike-yolo11n-fp16.ncnn.param",
    "spike-yolo11n-fp16.ncnn.bin",
    "spike-yolo11n_ppe-fp16.ncnn.param",
    "spike-yolo11n_ppe-fp16.ncnn.bin",
)
MANIFEST = "manifest.json"


def fingerprint(data):
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def pack(source, destination):
    files = {}
    for name in (DLL, *MODEL_FILES):
        path = source / name
        if not 0 < path.stat().st_size <= (MAX_DLL if name == DLL else MAX_MODEL):
            raise ValueError(f"invalid source asset size: {path}")
        files[name] = path.read_bytes()
    manifest = {name: fingerprint(data) for name, data in files.items()}
    stream = BytesIO()
    with ZipFile(stream, "w", compression=ZIP_DEFLATED, compresslevel=6) as archive:
        def write(name, data, method=ZIP_DEFLATED):
            info = ZipInfo(name)
            info.create_system = 0
            archive.writestr(info, data, compress_type=method, compresslevel=6)

        write(PACKED_DLL, lzma.compress(files[DLL], preset=6), ZIP_STORED)
        for name in MODEL_FILES:
            write(name, files[name])
        write(MANIFEST, json.dumps(manifest, sort_keys=True) + "\n")
    payload = stream.getbuffer()
    check_size(payload.nbytes)
    with destination.open("xb") as output:
        output.write(payload)
    print(f"{destination}: {payload.nbytes:,} bytes")


def check_size(size):
    if not 0 < size <= MAX_BUNDLE:
        raise ValueError(f"archive is {size:,} bytes, limit is {MAX_BUNDLE:,}")


def read_member(archive, name, limit):
    member = archive.getinfo(name)
    if not 0 < member.file_size <= limit:
        raise ValueError(f"invalid size for {name}: {member.file_size}")
    return archive.read(member)


def unpack(source, destination):
    check_size(source.stat().st_size)
    with ZipFile(source) as archive:
        names = archive.namelist()
        expected = {PACKED_DLL, MANIFEST, *MODEL_FILES}
        if len(names) != len(expected) or set(names) != expected:
            raise ValueError("archive members do not match the LS-086 asset set")
        manifest = json.loads(read_member(archive, MANIFEST, 4096))
        entries = {name: read_member(archive, name, MAX_MODEL) for name in MODEL_FILES}
        decoder = lzma.LZMADecompressor(format=lzma.FORMAT_XZ, memlimit=64 * 1024 * 1024)
        entries[DLL] = decoder.decompress(read_member(archive, PACKED_DLL, MAX_BUNDLE), max_length=MAX_DLL + 1)
        if not decoder.eof or decoder.unused_data or len(entries[DLL]) > MAX_DLL:
            raise ValueError("invalid XZ payload for ncnn.dll")
        if manifest != {name: fingerprint(data) for name, data in entries.items()}:
            raise ValueError("one or more unpacked assets failed the manifest check")
    destination.mkdir()
    for name, data in entries.items():
        (destination / name).write_bytes(data)
    print(f"{destination}: unpacked and verified {len(entries)} assets")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("pack", "unpack"))
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    if args.command == "pack":
        pack(args.source, args.destination)
    else:
        unpack(args.source, args.destination)


if __name__ == "__main__":
    main()
