"""Turns decrypted Unity asset bundles into the images and models the site serves.

The resources service decrypts each bundle and sends a job list as JSON on stdin:
{"jobs": [{"bundle": "<path>", "dependencies": ["<path>"], "outputs": [{"object": "<name>", "key": "<bucket key>", "path": "<path>",
"format": "webp" | "png" | "glb" | "json", "size": [width, height] | null,
"art_size": [width, height] | null}]}]}
`dependencies` are loaded with the bundle, such as the texture an atlas's sprites draw from or
the meshes, materials and textures a track prefab draws.
`size` null keeps the picture's own size. `art_size` is the art's real shape when it is centred
on a larger served canvas.
A `glb` output exports the job's `model`, its textures embedded, by its `kind`:
- "scenery", with `game_art_model`: {"prefabs": [{"object", "bundle"}], "runtime_textures":
  [{"name", "key", "bundle"}], "components": bool}. The prefabs are drawn side by side, such as a
  racecourse's track, trees, stands and sky. The runtime textures are the season and weather
  textures the game puts on their course materials. With `components`, as for an effect or a
  cut-in set, each object's particle systems, scripts and other components are kept.
- "uma", with `game_art_uma`: {"title", "prefabs", "tail_textures": [{"slot", "bundle"}],
  "traits"}, an outfit's body, head and tail joined in a T-pose.
- "character", with `game_art_uma`: {"title", "prefabs", "material_textures": [{"material",
  "slot", "bundle"}], "traits"?}, a chibi or a prop.
- "motion", with `game_art_uma`: {"title", "skeleton": prefabs, "moves_root": bool}, the job's
  own bundle, a clip, on the skeleton of its family.
- "node_motion", with `game_art_uma`: {"title"}, the job's own clip, which moves a camera or a
  position.
Each `bundle` is the job's own or one of its `dependencies`.
A `json` output is written by the model's `kind`:
- "shaders", with `game_art_shader`: the index of the shaders in the job's bundle; each shader
  goes in a file of its own beside the index, at a key named after it.
- "clip_curves", with `game_art_unity`: every curve of the job's own clip.
- "scripts", with `game_art_unity`: {"bundles": [{"name", "bundle"}]}, every object of the first
  bundle, such as a cut-in timeline, the others named so its references can be.
It prints {"results": [{"bundle": "<path>", "written": [...], "files": [{"key", "path"}],
"error": null | "<message>"}]}, where `files` are the files a job named after its bundle's contents.
`--inspect` lists the textures and sprites in the bundles named on the command line instead.
"""

import json
import os
import struct
import sys
from typing import Any, TextIO

import UnityPy
from PIL import Image

import game_art_unity as unity
from game_art_model import bundle_objects, export_model
from game_art_shader import export_shaders
from game_art_uma import export_character, export_motion, export_node_motion, export_uma

WEBP_QUALITY = 90
GLB_JSON_CHUNK = 0x4E4F534A


def read_images(environment: Any) -> tuple[dict[str, Any], dict[str, Any]]:
    sprites: dict[str, Any] = {}
    textures: dict[str, Any] = {}
    for obj in environment.objects:
        if obj.type.name == "Sprite":
            data = obj.read()
            sprites[data.m_Name] = data
        elif obj.type.name == "Texture2D":
            data = obj.read()
            textures[data.m_Name] = data

    return sprites, textures


def pick_image(sprites: dict[str, Any], textures: dict[str, Any], name: str) -> Image.Image:
    # Atlases hold many named sprites; single-image bundles hold one texture named like the file.
    if name in sprites:
        return sprites[name].image
    if name in textures:
        return textures[name].image
    # An atlas's one texture holds all its sprites, so only a bundle without sprites falls back.
    if not sprites and len(textures) == 1:
        return next(iter(textures.values())).image

    available = sorted([*sprites, *textures])
    raise ValueError(f"no sprite or texture named {name!r}; bundle has {available[:20]}")


