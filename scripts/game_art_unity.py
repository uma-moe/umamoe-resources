"""Turns Unity objects read by their type trees into plain JSON, for the parts of the game that
glTF has no place for: particle systems, scripts, timelines and every curve of a clip.

Each reference to another object (a PPtr) is handed to a function that decides what it becomes,
so a model can turn a material into its glTF index and a timeline can name the bundle it points
into. Floats are rounded to six places, raw bytes become base64 and non-finite numbers `null`.
Values stay in Unity's left-handed space unless a function says otherwise.
"""

import base64
import collections
import math
import struct
import zlib
from collections.abc import Callable
from typing import Any

from UnityPy.classes import PPtr

TRANSFORM_BINDING = 4
CURVES_PER_ATTRIBUTE = {1: 3, 2: 4, 3: 3, 4: 3}
TRANSFORM_ATTRIBUTES = {1: "translation", 2: "rotation", 3: "scale", 4: "euler"}
# Animated properties the game uses that a clip names only by hash.
KNOWN_ATTRIBUTES = {zlib.crc32(name.encode()): name for name in ("m_IsActive", "m_Enabled")}
LEGACY_CURVES = {
    "m_RotationCurves": "rotationCurves",
    "m_EulerCurves": "eulerCurves",
    "m_PositionCurves": "positionCurves",
    "m_ScaleCurves": "scaleCurves",
    "m_FloatCurves": "floatCurves",
    "m_PPtrCurves": "pptrCurves",
}
# Objects whose data is art published elsewhere or binary; a dump names them only.
OPAQUE_TYPES = {
    "AudioClip",
    "Font",
    "Material",
    "Mesh",
    "MonoScript",
    "Shader",
    "Sprite",
    "Texture2D",
}

# Unity's own assets, such as the default particle material, which no game bundle holds.
BUILTIN_FILES = {"unity default resources", "unity_builtin_extra"}

Reference = Callable[[int, int], Any]


def resolve(assets_file: Any, file_id: int, path_id: int) -> Any | None:
    """The reader a PPtr in `assets_file` points at, or `None` when it is empty or points into a
    bundle that was not loaded."""
    if path_id == 0:
        return None
    try:
        return PPtr(m_FileID=file_id, m_PathID=path_id).deref(assets_file)
    except (FileNotFoundError, KeyError, ValueError):
        return None


def external_path(assets_file: Any, file_id: int) -> str | None:
    if file_id <= 0 or file_id > len(assets_file.externals):
        return None
    return assets_file.externals[file_id - 1].path


def builtin(assets_file: Any, file_id: int, path_id: int) -> dict[str, str] | None:
    """A reference to one of Unity's built-in assets by file and path ID, or `None` for any
    other."""
    path = external_path(assets_file, file_id)
    if path is None:
        return None
    file_name = path.rsplit("/", 1)[-1]
    if file_name.lower() not in BUILTIN_FILES:
        return None
    return {"builtin": file_name, "pathId": str(path_id)}


def is_builtin(pointer: Any) -> bool:
    """Whether a parsed PPtr points at one of Unity's built-in assets."""
    return builtin(pointer.assetsfile, pointer.m_FileID, pointer.m_PathID) is not None


def unresolved(assets_file: Any, file_id: int, path_id: int) -> dict[str, str] | None:
    """What a reference that did not resolve becomes: a built-in asset, or the external file
    and path ID it points at."""
    if path_id == 0:
        return None
    builtin_asset = builtin(assets_file, file_id, path_id)
    if builtin_asset is not None:
        return builtin_asset
    return {"external": external_path(assets_file, file_id) or "", "pathId": str(path_id)}


def number(value: float) -> float | None:
    """A float for JSON: rounded, with `null` for infinities and NaN, which JSON lacks."""
    return round(value, 6) + 0.0 if math.isfinite(value) else None


def no_references(file_id: int, path_id: int) -> None:
    raise ValueError("these values hold no references")


def is_pointer(value: Any) -> bool:
    return isinstance(value, dict) and value.keys() == {"m_FileID", "m_PathID"}


def plain(value: Any, reference: Reference) -> Any:
    """`value`, read from a type tree, as JSON, each PPtr replaced by `reference(file_id,
    path_id)`."""
    if is_pointer(value):
        return reference(value["m_FileID"], value["m_PathID"])
    if isinstance(value, dict):
        return {key: plain(item, reference) for key, item in value.items()}
    if isinstance(value, list):
        return [plain(item, reference) for item in value]
    if isinstance(value, float):
        return number(value)
    if isinstance(value, (bytes, bytearray)):
        return base64.b64encode(value).decode()
    return value


def owner(reader: Any) -> Any | None:
    """The game object a component belongs to, or `None` for an object that is not one."""
    game_object = reader.read_typetree().get("m_GameObject")
    if not is_pointer(game_object):
        return None
    return resolve(reader.assets_file, game_object["m_FileID"], game_object["m_PathID"])


