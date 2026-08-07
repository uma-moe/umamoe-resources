# umamoe-resources

Rust webserver for preparing semi-static Umamusume resource JSON from `master.mdb` and serving it as versioned `json.gz` streams.

## What Exists Now

- `factors.json` generation from `text_data` category `147`
- `race_program.json` generation from `single_mode_program`
- `character_banners.json`, `supports_banners.json`, and `paid_gacha_banners.json` generation from `gacha_data` + `gacha_available`
- `banner_timeline.json` generation from bundled JP banner/event history plus confirmed global anchors from `master.mdb` and `src/jp_data/confirmed_global_banner_dates.csv`
- `jp_news_events.json` from an incremental umapyoi.net archive of every JP news post and detailed gacha record, including event-family tags and discovered image/banner URLs
- `planner_rewards.json` Global gift rewards from durable, revision-preserving archives of the official English news API and `@umamusume_eng` social posts
- `affinity.json` generation from `succession_relation` + `succession_relation_member`
- `character_names.json` overlay generation from the bundled mapping, with names refreshed from global `text_data` category `6`
- `character.json`, `supports.json`, `support-cards-db.json`, and `skills.json` DB-first generation from `master.mdb`
- Versioned gzip artifacts under `generated-data/{master-version}-timeline-{hash}/`
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
- `/resources/{version}/banner_timeline.json.gz` - one-year immutable CDN cache
- `/resources/{version}/jp_news_events.json.gz` - one-year immutable JP news/gacha archive
- `/resources/{version}/affinity.json.gz` - one-year immutable CDN cache
- `/resources/{version}/character_names.json.gz` - one-year immutable CDN cache
- `/resources/{version}/character.json.gz` - one-year immutable CDN cache
- `/resources/{version}/supports.json.gz` - one-year immutable CDN cache
- `/resources/{version}/support-cards-db.json.gz` - one-year immutable CDN cache
- `/resources/{version}/skills.json.gz` - one-year immutable CDN cache
- `/resources/{version}/simulator_course_geometry_<course-id>.json.gz` -
  source-backed, independently fetchable 1,001-frame course world transforms

All resource JSON routes return precompressed bytes with `Content-Encoding: gzip` and `Content-Type: application/json; charset=utf-8`.

Simulator course geometry is intentionally split into one artifact per course.
Clients should request only the active course instead of loading all geometry at
startup. The bundled source data is extracted from the client `CourseLaneAnim`
assets and the generator validates every current `race_course_set` row has an
exact matching course id, race track, distance, finite transform columns, and
non-zero quaternion at each of its 1,001 samples.

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

## Synchronize JP News And Gacha Metadata

The umapyoi API exposes gachas separately, but most other JP event families only exist as news posts. This command fetches the complete news index, downloads posts not already present in the local archive, refreshes detailed gacha records, classifies known event families, and retains the raw response so newly discovered fields are not lost:

```powershell
cargo run -- sync-umapyoi
```

The result is stored in `src/jp_data/umapyoi_archive.json` and emitted as `jp_news_events.json` during normal resource generation. Each news item contains its source page, normalized title/date fields when present, event tags such as `story_event`, `champions_meeting`, `training_scenario`, `league_of_heroes`, `legend_race`, `campaign`, and `gacha`, plus every image URL with its original JSON field path and a `likely_banner` hint. URLs named `gacha_banner_<id>` are emitted as structured `gacha_banners`, allowing news-only JP pickup banners to expand automatically. Select Pickup support banners are marked as master gacha type `12`, tagged as reruns, and enriched with every support-card candidate named by the news post. Detailed gacha entries embed normalized support-card pickup metadata.

The top-level `analysis` object is regenerated from the saved raw payloads. It reports classification counts, date coverage, image/banner coverage, discovered gacha-banner IDs, detailed API gacha count, and merged support-card count. Run `cargo run -- sync-umapyoi --offline` to rebuild normalized fields and analysis without accessing umapyoi.net.

The importer defaults to one request every 550 ms (about 1.8 requests/second). This stays below umapyoi's tightest sustained limits of 7,200 requests/hour and 172,800/day, as well as the 10/second and 500/minute burst limits. The scheduled job uses a gentler one request/second because sustained faster crawls have made the upstream service unstable. HTTP 429 and 5xx responses are retried with exponential backoff. Use `--request-interval-ms` to go slower; values below 500 ms are rejected. Routine runs only request the three index endpoints plus details for IDs that are not already archived. Image binaries are never downloaded from umapyoi.net; the importer only preserves image URLs embedded in news/API JSON.

