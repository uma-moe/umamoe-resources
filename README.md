# umamoe-resources

Rust webserver for preparing semi-static Umamusume resource JSON from `master.mdb` and serving it as versioned `json.gz` streams.

## What Exists Now

- `factors.json` generation from `text_data` category `147`
- `race_program.json` generation from `single_mode_program`
- `character_banners.json`, `supports_banners.json`, and `paid_gacha_banners.json` generation from `gacha_data` + `gacha_available`
- `banner_timeline.json` generation from the store's JP banner/event history plus Global dates confirmed by `master.mdb` and the official Global news, corrected by the store's `confirmed_global_banner_dates.csv`
- `jp_news_events.json` from an incremental umapyoi.net archive of every JP news post and detailed gacha record, including event-family tags and discovered image/banner URLs
- `planner_rewards.json` Global gift rewards from durable, revision-preserving archives of the official English news API and `@umamusume_eng` social posts
- `affinity.json` generation from `succession_relation` + `succession_relation_member`
- `character_names.json` overlay generation from the store's mapping, with names refreshed from global `text_data` category `6`
- `character.json`, `supports.json`, `support-cards-db.json`, and `skills.json` DB-first generation from `master.mdb`
- Versioned gzip artifacts, each stored once under `generated-data/blobs/` and listed by version manifests
- `generated-data/manifest.json` for frontend discovery
- HTTP serving with CDN-friendly cache headers
- Cloudflare purge hook for mutable `manifest` and `current` URLs
- Current Global game-version checks through `https://uma.moe/api/ver` before generation and while serving

The next good port is `campaign-extract.py`. Full `db-convert.py`, tierlist precompute, and statistics are larger domain ports and should be moved after the pipeline shape is stable.

## Serve Resources

```powershell
cargo run
```

