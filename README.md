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
- Current Global game-version checks through `https://uma.moe/api/ver` before generation and while serving

The next good port is `campaign-extract.py`. Full `db-convert.py`, tierlist precompute, and statistics are larger domain ports and should be moved after the pipeline shape is stable.

## Serve Resources

```powershell
cargo run
```

By default, the server generates resources from `master.mdb` into `generated-data`, then serves them at `127.0.0.1:3000`.

The server always checks `https://uma.moe/api/ver` before startup and every 300 seconds while serving. When the current game version changes, it downloads and validates a fresh `master.mdb`, then regenerates resources. Version-check failures are errors; Postgres and `DATABASE_URL` are not required.

Equivalent explicit command:

```powershell
cargo run -- serve --master master.mdb --data-dir generated-data --bind 127.0.0.1:3000 --refresh-interval-seconds 300
```

If you want mutable CDN URLs purged automatically after a background refresh:

```powershell
cargo run -- serve --purge-on-refresh
```

To reuse existing resources when they match the latest master (the live version check still runs):

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
- `/resources/{version}/simulator_course_geometry.json.gz` -
  all current courses' 1,001-frame world transforms in one `courses` array
- `/resources/{version}/simulator_course_lanes.json.gz` - retained finish-line
  continuation lanes and their shared transform assets
- `/resources/masterdata/versions.json` - short CDN cache, every master release
  (`hash`, `short_hash`, `date`, `previous_hash`, `summary`), oldest first
- `/resources/masterdata/diffs/{short_hash}.json.gz` - one-year immutable CDN cache,
  the release's row-level changes against the previous release

The simulator on the separate host reads this API directly over the subnet.
Deployment publishes the existing internal container listener (`3204`) at
`192.168.100.2:3004` for production and `192.168.100.2:3104` for beta. The existing
public Nginx upstreams remain on `127.0.0.1:3004` and `127.0.0.1:3104` against
container port `3000`. Allow the simulator host (`192.168.100.1`) through the
Docker firewall path to the private resource ports. No Nginx route or service
authentication key is needed for the subnet connection.

`master.path` in the manifest points to
`/resources/simulator/{version}/master.mdb.gz`, served on both listeners with a
one-year immutable cache. The simulator and the frontend's Master Data browser load
it. `master.bytes` and `master.sha256` describe the uncompressed SQLite
database. Generation exports all resources from these exact database bytes and
includes their hash in the version. This avoids mixing a refreshed live database
with resources from an earlier export.

The manifest supports `ETag` and `If-None-Match` (`304` when unchanged). The
simulator checks every five minutes, downloads only changed simulator artifacts,
validates them and switches complete snapshots. Deploy this resource API update
before starting a simulator with an empty cache. Its required files are courses,
skills, course geometry, continuation lanes and the versioned master database.

Each export whose master hash is new appends a release to
`masterdata/versions.json` and writes `masterdata/diffs/{short_hash}.json.gz`:
`{ from_hash, to_hash, date, summary, tables: { name: { columns, added, removed,
modified: [{ key, before, after }] } } }`. Hashes are the master's SHA-256, the
same value as `master.sha256`. Rows are matched by each table's primary key
(`key` is the value, or an array for a composite key); rows of tables without a
primary key are only added or removed. Internal `sqlite_*` tables are ignored.
The diff is computed against the snapshot of the export being replaced; when
that snapshot is unavailable, the release is listed with a `null` summary.

`backfill-master-history --data-dir generated-data` rebuilds the history from every
resource version on `https://uma.moe/api/ver/history`, oldest first. Each master is
downloaded to `masterdata/backfill/`, diffed against the previous one and deleted,
so only `versions.json` and the diffs are kept. Versions whose master is unchanged
are listed once; a version the CDN no longer serves is skipped and the next diff
starts from the last available master. Releases recorded after the backfill's newest
master are kept. If the recorded history does not contain that master, the command
fails and leaves the history unchanged.

All resource JSON routes return precompressed bytes with `Content-Encoding: gzip` and `Content-Type: application/json; charset=utf-8`.

Simulator course geometry is bundled into one artifact with shared
`schema_version` and `master_version` fields and a `courses` array. Each entry
contains its `course_id`, track, distance, source asset, and transform columns;
clients select the desired entry by `course_id`. The bundled source data is
extracted from the client `CourseLaneAnim`
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

Official English announcements are archived before resource generation so temporary or later-deleted posts remain auditable. JP and English website news are read from Umapyoi's backed-up API, while English Twitter/X posts are archived from the official account through X's public endpoints. Each post keeps its raw API payload, first-seen timestamp, and every changed snapshot rather than deleting missing upstream records or overwriting corrections:

```powershell
cargo run -- sync-global-news
cargo run -- sync-global-social
```

