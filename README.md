# umamoe-resources

Rust webserver for preparing semi-static Umamusume resource JSON from `master.mdb` and serving it as versioned `json.gz` streams.

## What Exists Now

- `factors.json` generation from `text_data` category `147`
- `race_program.json` generation from `single_mode_program`
- `character_banners.json`, `supports_banners.json`, and `paid_gacha_banners.json` generation from `gacha_data` + `gacha_available`
- `affinity.json` generation from `succession_relation` + `succession_relation_member`
- `character_names.json` overlay generation from the bundled mapping, with names refreshed from global `text_data` category `6`
- `character.json`, `supports.json`, `support-cards-db.json`, and `skills.json` DB-first generation from `master.mdb`
- Versioned gzip artifacts under `generated-data/{master-version}/`
- `generated-data/manifest.json` for frontend discovery
- HTTP serving with CDN-friendly cache headers
- Cloudflare purge hook for mutable `manifest` and `current` URLs
- Existing `master_fetch.rs` integration for `master_versions` driven refreshes

The next good port is `campaign-extract.py`. Full `db-convert.py`, tierlist precompute, and statistics are larger domain ports and should be moved after the pipeline shape is stable.

## Serve Resources

```powershell
cargo run
```

By default, the server generates resources from `master.mdb` into `generated-data`, then serves them at `127.0.0.1:3000`.

If `DATABASE_URL` is set, the server also checks `master_versions` before startup and then keeps polling in the background. When the version changes, it downloads a fresh `master.mdb`, regenerates `generated-data`, and the HTTP server starts serving the refreshed assets automatically.

Equivalent explicit command:

```powershell
cargo run -- serve --master master.mdb --data-dir generated-data --bind 127.0.0.1:3000 --refresh-interval-seconds 300
```

If you want mutable CDN URLs purged automatically after a background refresh:

```powershell
cargo run -- serve --purge-on-refresh
```

To serve the existing `generated-data` directory without regenerating first:

```powershell
cargo run -- serve --no-generate
```

## Frontend Integration

The frontend should treat this service as a resource origin rooted at `/resources`.

Public URL:

```text
https://uma.moe/resources
```

That means the frontend should first request:

```text
https://uma.moe/resources/manifest.json
```

Then it should read the manifest and request the versioned gzip artifact paths listed there, such as `/resources/1.21.0-10005900/skills.json.gz`.

For Docker-to-Docker traffic, use the internal service hostname on the Docker network instead of the public domain. Example if the container service is named `umamoe-resources`:

```text
http://umamoe-resources:3000/resources
```

So the internal manifest URL would be:

```text
http://umamoe-resources:3000/resources/manifest.json
```

Recommended frontend flow:

- Load `/resources/manifest.json`
- Read the current `version`
- Request only the versioned artifact paths from the manifest
- Treat `/resources/current/*` and `/resources/manifest.json` as mutable, short-cache endpoints
- Treat `/resources/{version}/*` as immutable CDN-friendly assets

The server also accepts legacy `.json` URLs, but the manifest advertises `.json.gz` URLs and those should be treated as canonical.

Useful routes:

- `/resources/` - default resource API entry point; returns the latest manifest
- `/resources/manifest.json` - short CDN cache, points the frontend at the current version
- `/resources/healthz` - resource API health check
- `/resources/current/sql?sql=SELECT%201` - read-only ad hoc SQLite query endpoint against `master.mdb`
- `/resources/current/factors.json.gz` - short CDN cache, convenient mutable alias
- `/resources/{version}/factors.json.gz` - one-year immutable CDN cache
- `/resources/{version}/race_program.json.gz` - one-year immutable CDN cache
- `/resources/{version}/character_banners.json.gz` - one-year immutable CDN cache
- `/resources/{version}/supports_banners.json.gz` - one-year immutable CDN cache
- `/resources/{version}/paid_gacha_banners.json.gz` - one-year immutable CDN cache
- `/resources/{version}/affinity.json.gz` - one-year immutable CDN cache
- `/resources/{version}/character_names.json.gz` - one-year immutable CDN cache
- `/resources/{version}/character.json.gz` - one-year immutable CDN cache
- `/resources/{version}/supports.json.gz` - one-year immutable CDN cache
- `/resources/{version}/support-cards-db.json.gz` - one-year immutable CDN cache
- `/resources/{version}/skills.json.gz` - one-year immutable CDN cache

All resource JSON routes return precompressed bytes with `Content-Encoding: gzip` and `Content-Type: application/json; charset=utf-8`.

The SQL endpoint returns normal JSON, not gzip. It only accepts a single `SELECT` or `WITH` statement, executes it against `master.mdb` through a read-only SQLite connection, and responds as `{ "columns": [...], "rows": [[...]], "truncated": false }`.

## Beta And Local Auth

Protected resource routes still use browser-proof validation by default.

For beta or local development, you can configure a shared static token instead:

```powershell
$env:STATIC_AUTH_TOKEN="replace-me"
```

The server also accepts `BETA_STATIC_AUTH_TOKEN` as a fallback env name if you want the variable to stay beta-specific.

When either variable is set, protected routes accept either of these headers:

```text
Authorization: Bearer replace-me
X-API-Key: replace-me
```

Leave the variable unset in normal production if you only want Redis-backed browser-proof access.

## Generate Only

```powershell
cargo run -- generate --master master.mdb --out generated-data
```