def object_name(reader: Any) -> str:
    """A named object's name; a component goes by its game object's."""
    name = reader.peek_name()
    if name:
        return name
    game_object = owner(reader)
    if game_object is None:
        return ""
    return game_object.peek_name() or ""


def names_in(assets_file: Any) -> Reference:
    """A reference that names its object's type and name only."""

    def name(file_id: int, path_id: int) -> dict[str, str] | None:
        target = resolve(assets_file, file_id, path_id)
        if target is None:
            return unresolved(assets_file, file_id, path_id)
        return {"type": target.type.name, "name": object_name(target)}

    return name


def script_name(reader: Any, tree: dict[str, Any]) -> str | None:
    """The class of a MonoBehaviour, with its namespace."""
    script = tree.get("m_Script")
    if not is_pointer(script):
        return None
    script_reader = resolve(reader.assets_file, script["m_FileID"], script["m_PathID"])
    if script_reader is None:
        return None
    script_tree = script_reader.read_typetree()
    namespace = script_tree.get("m_Namespace")
    class_name = script_tree.get("m_ClassName", "")
    return f"{namespace}.{class_name}" if namespace else class_name


def component(reader: Any, reference: Reference) -> dict[str, Any]:
    """A component's type, a MonoBehaviour's class as `script`, and its `fields` without the
    game object it belongs to."""
    entry = object_entry(reader, reference)
    entry["fields"].pop("m_GameObject", None)
    return entry


def object_entry(reader: Any, reference: Reference) -> dict[str, Any]:
    """An object's type, a MonoBehaviour's class as `script`, and its `fields`."""
    tree = reader.read_typetree()
    entry: dict[str, Any] = {"type": reader.type.name}
    if reader.type.name == "MonoBehaviour":
        entry["script"] = script_name(reader, tree)
        tree.pop("m_Script", None)
        if not tree.get("m_Name"):
            tree.pop("m_Name", None)
    if reader.type.name == "ParticleSystem":
        tree = enabled_modules(tree)
    entry["fields"] = plain(prune_curves(tree), reference)
    return entry


def bundle_dump(environment: Any, bundles: list[dict[str, str]]) -> dict[str, Any]:
    """Every object of the first bundle ({"name", "bundle", "file"?}): art and binary objects by
    name, clips with their curves, everything else with its fields. A reference names its object's
    type, name and `pathId`, and the game `bundle` it is in when that is another one, with the
    published `file` that bundle became (a path from the bucket root) when it has one; path IDs
    are strings, since they do not fit a JavaScript number."""
    bundle_of_file = {
        file_name.lower(): bundle["name"]
        for bundle in bundles
        for file_name in environment.files[bundle["bundle"]].files
    }
    published_file = {bundle["name"]: bundle["file"] for bundle in bundles if "file" in bundle}
    own_bundle = bundles[0]["name"]

    def reference_in(assets_file: Any) -> Reference:
        def reference(file_id: int, path_id: int) -> dict[str, str] | None:
            target = resolve(assets_file, file_id, path_id)
            if target is None:
                return unresolved(assets_file, file_id, path_id)
            entry = {"type": target.type.name, "name": object_name(target), "pathId": str(path_id)}
            target_bundle = bundle_of_file.get(target.assets_file.name.lower())
            if target_bundle != own_bundle:
                entry["bundle"] = target_bundle
                if target_bundle in published_file:
                    entry["file"] = published_file[target_bundle]
            return entry

        return reference

    objects = []
    for serialized in environment.files[bundles[0]["bundle"]].files.values():
        for path_id, reader in getattr(serialized, "objects", {}).items():
            reference = reference_in(reader.assets_file)
            kind = reader.type.name
            if kind == "AnimationClip":
                entry: dict[str, Any] = {"type": kind, "clip": clip_data(reader, reference)}
            elif kind in OPAQUE_TYPES:
                entry = {"type": kind, "name": object_name(reader)}
            else:
                entry = object_entry(reader, reference)
            objects.append({"pathId": str(path_id), **entry})
    return {"bundle": own_bundle, "objects": objects}


def enabled_modules(particle_system: dict[str, Any]) -> dict[str, Any]:
    """A particle system without the modules it switches off."""
    return {
        key: value
        for key, value in particle_system.items()
        if not (isinstance(value, dict) and value.get("enabled") in (False, 0))
    }


