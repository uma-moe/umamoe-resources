"""Exports a Unity prefab, with the meshes and materials it draws, as a binary glTF (GLB).

Unity is left-handed and glTF right-handed, so X is mirrored: positions and normals negate X,
rotations negate Y and Z, and triangles reverse their winding. Unity's UV origin is the
bottom-left corner and glTF's the top-left, so V is flipped.

Textures are written beside the model as WebP and linked by relative URI (EXT_texture_webp).
The game's shaders have no glTF equivalent, so a material's `_MainTex` becomes its base colour
texture, and every Unity texture slot (baked light in `_LightTex`, for one), number and colour
is kept in `extras.unity`. Course materials (turf, dirt, base) are left empty in the game files
and filled at runtime with one season and weather variant of their part's textures; those are
published on their own, the first variant is linked as the base colour, and every variant is
listed in `extras.unity.variants`. Vertex colours, which those shaders read in their own way, are kept
as `_UNITY_COLOR` rather than `COLOR_0`, so standard viewers do not tint the model by them.
"""

import json
import math
import posixpath
import re
import struct
from array import array
from typing import Any, Callable
from urllib.parse import quote

from UnityPy.helpers.MeshHelper import MeshHandler

FLOAT = 5126
UNSIGNED_BYTE = 5121
UNSIGNED_SHORT = 5123
UNSIGNED_INT = 5125
COMPONENT_TYPES = {"f": FLOAT, "B": UNSIGNED_BYTE, "H": UNSIGNED_SHORT, "I": UNSIGNED_INT}
ARRAY_BUFFER = 34962
ELEMENT_ARRAY_BUFFER = 34963

LINEAR = 9729
LINEAR_MIPMAP_LINEAR = 9987
REPEAT = 10497
# Unity wrap modes: repeat, clamp, mirror, mirror once.
WRAP_MODES = {0: REPEAT, 1: 33071, 2: 33648, 3: 33648}
IDENTITY_UV_TRANSFORM = {"scale": [1.0, 1.0], "offset": [0.0, 0.0]}

# `tex_env_race10001_0000_112_turf00` and its `_sub` texture: variant 112, kind turf00.
RUNTIME_TEXTURE_NAME = re.compile(r"_(\d{3})_([a-z]+\d+)(_sub)?$")

JSON_CHUNK = 0x4E4F534A
BIN_CHUNK = 0x004E4942

WEBP_QUALITY = 90
# Track textures are large; method 4 encodes several times faster than 6 for a few more bytes.
WEBP_METHOD = 4
DEFAULT_ALPHA_CUTOFF = 0.5

assert array("I").itemsize == 4 and array("f").itemsize == 4


class Gltf:
    """A glTF document and its binary buffer, built up one accessor at a time."""

    def __init__(self) -> None:
        self.document: dict[str, Any] = {
            "asset": {"version": "2.0", "generator": "uma.moe game art"},
            "scene": 0,
            "scenes": [{"nodes": []}],
            "nodes": [],
            "meshes": [],
            "materials": [],
            "textures": [],
            "images": [],
            "samplers": [],
            "accessors": [],
            "bufferViews": [],
        }
        self.binary = bytearray()
        self.extensions: set[str] = set()

    def add(self, kind: str, item: dict[str, Any]) -> int:
        items = self.document[kind]
        items.append(item)
        return len(items) - 1

    def accessor(self, values: array, kind: str, count: int, target: int, **fields: Any) -> int:
        self.binary += b"\0" * (-len(self.binary) % 4)
        data = values.tobytes()
        view = self.add(
            "bufferViews",
            {"buffer": 0, "byteOffset": len(self.binary), "byteLength": len(data), "target": target},
        )
        self.binary += data
        accessor = {
            "bufferView": view,
            "componentType": COMPONENT_TYPES[values.typecode],
            "count": count,
            "type": kind,
            **fields,
        }
        return self.add("accessors", accessor)

    def write(self, path: str) -> None:
        # glTF does not allow empty arrays, so unused kinds are left out.
        document = {key: value for key, value in self.document.items() if value != []}
        self.binary += b"\0" * (-len(self.binary) % 4)
        if self.binary:
            document["buffers"] = [{"byteLength": len(self.binary)}]
        if self.extensions:
            document["extensionsUsed"] = sorted(self.extensions)
        if "EXT_texture_webp" in self.extensions:
            document["extensionsRequired"] = ["EXT_texture_webp"]

        content = json.dumps(document, separators=(",", ":")).encode()
        content += b" " * (-len(content) % 4)
        chunks = struct.pack("<II", len(content), JSON_CHUNK) + content
        if self.binary:
            chunks += struct.pack("<II", len(self.binary), BIN_CHUNK) + bytes(self.binary)

        with open(path, "wb") as file:
            file.write(struct.pack("<4sII", b"glTF", 2, 12 + len(chunks)))
            file.write(chunks)


