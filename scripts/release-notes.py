#!/usr/bin/env python3
"""Extract one exact version from CHANGELOG.md for GitHub release publication."""

import argparse
from pathlib import Path
import re


def release_notes(changelog: str, tag: str) -> str:
    version = tag.removeprefix("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError(f"Invalid release tag: {tag!r}")
    headings = list(re.finditer(r"^## (.+)$", changelog, re.MULTILINE))
    for index, heading in enumerate(headings):
        heading_version = heading.group(1).split(" - ", 1)[0].strip("[]")
        if heading_version != version:
            continue
        end = headings[index + 1].start() if index + 1 < len(headings) else len(changelog)
        notes = changelog[heading.end():end].strip()
        if not notes:
            raise ValueError(f"Empty changelog entry for {tag}")
        return f"## What's new in {tag}\n\n{notes}\n"
    raise ValueError(f"No changelog entry for {tag}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag")
    parser.add_argument("--changelog", type=Path, default=Path(__file__).resolve().parents[1] / "CHANGELOG.md")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        notes = release_notes(args.changelog.read_text(), args.tag)
    except ValueError as error:
        parser.error(str(error))
    if args.output:
        args.output.write_text(notes)
    else:
        print(notes, end="")


if __name__ == "__main__":
    main()