def prune_curves(value: Any) -> Any:
    """Drops the curves and colours a MinMaxCurve or MinMaxGradient does not use, by its
    `minMaxState`: constant, curve, two curves or two constants; colour, gradient, two colours,
    two gradients or a random colour."""
    if isinstance(value, list):
        return [prune_curves(item) for item in value]
    if not isinstance(value, dict):
        return value

    pruned = {key: prune_curves(item) for key, item in value.items()}
    state = pruned.get("minMaxState")
    if state is None:
        return pruned
    if "scalar" in pruned:
        unused = {0: ("minScalar", "minCurve", "maxCurve"), 1: ("minScalar", "minCurve"), 3: ("minCurve", "maxCurve")}
    elif "maxColor" in pruned:
        unused = {
            0: ("minColor", "minGradient", "maxGradient"),
            1: ("minColor", "maxColor", "minGradient"),
            2: ("minGradient", "maxGradient"),
            3: ("minColor", "maxColor"),
            4: ("minColor", "maxColor", "minGradient"),
        }
    else:
        return pruned
    for key in unused.get(state, ()):
        pruned.pop(key, None)
    return pruned


def curve_keys(clip: Any) -> dict[int, list[tuple[float, float]]]:
    """Keys by curve index from a Mecanim clip's streamed, dense and constant parts, which number
    their curves in that order."""
    data = clip.m_MuscleClip.m_Clip.data
    streamed, dense, constant = data.m_StreamedClip, data.m_DenseClip, data.m_ConstantClip
    stop = clip.m_MuscleClip.m_StopTime
    keys: dict[int, list[tuple[float, float]]] = collections.defaultdict(list)

    raw = struct.pack(f"<{len(streamed.data)}I", *streamed.data)
    frames = []
    offset = 0
    while offset < len(raw):
        time, count = struct.unpack_from("<fi", raw, offset)
        offset += 8
        frame_keys = []
        for _ in range(count):
            index, _in_slope, _out_slope, _unused, value = struct.unpack_from("<i4f", raw, offset)
            offset += 20
            frame_keys.append((index, value))
        frames.append((time, frame_keys))
    # Streamed clips open and close with sentinel frames.
    for time, frame_keys in frames[1:-1]:
        for index, value in frame_keys:
            keys[index].append((time, value))

    stream_count = streamed.curveCount
    for frame in range(dense.m_FrameCount):
        time = dense.m_BeginTime + frame / dense.m_SampleRate
        for curve in range(dense.m_CurveCount):
            keys[stream_count + curve].append((time, dense.m_SampleArray[frame * dense.m_CurveCount + curve]))

    constant_start = stream_count + dense.m_CurveCount
    for curve, value in enumerate(constant.data):
        keys[constant_start + curve] = [(0.0, value), (stop, value)]
    return keys


def gltf_component(attribute: int, component: int, value: float) -> float:
    """One component of a Unity transform value in glTF space, whose X is mirrored."""
    mirrored = {1: {0}, 2: {1, 2}, 4: {1, 2}}
    return -value if component in mirrored.get(attribute, set()) else value


def clip_bindings(clip: Any, reference: Reference) -> list[dict[str, Any]]:
    """Every binding of a Mecanim clip with its keys per component. A binding names its object
    by the CRC-32 of its path below the animated root, as the game does; transform values are
    in glTF space."""
    keys = curve_keys(clip)
    bindings = []
    index = 0
    for binding in clip.m_ClipBindingConstant.genericBindings:
        is_transform = binding.typeID == TRANSFORM_BINDING
        width = CURVES_PER_ATTRIBUTE.get(binding.attribute, 1) if is_transform else 1
        components = []
        for component_index in range(width):
            values = sorted(keys.get(index, []))
            if is_transform:
                values = [(time, gltf_component(binding.attribute, component_index, value)) for time, value in values]
            components.append([[number(time), number(value)] for time, value in values])
            index += 1

        entry: dict[str, Any] = {"path": binding.path, "typeID": binding.typeID}
        if is_transform:
            entry["attribute"] = TRANSFORM_ATTRIBUTES.get(binding.attribute, binding.attribute)
        else:
            entry["attribute"] = binding.attribute
            if binding.attribute in KNOWN_ATTRIBUTES:
                entry["attributeName"] = KNOWN_ATTRIBUTES[binding.attribute]
            entry["customType"] = binding.customType
            script = binding.script
            if script.m_PathID != 0:
                entry["script"] = reference(script.m_FileID, script.m_PathID)
        entry["keys"] = components
        bindings.append(entry)
    return bindings


def clip_data(reader: Any, reference: Reference) -> dict[str, Any]:
    """A clip's length, frame rate, events and every curve: by binding for a Mecanim clip, or
    as Unity stores them, by path, for a legacy one."""
    clip = reader.read()
    tree = reader.read_typetree()
    data: dict[str, Any] = {
        "name": clip.m_Name,
        "sampleRate": number(clip.m_SampleRate),
        "loop": bool(tree.get("m_AnimationClipSettings", {}).get("m_LoopTime", 0)),
        "events": plain(tree.get("m_Events", []), reference),
    }
    if clip.m_Legacy:
        data["legacy"] = True
        for field, key in LEGACY_CURVES.items():
            if tree.get(field):
                data[key] = plain(tree[field], reference)
        return data

    data["length"] = number(clip.m_MuscleClip.m_StopTime)
    data["bindings"] = clip_bindings(clip, reference)
    return data
