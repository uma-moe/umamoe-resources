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
  FROM gacha_data d JOIN gacha_available a ON a.gacha_id=d.id AND (a.is_pickup=1 OR (d.cost_type=92 AND d.type<>14 AND a.rarity=3))
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
// Step-up card IDs are selectable slots, not real cards.
for (const row of db.prepare(`SELECT s.stepup_id, s.target_gacha_id, s.stepup_step, d.card_type,
  d.cost_single, d.draw_limit, d.draw_guarantee_rarity, d.draw_guarantee_type
  FROM gacha_stepup s JOIN gacha_data d ON d.id=s.target_gacha_id
  WHERE d.type=14 AND d.cost_type=92 ORDER BY s.stepup_id, s.stepup_step`).all()) {
  const gacha = catalog[row.stepup_id] ??= { card_type: row.card_type, gacha_type: 14,
    cost: 0, spark_pulls: 0, pickups: [], featured_pickups: [], rarity_rates: [],
    step_up: { rounds: row.draw_limit, steps: [] } };
  gacha.step_up.steps.push({ gacha_id: row.target_gacha_id, pulls: 10, cost: row.cost_single * 10,
    guaranteed_rarity: row.draw_guarantee_rarity, selectable: row.draw_guarantee_type === 1 });
}

for (const row of db.prepare('SELECT id, card_type, type, cost_single, draw_limit, draw_guarantee_rarity, draw_guarantee_num FROM gacha_data WHERE cost_type=92').all()) {
  const gacha = catalog[row.id] ??= { card_type: row.card_type, gacha_type: row.type,
    cost: row.cost_single, spark_pulls: 0, pickups: [], featured_pickups: [], rarity_rates: [] };
  gacha.paid_draw = { limit: row.draw_limit, guaranteed_rarity: row.draw_guarantee_rarity, guaranteed_count: row.draw_guarantee_num };
}
for (const row of db.prepare('SELECT gacha_id, COUNT(*) AS slots, MIN(odds) AS odds, MAX(odds) AS max_odds FROM gacha_available WHERE rarity=3 GROUP BY gacha_id').all()) {
  const stepUp = catalog[row.gacha_id]?.step_up;
  if (stepUp && row.odds === row.max_odds) {
    stepUp.selection_pool_size = row.slots;
    stepUp.selection_pickup_rate = row.odds / 1_000_000;
  }
}
db.close();
if (!Object.keys(catalog).length) throw new Error('No published gacha rates found');
fs.writeFileSync(outputPath, '{\n' + Object.entries(catalog).map(([id, entry]) => `  "${id}": ${JSON.stringify(entry)}`).join(',\n') + '\n}\n');
console.log(`Wrote published rates for ${Object.keys(catalog).length} JP banners to ${outputPath}`);
