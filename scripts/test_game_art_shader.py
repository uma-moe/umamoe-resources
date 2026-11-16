"""Run in the service image, beside the other converter tests:
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_uma test_game_art_face test_game_art_shader
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import game_art_shader as shader  # noqa: E402


class ShaderFileTests(unittest.TestCase):
    def test_a_shader_is_filed_under_its_name(self):
        files = shader.shader_files({7: "Gallop/3D/Chara/Toon/TSER", 8: "Hidden/Internal-Colored"})

        self.assertEqual(files, {7: "gallop/3d/chara/toon/tser.json", 8: "hidden/internal-colored.json"})

    def test_shaders_sharing_a_name_add_their_id(self):
        files = shader.shader_files({5: "Gallop/3D/Chara/MultiplyCheek", -1: "Gallop/3D/Chara/MultiplyCheek"})

        self.assertEqual(
            files,
            {
                5: "gallop/3d/chara/multiplycheek-0000000000000005.json",
                -1: "gallop/3d/chara/multiplycheek-ffffffffffffffff.json",
            },
        )


def program(vertex: str, fragment: str) -> str:
    return f"#ifdef VERTEX\n{vertex}\n#endif\n#ifdef FRAGMENT\n{fragment}\n#endif\n"


class StageTests(unittest.TestCase):
    def test_a_program_splits_into_its_vertex_and_fragment_shader(self):
        code = program("#version 300 es\n#if A\nvoid v() {}\n#endif", "#version 300 es\nvoid f() {}")

        vertex, fragment = shader.split_stages(code)

        self.assertEqual(vertex, "#version 300 es\n#if A\nvoid v() {}\n#endif\n")
        self.assertEqual(fragment, "#version 300 es\nvoid f() {}\n")

    def test_uniforms_are_not_placed_by_location_for_webgl(self):
        code = program(f"#version 300 es\n{shader.UNIFORM_LOCATION_ON}", "#version 300 es")

        vertex, _ = shader.split_stages(code)

        self.assertIn(shader.UNIFORM_LOCATION_OFF, vertex)
        self.assertNotIn(shader.UNIFORM_LOCATION_ON, vertex)

    def test_a_program_without_both_stages_is_refused(self):
        with self.assertRaises(ValueError):
            shader.split_stages("#ifdef VERTEX\nvoid v() {}\n#endif\n")


def float_value(value: float, name: str = shader.UNBOUND_PROPERTY) -> dict:
    return {"val": value, "name": name}


class RenderStateTests(unittest.TestCase):
    def test_a_state_a_material_property_sets_names_it(self):
        bound = shader.state_value(float_value(2.0, "_Cull"))
        fixed = shader.state_value(float_value(4.0))

        self.assertEqual(bound, {"value": 2.0, "property": "_Cull"})
        self.assertEqual(fixed, {"value": 4.0})


def serialized_property(kind: int, defaults: tuple[float, float, float, float], texture: str = "") -> dict:
    prop = {
        "m_Name": "_Value",
        "m_Description": "Value",
        "m_Attributes": [],
        "m_Type": kind,
        "m_DefTexture": {"m_DefaultName": texture, "m_TexDim": 2},
    }
    prop.update({f"m_DefValue[{index}]": value for index, value in enumerate(defaults)})
    return prop


class PropertyTests(unittest.TestCase):
    def test_a_range_keeps_its_default_and_limits(self):
        prop = shader.shader_property(serialized_property(3, (0.5, 0.0, 2.0, 0.0)))

        self.assertEqual((prop["type"], prop["default"], prop["range"]), ("range", 0.5, [0.0, 2.0]))

    def test_a_texture_keeps_its_default_colour_and_dimension(self):
        prop = shader.shader_property(serialized_property(4, (0.0, 0.0, 0.0, 0.0), texture="white"))

        self.assertEqual((prop["type"], prop["default"], prop["dimension"]), ("texture", "white", "2d"))


if __name__ == "__main__":
    unittest.main()