def object_key(reader: Any) -> tuple[str, int]:
    return reader.assets_file.name, reader.path_id


def rounded(values: list[float]) -> list[float]:
    return [round(value, 6) + 0.0 for value in values]


def uv_transform(scale: Any, offset: Any) -> dict[str, list[float]]:
    """A Unity texture scale and offset in glTF's UV space, where V runs downwards."""
    return {
        "scale": rounded([scale.x, scale.y]),
        "offset": rounded([offset.x, 1 - scale.y - offset.y]),
    }


def node_transform(transform: Any) -> dict[str, list[float]]:
    position = transform.m_LocalPosition
    rotation = transform.m_LocalRotation
    scale = transform.m_LocalScale
    fields: dict[str, list[float]] = {}

    translation = [-position.x, position.y, position.z]
    if any(translation):
        fields["translation"] = rounded(translation)

    quaternion = [rotation.x, -rotation.y, -rotation.z, rotation.w]
    length = math.sqrt(sum(part * part for part in quaternion))
    if length > 0 and quaternion != [0, 0, 0, 1]:
        fields["rotation"] = rounded([part / length for part in quaternion])

    scaling = [scale.x, scale.y, scale.z]
    if scaling != [1, 1, 1]:
        fields["scale"] = rounded(scaling)
    return fields


