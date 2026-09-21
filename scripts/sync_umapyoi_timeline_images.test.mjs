import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { officialEnglishJobs } from './sync_umapyoi_timeline_images.mjs';

const archive = JSON.parse(readFileSync(new URL('../src/jp_data/umapyoi_archive.json', import.meta.url)));
const posts = JSON.parse(readFileSync(new URL('../src/global_data/official_news_archive.json', import.meta.url)))
  .posts.map(post => post.snapshots.at(-1).raw);
const frontend = path.resolve('test-frontend');
const jobsFor = events => officialEnglishJobs({ events }, archive, posts, [], frontend);
const target = event => path.join(frontend, 'src', event.image_path);
const story = {
  id: 'story-event-10_intertwined_memories_banner', type: 'story_event', is_confirmed: true,
  jp_release_date: '2022-10-28T03:00:00Z', global_release_date: '2026-09-07T22:00:00Z',
  image_path: 'assets/images/story/10_intertwined_memories_banner.webp'
};

test('story artwork uses the main news banner, not an earlier reward-section image', () => {
  const noGlobalDate = { ...story, global_release_date: undefined };
  assert.equal(jobsFor([noGlobalDate]).get(target(story))?.source, posts.find(post => post.announce_id === 1023).image);
});

test('story launch notices handle regional banner IDs and replace preview artwork', () => {
  const summer = { ...story, id: 'summer', jp_release_date: '2021-07-29T03:00:00Z',
    global_release_date: '2025-10-14T22:00:00Z', image_path: 'assets/images/story/summer.webp' };
  const halloween = { ...story, id: 'halloween', jp_release_date: '2021-09-29T03:00:00Z',
    global_release_date: '2025-11-24T22:00:00Z', image_path: 'assets/images/story/halloween.webp',
    umapyoi_url: 'https://umapyoi.net/en/news/427' };
  const jobs = jobsFor([summer, halloween]);
  assert.equal(jobs.get(target(summer))?.source, posts.find(post => post.announce_id === 332).image);
  assert.equal(jobs.get(target(halloween))?.source, posts.find(post => post.announce_id === 428).image);
});

test('linked English news takes precedence over stale image identities', () => {
  const linked = { ...story, umapyoi_url: 'https://umapyoi.net/en/news/1023',
    image: 'https://prd-info-umamusume.akamaized.net/announce/973/Thumbnail/banner_30100034.png' };
  assert.equal(jobsFor([linked]).get(target(linked))?.source, posts.find(post => post.announce_id === 1023).image);
});

test('celebration posts cover each mission in their phase and use Unix-second dates', () => {
  const events = ['2026-09-10', '2026-09-23', '2026-09-29', '2026-10-01', '2026-10-08'].map((date, index) => ({
    id: `campaign-${199 + index}`, type: 'campaign', is_confirmed: true,
    title: 'Fall G1 Celebration Missions, Part 2: JBC Series',
    global_release_date: `${date}T15:00:00Z`, image_path: `assets/images/campaign/${199 + index}.webp`
  }));
  const future = { ...events[0], global_release_date: '2027-09-10T15:00:00Z', image_path: 'assets/images/campaign/future.webp' };
  const otherPhase = { ...events[0], title: 'Spring G1 Celebration Missions, Part 1', image_path: 'assets/images/campaign/other.webp' };
  const post = posts.find(post => post.announce_id === 1001);
  const jobs = officialEnglishJobs({ events: [...events, future, otherPhase] }, archive, [post], [], frontend);
  for (const event of events) assert.equal(jobs.get(target(event))?.source, post.image);
  assert.equal(jobs.has(target(future)), false);
  assert.equal(jobs.has(target(otherPhase)), false);
  const unrelated = { ...post, title: 'A new story event is here!', message: 'Clear event missions for rewards.' };
  assert.equal(officialEnglishJobs({ events: [events[0]] }, archive, [unrelated], [], frontend).size, 0);
});
