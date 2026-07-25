#!/usr/bin/env python3
"""Build the bundled JP mission-campaign catalogue from a JP master database."""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import re
import sqlite3
from datetime import datetime, timezone
from pathlib import Path
from zoneinfo import ZoneInfo

from PIL import Image


SIGNATURE_COLUMNS = (
    "condition_type",
    "condition_value_1",
    "condition_value_2",
    "condition_value_3",
    "condition_value_4",
    "condition_num",
    "item_category",
    "item_id",
    "item_num",
)
TOKYO = ZoneInfo("Asia/Tokyo")
JP_CAMPAIGN_ASSET_PATTERN = re.compile(
    r"tex_campaign_mission_logo_(\d+)\.png$", re.IGNORECASE
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--jp-master",
        type=Path,
        help=(
            "JP master.mdb used to discover and fingerprint mission groups; may be omitted "
            "when only restoring images already present in the catalogue"
        ),
    )
    parser.add_argument("--frontend-root", required=True, type=Path)
    parser.add_argument(
        "--jp-image-source",
        type=Path,
        default=Path(os.environ["UMAMUSUME_JP_CAMPAIGN_ASSET_DIR"])
        if os.environ.get("UMAMUSUME_JP_CAMPAIGN_ASSET_DIR")
        else None,
        help=(
            "directory containing AssetRipper tex_campaign_mission_logo_<id>.png files; "
            "also read from UMAMUSUME_JP_CAMPAIGN_ASSET_DIR"
        ),
    )
    parser.add_argument(
        "--catalog",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "src"
        / "jp_data"
        / "timeline_campaigns.json",
    )
    parser.add_argument(
        "--planner-rewards",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "src"
        / "jp_data"
        / "planner_mission_rewards.json",
        help="protected planner-only JP mission reward catalogue",
    )
    return parser.parse_args()


def parse_master_date(value: str) -> datetime:
    for pattern in ("%Y/%m/%d %H:%M:%S", "%Y/%m/%d %H:%M"):
        try:
            return datetime.strptime(value, pattern)
        except ValueError:
            continue
    raise ValueError(f"unsupported master.mdb date {value!r}")


def utc_catalog_date(value: str) -> str:
    local = parse_master_date(value).replace(tzinfo=TOKYO)
    return local.astimezone(timezone.utc).replace(tzinfo=None).isoformat(timespec="seconds")


def mission_fingerprint(missions: collections.Counter[tuple[int, ...]]) -> str:
    lines = []
    for signature, count in sorted(missions.items()):
        lines.extend(",".join(map(str, signature)) for _ in range(count))
    return hashlib.sha256("\n".join(lines).encode()).hexdigest()


def repair_mojibake(value: str) -> str:
    if any("\u3040" <= char <= "\u30ff" or "\u4e00" <= char <= "\u9fff" for char in value):
        return value
    for encoding in ("cp1252", "latin1"):
        try:
            repaired = value.encode(encoding).decode("utf-8")
        except (UnicodeEncodeError, UnicodeDecodeError):
            continue
        if any("\u3040" <= char <= "\u30ff" or "\u4e00" <= char <= "\u9fff" for char in repaired):
            return repaired
    return value


def load_mission_groups(connection: sqlite3.Connection) -> dict[int, dict]:
    connection.row_factory = sqlite3.Row
    titles = {
        row["index"]: repair_mojibake(row["text"].replace("\\n", " "))
        for row in connection.execute(
            'SELECT "index", text FROM text_data WHERE category = 187'
        )
    }
    groups: dict[int, dict] = {}
    for row in connection.execute(
        "SELECT * FROM mission_data WHERE mission_type = 4 AND event_id > 0 ORDER BY id"
    ):
        group = groups.setdefault(
            row["event_id"],
            {
                "id": row["event_id"],
                "title": titles.get(row["event_id"]),
                "start_date": row["start_date"],
                "end_date": row["end_date"],
                "missions": collections.Counter(),
                "rewards": collections.Counter(),
                "reward_mission_counts": collections.Counter(),
            },
        )
        if parse_master_date(row["start_date"]) < parse_master_date(group["start_date"]):
            group["start_date"] = row["start_date"]
        if parse_master_date(row["end_date"]) > parse_master_date(group["end_date"]):
            group["end_date"] = row["end_date"]
        group["missions"][tuple(row[column] for column in SIGNATURE_COLUMNS)] += 1
        reward_key = (int(row["item_category"]), int(row["item_id"]))
        group["rewards"][reward_key] += int(row["item_num"])
        group["reward_mission_counts"][reward_key] += 1
    for group in groups.values():
        group["mission_count"] = sum(group["missions"].values())
        group["mission_fingerprint"] = mission_fingerprint(group["missions"])
        group["structured_rewards"] = [
            {
                "item_category": item_category,
                "item_id": item_id,
                "amount": amount,
                "mission_count": group["reward_mission_counts"][(item_category, item_id)],
            }
            for (item_category, item_id), amount in sorted(group["rewards"].items())
            if amount > 0
        ]
    return groups


def attach_group(row: dict, group: dict) -> None:
    row["start_date"] = utc_catalog_date(group["start_date"])
    row["end_date"] = utc_catalog_date(group["end_date"])
    row["jp_mission_event_id"] = group["id"]
    row["jp_title"] = group["title"]
    row["mission_count"] = group["mission_count"]
    row["mission_fingerprint"] = group["mission_fingerprint"]


