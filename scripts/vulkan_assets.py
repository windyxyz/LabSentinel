"""Verify the pinned Windows Vulkan probe assets before staging a gray-test package."""

import hashlib
from io import BytesIO
import json
import lzma
from pathlib import Path
from zipfile import ZipFile


ROOT = Path(__file__).resolve().parent.parent
MAX_INCREMENT = 15_000_000


def verify(data, metadata, name):
    if len(data) != metadata["size"] or hashlib.sha256(data).hexdigest() != metadata["sha256"]:
        raise ValueError(f"{name} differs from the pinned Vulkan asset")
    return data


def read_verified(path, metadata):
    with path.open("rb") as stream:
        return verify(stream.read(metadata["size"] + 1), metadata, path.name)


def read_bundle(path, source_models):
    lock = json.loads((ROOT / "scripts" / "assets.lock.json").read_text(encoding="utf-8"))["vulkan_probe"]
    for name, metadata in lock["sources"].items():
        read_verified(source_models / name, metadata)
    bundle = read_verified(path, lock["bundle"])
    if len(bundle) > MAX_INCREMENT:
        raise ValueError("Vulkan probe exceeds the Windows package increment budget")
    with ZipFile(BytesIO(bundle)) as archive:
        members = lock["members"]
        names = archive.namelist()
        if len(names) != len(members) or set(names) != set(members):
            raise ValueError("Vulkan probe ZIP has unexpected members")
        files = {}
        for name, metadata in members.items():
            if archive.getinfo(name).file_size != metadata["size"]:
                raise ValueError(f"Unexpected archived size for {name}")
            files[name] = verify(archive.read(name), metadata, name)
    decoder = lzma.LZMADecompressor(format=lzma.FORMAT_XZ, memlimit=64 * 1024 * 1024)
    dll = decoder.decompress(files["ncnn.dll.xz"], max_length=lock["dll"]["size"] + 1)
    if not decoder.eof or decoder.unused_data:
        raise ValueError("ncnn.dll.xz must contain one complete XZ stream")
    verify(dll, lock["dll"], "ncnn.dll")
    files["ncnn-LICENSE.txt"] = (ROOT / "docs" / "licenses" / "ncnn-BSD-3-Clause.txt").read_bytes()
    return files