Website news is stored in `src/global_data/official_news_archive.json`. The sync reads Umapyoi's `/api/v1/news/en/latest/32/<offset>` batches, including early posts that have disappeared from the live official site and the official `200xxx` X-campaign notices. Social posts are stored separately in `src/global_data/official_social_archive.json`. Social discovery reads the public official X profile, X's public syndication history and the Internet Archive's index of `x.com/umamusume_eng/status/` URLs captured in the last 90 days. Every source is best effort. X refuses its logged-out profile to datacenter IPs such as GitHub's runners, so in CI the archive index usually finds the new posts, while a local run also reaches the live profile. Whichever source lists an ID, the post is verified and its canonical text stored through X's own oEmbed endpoint, which must name exactly `@umamusume_eng` as the author. Unreadable sources and posts become warnings (annotations on the workflow run) instead of failing the sync. Records and changed snapshots are never removed from either archive when a source later deletes them. Website snapshots ignore Umapyoi's feed position (`row_number`), so a post only gets a new snapshot when its content changes.

Normal planner generation extracts exact free-Carat gifts and detailed login bonuses as default-enabled `global_news` or `global_social` rewards. Login bonuses with multiple explicitly listed availability periods are emitted as one bundled reward anchored to the first period, with the overall claim window in `available_at` and `available_until` and the daily amount breakdown retained as evidence; free-pull campaigns remain on their separate banner-allocation path. Matching one-day Global master rows are absorbed by their exact listed period start as soon as each row appears, so a partially released master schedule cannot duplicate a source-linked bundle. Correction and erratum notices are retained in the archive but never create a new income entry; the original distribution announcement remains the single source of that gift. A social post must explicitly say the gift was distributed; follow/repost contests, chance-to-win prizes, previews, paid-Carat sales, entry costs, and qualitative or player-dependent rewards are retained in the raw archives but not added as fixed planner income. Same-day, same-amount website and social announcements are counted once. A matching Global reward replaces the corresponding future JP-news estimate, while login bonuses already represented by Global master data are not counted twice.

`planner_rewards.json` also publishes `global_reward_comparison`. Speculative income is deliberately narrower than confirmed Global income: rewards with a JP counterpart, login bonuses, and recognizable recurring JP campaign families are excluded in full. The remaining website bucket contains only direct Global-only gifts or compensation, while official Global social giveaways form a separate bucket after website/social deduplication. The resource exposes both the retained social total and the count/Carat value of overlapping items removed. For long-term planning, `speculative_monthly_carats` is the arithmetic mean of those Global-only buckets across the latest six completed calendar months, including zero-reward months. The six inputs, their median, and the whole-archive mean are also published for auditability. The frontend starts that projection only after the latest confirmed reward date, preventing confirmed and speculative income from overlapping.

`.github/workflows/sync-umapyoi.yml` runs the Umapyoi JP news/gacha sync, Umapyoi English website-news sync, and official English Twitter sync four times daily, then commits the archives only when source content changed. The `umapyoi-en-news-published` repository-dispatch event can trigger the same job immediately from the Umapyoi publisher, with the schedule retained as a fallback. Every EN sync audits all archived posts with the production reward parser and fails before committing when a confirmed Carat login bonus or direct gift produces no planner reward. It can also be started manually with the full-refresh option for the JP source.

The Umapyoi publisher can request the immediate run with an authenticated `POST /repos/uma-moe/umamoe-resources/dispatches` GitHub API call and this body:

```json
{
  "event_type": "umapyoi-en-news-published",
  "client_payload": { "announce_id": 994 }
}
```

Timeline images are stored in the frontend rather than hotlinked at runtime. Generate a plain `banner_timeline.json`, then run `node scripts/sync_umapyoi_timeline_images.mjs --timeline-json <path> --frontend-root <umamoe-frontend>`. The Node/Sharp sync consumes the archive maintained by the Rust `sync-umapyoi` command; it is not a separate Python scraper. The original files below `assets/images/...` and `assets/timeline-images/...` are immutable production fallbacks: the sync never writes, deletes, or restores those paths. Official JP news-post artwork is stored separately below `assets/timeline-images/jp/`; official EN news-post artwork is stored below `assets/timeline-images/en/`. Each locale has a manifest mapping the unchanged timeline `image_path` to its overlay only after that overlay exists and opens successfully. The frontend resolves EN first, then JP, then the untouched legacy path, so removing either overlay directory is a complete rollback. Pass `--refresh-jp` to refresh only the managed JP overlay.

