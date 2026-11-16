"""Run in the service image, beside the other converter tests:
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_convert
"""
import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import game_art_convert as convert  # noqa: E402


def write_glb(path: Path, document: dict) -> None:
    content = json.dumps(document).encode()
    content += b" " * (-len(content) % 4)
    header = struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(content))
    path.write_bytes(header + struct.pack("<II", len(content), convert.GLB_JSON_CHUNK) + content)


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