By default, the server generates resources from `master.mdb` and the [shared store](#shared-store) into `generated-data`, then serves them at `127.0.0.1:3000`. Nothing generates without the store: run `store pull` first locally.

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
https://uma.moe/resources/en/manifest.json
```

Then it should read the manifest and request the versioned gzip artifact paths listed there, such as `/resources/en/1.21.0-10005900/skills.json.gz`.

Each region's set has a path of its own: Global's is `/resources/en/`. The paths directly below `/resources/` from before the split (`/resources/manifest.json`, `/resources/current/*`, `/resources/{version}/*`) keep serving the Global set for older clients and services. The SQL endpoint, the planner, the master history and the simulator master stay where they are.

For Docker-to-Docker traffic, use the internal service hostname on the Docker network instead of the public domain. Example if the container service is named `umamoe-resources`:

```text
http://umamoe-resources:3000/resources
```

So the internal manifest URL would be:

```text
http://umamoe-resources:3000/resources/en/manifest.json
```

Recommended frontend flow:

- Load `/resources/en/manifest.json`
- Read the current `version`
- Request only the versioned artifact paths from the manifest
- Treat `/resources/en/current/*` and `/resources/en/manifest.json` as mutable, short-cache endpoints
- Treat `/resources/en/{version}/*` as immutable CDN-friendly assets

The server also accepts legacy `.json` URLs, but the manifest advertises `.json.gz` URLs and those should be treated as canonical.

Useful routes:

- `/resources/` - default resource API entry point; returns the latest Global manifest
- `/resources/en/manifest.json` - short CDN cache, points the frontend at the current Global version
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
clients select the desired entry by `course_id`. The source data, the store's
`simulator_course_geometry.json.gz`, is extracted from the client `CourseLaneAnim`
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
cargo run --release -- store pull
cargo run --release -- generate --master master.mdb --out generated-data
```

`generate` reads the store copy in `generated-data/store/` and fails, naming the missing files, when it is incomplete.

## Synchronize JP News And Gacha Metadata

The umapyoi API exposes gachas separately, but most other JP event families only exist as news posts. This command fetches the complete news index, downloads posts not already present in the local archive, refreshes detailed gacha records, classifies known event families, and retains the raw response so newly discovered fields are not lost:

```powershell
cargo run -- sync-umapyoi
```

The command writes the local store copy, `generated-data/store/umapyoi_archive.json`; servers read the copy in the [shared store](#shared-store), which the publisher keeps current. The archive is emitted as `jp_news_events.json` during normal resource generation. Each news item contains its source page, normalized title/date fields when present, event tags such as `story_event`, `champions_meeting`, `training_scenario`, `league_of_heroes`, `legend_race`, `campaign`, and `gacha`, plus every image URL with its original JSON field path and a `likely_banner` hint. URLs named `gacha_banner_<id>` are emitted as structured `gacha_banners`, allowing news-only JP pickup banners to expand automatically. Select Pickup support banners are marked as master gacha type `12`, tagged as reruns, and enriched with every support-card candidate named by the news post. Detailed gacha entries embed normalized support-card pickup metadata.

The top-level `analysis` object is regenerated from the saved raw payloads. It reports classification counts, date coverage, image/banner coverage, discovered gacha-banner IDs, detailed API gacha count, and merged support-card count. Run `cargo run -- sync-umapyoi --offline` to rebuild normalized fields and analysis without accessing umapyoi.net.

The importer defaults to one request every 550 ms (about 1.8 requests/second). This stays below umapyoi's tightest sustained limits of 7,200 requests/hour and 172,800/day, as well as the 10/second and 500/minute burst limits. The publishing server uses a gentler one request/second because sustained faster crawls have made the upstream service unstable. HTTP 429 and 5xx responses are retried with exponential backoff. Use `--request-interval-ms` to go slower; values below 500 ms are rejected. Routine runs only request the three index endpoints plus details for IDs that are not already archived. Image binaries are never downloaded from umapyoi.net; the importer only preserves image URLs embedded in news/API JSON.

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

The commands write website news to the local store copy, `generated-data/store/official_news_archive.json`; servers read the copy in the [shared store](#shared-store), which the publisher keeps current. The sync reads Umapyoi's `/api/v1/news/en/latest/32/<offset>` batches, including early posts that have disappeared from the live official site and the official `200xxx` X-campaign notices. Social posts are kept separately, in `official_social_archive.json`. Social discovery reads the public official X profile, X's public syndication history and the Internet Archive's index of `x.com/umamusume_eng/status/` URLs captured in the last 90 days. Every source is best effort. X refuses its logged-out profile to datacenter IPs, so on a server the archive index usually finds the new posts, while a local run also reaches the live profile. Whichever source lists an ID, the post is verified and its canonical text stored through X's own oEmbed endpoint, which must name exactly `@umamusume_eng` as the author. Unreadable sources and posts become logged warnings instead of failing the sync. Records and changed snapshots are never removed from either archive when a source later deletes them. Website snapshots ignore Umapyoi's feed position (`row_number`), so a post only gets a new snapshot when its content changes.

Normal planner generation extracts exact free-Carat gifts and detailed login bonuses as default-enabled `global_news` or `global_social` rewards. Login bonuses with multiple explicitly listed availability periods are emitted as one bundled reward anchored to the first period, with the overall claim window in `available_at` and `available_until` and the daily amount breakdown retained as evidence; free-pull campaigns remain on their separate banner-allocation path. Matching one-day Global master rows are absorbed by their exact listed period start as soon as each row appears, so a partially released master schedule cannot duplicate a source-linked bundle. Correction and erratum notices are retained in the archive but never create a new income entry; the original distribution announcement remains the single source of that gift. A social post must explicitly say the gift was distributed; follow/repost contests, chance-to-win prizes, previews, paid-Carat sales, entry costs, and qualitative or player-dependent rewards are retained in the raw archives but not added as fixed planner income. Same-day, same-amount website and social announcements are counted once. A matching Global reward replaces the corresponding future JP-news estimate, while login bonuses already represented by Global master data are not counted twice.

`planner_rewards.json` also publishes `global_reward_comparison`. Speculative income is deliberately narrower than confirmed Global income: rewards with a JP counterpart, login bonuses, and recognizable recurring JP campaign families are excluded in full. The remaining website bucket contains only direct Global-only gifts or compensation, while official Global social giveaways form a separate bucket after website/social deduplication. The resource exposes both the retained social total and the count/Carat value of overlapping items removed. For long-term planning, `speculative_monthly_carats` is the arithmetic mean of those Global-only buckets across the latest six completed calendar months, including zero-reward months. The six inputs, their median, and the whole-archive mean are also published for auditability. The frontend starts that projection only after the latest confirmed reward date, preventing confirmed and speculative income from overlapping.

The publishing server runs these syncs on its own; see [Shared Store](#shared-store).

Timeline images are stored in the frontend rather than hotlinked at runtime. Generate a plain `banner_timeline.json`, then run `node scripts/sync_umapyoi_timeline_images.mjs --timeline-json <path> --frontend-root <umamoe-frontend>`. The Node/Sharp sync consumes the archive in the local store copy (`generated-data/store/`); it is not a separate Python scraper. The original files below `assets/images/...` and `assets/timeline-images/...` are immutable production fallbacks: the sync never writes, deletes, or restores those paths. Official JP news-post artwork is stored separately below `assets/timeline-images/jp/`; official EN news-post artwork is stored below `assets/timeline-images/en/`. Each locale has a manifest mapping the unchanged timeline `image_path` to its overlay only after that overlay exists and opens successfully. The frontend resolves EN first, then JP, then the untouched legacy path, so removing either overlay directory is a complete rollback. Pass `--refresh-jp` to refresh only the managed JP overlay.

The JP resolver reads the official Umapyoi news archive. It uses exact gacha IDs, stable asset identities, event family and release dates, title similarity, and semantic campaign phases to select official post artwork. EN matching audits the official public news API and uses the same stable identities and campaign semantics; regional campaign IDs are not assumed to match. For campaign posts it verifies and prefers the official `Thumbnail/banner_*` counterpart over a shallow `Header/header_*` image. Equal-date EN matches without a semantic identity are rejected as ambiguous. Historical official EN URLs that predate the public index remain recorded in the store's `english_mission_campaign_assets.json`. JP images are downscaled only when wider than 512 px and preserve the official aspect ratio without cropping, padding, or stretching. EN replacements use the selected JP overlay dimensions as their target, receive at most a 10% vertical adjustment, and are never cropped or padded. Source URLs and transform versions are recorded in `timeline-image-sync/japanese-image-sources.json` and `english-image-sources.json`, making subsequent runs idempotent and allowing a changed official URL to refresh only its overlay. Missing EN candidates are retried during the release window and retired seven days after release. The frontend `sync-timeline-images.yml` workflow runs at 13:15 JST, after the metadata sync, and commits only files reported by the image sync. Timeline events also expose the matching `umapyoi_url` so clients can link back to the archived source post.

The mission-campaign catalogue (the store's `timeline_campaigns.json` and `planner_mission_rewards.json`) is generated from `mission_data` rows with `mission_type = 4`. Refresh it from an installed JP master database with `scripts/sync_jp_mission_campaign_catalog.py --jp-master <jp-master.mdb> --frontend-root <umamoe-frontend>`, which updates both files in the local store copy; upload them with `store put`. Point `--jp-image-source` (or `UMAMUSUME_JP_CAMPAIGN_ASSET_DIR`) at an AssetRipper directory containing `tex_campaign_mission_logo_<id>.png`; a same-aspect-ratio WebP is created only when a legacy fallback does not already exist, and existing production assets are never overwritten. Every newly available mission group is added to the catalogue automatically. The catalogue records the JP mission event ID, exact mission-condition/reward fingerprint, mission count, and JP display title. During normal resource generation, those fingerprints are compared with the current EN `master.mdb`; an EN event ID is only a guarded fallback when content matching is unavailable. This supplies the actual EN title and global start/end dates even when the JP image ID, JP mission event ID, and EN mission event ID are different. A semantic phase identity such as `fall-g1-missions-part-1` applies official news artwork to every matching mission row in the same release phase, so later races self-resolve without manual ranges.

Production does not ship the full JP `master.mdb`. Planner-relevant JP reward rows are kept in the compact, versioned `planner_master_rewards.json` catalogue in the store instead. To refresh it, generate planner JSON with `JP_MASTER_PATH` pointing to a current installed JP master, then run `node scripts/sync_jp_planner_reward_catalog.mjs <generated-planner_rewards.json> generated-data/store/planner_master_rewards.json` and upload the result with `store put`. The manifest's `inputs` record this input as `jp_catalog`; a direct local JP database is recorded as `jp_master`. Setting `JP_MASTER_PATH` to a missing file is an error, preventing an intended direct-master build from silently falling back to the catalogue.

Published JP banner odds are kept separately in the store's `planner_gacha_rates.json`, so special banners retain exact rates when production has no JP database. Refresh this catalogue with Node 22+ using `node scripts/sync_jp_planner_gacha_catalog.mjs <jp-master.mdb> generated-data/store/planner_gacha_rates.json` and upload the result with `store put`. Matching Global master entries take precedence; JP entries must pass the same pickup-identity check as direct master data. The catalogue is included in the planner resource hash.

Campaign rewards that are not represented in the master database are stored separately in `src/jp_data/planner_campaign_rewards.json` with their source page and exact item rows. The generator links these records to the corresponding news-derived campaign and projects their availability onto the current Global timeline; exact Global master rewards replace matching projected rows after release.

Generation emits `character_names.json` from the store's `character_names.json` mapping, preserving the existing JP/mapping and skin entries while overwriting known character `name` fields with global names from `master.mdb`.

`character.json`, `supports.json`, `support-cards-db.json`, and `skills.json` are generated from `master.mdb` and the store's character-name mapping only. No external frontend data directory is read during generation.

Both resource sets also name every character and card in English and Japanese, whatever the set's own language: `character_names.json` entries carry `name_en` and `name_jp`; `character.json`, `supports.json` and `support-cards-db.json` entries carry `name_en`, `name_jp`, `title_en` and `title_jp` (the character, and the outfit or card title without brackets); `banner_timeline.json` has a `card_names` map with the same four fields for every card an event picks up. A field is `null` when no source names it. English names come from Global's master, then the store's hand-written `name_overrides.json`, then its `character_names.json`, then its `name_translations.json`. Japanese names come from JP's master: the JP set's own, and for the Global set the JP master the server keeps beside the Global one (`jp-master.mdb`, kept current while `JP_RESOURCES` or `JP_GAME_ART_PUBLISH` is on). The Global timeline also takes JP's banners past the curated banner lists from that master. Its hash is part of the Global version, so a new JP master regenerates the Global set; without it the Global set has no Japanese names and those banners come from the news archive.

The publisher asks Claude for English names of what only JP has released and keeps them in the store's `name_translations.json`, so every server names a card the same way and only the publisher calls the model. It needs `ANTHROPIC_API_KEY`; without it those names stay untranslated. It translates on its first sync and whenever a master changed, and otherwise retries names the model left out at most every 6 hours. `translate-names` does the same for a local copy (`generated-data/store/name_translations.json` by default). To correct a translation, add the name to the store's `name_overrides.json` under its kind (`uma_name`, `outfit_title` or `support_title`) and id, and upload it with `store put name_overrides.json <path>`.

The Global master confirms most of the timeline's Global dates by id: character, support and paid gachas, and Champions Meetings, story events, Legend Races and Uma Sanpo campaigns, each with its end. A Global schedule keeps its JP id, and the JP master the server keeps beside the Global one ties it to the JP event the timeline lists: the Champions Meeting whose race starts inside the JP schedule of that id, the story event or Uma Sanpo campaign that started on the JP schedule's day, and the Legend Race spanning every group whose JP start falls inside it. Only schedules starting on or after the Global launch count, as the Global master keeps JP's dates for what Global has not run. The official Global news confirms the events it announces. Without the JP master, as on a first start before the JP refresh has fetched it, only the gachas are confirmed from the master.

Corrections to these dates, and dates no source gives, live in the store's `confirmed_global_banner_dates.csv`, applied after every other source. Edit it with `store put confirmed_global_banner_dates.csv <path>`, which uploads it only once every row parses, or in the R2 dashboard (the `store/` folder of the store's bucket). Every server takes it on its next refresh check (`MASTER_REFRESH_INTERVAL_SECONDS`), regenerates, and purges the mutable Cloudflare URLs where `PURGE_ON_REFRESH` is set, as on both deployed servers. A row confirms an event or replaces its date:

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

To set a custom global end, add a second date after the start. It replaces the master's or the projected end, and `banner_duration_days` becomes the global start-to-end span. End dates from the official Global news and the master's mission campaigns still take priority. Anniversary and hidden rows do not take an end date, paid rows ignore both dates, and a row without an end date leaves the event to its projected end, even where the master gives one:

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

`banner_timeline.json` includes character, support, paid, story, Champions Meeting, Legend Race, campaign, League of Heroes, Masters Challenge, Trainer Skills Test, Factor Research, Aim! The Strongest Team, Racing Carnival, Uma Sanpo, Holiday Celebration, and Training Scenario releases, plus anniversary marker metadata in the top-level `anniversaries` array. Champions Meetings after the curated April 2024 dataset are extended from their start news posts; matching older posts provide local banner images without duplicating the curated rows. News-discovered Pick 2 reruns, Twinkle Collection, scenario, and guaranteed/paid gachas are merged into the existing banner families. Every gacha row exposes the numeric `gacha_type` and a stable `gacha_type_name`: `standard_pool`, `makeup_debut`, `standard_pickup`, `guaranteed`, `group_select`, `twinkle_collection`, `pick_2`, `special_guaranteed`, `select_step_up`, or `stamp_sheet`. Pick 2 banners let the player choose two SSR support cards from the candidate pool. Character and support banners whose complete pickup set appeared on earlier dates receive the `rerun-banner` tag; same-day duplicate rows are not treated as reruns. Character and support banners define the baseline acceleration curve; news-derived special banners do not become confirmed calibration anchors. Other family confirmations apply isolated same-family residual corrections on top of that baseline.

Monthly Match is intentionally excluded because the JP history contains only one beta run. Generic campaigns, login bonuses, updates, maintenance, media announcements, and one-off challenges remain in `jp_news_events.json` instead of becoming timeline sections because their schedules overlap heavily or do not represent a repeatable content family. Repeatable, player-facing campaign families with their own banner and activity window, including Uma Sanpo and Holiday Celebration, are promoted to campaign timeline rows.

The resource also includes prediction likelihood metadata. The calculation block reports observed character-banner monthly count, adjacent character-banner gap, weekday, and day-of-month frequencies; unconfirmed events include a compact `prediction.calendar_likelihood` score showing how common that predicted month shape, spacing, and release date are compared with confirmed schedules.

The corrections file's hash is part of the generated resource version, so clients that read `/resources/manifest.json` will see a new versioned `banner_timeline.json.gz` path after an edit.

For debugging, also write plain JSON into the version folder:

```powershell
cargo run -- generate --master master.mdb --out generated-data --write-json
```

Output layout:

```text
generated-data/
	manifest.json                  current Global manifest
	<version>/
		manifest.json              one manifest per version, nothing else (plus JSON with --write-json)
	planner/                       the planner set: manifest.json and one folder per version
	jp/                            the JP set: manifest.json and one folder per version
	blobs/
		<sha256>.json.gz           every artifact of every set and version, once
	masters/
		<sha256>.mdb               every Global master a version was generated from, once
		<sha256>.mdb.gz
	masterdata/                    master release history
	store/                         shared store copy
```

A version is named `<master marker>-<hash>`: the hash is the first 12 hex digits of a SHA-256 over every input the set is generated from (the master, the confirmed dates, the news archive and the store's data files; for JP its master, the Global master and the names), so a change to any of them names a new version. The manifest lists each input's own hash under `inputs`.

An artifact is stored under the SHA-256 of its JSON, the `sha256` its manifest lists, so a file that did not change between versions is written once. URLs stay `/<set>/<version>/<file>.gz`; the server finds the file through the version's manifest. Versions written before this layout keep their files inside their folder and are still served from there.

After each generation the newest 10 versions of the set stay, the current one always among them, and older version folders are deleted. Then every file in `blobs/` and `masters/` that no remaining manifest of any set lists is deleted once it is at least an hour old, so a file another process has just written for a manifest it has not written yet stays. Only folders that hold a `manifest.json` count as versions, never `blobs`, `masters`, `masterdata`, `store`, `planner` or `jp`; a manifest that cannot be read stops the deletion, and a failed cleanup never fails the generation.

## Shared Store

The data files do not travel through git. They live in a bucket; every server reads them from there and generates from the copy it last loaded. The store holds two kinds of file:

- Collected files, which one server, the publisher (`GAME_ART_PUBLISH=true`), fetches from their sources and uploads. Its copy is their source of truth. They are `umapyoi_archive.json` (JP news, gachas and support cards from Umapyoi), `official_news_archive.json` (official English website news), `official_social_archive.json` (official English X posts) and `name_translations.json` (English names of what only JP has released).
- Provided files, static data a maintainer uploads with `store put`. The bucket's copy is their source of truth for every server, the publisher included. They are `character_names.json`, `support-cards-db.json`, `timeline_character_banners.json`, `timeline_support_banners.json`, `timeline_paid_banners.json`, `timeline_story_events.json`, `timeline_champions_meeting.json`, `timeline_legend_races.json`, `timeline_campaigns.json`, `planner_gacha_rates.json`, `planner_master_rewards.json`, `planner_mission_rewards.json`, `english_mission_campaign_assets.json`, `simulator_course_geometry.json.gz` and `simulator_course_lanes.json.gz` (stored gzipped as they are), and `course_event_params_jp.json` and `course_event_params_global.json` (each course's event table, keyed by course id), `race_params_global.json` (the decoded Global race parameter asset, which only tests read), `confirmed_global_banner_dates.csv` (the maintainers' corrections to the timeline's Global dates) and `name_overrides.json` (hand-written English names).

- The publisher runs the three news syncs at start and every 30 minutes (`STORE_SYNC_INTERVAL_SECONDS`, at least 300), then uploads each collected file that changed. It never uploads a provided file. A source that fails is tried again on the next round; the other files do not wait for it. A website-news sync is kept only when the production reward parser reads every confirmed Carat login bonus and direct gift in it, so a post it cannot read never reaches generation.
- Every server keeps its copy in `<data-dir>/store/`. On every refresh check each server, the publisher included, fetches the provided files that changed in the bucket, and a reader also the collected ones. A changed file changes the resource versions, so the sets regenerate. A server that cannot reach the store keeps generating from its copy.
- The publisher starts a collected file the bucket does not hold from nothing and builds it from its source. A provided file the bucket does not hold is an error until a maintainer uploads it with `store put`.
- Nothing generates without a complete store copy: generation fails and names the missing files. `name_translations.json`, `name_overrides.json` and `race_params_global.json` are the exceptions: the translations exist only once something was translated (a publisher without `ANTHROPIC_API_KEY` never writes them), without overrides no name is overridden, and only tests read the race parameters.
- A server whose first generation fails, for example because the store is out of reach, keeps serving the resources already in its data directory and retries on every refresh check. With none there, a reader exits with that error, while the publisher keeps running: its sync collects the missing files and the next refresh check generates. Until then resource requests fail with an error status.
- The store lives in a `store/` folder and is reached with the asset token. Each can be replaced on its own: `STORE_S3_BUCKET` names the bucket that holds the `store/` folder (default: the asset bucket), such as a private one, and `STORE_S3_ACCESS_KEY_ID` with `STORE_S3_SECRET_ACCESS_KEY` a token of its own (both or neither). The endpoint stays `ASSETS_S3_ENDPOINT`, so the store's bucket must be in the same account and jurisdiction as the asset bucket. The publisher's token needs Object Read & Write on the store's bucket; a reading server only reads, so Object Read only is enough for it.
- Git holds code only. Every data file the sets are generated from, hand-written or game-derived, is in the store.
- `store pull [--data-dir generated-data]` fetches every file the bucket holds into `<data-dir>/store/`, and `store put <file-name> <path>` uploads a local file as a provided file after checking it parses. Both need the bucket's credentials. The maintainer scripts write to the local store copy in `generated-data/store/`; upload what they regenerate with `store put`, for example `cargo run --release -- store put planner_gacha_rates.json generated-data/store/planner_gacha_rates.json`.
- To work locally, fetch the store with `store pull` (it needs the bucket's credentials in `.env`), then run `generate` or `serve`.
- The tests that read store data read `generated-data/store/` too. Without a complete copy they pass early and print `skipped: the local store copy lacks <files>`.
- `generate`, `refresh` and `timeline-art` read the store copy in their data directory, fetching it first when they have the bucket's credentials.

## Refresh From the Current Game Version

The long-running server always checks the live Global game version. One-shot commands use the same check:

```powershell
cargo run -- fetch-master --master master.mdb
cargo run -- refresh --master master.mdb --out generated-data --purge
```

`generate` also checks the live version before reading the master. Refresh regenerates resources when the master changes or the manifest is missing or has a different master hash. The legacy `--database-url` option remains accepted for compatibility.

## Cache Strategy

The frontend should load `/resources/en/manifest.json`, then request the versioned gzip artifact paths listed in it. Versioned paths can sit in Cloudflare for a year because the URL changes whenever `master.mdb.version` changes.

Only these mutable URLs need purging after regeneration, at both the region path and the path from before the split:

- `/resources/en/manifest.json` and `/resources/manifest.json`
- `/resources/en/current/*.json.gz` and `/resources/current/*.json.gz`

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
- Replaced files are purged from Cloudflare on every `--purge-origin` (or `ASSETS_PURGE_ORIGINS`, comma separated; the deploy sets `https://assets.uma.moe`, where the bucket is served). New files need no purge. A failed purge is logged and the publish goes on, since cached copies expire on their own.

Files go to the `umamoe-assets` bucket at `ASSETS_S3_ENDPOINT`, signed with `ASSETS_S3_REGION` (default `auto`), `ASSETS_S3_ACCESS_KEY_ID` and `ASSETS_S3_SECRET_ACCESS_KEY`. Use an R2 token with Object Read & Write on that bucket only. In production the deploy writes these from GitHub secrets (see [Docker Deployment](#docker-deployment)).

## Game Art

One server builds the site's game images itself. When it starts and whenever the game version changes, it reads the game's file list, picks the files the site serves (character icons, card portraits, chibi sprites, support cards, skill icons, race banners, rank text, item icons; see `ART_FAMILIES` in `src/types/game_art.rs`, and status ranks, club ranks, stat, mood, run style, weather, season, ground and rarity icons cut from the game's atlases; see `ATLAS_FAMILIES`), the race tracks, a 3D uma and chibi per outfit, the race motions, the chibi and cut-in clips, the skill cut-in timelines, sets and props, the effects and the game's shaders, and builds only those that are new or changed: download, decrypt, convert to WebP (and models to glTF) with `scripts/game_art_convert.py`, upload to the same URL paths (skipping files the bucket already holds unchanged, each key once per batch, retrying writes R2 refuses for load), purge URLs whose image changed. Files are built in batches of eight per converter process, with four processes converting at once (`GAME_ART_CONVERTERS`, up to eight; the deploy passes the repository variable of that name), at the lowest CPU priority, since the server's cores also serve the site; while one batch converts, the next downloads and the previous one uploads, so the network and the CPU stay busy. A run logs how many files it will build; each batch then logs the files done of that total, the share done, the minutes left at the pace so far, and how long its download, conversion, upload, records and directory documents took.

Each region publishes a tree of its own in a bucket folder named for it: Global's is `en/`, served at `https://assets.uma.moe/en/art/...`. Purges add the folder to every `ASSETS_PURGE_ORIGINS` origin. Global's tree was first published at the bucket's root; on its first start the server copies that tree into `en/` inside the bucket (art first, then the publishing records) before any publisher runs, so nothing is rebuilt. The root copy stays until it is cleared by hand.

Inside its folder, everything is published below `art/`, in one tree built for loading:

```
art/
  index.json                 links to each section's index
  umas/index.json            every uma: site id, game outfit, name, character
  umas/100101/               an outfit, in the folder of the card that wears it (the site's id);
                             an outfit no card wears is <character>-<outfit>, 1066-106601,
                             as NPC race rows name it
    index.json               every file the uma loads, see below
    model.glb  chibi.glb  portrait.webp  stand.webp  icon.webp  dress.webp  sprites/  cutin/  …
  characters/1001/           what all of a character's umas share
    index.json               name, icon, its umas and support cards
    icon.webp  cutin/  chibi-motions/<role>/  effects/<set>/  sprites/
  motions/race/<set>/        race clips, run-fast-standard.glb
  motions/chibi/<scene>/<set>/<role>/   chibi clips any chibi plays
  cutins/<timeline>/         cut-ins no card or known character owns; cutins/sets/ for shared sets
  effects/<game folders>/    effects, without the pfb_eff_ prefix
  props/  shaders/  racecourses/<course>/{model.glb,parts/,textures/}  racecourses/shared/
  support-cards/<id>/{card,half}.webp  skills/icons/  races/banners/  items/  ui/<family>/
  timeline/                  the timeline's banners, see Timeline Banners below
```

Every folder an uma, character, racecourse or library lives in has an `index.json`. Every path in them, in timelines and in models is a link from the tree's root, `/art/umas/100101/model.glb`, so the tree's origin (`https://assets.uma.moe/en` for Global) plus the link is the file's URL. An uma's document is the one file to read to load it:

```json
{
  "schema": 1, "id": 100101, "name": "Special Dreamer", "outfit": 100101,
  "character": {"id": 1001, "name": "Special Week", "icon": "/art/characters/1001/icon.webp", "index": "/art/characters/1001/index.json"},
  "pictures": {"portrait": "/art/umas/100101/portrait.webp", "stand": "/art/umas/100101/stand.webp", "icon": "/art/umas/100101/icon.webp", "…": "…"},
  "model": {"file": "/art/umas/100101/model.glb", "shaders": ["/art/shaders/gallop/3d/chara/toon/tser.json"],
            "motions": {"run": ["/art/motions/race/bright/run-fast-standard.glb"], "gate": ["…"], "finish": ["…"]}},
  "chibi": {"file": "/art/umas/100101/chibi.glb", "sprites": ["…"], "motions": {"body": ["/art/characters/1001/chibi-motions/body/00_idle01_loop.glb"]}},
  "cutins": [{"category": "unique-rare", "timeline": "/art/umas/100101/cutin/timeline.json",
              "clips": {"camera": ["/art/umas/100101/cutin/camera/01_cam.glb"]}, "sets": ["…"],
              "effects": ["/art/characters/1001/effects/00/01.glb"], "shaders": ["…"]}]
}
```

- `id` is the site's id of a playable outfit, its card; `outfit` is the game's, which NPC race rows and outfit changes give. A character's document lists each of its umas with both.
- `model.motions` are the race clips this uma plays: runs of its running form and gate starts from its run set, finish reactions from its finish set (`chara_type` scene 1, cut 1 and 4; `chara_data.race_running_type`). `chibi.motions` are the character's own chibi clips by role.
- `cutins` are the outfit's card cut-in (`unique-rare`) and the character's cut-in (`unique`), each with its clips by role, the sets, effects and props its timeline plays (from the file list's dependencies) and the shaders those use.
- Shaders are the ones the model's materials name, which the converter reports per model. A document lists only files that converted, and every document starts with `"schema": 1`, raised when its shape changes.
- Library folders (`motions`, `effects`, `props`, `cutins`, `skills`, `races`, `items`, `ui`, `racecourses/shared`) are listed folder by folder for browsing; nothing needs them to load an uma.
- Documents are rewritten at most every ten minutes during a run, and at its end. `_state/game-art/_directory.json` keeps each document's MD5, so only changed documents are uploaded and documents no longer written are deleted. Documents are cached for five minutes, files for a day.
- A file keeps the part of its game name that tells it apart from its siblings. When two files would still share a key, both keep their full game names and the run logs the clash.

- It publishes for the version of the local `master.mdb`, which the refresh loop keeps current. Card portraits come from that master: the stand art (`chara_stand`) is published at 128 px as `portrait.webp` and at its full 512 px as `stand.webp`; the game names each by the outfit it shows, and `card_rarity_data.race_dress_id` at the card's top rarity says which outfit it wears (cards that start below 3★ wear the shared uniform until then), so card 100102 shows outfit 100130 and card 100701 outfit 100701. The uma folder also gets the outfit's other pictures at the game's size (`OUTFIT_PICTURES`): the square icon in its silver and gold frame (`icon.webp`, `icon-gold.webp`, 256 px), the round icon of a trained uma (`trained-icon.webp`, `trained-icon-gold.webp`), the 64 px notification icon (`push-icon.webp`), the outfit on its hanger (`dress.webp`) and the card's 512×256 gacha close-up (`gacha-thumb.webp`). Shared outfits such as the uniform have no uma folder and are left out. The uma document lists all of them under `pictures`.
- An atlas is one bundle of named sprites drawing from a second texture bundle; both are downloaded together, sprites are cut at the size they are drawn, and the atlas is rebuilt when either bundle changes.
- Every race track prefab (`3d/env/race/…/pfb_…`, 773 today, 15 tracks plus gates and props) becomes a binary glTF in its course's `parts/` folder, `art/racecourses/10001/parts/0000_002.glb`; gates, skies and props every course shares go to `art/racecourses/shared/` (`gates/`, `skies/`, `props/`). The file list's dependency column says which bundles a prefab draws from (meshes in `3d/probuilderexport/`, materials and textures under `sourceresources/`); they are downloaded with it, except shaders and race animations, and the model is rebuilt when any of them changes. A full first build downloads about 460 MiB.
- Every texture a model draws is embedded in it as full-size WebP (`EXT_texture_webp`), so one `.glb` opens on its own, in Blender or three.js. The game fills its course materials (turf, dirt, base) at runtime with one season and weather variant from the part's `textures/` folder; those textures are published on their own in the course's `textures/` folder (`art/racecourses/10006/textures/0000_112_turf00.webp`) and downloaded with the prefab, whose model embeds the first variant in those slots and lists every variant in the material's `extras.unity.variants` as links from the tree's root.
- Each racecourse is also published whole, as one `.glb` to drag into Blender: `art/racecourses/10006/model.glb`; the course's document gives its name from master.mdb and links its parts, textures and shaders. It shows a sunny spring day: the track and turf (`0000`, `0001`), the trees (`6000`) and the stands (`7001`–`7003`), each in the variant whose season, weather and time digits fit `112` best (`0` fits any), under the shared sky `race00000_9000_112`. These parts are modelled where they stand. Rails, signs and flower beds (`1xxx`–`5xxx`) are placed along the track by the game at runtime, so they stay out, as do the cutscene copy (`cutt000`) and `7013`, which only marks where 3D spectators stand. The course model is rebuilt when any of its prefabs, or anything they draw, changes.
- `scripts/game_art_model.py` writes the glTF, with one or more prefabs side by side. Runtime textures fill the materials named after their part and kind (`tex_env_race10006_0000_112_turf00` fills `mtl_env_race10006_0000_002_turf00_lm00`). Unity is left-handed, so X is mirrored and triangles reverse; V is flipped. The game's shaders have no glTF equivalent: `_MainTex` becomes the base colour, materials named `alpha` are cut out, and every texture slot (baked light in `_LightTex`, read with the second UV set), number and colour stays in `extras.unity`, each slot naming its embedded texture. `extras.unity.shader` names the material's shader and its file among the published shaders (`{"name": "Gallop/3D/Chara/Toon/TSER", "file": "gallop/3d/chara/toon/tser.json"}`), and `extras.unity.keywords` lists the shader keywords the material turns on. Models load the Windows shader bundle to look the names up, without being rebuilt when it changes. Vertex colours stay in `_UNITY_COLOR`, since standard viewers would tint by `COLOR_0`. Inactive objects carry `extras.active: false`. Models pass the Khronos glTF validator without errors or warnings.
- Every outfit with a body of its own (`dress_data` rows with a `chara_id`) becomes one uma: `art/umas/100101/model.glb`, titled after the character and outfit in master.mdb (`text_data` categories 6 and 14). It joins the body `3d/chara/body/bdy1001_00`, the head `3d/chara/head/chr1001_00` (`head_sub_id`) and the tail (the outfit's own `tail_model_id`, else the character's `tail0001_00`; staff have none) on one skeleton in its rest pose, a T-pose, sharing bones by name. Heads and tails are modelled at the origin, so their meshes are moved onto the T-pose and every skin binds to the rest pose; a mesh whose bones were bound rotated or apart is posed onto it vertex by vertex, as the game skins it there. The uma looks as it does at rest in the game: meshes the game switches off until an expression needs them, such as tears, are left out, and the blush (`M_Cheek`, which the game multiplies over the skin while an uma blushes) is fully transparent, with `extras.unity.blend` set to `multiply`. The game moves faces with bones rather than blend shapes: each head's facial target (`ast_chr1001_00_facial_target`) holds, per eye, eyebrow and mouth expression, bone offsets from its `Base` pose, which the game adds up by weight. Every expression is baked into a blend shape (a glTF morph target, a shape key in Blender) on the meshes it moves, named after the game client's `TargetInfomation.Part` and `FaceEyeType`, `FaceEyebrowType` or `FaceMouthType`: `EyeR CloseA`, `EyebrowL IkariA`, `Mouth TalkA_A_L`; eyes and eyebrows have one per side, so a wink is `EyeL CloseA` alone. `scripts/game_art_face.py` bakes them. The game colours a tail per character at runtime (`ModelLoader.GetTailTextures`), so its `_diff`, `_shad_c`, `_base` and `_ctrl` textures fill the tail material's `_MainTex`, `_ToonMap`, `_TripleMaskMap` and `_OptionMaskMap`, the slots the body and head materials hold those kinds in. Every texture and value the game's toon shaders read is in `extras.unity`, for a viewer to rebuild the look. The root is named after the outfit (`Special Week: Special Dreamer`) and its extras record the character, the outfit, its running form and which race motion sets it uses (`chara_type` scene 1, cut 1 for runs and starts, cut 4 for finishes). It is published under its body's folder and rebuilt when any of its prefabs, anything they draw or a tail texture changes. `scripts/game_art_uma.py` writes umas and race motions.
- Every race clip (`3d/motion/racemain/body/…`) becomes its own `.glb` on the skeleton every uma shares (Special Week's racewear and the common tail), without meshes, so it fits any uma: `art/motions/race/bright/run-fast-standard.glb`. Names come from the game client's own enums: the personality set (`type01`–`04` Bright, Sporty, Mature, Cute; `typexx` Default; higher numbers keep theirs as `special-12`), the pace (`run01`–`05` Slow, Normal, Fast, Blocked, Tired), the running form (`_base`, `_pitch`, `_stride` as standard, pitch, stride), the gate phase and leading foot, and the finish placing and course direction. The title, which Blender names the armature and action after, is `Bright: Fast run, standard form`, at most 63 bytes, and the root's extras keep the game's clip name. Clips the game code does not name (`type00`) are left out. A clip is rebuilt when it or the skeleton changes.
- Every shader the game draws with is published from the Android build's `shader` bundle, whose programs are GLSL ES, the language of WebGL: one JSON file per shader, named after it (`Gallop/3D/Chara/Toon/TSER` at `art/shaders/gallop/3d/chara/toon/tser.json`), and `art/shaders/index.json` listing every shader's name and file. A model's material names its shader's file as a link from the tree's root. Two pairs of shaders share a name (`Gallop/3D/Chara/MultiplyCheek`, `Gallop/3D/MirrorAndShadow/BlendProjector`); materials tell them apart by object ID, which every platform's bundle shares, so their files add it (`multiplycheek-fa04f11e193430d1.json`). A file keeps the shader's properties (type, default, range), keywords and sub-shaders; each pass keeps its tags, render state (Unity's enums; a value a material property sets names that property, such as `_Cull`) and one program per keyword variant: its GLSL version and its vertex and fragment shaders, as indexes into the file's shared `sources`. Uniforms are not placed by location (`UNITY_SUPPORTS_UNIFORM_LOCATION 0`), which WebGL 2 does not support; every `300 es` program then compiles and links in WebGL 2, and the one `310 es` variant (procedural instancing) needs GLES 3.1. Shader files the bundle no longer holds are deleted. `scripts/game_art_shader.py` writes them. The shaders are built before anything else in a run.
- Every outfit with a chibi (`dress_data.have_mini`) becomes one chibi beside its uma: `art/umas/100101/chibi.glb`. It joins the chibi body `3d/chara/mini/body/mbdy1001_00` (`body_type_sub`), the face every chibi shares (`3d/chara/mini/head/mchr0001_00/pfb_mchr0001_00_face0`, the only one the file list holds), the hair `3d/chara/mini/head/mchr1001_00/pfb_mchr1001_00_hair` (`head_sub_id`) and the uma's tail model as a chibi tail (`mtail0001_00`), the paths of the client's `MINI_BODY_SPECIAL_DRESS_MODEL_PATH`, `MINI_HAIR_MODEL_PATH` and `MINI_TAIL_MODEL_PATH`, on one skeleton at rest, like a uma. The face's materials are empty until the game fills them, so the model does: the skin and cheek by the character's skin colour (`chara_data.skin`; `MINI_FACE_TEXTURE_PATH`, `MINI_CHEEK_TEXTURE_PATH`: `tex_mchr0001_00_face0_1_diff`, `_cheek0`), the eyes, both eyebrows and the mouth with the head's sheets (`tex_mchr1001_00_eye_diff`, `_mayu_diff`, `_mouth_diff`), which hold every expression and show the default one. The cheek is the blush and hidden, as on a uma. The character's tail colour (`tex_mtail0001_00_1001_diff`) fills the tail material's `_MainTex`. The root's extras record the character and outfit. A texture the file list lacks is left out. It is published under its body's folder and rebuilt when any of its prefabs, anything they draw or one of its textures changes.
- Chibi and cut-in clips (`3d/motion/mini/…`, `3d/motion/cutin/…`, every `anm_` file) keep the part of their game name that tells them apart, since the game code names few of them: a cut-in clip goes to its cut-in's folder by the game's role folder (`anm_cti_crd100101_001_01_cam` to `art/umas/100101/cutin/camera/01_cam.glb`), a character's own chibi clip to `art/characters/1001/chibi-motions/<role>/`, and a chibi clip any chibi plays below `art/motions/chibi/<scene>/<set>/<role>/`. A body clip goes on its family's skeleton (Special Week's chibi body, hair and the common chibi tail; for cut-ins her racewear, head and the common tail), may move its root, and is rebuilt with the skeleton: `art/umas/100101/cutin/body/01.glb`. A camera or position clip (`_cam`, `_pos`) moves a root, with a child per other object it moves named by the CRC-32 of that object's path. Curves of these clips that move no transform, such as a camera's field of view, are kept in the root's `extras.unity.curves`. The clips that move no bone, a face's or ears' clip (`_face`, `_ear`, which the game plays through their controllers) and a prop's or set's clip (whose model is published apart), become JSON with every curve (`.json`): per binding the path's CRC-32, Unity's type ID, the attribute (transform attributes by name, in glTF space; others by their hash, with `customType`) and the keys per component, plus the clip's length, frame rate, loop flag and events. A `_driven` clip, which drives a face by script, becomes a scripts dump as below.
- Every skill cut-in timeline (`cutt/cutin/skill/crd100101_001/crd100101_001`) becomes `timeline.json` in its cut-in's folder (a card's in its uma's, `cutin-002/` for its second; a character's in its character's; any other below `art/cutins/<timeline>/`): every object of the bundle with its fields, MonoBehaviours under their class as `script`, clips with their curves, meshes and textures by name only. A reference names its object's type, name and path ID (a string, too large for a JavaScript number) and, when it lies in another bundle, that bundle's game name and the `file` it was published as. The bundles the timeline lists (its clips, effects and set) are downloaded only to name them, so a change to them does not rebuild it, but a move of their published files does; the timeline waits until all of them are listed.
- Every effect (`3d/effect/…/pfb_…`) and cut-in set (`3d/env/cutin/…/pfb_…`) becomes a `.glb`, drawn like a track prefab, that also keeps each object's other components in its node's `extras.unity.components`, as `game_art_unity.py` reads them, in Unity's units: particle systems with only the modules they switch on (a curve or gradient keeps only what its `minMaxState` uses), particle and trail renderers, lights, animators and scripts. A reference there names its object's type and name and, where the model holds it, its index: `material`, `mesh` (meshes only particles draw are added unattached), `texture`, `node`, or `clip` in the document's `extras.unity.clips`. A character's skill effect goes to its character (`art/characters/1001/effects/00/01.glb`), any other below `art/effects/` by its game folders; a set goes to the folder of the one cut-in that plays it (`set/00.glb`), else below `art/cutins/sets/`. Every prop (`3d/chara/prop/<prop>/pfb_…`) becomes `art/props/<prop>.glb` on its own bones.
- The game squeezes textures to powers of two (a 3:4 support card is stored as 2048×2048), so those outputs are resized to the size the site serves. Character icons are narrower than their 256×256 canvas and are centred on it. Chibi sprites are stored at their real size (most 256×256, wider for scenes with props) and keep it.
- What was published is recorded in the bucket (game file, source, checksum, converter revision, output paths), so a run with no changes downloads no game files, and a new card that wears an existing outfit still gets its portrait. The first run builds everything.
- The record is a folder, `_state/game-art/`, with one shard per kind of file: track scenery by what it shows (`racetracks.json` for the whole courses, `racetrack-parts.json`, `skyboxes.json`, `starting-gates.json`, `track-props.json`), umas, race motions and shaders apart (`umas.json`, `uma-motions.json`, `shaders.json`), as are chibis, their clips, cut-in clips and timelines (`chibis.json`, `chibi-motions.json`, `cutin-motions.json`, `cutins.json`) and effects, cut-in sets and props (`effects.json`, `cutin-sets.json`, `props.json`), every other file by its top-level game folder (`chara.json`, `supportcard.json`). `index.json` links every shard like the resources manifest links its artifacts: `{"name": "chara", "path": "/_state/game-art/chara.json", "files": 230}`, with the resource version and when it was written. Only the shards batches touched are rewritten, at most every 30 seconds and at the end of a run; a run that stops in between builds those files again, and their unchanged uploads are skipped. A record in an earlier layout, or one whose records belong to another shard now, is rewritten once and its old files removed.
- Raising `CONVERTER_REVISION` rebuilds every picture once, `MODEL_CONVERTER_REVISION` every track and course model, `UMA_CONVERTER_REVISION` every uma and race motion, `CHIBI_CONVERTER_REVISION` every chibi and chibi clip, `CUTIN_CONVERTER_REVISION` every cut-in timeline, clip, set and prop, and `EFFECT_CONVERTER_REVISION` every effect. Revision 2 embedded the textures that earlier models kept beside them.
- Nothing the publisher put in the bucket stays behind once no record lists it. A rebuild that moves a file's output to another key (a card added for an outfit, a clash resolved) deletes the old key, and so do the loose textures of earlier models. A record the catalog no longer builds is removed with what only it listed: a file no source lists any more or the catalog stopped serving, a racecourse whose track is gone, an outfit that left master.mdb or whose body is gone. Records waiting for a missing dependency stay. Keys are deleted before the record is saved, so an interrupted run deletes them again, and their URLs are purged. Objects it never recorded, such as the copied web root and the timeline banners, are never deleted. When more than 20 records, and more than 20% of them, look orphaned at once, none are removed and a warning is logged, since an incomplete file list is likelier than such a change.
- A file that does not convert is recorded and skipped until the game changes it; a file that fails to download is retried on the next check.
- Decrypted bundles and converted images live in a temporary folder that is removed after each run and at startup.
- Each region is a tree of its own, and trees share no files. Global is published in `en/`. JP is published in `jp/` by the same server: `jp/art/umas/100101/model.glb`, its record in `jp/_state/game-art/`. The timeline banners belong to Global's tree. A tree is built from its region's CDN and its region's master only, so the JP tree holds every card and outfit JP has, titled in Japanese, and the Global tree never holds a JP file. Links inside a tree's documents are relative to that tree's root. Both trees place files by the same rules, so a file both masters describe alike has the same key in both, and a page that lacks a Global picture can ask for the same key below `jp/`. Keys differ where the masters differ: an outfit that has no card in Global yet is filed under its character and outfit there (`art/umas/1054-105401/`) and under its card in JP (`art/umas/105401/`), and its cut-ins likewise.
- The JP master is kept beside the Global one as `jp-master.mdb`. Versions come from `GAME_VERSION_URL` (default `https://uma.moe/api/ver`), asked as `?region=jp` for JP. A JP resource version is `number:token`; the token is base64 of the root manifest's size and checksum, and the JP CDN stores that manifest under the name they hash to, like every other manifest. An answer without a token is refused, so a backend that does not serve regions yet cannot pass the Global version off as JP's. Both CDNs encrypt bundles with the same `GAME_ASSET_KEY`.
- The server also generates a JP resource set from the JP master and serves it below `/resources/jp/`, with a manifest of its own: `/resources/jp/manifest.json`, `/resources/jp/current/<file>.json.gz` and `/resources/jp/<version>/<file>.json.gz`. It lives in `jp/` of the data directory and shares nothing with the Global export. The set holds what the JP master alone describes, under the names the Global set uses and in the same shapes: `factors`, `race_program`, `race_to_saddle_mapping`, `room_match_races`, the three banner lists, `affinity`, `character_names`, `character`, `supports`, `support-cards-db` and `skills`. Text is Japanese. Every card is marked `isReleased_jp`, and `isReleased_en` when the Global master has it too, so the set is regenerated when either master changes. The timeline, planner and simulator resources are built around Global's schedule and the store's Global data and have no JP counterpart. `generate --region jp` writes the set once.
- JP needs no configuration. `JP_RESOURCES=false` turns the JP set off and `JP_GAME_ART_PUBLISH=false` the JP tree, in the host's env file (`/opt/umamoe-resources/env`, `/opt/umamoe-resources-beta/env`).
- The refresh loop is the only writer of the JP master. It keeps it current unless both switches are off. The server starts serving without waiting for JP's first check, and a failed JP check never holds back the Global refresh.
- Regions are published one after the other on every check, Global first, because they share the converter and its work folder. JP's first build is several times Global's (54,219 files against 458 umas and 568 support cards in November 2026), and a Global update waits for the JP run in progress.
- One server publishes (`GAME_ART_PUBLISH=true`), so two never race on the bucket's record. The repository variable `GAME_ART_PUBLISHER` (`beta` or `production`, default `production`) picks that server; after changing it, deploy both so the old publisher stops. The publisher needs `GAME_ASSET_KEY` (the game's asset bundle key, hex) besides the bucket settings.

Check conversions without the bucket, using the Docker image (it has the converter):

```powershell
docker run --rm --env-file .env -v ${PWD}/art-sample:/out -v ${PWD}/art-master:/work umamoe-resources game-art --master /work/master.mdb --out-dir /out --sample 3
```

`--only art/umas/100101/` keeps the files served below a key prefix, to check one kind; repeat it for several. The sample folder gets the directory documents of what was built.

The converter's tests need UnityPy, so they run in the image too:

```powershell
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_game_art_uma test_game_art_face test_game_art_shader test_game_art_unity test_game_art_convert
```

`game-art --dry-run` lists what the next run would build and delete and the largest documents; `game-art` runs it once. `--region jp` does the same for the JP tree, and `fetch-master --region jp` only downloads the JP master. `game-art --dry-run --out-dir <folder>` writes every directory document there as if every file converted, downloading no bundles.

`asset-bucket inventory` logs the objects and bytes below each top folder of the bucket. `asset-bucket clear --expect <count>` deletes every object, only when the bucket holds exactly that many, so nothing written since the inventory goes unseen. The R2 token only works from the deploy host, so both run there through the deploy workflow: dispatch it on `main` with `asset_bucket` set to `inventory` (lists the bucket and skips the deploy), then `clear` with `expected_objects` from that inventory, which stops the beta container, empties the bucket and deploys beta, whose publisher then builds everything again.

## Timeline Banners

The publishing server (`GAME_ART_PUBLISH=true`) also publishes the picture of every banner the timeline shows, one file per banner below `art/timeline/`: the official English art once it exists, else the official Japanese art, else the site's own file. It reads the current `banner_timeline.json.gz` in the data folder at start and every 30 minutes (`TIMELINE_ART_INTERVAL_SECONDS`, at least 60), which catches new timelines, new news archives and English art uploaded around a release. A run with nothing new downloads nothing.

- A banner keeps the timeline's `image_path` below `art/timeline/`, without its `assets/images/` or `assets/timeline-images/` prefix: `assets/images/character/banner/2021_30002.webp` is `art/timeline/character/banner/2021_30002.webp`, `assets/timeline-images/events/campaign/1001.webp` is `art/timeline/events/campaign/1001.webp`. An event without a picture of its own gets its type's picture for its master id, `assets/timeline-images/events/<category>/<id>.webp`, as the frontend resolves it.
- `art/timeline/index.json` links each published banner by its `image_path`, from the tree's root: `{"schema": 1, "images": {"assets/images/campaign/100.webp": "/art/timeline/campaign/100.webp"}}`. It lists only published files, is cached for five minutes and is uploaded only when it changes.
- Sources are picked as the frontend's image sync picked them (`src/generators/timeline_images/`, a port of `scripts/sync_umapyoi_timeline_images.mjs`). Japanese art comes from the Umapyoi archive: the gacha's banner, else the closest titled post of the event's type, campaigns by the phase they belong to. English art comes from the official Global news in the stored archive: the announcement's picture, else the `_L<release>` header the game uploads for a banner. Only confirmed events get an English source, and a predicted Champions Meeting keeps its Japanese art even where the site has English art for it. English art is only as fresh as the archive, which the publisher refreshes every 30 minutes.
- Japanese art is scaled down to at most 512 px wide. English art is fitted into the size of the banner's Japanese art, stretched at most 10% in height rather than cropped. Pictures are WebP at quality 88, lossless when they have alpha, converted by `scripts/timeline_art_convert.py` at the lowest CPU priority. A campaign's thumbnail banner is preferred over its header when it exists.
- Better art overwrites the same key and its URL is purged; English art is never traded back for Japanese. A missing English header (403/404) is asked for again on every run until a week after its release, then retired.
- A banner's first run copies the site's file when its own sources fail, so nothing the site shows is lost: the synced copy the site's manifests list (`/assets/timeline-images/{en,jp}/manifest.json` on `TIMELINE_ART_SITE_ORIGIN`, default `https://uma.moe`), else the original. A copied English picture may be older art, such as last year's cup of a Champions Meeting, so its planned source replaces it once that downloads.
- Requests go one at a time, 250 ms apart, retried with backoff for 429 and 5xx.
- `_state/timeline-art/index.json` records each banner's variant, planned source, download URL, fit revision and size, and the retired English sources. Raising `FIT_REVISION` rebuilds every banner. Banners are never deleted when an event leaves the timeline, since a cached older timeline may still name them.

```powershell
cargo run -- timeline-art --timeline <banner_timeline.json or .json.gz> --dry-run
cargo run -- timeline-art --timeline <banner_timeline.json> --compare-frontend ../umamoe-frontend/timeline-image-sync
docker run --rm -v ${PWD}/timeline-sample:/out -v ${PWD}/generated-data:/data umamoe-resources timeline-art --data-dir /data --out-dir /out --sample 3
```

- `--dry-run` counts the banners with an English, a Japanese or no source.
- `--compare-frontend` lists every banner whose source differs from those the frontend's sync recorded. Those records keep mappings from older timelines and came from the live news feed, so some differences are expected; on the same timeline and archives the two pickers agree.
- `--out-dir` builds a few banners per folder (`--sample`, default 3) and their index into a folder instead of the bucket. It needs the image's converter.

The converter's tests run in the image:

```powershell
docker run --rm --entrypoint /opt/game-art/bin/python -v ${PWD}/scripts:/scripts -w /scripts umamoe-resources -m unittest test_timeline_art_convert
```

The frontend's Sync Timeline Images workflow keeps running until the site reads its banners from the bucket.

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

The repository variable `STORE_S3_BUCKET`, when set, goes into the same file for every server, so the [shared store](#shared-store) lives in that bucket's `store/` folder instead of the asset bucket's. The secrets `R2_CICD_ACCESS_KEY_ID` and `R2_CICD_SECRET_ACCESS_KEY` (the 32- and 64-character pair of an R2 token, both or neither) go in as `STORE_S3_ACCESS_KEY_ID` and `STORE_S3_SECRET_ACCESS_KEY`, so the store is reached with that token instead of the asset token. Set them in each deploy environment: a token with Object Read & Write on the store's bucket for the publisher, Object Read only for a reading server. The repository secret `ANTHROPIC_API_KEY` goes in only on the publishing server, which translates names; without it a deploy warns that names only JP has released stay untranslated.

The repository variable `GAME_ASSET_KEY` (the game's asset bundle key as hex, UmaViewer's `ABKeyText`) goes into the same file. UmaViewer publishes it, so it is a variable rather than a secret, shared by beta and production. Production needs it to build game art.

The repository secrets `CLOUDFLARE_API_TOKEN` (a token with Zone, Cache Purge permission on the site's zone) and `CLOUDFLARE_ZONE_ID` go into the same file when both are set. It is read after the manual `env` file, so they replace a token kept there; without them, the manual file's values apply.

Manual remote `env` files should contain the normal application env values only:

- `DATABASE_URL` (legacy compatibility; optional)
- `MASTER_REFRESH_INTERVAL_SECONDS`
- `PURGE_ON_REFRESH`
- `CLOUDFLARE_ZONE_ID` and `CLOUDFLARE_API_TOKEN`, unless the repository secrets above set them
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