The JP resolver reads the bundled official Umapyoi news archive. It uses exact gacha IDs, stable asset identities, event family and release dates, title similarity, and semantic campaign phases to select official post artwork. EN matching audits the official public news API and uses the same stable identities and campaign semantics; regional campaign IDs are not assumed to match. For campaign posts it verifies and prefers the official `Thumbnail/banner_*` counterpart over a shallow `Header/header_*` image. Equal-date EN matches without a semantic identity are rejected as ambiguous. Historical official EN URLs that predate the public index remain recorded in `src/jp_data/english_mission_campaign_assets.json`. JP images are downscaled only when wider than 512 px and preserve the official aspect ratio without cropping, padding, or stretching. EN replacements use the selected JP overlay dimensions as their target, receive at most a 10% vertical adjustment, and are never cropped or padded. Source URLs and transform versions are recorded in `timeline-image-sync/japanese-image-sources.json` and `english-image-sources.json`, making subsequent runs idempotent and allowing a changed official URL to refresh only its overlay. Missing EN candidates are retried during the release window and retired seven days after release. The frontend `sync-timeline-images.yml` workflow runs at 13:15 JST, after the metadata sync, and commits only files reported by the image sync. Timeline events also expose the matching `umapyoi_url` so clients can link back to the archived source post.

The bundled mission-campaign catalogue is generated from `mission_data` rows with `mission_type = 4`. Refresh it from an installed JP master database with `scripts/sync_jp_mission_campaign_catalog.py --jp-master <jp-master.mdb> --frontend-root <umamoe-frontend>`. Point `--jp-image-source` (or `UMAMUSUME_JP_CAMPAIGN_ASSET_DIR`) at an AssetRipper directory containing `tex_campaign_mission_logo_<id>.png`; a same-aspect-ratio WebP is created only when a legacy fallback does not already exist, and existing production assets are never overwritten. Every newly available mission group is added to the catalogue automatically. The catalogue records the JP mission event ID, exact mission-condition/reward fingerprint, mission count, and JP display title. During normal resource generation, those fingerprints are compared with the current EN `master.mdb`; an EN event ID is only a guarded fallback when content matching is unavailable. This supplies the actual EN title and global start/end dates even when the JP image ID, JP mission event ID, and EN mission event ID are different. A semantic phase identity such as `fall-g1-missions-part-1` applies official news artwork to every matching mission row in the same release phase, so later races self-resolve without manual ranges.

Production does not ship the full JP `master.mdb`. Planner-relevant JP reward rows are stored in the compact, versioned `src/jp_data/planner_master_rewards.json` catalogue instead. To refresh it, generate planner JSON with `JP_MASTER_PATH` pointing to a current installed JP master, then run `node scripts/sync_jp_planner_reward_catalog.mjs <generated-planner_rewards.json> src/jp_data/planner_master_rewards.json`. Resource versions identify this input as `jp-catalog-<hash>`; a direct local JP database is identified as `jp-master-<hash>`. Setting `JP_MASTER_PATH` to a missing file is an error, preventing an intended direct-master build from silently falling back to the bundled catalogue.

Published JP banner odds are bundled separately in `src/jp_data/planner_gacha_rates.json`, so special banners retain exact rates when production has no JP database. Refresh this catalogue with Node 22+ using `node scripts/sync_jp_planner_gacha_catalog.mjs <jp-master.mdb> src/jp_data/planner_gacha_rates.json`. Matching Global master entries take precedence; JP entries must pass the same pickup-identity check as direct master data. The catalogue is included in the planner resource hash.

Campaign rewards that are not represented in the master database are stored separately in `src/jp_data/planner_campaign_rewards.json` with their source page and exact item rows. The generator links these records to the corresponding news-derived campaign and projects their availability onto the current Global timeline; exact Global master rewards replace matching projected rows after release.

Generation emits `character_names.json` from the bundled `src/character_names.json` mapping, preserving the existing JP/mapping and skin entries while overwriting known character `name` fields with global names from `master.mdb`.

`character.json`, `supports.json`, `support-cards-db.json`, and `skills.json` are generated from `master.mdb` and the bundled character-name mapping only. No external frontend data directory is read during generation.

Confirmed global timeline dates and visibility overrides for `banner_timeline.json` live in `src/jp_data/confirmed_global_banner_dates.csv`. To confirm a newly announced schedule entry, append one line:

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

To set a custom global end, add a second date after the start. It replaces the projected end, and `banner_duration_days` becomes the global start-to-end span. Official Global news and game-data end dates still take priority. Anniversary and hidden rows do not take an end date, paid rows ignore both dates, and a later row without an end date restores the projected end:

```csv
character,30104,2026-07-02,2026-07-12
```

To omit a JP banner that Global may skip, use `hidden` in place of the date:

```csv
character,2022_30130.webp,hidden
support,2022_30131.webp,hidden
```

Or prefix the row with `-`. The date is optional and ignored, so an existing row can be toggled off in place:

```csv
-character,30130
-support,30131,2026-07-02
```

Hidden rows are not emitted in `banner_timeline.json` and do not participate in banner-date calibration. Sources are applied in order, so a later dated row for the same type and gacha ID restores and confirms the banner if it is announced after all.