def attach_missing_group_mapping(row: dict, group: dict) -> None:
    if row.get("jp_mission_event_id") is None:
        row["jp_mission_event_id"] = group["id"]
    if not row.get("jp_title"):
        row["jp_title"] = group["title"]


def merge_existing_rows(existing: list[dict], groups_by_day: dict[str, list[dict]]) -> tuple[list[dict], set[int]]:
    rows = []
    mapped_group_ids: set[int] = set()
    for original in existing:
        row = dict(original)
        day = str(row["start_date"])[:10]
        candidates = groups_by_day.get(day, [])
        exact_time = [
            group for group in candidates if group["catalog_start_date"] == row["start_date"]
        ]
        if exact_time:
            candidates = exact_time
        exact = [group for group in candidates if group["id"] == row["campaign_id"]]
        group = exact[0] if len(exact) == 1 else candidates[0] if len(candidates) == 1 else None
        if group is not None:
            mapped_group_ids.add(group["id"])
            existing_count = row.get("mission_count")
            existing_fingerprint = row.get("mission_fingerprint")
            if isinstance(existing_count, int) and existing_fingerprint:
                # The standalone JP snapshot may be historical. Without an
                # explicit freshness proof, never replace an already populated
                # public signature, even when the row count happens to match.
                # The protected planner snapshot is still written separately.
                attach_missing_group_mapping(row, group)
            else:
                attach_group(row, group)
        rows.append(row)
    return rows, mapped_group_ids


def write_planner_reward_catalog(path: Path, groups: dict[int, dict]) -> None:
    rows = [
        {
            "jp_mission_event_id": group["id"],
            "jp_title": group["title"],
            "start_date": utc_catalog_date(group["start_date"]),
            "end_date": utc_catalog_date(group["end_date"]),
            "mission_count": group["mission_count"],
            "mission_fingerprint": group["mission_fingerprint"],
            "rewards": group["structured_rewards"],
        }
        for group in sorted(groups.values(), key=lambda group: (group["start_date"], group["id"]))
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def jp_campaign_asset_index(source: Path | None) -> dict[int, Path]:
    if source is None:
        return {}
    if not source.is_dir():
        raise FileNotFoundError(f"JP campaign image source does not exist: {source}")
    assets = {}
    for path in source.rglob("tex_campaign_mission_logo_*.png"):
        match = JP_CAMPAIGN_ASSET_PATTERN.search(path.name)
        if match:
            assets[int(match.group(1))] = path
    return assets


def write_jp_campaign_webp(source: Path, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_suffix(".webp.tmp")
    with Image.open(source) as image:
        image.load()
        converted = image.convert("RGBA")
        if converted.height > converted.width:
            converted = converted.rotate(90, expand=True)
        if converted.width > 512:
            height = round(converted.height * 512 / converted.width)
            converted = converted.resize((512, height), Image.Resampling.LANCZOS)
        converted.save(temporary, format="WEBP", lossless=True, method=6)
    os.replace(temporary, target)


def main() -> int:
    args = parse_args()
    existing = json.loads(args.catalog.read_text(encoding="utf-8"))
    image_root = args.frontend_root / "src" / "assets" / "images" / "campaign"
    jp_assets = jp_campaign_asset_index(args.jp_image_source)
    if args.jp_master is not None:
        groups = load_mission_groups(sqlite3.connect(args.jp_master))
        write_planner_reward_catalog(args.planner_rewards, groups)
        image_group_ids = set(groups) | {int(row["campaign_id"]) for row in existing}
    else:
        groups = None
        image_group_ids = {int(row["campaign_id"]) for row in existing}
        if not jp_assets:
            raise ValueError("--jp-master is required unless --jp-image-source is supplied")
    created_images = 0
    for group_id in sorted(image_group_ids):
        source = jp_assets.get(group_id)
        if source is None:
            continue
        target = image_root / f"{group_id}.webp"
        if target.is_file():
            continue
        write_jp_campaign_webp(source, target)
        created_images += 1

    if groups is None:
        print(
            f"JP mission campaigns: {len(existing)} catalogue rows unchanged, "
            f"{created_images} missing JP fallback images created"
        )
        return 0

    groups_by_day: dict[str, list[dict]] = collections.defaultdict(list)
    for group in groups.values():
        group["catalog_start_date"] = utc_catalog_date(group["start_date"])
        groups_by_day[group["catalog_start_date"][:10]].append(group)

    rows, mapped_group_ids = merge_existing_rows(existing, groups_by_day)

    for group in groups.values():
        if group["id"] in mapped_group_ids:
            continue
        if not (image_root / f"{group['id']}.webp").is_file():
            continue
        row = {
            "campaign_id": group["id"],
            "image": f"{group['id']}.png",
            "start_date": utc_catalog_date(group["start_date"]),
            "end_date": utc_catalog_date(group["end_date"]),
        }
        attach_group(row, group)
        rows.append(row)

    deduplicated = {}
    for row in rows:
        key = (row["campaign_id"], row["start_date"])
        current = deduplicated.get(key)
        if current is None or (
            (row.get("jp_mission_event_id") is not None, row.get("title") is not None)
            > (current.get("jp_mission_event_id") is not None, current.get("title") is not None)
        ):
            deduplicated[key] = row
    rows = sorted(
        deduplicated.values(), key=lambda row: (row["start_date"], row["campaign_id"])
    )
    args.catalog.write_text(
        json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(
        f"JP mission campaigns: {len(rows)} timeline rows, "
        f"{len(mapped_group_ids)} existing rows linked, "
        f"{len(rows) - len(existing)} rows added, "
        f"{created_images} missing JP fallback images created"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