def fit_image(
    image: Image.Image, size: list[int] | None, art_size: list[int] | None
) -> Image.Image:
    if size is None:
        return image

    # The game squeezes textures to powers of two; resizing restores the served shape.
    if art_size is None:
        if list(image.size) == size:
            return image
        return image.resize((size[0], size[1]), Image.LANCZOS)

    art = image.convert("RGBA").resize((art_size[0], art_size[1]), Image.LANCZOS)
    canvas = Image.new("RGBA", (size[0], size[1]), (0, 0, 0, 0))
    canvas.alpha_composite(art, ((size[0] - art_size[0]) // 2, (size[1] - art_size[1]) // 2))
    return canvas


def save_image(image: Image.Image, output: dict[str, Any]) -> None:
    image = fit_image(image, output.get("size"), output.get("art_size"))

    if output["format"] == "png":
        image.save(output["path"], "PNG", optimize=True)
        return

    image.save(output["path"], "WEBP", quality=WEBP_QUALITY, method=6)


def export_glb(environment: Any, job: dict[str, Any], output: dict[str, Any]) -> None:
    model = job["model"]
    kind = model["kind"]
    if kind == "scenery":
        components = model.get("components", False)
        export_model(environment, model["prefabs"], model["runtime_textures"], output, components)
    elif kind == "uma":
        export_uma(environment, model, output)
    elif kind == "character":
        export_character(environment, model, output)
    elif kind == "motion":
        export_motion(environment, job["bundle"], model, output)
    elif kind == "node_motion":
        export_node_motion(environment, job["bundle"], model, output)
    else:
        raise ValueError(f"unknown model kind {kind!r}")


def export_json(environment: Any, job: dict[str, Any], output: dict[str, Any]) -> list[dict[str, str]]:
    """Writes the output and returns the further files it names, if any."""
    model = job.get("model", {})
    kind = model.get("kind")
    if kind == "shaders":
        return export_shaders(bundle_objects(environment, job["bundle"], "Shader"), output)
    if kind == "clip_curves":
        clips = bundle_objects(environment, job["bundle"], "AnimationClip")
        if not clips:
            raise ValueError("the bundle holds no animation clip")
        content = unity.clip_data(clips[0], unity.names_in(clips[0].assets_file))
    elif kind == "scripts":
        content = unity.bundle_dump(environment, model["bundles"])
    else:
        raise ValueError(f"no JSON output for model kind {kind!r}")

    with open(output["path"], "w", encoding="utf-8") as file:
        json.dump(content, file, separators=(",", ":"), ensure_ascii=False, allow_nan=False)
    return []


def glb_shaders(path: str) -> set[str]:
    """The published shader files the model's materials name, as paths from the tree's root."""
    with open(path, "rb") as file:
        magic, _version, _length = struct.unpack("<4sII", file.read(12))
        chunk_length, chunk_type = struct.unpack("<II", file.read(8))
        if magic != b"glTF" or chunk_type != GLB_JSON_CHUNK:
            raise ValueError(f"{path} is not a binary glTF")
        document = json.loads(file.read(chunk_length))

    shaders = set()
    for material in document.get("materials", []):
        shader = material.get("extras", {}).get("unity", {}).get("shader")
        if isinstance(shader, dict) and "file" in shader:
            shaders.add(shader["file"])
    return shaders


def convert_bundle(job: dict[str, Any]) -> dict[str, Any]:
    try:
        bundle_paths = [job["bundle"], *job.get("dependencies", [])]
        environment = UnityPy.load(*bundle_paths)

        images = None
        written = []
        files = []
        shaders: set[str] = set()
        for output in job["outputs"]:
            if output["format"] == "glb":
                export_glb(environment, job, output)
                shaders |= glb_shaders(output["path"])
            elif output["format"] == "json":
                files.extend(export_json(environment, job, output))
            else:
                images = images or read_images(environment)
                save_image(pick_image(*images, output["object"]), output)
            written.append(output["path"])
        return {
            "bundle": job["bundle"],
            "written": written,
            "files": files,
            "shaders": sorted(shaders),
            "error": None,
        }
    except Exception as error:
        return {"bundle": job["bundle"], "written": [], "files": [], "error": f"{type(error).__name__}: {error}"}


def inspect_bundle(bundle_path: str) -> dict[str, Any]:
    sprites, textures = read_images(UnityPy.load(bundle_path))
    return {
        "bundle": bundle_path,
        "sprites": {name: list(sprite.image.size) for name, sprite in sorted(sprites.items())},
        "textures": {
            name: {"size": [texture.m_Width, texture.m_Height], "format": str(texture.m_TextureFormat)}
            for name, texture in sorted(textures.items())
        },
    }


def take_stdout() -> TextIO:
    """Keeps stdout for the JSON result; anything else written there, from Python or native
    decoders, goes to stderr instead."""
    sys.stdout.flush()
    result = os.fdopen(os.dup(1), "w")
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return result


def main() -> int:
    result = take_stdout()
    if len(sys.argv) > 1 and sys.argv[1] == "--inspect":
        payload: Any = [inspect_bundle(path) for path in sys.argv[2:]]
    else:
        request = json.load(sys.stdin)
        payload = {"results": [convert_bundle(job) for job in request["jobs"]]}

    json.dump(payload, result)
    result.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