Banner rows accept a bare gacha id, image stem, `.png`, or `.webp`. Story and campaign rows use the image stem or filename. Campaign rows also accept the umapyoi news post id, unless another campaign already uses that number as its id or image stem. Champions Meeting and Legend Race rows use the sorted 0-based index, or the full key such as `champions_meeting_14`. Anniversary rows use the marker index, where `1` is the first half-anniversary. `YYYY-MM-DD` dates are emitted at `22:00 UTC`.

Months are inferred as complete banner schedules from confirmed character and support CSV entries. Unknown future scouts are shifted out of those months, with related future events following the same adjustment. Campaign, story, paid-banner and other event confirmations cannot close a month for scouts or shift their release sequence. Character and support confirmations retain their individual dates when a release is split; both contribute to the baseline calibration. Unconfirmed anniversary markers also respect the closed banner months.

`banner_timeline.json` includes character, support, paid, story, Champions Meeting, Legend Race, campaign, League of Heroes, Masters Challenge, Trainer Skills Test, Factor Research, Aim! The Strongest Team, Racing Carnival, Uma Sanpo, Holiday Celebration, and Training Scenario releases, plus anniversary marker metadata in the top-level `anniversaries` array. Champions Meetings after the bundled April 2024 dataset are extended from their start news posts; matching older posts provide local banner images without duplicating the bundled rows. News-discovered Pick 2 reruns, Twinkle Collection, scenario, and guaranteed/paid gachas are merged into the existing banner families. Every gacha row exposes the numeric `gacha_type` and a stable `gacha_type_name`: `standard_pool`, `makeup_debut`, `standard_pickup`, `guaranteed`, `group_select`, `twinkle_collection`, `pick_2`, `special_guaranteed`, `select_step_up`, or `stamp_sheet`. Pick 2 banners let the player choose two SSR support cards from the candidate pool. Character and support banners whose complete pickup set appeared on earlier dates receive the `rerun-banner` tag; same-day duplicate rows are not treated as reruns. Character and support banners define the baseline acceleration curve; news-derived special banners do not become confirmed calibration anchors. Other family confirmations apply isolated same-family residual corrections on top of that baseline.

Monthly Match is intentionally excluded because the JP history contains only one beta run. Generic campaigns, login bonuses, updates, maintenance, media announcements, and one-off challenges remain in `jp_news_events.json` instead of becoming timeline sections because their schedules overlap heavily or do not represent a repeatable content family. Repeatable, player-facing campaign families with their own banner and activity window, including Uma Sanpo and Holiday Celebration, are promoted to campaign timeline rows.

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

## Refresh From the Current Game Version

The long-running server always checks the live Global game version. One-shot commands use the same check:

```powershell
cargo run -- fetch-master --master master.mdb
cargo run -- refresh --master master.mdb --out generated-data --purge
```

`generate` also checks the live version before reading the master. Refresh regenerates resources when the master changes or the manifest is missing or has a different master hash. The legacy `--database-url` option remains accepted for compatibility.

## Cache Strategy

The frontend should load `/resources/manifest.json`, then request the versioned gzip artifact paths listed in it. Versioned paths can sit in Cloudflare for a year because the URL changes whenever `master.mdb.version` changes.

Only these mutable URLs need purging after regeneration:

- `/resources/manifest.json`
- `/resources/current/*.json.gz`

You can purge them manually from the latest manifest:

```powershell
cargo run -- purge --data-dir generated-data
```

## Publish Images

`assets-publish` uploads a folder of images to the asset bucket (Cloudflare R2 in production). The site routes its image paths to that bucket, so the key prefix is the URL path the files are served under:

```powershell
cargo run -- assets-publish --dir <web root>/game-assets --prefix game-assets --dry-run
```

- A file is uploaded only when the bucket has no object at its key or holds different bytes, so a second run over the same folder uploads nothing.
- Nothing is ever deleted from the bucket.
- `--dry-run` compares against the bucket and reports what would change.
- Replaced files are purged from Cloudflare on every `--purge-origin` (or `ASSETS_PURGE_ORIGINS`, comma separated). New files need no purge.