Generation emits `character_names.json` from the bundled `src/character_names.json` mapping, preserving the existing JP/mapping and skin entries while overwriting known character `name` fields with global names from `master.mdb`.

`character.json`, `supports.json`, `support-cards-db.json`, and `skills.json` are generated from `master.mdb` and the bundled character-name mapping only. No external frontend data directory is read during generation.

For debugging, also write plain JSON beside the gzip files:

```powershell
cargo run -- generate --master master.mdb --out generated-data --write-json
```

Output layout:

```text
generated-data/
	manifest.json
	1.21.0-10005900/
		manifest.json
		factors.json.gz
		race_program.json.gz
		character_banners.json.gz
		supports_banners.json.gz
		paid_gacha_banners.json.gz
		affinity.json.gz
		character_names.json.gz
		character.json.gz
		supports.json.gz
		support-cards-db.json.gz
		skills.json.gz
```

## Refresh From `master_versions`

The long-running server already supports this flow automatically when `DATABASE_URL` is set. The `refresh` command is still useful for one-shot maintenance runs.

When `DATABASE_URL` points at the DB containing `master_versions`, this checks for a new app/resource version, downloads a fresh `master.mdb`, regenerates resources, and optionally purges Cloudflare current URLs.

Copy `.env.example` to `.env` for local configuration. The checked-in example documents the Postgres, Cloudflare, public URL, and logging variables used by the CLI.

```powershell
$env:DATABASE_URL="postgresql://user:pass@host:5432/db"
$env:CLOUDFLARE_ZONE_ID="..."
$env:CLOUDFLARE_API_TOKEN="..."
$env:PUBLIC_BASE_URL="https://uma.moe"

cargo run -- refresh --master master.mdb --out generated-data --purge
```

If the master marker has not changed, regeneration is skipped.

## Cache Strategy

The frontend should load `/resources/manifest.json`, then request the versioned gzip artifact paths listed in it. Versioned paths can sit in Cloudflare for a year because the URL changes whenever `master.mdb.version` changes.

Only these mutable URLs need purging after regeneration:

- `/resources/manifest.json`
- `/resources/current/*.json.gz`

You can purge them manually from the latest manifest:

```powershell
cargo run -- purge --data-dir generated-data
```

## Local Docker

For local containerized development, use [compose.yaml](compose.yaml). It builds the service image, starts Redis, mounts your local `master.mdb` and `generated-data`, and exposes the API on `127.0.0.1:3000`.

Start it with:

```powershell
docker compose up --build
```

The compose stack sets `STATIC_AUTH_TOKEN=local-dev-token` by default so protected routes are immediately testable. You can override that by creating a local `.env` from `.env.example` and changing `STATIC_AUTH_TOKEN` there.

Example request from Windows PowerShell:

```powershell
curl.exe -H "X-API-Key: local-dev-token" http://127.0.0.1:3000/resources/manifest.json
```

If you want automatic `master_versions` refreshes inside Docker, set `RESOURCE_DATABASE_URL` in `.env` instead of `DATABASE_URL`. The hostname must be reachable from the container, so use `host.docker.internal` or another container name, not `127.0.0.1`.

## Docker Deployment

This repo now deploys the same way as the backend/statistics style image-archive workflows: build the image in GitHub Actions, upload it as an artifact, copy it to the host, `docker load` it there, and restart the target container with `docker run`.

What the workflow in [.github/workflows/docker-deploy.yml](.github/workflows/docker-deploy.yml) does on pushes to `main` or `master`:

- Builds the container image from [Dockerfile](Dockerfile)
- Saves the built image as a compressed artifact
- Deploys beta first, then production
- Copies the image archive to the remote host
- Recreates the target container with `docker run`
- Leaves your manual runtime env files alone

Manual runtime env files on the remote host:

- Beta: `/opt/umamoe-resources-beta/env`
- Production: `/opt/umamoe-resources/env`

Those files are not written or overwritten by the workflow.

Explicit runtime names managed by the workflow:

- Beta container: `umamoe-resources-beta`
- Production container: `umamoe-resources`
- Beta data volume: `umamoe-resources-beta-data`
- Production data volume: `umamoe-resources-data`

Fixed host ports in the workflow:

- Beta: `3104`
- Production: `3004`

Required GitHub secrets:

- `DEPLOY_HOST`
- `DEPLOY_PORT` optional, defaults to `22`
- `DEPLOY_SSH_KEY`
- `DEPLOY_KNOWN_HOSTS`

Manual remote `env` files should contain the normal application env values only:

- `DATABASE_URL`
- `MASTER_REFRESH_INTERVAL_SECONDS`
- `CLOUDFLARE_ZONE_ID`
- `CLOUDFLARE_API_TOKEN`
- `PUBLIC_BASE_URL`
- `RUST_LOG`

Recommended environment-specific values:

- Beta `env`: `DATABASE_URL=...`, `MASTER_REFRESH_INTERVAL_SECONDS=300`, `PUBLIC_BASE_URL=https://beta.uma.moe`
- Production `env`: `DATABASE_URL=...`, `MASTER_REFRESH_INTERVAL_SECONDS=300`, `PUBLIC_BASE_URL=https://uma.moe`

Remote host requirements:

- Docker installed
- A user with permission to run `docker`
- Enough temporary disk space under `/tmp/umamoe-resources-images` for the uploaded image archive during deployment
- Network access from the container to the Postgres instance referenced by `DATABASE_URL`

The workflow mounts the named Docker volume at `/data`, so refreshed `master.mdb` and generated resource artifacts persist across container replacements.