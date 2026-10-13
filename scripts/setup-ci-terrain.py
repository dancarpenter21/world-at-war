#!/usr/bin/env python3
"""Create explicitly synthetic flat terrain in a fresh CI-only directory."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
data = bytes(1201 * 1201 * 2)
digest = hashlib.sha256(data).hexdigest()
tiles = []
for latitude in (37, 38):
    for longitude in (-78, -77, -76):
        filename = f"N{latitude}W{abs(longitude):03d}.hgt"
        (args.output / filename).write_bytes(data)
        tiles.append(dict(file=filename, latitude=latitude, longitude=longitude,
                          samples=1201, sha256=digest))
(args.output / "manifest.json").write_text(json.dumps(dict(
    version=1, source="Synthetic flat terrain for CI; not real SRTM acceptance evidence",
    vertical_datum="EGM96", tiles=tiles)) + "\n")
