#!/usr/bin/env python3
"""Prune superseded build artifacts from a Cargo `target/` directory.

Every `cargo test` leaves each test executable (and, on Windows, its `.pdb`,
which with `debuginfo=2` runs to hundreds of MB) under a fresh content hash in
`target/<profile>/deps`. Cargo never removes the old hashes, so a workspace
with ~250 test targets grows by tens of GB per full-suite run and eventually
fills the disk (`LNK1180: insufficient disk space to complete link`).

This keeps, per target stem, only the newest hash of the executable-shaped
outputs (`.exe`/`.pdb`/`.d` on Windows; the extensionless binary and `.d` on
Unix) and deletes the rest. Library outputs (`.rlib`/`.rmeta`/`.so`/`.dll`)
are left alone: dependents pin them by hash and Cargo rebuilds what it needs.
With `--incremental` it also removes `target/<profile>/incremental`, which is a
pure cache. Both are safe to run at any time except while a build is running.

A one-line summary is appended to `<target>/prune.log` so an unattended run
leaves a trace.

    python scripts/prune_target.py --repo . --dry-run
    python scripts/prune_target.py --repo . --incremental
"""

from __future__ import annotations

import argparse
import datetime as dt
import os
import re
import shutil
import sys
from collections import defaultdict
from pathlib import Path

HASHED = re.compile(r"^(?P<stem>.+)-(?P<hash>[0-9a-f]{16})(?P<ext>\.exe|\.pdb|\.d)?$")
PROFILES = ("debug", "release")


def prune_deps(deps: Path, dry_run: bool) -> tuple[int, int, int]:
    """Delete every non-newest hash of each executable-shaped stem in `deps`.

    Returns (targets kept, files removed, bytes freed)."""
    groups: dict[str, dict[str, dict[str, tuple[Path, int, float]]]] = defaultdict(dict)
    for entry in os.scandir(deps):
        if not entry.is_file():
            continue
        m = HASHED.match(entry.name)
        if not m:
            continue
        ext = m["ext"] or ""
        # Extensionless files are Unix executables; on Windows every
        # executable-shaped output carries one of the three extensions.
        if ext == "" and os.name == "nt":
            continue
        st = entry.stat()
        groups[m["stem"]].setdefault(m["hash"], {})[ext] = (Path(entry.path), st.st_size, st.st_mtime)

    kept = removed = freed = 0
    for stem, by_hash in groups.items():
        def newest(h: str) -> float:
            files = by_hash[h]
            primary = files.get(".exe") or files.get("") or next(iter(files.values()))
            return primary[2]

        keep = max(by_hash, key=newest)
        for h, files in by_hash.items():
            if h == keep:
                kept += 1
                continue
            for _ext, (path, size, _mtime) in files.items():
                if not dry_run:
                    try:
                        path.unlink()
                    except OSError as err:
                        print(f"skip {path}: {err}", file=sys.stderr)
                        continue
                removed += 1
                freed += size
    return kept, removed, freed


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--repo", default=".", help="workspace root containing target/")
    ap.add_argument("--target", default=None, help="explicit target dir (overrides --repo)")
    ap.add_argument("--incremental", action="store_true", help="also remove <profile>/incremental")
    ap.add_argument("--dry-run", action="store_true", help="report only; delete nothing")
    args = ap.parse_args()

    target = Path(args.target) if args.target else Path(args.repo) / "target"
    if not target.is_dir():
        print(f"no target dir at {target}", file=sys.stderr)
        return 0

    total_kept = total_removed = total_freed = 0
    for profile in PROFILES:
        deps = target / profile / "deps"
        if deps.is_dir():
            kept, removed, freed = prune_deps(deps, args.dry_run)
            total_kept += kept
            total_removed += removed
            total_freed += freed
        incr = target / profile / "incremental"
        if args.incremental and incr.is_dir():
            size = sum(p.stat().st_size for p in incr.rglob("*") if p.is_file())
            if not args.dry_run:
                shutil.rmtree(incr, ignore_errors=True)
            total_freed += size

    line = (
        f"{dt.datetime.now():%Y-%m-%d %H:%M:%S} "
        f"{'DRY-RUN ' if args.dry_run else ''}kept={total_kept} removed={total_removed} "
        f"freed={total_freed / 1e9:.1f}GB incremental={'yes' if args.incremental else 'no'}"
    )
    print(line)
    if not args.dry_run:
        try:
            with open(target / "prune.log", "a", encoding="utf-8") as log:
                log.write(line + "\n")
        except OSError:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
