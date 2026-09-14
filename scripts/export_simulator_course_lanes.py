#!/usr/bin/env python3
"""Package retained course-lanes.json overruns for the simulator resource API."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("source", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
source = args.source.read_bytes()
document = json.loads(source)
courses = [
    {"courseId": course["courseId"], "overrun": course["overrun"]}
    for course in document["courses"] if course.get("overrun")
]
names = {course["overrun"] for course in courses}
assets = [asset for asset in document["assets"] if asset["resourcePath"] in names]
if not courses or {asset["resourcePath"] for asset in assets} != names:
    raise ValueError("missing overrun courses or assets")
if len({course["courseId"] for course in courses}) != len(courses):
    raise ValueError("duplicate course ID")
if len(assets) != len(names):
    raise ValueError("duplicate overrun asset")
payload = {
    "schema_version": 1,
    "source_sha256": hashlib.sha256(source).hexdigest(),
    "courses": courses,
    "assets": assets,
}
encoded = json.dumps(payload, allow_nan=False, separators=(",", ":")).encode()
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_bytes(gzip.compress(encoded, mtime=0))
print(f"Wrote {len(courses)} courses and {len(assets)} overrun assets")
