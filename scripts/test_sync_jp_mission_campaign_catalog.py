import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("sync_jp_mission_campaign_catalog.py")
SPEC = importlib.util.spec_from_file_location("sync_jp_mission_campaign_catalog", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
sync = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sync)


class MergeExistingCatalogTests(unittest.TestCase):
    def test_unmatched_historical_row_keeps_structured_rewards(self) -> None:
        existing = [{
            "campaign_id": 100,
            "start_date": "2022-08-16T03:00:00",
            "end_date": "2022-09-29T19:59:59",
            "mission_fingerprint": "historical",
            "rewards": [{"item_category": 90, "item_id": 43, "amount": 1200}],
        }]

        rows, mapped = sync.merge_existing_rows(existing, {})

        self.assertEqual(mapped, set())
        self.assertEqual(rows, existing)
        self.assertEqual(rows[0]["rewards"][0]["amount"], 1200)

    def test_stale_source_cannot_reduce_existing_mission_count(self) -> None:
        existing = [{
            "campaign_id": 28,
            "start_date": "2021-05-02T20:00:00",
            "end_date": "2021-06-27T19:59:59",
            "mission_count": 188,
            "mission_fingerprint": "newer-catalogue",
        }]
        stale_group = {
            "id": 28,
            "title": "Spring G1 missions",
            "start_date": "2021/05/03 05:00:00",
            "end_date": "2021/06/28 04:59:59",
            "catalog_start_date": "2021-05-02T20:00:00",
            "mission_count": 132,
            "mission_fingerprint": "older-snapshot",
        }

        rows, mapped = sync.merge_existing_rows(
            existing,
            {"2021-05-02": [stale_group]},
        )

        self.assertEqual(mapped, {28})
        self.assertEqual(rows[0]["mission_count"], 188)
        self.assertEqual(rows[0]["mission_fingerprint"], "newer-catalogue")
        self.assertEqual(rows[0]["jp_mission_event_id"], 28)
        self.assertEqual(rows[0]["jp_title"], "Spring G1 missions")

    def test_equal_count_signature_drift_cannot_replace_existing_fingerprint(self) -> None:
        existing = [{
            "campaign_id": 4,
            "start_date": "2021-02-23T20:00:00",
            "end_date": "2021-03-31T19:59:59",
            "mission_count": 36,
            "mission_fingerprint": "newer-signature",
        }]
        historical_group = {
            "id": 4,
            "title": "Release missions",
            "start_date": "2021/02/24 05:00:00",
            "end_date": "2021/04/01 04:59:59",
            "catalog_start_date": "2021-02-23T20:00:00",
            "mission_count": 36,
            "mission_fingerprint": "older-equal-count-signature",
        }

        rows, mapped = sync.merge_existing_rows(
            existing,
            {"2021-02-23": [historical_group]},
        )

        self.assertEqual(mapped, {4})
        self.assertEqual(rows[0]["mission_count"], 36)
        self.assertEqual(rows[0]["mission_fingerprint"], "newer-signature")
        self.assertEqual(rows[0]["jp_mission_event_id"], 4)
        self.assertEqual(rows[0]["jp_title"], "Release missions")


if __name__ == "__main__":
    unittest.main()