class PrefabExporter:
    def __init__(
        self,
        glb_key: str,
        texture_folder: Callable[[Any], str],
        texture_file: Callable[[int], str],
        runtime_textures: list[dict[str, str]],
    ) -> None:
        self.gltf = Gltf()
        self.glb_folder = posixpath.dirname(glb_key)
        self.texture_folder = texture_folder
        self.texture_file = texture_file
        self.runtime_textures = runtime_textures
        self.attributes: dict[tuple[str, int], dict[str, int] | None] = {}
        self.meshes: dict[tuple[Any, ...], int | None] = {}
        self.materials: dict[tuple[str, int], int] = {}
        self.texture_uris: dict[tuple[str, int], str] = {}
        self.images: dict[str, int] = {}
        self.gltf_textures: dict[tuple[str, tuple[int, int]], int] = {}
        self.samplers: dict[tuple[int, int], int] = {}
        # Textures written beside the model, as {"key", "path"}.
        self.written: list[dict[str, str]] = []

    def node(self, transform: Any) -> int:
        game_object = transform.m_GameObject.read()
        node: dict[str, Any] = {"name": game_object.m_Name, **node_transform(transform)}
        if not game_object.m_IsActive:
            node["extras"] = {"active": False}
        mesh = self.renderer_mesh(game_object)
        if mesh is not None:
            node["mesh"] = mesh

        index = self.gltf.add("nodes", node)
        children = [self.node(child.read()) for child in transform.m_Children]
        if children:
            node["children"] = children
        return index

    def renderer_mesh(self, game_object: Any) -> int | None:
        mesh_filter = None
        renderer = None
        for pair in game_object.m_Component:
            reader = pair.component.deref()
            if reader.type.name == "MeshFilter":
                mesh_filter = reader.read()
            elif reader.type.name == "MeshRenderer":
                renderer = reader.read()

        if mesh_filter is None or renderer is None or not renderer.m_Enabled:
            return None
        if mesh_filter.m_Mesh.m_PathID == 0:
            return None

        mesh_reader = mesh_filter.m_Mesh.deref()
        materials = tuple(self.material(pptr) for pptr in renderer.m_Materials)
        key = (*object_key(mesh_reader), materials)
        if key not in self.meshes:
            self.meshes[key] = self.mesh(mesh_reader, materials)
        return self.meshes[key]

    def mesh(self, mesh_reader: Any, materials: tuple[int | None, ...]) -> int | None:
        mesh = mesh_reader.read()
        handler = MeshHandler(mesh)
        handler.process()

        key = object_key(mesh_reader)
        if key not in self.attributes:
            self.attributes[key] = self.vertex_attributes(handler)
        attributes = self.attributes[key]
        if attributes is None:
            return None

        primitives = []
        for submesh, triangles in enumerate(handler.get_triangles()):
            if not triangles:
                continue
            primitive: dict[str, Any] = {
                "attributes": attributes,
                "indices": self.indices(triangles, handler.m_VertexCount),
            }
            # Unity draws extra submeshes with the last material.
            material = materials[min(submesh, len(materials) - 1)] if materials else None
            if material is not None:
                primitive["material"] = material
            primitives.append(primitive)

        if not primitives:
            return None
        return self.gltf.add("meshes", {"name": mesh.m_Name, "primitives": primitives})

    def vertex_attributes(self, handler: Any) -> dict[str, int] | None:
        count = handler.m_VertexCount
        vertices = handler.m_Vertices or []
        if count == 0 or len(vertices) < count:
            return None

        positions = array("f")
        for vertex in vertices[:count]:
            positions.extend((-vertex[0], vertex[1], vertex[2]))
        bounds = [positions[axis::3] for axis in range(3)]
        attributes = {
            "POSITION": self.gltf.accessor(
                positions,
                "VEC3",
                count,
                ARRAY_BUFFER,
                min=[min(axis) for axis in bounds],
                max=[max(axis) for axis in bounds],
            )
        }

        normals = handler.m_Normals or []
        if len(normals) >= count:
            values = array("f")
            for normal in normals[:count]:
                length = math.sqrt(normal[0] ** 2 + normal[1] ** 2 + normal[2] ** 2) or 1.0
                values.extend((-normal[0] / length, normal[1] / length, normal[2] / length))
            attributes["NORMAL"] = self.gltf.accessor(values, "VEC3", count, ARRAY_BUFFER)

        for semantic, uvs in (("TEXCOORD_0", handler.m_UV0), ("TEXCOORD_1", handler.m_UV1)):
            if not uvs or len(uvs) < count:
                continue
            values = array("f")
            for uv in uvs[:count]:
                values.extend((uv[0], 1 - uv[1]))
            attributes[semantic] = self.gltf.accessor(values, "VEC2", count, ARRAY_BUFFER)

        colors = handler.m_Colors or []
        if len(colors) >= count:
            stored_as_bytes = isinstance(colors[0][0], int)
            values = array("B" if stored_as_bytes else "f")
            for color in colors[:count]:
                values.extend(color[:4])
            attributes["_UNITY_COLOR"] = self.gltf.accessor(
                values, "VEC4", count, ARRAY_BUFFER, normalized=stored_as_bytes
            )
        return attributes

    def indices(self, triangles: list[tuple[int, int, int]], vertex_count: int) -> int:
        # 65535 is the primitive restart value, so 16-bit indices stop just below it.
        values = array("H" if vertex_count <= 65535 else "I")
        for first, second, third in triangles:
            values.extend((first, third, second))
        return self.gltf.accessor(values, "SCALAR", len(values), ELEMENT_ARRAY_BUFFER)

    def material(self, pptr: Any) -> int | None:
        if pptr.m_PathID == 0:
            return None
        reader = pptr.deref()
        key = object_key(reader)
        if key in self.materials:
            return self.materials[key]

        material = reader.read()
        properties = material.m_SavedProperties
        floats = {name: round(value, 6) for name, value in properties.m_Floats}
        colors = {
            name: rounded([color.r, color.g, color.b, color.a])
            for name, color in properties.m_Colors
        }

        slots: dict[str, dict[str, Any]] = {}
        empty_slots = set()
        base_color_texture = None
        for slot, texture_slot in properties.m_TexEnvs:
            if texture_slot.m_Texture.m_PathID == 0:
                empty_slots.add(slot)
                continue
            texture_reader = texture_slot.m_Texture.deref()
            if texture_reader.type.name != "Texture2D":
                continue
            texture = texture_reader.read()
            transform = uv_transform(texture_slot.m_Scale, texture_slot.m_Offset)
            uri = self.texture_uri(texture_reader, texture)
            slots[slot] = {"uri": uri, **transform}
            if slot == "_MainTex":
                index = self.gltf_texture(uri, texture_wrap(texture))
                base_color_texture = self.texture_info(index, transform)

        variants = self.runtime_variants(material.m_Name) if "_MainTex" in empty_slots else {}
        default_variant = next(iter(variants), None)
        if default_variant is not None and "_MainTex" in variants[default_variant]:
            index = self.gltf_texture(variants[default_variant]["_MainTex"], (REPEAT, REPEAT))
            base_color_texture = self.texture_info(index, IDENTITY_UV_TRANSFORM)

        pbr: dict[str, Any] = {"metallicFactor": 0.0, "roughnessFactor": 1.0}
        if "_Color" in colors:
            pbr["baseColorFactor"] = colors["_Color"]
        if base_color_texture is not None:
            pbr["baseColorTexture"] = base_color_texture

        entry: dict[str, Any] = {"name": material.m_Name, "pbrMetallicRoughness": pbr}
        if floats.get("_Cull") == 0:
            entry["doubleSided"] = True
        # The game names cut-out materials (fences, foliage, crowds) "alpha".
        if "alpha" in material.m_Name.lower():
            entry["alphaMode"] = "MASK"
            cutoff = floats.get("_Cutoff", 0)
            entry["alphaCutoff"] = cutoff if cutoff > 0 else DEFAULT_ALPHA_CUTOFF
        unity: dict[str, Any] = {
            "textures": slots,
            "floats": floats,
            "colors": colors,
            "renderQueue": material.m_CustomRenderQueue,
        }
        if variants:
            unity["variants"] = variants
            unity["defaultVariant"] = default_variant
        entry["extras"] = {"unity": unity}

        index = self.gltf.add("materials", entry)
        self.materials[key] = index
        return index

    def runtime_variants(self, material_name: str) -> dict[str, dict[str, str]]:
        """The part's textures the game puts in this material's `_MainTex` and `_SubTex`, by
        variant: those whose kind (turf00, dirt00, base00) the material is named after."""
        variants: dict[str, dict[str, str]] = {}
        for texture in self.runtime_textures:
            match = RUNTIME_TEXTURE_NAME.search(posixpath.basename(texture["name"]))
            if match is None:
                continue
            variant, kind, sub = match.groups()
            if f"_{kind}" not in material_name:
                continue
            slot = "_SubTex" if sub else "_MainTex"
            variants.setdefault(variant, {})[slot] = self.link(texture["key"])
        return dict(sorted(variants.items()))

    def link(self, bucket_key: str) -> str:
        return quote(posixpath.relpath(bucket_key, self.glb_folder))

    def texture_uri(self, reader: Any, texture: Any) -> str:
        """Writes the texture beside the model once and returns its link from the model."""
        key = object_key(reader)
        if key not in self.texture_uris:
            bucket_key = f"{self.texture_folder(reader)}/{texture.m_Name}.webp"
            path = self.texture_file(len(self.written))
            texture.image.save(path, "WEBP", quality=WEBP_QUALITY, method=WEBP_METHOD)
            self.written.append({"key": bucket_key, "path": path})
            self.texture_uris[key] = self.link(bucket_key)
        return self.texture_uris[key]

    def gltf_texture(self, uri: str, wrap: tuple[int, int]) -> int:
        key = (uri, wrap)
        if key not in self.gltf_textures:
            if uri not in self.images:
                self.images[uri] = self.gltf.add("images", {"uri": uri})
            texture = {
                "sampler": self.sampler(wrap),
                "extensions": {"EXT_texture_webp": {"source": self.images[uri]}},
            }
            self.gltf_textures[key] = self.gltf.add("textures", texture)
            self.gltf.extensions.add("EXT_texture_webp")
        return self.gltf_textures[key]

    def texture_info(self, index: int, transform: dict[str, Any]) -> dict[str, Any]:
        info: dict[str, Any] = {"index": index}
        if transform != IDENTITY_UV_TRANSFORM:
            info["extensions"] = {"KHR_texture_transform": transform}
            self.gltf.extensions.add("KHR_texture_transform")
        return info

    def sampler(self, wrap: tuple[int, int]) -> int:
        if wrap not in self.samplers:
            sampler = {
                "magFilter": LINEAR,
                "minFilter": LINEAR_MIPMAP_LINEAR,
                "wrapS": wrap[0],
                "wrapT": wrap[1],
            }
            self.samplers[wrap] = self.gltf.add("samplers", sampler)
        return self.samplers[wrap]


