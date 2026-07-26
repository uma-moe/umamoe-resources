#!/usr/bin/env node

import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const SCRIPT_ROOT = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_ROOT, '..');
const ARCHIVE_PATH = path.join(REPO_ROOT, 'src', 'jp_data', 'umapyoi_archive.json');
const RECOVERED_CAMPAIGNS_PATH = path.join(
  REPO_ROOT,
  'src',
  'jp_data',
  'english_mission_campaign_assets.json'
);
const EN_NEWS_API = 'https://umamusume.com/api/ajax/pr_info_index?format=json';
const EN_ASSET_ROOT =
  'https://assets-webview-umamusume-en.akamaized.net/contents/assets/images/uploads/Header';
const USER_AGENT = 'umamoe-resources-image-sync/2.0 (+https://uma.moe)';
const JP_TRANSFORM_VERSION = 'official-news-max512-native-aspect-v2';
const EN_TRANSFORM_VERSION = 'jp-aspect-uncropped-mild-stretch-v9';
const MAX_EN_VERTICAL_STRETCH = 1.1;
const EN_CAMPAIGN_PHASE_DAYS = 56;
const EN_RETRY_DAYS = 7;

function argumentsFrom(argv) {
  const args = { requestIntervalMs: 250, refreshJp: false };
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === '--timeline-json') args.timelineJson = argv[++index];
    else if (value === '--frontend-root') args.frontendRoot = argv[++index];
    else if (value === '--request-interval-ms') args.requestIntervalMs = Number(argv[++index]);
    else if (value === '--changed-files-output') args.changedFilesOutput = argv[++index];
    else if (value === '--refresh-jp') args.refreshJp = true;
    else throw new Error(`unknown argument: ${value}`);
  }
  if (!args.timelineJson || !args.frontendRoot) {
    throw new Error('--timeline-json and --frontend-root are required');
  }
  args.timelineJson = path.resolve(args.timelineJson);
  args.frontendRoot = path.resolve(args.frontendRoot);
  return args;
}

const args = argumentsFrom(process.argv.slice(2));
const requireFromFrontend = createRequire(path.join(args.frontendRoot, 'package.json'));
const sharp = requireFromFrontend('sharp');

const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const preferredEnglishSourceCache = new Map();
const normalizedTitle = value =>
  String(value ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim()
    .replace(/\s+/g, ' ');
const eventTypes = post => new Set(Array.isArray(post.event_types) ? post.event_types : []);
const dateValue = value => {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? null : date;
};
const daysBetween = (left, right) => Math.abs(left - right) / 86_400_000;
const sortedObject = value => Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)));
const decodeHtml = value =>
  String(value ?? '')
    .replaceAll('&amp;', '&')
    .replaceAll('&#x2F;', '/')
    .replaceAll('&quot;', '"');

async function readJson(file, fallback = {}) {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8'));
  } catch (error) {
    if (error.code === 'ENOENT') return fallback;
    throw error;
  }
}

async function writeJson(file, value) {
  await fs.mkdir(path.dirname(file), { recursive: true });
  const temporary = `${file}.tmp`;
  await fs.writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
  await fs.rename(temporary, file);
}

function preferredPostImage(post) {
  const candidates = [];
  const seen = new Set();
  for (const image of Array.isArray(post.images) ? post.images : []) {
    if (!image || typeof image.url !== 'string' || !/^https?:/.test(image.url) || seen.has(image.url)) {
      continue;
    }
    seen.add(image.url);
    candidates.push(image);
  }
  candidates.sort((left, right) => imageScore(right) - imageScore(left));
  return candidates[0]?.url;
}

function imageScore(image) {
  const url = image.url.toLowerCase();
  return (
    (image.likely_banner === true ? 30 : 0) +
    (url.includes('/thumbnail/banner_') ? 25 : 0) +
    (image.field_path === '$.image' ? 20 : 0) +
    (image.field_path === '$.article_image' ? 15 : 0) +
    (url.includes('/header/') ? 12 : 0) +
    (url.includes('gacha_banner_') ? 10 : 0) +
    (url.includes('header_') ? 6 : 0) -
    (url.includes('announce_banner_') ? 8 : 0)
  );
}

