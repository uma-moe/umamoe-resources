"""The game's shaders as JSON, so a browser can compile them.

The Android shader bundle holds every shader's programs as GLSL ES, which Unity generated from the
game's HLSL; nearly all are GLSL ES 3.00, the language of WebGL 2. Each shader becomes one file
named after its path, `Gallop/3D/Chara/Toon/TSER` at `gallop/3d/chara/toon/tser.json`, beside an
`index.json` that lists every shader's name and file. The few shaders sharing a name add their
object ID to the file name, which is how materials, and `extras.unity.shader` in the models, tell
them apart.

A file keeps the shader's properties, keywords and passes. Each pass keeps its tags, render state
and one program per keyword variant: its GLSL version and its vertex and fragment shaders, as
indexes into the shared `sources`. WebGL 2 runs the `300 es` programs; the few `310 es` ones need
GLES 3.1. Render state numbers are Unity's enums (CompareFunction, CullMode, BlendMode, BlendOp,
StencilOp); a state set by a material property names that property.
"""

import json
import os
import posixpath
import re
from typing import Any

from UnityPy.export.ShaderConverter import ShaderSubProgram
from UnityPy.helpers import CompressionHelper
from UnityPy.streams import EndianBinaryReader

# Unity's SerializedPropertyType, PassType and TextureDimension.
PROPERTY_TYPES = ("color", "vector", "float", "range", "texture", "int")
PASS_TYPES = ("normal", "use", "grab")
TEXTURE_DIMENSIONS = {1: "any", 2: "2d", 3: "3d", 4: "cube", 5: "2d-array", 6: "cube-array"}
# A render state value not set by a material property names none.
UNBOUND_PROPERTY = "<noninit>"
INDEX_FILE = "index.json"
# Where the publisher puts the shader bundle: the folder of `SHADER_INDEX_KEY` in layout.rs.
SHADER_FOLDER = "art/shaders"
# Raised when the shape of a folder index changes.
INDEX_SCHEMA = 1
# Unity's ShaderGpuProgramType of the GLES programs, by the GLSL version each is written in.
GLSL_VERSIONS = {2: "310 es", 3: "310 es", 4: "300 es"}
UNIFORM_LOCATION_ON = "#define UNITY_SUPPORTS_UNIFORM_LOCATION 1"
UNIFORM_LOCATION_OFF = "#define UNITY_SUPPORTS_UNIFORM_LOCATION 0"


def name_path(name: str) -> str:
    """Each segment of a shader name, lower case, with anything but letters and digits as a dash."""
    segments = [re.sub(r"[^a-z0-9]+", "-", segment.lower()).strip("-") for segment in name.split("/")]
    if not all(segments):
        raise ValueError(f"shader name {name!r} has an empty segment")
    return "/".join(segments)


def shader_files(names: dict[int, str]) -> dict[int, str]:
    """Each shader's file below the shader folder, by object ID. A few shaders share a name and
    materials tell them apart by ID, which every platform's bundle shares, so their files carry it."""
    paths = {shader_id: name_path(name) for shader_id, name in names.items()}
    counts: dict[str, int] = {}
    for path in paths.values():
        counts[path] = counts.get(path, 0) + 1

    files = {}
    for shader_id, path in paths.items():
        if counts[path] > 1:
            path = f"{path}-{shader_id & 0xFFFFFFFFFFFFFFFF:016x}"
        files[shader_id] = f"{path}.json"
    return files


def shader_name(reader: Any) -> str:
    return reader.read_typetree()["m_ParsedForm"]["m_Name"]


def serialized_shader_files(serialized: Any) -> dict[int, dict[str, str]]:
    """The name and file of every shader in one loaded serialized file, by object ID."""
    names = {
        path_id: shader_name(reader)
        for path_id, reader in serialized.objects.items()
        if reader.type.name == "Shader"
    }
    files = shader_files(names)
    return {shader_id: {"name": names[shader_id], "file": files[shader_id]} for shader_id in names}


def tag_map(tags: dict[str, Any]) -> dict[str, str]:
    return {key: value for key, value in tags["tags"]}


def state_value(value: dict[str, Any]) -> dict[str, Any]:
    entry: dict[str, Any] = {"value": round(value["val"], 6)}
    if value["name"] != UNBOUND_PROPERTY:
        entry["property"] = value["name"]
    return entry


def blend(target: dict[str, Any]) -> dict[str, Any]:
    return {
        "src": state_value(target["srcBlend"]),
        "dst": state_value(target["destBlend"]),
        "srcAlpha": state_value(target["srcBlendAlpha"]),
        "dstAlpha": state_value(target["destBlendAlpha"]),
        "op": state_value(target["blendOp"]),
        "opAlpha": state_value(target["blendOpAlpha"]),
        "colorMask": state_value(target["colMask"]),
    }


