#!/usr/bin/env python3
"""Extract a type-2 AppImage without executing its ELF runtime (for Rosetta)."""
import argparse
import mmap
from pathlib import Path
import subprocess


def extract(image: Path, destination: Path):
    if destination.exists() and any(destination.iterdir()):
        raise ValueError("AppImage extraction destination must be empty.")
    with image.open("rb") as file, mmap.mmap(file.fileno(), 0, access=mmap.ACCESS_READ) as data:
        if data[:4] != b"\x7fELF" or data[8:11] != b"AI\x02":
            raise ValueError("Expected a type-2 AppImage.")
        offset = data.find(b"hsqs")
        while offset != -1:
            # The runtime can contain the magic bytes too. Ask unsquashfs to
            # validate each candidate superblock rather than assuming the first.
            probe = subprocess.run(
                ["unsquashfs", "-s", "-o", str(offset), str(image)],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            if probe.returncode == 0:
                # unsquashfs creates its destination itself, even when callers
                # reserve the path with mktemp -d.
                if destination.exists():
                    destination.rmdir()
                subprocess.run(
                    ["unsquashfs", "-no-progress", "-d", str(destination), "-o", str(offset), str(image)],
                    stdout=subprocess.DEVNULL, check=True,
                )
                return
            offset = data.find(b"hsqs", offset + 1)
    raise ValueError("No valid SquashFS filesystem found in AppImage.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    extract(args.image, args.destination)
