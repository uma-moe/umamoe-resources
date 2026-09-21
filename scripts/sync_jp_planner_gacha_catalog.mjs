#!/usr/bin/env node
import fs from 'node:fs';
import { DatabaseSync } from 'node:sqlite';

const [masterPath, outputPath] = process.argv.slice(2);
if (!masterPath || !outputPath) throw new Error('Usage: node scripts/sync_jp_planner_gacha_catalog.mjs <jp-master.mdb> <output.json> (Node 22+)');
const db = new DatabaseSync(masterPath, { readOnly: true });
const catalog = {};
const rows = db.prepare(`SELECT d.id, d.card_type, d.type, d.cost_single,
  a.card_id, a.rarity, a.odds,
  COALESCE((SELECT MIN(e.pay_item_num) FROM gacha_exchange e
    WHERE e.gacha_id=d.id AND e.card_id=a.card_id), 0) AS spark
  FROM gacha_data d JOIN gacha_available a ON a.gacha_id=d.id AND a.is_pickup=1
  ORDER BY d.id, a.card_id`).all();
for (const row of rows) {
  const gacha = catalog[row.id] ??= { card_type: row.card_type, gacha_type: row.type,
    cost: row.cost_single, spark_pulls: 0, pickups: [], featured_pickups: [], rarity_rates: [] };
  gacha.spark_pulls = Math.max(gacha.spark_pulls, row.spark);
  const pickup = { pickup_id: row.card_id, label: `${row.card_type === 1 ? 'Umamusume' : 'Support Card'} ${row.card_id}`,
    rate: row.odds / 1_000_000, exchangeable: row.spark > 0 };
  gacha.featured_pickups.push(pickup);
  if (row.rarity === 3) gacha.pickups.push(pickup);
}
for (const row of db.prepare('SELECT gacha_id, rarity, SUM(odds) AS odds FROM gacha_available GROUP BY gacha_id, rarity ORDER BY gacha_id, rarity DESC').all()) {
  catalog[row.gacha_id]?.rarity_rates.push({ rarity: row.rarity, rate: row.odds / 1_000_000 });
}
db.close();
if (!Object.keys(catalog).length) throw new Error('No published gacha rates found');
fs.writeFileSync(outputPath, '{\n' + Object.entries(catalog).map(([id, entry]) => `  "${id}": ${JSON.stringify(entry)}`).join(',\n') + '\n}\n');
console.log(`Wrote published rates for ${Object.keys(catalog).length} JP banners to ${outputPath}`);
