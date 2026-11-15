"""Turns decrypted Unity asset bundles into the images and models the site serves.

The resources service decrypts each bundle and sends a job list as JSON on stdin:
{"jobs": [{"bundle": "<path>", "dependencies": ["<path>"], "outputs": [{"object": "<name>", "key": "<bucket key>", "path": "<path>",
"format": "webp" | "png" | "glb", "size": [width, height] | null,
"art_size": [width, height] | null}]}]}
`dependencies` are loaded with the bundle, such as the texture an atlas's sprites draw from or
the meshes, materials and textures a track prefab draws.
`size` null keeps the picture's own size. `art_size` is the art's real shape when it is centred
on a larger served canvas.
A `glb` output exports the prefab `object` with `game_art_model`, its textures embedded.
`runtime_textures` ({"name", "key", "bundle"}) are the season and weather textures the game puts
on the prefab's course materials; each `bundle` is one of the `dependencies`.
It prints {"results": [{"bundle": "<path>", "written": [...], "error": null | "<message>"}]}.
`--inspect` lists the textures and sprites in the bundles named on the command line instead.
"""

import json
import os
import sys
from typing import Any, TextIO

import UnityPy
from PIL import Image

from game_art_model import export_prefab

WEBP_QUALITY = 90


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


def convert_bundle(job: dict[str, Any]) -> dict[str, Any]:
    try:
        bundle_paths = [job["bundle"], *job.get("dependencies", [])]
        environment = UnityPy.load(*bundle_paths)

        images = None
        written = []
        for output in job["outputs"]:
            if output["format"] == "glb":
                export_prefab(environment, job["bundle"], output, job.get("runtime_textures", []))
            else:
                images = images or read_images(environment)
                save_image(pick_image(*images, output["object"]), output)
            written.append(output["path"])
        return {"bundle": job["bundle"], "written": written, "error": None}
    except Exception as error:
        return {"bundle": job["bundle"], "written": [], "error": f"{type(error).__name__}: {error}"}


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