Existing news posts are not fetched again, so routine refreshes are incremental. Use `--full` only when upstream corrected old posts or the extraction logic needs to reprocess the original payloads:

```powershell
cargo run -- sync-umapyoi --full
```

## Synchronize Official Global Rewards

Official English announcements are archived before resource generation so temporary or later-deleted posts remain auditable. Each post keeps its raw API payload, first-seen timestamp, and every changed snapshot rather than deleting missing upstream records or overwriting corrections:

```powershell
cargo run -- sync-global-news
cargo run -- sync-global-social
```

Website news is stored in `src/global_data/official_news_archive.json`. Social posts are stored separately in `src/global_data/official_social_archive.json`. Social discovery reads the public official X profile plus X's public syndication history, then verifies and stores the canonical text through X's own oEmbed endpoint. The live profile is checked often enough to capture posts before they fall out of its small public response; records and changed snapshots are never removed from either archive when a source later deletes them.

Normal planner generation extracts exact free-Carat gifts and detailed login bonuses as default-enabled `global_news` or `global_social` rewards. Correction and erratum notices are retained in the archive but never create a new income entry; the original distribution announcement remains the single source of that gift. A social post must explicitly say the gift was distributed; follow/repost contests, chance-to-win prizes, previews, paid-Carat sales, entry costs, and qualitative or player-dependent rewards are retained in the raw archives but not added as fixed planner income. Same-day, same-amount website and social announcements are counted once. A matching Global reward replaces the corresponding future JP-news estimate, while login bonuses already represented by Global master data are not counted twice.

`planner_rewards.json` also publishes `global_reward_comparison`. Website posts are matched to JP only by their shared numeric announce ID, then split into the Global-minus-JP delta for matched news and the full reward for EN-only news. Official social rewards are a third bucket after website/social deduplication; the resource exposes both the retained social total and the count/Carat value of overlapping items removed. For long-term planning, `speculative_monthly_carats` is the arithmetic mean of those mutually exclusive uplift buckets across the latest six completed calendar months, including zero-reward months. This preserves infrequent large gifts in the expected total while adapting relatively quickly when Global rewards trend down or up. The six inputs, their median, and the whole-archive mean are also published for auditability. The frontend starts that projection only after the latest confirmed reward date, preventing confirmed and speculative income from overlapping.

`.github/workflows/sync-umapyoi.yml` runs the JP, Global website, and Global social syncs every six hours and commits the archives only when source content changed. It can also be started manually with the full-refresh option for the JP source.

Timeline images are stored in the frontend rather than hotlinked at runtime. Generate a plain `banner_timeline.json`, then run `node scripts/sync_umapyoi_timeline_images.mjs --timeline-json <path> --frontend-root <umamoe-frontend>`. The Node/Sharp sync consumes the archive maintained by the Rust `sync-umapyoi` command; it is not a separate Python scraper. The original files below `assets/images/...` and `assets/timeline-images/...` are immutable production fallbacks: the sync never writes, deletes, or restores those paths. Official JP news-post artwork is stored separately below `assets/timeline-images/jp/`; official EN news-post artwork is stored below `assets/timeline-images/en/`. Each locale has a manifest mapping the unchanged timeline `image_path` to its overlay only after that overlay exists and opens successfully. The frontend resolves EN first, then JP, then the untouched legacy path, so removing either overlay directory is a complete rollback. Pass `--refresh-jp` to refresh only the managed JP overlay.

The JP resolver reads the bundled official Umapyoi news archive. It uses exact gacha IDs, stable asset identities, event family and release dates, title similarity, and semantic campaign phases to select official post artwork. EN matching audits the official public news API and uses the same stable identities and campaign semantics; regional campaign IDs are not assumed to match. For campaign posts it verifies and prefers the official `Thumbnail/banner_*` counterpart over a shallow `Header/header_*` image. Equal-date EN matches without a semantic identity are rejected as ambiguous. Historical official EN URLs that predate the public index remain recorded in `src/jp_data/english_mission_campaign_assets.json`. JP images are downscaled only when wider than 512 px and preserve the official aspect ratio without cropping, padding, or stretching. EN replacements use the selected JP overlay dimensions as their target, receive at most a 10% vertical adjustment, and are never cropped or padded. Source URLs and transform versions are recorded in `timeline-image-sync/japanese-image-sources.json` and `english-image-sources.json`, making subsequent runs idempotent and allowing a changed official URL to refresh only its overlay. Missing EN candidates are retried during the release window and retired seven days after release. The frontend `sync-timeline-images.yml` workflow runs at 13:15 JST, after the metadata sync, and commits only files reported by the image sync. Timeline events also expose the matching `umapyoi_url` so clients can link back to the archived source post.