def stencil_operation(operation: dict[str, Any]) -> dict[str, Any]:
    return {
        "pass": state_value(operation["pass"]),
        "fail": state_value(operation["fail"]),
        "zFail": state_value(operation["zFail"]),
        "comp": state_value(operation["comp"]),
    }


def render_state(state: dict[str, Any]) -> dict[str, Any]:
    # Without separate blending every render target uses the first one's.
    targets = 8 if state["rtSeparateBlend"] else 1
    return {
        "blend": [blend(state[f"rtBlend{target}"]) for target in range(targets)],
        "zTest": state_value(state["zTest"]),
        "zWrite": state_value(state["zWrite"]),
        "zClip": state_value(state["zClip"]),
        "cull": state_value(state["culling"]),
        "offsetFactor": state_value(state["offsetFactor"]),
        "offsetUnits": state_value(state["offsetUnits"]),
        "alphaToMask": state_value(state["alphaToMask"]),
        "stencil": {
            "ref": state_value(state["stencilRef"]),
            "readMask": state_value(state["stencilReadMask"]),
            "writeMask": state_value(state["stencilWriteMask"]),
            "op": stencil_operation(state["stencilOp"]),
            "front": stencil_operation(state["stencilOpFront"]),
            "back": stencil_operation(state["stencilOpBack"]),
        },
    }


def shader_property(prop: dict[str, Any]) -> dict[str, Any]:
    kind = PROPERTY_TYPES[prop["m_Type"]]
    defaults = [round(prop[f"m_DefValue[{index}]"], 6) for index in range(4)]
    entry: dict[str, Any] = {"name": prop["m_Name"], "description": prop["m_Description"], "type": kind}
    if kind in ("color", "vector"):
        entry["default"] = defaults
    elif kind == "texture":
        texture = prop["m_DefTexture"]
        entry["default"] = texture["m_DefaultName"]
        entry["dimension"] = TEXTURE_DIMENSIONS.get(texture["m_TexDim"], texture["m_TexDim"])
    else:
        entry["default"] = defaults[0]
    if kind == "range":
        entry["range"] = defaults[1:3]
    if prop["m_Attributes"]:
        entry["attributes"] = list(prop["m_Attributes"])
    return entry


class ProgramBlob:
    """The compiled programs of one shader on its one platform, read on demand by blob index."""

    def __init__(self, tree: dict[str, Any]) -> None:
        if len(tree["platforms"]) != 1:
            raise ValueError(f"expected programs for one platform, found {tree['platforms']}")
        compressed = bytes(tree["compressedBlob"])
        self.segments = [
            CompressionHelper.decompress_lz4(compressed[offset : offset + length], size)
            for offset, length, size in zip(
                tree["offsets"][0], tree["compressedLengths"][0], tree["decompressedLengths"][0]
            )
        ]
        header = EndianBinaryReader(self.segments[0], endian="<")
        count = header.read_int()
        self.entries = [(header.read_int(), header.read_int(), header.read_int()) for _ in range(count)]

    def source(self, index: int) -> str:
        offset, _, segment = self.entries[index]
        reader = EndianBinaryReader(self.segments[segment], endian="<")
        reader.Position = offset
        return bytes(ShaderSubProgram(reader).m_ProgramCode).decode("utf-8")


def stage_body(stage: str) -> str:
    body = stage.rstrip()
    if not body.endswith("#endif"):
        raise ValueError("a shader stage does not end its #ifdef")
    body = body[: -len("#endif")].rstrip() + "\n"
    # WebGL 2 is GLES 3.0, which cannot place uniforms by location; the source does without when told.
    return body.replace(UNIFORM_LOCATION_ON, UNIFORM_LOCATION_OFF)


def split_stages(code: str) -> tuple[str, str]:
    """The vertex and fragment shaders of one program, which holds both as
    `#ifdef VERTEX … #endif` and `#ifdef FRAGMENT … #endif`."""
    vertex_marker, fragment_marker = "#ifdef VERTEX\n", "\n#ifdef FRAGMENT\n"
    if not code.startswith(vertex_marker) or fragment_marker not in code:
        raise ValueError("a program is not one vertex and one fragment shader")
    vertex, fragment = code[len(vertex_marker) :].split(fragment_marker, 1)
    return stage_body(vertex), stage_body(fragment)


