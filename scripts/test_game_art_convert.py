"""Run in the service image, beside the other converter tests:
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_convert
"""
import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))

import game_art_convert as convert  # noqa: E402


def write_glb(path: Path, document: dict) -> None:
    content = json.dumps(document).encode()
    content += b" " * (-len(content) % 4)
    header = struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(content))
    path.write_bytes(header + struct.pack("<II", len(content), convert.GLB_JSON_CHUNK) + content)


class FlashSpriteTests(unittest.TestCase):
    def test_crop_flash_sprite_flips_unity_v_and_unrotates(self):
        sheet = Image.new("RGBA", (8, 4), (0, 0, 0, 0))
        # Top-left 2x4 block in PIL corresponds to Unity UV x=[0, 0.25], y=[0, 1].
        sheet.putpixel((0, 0), (255, 0, 0, 255))
        sheet.putpixel((1, 0), (0, 255, 0, 255))
        sheet.putpixel((0, 3), (0, 0, 255, 255))

        upright = convert.crop_flash_sprite(
            sheet, {"x": 0.0, "y": 0.0}, {"x": 0.25, "y": 1.0}, rotated=0
        )
        self.assertEqual(upright.size, (2, 4))
        self.assertEqual(upright.getpixel((0, 0)), (255, 0, 0, 255))
        self.assertEqual(upright.getpixel((0, 3)), (0, 0, 255, 255))

        rotated = convert.crop_flash_sprite(
            sheet, {"x": 0.0, "y": 0.0}, {"x": 0.25, "y": 1.0}, rotated=1
        )
        self.assertEqual(rotated.size, (4, 2))
        # ROTATE_90 moves PIL (1, 0) to (0, 0) and (0, 0) to (0, 1).
        self.assertEqual(rotated.getpixel((0, 0)), (0, 255, 0, 255))
        self.assertEqual(rotated.getpixel((0, 1)), (255, 0, 0, 255))

    def test_pick_image_reads_flash_sprites_and_caches_sheet_decoding(self):
        sheet = Image.new("RGBA", (4, 4), (10, 20, 30, 255))
        sheet.putpixel((0, 0), (200, 100, 50, 255))
        reads = 0

        class CountingTexture:
            m_Name = "sheet_C"

            @property
            def image(self):
                nonlocal reads
                reads += 1
                return sheet

        tex = CountingTexture()
        flash = {
            "card_back": (tex, {"x": 0.0, "y": 0.5}, {"x": 0.5, "y": 0.5}, 0),
            "card_border": (tex, {"x": 0.5, "y": 0.0}, {"x": 0.5, "y": 0.5}, 0),
        }
        cache: dict[str, Image.Image] = {}

        first = convert.pick_image({}, {"sheet_C": tex}, "card_back", flash, cache)
        second = convert.pick_image({}, {"sheet_C": tex}, "card_border", flash, cache)

        self.assertEqual(first.size, (2, 2))
        self.assertEqual(first.getpixel((0, 0)), (200, 100, 50, 255))
        self.assertEqual(second.size, (2, 2))
        self.assertEqual(reads, 1)

    def test_pick_image_selects_explicit_texture_from_multi_texture_bundle(self):
        door = SimpleNamespace(image=Image.new("RGBA", (4, 4), (255, 0, 0, 255)))
        mask = SimpleNamespace(image=Image.new("RGBA", (4, 4), (0, 255, 0, 255)))

        picked = convert.pick_image({}, {"door": door, "mask": mask}, "door")

        self.assertEqual(picked.getpixel((0, 0)), (255, 0, 0, 255))
        with self.assertRaises(ValueError):
            convert.pick_image({}, {"door": door, "mask": mask}, "missing")


class ShaderReportTests(unittest.TestCase):
    def test_a_model_reports_each_shader_its_materials_name_once(self):
        def material(file: str | None) -> dict:
            shader = {"name": "Shader"} if file is None else {"name": "Shader", "file": file}
            return {"extras": {"unity": {"shader": shader}}}

        document = {
            "asset": {"version": "2.0"},
            "materials": [
                material("/art/shaders/gallop/3d/chara/toon/tser.json"),
                material("/art/shaders/gallop/3d/chara/toon/tser.json"),
                material("/art/shaders/gallop/3d/chara/tooneye/t.json"),
                material(None),
                {},
            ],
        }
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "model.glb"
            write_glb(path, document)

            shaders = convert.glb_shaders(str(path))

        self.assertEqual(
            shaders, {"/art/shaders/gallop/3d/chara/toon/tser.json", "/art/shaders/gallop/3d/chara/tooneye/t.json"}
        )


if __name__ == "__main__":
    unittest.main()