The bundled mission-campaign catalogue is generated from `mission_data` rows with `mission_type = 4`. Refresh it from an installed JP master database with `scripts/sync_jp_mission_campaign_catalog.py --jp-master <jp-master.mdb> --frontend-root <umamoe-frontend>`. Point `--jp-image-source` (or `UMAMUSUME_JP_CAMPAIGN_ASSET_DIR`) at an AssetRipper directory containing `tex_campaign_mission_logo_<id>.png`; a same-aspect-ratio WebP is created only when a legacy fallback does not already exist, and existing production assets are never overwritten. Every newly available mission group is added to the catalogue automatically. The catalogue records the JP mission event ID, exact mission-condition/reward fingerprint, mission count, and JP display title. During normal resource generation, those fingerprints are compared with the current EN `master.mdb`; an EN event ID is only a guarded fallback when content matching is unavailable. This supplies the actual EN title and global start/end dates even when the JP image ID, JP mission event ID, and EN mission event ID are different. A semantic phase identity such as `fall-g1-missions-part-1` applies official news artwork to every matching mission row in the same release phase, so later races self-resolve without manual ranges.

Generation emits `character_names.json` from the bundled `src/character_names.json` mapping, preserving the existing JP/mapping and skin entries while overwriting known character `name` fields with global names from `master.mdb`.

`character.json`, `supports.json`, `support-cards-db.json`, and `skills.json` are generated from `master.mdb` and the bundled character-name mapping only. No external frontend data directory is read during generation.

Confirmed global timeline dates for `banner_timeline.json` live in `src/jp_data/confirmed_global_banner_dates.csv`. To confirm a newly announced schedule entry, append one line:

```csv
character,30104,2026-07-02
support,30105,2026-07-02
paid,50009,2026-07-08
story,07_uma_musume_summer_story_banner,2025-10-14
champions,14,2026-06-21
legend,12,2026-06-07
league_of_heroes,2023-05-12,2026-08-15
masters_challenge,2023-12-28,2026-09-01
trainer_skills_test,2024-02-24,2026-10-10
factor_research,2024-03-21,2026-11-08
strongest_team,2024-04-22,2026-12-01
racing_carnival,2024-10-11,2026-12-15
training_scenario,2022-08-24,2026-12-20
anniversary,1,2025-10-26
```

Banner rows accept a bare gacha id, image stem, `.png`, or `.webp`. Story and campaign rows use the image stem or filename. Champions Meeting and Legend Race rows use the sorted 0-based index, or the full key such as `champions_meeting_14`. Anniversary rows use the marker index, where `1` is the first half-anniversary. `YYYY-MM-DD` dates are emitted at `22:00 UTC`.

Months are inferred as complete schedules. If the CSV contains any confirmed timeline entry in a global month, `banner_timeline.json` treats that whole month as closed and shifts unknown future predictions and unconfirmed anniversary markers out of that month. This matches the monthly schedule release pattern: once July is entered, there should be no more unconfirmed July entries predicted by the site.

`banner_timeline.json` includes character, support, paid, story, Champions Meeting, Legend Race, campaign, League of Heroes, Masters Challenge, Trainer Skills Test, Factor Research, Aim! The Strongest Team, Racing Carnival, and Training Scenario releases, plus anniversary marker metadata in the top-level `anniversaries` array. Champions Meetings after the bundled April 2024 dataset are extended from their start news posts; matching older posts provide local banner images without duplicating the bundled rows. News-discovered Pick 2 reruns, Twinkle Collection, scenario, and guaranteed/paid gachas are merged into the existing banner families. Every gacha row exposes the numeric `gacha_type` and a stable `gacha_type_name`: `standard_pool`, `makeup_debut`, `standard_pickup`, `guaranteed`, `group_select`, `twinkle_collection`, `pick_2`, `special_guaranteed`, `select_step_up`, or `stamp_sheet`. Pick 2 banners let the player choose two SSR support cards from the candidate pool. Character and support banners whose complete pickup set appeared on earlier dates receive the `rerun-banner` tag; same-day duplicate rows are not treated as reruns. Character and support banners define the baseline acceleration curve; news-derived special banners do not become confirmed calibration anchors. Other family confirmations apply isolated same-family residual corrections on top of that baseline.