function campaignSemanticKey(value, japanese = false) {
  const title = normalizedTitle(value);
  if (!japanese && !title.includes('mission')) return null;
  const part = title.match(/\b(?:part|vol) (\d+)\b/)?.[1] ?? '1';
  if (title.includes('g1 celebration missions') && title.includes('february stakes')) {
    return 'g1-celebration-missions-february-stakes';
  }
  const anniversary = title.match(/\b(\d+)(?:st|nd|rd|th) anniv(?:ersary)?\b/)?.[1];
  if (title.includes('half anniversary')) return `half-anniversary-missions-part-${part}`;
  if (anniversary && title.includes('anniversary')) {
    return `${anniversary}-anniversary-missions-part-${part}`;
  }
  if (title.includes('spring g1') || title.includes('spring gi')) {
    return `spring-g1-missions-part-${part}`;
  }
  if (['fall g1', 'fall gi', 'autumn g1', 'autumn gi'].some(term => title.includes(term))) {
    return `fall-g1-missions-part-${part}`;
  }
  if (title.includes('golshi') || title.includes('gw special')) return 'golshi-week-special-missions';
  if (!japanese && title.includes('tracen special') && title.includes('training the trainer')) {
    return `training-the-trainer-part-${part}`;
  }
  const chapter = title.match(/main story (?:ch|chapter) (\d+)/)?.[1];
  if (!japanese && chapter && title.includes('release celebration')) {
    return `main-story-chapter-${chapter}-missions`;
  }
  if (!japanese && title.includes('release celebration missions')) {
    return `release-celebration-missions-part-${part}`;
  }
  if (title.includes('gi campaign') || title.includes('g1 campaign')) {
    return 'g1-celebration-missions';
  }
  if (title.startsWith('g1 celebration missions')) {
    return part === '1' && !/\b(?:part|vol) 1\b/.test(title)
      ? 'g1-celebration-missions'
      : `g1-celebration-missions-part-${part}`;
  }
  return null;
}

function titleSimilarity(left, right) {
  const ignored = new Set([
    'the', 'a', 'an', 'is', 'are', 'now', 'here', 'held', 'event', 'campaign', 'missions',
    'mission', 'race', 'live', 'underway'
  ]);
  const leftTokens = new Set(normalizedTitle(left).split(' ').filter(token => token && !ignored.has(token)));
  return new Set(
    normalizedTitle(right)
      .split(' ')
      .filter(token => token && !ignored.has(token) && leftTokens.has(token))
  ).size;
}

function eventGachaIds(event) {
  return [event.gacha_id, ...(Array.isArray(event.gacha_ids) ? event.gacha_ids : [])]
    .filter((value, index, values) => Number.isInteger(value) && values.indexOf(value) === index);
}