class ShaderDocument:
    """One shader's JSON; programs share the stage sources they have in common."""

    def __init__(self, tree: dict[str, Any]) -> None:
        self.parsed = tree["m_ParsedForm"]
        self.blob = ProgramBlob(tree)
        self.keywords = list(self.parsed["m_KeywordNames"])
        self.sources: list[str] = []
        self.source_indices: dict[str, int] = {}
        self.stages: dict[int, dict[str, int]] = {}

    def source_index(self, source: str) -> int:
        if source not in self.source_indices:
            self.source_indices[source] = len(self.sources)
            self.sources.append(source)
        return self.source_indices[source]

    def program_stages(self, blob_index: int) -> dict[str, int]:
        if blob_index not in self.stages:
            vertex, fragment = split_stages(self.blob.source(blob_index))
            self.stages[blob_index] = {
                "vertex": self.source_index(vertex),
                "fragment": self.source_index(fragment),
            }
        return self.stages[blob_index]

    def programs(self, program: dict[str, Any]) -> list[dict[str, Any]]:
        # Hardware tiers repeat the same variants; GLES needs only the first.
        tier = next((tier for tier in program["m_PlayerSubPrograms"] if tier), [])
        return [
            {
                "keywords": [self.keywords[index] for index in sub_program["m_KeywordIndices"]],
                "glsl": GLSL_VERSIONS[sub_program["m_GpuProgramType"]],
                **self.program_stages(sub_program["m_BlobIndex"]),
            }
            for sub_program in tier
        ]

    def shader_pass(self, shader_pass: dict[str, Any]) -> dict[str, Any]:
        kind = PASS_TYPES[shader_pass["m_Type"]]
        if kind == "use":
            return {"type": kind, "use": shader_pass["m_UseName"]}
        if kind == "grab":
            return {"type": kind, "texture": shader_pass["m_TextureName"]}

        state = shader_pass["m_State"]
        return {
            "type": kind,
            "name": state["m_Name"],
            "tags": tag_map(state["m_Tags"]),
            "state": render_state(state),
            "programs": self.programs(shader_pass["progVertex"]),
        }

    def document(self) -> dict[str, Any]:
        sub_shaders = [
            {
                "lod": sub_shader["m_LOD"],
                "tags": tag_map(sub_shader["m_Tags"]),
                "passes": [self.shader_pass(shader_pass) for shader_pass in sub_shader["m_Passes"]],
            }
            for sub_shader in self.parsed["m_SubShaders"]
        ]
        return {
            "name": self.parsed["m_Name"],
            "fallback": self.parsed["m_FallbackName"] or None,
            "properties": [shader_property(prop) for prop in self.parsed["m_PropInfo"]["m_Props"]],
            "keywords": self.keywords,
            "subShaders": sub_shaders,
            "sources": self.sources,
        }


def write_json(path: str, document: dict[str, Any]) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as file:
        json.dump(document, file, separators=(",", ":"))


def folder_indexes(listed: list[dict[str, str]]) -> dict[str, dict[str, Any]]:
    """One browsing document per folder of shader files, by folder ("" for the top): the folders
    below it and the shaders in it, each named relative to that folder."""
    indexes: dict[str, dict[str, Any]] = {"": {"schema": INDEX_SCHEMA}}

    def add_folder(folder: str) -> None:
        if folder in indexes:
            return
        indexes[folder] = {"schema": INDEX_SCHEMA}
        above, name = posixpath.split(folder)
        add_folder(above)
        indexes[above].setdefault("folders", []).append(name)

    for entry in sorted(listed, key=lambda item: item["file"]):
        folder, file = posixpath.split(entry["file"])
        add_folder(folder)
        indexes[folder].setdefault("shaders", []).append({"name": entry["name"], "file": file})
    for index in indexes.values():
        if "folders" in index:
            index["folders"].sort()
    return indexes


def export_shaders(readers: list[Any], output: dict[str, Any]) -> list[dict[str, str]]:
    """Writes every shader beside the output, which becomes the top folder's index, with an
    `index.json` in each folder below, and returns each further file as {"key", "path"}."""
    shader_folder = posixpath.dirname(output["key"])
    local_folder = os.path.splitext(output["path"])[0]
    if not readers:
        raise ValueError("the bundle holds no shaders")

    files = shader_files({reader.path_id: shader_name(reader) for reader in readers})
    listed: list[dict[str, str]] = []
    written: list[dict[str, str]] = []
    for reader in readers:
        document = ShaderDocument(reader.read_typetree()).document()
        file = files[reader.path_id]
        if posixpath.basename(file) == INDEX_FILE:
            raise ValueError(f"shader file {file} would replace its folder's index")
        local_path = os.path.join(local_folder, *file.split("/"))
        write_json(local_path, document)
        listed.append({"name": document["name"], "file": file})
        written.append({"key": f"{shader_folder}/{file}", "path": local_path})

    for folder, index in folder_indexes(listed).items():
        if not folder:
            write_json(output["path"], index)
            continue
        local_path = os.path.join(local_folder, *folder.split("/"), INDEX_FILE)
        write_json(local_path, index)
        written.append({"key": f"{shader_folder}/{folder}/{INDEX_FILE}", "path": local_path})
    return written
