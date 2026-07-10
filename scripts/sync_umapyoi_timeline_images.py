#!/usr/bin/env python3
"""Download new timeline image references once and store them as frontend WebP assets."""

from __future__ import annotations

import argparse
import io
import json
import os
import time
import urllib.error
import urllib.request
from pathlib import Path

from PIL import Image


USER_AGENT = "umamoe-resources-image-sync/1.0 (+https://uma.moe)"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeline-json", required=True, type=Path)
    parser.add_argument("--frontend-root", required=True, type=Path)
    parser.add_argument("--request-interval-ms", type=int, default=250)
    return parser.parse_args()


def image_jobs(timeline_path: Path, frontend_root: Path) -> list[tuple[str, Path]]:
    timeline = json.loads(timeline_path.read_text(encoding="utf-8"))
    jobs: dict[Path, str] = {}
    for event in timeline.get("events", []):
        source_url = event.get("image")
        asset_path = event.get("image_path")
        if not isinstance(source_url, str) or not source_url.startswith(("https://", "http://")):
            continue
        if not isinstance(asset_path, str) or not asset_path.startswith(
            ("assets/images/", "assets/timeline-images/")
        ):
            continue
        if not asset_path.lower().endswith(".webp"):
            continue
        target = frontend_root / "src" / Path(asset_path)
        jobs.setdefault(target, source_url)
    return sorted(((url, path) for path, url in jobs.items()), key=lambda job: str(job[1]))


def download(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            if error.code != 429 and error.code < 500:
                raise
        except urllib.error.URLError:
            if attempt == 3:
                raise
        time.sleep(2**attempt)
    raise RuntimeError(f"failed to download {url}")


def write_webp(payload: bytes, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_suffix(".webp.tmp")
    with Image.open(io.BytesIO(payload)) as source:
        source.load()
        has_alpha = source.mode in {"RGBA", "LA"} or "transparency" in source.info
        converted = source.convert("RGBA" if has_alpha else "RGB")
        converted.save(
            temporary,
            format="WEBP",
            lossless=has_alpha,
            quality=88,
            method=6,
        )
    os.replace(temporary, target)


def main() -> int:
    args = parse_args()
    jobs = image_jobs(args.timeline_json, args.frontend_root)
    created = 0
    skipped = 0
    failures: list[str] = []
    interval = max(args.request_interval_ms, 0) / 1000

    for source_url, target in jobs:
        if target.is_file() and target.stat().st_size > 0:
            skipped += 1
            continue
        try:
            write_webp(download(source_url), target)
            created += 1
            print(f"created {target}")
        except Exception as error:  # continue so one broken historical image does not lose the batch
            failures.append(f"{source_url}: {error}")
        if interval:
            time.sleep(interval)

    print(f"timeline images: {created} created, {skipped} already present, {len(failures)} failed")
    for failure in failures:
        print(f"warning: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
