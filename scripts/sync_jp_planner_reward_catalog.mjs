#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import zlib from 'node:zlib';

const [inputPath, outputPath] = process.argv.slice(2);
if (!inputPath || !outputPath) {
  console.error(
    'Usage: node scripts/sync_jp_planner_reward_catalog.mjs <planner_rewards.json[.gz]> <output.json>',
  );
  process.exit(2);
}

const input = fs.readFileSync(inputPath);
const json = inputPath.endsWith('.gz') ? zlib.gunzipSync(input) : input;
const document = JSON.parse(json.toString('utf8'));
if (!Array.isArray(document.rewards)) {
  throw new Error(`Expected a rewards array in ${inputPath}`);
}

const rewards = document.rewards
  .filter((reward) => reward.provenance === 'jp_master')
  .map(
    ({
      id,
      label,
      event_id,
      gacha_id,
      currency,
      amount,
      available_at,
      assumption,
      default_enabled,
      source_url,
      source_items,
      evidence,
    }) => ({
      id,
      label,
      ...(event_id == null ? {} : { event_id }),
      ...(gacha_id == null ? {} : { gacha_id }),
      currency,
      amount,
      available_at,
      assumption,
      default_enabled,
      ...(source_url == null ? {} : { source_url }),
      ...(source_items?.length ? { source_items } : {}),
      ...(evidence == null ? {} : { evidence }),
    }),
  )
  .sort((left, right) =>
    `${left.available_at}\0${left.id}`.localeCompare(
      `${right.available_at}\0${right.id}`,
    ),
  );

if (rewards.length === 0) {
  throw new Error(`No jp_master rewards found in ${inputPath}`);
}

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(rewards, null, 2)}\n`);
console.log(`Wrote ${rewards.length} JP planner reward rows to ${outputPath}`);