function officialJapaneseJobs(timeline, archive) {
  const posts = Array.isArray(archive.news) ? archive.news : [];
  const jobs = new Map();
  const gachaSources = new Map();
  for (const post of posts) {
    for (const banner of Array.isArray(post.gacha_banners) ? post.gacha_banners : []) {
      if (Number.isInteger(banner.gacha_id) && typeof banner.image_url === 'string') {
        gachaSources.set(banner.gacha_id, banner.image_url);
      }
    }
  }
  const typedPosts = posts
    .map(post => ({
      post,
      date: dateValue(post.posted_at),
      source: preferredPostImage(post),
      types: eventTypes(post)
    }))
    .filter(candidate => candidate.date && candidate.source);
  const aliases = new Map([
    ['story_event', 'story_event'], ['champions_meeting', 'champions_meeting'],
    ['legend_race', 'legend_race'], ['league_of_heroes', 'league_of_heroes'],
    ['masters_challenge', 'masters_challenge'], ['trainer_skills_test', 'trainer_skills_test'],
    ['factor_research', 'factor_research'], ['strongest_team', 'strongest_team'],
    ['racing_carnival', 'racing_carnival'], ['scenario_release', 'training_scenario']
  ]);
  const campaignEvents = [];

  for (const event of timeline.events ?? []) {
    if (typeof event.image_path !== 'string' || !event.image_path.endsWith('.webp')) continue;
    const target = path.join(args.frontendRoot, 'src', event.image_path);
    const sourcedGachaId = eventGachaIds(event).find(gachaId => gachaSources.has(gachaId));
    if (sourcedGachaId !== undefined) {
      jobs.set(target, { source: gachaSources.get(sourcedGachaId), eventType: event.type });
      continue;
    }
    if (typeof event.image === 'string' && /^https?:/.test(event.image)) {
      jobs.set(target, { source: event.image, eventType: event.type });
    }
    const release = dateValue(event.jp_release_date);
    if (!release) continue;
    if (event.type === 'campaign') {
      campaignEvents.push({ target, release, key: campaignSemanticKey(event.title, true) });
      continue;
    }
    const archiveType = aliases.get(event.type);
    if (!archiveType) continue;
    const candidates = [];
    for (const candidate of typedPosts) {
      if (!candidate.types.has(archiveType) || daysBetween(candidate.date, release) > 2) continue;
      const similarity = titleSimilarity(event.title, candidate.post.title);
      const dedicatedStory =
        archiveType === 'story_event' &&
        candidate.types.size === 1 &&
        candidate.types.has('story_event') &&
        /(?:banner|header)_301\d+/i.test(candidate.source);
      if (similarity === 0 && !dedicatedStory) continue;
      candidates.push({
        quality: similarity * 10 + (dedicatedStory ? 30 : 0),
        delta: daysBetween(candidate.date, release),
        source: candidate.source
      });
    }
    candidates.sort((left, right) => right.quality - left.quality || left.delta - right.delta);
    if (candidates[0]) jobs.set(target, { source: candidates[0].source, eventType: event.type });
  }

  for (const candidate of typedPosts) {
    if (!candidate.types.has('campaign')) continue;
    const key = campaignSemanticKey(candidate.post.title, true);
    if (!key) continue;
    for (const event of campaignEvents) {
      if (event.key !== key || event.release < candidate.date - 2 * 86_400_000 ||
          event.release > candidate.date.getTime() + EN_CAMPAIGN_PHASE_DAYS * 86_400_000) continue;
      jobs.set(event.target, { source: candidate.source, eventType: 'campaign' });
    }
  }
  return jobs;
}

function assetIdentity(url) {
  const match = decodeHtml(url).match(/\/(?:Thumbnail|Header)\/([^/?]+?)(?:_L\d+)?\.(?:png|jpe?g|webp)(?:\?|$)/i);
  return match?.[1]?.toLowerCase();
}