Files go to the `umamoe-assets` bucket at `ASSETS_S3_ENDPOINT`, signed with `ASSETS_S3_REGION` (default `auto`), `ASSETS_S3_ACCESS_KEY_ID` and `ASSETS_S3_SECRET_ACCESS_KEY`. Use an R2 token with Object Read & Write on that bucket only. In production the deploy writes these from GitHub secrets (see [Docker Deployment](#docker-deployment)).

## Game Art

One server builds the site's game images itself. When it starts and whenever the game version changes, it reads the game's file list, picks the files the site serves (character icons, card portraits, chibi sprites, support cards, skill icons, race banners, rank text, item icons; see `ART_FAMILIES` in `src/types/game_art.rs`, and status ranks, club ranks, stat, mood, run style, weather, season, ground and rarity icons cut from the game's atlases; see `ATLAS_FAMILIES`), the race tracks, a 3D uma and chibi per outfit, the race motions, the chibi and cut-in clips, the skill cut-in timelines, sets and props, the effects and the game's shaders, and builds only those that are new or changed: download, decrypt, convert to WebP (and models to glTF) with `scripts/game_art_convert.py`, upload to the same URL paths (skipping files the bucket already holds unchanged, each key once per batch, retrying writes R2 refuses for load), purge URLs whose image changed.

- It publishes for the version of the local `master.mdb`, which the refresh loop keeps current. Card portraits come from that master: the game names each portrait by the outfit it shows, and `card_rarity_data.race_dress_id` at the card's top rarity says which outfit it wears (cards that start below 3★ wear the shared uniform until then), so card 100102 shows outfit 100130 and card 100701 outfit 100701.
- An atlas is one bundle of named sprites drawing from a second texture bundle; both are downloaded together, sprites are cut at the size they are drawn, and the atlas is rebuilt when either bundle changes.
- Every race track prefab (`3d/env/race/…/pfb_…`, 773 today, 15 tracks plus gates and props) becomes a binary glTF at its game path, `game-assets/3d/env/race/race10001/race10001_0000/pfb_env_race10001_0000_002.glb`. The file list's dependency column says which bundles a prefab draws from (meshes in `3d/probuilderexport/`, materials and textures under `sourceresources/`); they are downloaded with it, except shaders and race animations, and the model is rebuilt when any of them changes. A full first build downloads about 460 MiB.
- Every texture a model draws is embedded in it as full-size WebP (`EXT_texture_webp`), so one `.glb` opens on its own, in Blender or three.js. The game fills its course materials (turf, dirt, base) at runtime with one season and weather variant from the part's `textures/` folder; those textures are published on their own at their game path and downloaded with the prefab, whose model embeds the first variant in those slots and lists every variant in the material's `extras.unity.variants` as links relative to the model's URL.
- Each racecourse is also published whole, as one `.glb` to drag into Blender: `game-assets/racetracks/tokyo.glb`, named after the course in master.mdb (`race10006.glb` when it has no name). It shows a sunny spring day: the track and turf (`0000`, `0001`), the trees (`6000`) and the stands (`7001`–`7003`), each in the variant whose season, weather and time digits fit `112` best (`0` fits any), under the shared sky `race00000_9000_112`. These parts are modelled where they stand. Rails, signs and flower beds (`1xxx`–`5xxx`) are placed along the track by the game at runtime, so they stay out, as do the cutscene copy (`cutt000`) and `7013`, which only marks where 3D spectators stand. The course model is rebuilt when any of its prefabs, or anything they draw, changes.
- `scripts/game_art_model.py` writes the glTF, with one or more prefabs side by side. Runtime textures fill the materials named after their part and kind (`tex_env_race10006_0000_112_turf00` fills `mtl_env_race10006_0000_002_turf00_lm00`). Unity is left-handed, so X is mirrored and triangles reverse; V is flipped. The game's shaders have no glTF equivalent: `_MainTex` becomes the base colour, materials named `alpha` are cut out, and every texture slot (baked light in `_LightTex`, read with the second UV set), number and colour stays in `extras.unity`, each slot naming its embedded texture. `extras.unity.shader` names the material's shader and its file among the published shaders (`{"name": "Gallop/3D/Chara/Toon/TSER", "file": "gallop/3d/chara/toon/tser.json"}`), and `extras.unity.keywords` lists the shader keywords the material turns on. Models load the Windows shader bundle to look the names up, without being rebuilt when it changes. Vertex colours stay in `_UNITY_COLOR`, since standard viewers would tint by `COLOR_0`. Inactive objects carry `extras.active: false`. Models pass the Khronos glTF validator without errors or warnings.
- Every outfit with a body of its own (`dress_data` rows with a `chara_id`) becomes one uma: `game-assets/umas/special-week/special-dreamer.glb`, named after the character and outfit in master.mdb (`text_data` categories 6 and 14; outfits of one character whose names read alike keep their outfit number). It joins the body `3d/chara/body/bdy1001_00`, the head `3d/chara/head/chr1001_00` (`head_sub_id`) and the tail (the outfit's own `tail_model_id`, else the character's `tail0001_00`; staff have none) on one skeleton in its rest pose, a T-pose, sharing bones by name. Heads and tails are modelled at the origin, so their meshes are moved onto the T-pose and every skin binds to the rest pose; a mesh whose bones were bound rotated or apart is posed onto it vertex by vertex, as the game skins it there. The uma looks as it does at rest in the game: meshes the game switches off until an expression needs them, such as tears, are left out, and the blush (`M_Cheek`, which the game multiplies over the skin while an uma blushes) is fully transparent, with `extras.unity.blend` set to `multiply`. The game moves faces with bones rather than blend shapes: each head's facial target (`ast_chr1001_00_facial_target`) holds, per eye, eyebrow and mouth expression, bone offsets from its `Base` pose, which the game adds up by weight. Every expression is baked into a blend shape (a glTF morph target, a shape key in Blender) on the meshes it moves, named after the game client's `TargetInfomation.Part` and `FaceEyeType`, `FaceEyebrowType` or `FaceMouthType`: `EyeR CloseA`, `EyebrowL IkariA`, `Mouth TalkA_A_L`; eyes and eyebrows have one per side, so a wink is `EyeL CloseA` alone. `scripts/game_art_face.py` bakes them. The game colours a tail per character at runtime (`ModelLoader.GetTailTextures`), so its `_diff`, `_shad_c`, `_base` and `_ctrl` textures fill the tail material's `_MainTex`, `_ToonMap`, `_TripleMaskMap` and `_OptionMaskMap`, the slots the body and head materials hold those kinds in. Every texture and value the game's toon shaders read is in `extras.unity`, for a viewer to rebuild the look. The root is named after the outfit (`Special Week: Special Dreamer`) and its extras record the character, the outfit, its running form and which race motion sets it uses (`chara_type` scene 1, cut 1 for runs and starts, cut 4 for finishes). It is published under its body's folder and rebuilt when any of its prefabs, anything they draw or a tail texture changes. `scripts/game_art_uma.py` writes umas and race motions.
- Every race clip (`3d/motion/racemain/body/…`) becomes its own `.glb` on the skeleton every uma shares (Special Week's racewear and the common tail), without meshes, so it fits any uma: `game-assets/uma-motions/race/bright/run-fast-standard.glb`. Names come from the game client's own enums: the personality set (`type01`–`04` Bright, Sporty, Mature, Cute; `typexx` Default; higher numbers keep theirs as `special-12`), the pace (`run01`–`05` Slow, Normal, Fast, Blocked, Tired), the running form (`_base`, `_pitch`, `_stride` as standard, pitch, stride), the gate phase and leading foot, and the finish placing and course direction. The title, which Blender names the armature and action after, is `Bright: Fast run, standard form`, at most 63 bytes, and the root's extras keep the game's clip name. Clips the game code does not name (`type00`) are left out. A clip is rebuilt when it or the skeleton changes.
- Every shader the game draws with is published from the Android build's `shader` bundle, whose programs are GLSL ES, the language of WebGL: one JSON file per shader, named after it (`Gallop/3D/Chara/Toon/TSER` at `game-assets/shaders/gallop/3d/chara/toon/tser.json`), and `game-assets/shaders/index.json` listing every shader's name and file. Two pairs of shaders share a name (`Gallop/3D/Chara/MultiplyCheek`, `Gallop/3D/MirrorAndShadow/BlendProjector`); materials tell them apart by object ID, which every platform's bundle shares, so their files add it (`multiplycheek-fa04f11e193430d1.json`). A file keeps the shader's properties (type, default, range), keywords and sub-shaders; each pass keeps its tags, render state (Unity's enums; a value a material property sets names that property, such as `_Cull`) and one program per keyword variant: its GLSL version and its vertex and fragment shaders, as indexes into the file's shared `sources`. Uniforms are not placed by location (`UNITY_SUPPORTS_UNIFORM_LOCATION 0`), which WebGL 2 does not support; every `300 es` program then compiles and links in WebGL 2, and the one `310 es` variant (procedural instancing) needs GLES 3.1. Shader files the bundle no longer holds are deleted. `scripts/game_art_shader.py` writes them. The shaders are built before anything else in a run.
- Every outfit with a chibi (`dress_data.have_mini`) becomes one chibi, named like its uma: `game-assets/chibis/special-week/special-dreamer.glb`. It joins the chibi body `3d/chara/mini/body/mbdy1001_00` (`body_type_sub`), the face every chibi shares (`3d/chara/mini/head/mchr0001_00/pfb_mchr0001_00_face0`, the only one the file list holds), the hair `3d/chara/mini/head/mchr1001_00/pfb_mchr1001_00_hair` (`head_sub_id`) and the uma's tail model as a chibi tail (`mtail0001_00`), the paths of the client's `MINI_BODY_SPECIAL_DRESS_MODEL_PATH`, `MINI_HAIR_MODEL_PATH` and `MINI_TAIL_MODEL_PATH`, on one skeleton at rest, like a uma. The face's materials are empty until the game fills them, so the model does: the skin and cheek by the character's skin colour (`chara_data.skin`; `MINI_FACE_TEXTURE_PATH`, `MINI_CHEEK_TEXTURE_PATH`: `tex_mchr0001_00_face0_1_diff`, `_cheek0`), the eyes, both eyebrows and the mouth with the head's sheets (`tex_mchr1001_00_eye_diff`, `_mayu_diff`, `_mouth_diff`), which hold every expression and show the default one. The cheek is the blush and hidden, as on a uma. The character's tail colour (`tex_mtail0001_00_1001_diff`) fills the tail material's `_MainTex`. The root's extras record the character and outfit. A texture the file list lacks is left out. It is published under its body's folder and rebuilt when any of its prefabs, anything they draw or one of its textures changes.
- Chibi and cut-in clips (`3d/motion/mini/…`, `3d/motion/cutin/…`, every `anm_` file) are served at their game path under their game name, since the game code names few of them. A body clip goes on its family's skeleton (Special Week's chibi body, hair and the common chibi tail; for cut-ins her racewear, head and the common tail), may move its root, and is rebuilt with the skeleton: `game-assets/3d/motion/cutin/card/body/crd100101_001/anm_cti_crd100101_001_01.glb`. A camera or position clip (`_cam`, `_pos`) moves a root, with a child per other object it moves named by the CRC-32 of that object's path. Curves of these clips that move no transform, such as a camera's field of view, are kept in the root's `extras.unity.curves`. The clips that move no bone, a face's or ears' clip (`_face`, `_ear`, which the game plays through their controllers) and a prop's or set's clip (whose model is published apart), become JSON with every curve (`.json`): per binding the path's CRC-32, Unity's type ID, the attribute (transform attributes by name, in glTF space; others by their hash, with `customType`) and the keys per component, plus the clip's length, frame rate, loop flag and events. A `_driven` clip, which drives a face by script, becomes a scripts dump as below.
- Every skill cut-in timeline (`cutt/cutin/skill/crd100101_001/crd100101_001`) becomes JSON at its game path: every object of the bundle with its fields, MonoBehaviours under their class as `script`, clips with their curves, meshes and textures by name only. A reference names its object's type, name and path ID (a string, too large for a JavaScript number) and, when it lies in another bundle, that bundle's game name. The bundles the timeline lists (its clips, effects and set) are downloaded only to name them, so a change to them does not rebuild it; the timeline waits until all of them are listed.
- Every effect (`3d/effect/…/pfb_…`) and cut-in set (`3d/env/cutin/…/pfb_…`) becomes a `.glb` at its game path, drawn like a track prefab, that also keeps each object's other components in its node's `extras.unity.components`, as `game_art_unity.py` reads them, in Unity's units: particle systems with only the modules they switch on (a curve or gradient keeps only what its `minMaxState` uses), particle and trail renderers, lights, animators and scripts. A reference there names its object's type and name and, where the model holds it, its index: `material`, `mesh` (meshes only particles draw are added unattached), `texture`, `node`, or `clip` in the document's `extras.unity.clips`. Every prop (`3d/chara/prop/<prop>/pfb_…`) becomes a `.glb` at its game path on its own bones.
- The game squeezes textures to powers of two (a 3:4 support card is stored as 2048×2048), so those outputs are resized to the size the site serves. Character icons are narrower than their 256×256 canvas and are centred on it. Chibi sprites are stored at their real size (most 256×256, wider for scenes with props) and keep it.
- What was published is recorded in the bucket (game file, source, checksum, converter revision, output paths), so a run with no changes downloads no game files, and a new card that wears an existing outfit still gets its portrait. The first run builds everything.
- The record is a folder, `_state/game-art/`, with one shard per kind of file: track scenery by what it shows (`racetracks.json` for the whole courses, `racetrack-parts.json`, `skyboxes.json`, `starting-gates.json`, `track-props.json`), umas, race motions and shaders apart (`umas.json`, `uma-motions.json`, `shaders.json`), as are chibis, their clips, cut-in clips and timelines (`chibis.json`, `chibi-motions.json`, `cutin-motions.json`, `cutins.json`) and effects, cut-in sets and props (`effects.json`, `cutin-sets.json`, `props.json`), every other file by its top-level game folder (`chara.json`, `supportcard.json`). `index.json` links every shard like the resources manifest links its artifacts: `{"name": "chara", "path": "/_state/game-art/chara.json", "files": 230}`, with the resource version and when it was written. Only the shards a batch touched are rewritten. A record in an earlier layout, or one whose records belong to another shard now, is rewritten once and its old files removed.
- Raising `CONVERTER_REVISION` rebuilds every picture once, `MODEL_CONVERTER_REVISION` every track and course model, `UMA_CONVERTER_REVISION` every uma and race motion, `CHIBI_CONVERTER_REVISION` every chibi and chibi clip, `CUTIN_CONVERTER_REVISION` every cut-in timeline, clip, set and prop, and `EFFECT_CONVERTER_REVISION` every effect. Revision 2 embedded the textures that earlier models kept beside them.
- Nothing the publisher put in the bucket stays behind once no record lists it. A rebuild that moves a file's output to another key (a renamed outfit, a new slug) deletes the old key, and so do the loose textures of earlier models. A record the catalog no longer builds is removed with what only it listed: a file no source lists any more or the catalog stopped serving, a racecourse whose track is gone, an outfit that left master.mdb or whose body is gone. Records waiting for a missing dependency stay. Keys are deleted before the record is saved, so an interrupted run deletes them again, and their URLs are purged. Objects it never recorded, such as the copied web root and the timeline banners, are never deleted. When more than 20 records, and more than 20% of them, look orphaned at once, none are removed and a warning is logged, since an incomplete file list is likelier than such a change.
- A file that does not convert is recorded and skipped until the game changes it; a file that fails to download is retried on the next check.
- Decrypted bundles and converted images live in a temporary folder that is removed after each run and at startup.
- Sources merge in precedence order. Global is the only source today; JP joins after it once there is a JP version check, filling in files Global does not have yet, and Global replaces them as soon as it lists them.
- One server publishes (`GAME_ART_PUBLISH=true`), so two never race on the bucket's record. The repository variable `GAME_ART_PUBLISHER` (`beta` or `production`, default `production`) picks that server; after changing it, deploy both so the old publisher stops. The publisher needs `GAME_ASSET_KEY` (the game's asset bundle key, hex) besides the bucket settings.

Check conversions without the bucket, using the Docker image (it has the converter):

```powershell
docker run --rm --env-file .env -v ${PWD}/art-sample:/out -v ${PWD}/art-master:/work umamoe-resources game-art --master /work/master.mdb --out-dir /out --sample 3
```

`--only game-assets/umas/special-week/` keeps the files served below a key prefix, to check one kind.

The converter's tests need UnityPy, so they run in the image too:

```powershell
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_uma test_game_art_face test_game_art_shader test_game_art_unity
```

`game-art --dry-run` lists what the next run would build and delete; `game-art` runs it once.

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

Docker also checks the current Global game version automatically; no Postgres connection is needed for master refresh.

## Docker Deployment

This repo now deploys the same way as the backend/statistics style image-archive workflows: build the image in GitHub Actions, upload it as an artifact, copy it to the host, `docker load` it there, and restart the target container with `docker run`.

What the workflow in [.github/workflows/docker-deploy.yml](.github/workflows/docker-deploy.yml) does on pushes to `main` or `master`:

- Builds the container image from [Dockerfile](Dockerfile)
- Saves the built image as a compressed artifact
- Deploys beta first, then production
- Copies the image archive to the remote host
- Recreates the target container with `docker run`
- Leaves your manual runtime env files alone
- Writes the asset bucket credentials to `assets.env` (mode 600) next to each manual `env` file

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

Asset bucket secrets. The R2 token only accepts the deploy host's IP, which beta and production share:

- `R2_ENDPOINT`: `https://<account id>.r2.cloudflarestorage.com`, without the bucket name
- `R2_ASSETS_ACCESS_KEY_ID`: the token's 32-character Access Key ID
- `R2_ASSETS_SECRET_ACCESS_KEY`: the token's 64-character Secret Access Key, not the token value

Set all three or none. Without them a deploy writes an empty `assets.env` and the container cannot publish images; deleting them removes the key from the host on the next deploy.

The repository variable `GAME_ASSET_KEY` (the game's asset bundle key as hex, UmaViewer's `ABKeyText`) goes into the same file. UmaViewer publishes it, so it is a variable rather than a secret, shared by beta and production. Production needs it to build game art.

Manual remote `env` files should contain the normal application env values only:

- `DATABASE_URL` (legacy compatibility; optional)
- `MASTER_REFRESH_INTERVAL_SECONDS`
- `PURGE_ON_REFRESH`
- `CONFIRMED_BANNER_DATES_URL` if using a remote CSV instead of the mounted `/opt` file
- `CONFIRMED_BANNER_DATES_REFRESH_INTERVAL_SECONDS`
- `CLOUDFLARE_ZONE_ID`
- `CLOUDFLARE_API_TOKEN`
- `PUBLIC_BASE_URL`
- `RUST_LOG`

Recommended environment-specific values:

- Beta `env`: `MASTER_REFRESH_INTERVAL_SECONDS=300`, `PURGE_ON_REFRESH=true`, `PUBLIC_BASE_URL=https://beta.uma.moe`
- Production `env`: `MASTER_REFRESH_INTERVAL_SECONDS=300`, `PURGE_ON_REFRESH=true`, `PUBLIC_BASE_URL=https://uma.moe`

Remote host requirements:

- Docker Engine 26 or newer, for the beta history mount's `volume-subpath`
- A user with permission to run `docker`
- Enough temporary disk space under `/tmp/umamoe-resources-images` for the uploaded image archive during deployment
- Network access from the container to `https://uma.moe/api/ver` and the Global game asset CDN

The workflow mounts the named Docker volume at `/data`, so refreshed `master.mdb` and generated resource artifacts persist across container replacements.

Beta also mounts production's `generated-data/masterdata` read-only over its own, so both
serve one master release history and only production records releases. Run
`backfill-master-history` in the production container; beta shows the result immediately.
While beta sees a new master before production does, it logs that it could not record the
release; production records it on its next refresh.
