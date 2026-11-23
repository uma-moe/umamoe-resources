"""Run in the service image, beside the other converter tests:
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_timeline_art_convert
"""
import sys
import tempfile
import unittest
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))

import timeline_art_convert as convert  # noqa: E402


class SizeTests(unittest.TestCase):
    def test_japanese_art_is_scaled_down_to_512_wide_only(self):
        self.assertEqual(convert.japanese_size(1024, 401), (512, 201))
        self.assertEqual(convert.japanese_size(400, 150), (400, 150))

    def test_wider_english_art_is_stretched_at_most_ten_percent(self):
        wide = convert.english_size(1200, 300, (512, 200))
        slightly_wide = convert.english_size(560, 200, (512, 200))

        self.assertEqual(wide, (512, 141))
        self.assertEqual(slightly_wide, (512, 200))

    def test_english_art_takes_the_size_of_its_japanese_reference(self):
        with tempfile.TemporaryDirectory() as folder:
            reference = Path(folder) / "jp.png"
            Image.new("RGB", (1024, 400)).save(reference)

            target = convert.english_target({"reference": str(reference), "reference_is_japanese": True})

        self.assertEqual(target, (512, 200))


class ConvertTests(unittest.TestCase):
    def test_pictures_with_alpha_stay_lossless_with_their_alpha(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "banner.png"
            output = Path(folder) / "banner.webp"
            Image.new("RGBA", (1024, 400), (255, 0, 0, 128)).save(source)

            results = convert.convert_all([{"source": str(source), "output": str(output), "fit": "japanese"}])

            with Image.open(output) as written:
                self.assertEqual(written.size, (512, 200))
                self.assertEqual(written.mode, "RGBA")
        self.assertEqual(results[0]["size"], [512, 200])
        self.assertIsNone(results[0]["error"])

    def test_a_broken_picture_reports_an_error_and_the_batch_goes_on(self):
        with tempfile.TemporaryDirectory() as folder:
            broken = Path(folder) / "broken.png"
            broken.write_bytes(b"not a picture")
            good = Path(folder) / "good.png"
            Image.new("RGB", (100, 40)).save(good)

            results = convert.convert_all([
                {"source": str(broken), "output": str(Path(folder) / "a.webp"), "fit": "japanese"},
                {"source": str(good), "output": str(Path(folder) / "b.webp"), "fit": "japanese"},
            ])

        self.assertIsNotNone(results[0]["error"])
        self.assertEqual(results[1]["size"], [100, 40])


if __name__ == "__main__":
    unittest.main()