function officialPostAssetUrls(post) {
  const urls = [];
  if (typeof post.image === 'string' && /^https?:/.test(post.image)) urls.push(decodeHtml(post.image));
  if (typeof post.message === 'string') {
    urls.push(...[...post.message.matchAll(/https:\/\/assets-webview-umamusume-en\.akamaized\.net\/[^"'< >]+/gi)]
      .map(match => decodeHtml(match[0])));
  }
  return [...new Set(urls)];
}

async function fetchOfficialEnglishNews() {
  const posts = [];
  for (let offset = 0; offset < 5000; offset += 100) {
    const response = await fetchWithRetry(EN_NEWS_API, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'user-agent': USER_AGENT },
      body: JSON.stringify({ announce_label: 0, limit: 100, offset })
    });
    const payload = await response.json();
    if (!Array.isArray(payload.information_list)) throw new Error('invalid official EN news response');
    posts.push(...payload.information_list);
    if (!payload.information_list.length || !payload.show_more_button) break;
  }
  return posts;
}

function storyBannerIds(archive) {
  const ids = new Map();
  for (const post of archive.news ?? []) {
    if (!eventTypes(post).has('story_event') || typeof post.posted_at !== 'string') continue;
    for (const image of post.images ?? []) {
      const match = String(image.url).match(/\/(?:Thumbnail|Header)\/(?:banner|header)_(301\d+)/i);
      if (match && !ids.has(post.posted_at.slice(0, 10))) ids.set(post.posted_at.slice(0, 10), match[1]);
    }
  }
  return ids;
}

function timelineStoryIdentities(timeline, archive) {
  const byDate = storyBannerIds(archive);
  const stories = (timeline.events ?? []).filter(event => event.type === 'story_event')
    .sort((left, right) => String(left.jp_release_date).localeCompare(String(right.jp_release_date)));
  return new Map(stories.map((event, index) => [
    event.id,
    `banner_${byDate.get(String(event.jp_release_date).slice(0, 10)) ?? 30_100_000 + (index + 1) * 2}`
  ]));
}

function officialCampaignAssets(posts, recovered) {
  const candidates = [...recovered];
  for (const post of posts) {
    const title = String(post.title ?? '');
    if (normalizedTitle(title).includes('mission') && typeof post.image === 'string' && post.post_at) {
      candidates.push({ title, post_at: post.post_at, image: decodeHtml(post.image) });
    }
  }
  return candidates;
}

function championsTitleMatch(event, posts) {
  const eventTitle = normalizedTitle(event.title);
  if (!eventTitle || eventTitle === 'champions meeting') return null;
  const candidates = [];
  for (const post of posts) {
    const title = normalizedTitle(post.title);
    if (!title.includes('champions meeting') || !title.includes(eventTitle) || typeof post.image !== 'string') continue;
    const identity = assetIdentity(post.image) ?? '';
    const score = (identity.startsWith('banner_3031') ? 8 : 0) +
      (title.includes('is here') ? 4 : 0) -
      (title.includes('coming') ? 4 : 0) -
      (title.includes('league selection') ? 4 : 0);
    candidates.push({ score, posted: post.post_at, source: decodeHtml(post.image) });
  }
  candidates.sort((left, right) => right.score - left.score || String(right.posted).localeCompare(String(left.posted)));
  return candidates[0]?.score >= 8 ? candidates[0].source : null;
}

function officialEnglishJobs(timeline, archive, posts, recovered) {
  const assets = new Map();
  for (const post of posts) {
    for (const url of officialPostAssetUrls(post)) {
      const identity = assetIdentity(url);
      if (identity) assets.set(identity, url);
    }
  }
  const storyIdentities = timelineStoryIdentities(timeline, archive);
  const jobs = new Map();
  for (const event of timeline.events ?? []) {
    if (typeof event.image_path !== 'string' || !event.image_path.endsWith('.webp')) continue;
    const identities = [];
    const identity = typeof event.image === 'string' ? assetIdentity(event.image) : null;
    if (identity) identities.push(identity);
    identities.push(...eventGachaIds(event).map(gachaId => `gacha_banner_${gachaId}`));
    if (storyIdentities.has(event.id)) identities.push(storyIdentities.get(event.id));
    let source = identities.map(key => assets.get(key)).find(Boolean);
    if (!source && event.type === 'champions_meeting' && event.source === 'champions') {
      source = championsTitleMatch(event, posts);
    }
    if (source) jobs.set(path.join(args.frontendRoot, 'src', event.image_path), { source, eventType: event.type });
  }

  const campaignEvents = (timeline.events ?? [])
    .filter(event => event.type === 'campaign' && typeof event.image_path === 'string')
    .map(event => ({
      target: path.join(args.frontendRoot, 'src', event.image_path),
      release: dateValue(event.global_release_date),
      key: campaignSemanticKey(event.title)
    }))
    .filter(event => event.release);
  for (const source of officialCampaignAssets(posts, recovered)) {
    const posted = dateValue(source.post_at);
    const availableUntil = dateValue(source.available_until);
    if (!posted || typeof source.image !== 'string') continue;
    const key = campaignSemanticKey(source.title);
    const matches = campaignEvents.filter(event => {
      const inRange = availableUntil && event.release >= posted && event.release <= availableUntil;
      if (key && event.key !== key) return false;
      if (key) return inRange || (event.release >= posted - 2 * 86_400_000 &&
        event.release <= posted.getTime() + EN_CAMPAIGN_PHASE_DAYS * 86_400_000);
      return inRange || daysBetween(event.release, posted) <= 7;
    }).sort((left, right) => daysBetween(left.release, posted) - daysBetween(right.release, posted));
    if (key) {
      for (const event of matches) if (!jobs.has(event.target)) jobs.set(event.target, { source: source.image, eventType: 'campaign' });
    } else if (matches.length === 1) {
      if (!jobs.has(matches[0].target)) jobs.set(matches[0].target, { source: source.image, eventType: 'campaign' });
    }
  }
  return jobs;
}

function fallbackEnglishJobs(timeline, archive) {
  const jobs = new Map();
  const storyIds = storyBannerIds(archive);
  const stories = (timeline.events ?? []).filter(event => event.type === 'story_event')
    .sort((left, right) => String(left.jp_release_date).localeCompare(String(right.jp_release_date)));
  stories.forEach((event, index) => {
    if (!event.is_confirmed || !String(event.image_path).startsWith('assets/images/story/')) return;
    const release = dateValue(event.global_release_date);
    if (!release) return;
    const bannerId = storyIds.get(String(event.jp_release_date).slice(0, 10)) ?? 30_100_000 + (index + 1) * 2;
    jobs.set(path.join(args.frontendRoot, 'src', event.image_path), {
      source: `${EN_ASSET_ROOT}/banner_${bannerId}_L${Math.floor(release / 1000)}.png`,
      eventType: event.type
    });
  });
  for (const event of timeline.events ?? []) {
    if (!event.is_confirmed || typeof event.image_path !== 'string') continue;
    const release = dateValue(event.global_release_date);
    if (!release) continue;
    const gachaId = eventGachaIds(event)[0];
    if (['character_banner', 'support_card_banner', 'paid_banner'].includes(event.type) && gachaId !== undefined) {
      jobs.set(path.join(args.frontendRoot, 'src', event.image_path), {
        source: `${EN_ASSET_ROOT}/gacha_banner_${gachaId}_L${Math.floor(release / 1000)}.png`,
        eventType: event.type
      });
    } else if (['campaign', 'champions_meeting', 'factor_research', 'league_of_heroes', 'legend_race', 'masters_challenge',
      'racing_carnival', 'scenario_release', 'strongest_team', 'trainer_skills_test'].includes(event.type)) {
      const identity = typeof event.image === 'string' ? assetIdentity(event.image) : null;
      if (identity) jobs.set(path.join(args.frontendRoot, 'src', event.image_path), {
        source: `${EN_ASSET_ROOT}/${identity}_L${Math.floor(release / 1000)}.png`,
        eventType: event.type
      });
    }
  }
  return jobs;
}

function relativeToFrontend(file) {
  return path.relative(args.frontendRoot, file).replaceAll(path.sep, '/');
}
function publicAssetPath(file) {
  return relativeToFrontend(file).replace(/^src\//, '');
}
function localizedTarget(legacyTarget, locale) {
  const assetsRoot = path.join(args.frontendRoot, 'src', 'assets');
  let relative = path.relative(assetsRoot, legacyTarget);
  if (relative.split(path.sep)[0] === 'timeline-images') relative = relative.split(path.sep).slice(1).join(path.sep);
  return path.join(assetsRoot, 'timeline-images', locale, relative);
}

async function fetchWithRetry(url, options = {}, missingOkay = false) {
  for (let attempt = 0; attempt < 4; attempt += 1) {
    try {
      const response = await fetch(url, { ...options, headers: { 'user-agent': USER_AGENT, ...(options.headers ?? {}) } });
      if (response.ok) return response;
      if (missingOkay && [403, 404].includes(response.status)) return null;
      if (response.status !== 429 && response.status < 500) throw new Error(`${response.status} ${url}`);
    } catch (error) {
      if (attempt === 3) throw error;
    }
    await sleep(2 ** attempt * 1000);
  }
  throw new Error(`failed to download ${url}`);
}

async function download(url, missingOkay = false) {
  const response = await fetchWithRetry(url, {}, missingOkay);
  return response ? Buffer.from(await response.arrayBuffer()) : null;
}

async function imageSize(file) {
  try {
    const metadata = await sharp(await fs.readFile(file)).metadata();
    return metadata.width && metadata.height ? [metadata.width, metadata.height] : null;
  } catch {
    return null;
  }
}

async function replaceFile(temporary, target) {
  for (let attempt = 0; attempt < 10; attempt += 1) {
    try {
      await fs.rm(target, { force: true });
      await fs.rename(temporary, target);
      return;
    } catch (error) {
      if (!['EBUSY', 'EPERM', 'EACCES'].includes(error.code) || attempt === 9) throw error;
      await sleep(100 * (attempt + 1));
    }
  }
}

async function saveWebp(payload, target, options = {}) {
  const metadata = await sharp(payload).metadata();
  let width = metadata.width;
  let height = metadata.height;
  if (!width || !height) throw new Error(`image dimensions unavailable for ${target}`);
  if (options.maximumWidth && width > options.maximumWidth) {
    height = Math.round(height * options.maximumWidth / width);
    width = options.maximumWidth;
  }
  if (options.targetSize) {
    const [targetWidth, targetHeight] = options.targetSize;
    const sourceAspect = width / height;
    const targetAspect = targetWidth / targetHeight;
    const scale = Math.min(targetWidth / width, targetHeight / height);
    width = Math.round(width * scale);
    height = Math.round(height * scale);
    if (sourceAspect > targetAspect) {
      const stretch = Math.min(sourceAspect / targetAspect, MAX_EN_VERTICAL_STRETCH);
      height = Math.min(targetHeight, Math.round(height * stretch));
    }
  }
  const temporary = `${target}.tmp`;
  await fs.mkdir(path.dirname(target), { recursive: true });
  await fs.rm(temporary, { force: true });
  try {
    await sharp(payload)
      .resize(width, height, { fit: 'fill', kernel: sharp.kernel.lanczos3 })
      .webp({ quality: 88, lossless: Boolean(metadata.hasAlpha), effort: options.effort ?? 6 })
      .toFile(temporary);
    await replaceFile(temporary, target);
  } catch (error) {
    await fs.rm(temporary, { force: true });
    throw error;
  }
}

async function preferredEnglishSource(source, eventType) {
  if (eventType !== 'campaign') return source;
  if (preferredEnglishSourceCache.has(source)) return preferredEnglishSourceCache.get(source);
  const candidate = source.replace('/Header/header_', '/Thumbnail/banner_');
  if (candidate === source) return source;
  const response = await fetchWithRetry(candidate, { method: 'HEAD' }, true);
  const resolved = response ? candidate : source;
  preferredEnglishSourceCache.set(source, resolved);
  return resolved;
}

async function main() {
  const timeline = await readJson(args.timelineJson);
  const archive = await readJson(ARCHIVE_PATH);
  const recovered = await readJson(RECOVERED_CAMPAIGNS_PATH, []);
  const jpSources = officialJapaneseJobs(timeline, archive);
  const fallbackEn = fallbackEnglishJobs(timeline, archive);
  let officialNews = [];
  let officialError = null;
  try {
    officialNews = await fetchOfficialEnglishNews();
  } catch (error) {
    officialError = error;
  }
  const enSources = new Map(fallbackEn);
  if (!officialError) {
    for (const [target, value] of officialEnglishJobs(timeline, archive, officialNews, recovered)) {
      enSources.set(target, value);
    }
  }

  const stateRoot = path.join(args.frontendRoot, 'timeline-image-sync');
  const jpStatePath = path.join(stateRoot, 'japanese-image-sources.json');
  const enStatePath = path.join(stateRoot, 'english-image-sources.json');
  const unavailablePath = path.join(stateRoot, 'english-image-unavailable.json');
  const jpManifestPath = path.join(args.frontendRoot, 'src', 'assets', 'timeline-images', 'jp', 'manifest.json');
  const enManifestPath = path.join(args.frontendRoot, 'src', 'assets', 'timeline-images', 'en', 'manifest.json');
  const previousJpState = await readJson(jpStatePath);
  const previousEnState = await readJson(enStatePath);
  const previousUnavailable = await readJson(unavailablePath);
  const previousJpManifest = await readJson(jpManifestPath);
  const previousEnManifest = await readJson(enManifestPath);
  // Keep valid historical mappings. The resource payload and frontend deploy can
  // update independently, so pruning an older mapping here can temporarily turn
  // a cached or rolling-deploy timeline event into a broken image.
  const jpState = { ...previousJpState };
  const enState = { ...previousEnState };
  const unavailable = { ...previousUnavailable };
  const jpManifest = { ...previousJpManifest };
  const enManifest = { ...previousEnManifest };
  const changed = new Set();
  let jpCreated = 0, jpRefreshed = 0, jpCurrent = 0, enCreated = 0, enCurrent = 0, enMissing = 0, retired = 0;
  const failures = [];

  for (const [legacyTarget, job] of [...jpSources].sort(([left], [right]) => left.localeCompare(right))) {
    const target = localizedTarget(legacyTarget, 'jp');
    const key = relativeToFrontend(target);
    const sourceValue = `${job.source}#${JP_TRANSFORM_VERSION}`;
    const exists = Boolean(await imageSize(target));
    if (exists && previousJpState[key] === sourceValue && !args.refreshJp) {
      jpCurrent += 1;
    } else {
      try {
        const payload = await download(job.source);
        await saveWebp(payload, target, { maximumWidth: 512, effort: 3 });
        exists ? jpRefreshed += 1 : jpCreated += 1;
        changed.add(key);
      } catch (error) {
        failures.push(`${job.source}: ${error.message}`);
        if (!exists) continue;
      }
    }
    jpState[key] = sourceValue;
    jpManifest[publicAssetPath(legacyTarget)] = publicAssetPath(target);
    if (args.requestIntervalMs) await sleep(args.requestIntervalMs);
  }

  const enEntries = [...enSources].sort(([left], [right]) => left.localeCompare(right));
  let nextEnIndex = 0;
  const syncEnglishWorker = async () => {
    while (nextEnIndex < enEntries.length) {
      const [legacyTarget, rawJob] = enEntries[nextEnIndex++];
    const jpTarget = localizedTarget(legacyTarget, 'jp');
    const target = localizedTarget(legacyTarget, 'en');
    const key = relativeToFrontend(target);
    let source;
    try {
      source = await preferredEnglishSource(rawJob.source, rawJob.eventType);
    } catch (error) {
      failures.push(`${rawJob.source}: ${error.message}`);
      continue;
    }
    const sourceValue = `${source}#${EN_TRANSFORM_VERSION}`;
    const exists = Boolean(await imageSize(target));
    if (exists && previousEnState[key] === sourceValue) {
      enCurrent += 1;
      enState[key] = sourceValue;
      enManifest[publicAssetPath(legacyTarget)] = publicAssetPath(target);
      continue;
    }
    if (previousUnavailable[key] === source) {
      unavailable[key] = source;
      retired += 1;
      continue;
    }
    try {
      const payload = await download(source, true);
      if (!payload) {
        const timestamp = Number(source.match(/_L(\d+)\./)?.[1]);
        if (timestamp && Date.now() / 1000 > timestamp + EN_RETRY_DAYS * 86_400) {
          unavailable[key] = source;
          retired += 1;
        } else {
          enMissing += 1;
        }
      } else {
        const jpSize = await imageSize(jpTarget) ?? await imageSize(legacyTarget) ?? [512, 200];
        await saveWebp(payload, target, { targetSize: jpSize });
        enState[key] = sourceValue;
        enManifest[publicAssetPath(legacyTarget)] = publicAssetPath(target);
        enCreated += 1;
        changed.add(key);
      }
    } catch (error) {
      failures.push(`${source}: ${error.message}`);
      if (exists) {
        enState[key] = previousEnState[key];
        enManifest[publicAssetPath(legacyTarget)] = publicAssetPath(target);
      }
    }
    if (args.requestIntervalMs) await sleep(args.requestIntervalMs);
    }
  };
  await Promise.all(Array.from({ length: Math.min(8, enEntries.length) }, syncEnglishWorker));

  if (officialError) Object.assign(enManifest, previousEnManifest);
  const sortedJpState = sortedObject(jpState);
  const sortedEnState = sortedObject(enState);
  const sortedUnavailable = sortedObject(unavailable);
  if (JSON.stringify(sortedJpState) !== JSON.stringify(sortedObject(previousJpState))) { await writeJson(jpStatePath, sortedJpState); changed.add(relativeToFrontend(jpStatePath)); }
  if (JSON.stringify(sortedEnState) !== JSON.stringify(sortedObject(previousEnState))) { await writeJson(enStatePath, sortedEnState); changed.add(relativeToFrontend(enStatePath)); }
  if (JSON.stringify(sortedUnavailable) !== JSON.stringify(sortedObject(previousUnavailable))) { await writeJson(unavailablePath, sortedUnavailable); changed.add(relativeToFrontend(unavailablePath)); }
  const sortedJpManifest = sortedObject(jpManifest);
  const sortedEnManifest = sortedObject(enManifest);
  if (JSON.stringify(sortedJpManifest) !== JSON.stringify(sortedObject(previousJpManifest))) {
    await writeJson(jpManifestPath, sortedJpManifest);
    changed.add(relativeToFrontend(jpManifestPath));
  }
  if (JSON.stringify(sortedEnManifest) !== JSON.stringify(sortedObject(previousEnManifest))) {
    await writeJson(enManifestPath, sortedEnManifest);
    changed.add(relativeToFrontend(enManifestPath));
  }
  const referencedAssets = [
    ...(Array.isArray(timeline.events) ? timeline.events : []),
    ...(Array.isArray(timeline.anniversaries) ? timeline.anniversaries : [])
  ];
  const missingReferencedAssets = [];
  for (const item of referencedAssets) {
    const source = item?.image_path;
    if (typeof source !== 'string' || /^https?:/i.test(source)) continue;
    const resolved = sortedEnManifest[source] ?? sortedJpManifest[source] ?? source;
    const target = path.join(args.frontendRoot, 'src', resolved);
    if (!await imageSize(target)) missingReferencedAssets.push(`${item.id ?? item.label ?? 'unknown'}: ${resolved}`);
  }
  if (missingReferencedAssets.length) {
    failures.push(
      `${missingReferencedAssets.length} timeline image reference(s) have no usable frontend asset: ` +
      missingReferencedAssets.slice(0, 20).join(', ')
    );
  }
  if (args.changedFilesOutput) {
    await fs.mkdir(path.dirname(path.resolve(args.changedFilesOutput)), { recursive: true });
    await fs.writeFile(path.resolve(args.changedFilesOutput), [...changed].sort().map(value => `${value}\n`).join(''));
  }
  console.log(`Official JP timeline images: ${jpCreated} created, ${jpRefreshed} refreshed, ${jpCurrent} already current, ${jpSources.size} matched, ${failures.length} total failures`);
  console.log(`English timeline images: ${enCreated} created/refreshed, ${enCurrent} already English, ${enMissing} not available yet, ${retired} retired misses`);
  if (!officialError) console.log(`Official EN audit: ${officialNews.length} announcements`);
  else console.warn(`warning: official EN audit unavailable: ${officialError.message}`);
  for (const failure of failures) console.warn(`warning: ${failure}`);
  process.exitCode = failures.length ? 1 : 0;
}

await main();