def texture_wrap(texture: Any) -> tuple[int, int]:
    settings = texture.m_TextureSettings
    return WRAP_MODES.get(settings.m_WrapU, REPEAT), WRAP_MODES.get(settings.m_WrapV, REPEAT)


def prefab_roots(environment: Any, bundle_path: str) -> list[Any]:
    """The top transforms of the prefab bundle itself, not of the bundles it draws from."""
    bundle = environment.files[bundle_path]
    roots = []
    for serialized in bundle.files.values():
        for reader in getattr(serialized, "objects", {}).values():
            if reader.type.name != "Transform":
                continue
            transform = reader.read()
            if transform.m_Father.m_PathID == 0:
                roots.append(transform)
    return roots


def export_prefab(
    environment: Any,
    bundle_path: str,
    output: dict[str, Any],
    texture_folders: dict[str, str],
    runtime_textures: list[dict[str, str]],
) -> list[dict[str, str]]:
    """Writes `output["path"]` and the textures it links to; returns the textures it wrote."""
    exporter = PrefabExporter(
        output["key"],
        texture_folder=lambda reader: texture_folders[reader.assets_file.parent.name],
        texture_file=lambda number: f"{output['path']}.{number}.webp",
        runtime_textures=runtime_textures,
    )
    roots = prefab_roots(environment, bundle_path)
    if not roots:
        raise ValueError("the bundle holds no prefab")

    named = [root for root in roots if root.m_GameObject.read().m_Name == output["object"]]
    scene_nodes = exporter.gltf.document["scenes"][0]["nodes"]
    for root in named or roots:
        scene_nodes.append(exporter.node(root))

    exporter.gltf.write(output["path"])
    return exporter.written