Monthly Match is intentionally excluded because the JP history contains only one beta run. Generic campaigns, login bonuses, updates, maintenance, media announcements, and one-off challenges remain in `jp_news_events.json` instead of becoming timeline sections because their schedules overlap heavily or do not represent a repeatable content family.

The resource also includes prediction likelihood metadata. The calculation block reports observed character-banner monthly count, adjacent character-banner gap, weekday, and day-of-month frequencies; unconfirmed events include a compact `prediction.calendar_likelihood` score showing how common that predicted month shape, spacing, and release date are compared with confirmed schedules.

For server-side phone-friendly updates, edit the mounted file on the host:

```text
/opt/umamoe-resources/config/confirmed_global_banner_dates.csv
```

The deploy workflow mounts that directory into the production container at `/config` and starts the service with:

```text
CONFIRMED_BANNER_DATES_PATH=/config/confirmed_global_banner_dates.csv
CONFIRMED_BANNER_DATES_REFRESH_INTERVAL_SECONDS=60
```

The container polls the mounted file, validates the CSV, regenerates resources when it changes, and purges mutable Cloudflare resource URLs after confirmed-date edits when Cloudflare purge credentials are configured.
The mounted file is layered on top of the bundled CSV, so it can contain only new or corrected lines instead of the full history.

If you prefer editing a raw GitHub/Gist URL instead of the server file, configure:

```text
CONFIRMED_BANNER_DATES_URL=https://raw.githubusercontent.com/<owner>/umamoe-resources/master/src/jp_data/confirmed_global_banner_dates.csv
CONFIRMED_BANNER_DATES_REFRESH_INTERVAL_SECONDS=60
```

The confirmation CSV hash is part of the generated resource version, so clients that read `/resources/manifest.json` will see a new versioned `banner_timeline.json.gz` path after an edit.
`PURGE_ON_REFRESH` only controls cache purging for automatic `master.mdb` refreshes; confirmed-date refreshes always purge the mutable manifest/current URLs.

For debugging, also write plain JSON beside the gzip files:

```powershell
cargo run -- generate --master master.mdb --out generated-data --write-json
```

Output layout:

```text
generated-data/
	manifest.json
	1.21.0-10005900-timeline-0123abcd4567/
		manifest.json
		factors.json.gz
		race_program.json.gz
		room_match_races.json.gz
		character_banners.json.gz
		supports_banners.json.gz
		paid_gacha_banners.json.gz
		banner_timeline.json.gz
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
- Beta confirmed-banner config dir: `/opt/umamoe-resources-beta/config`
- Production confirmed-banner config dir: `/opt/umamoe-resources/config`

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
- `PURGE_ON_REFRESH`
- `CONFIRMED_BANNER_DATES_URL` if using a remote CSV instead of the mounted `/opt` file
- `CONFIRMED_BANNER_DATES_REFRESH_INTERVAL_SECONDS`
- `CLOUDFLARE_ZONE_ID`
- `CLOUDFLARE_API_TOKEN`
- `PUBLIC_BASE_URL`
- `RUST_LOG`

Recommended environment-specific values:

- Beta `env`: `DATABASE_URL=...`, `MASTER_REFRESH_INTERVAL_SECONDS=300`, `PURGE_ON_REFRESH=true`, `PUBLIC_BASE_URL=https://beta.uma.moe`
- Production `env`: `DATABASE_URL=...`, `MASTER_REFRESH_INTERVAL_SECONDS=300`, `PURGE_ON_REFRESH=true`, `PUBLIC_BASE_URL=https://uma.moe`

Remote host requirements:

- Docker installed
- A user with permission to run `docker`
- Enough temporary disk space under `/tmp/umamoe-resources-images` for the uploaded image archive during deployment
- Network access from the container to the Postgres instance referenced by `DATABASE_URL`

The workflow mounts the named Docker volume at `/data`, so refreshed `master.mdb` and generated resource artifacts persist across container replacements.
