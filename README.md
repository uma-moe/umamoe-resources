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

## Docker Deployment

This repo now includes automatic Docker deployment via GitHub Actions in [.github/workflows/docker-deploy.yml](.github/workflows/docker-deploy.yml).

What it does on pushes to `main` or `master`:

- Builds the container image from [Dockerfile](Dockerfile)
- Pushes the image to GHCR as `ghcr.io/<owner>/umamoe-resources:sha-<commit>`
- Deploys the same image to beta and then production with separate remote app directories
- Uploads [deploy/docker-compose.yml](deploy/docker-compose.yml) and a generated runtime `.env` file to each remote target
- Pulls the new image on the server and restarts each environment with Docker Compose

The container runs the server on `0.0.0.0:3000` and stores `master.mdb` plus generated artifacts in the named Docker volume `resources-data`.

Fixed host ports in the workflow:

- Beta: `3104`
- Production: `3004`

Required GitHub secrets:

- `DEPLOY_HOST`
- `DEPLOY_PORT` optional, defaults to `22`
- `DEPLOY_SSH_KEY`
- `DEPLOY_KNOWN_HOSTS`
- `GHCR_PULL_USERNAME`
- `GHCR_PULL_TOKEN`
- `DATABASE_URL`
- `PUBLIC_BASE_URL`
- `MASTER_REFRESH_INTERVAL_SECONDS` optional, defaults to `300`
- `RUST_LOG` optional
- `CLOUDFLARE_ZONE_ID` optional
- `CLOUDFLARE_API_TOKEN` optional

Remote host requirements:

- Docker with the Compose plugin installed
- A user with permission to run `docker compose`
- Network access from the container to the Postgres instance referenced by `DATABASE_URL`

The remote app directories default to `/opt/umamoe-resources-beta` for beta and `/opt/umamoe-resources` for production inside [.github/workflows/docker-deploy.yml](.github/workflows/docker-deploy.yml).