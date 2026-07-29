use anyhow::{anyhow, bail, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tracing::warn;

const SCHEMA_VERSION: u32 = 4;
const RACE_PARAMETERS_SCHEMA_VERSION: u32 = 22;
const START_DELAY_MAX_SECONDS: f64 = 0.1;
const TARGET_SPEED_MIN: f64 = 13.0;
// Current Global ast_race_paramdefine base-speed and
// HorseBaseTargetSpeedCalculator inputs.
const RACE_BASE_SPEED: f64 = 20.0;
const RACE_BASE_DISTANCE_M: f64 = 2_000.0;
const RACE_BASE_DISTANCE_RATE_M: f64 = 1_000.0;
const PHASE_TARGET_SPEED_PERCENT: [[f64; 4]; 6] = [
    [0.0; 4],
    [100.0, 98.0, 96.2_f32 as f64, 96.2_f32 as f64],
    [97.8_f32 as f64, 99.1_f32 as f64, 97.5, 97.5],
    [
        93.8_f32 as f64,
        99.8_f32 as f64,
        99.4_f32 as f64,
        99.4_f32 as f64,
    ],
    [93.1_f32 as f64, 100.0, 100.0, 100.0],
    [106.3_f32 as f64, 96.2_f32 as f64, 95.0, 95.0],
];
const BASE_TARGET_SPEED_RANDOM_MINUS_BASE_PERCENT: f64 = -0.65_f32 as f64;
const BASE_TARGET_SPEED_RANDOM_MINUS_WISDOM_DIVISOR: f64 = 5_500.0;
const BASE_TARGET_SPEED_RANDOM_PLUS_WISDOM_DIVISOR: f64 = 5_500.0;
const PHASE_END_SPEED_SQRT_COEFFICIENT: f64 = 500.0;
const PHASE_END_SPEED_SCALE: f64 = 0.002_f32 as f64;
// Current Global ast_race_paramdefine HorseAccelCalculator inputs. Preserve
// the serialized f32 payload and source strategy indexing.
const ACCEL_POWER_COEFFICIENT: f64 = 0.0006_f32 as f64;
const ACCEL_UPHILL_POWER_COEFFICIENT: f64 = 0.0004_f32 as f64;
const ACCEL_POWER_SQRT_COEFFICIENT: f64 = 500.0;
const ACCEL_START_DASH_ADD: f64 = 24.0;
const ACCEL_PHASE_COEFFICIENTS: [[f64; 3]; 6] = [
    [0.0; 3],
    [1.0, 1.0, 0.996_f32 as f64],
    [0.985_f32 as f64, 1.0, 0.996_f32 as f64],
    [0.975_f32 as f64, 1.0, 1.0],
    [0.945_f32 as f64, 1.0, 0.997_f32 as f64],
    [1.17_f32 as f64, 0.94_f32 as f64, 0.956_f32 as f64],
];
const NEAR_HORSE_DISTANCE_THRESHOLD_M: f64 = 3.0;
const NEAR_HORSE_LANE_DISTANCE_THRESHOLD_M: f64 = 1.875;
const EXTRA_MOVE_START_FINAL_CORNER_RATE: f64 = 0.0;
const EXTRA_MOVE_LANE_NORMALIZE_COURSE_WIDTHS: f64 = 0.1;
const EXTRA_MOVE_LANE_COEFFICIENT_COURSE_WIDTHS: f64 = 0.5;
const EXTRA_MOVE_LANE_RANDOM_MAX_COURSE_WIDTHS: f64 = 0.1;
// Current JP ast_race_paramdefine serialized f32 values. Global's
// HorseRaceInfo.MoveLane reads the corresponding RaceParamDefine fields.
const LANE_MOVE_SPEED_BASE: f64 = 0.019_999_999_552_965_164;
const LANE_MOVE_SPEED_ADD: f64 = 0.300_000_011_920_928_96;
const LANE_MOVE_SPEED_POWER_COEFFICIENT: f64 = 0.001_000_000_047_497_451_3;
const LANE_MOVE_SPEED_OUTSIDE_POSITION_COEFFICIENT: f64 = 0.050_000_000_745_058_06;
const LANE_MOVE_ACCELERATION_BASE_COEFFICIENT: f64 = 1.5;
const OVERTAKE_FINAL_IN_LANE_COEFFICIENT: f64 = 1.0;
const OVERTAKE_FINAL_OUT_LANE_COEFFICIENT: f64 = 1.15;
// JP ast_race_paramdefine MonoBehaviour fields overTakeCoolDownTime and
// overTakeCountCoolDownTime. Global constructs the same manager subsystem.
const OVERTAKE_TARGET_COOLDOWN_SECONDS: f64 = 1.5;
const ORDER_CHANGE_COUNT_COOLDOWN_SECONDS: f64 = 6.0;
// Current JP ast_race_paramdefine serialized f32 values. Global retains the
// same Block, Surrounded, and congestion runtime contract.
const BLOCK_FRONT_DISTANCE_M: f64 = 2.0;
const BLOCK_FRONT_LANE_COURSE_WIDTHS: f64 = 0.041_666_671_633_720_4;
const BLOCK_SIDE_DISTANCE_M: f64 = 1.049_999_952_316_284_2;
const BLOCK_SIDE_LANE_COURSE_WIDTHS: f64 = 0.111_111_097_037_792_2;
const BLOCK_FRONT_MIN_SPEED_RATE: f64 = 0.987_999_975_681_304_9;
const BLOCK_FRONT_MAX_SPEED_RATE: f64 = 1.0;
const SURROUNDED_OUTSIDE_DISTANCE_M: f64 = 1.5;
const SURROUNDED_OUTSIDE_LANE_COURSE_WIDTHS: f64 = 0.166_659_995_913_505_55;
const SURROUNDED_DIRECTIONAL_DISTANCE_M: f64 = 3.0;
const SURROUNDED_DIRECTIONAL_LANE_COURSE_WIDTHS: f64 = 0.083_329_997_956_752_78;
const CONGESTION_LANE_COURSE_WIDTHS: f64 = 0.300_000_011_920_928_96;
const CONGESTION_HORSE_COUNT_THRESHOLD: u32 = 3;
const SKILL_INFRONT_HORSE_NEAR_DISTANCE_M: f64 = 2.5;
const SKILL_BEHIND_HORSE_NEAR_DISTANCE_M: f64 = 2.5;
const SKILL_INFRONT_HORSE_NEAR_LANE_COURSE_WIDTHS: f64 = 0.055_599_998_682_737_35;
const SKILL_BEHIND_HORSE_NEAR_LANE_COURSE_WIDTHS: f64 = 0.055_599_998_682_737_35;
const SKILL_BEHIND_NEAR_PARAMETER_SETS: &[SimulatorNearLaneParameters] =
    &[SimulatorNearLaneParameters {
        distance_m: 5.0,
        lane_course_widths: 0.150_000_005_960_464_48,
    }];
const DECELERATION_BASE: f64 = 1.0;
const DECELERATION_PHASE_RATES: [f64; 3] = [1.2, 0.8, 1.0];
const DECELERATION_HP_ZERO_RATE: f64 = 1.2;
const DECELERATION_PACE_DOWN_RATE: f64 = 0.5;
const MINIMUM_SPEED_START_SPEED: f64 = 3.0;
const MINIMUM_SPEED_BASE_SPEED_RATE: f64 = 0.850_000_023_841_857_9;
const MINIMUM_SPEED_GUTS_SQRT_COEFFICIENT: f64 = 200.0;
const MINIMUM_SPEED_GUTS_COEFFICIENT: f64 = 0.001_000_000_047_497_451_3;
const HP_INITIAL_STAMINA_COEFFICIENT: f64 = 0.8;
const HP_MAX_STRATEGY_COEFFICIENTS: [f64; 6] = [0.0, 0.95, 0.89, 1.0, 0.995, 0.86];
const HP_CONSUMPTION_BASE: f64 = 20.0;
const HP_SPEED_GAP_OFFSET: f64 = 12.0;
const HP_SPEED_GAP_SQUARE_DIVISOR: f64 = 144.0;
const HP_NORMAL_RATE: f64 = 1.0;
const HP_RUSHED_RATE: f64 = 1.6;
const HP_PACE_DOWN_RATE: f64 = 0.6;
const HP_LEAD_COMPETITION_NIGE_RATE: f64 = 1.4;
const HP_RUSHED_LEAD_COMPETITION_NIGE_RATE: f64 = 3.6;
const HP_LEAD_COMPETITION_OONIGE_RATE: f64 = 3.5;
const HP_RUSHED_LEAD_COMPETITION_OONIGE_RATE: f64 = 7.7;
const HP_DOWNHILL_RATE: f64 = 0.4;
const HP_GUTS_COEFFICIENT: f64 = 200.0;
const HP_GUTS_SQRT_COEFFICIENT: f64 = 600.0;
const HP_GROUND_CONDITION_RATES: [[f64; 5]; 3] = [
    [0.0; 5],
    [0.0, 1.0, 1.0, 1.02, 1.02],
    [0.0, 1.0, 1.0, 1.01, 1.02],
];
// Current Global ast_race_paramdefine Temptation block. JP retains the same
// serialized field layout; runtime arithmetic is performed as `float`.
const TEMPTATION_LOT_SECTION_MIN: u8 = 2;
const TEMPTATION_LOT_SECTION_MAX: u8 = 9;
const TEMPTATION_START_PERCENT_VALUE1: f64 = 13.0;
const TEMPTATION_FORCE_END_SECONDS: f64 = 12.0;
const TEMPTATION_END_CHECK_SECONDS: f64 = 3.0;
const TEMPTATION_END_CHANCE_PERCENT: f64 = 55.0;
const TEMPTATION_OIKOMI_TO_SASHI_PERCENT: f64 = 10.0;
const TEMPTATION_OIKOMI_TO_SENKO_PERCENT: f64 = 20.0;
const TEMPTATION_OIKOMI_TO_NIGE_PERCENT: f64 = 70.0;
const TEMPTATION_SASHI_TO_SENKO_PERCENT: f64 = 25.0;
const TEMPTATION_SASHI_TO_NIGE_PERCENT: f64 = 75.0;
const TEMPTATION_SENKO_TO_NIGE_PERCENT: f64 = 100.0;
// Current Global AdditionalActivateAbilityMaxCountArray, indexed by type.
const ADDITIONAL_ACTIVATION_ORDER_UP_MAX_COUNT: u8 = 3;
const ADDITIONAL_ACTIVATION_ANY_SKILL_TYPE_2_MAX_COUNT: u8 = 3;
const ADDITIONAL_ACTIVATION_ANY_SKILL_TYPE_3_MAX_COUNT: u8 = 2;
// Current Global RaceParam/SkillParam fields used by SkillDetail construction
// and SkillAbilityTimeIncrementOrderUp.UpdateAbilityTime.
const SKILL_ABILITY_TIME_DIVIDE_DISTANCE_M: u32 = 1000;
const SKILL_COOLDOWN_TIME_DIVIDE_DISTANCE_M: u32 = 1000;
const SKILL_ORDER_UP_ADD_ABILITY_TIME_SECONDS: f64 = 1.0;
const SKILL_ORDER_UP_ADD_ABILITY_TIME_MAX_COUNT: u8 = 3;
// Current Global SkillParam dynamic ability-time calculator fields. Global
// evaluates these values as float and selects the first strict upper bound.
const SKILL_DURATION_DISTANCE_DIFF_TOP_DIVISOR_M: f64 = 62.5;
const SKILL_DURATION_DISTANCE_DIFF_TOP_MIN_MULTIPLIER: f64 = 0.800_000_011_920_929;
const SKILL_DURATION_DISTANCE_DIFF_TOP_MAX_MULTIPLIER: f64 = 1.600_000_023_841_858;
const SKILL_DURATION_REMAIN_HP_TYPE_1: [SimulatorSkillDurationThreshold; 8] = [
    SimulatorSkillDurationThreshold::new(2_000.0, 1.0),
    SimulatorSkillDurationThreshold::new(2_400.0, 1.5),
    SimulatorSkillDurationThreshold::new(2_600.0, 2.0),
    SimulatorSkillDurationThreshold::new(2_800.0, 2.200_000_047_683_716),
    SimulatorSkillDurationThreshold::new(3_000.0, 2.5),
    SimulatorSkillDurationThreshold::new(3_200.0, 3.0),
    SimulatorSkillDurationThreshold::new(3_500.0, 3.5),
    SimulatorSkillDurationThreshold::new(99_999.0, 4.0),
];
const SKILL_DURATION_REMAIN_HP_TYPE_2: [SimulatorSkillDurationThreshold; 5] = [
    SimulatorSkillDurationThreshold::new(1_500.0, 1.0),
    SimulatorSkillDurationThreshold::new(1_800.0, 1.5),
    SimulatorSkillDurationThreshold::new(2_000.0, 2.0),
    SimulatorSkillDurationThreshold::new(2_100.0, 2.5),
    SimulatorSkillDurationThreshold::new(99_999.0, 3.0),
];
const SKILL_DURATION_BLOCKED_TIME_TYPE_1: [SimulatorSkillDurationThreshold; 4] = [
    SimulatorSkillDurationThreshold::new(2.0, 1.0),
    SimulatorSkillDurationThreshold::new(4.0, 2.0),
    SimulatorSkillDurationThreshold::new(6.0, 3.0),
    SimulatorSkillDurationThreshold::new(999.0, 4.0),
];
const SKILL_DURATION_BLOCKED_TIME_TYPE_2: [SimulatorSkillDurationThreshold; 0] = [];
// Current Global SkillParam ability-value calculator block for scaling codes
// 2..=23. Values preserve the serialized f32 payload exactly.
const SKILL_VALUE_ACQUIRED_SKILL_EXCLUDED_COUNT: u16 = 1;
const SKILL_VALUE_ACQUIRED_SKILL_BASE_MULTIPLIER: f64 = 1.0;
const SKILL_VALUE_ACQUIRED_SKILL_PER_COUNT_MULTIPLIER: f64 = 0.009_999_999_776_482_582;
const SKILL_VALUE_ACQUIRED_SKILL_MAX_MULTIPLIER: f64 = 1.200_000_047_683_715_8;
const SKILL_VALUE_AOHARU_TEAM_TOTAL_STATUS: [SimulatorSkillValueThreshold; 5] = [
    SimulatorSkillValueThreshold::new(1_200.0, 0.800_000_011_920_929),
    SimulatorSkillValueThreshold::new(1_800.0, 0.899_999_976_158_142_1),
    SimulatorSkillValueThreshold::new(2_600.0, 1.0),
    SimulatorSkillValueThreshold::new(3_600.0, 1.100_000_023_841_858),
    SimulatorSkillValueThreshold::new(99_999.0, 1.200_000_047_683_715_8),
];
const SKILL_VALUE_RANDOM_ABILITY_TYPE_1: [SimulatorSkillValueRandomRow; 3] = [
    SimulatorSkillValueRandomRow::new(0.0, 60),
    SimulatorSkillValueRandomRow::new(0.019_999_999_552_965_164, 30),
    SimulatorSkillValueRandomRow::new(0.039_999_999_105_930_33, 10),
];
const SKILL_VALUE_RANDOM_ABILITY_TYPE_2: [SimulatorSkillValueRandomRow; 3] =
    SKILL_VALUE_RANDOM_ABILITY_TYPE_1;
const SKILL_VALUE_RACE_WINS: [SimulatorSkillValueThreshold; 5] = [
    SimulatorSkillValueThreshold::new(6.0, 0.800_000_011_920_929),
    SimulatorSkillValueThreshold::new(14.0, 0.899_999_976_158_142_1),
    SimulatorSkillValueThreshold::new(18.0, 1.0),
    SimulatorSkillValueThreshold::new(25.0, 1.100_000_023_841_858),
    SimulatorSkillValueThreshold::new(99_999.0, 1.200_000_047_683_715_8),
];
const SKILL_VALUE_ORDER_UP_COUNT_CORNER_PHASE_END_AFTER: [SimulatorSkillValueThreshold; 4] = [
    SimulatorSkillValueThreshold::new(2.0, 1.0),
    SimulatorSkillValueThreshold::new(3.0, 1.100_000_023_841_858),
    SimulatorSkillValueThreshold::new(4.0, 1.200_000_047_683_715_8),
    SimulatorSkillValueThreshold::new(99.0, 1.25),
];
const SKILL_VALUE_FAN_COUNT: [SimulatorSkillValueThreshold; 5] = [
    SimulatorSkillValueThreshold::new(20_000.0, 0.800_000_011_920_929),
    SimulatorSkillValueThreshold::new(50_000.0, 0.899_999_976_158_142_1),
    SimulatorSkillValueThreshold::new(100_000.0, 1.0),
    SimulatorSkillValueThreshold::new(160_000.0, 1.100_000_023_841_858),
    SimulatorSkillValueThreshold::new(99_999_999.0, 1.200_000_047_683_715_8),
];
const SKILL_VALUE_MAX_RAW_STAT: [SimulatorSkillValueThreshold; 5] = [
    SimulatorSkillValueThreshold::new(600.0, 0.800_000_011_920_929),
    SimulatorSkillValueThreshold::new(800.0, 0.899_999_976_158_142_1),
    SimulatorSkillValueThreshold::new(1_000.0, 1.0),
    SimulatorSkillValueThreshold::new(1_100.0, 1.100_000_023_841_858),
    SimulatorSkillValueThreshold::new(9_999.0, 1.200_000_047_683_715_8),
];
const SKILL_VALUE_ACTIVATED_PASSIVE_SKILL_COUNT: [SimulatorSkillValueThreshold; 4] = [
    SimulatorSkillValueThreshold::new(3.0, 0.0),
    SimulatorSkillValueThreshold::new(5.0, 1.0),
    SimulatorSkillValueThreshold::new(6.0, 2.0),
    SimulatorSkillValueThreshold::new(99.0, 3.0),
];
const SKILL_VALUE_HEAL_BASE_MULTIPLIER: f64 = 1.0;
const SKILL_VALUE_HEAL_PER_ACTIVATION_MULTIPLIER: f64 = 0.100_000_001_490_116_12;
const SKILL_VALUE_HEAL_MAX_MULTIPLIER: f64 = 1.5;
const SKILL_VALUE_FINAL_CORNER_BASE_MULTIPLIER: f64 = 1.200_000_047_683_715_8;
const SKILL_VALUE_FINAL_CORNER_PER_ORDER_SUBTRACT: f64 = 0.019_999_999_552_965_164;
const SKILL_VALUE_FINAL_CORNER_MIN_MULTIPLIER: f64 = 1.0;
const SKILL_VALUE_INVERSE_TEAM_MEMBER_NUMERATOR: f64 = 1.0;
const SKILL_VALUE_BASE_WISDOM: [SimulatorSkillValueThreshold; 6] = [
    SimulatorSkillValueThreshold::new(500.0, 0.5),
    SimulatorSkillValueThreshold::new(800.0, 1.0),
    SimulatorSkillValueThreshold::new(1_000.0, 1.100_000_023_841_858),
    SimulatorSkillValueThreshold::new(1_200.0, 1.200_000_047_683_715_8),
    SimulatorSkillValueThreshold::new(1_300.0, 1.299_999_952_316_284_2),
    SimulatorSkillValueThreshold::new(9_999.0, 1.399_999_976_158_142),
];
const SKILL_VALUE_DISTANCE_DIFF_TOP_ADDITIVE: [SimulatorSkillValueAdditiveThreshold; 2] = [
    SimulatorSkillValueAdditiveThreshold::new(20.0, 0.0),
    SimulatorSkillValueAdditiveThreshold::new(9_999.0, 0.100_000_001_490_116_12),
];
const SKILL_VALUE_BLOCKED_TIME_TYPE_1: [SimulatorSkillValueThreshold; 4] = [
    SimulatorSkillValueThreshold::new(2.0, 1.0),
    SimulatorSkillValueThreshold::new(4.0, 2.0),
    SimulatorSkillValueThreshold::new(6.0, 3.0),
    SimulatorSkillValueThreshold::new(999.0, 4.0),
];
const SKILL_VALUE_BLOCKED_TIME_TYPE_2: [SimulatorSkillValueThreshold; 0] = [];
const SKILL_VALUE_FINAL_SPEED_TYPE_1: [SimulatorSkillValueThreshold; 5] = [
    SimulatorSkillValueThreshold::new(1_700.0, 0.0),
    SimulatorSkillValueThreshold::new(1_800.0, 1.0),
    SimulatorSkillValueThreshold::new(1_900.0, 2.0),
    SimulatorSkillValueThreshold::new(2_000.0, 3.0),
    SimulatorSkillValueThreshold::new(9_999.0, 4.0),
];
const SKILL_VALUE_FINAL_SPEED_TYPE_2: [SimulatorSkillValueThreshold; 3] = [
    SimulatorSkillValueThreshold::new(1_400.0, 1.0),
    SimulatorSkillValueThreshold::new(1_600.0, 2.0),
    SimulatorSkillValueThreshold::new(9_999.0, 3.0),
];
const LAST_SPURT_TARGET_SPEED_SPEED_SQRT_COEFFICIENT: f64 = 500.0;
const LAST_SPURT_TARGET_SPEED_ADD_SPEED_COEFFICIENT: f64 = 0.002_000_000_094_994_902_6;
const LAST_SPURT_TARGET_SPEED_BASE_COEFFICIENT: f64 = 1.049_999_952_316_284_2;
const LAST_SPURT_TARGET_SPEED_GUTS_COEFFICIENT: f64 = 450.0;
const LAST_SPURT_TARGET_SPEED_GUTS_POWER: f64 = 0.597_000_002_861_023;
const LAST_SPURT_TARGET_SPEED_GUTS_SCALE: f64 = 0.000_099_999_997_473_787_52;
const LAST_SPURT_BASE_TARGET_SPEED_ADD_COEFFICIENT: f64 = 0.009_999_999_776_482_582;
const LAST_SPURT_DISTANCE_GOAL_BUFFER_M: f64 = 60.0;
const LAST_SPURT_START_DISTANCE_MAX_FROM_GOAL_M: f64 = 100.0;
const LAST_SPURT_INITIAL_SPEED_DELTA_MPS: f64 = 0.100_000_001_490_116_12;
const LAST_SPURT_CANDIDATE_SPEED_DELTA_MPS: f64 = 0.100_000_001_490_116_12;
const LAST_SPURT_ACCEPTANCE_BASE_PERCENT: f64 = 15.0;
const LAST_SPURT_ACCEPTANCE_WISDOM_PERCENT_COEFFICIENT: f64 = 0.050_000_000_745_058_06;
const SLOPE_CHECK_INTERVAL_SECONDS: f64 = 1.0;
const SLOPE_PER_THRESHOLD: f64 = 0.0;
const SLOPE_UPHILL_ADD_SPEED_VALUE1: f64 = 200.0;
const SLOPE_DOWNHILL_ADD_SPEED_VALUE1: f64 = 0.3;
const SLOPE_DOWNHILL_ADD_SPEED_VALUE2: f64 = 10.0;
const SLOPE_DOWNHILL_START_WISDOM_PERCENT_COEFFICIENT: f64 = 0.04;
const SLOPE_DOWNHILL_END_CHANCE_PERCENT: f64 = 20.0;
const FORCE_IN_LANE_THRESHOLD_COURSE_WIDTHS: f64 = 0.12;
const FORCE_IN_TARGET_SPEED_ADD_NIGE_MPS: f64 = 0.02;
const FORCE_IN_TARGET_SPEED_ADD_SENKO_MPS: f64 = 0.01;
const FORCE_IN_TARGET_SPEED_ADD_SASHI_MPS: f64 = 0.01;
const FORCE_IN_TARGET_SPEED_ADD_OIKOMI_MPS: f64 = 0.03;
const FORCE_IN_TARGET_SPEED_ADD_RANDOM_RANGE_MPS: f64 = 0.1;
const POSITION_KEEP_CHECK_INTERVAL_SECONDS: f64 = 2.0;
const POSITION_KEEP_COOLDOWN_SECONDS: f64 = 1.0;
const POSITION_KEEP_START_SECTION: u8 = 1;
const POSITION_KEEP_END_SECTION: u8 = 10;
const POSITION_KEEP_CONTINUE_SECTION: u8 = 1;
const POSITION_KEEP_OONIGE_CONTINUE_SECTION: u8 = 3;
const POSITION_KEEP_SPEED_UP_WISDOM_PERCENT_COEFFICIENT: f64 = 20.0;
const POSITION_KEEP_SPEED_UP_TARGET_SPEED_MULTIPLIER: f64 = 1.04;
const POSITION_KEEP_SPEED_UP_END_DISTANCE_M: f64 = 4.5;
const POSITION_KEEP_SPEED_UP_ONLY_NIGE_END_DISTANCE_M: f64 = 12.5;
const POSITION_KEEP_OVERTAKE_WISDOM_PERCENT_COEFFICIENT: f64 = 20.0;
const POSITION_KEEP_OVERTAKE_END_DISTANCE_M: f64 = 10.0;
const POSITION_KEEP_OVERTAKE_TARGET_SPEED_MULTIPLIER: f64 = 1.05;
const POSITION_KEEP_OONIGE_SPEED_UP_WISDOM_PERCENT_COEFFICIENT: f64 = 20.0;
const POSITION_KEEP_OONIGE_SPEED_UP_TARGET_SPEED_MULTIPLIER: f64 = 1.04;
const POSITION_KEEP_OONIGE_SPEED_UP_END_DISTANCE_M: f64 = 17.5;
const POSITION_KEEP_OONIGE_OVERTAKE_WISDOM_PERCENT_COEFFICIENT: f64 = 20.0;
const POSITION_KEEP_OONIGE_OVERTAKE_END_DISTANCE_M: f64 = 27.5;
const POSITION_KEEP_OONIGE_OVERTAKE_TARGET_SPEED_MULTIPLIER: f64 = 1.05;
const POSITION_KEEP_PACE_DISTANCE_DIFF_MAX_COEFFICIENT: f64 = 0.5;
const POSITION_KEEP_SENKO_MIN_DISTANCE_M: f64 = 3.0;
const POSITION_KEEP_SENKO_MAX_DISTANCE_M: f64 = 5.0;
const POSITION_KEEP_SASHI_MIN_DISTANCE_M: f64 = 6.5;
const POSITION_KEEP_SASHI_MAX_DISTANCE_M: f64 = 7.0;
const POSITION_KEEP_OIKOMI_MIN_DISTANCE_M: f64 = 7.5;
const POSITION_KEEP_OIKOMI_MAX_DISTANCE_M: f64 = 8.0;
const POSITION_KEEP_PACE_BASE_DISTANCE_M: f64 = 1_000.0;
const POSITION_KEEP_PACE_DISTANCE_COEFFICIENT: f64 = 0.0008;
const POSITION_KEEP_PACE_DOWN_TARGET_SPEED_MULTIPLIER: f64 = 0.915;
const POSITION_KEEP_PACE_DOWN_MIDDLE_TARGET_SPEED_MULTIPLIER: f64 = 0.945;
const POSITION_KEEP_PACE_DOWN_TARGET_LANE: f64 = 0.18;
const POSITION_KEEP_PACE_UP_WISDOM_PERCENT_COEFFICIENT: f64 = 15.0;
const POSITION_KEEP_PACE_UP_TARGET_SPEED_MULTIPLIER: f64 = 1.04;
const POSITION_KEEP_PACE_UP_EX_TARGET_SPEED_MULTIPLIER: f64 = 2.0;
const PACEMAKER_FORWARD_RUNNING_STYLE_RANGE_M: f64 = 10.0;
const PACEMAKER_MOST_FORWARD_STYLE_RANGE_M: f64 = 10.0;
const PACEMAKER_TOP_NOT_MOST_FORWARD_STYLE_COUNT: u32 = 2;
const CONSERVE_POWER_THRESHOLD: f64 = 1200.0;
const CONSERVE_POWER_RELEASE_POWER_COEFFICIENT: f64 = 130.0;
const CONSERVE_POWER_RELEASE_ACCELERATION_SCALE: f64 = 0.001;
const CONSERVE_POWER_RELEASE_BASE_DURATION_SECONDS: f64 = 3.0;
const CONSERVE_POWER_ACTIVITY_TIME_COEFFICIENT: f64 = 1450.0;
const CONSERVE_POWER_STRATEGY_DISTANCE_COEFFICIENTS: [[f64; 4]; 5] = [
    [1.0, 1.0, 1.0, 1.0],
    [0.7, 0.8, 0.9, 0.9],
    [0.75, 0.7, 0.875, 1.0],
    [0.7, 0.75, 0.86, 0.9],
    [1.0, 1.0, 1.0, 1.0],
];
const CONSERVE_POWER_ACTIVITY_STATE_DELTAS: [f64; 4] = [6.7, 4.2, -0.95, -0.8];
const CONSERVE_POWER_ACTIVITY_ACCELERATION_COEFFICIENTS: [f64; 4] = [1.0, 1.0, 0.98, 0.8];
const CONSERVE_POWER_DURATION_DISTANCE_COEFFICIENTS: [f64; 4] = [0.45, 1.0, 0.875, 0.8];
const COMPETE_FIGHT_TARGET_DISTANCE_M: f64 = 3.0;
const COMPETE_FIGHT_TARGET_LANE_COURSE_WIDTHS: f64 = 0.25;
const COMPETE_FIGHT_TARGET_CONTINUE_SECONDS: f64 = 2.0;
const COMPETE_FIGHT_TARGET_SPEED_GAP_MPS: f64 = 0.6_f32 as f64;
const COMPETE_FIGHT_TARGET_ORDER_PERCENT: u8 = 50;
const COMPETE_FIGHT_EXIT_DISTANCE_M: f64 = 5.0;
const COMPETE_FIGHT_END_HP_PERCENT: u8 = 5;
const COMPETE_FIGHT_START_HP_PERCENT: u8 = 15;
const COMPETE_FIGHT_TARGET_SPEED_BASE: f64 = 200.0;
const COMPETE_FIGHT_TARGET_SPEED_POWER: f64 = 0.708_f32 as f64;
const COMPETE_FIGHT_TARGET_SPEED_SCALE: f64 = 0.0001_f32 as f64;
const COMPETE_FIGHT_ACCELERATION_BASE: f64 = 160.0;
const COMPETE_FIGHT_ACCELERATION_POWER: f64 = 0.59_f32 as f64;
const COMPETE_FIGHT_ACCELERATION_SCALE: f64 = 0.0001_f32 as f64;
const RACE_PARAMETERS_PROVENANCE: &str =
    "current JP ast_race_paramdefine startDelayMax, Speed.TargetSpeedMin/StartSpeed/MinSpeed*, declBase/declRate*, HpParam, last-spurt fields, turf/dirt ground multiHpSub, SlopeParam, Force In fields, PositionKeepParam, ConservePowerParam, CompeteFightParam, Block, Surrounded, CongestionLaneGapAbs, and CongestionHorseCntThreshold; current Global ast_race_paramdefine raceBaseSpeed*, BasetTargetSpeed.*, addSpeedParamCoef, accelPowCoef, accelPowCoefUpSlope, AccelPowCoefSqrt, StartAccelAdd, Speed.PhaseAccelCoefArray/ExArray, Skill.*HorseNearDistance, Skill.*HorseNearLaneDistance, Skill.BehindNearParamArray, SkillParam.AdditionalActivateAbilityMaxCountArray, RaceParam.AbilityTimeDivideDistance, RaceParam.CoolDownTimeDivideDistance, SkillParam.OrderUpAddAbilityTime, SkillParam.OrderUpAddAbilityTimeMaxCount, and the complete ability-value calculator block for scaling codes 2 through 23; Global 10006800 CompeteFightParam and HorseRaceAIBase.CheckCompeteFightNear source semantics; legacy extracted RaceParamDefine.Near, final-corner LastMoveOut, and overtake-lane coefficient numeric values with current Global _CheckNearHorse and HorseTargetLaneCalculatorRace source semantics; Global HorseRaceAIBase.UpdateAroundHorsesParam, HorseRaceAISimulate.UpdateSurrounded, HorseRaceInfoSimulate._UpdateCongestionTime, HorseRaceInfoSimulate.UpdateBehindHorseNearTimeParamSet, 10006800 Force In construction/live gate, slope check interval, and HorseRaceInfo.UpdateMinSpeed formula; replay-observed conserve-power release duration";
const COURSE_EVENT_PARAMS: &[(&str, &str)] = &[
    (
        "10101",
        include_str!("../jp_data/courseeventparams/10101.json"),
    ),
    (
        "10102",
        include_str!("../jp_data/courseeventparams/10102.json"),
    ),
    (
        "10103",
        include_str!("../jp_data/courseeventparams/10103.json"),
    ),
    (
        "10104",
        include_str!("../jp_data/courseeventparams/10104.json"),
    ),
    (
        "10105",
        include_str!("../jp_data/courseeventparams/10105.json"),
    ),
    (
        "10106",
        include_str!("../jp_data/courseeventparams/10106.json"),
    ),
    (
        "10107",
        include_str!("../jp_data/courseeventparams/10107.json"),
    ),
    (
        "10108",
        include_str!("../jp_data/courseeventparams/10108.json"),
    ),
    (
        "10201",
        include_str!("../jp_data/courseeventparams/10201.json"),
    ),
    (
        "10202",
        include_str!("../jp_data/courseeventparams/10202.json"),
    ),
    (
        "10203",
        include_str!("../jp_data/courseeventparams/10203.json"),
    ),
    (
        "10204",
        include_str!("../jp_data/courseeventparams/10204.json"),
    ),
    (
        "10205",
        include_str!("../jp_data/courseeventparams/10205.json"),
    ),
    (
        "10206",
        include_str!("../jp_data/courseeventparams/10206.json"),
    ),
    (
        "10207",
        include_str!("../jp_data/courseeventparams/10207.json"),
    ),
    (
        "10208",
        include_str!("../jp_data/courseeventparams/10208.json"),
    ),
    (
        "10301",
        include_str!("../jp_data/courseeventparams/10301.json"),
    ),
    (
        "10302",
        include_str!("../jp_data/courseeventparams/10302.json"),
    ),
    (
        "10303",
        include_str!("../jp_data/courseeventparams/10303.json"),
    ),
    (
        "10304",
        include_str!("../jp_data/courseeventparams/10304.json"),
    ),
    (
        "10305",
        include_str!("../jp_data/courseeventparams/10305.json"),
    ),
    (
        "10306",
        include_str!("../jp_data/courseeventparams/10306.json"),
    ),
    (
        "10307",
        include_str!("../jp_data/courseeventparams/10307.json"),
    ),
    (
        "10308",
        include_str!("../jp_data/courseeventparams/10308.json"),
    ),
    (
        "10309",
        include_str!("../jp_data/courseeventparams/10309.json"),
    ),
    (
        "10310",
        include_str!("../jp_data/courseeventparams/10310.json"),
    ),
    (
        "10311",
        include_str!("../jp_data/courseeventparams/10311.json"),
    ),
    (
        "10312",
        include_str!("../jp_data/courseeventparams/10312.json"),
    ),
    (
        "10401",
        include_str!("../jp_data/courseeventparams/10401.json"),
    ),
    (
        "10402",
        include_str!("../jp_data/courseeventparams/10402.json"),
    ),
    (
        "10403",
        include_str!("../jp_data/courseeventparams/10403.json"),
    ),
    (
        "10404",
        include_str!("../jp_data/courseeventparams/10404.json"),
    ),
    (
        "10405",
        include_str!("../jp_data/courseeventparams/10405.json"),
    ),
    (
        "10406",
        include_str!("../jp_data/courseeventparams/10406.json"),
    ),
    (
        "10407",
        include_str!("../jp_data/courseeventparams/10407.json"),
    ),
    (
        "10501",
        include_str!("../jp_data/courseeventparams/10501.json"),
    ),
    (
        "10502",
        include_str!("../jp_data/courseeventparams/10502.json"),
    ),
    (
        "10503",
        include_str!("../jp_data/courseeventparams/10503.json"),
    ),
    (
        "10504",
        include_str!("../jp_data/courseeventparams/10504.json"),
    ),
    (
        "10505",
        include_str!("../jp_data/courseeventparams/10505.json"),
    ),
    (
        "10506",
        include_str!("../jp_data/courseeventparams/10506.json"),
    ),
    (
        "10507",
        include_str!("../jp_data/courseeventparams/10507.json"),
    ),
    (
        "10508",
        include_str!("../jp_data/courseeventparams/10508.json"),
    ),
    (
        "10509",
        include_str!("../jp_data/courseeventparams/10509.json"),
    ),
    (
        "10510",
        include_str!("../jp_data/courseeventparams/10510.json"),
    ),
    (
        "10511",
        include_str!("../jp_data/courseeventparams/10511.json"),
    ),
    (
        "10601",
        include_str!("../jp_data/courseeventparams/10601.json"),
    ),
    (
        "10602",
        include_str!("../jp_data/courseeventparams/10602.json"),
    ),
    (
        "10603",
        include_str!("../jp_data/courseeventparams/10603.json"),
    ),
    (
        "10604",
        include_str!("../jp_data/courseeventparams/10604.json"),
    ),
    (
        "10605",
        include_str!("../jp_data/courseeventparams/10605.json"),
    ),
    (
        "10606",
        include_str!("../jp_data/courseeventparams/10606.json"),
    ),
    (
        "10607",
        include_str!("../jp_data/courseeventparams/10607.json"),
    ),
    (
        "10608",
        include_str!("../jp_data/courseeventparams/10608.json"),
    ),
    (
        "10609",
        include_str!("../jp_data/courseeventparams/10609.json"),
    ),
    (
        "10610",
        include_str!("../jp_data/courseeventparams/10610.json"),
    ),
    (
        "10611",
        include_str!("../jp_data/courseeventparams/10611.json"),
    ),
    (
        "10612",
        include_str!("../jp_data/courseeventparams/10612.json"),
    ),
    (
        "10613",
        include_str!("../jp_data/courseeventparams/10613.json"),
    ),
    (
        "10614",
        include_str!("../jp_data/courseeventparams/10614.json"),
    ),
    (
        "10701",
        include_str!("../jp_data/courseeventparams/10701.json"),
    ),
    (
        "10702",
        include_str!("../jp_data/courseeventparams/10702.json"),
    ),
    (
        "10703",
        include_str!("../jp_data/courseeventparams/10703.json"),
    ),
    (
        "10704",
        include_str!("../jp_data/courseeventparams/10704.json"),
    ),
    (
        "10705",
        include_str!("../jp_data/courseeventparams/10705.json"),
    ),
    (
        "10706",
        include_str!("../jp_data/courseeventparams/10706.json"),
    ),
    (
        "10707",
        include_str!("../jp_data/courseeventparams/10707.json"),
    ),
    (
        "10708",
        include_str!("../jp_data/courseeventparams/10708.json"),
    ),
    (
        "10709",
        include_str!("../jp_data/courseeventparams/10709.json"),
    ),
    (
        "10801",
        include_str!("../jp_data/courseeventparams/10801.json"),
    ),
    (
        "10802",
        include_str!("../jp_data/courseeventparams/10802.json"),
    ),
    (
        "10803",
        include_str!("../jp_data/courseeventparams/10803.json"),
    ),
    (
        "10804",
        include_str!("../jp_data/courseeventparams/10804.json"),
    ),
    (
        "10805",
        include_str!("../jp_data/courseeventparams/10805.json"),
    ),
    (
        "10806",
        include_str!("../jp_data/courseeventparams/10806.json"),
    ),
    (
        "10807",
        include_str!("../jp_data/courseeventparams/10807.json"),
    ),
    (
        "10808",
        include_str!("../jp_data/courseeventparams/10808.json"),
    ),
    (
        "10809",
        include_str!("../jp_data/courseeventparams/10809.json"),
    ),
    (
        "10810",
        include_str!("../jp_data/courseeventparams/10810.json"),
    ),
    (
        "10811",
        include_str!("../jp_data/courseeventparams/10811.json"),
    ),
    (
        "10812",
        include_str!("../jp_data/courseeventparams/10812.json"),
    ),
    (
        "10813",
        include_str!("../jp_data/courseeventparams/10813.json"),
    ),
    (
        "10814",
        include_str!("../jp_data/courseeventparams/10814.json"),
    ),
    (
        "10815",
        include_str!("../jp_data/courseeventparams/10815.json"),
    ),
    (
        "10901",
        include_str!("../jp_data/courseeventparams/10901.json"),
    ),
    (
        "10902",
        include_str!("../jp_data/courseeventparams/10902.json"),
    ),
    (
        "10903",
        include_str!("../jp_data/courseeventparams/10903.json"),
    ),
    (
        "10904",
        include_str!("../jp_data/courseeventparams/10904.json"),
    ),
    (
        "10905",
        include_str!("../jp_data/courseeventparams/10905.json"),
    ),
    (
        "10906",
        include_str!("../jp_data/courseeventparams/10906.json"),
    ),
    (
        "10907",
        include_str!("../jp_data/courseeventparams/10907.json"),
    ),
    (
        "10908",
        include_str!("../jp_data/courseeventparams/10908.json"),
    ),
    (
        "10909",
        include_str!("../jp_data/courseeventparams/10909.json"),
    ),
    (
        "10910",
        include_str!("../jp_data/courseeventparams/10910.json"),
    ),
    (
        "10911",
        include_str!("../jp_data/courseeventparams/10911.json"),
    ),
    (
        "10912",
        include_str!("../jp_data/courseeventparams/10912.json"),
    ),
    (
        "10913",
        include_str!("../jp_data/courseeventparams/10913.json"),
    ),
    (
        "10914",
        include_str!("../jp_data/courseeventparams/10914.json"),
    ),
    (
        "11001",
        include_str!("../jp_data/courseeventparams/11001.json"),
    ),
    (
        "11002",
        include_str!("../jp_data/courseeventparams/11002.json"),
    ),
    (
        "11003",
        include_str!("../jp_data/courseeventparams/11003.json"),
    ),
    (
        "11004",
        include_str!("../jp_data/courseeventparams/11004.json"),
    ),
    (
        "11005",
        include_str!("../jp_data/courseeventparams/11005.json"),
    ),
    (
        "11006",
        include_str!("../jp_data/courseeventparams/11006.json"),
    ),
    (
        "11007",
        include_str!("../jp_data/courseeventparams/11007.json"),
    ),
    (
        "11101",
        include_str!("../jp_data/courseeventparams/11101.json"),
    ),
    (
        "11102",
        include_str!("../jp_data/courseeventparams/11102.json"),
    ),
    (
        "11103",
        include_str!("../jp_data/courseeventparams/11103.json"),
    ),
    (
        "11201",
        include_str!("../jp_data/courseeventparams/11201.json"),
    ),
    (
        "11203",
        include_str!("../jp_data/courseeventparams/11203.json"),
    ),
];

// Global course-event bundles are authoritative for Global race replay. Keep
// JP as the complete fallback set and replace rows only after extracting the
// corresponding current Global client asset.
const GLOBAL_COURSE_EVENT_PARAM_OVERRIDES: &[(&str, &str)] = &[(
    "10501",
    include_str!("../global_data/courseeventparams/10501.json"),
)];

pub fn version_hash() -> String {
    let mut digest = Sha256::new();
    digest.update(SCHEMA_VERSION.to_le_bytes());
    digest.update(
        serde_json::to_vec(&SimulatorRaceParameters::current())
            .expect("simulator race parameters must serialize"),
    );
    for (course_id, json) in COURSE_EVENT_PARAMS
        .iter()
        .chain(GLOBAL_COURSE_EVENT_PARAM_OVERRIDES)
    {
        digest.update(course_id.as_bytes());
        digest.update([0]);
        digest.update(json.as_bytes());
    }
    hex::encode(digest.finalize())
}

#[derive(Debug, Serialize)]
pub struct SimulatorCourseSet<'a> {
    pub schema_version: u32,
    pub master_version: &'a str,
    pub race_parameters: SimulatorRaceParameters<'static>,
    pub courses: Vec<SimulatorCourse>,
}

#[derive(Debug, Serialize)]
pub struct SimulatorRaceParameters<'a> {
    pub schema_version: u32,
    pub provenance: &'a str,
    pub start_delay_max_seconds: f64,
    pub target_speed_min: f64,
    pub skill: SimulatorSkillProximityParameters<'a>,
    pub near_horse: SimulatorNearHorseParameters,
    pub extra_move_lane: SimulatorExtraMoveLaneParameters,
    pub lane_movement: SimulatorLaneMovementParameters,
    pub overtake_lane: SimulatorOvertakeLaneParameters,
    pub around_horse: SimulatorAroundHorseParameters,
    pub deceleration: SimulatorDecelerationParameters,
    pub minimum_speed: SimulatorMinimumSpeedParameters,
    pub base_motion: SimulatorBaseMotionParameters,
    pub acceleration: SimulatorAccelerationParameters,
    pub hp: SimulatorHpParameters,
    pub temptation: SimulatorTemptationParameters,
    pub additional_activation: SimulatorAdditionalActivationParameters,
    pub skill_timing: SimulatorSkillTimingParameters,
    pub skill_value_scaling: SimulatorSkillValueScalingParameters<'a>,
    pub last_spurt: SimulatorLastSpurtParameters,
    pub slope: SimulatorSlopeParameters,
    pub force_in: SimulatorForceInParameters,
    pub position_keep: SimulatorPositionKeepParameters,
    pub conserve_power: SimulatorConservePowerParameters,
    pub compete_fight: SimulatorCompeteFightParameters,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillProximityParameters<'a> {
    pub infront_horse_near_distance_m: f64,
    pub behind_horse_near_distance_m: f64,
    pub infront_horse_near_lane_course_widths: f64,
    pub behind_horse_near_lane_course_widths: f64,
    pub behind_near_parameter_sets: &'a [SimulatorNearLaneParameters],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorNearHorseParameters {
    pub distance_threshold_m: f64,
    pub lane_distance_threshold_m: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorExtraMoveLaneParameters {
    pub start_final_corner_rate: f64,
    pub lane_normalize_course_widths: f64,
    pub lane_coefficient_course_widths: f64,
    pub lane_random_max_course_widths: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorLaneMovementParameters {
    pub speed_base: f64,
    pub speed_add: f64,
    pub power_coefficient: f64,
    pub outside_position_coefficient: f64,
    pub acceleration_base_coefficient: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorOvertakeLaneParameters {
    pub final_in_lane_coefficient: f64,
    pub final_out_lane_coefficient: f64,
    pub target_cooldown_seconds: f64,
    pub order_change_count_cooldown_seconds: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorAroundHorseParameters {
    pub block: SimulatorBlockParameters,
    pub surrounded: SimulatorSurroundedParameters,
    pub congestion: SimulatorCongestionParameters,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorBlockParameters {
    pub front_distance_m: f64,
    pub front_lane_course_widths: f64,
    pub side_distance_m: f64,
    pub side_lane_course_widths: f64,
    pub front_min_speed_rate: f64,
    pub front_max_speed_rate: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSurroundedParameters {
    pub outside_distance_m: f64,
    pub outside_lane_course_widths: f64,
    pub directional_distance_m: f64,
    pub directional_lane_course_widths: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorCongestionParameters {
    pub lane_course_widths: f64,
    pub horse_count_threshold: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SimulatorNearLaneParameters {
    pub distance_m: f64,
    pub lane_course_widths: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorDecelerationParameters {
    pub base: f64,
    pub phase_rates: [f64; 3],
    pub hp_zero_rate: f64,
    pub pace_down_rate: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorMinimumSpeedParameters {
    pub start_speed: f64,
    pub base_speed_rate: f64,
    pub guts_sqrt_coefficient: f64,
    pub guts_coefficient: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorAccelerationParameters {
    pub power_coefficient: f64,
    pub uphill_power_coefficient: f64,
    pub power_sqrt_coefficient: f64,
    pub start_dash_add: f64,
    pub phase_coefficients: [[f64; 3]; 6],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorBaseMotionParameters {
    pub race_base_speed: f64,
    pub race_base_distance_m: f64,
    pub race_base_distance_rate_m: f64,
    pub phase_target_speed_percent: [[f64; 4]; 6],
    pub random_minus_base_percent: f64,
    pub random_minus_wisdom_divisor: f64,
    pub random_plus_wisdom_divisor: f64,
    pub phase_end_speed_sqrt_coefficient: f64,
    pub phase_end_speed_scale: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorHpParameters {
    pub initial_stamina_coefficient: f64,
    pub max_hp_strategy_coefficients: [f64; 6],
    pub consumption_base: f64,
    pub speed_gap_offset: f64,
    pub speed_gap_square_divisor: f64,
    pub normal_rate: f64,
    pub rushed_rate: f64,
    pub pace_down_rate: f64,
    pub lead_competition_nige_rate: f64,
    pub rushed_lead_competition_nige_rate: f64,
    pub lead_competition_oonige_rate: f64,
    pub rushed_lead_competition_oonige_rate: f64,
    pub downhill_rate: f64,
    pub guts_coefficient: f64,
    pub guts_sqrt_coefficient: f64,
    pub ground_condition_rates: [[f64; 5]; 3],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorTemptationParameters {
    pub lot_section_min: u8,
    pub lot_section_max: u8,
    pub start_percent_value1: f64,
    pub force_end_seconds: f64,
    pub end_check_seconds: f64,
    pub end_chance_percent: f64,
    pub oikomi_to_sashi_percent: f64,
    pub oikomi_to_senko_percent: f64,
    pub oikomi_to_nige_percent: f64,
    pub sashi_to_senko_percent: f64,
    pub sashi_to_nige_percent: f64,
    pub senko_to_nige_percent: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorAdditionalActivationParameters {
    pub order_up_max_count: u8,
    pub activate_any_skill_type_2_max_count: u8,
    pub activate_any_skill_type_3_max_count: u8,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillTimingParameters {
    pub ability_time_divide_distance_m: u32,
    pub cooldown_time_divide_distance_m: u32,
    pub order_up_add_ability_time_seconds: f64,
    pub order_up_add_ability_time_max_count: u8,
    pub duration_scaling: SimulatorSkillDurationScalingParameters,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillDurationScalingParameters {
    pub distance_diff_top_divisor_m: f64,
    pub distance_diff_top_min_multiplier: f64,
    pub distance_diff_top_max_multiplier: f64,
    pub remain_hp_type_1: &'static [SimulatorSkillDurationThreshold],
    pub remain_hp_type_2: &'static [SimulatorSkillDurationThreshold],
    pub blocked_time_type_1: &'static [SimulatorSkillDurationThreshold],
    pub blocked_time_type_2: &'static [SimulatorSkillDurationThreshold],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillDurationThreshold {
    pub upper_bound: f64,
    pub multiplier: f64,
}

impl SimulatorSkillDurationThreshold {
    const fn new(upper_bound: f64, multiplier: f64) -> Self {
        Self {
            upper_bound,
            multiplier,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillValueScalingParameters<'a> {
    pub acquired_skill_excluded_count: u16,
    pub acquired_skill_base_multiplier: f64,
    pub acquired_skill_per_count_multiplier: f64,
    pub acquired_skill_max_multiplier: f64,
    pub aoharu_team_total_status: &'a [SimulatorSkillValueThreshold],
    pub random_ability_value_type_1: &'a [SimulatorSkillValueRandomRow],
    pub random_ability_value_type_2: &'a [SimulatorSkillValueRandomRow],
    pub race_wins: &'a [SimulatorSkillValueThreshold],
    pub order_up_count_corner_phase_end_after: &'a [SimulatorSkillValueThreshold],
    pub fan_count: &'a [SimulatorSkillValueThreshold],
    pub max_raw_stat: &'a [SimulatorSkillValueThreshold],
    pub activated_passive_skill_count: &'a [SimulatorSkillValueThreshold],
    pub heal_base_multiplier: f64,
    pub heal_per_activation_multiplier: f64,
    pub heal_max_multiplier: f64,
    pub final_corner_base_multiplier: f64,
    pub final_corner_per_order_subtract: f64,
    pub final_corner_min_multiplier: f64,
    pub inverse_team_member_numerator: f64,
    pub base_wisdom: &'a [SimulatorSkillValueThreshold],
    pub distance_diff_top_additive: &'a [SimulatorSkillValueAdditiveThreshold],
    pub blocked_time_type_1: &'a [SimulatorSkillValueThreshold],
    pub blocked_time_type_2: &'a [SimulatorSkillValueThreshold],
    pub final_speed_type_1: &'a [SimulatorSkillValueThreshold],
    pub final_speed_type_2: &'a [SimulatorSkillValueThreshold],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillValueThreshold {
    pub upper_bound: f64,
    pub multiplier: f64,
}

impl SimulatorSkillValueThreshold {
    const fn new(upper_bound: f64, multiplier: f64) -> Self {
        Self {
            upper_bound,
            multiplier,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillValueRandomRow {
    pub modifier: f64,
    pub weight: u32,
}

impl SimulatorSkillValueRandomRow {
    const fn new(modifier: f64, weight: u32) -> Self {
        Self { modifier, weight }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSkillValueAdditiveThreshold {
    pub upper_bound: f64,
    pub additive: f64,
}

impl SimulatorSkillValueAdditiveThreshold {
    const fn new(upper_bound: f64, additive: f64) -> Self {
        Self {
            upper_bound,
            additive,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorLastSpurtParameters {
    pub target_speed_speed_sqrt_coefficient: f64,
    pub target_speed_add_speed_coefficient: f64,
    pub target_speed_base_coefficient: f64,
    pub target_speed_guts_coefficient: f64,
    pub target_speed_guts_power: f64,
    pub target_speed_guts_scale: f64,
    pub base_target_speed_add_coefficient: f64,
    pub distance_goal_buffer_m: f64,
    pub start_distance_max_from_goal_m: f64,
    pub initial_speed_delta_mps: f64,
    pub candidate_speed_delta_mps: f64,
    pub acceptance_base_percent: f64,
    pub acceptance_wisdom_percent_coefficient: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorSlopeParameters {
    pub check_interval_seconds: f64,
    pub slope_per_threshold: f64,
    pub uphill_add_speed_value1: f64,
    pub downhill_add_speed_value1: f64,
    pub downhill_add_speed_value2: f64,
    pub downhill_start_wisdom_percent_coefficient: f64,
    pub downhill_end_chance_percent: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorForceInParameters {
    pub lane_threshold_course_widths: f64,
    pub target_speed_add_nige_mps: f64,
    pub target_speed_add_senko_mps: f64,
    pub target_speed_add_sashi_mps: f64,
    pub target_speed_add_oikomi_mps: f64,
    pub target_speed_add_random_range_mps: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorPositionKeepParameters {
    pub check_interval_seconds: f64,
    pub cooldown_seconds: f64,
    pub start_section: u8,
    pub end_section: u8,
    pub continue_section: u8,
    pub oonige_continue_section: u8,
    pub speed_up_wisdom_percent_coefficient: f64,
    pub speed_up_target_speed_multiplier: f64,
    pub speed_up_end_distance_m: f64,
    pub speed_up_only_nige_end_distance_m: f64,
    pub overtake_wisdom_percent_coefficient: f64,
    pub overtake_end_distance_m: f64,
    pub overtake_target_speed_multiplier: f64,
    pub oonige_speed_up_wisdom_percent_coefficient: f64,
    pub oonige_speed_up_target_speed_multiplier: f64,
    pub oonige_speed_up_end_distance_m: f64,
    pub oonige_overtake_wisdom_percent_coefficient: f64,
    pub oonige_overtake_end_distance_m: f64,
    pub oonige_overtake_target_speed_multiplier: f64,
    pub pace_distance_diff_max_coefficient: f64,
    pub senko_min_distance_m: f64,
    pub senko_max_distance_m: f64,
    pub sashi_min_distance_m: f64,
    pub sashi_max_distance_m: f64,
    pub oikomi_min_distance_m: f64,
    pub oikomi_max_distance_m: f64,
    pub pace_base_distance_m: f64,
    pub pace_distance_coefficient: f64,
    pub pace_down_target_speed_multiplier: f64,
    pub pace_down_middle_target_speed_multiplier: f64,
    pub pace_down_target_lane: f64,
    pub pace_up_wisdom_percent_coefficient: f64,
    pub pace_up_target_speed_multiplier: f64,
    pub pace_up_ex_target_speed_multiplier: f64,
    pub pacemaker: SimulatorPacemakerParameters,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorPacemakerParameters {
    pub forward_running_style_range_m: f64,
    pub most_forward_style_range_m: f64,
    pub top_not_most_forward_style_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorConservePowerParameters {
    pub power_threshold: f64,
    pub release_power_coefficient: f64,
    pub release_acceleration_scale: f64,
    pub release_base_duration_seconds: f64,
    pub activity_time_coefficient: f64,
    pub strategy_distance_acceleration_coefficients: [[f64; 4]; 5],
    pub activity_state_deltas: [f64; 4],
    pub activity_acceleration_coefficients: [f64; 4],
    pub duration_distance_coefficients: [f64; 4],
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SimulatorCompeteFightParameters {
    pub target_distance_m: f64,
    pub target_lane_course_widths: f64,
    pub target_continue_seconds: f64,
    pub target_speed_gap_mps: f64,
    pub target_order_percent: u8,
    pub exit_distance_m: f64,
    pub end_hp_percent: u8,
    pub start_hp_percent: u8,
    pub target_speed_base: f64,
    pub target_speed_power: f64,
    pub target_speed_scale: f64,
    pub acceleration_base: f64,
    pub acceleration_power: f64,
    pub acceleration_scale: f64,
}

impl SimulatorRaceParameters<'static> {
    pub(crate) fn current() -> Self {
        Self {
            schema_version: RACE_PARAMETERS_SCHEMA_VERSION,
            provenance: RACE_PARAMETERS_PROVENANCE,
            start_delay_max_seconds: START_DELAY_MAX_SECONDS,
            target_speed_min: TARGET_SPEED_MIN,
            skill: SimulatorSkillProximityParameters {
                infront_horse_near_distance_m: SKILL_INFRONT_HORSE_NEAR_DISTANCE_M,
                behind_horse_near_distance_m: SKILL_BEHIND_HORSE_NEAR_DISTANCE_M,
                infront_horse_near_lane_course_widths: SKILL_INFRONT_HORSE_NEAR_LANE_COURSE_WIDTHS,
                behind_horse_near_lane_course_widths: SKILL_BEHIND_HORSE_NEAR_LANE_COURSE_WIDTHS,
                behind_near_parameter_sets: SKILL_BEHIND_NEAR_PARAMETER_SETS,
            },
            near_horse: SimulatorNearHorseParameters {
                distance_threshold_m: NEAR_HORSE_DISTANCE_THRESHOLD_M,
                lane_distance_threshold_m: NEAR_HORSE_LANE_DISTANCE_THRESHOLD_M,
            },
            extra_move_lane: SimulatorExtraMoveLaneParameters {
                start_final_corner_rate: EXTRA_MOVE_START_FINAL_CORNER_RATE,
                lane_normalize_course_widths: EXTRA_MOVE_LANE_NORMALIZE_COURSE_WIDTHS,
                lane_coefficient_course_widths: EXTRA_MOVE_LANE_COEFFICIENT_COURSE_WIDTHS,
                lane_random_max_course_widths: EXTRA_MOVE_LANE_RANDOM_MAX_COURSE_WIDTHS,
            },
            lane_movement: SimulatorLaneMovementParameters {
                speed_base: LANE_MOVE_SPEED_BASE,
                speed_add: LANE_MOVE_SPEED_ADD,
                power_coefficient: LANE_MOVE_SPEED_POWER_COEFFICIENT,
                outside_position_coefficient: LANE_MOVE_SPEED_OUTSIDE_POSITION_COEFFICIENT,
                acceleration_base_coefficient: LANE_MOVE_ACCELERATION_BASE_COEFFICIENT,
            },
            overtake_lane: SimulatorOvertakeLaneParameters {
                final_in_lane_coefficient: OVERTAKE_FINAL_IN_LANE_COEFFICIENT,
                final_out_lane_coefficient: OVERTAKE_FINAL_OUT_LANE_COEFFICIENT,
                target_cooldown_seconds: OVERTAKE_TARGET_COOLDOWN_SECONDS,
                order_change_count_cooldown_seconds: ORDER_CHANGE_COUNT_COOLDOWN_SECONDS,
            },
            around_horse: SimulatorAroundHorseParameters {
                block: SimulatorBlockParameters {
                    front_distance_m: BLOCK_FRONT_DISTANCE_M,
                    front_lane_course_widths: BLOCK_FRONT_LANE_COURSE_WIDTHS,
                    side_distance_m: BLOCK_SIDE_DISTANCE_M,
                    side_lane_course_widths: BLOCK_SIDE_LANE_COURSE_WIDTHS,
                    front_min_speed_rate: BLOCK_FRONT_MIN_SPEED_RATE,
                    front_max_speed_rate: BLOCK_FRONT_MAX_SPEED_RATE,
                },
                surrounded: SimulatorSurroundedParameters {
                    outside_distance_m: SURROUNDED_OUTSIDE_DISTANCE_M,
                    outside_lane_course_widths: SURROUNDED_OUTSIDE_LANE_COURSE_WIDTHS,
                    directional_distance_m: SURROUNDED_DIRECTIONAL_DISTANCE_M,
                    directional_lane_course_widths: SURROUNDED_DIRECTIONAL_LANE_COURSE_WIDTHS,
                },
                congestion: SimulatorCongestionParameters {
                    lane_course_widths: CONGESTION_LANE_COURSE_WIDTHS,
                    horse_count_threshold: CONGESTION_HORSE_COUNT_THRESHOLD,
                },
            },
            deceleration: SimulatorDecelerationParameters {
                base: DECELERATION_BASE,
                phase_rates: DECELERATION_PHASE_RATES,
                hp_zero_rate: DECELERATION_HP_ZERO_RATE,
                pace_down_rate: DECELERATION_PACE_DOWN_RATE,
            },
            minimum_speed: SimulatorMinimumSpeedParameters {
                start_speed: MINIMUM_SPEED_START_SPEED,
                base_speed_rate: MINIMUM_SPEED_BASE_SPEED_RATE,
                guts_sqrt_coefficient: MINIMUM_SPEED_GUTS_SQRT_COEFFICIENT,
                guts_coefficient: MINIMUM_SPEED_GUTS_COEFFICIENT,
            },
            base_motion: SimulatorBaseMotionParameters {
                race_base_speed: RACE_BASE_SPEED,
                race_base_distance_m: RACE_BASE_DISTANCE_M,
                race_base_distance_rate_m: RACE_BASE_DISTANCE_RATE_M,
                phase_target_speed_percent: PHASE_TARGET_SPEED_PERCENT,
                random_minus_base_percent: BASE_TARGET_SPEED_RANDOM_MINUS_BASE_PERCENT,
                random_minus_wisdom_divisor: BASE_TARGET_SPEED_RANDOM_MINUS_WISDOM_DIVISOR,
                random_plus_wisdom_divisor: BASE_TARGET_SPEED_RANDOM_PLUS_WISDOM_DIVISOR,
                phase_end_speed_sqrt_coefficient: PHASE_END_SPEED_SQRT_COEFFICIENT,
                phase_end_speed_scale: PHASE_END_SPEED_SCALE,
            },
            acceleration: SimulatorAccelerationParameters {
                power_coefficient: ACCEL_POWER_COEFFICIENT,
                uphill_power_coefficient: ACCEL_UPHILL_POWER_COEFFICIENT,
                power_sqrt_coefficient: ACCEL_POWER_SQRT_COEFFICIENT,
                start_dash_add: ACCEL_START_DASH_ADD,
                phase_coefficients: ACCEL_PHASE_COEFFICIENTS,
            },
            hp: SimulatorHpParameters {
                initial_stamina_coefficient: HP_INITIAL_STAMINA_COEFFICIENT,
                max_hp_strategy_coefficients: HP_MAX_STRATEGY_COEFFICIENTS,
                consumption_base: HP_CONSUMPTION_BASE,
                speed_gap_offset: HP_SPEED_GAP_OFFSET,
                speed_gap_square_divisor: HP_SPEED_GAP_SQUARE_DIVISOR,
                normal_rate: HP_NORMAL_RATE,
                rushed_rate: HP_RUSHED_RATE,
                pace_down_rate: HP_PACE_DOWN_RATE,
                lead_competition_nige_rate: HP_LEAD_COMPETITION_NIGE_RATE,
                rushed_lead_competition_nige_rate: HP_RUSHED_LEAD_COMPETITION_NIGE_RATE,
                lead_competition_oonige_rate: HP_LEAD_COMPETITION_OONIGE_RATE,
                rushed_lead_competition_oonige_rate: HP_RUSHED_LEAD_COMPETITION_OONIGE_RATE,
                downhill_rate: HP_DOWNHILL_RATE,
                guts_coefficient: HP_GUTS_COEFFICIENT,
                guts_sqrt_coefficient: HP_GUTS_SQRT_COEFFICIENT,
                ground_condition_rates: HP_GROUND_CONDITION_RATES,
            },
            temptation: SimulatorTemptationParameters {
                lot_section_min: TEMPTATION_LOT_SECTION_MIN,
                lot_section_max: TEMPTATION_LOT_SECTION_MAX,
                start_percent_value1: TEMPTATION_START_PERCENT_VALUE1,
                force_end_seconds: TEMPTATION_FORCE_END_SECONDS,
                end_check_seconds: TEMPTATION_END_CHECK_SECONDS,
                end_chance_percent: TEMPTATION_END_CHANCE_PERCENT,
                oikomi_to_sashi_percent: TEMPTATION_OIKOMI_TO_SASHI_PERCENT,
                oikomi_to_senko_percent: TEMPTATION_OIKOMI_TO_SENKO_PERCENT,
                oikomi_to_nige_percent: TEMPTATION_OIKOMI_TO_NIGE_PERCENT,
                sashi_to_senko_percent: TEMPTATION_SASHI_TO_SENKO_PERCENT,
                sashi_to_nige_percent: TEMPTATION_SASHI_TO_NIGE_PERCENT,
                senko_to_nige_percent: TEMPTATION_SENKO_TO_NIGE_PERCENT,
            },
            additional_activation: SimulatorAdditionalActivationParameters {
                order_up_max_count: ADDITIONAL_ACTIVATION_ORDER_UP_MAX_COUNT,
                activate_any_skill_type_2_max_count:
                    ADDITIONAL_ACTIVATION_ANY_SKILL_TYPE_2_MAX_COUNT,
                activate_any_skill_type_3_max_count:
                    ADDITIONAL_ACTIVATION_ANY_SKILL_TYPE_3_MAX_COUNT,
            },
            skill_timing: SimulatorSkillTimingParameters {
                ability_time_divide_distance_m: SKILL_ABILITY_TIME_DIVIDE_DISTANCE_M,
                cooldown_time_divide_distance_m: SKILL_COOLDOWN_TIME_DIVIDE_DISTANCE_M,
                order_up_add_ability_time_seconds: SKILL_ORDER_UP_ADD_ABILITY_TIME_SECONDS,
                order_up_add_ability_time_max_count: SKILL_ORDER_UP_ADD_ABILITY_TIME_MAX_COUNT,
                duration_scaling: SimulatorSkillDurationScalingParameters {
                    distance_diff_top_divisor_m: SKILL_DURATION_DISTANCE_DIFF_TOP_DIVISOR_M,
                    distance_diff_top_min_multiplier:
                        SKILL_DURATION_DISTANCE_DIFF_TOP_MIN_MULTIPLIER,
                    distance_diff_top_max_multiplier:
                        SKILL_DURATION_DISTANCE_DIFF_TOP_MAX_MULTIPLIER,
                    remain_hp_type_1: &SKILL_DURATION_REMAIN_HP_TYPE_1,
                    remain_hp_type_2: &SKILL_DURATION_REMAIN_HP_TYPE_2,
                    blocked_time_type_1: &SKILL_DURATION_BLOCKED_TIME_TYPE_1,
                    blocked_time_type_2: &SKILL_DURATION_BLOCKED_TIME_TYPE_2,
                },
            },
            skill_value_scaling: SimulatorSkillValueScalingParameters {
                acquired_skill_excluded_count: SKILL_VALUE_ACQUIRED_SKILL_EXCLUDED_COUNT,
                acquired_skill_base_multiplier: SKILL_VALUE_ACQUIRED_SKILL_BASE_MULTIPLIER,
                acquired_skill_per_count_multiplier:
                    SKILL_VALUE_ACQUIRED_SKILL_PER_COUNT_MULTIPLIER,
                acquired_skill_max_multiplier: SKILL_VALUE_ACQUIRED_SKILL_MAX_MULTIPLIER,
                aoharu_team_total_status: &SKILL_VALUE_AOHARU_TEAM_TOTAL_STATUS,
                random_ability_value_type_1: &SKILL_VALUE_RANDOM_ABILITY_TYPE_1,
                random_ability_value_type_2: &SKILL_VALUE_RANDOM_ABILITY_TYPE_2,
                race_wins: &SKILL_VALUE_RACE_WINS,
                order_up_count_corner_phase_end_after:
                    &SKILL_VALUE_ORDER_UP_COUNT_CORNER_PHASE_END_AFTER,
                fan_count: &SKILL_VALUE_FAN_COUNT,
                max_raw_stat: &SKILL_VALUE_MAX_RAW_STAT,
                activated_passive_skill_count: &SKILL_VALUE_ACTIVATED_PASSIVE_SKILL_COUNT,
                heal_base_multiplier: SKILL_VALUE_HEAL_BASE_MULTIPLIER,
                heal_per_activation_multiplier: SKILL_VALUE_HEAL_PER_ACTIVATION_MULTIPLIER,
                heal_max_multiplier: SKILL_VALUE_HEAL_MAX_MULTIPLIER,
                final_corner_base_multiplier: SKILL_VALUE_FINAL_CORNER_BASE_MULTIPLIER,
                final_corner_per_order_subtract: SKILL_VALUE_FINAL_CORNER_PER_ORDER_SUBTRACT,
                final_corner_min_multiplier: SKILL_VALUE_FINAL_CORNER_MIN_MULTIPLIER,
                inverse_team_member_numerator: SKILL_VALUE_INVERSE_TEAM_MEMBER_NUMERATOR,
                base_wisdom: &SKILL_VALUE_BASE_WISDOM,
                distance_diff_top_additive: &SKILL_VALUE_DISTANCE_DIFF_TOP_ADDITIVE,
                blocked_time_type_1: &SKILL_VALUE_BLOCKED_TIME_TYPE_1,
                blocked_time_type_2: &SKILL_VALUE_BLOCKED_TIME_TYPE_2,
                final_speed_type_1: &SKILL_VALUE_FINAL_SPEED_TYPE_1,
                final_speed_type_2: &SKILL_VALUE_FINAL_SPEED_TYPE_2,
            },
            last_spurt: SimulatorLastSpurtParameters {
                target_speed_speed_sqrt_coefficient: LAST_SPURT_TARGET_SPEED_SPEED_SQRT_COEFFICIENT,
                target_speed_add_speed_coefficient: LAST_SPURT_TARGET_SPEED_ADD_SPEED_COEFFICIENT,
                target_speed_base_coefficient: LAST_SPURT_TARGET_SPEED_BASE_COEFFICIENT,
                target_speed_guts_coefficient: LAST_SPURT_TARGET_SPEED_GUTS_COEFFICIENT,
                target_speed_guts_power: LAST_SPURT_TARGET_SPEED_GUTS_POWER,
                target_speed_guts_scale: LAST_SPURT_TARGET_SPEED_GUTS_SCALE,
                base_target_speed_add_coefficient: LAST_SPURT_BASE_TARGET_SPEED_ADD_COEFFICIENT,
                distance_goal_buffer_m: LAST_SPURT_DISTANCE_GOAL_BUFFER_M,
                start_distance_max_from_goal_m: LAST_SPURT_START_DISTANCE_MAX_FROM_GOAL_M,
                initial_speed_delta_mps: LAST_SPURT_INITIAL_SPEED_DELTA_MPS,
                candidate_speed_delta_mps: LAST_SPURT_CANDIDATE_SPEED_DELTA_MPS,
                acceptance_base_percent: LAST_SPURT_ACCEPTANCE_BASE_PERCENT,
                acceptance_wisdom_percent_coefficient:
                    LAST_SPURT_ACCEPTANCE_WISDOM_PERCENT_COEFFICIENT,
            },
            slope: SimulatorSlopeParameters {
                check_interval_seconds: SLOPE_CHECK_INTERVAL_SECONDS,
                slope_per_threshold: SLOPE_PER_THRESHOLD,
                uphill_add_speed_value1: SLOPE_UPHILL_ADD_SPEED_VALUE1,
                downhill_add_speed_value1: SLOPE_DOWNHILL_ADD_SPEED_VALUE1,
                downhill_add_speed_value2: SLOPE_DOWNHILL_ADD_SPEED_VALUE2,
                downhill_start_wisdom_percent_coefficient:
                    SLOPE_DOWNHILL_START_WISDOM_PERCENT_COEFFICIENT,
                downhill_end_chance_percent: SLOPE_DOWNHILL_END_CHANCE_PERCENT,
            },
            force_in: SimulatorForceInParameters {
                lane_threshold_course_widths: FORCE_IN_LANE_THRESHOLD_COURSE_WIDTHS,
                target_speed_add_nige_mps: FORCE_IN_TARGET_SPEED_ADD_NIGE_MPS,
                target_speed_add_senko_mps: FORCE_IN_TARGET_SPEED_ADD_SENKO_MPS,
                target_speed_add_sashi_mps: FORCE_IN_TARGET_SPEED_ADD_SASHI_MPS,
                target_speed_add_oikomi_mps: FORCE_IN_TARGET_SPEED_ADD_OIKOMI_MPS,
                target_speed_add_random_range_mps: FORCE_IN_TARGET_SPEED_ADD_RANDOM_RANGE_MPS,
            },
            position_keep: SimulatorPositionKeepParameters {
                check_interval_seconds: POSITION_KEEP_CHECK_INTERVAL_SECONDS,
                cooldown_seconds: POSITION_KEEP_COOLDOWN_SECONDS,
                start_section: POSITION_KEEP_START_SECTION,
                end_section: POSITION_KEEP_END_SECTION,
                continue_section: POSITION_KEEP_CONTINUE_SECTION,
                oonige_continue_section: POSITION_KEEP_OONIGE_CONTINUE_SECTION,
                speed_up_wisdom_percent_coefficient:
                    POSITION_KEEP_SPEED_UP_WISDOM_PERCENT_COEFFICIENT,
                speed_up_target_speed_multiplier: POSITION_KEEP_SPEED_UP_TARGET_SPEED_MULTIPLIER,
                speed_up_end_distance_m: POSITION_KEEP_SPEED_UP_END_DISTANCE_M,
                speed_up_only_nige_end_distance_m: POSITION_KEEP_SPEED_UP_ONLY_NIGE_END_DISTANCE_M,
                overtake_wisdom_percent_coefficient:
                    POSITION_KEEP_OVERTAKE_WISDOM_PERCENT_COEFFICIENT,
                overtake_end_distance_m: POSITION_KEEP_OVERTAKE_END_DISTANCE_M,
                overtake_target_speed_multiplier: POSITION_KEEP_OVERTAKE_TARGET_SPEED_MULTIPLIER,
                oonige_speed_up_wisdom_percent_coefficient:
                    POSITION_KEEP_OONIGE_SPEED_UP_WISDOM_PERCENT_COEFFICIENT,
                oonige_speed_up_target_speed_multiplier:
                    POSITION_KEEP_OONIGE_SPEED_UP_TARGET_SPEED_MULTIPLIER,
                oonige_speed_up_end_distance_m: POSITION_KEEP_OONIGE_SPEED_UP_END_DISTANCE_M,
                oonige_overtake_wisdom_percent_coefficient:
                    POSITION_KEEP_OONIGE_OVERTAKE_WISDOM_PERCENT_COEFFICIENT,
                oonige_overtake_end_distance_m: POSITION_KEEP_OONIGE_OVERTAKE_END_DISTANCE_M,
                oonige_overtake_target_speed_multiplier:
                    POSITION_KEEP_OONIGE_OVERTAKE_TARGET_SPEED_MULTIPLIER,
                pace_distance_diff_max_coefficient:
                    POSITION_KEEP_PACE_DISTANCE_DIFF_MAX_COEFFICIENT,
                senko_min_distance_m: POSITION_KEEP_SENKO_MIN_DISTANCE_M,
                senko_max_distance_m: POSITION_KEEP_SENKO_MAX_DISTANCE_M,
                sashi_min_distance_m: POSITION_KEEP_SASHI_MIN_DISTANCE_M,
                sashi_max_distance_m: POSITION_KEEP_SASHI_MAX_DISTANCE_M,
                oikomi_min_distance_m: POSITION_KEEP_OIKOMI_MIN_DISTANCE_M,
                oikomi_max_distance_m: POSITION_KEEP_OIKOMI_MAX_DISTANCE_M,
                pace_base_distance_m: POSITION_KEEP_PACE_BASE_DISTANCE_M,
                pace_distance_coefficient: POSITION_KEEP_PACE_DISTANCE_COEFFICIENT,
                pace_down_target_speed_multiplier: POSITION_KEEP_PACE_DOWN_TARGET_SPEED_MULTIPLIER,
                pace_down_middle_target_speed_multiplier:
                    POSITION_KEEP_PACE_DOWN_MIDDLE_TARGET_SPEED_MULTIPLIER,
                pace_down_target_lane: POSITION_KEEP_PACE_DOWN_TARGET_LANE,
                pace_up_wisdom_percent_coefficient:
                    POSITION_KEEP_PACE_UP_WISDOM_PERCENT_COEFFICIENT,
                pace_up_target_speed_multiplier: POSITION_KEEP_PACE_UP_TARGET_SPEED_MULTIPLIER,
                pace_up_ex_target_speed_multiplier:
                    POSITION_KEEP_PACE_UP_EX_TARGET_SPEED_MULTIPLIER,
                pacemaker: SimulatorPacemakerParameters {
                    forward_running_style_range_m: PACEMAKER_FORWARD_RUNNING_STYLE_RANGE_M,
                    most_forward_style_range_m: PACEMAKER_MOST_FORWARD_STYLE_RANGE_M,
                    top_not_most_forward_style_count: PACEMAKER_TOP_NOT_MOST_FORWARD_STYLE_COUNT,
                },
            },
            conserve_power: SimulatorConservePowerParameters {
                power_threshold: CONSERVE_POWER_THRESHOLD,
                release_power_coefficient: CONSERVE_POWER_RELEASE_POWER_COEFFICIENT,
                release_acceleration_scale: CONSERVE_POWER_RELEASE_ACCELERATION_SCALE,
                release_base_duration_seconds: CONSERVE_POWER_RELEASE_BASE_DURATION_SECONDS,
                activity_time_coefficient: CONSERVE_POWER_ACTIVITY_TIME_COEFFICIENT,
                strategy_distance_acceleration_coefficients:
                    CONSERVE_POWER_STRATEGY_DISTANCE_COEFFICIENTS,
                activity_state_deltas: CONSERVE_POWER_ACTIVITY_STATE_DELTAS,
                activity_acceleration_coefficients:
                    CONSERVE_POWER_ACTIVITY_ACCELERATION_COEFFICIENTS,
                duration_distance_coefficients: CONSERVE_POWER_DURATION_DISTANCE_COEFFICIENTS,
            },
            compete_fight: SimulatorCompeteFightParameters {
                target_distance_m: COMPETE_FIGHT_TARGET_DISTANCE_M,
                target_lane_course_widths: COMPETE_FIGHT_TARGET_LANE_COURSE_WIDTHS,
                target_continue_seconds: COMPETE_FIGHT_TARGET_CONTINUE_SECONDS,
                target_speed_gap_mps: COMPETE_FIGHT_TARGET_SPEED_GAP_MPS,
                target_order_percent: COMPETE_FIGHT_TARGET_ORDER_PERCENT,
                exit_distance_m: COMPETE_FIGHT_EXIT_DISTANCE_M,
                end_hp_percent: COMPETE_FIGHT_END_HP_PERCENT,
                start_hp_percent: COMPETE_FIGHT_START_HP_PERCENT,
                target_speed_base: COMPETE_FIGHT_TARGET_SPEED_BASE,
                target_speed_power: COMPETE_FIGHT_TARGET_SPEED_POWER,
                target_speed_scale: COMPETE_FIGHT_TARGET_SPEED_SCALE,
                acceleration_base: COMPETE_FIGHT_ACCELERATION_BASE,
                acceleration_power: COMPETE_FIGHT_ACCELERATION_POWER,
                acceleration_scale: COMPETE_FIGHT_ACCELERATION_SCALE,
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SimulatorCourse {
    pub course_id: u32,
    pub race_track_id: u32,
    pub initial_lane_type: u8,
    pub enable_half_gate: bool,
    pub run_outside: bool,
    pub distance: u16,
    pub distance_type: u8,
    pub surface: u8,
    pub turn: u8,
    pub course: u8,
    pub lane_max: u32,
    pub lane_max_events: Vec<CourseLaneMaxEvent>,
    pub move_lane_point: f32,
    pub first_move_lane_is_in: bool,
    pub finish_time_min: u32,
    pub finish_time_min_random_range: u32,
    pub finish_time_max: u32,
    pub finish_time_max_random_range: u32,
    pub course_set_status: Vec<u8>,
    pub corners: Vec<CourseCorner>,
    pub straights: Vec<CourseStraight>,
    pub slopes: Vec<CourseSlope>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CourseCorner {
    pub start: f32,
    pub length: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CourseLaneMaxEvent {
    pub start: f32,
    pub lane_max: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CourseStraight {
    pub start: f32,
    pub end: f32,
    pub front_type: u8,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CourseSlope {
    pub start: f32,
    pub length: f32,
    pub slope: i16,
}

#[derive(Debug, Deserialize)]
struct CourseEventFile {
    #[serde(rename = "courseParams")]
    course_params: Vec<CourseEventParam>,
}

#[derive(Debug, Deserialize)]
struct CourseEventParam {
    #[serde(rename = "_paramType")]
    param_type: i64,
    #[serde(rename = "_values")]
    values: Vec<i64>,
    #[serde(rename = "_distance")]
    distance: f64,
}

#[derive(Debug)]
struct CourseGeometry {
    lane_max_events: Vec<CourseLaneMaxEvent>,
    move_lane_point: f32,
    first_move_lane_is_in: bool,
    corners: Vec<CourseCorner>,
    straights: Vec<CourseStraight>,
    slopes: Vec<CourseSlope>,
}

pub fn generate<'a>(
    connection: &Connection,
    master_version: &'a str,
) -> Result<SimulatorCourseSet<'a>> {
    let status_by_id = load_course_set_statuses(connection)?;
    let event_params = load_course_event_params()?;
    let has_run_outside = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('race_course_set') WHERE name = 'run_outside')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    let course_query = if has_run_outside {
        r#"
        SELECT course.id, course.race_track_id, course.distance, course.ground,
               course.inout, course.turn, course.float_lane_max,
               course.course_set_status_id, course.finish_time_min,
               course.finish_time_min_random_range, course.finish_time_max,
               course.finish_time_max_random_range, track.initial_lane_type,
               track.enable_half_gate, course.run_outside
          FROM race_course_set AS course
          JOIN race_track AS track ON track.id = course.race_track_id
         ORDER BY course.id
        "#
    } else {
        r#"
        SELECT course.id, course.race_track_id, course.distance, course.ground,
               course.inout, course.turn, course.float_lane_max,
               course.course_set_status_id, course.finish_time_min,
               course.finish_time_min_random_range, course.finish_time_max,
               course.finish_time_max_random_range, track.initial_lane_type,
               track.enable_half_gate, 0 AS run_outside
          FROM race_course_set AS course
          JOIN race_track AS track ON track.id = course.race_track_id
         ORDER BY course.id
        "#
    };
    let mut statement = connection.prepare(course_query)?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, i64>(12)?,
            row.get::<_, i64>(13)?,
            row.get::<_, i64>(14)?,
        ))
    })?;

    let mut courses = Vec::new();
    for row in rows {
        let (
            course_id,
            race_track_id,
            distance,
            surface,
            course,
            turn,
            lane_max,
            course_set_status_id,
            finish_time_min,
            finish_time_min_random_range,
            finish_time_max,
            finish_time_max_random_range,
            initial_lane_type,
            enable_half_gate,
            run_outside,
        ) = row?;

        // Matches uma-tools: Longchamp 1000m data is incomplete, and 11202 has no event params.
        if course_id == 11201 || course_id == 11202 {
            continue;
        }

        let Some(geometry) = event_params.get(&course_id) else {
            warn!(
                course_id,
                "skipping simulator course without bundled event params"
            );
            continue;
        };

        courses.push(SimulatorCourse {
            course_id: as_u32(course_id, "id")?,
            race_track_id: as_u32(race_track_id, "race_track_id")?,
            initial_lane_type: as_u8(initial_lane_type, "initial_lane_type")?,
            enable_half_gate: enable_half_gate != 0,
            run_outside: run_outside != 0,
            distance: as_u16(distance, "distance")?,
            distance_type: distance_type(distance),
            surface: as_u8(surface, "ground")?,
            turn: as_u8(turn, "turn")?,
            course: as_u8(course, "inout")?,
            lane_max: as_u32(lane_max, "float_lane_max")?,
            lane_max_events: geometry.lane_max_events.clone(),
            move_lane_point: geometry.move_lane_point,
            first_move_lane_is_in: geometry.first_move_lane_is_in,
            finish_time_min: as_u32(finish_time_min, "finish_time_min")?,
            finish_time_min_random_range: as_u32(
                finish_time_min_random_range,
                "finish_time_min_random_range",
            )?,
            finish_time_max: as_u32(finish_time_max, "finish_time_max")?,
            finish_time_max_random_range: as_u32(
                finish_time_max_random_range,
                "finish_time_max_random_range",
            )?,
            course_set_status: status_by_id
                .get(&course_set_status_id)
                .cloned()
                .unwrap_or_default(),
            corners: geometry.corners.clone(),
            straights: geometry.straights.clone(),
            slopes: geometry.slopes.clone(),
        });
    }

    Ok(SimulatorCourseSet {
        schema_version: SCHEMA_VERSION,
        master_version,
        race_parameters: SimulatorRaceParameters::current(),
        courses,
    })
}

fn load_course_set_statuses(connection: &Connection) -> Result<BTreeMap<i64, Vec<u8>>> {
    let mut statement = connection.prepare(
        "SELECT course_set_status_id, target_status_1, target_status_2 FROM race_course_set_status ORDER BY course_set_status_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut statuses = BTreeMap::new();
    for row in rows {
        let (id, status_1, status_2) = row?;
        let mut values = vec![as_u8(status_1, "target_status_1")?];
        if status_2 != 0 {
            values.push(as_u8(status_2, "target_status_2")?);
        }
        statuses.insert(id, values);
    }
    Ok(statuses)
}

fn load_course_event_params() -> Result<BTreeMap<i64, CourseGeometry>> {
    let mut courses = BTreeMap::new();
    for (course_id, json) in COURSE_EVENT_PARAMS {
        let id = course_id
            .parse::<i64>()
            .with_context(|| format!("invalid bundled course event param id {course_id}"))?;
        if id == 11201 || id == 11202 {
            continue;
        }
        courses.insert(id, parse_course_event_params(id, json)?);
    }
    for (course_id, json) in GLOBAL_COURSE_EVENT_PARAM_OVERRIDES {
        let id = course_id
            .parse::<i64>()
            .with_context(|| format!("invalid bundled Global course event param id {course_id}"))?;
        courses.insert(id, parse_course_event_params(id, json)?);
    }
    Ok(courses)
}

fn parse_course_event_params(course_id: i64, json: &str) -> Result<CourseGeometry> {
    let file: CourseEventFile = serde_json::from_str(json)
        .with_context(|| format!("failed to parse course event params for {course_id}"))?;
    let mut corners = Vec::new();
    let mut lane_max_events = Vec::new();
    let mut explicit_move_lane_point: Option<(f32, bool)> = None;
    let mut straights = Vec::new();
    let mut slopes = Vec::new();
    let mut pending_straight: Option<(f32, u8)> = None;

    for event in file.course_params {
        match event.param_type {
            0 => corners.push(CourseCorner {
                start: distance_as_f32(event.distance, course_id, "corner.start")?,
                length: event_value_as_f32(&event, 1, course_id, "corner.length")?,
            }),
            3 => lane_max_events.push(CourseLaneMaxEvent {
                start: distance_as_f32(event.distance, course_id, "lane_max_event.start")?,
                lane_max: as_u32(
                    event_value(&event, 0, course_id, "lane_max_event.lane_max")?,
                    "lane_max_event.lane_max",
                )?,
            }),
            7 => {
                let direction = event_value(&event, 0, course_id, "move_lane_point.direction")?;
                if direction != 0 {
                    let point =
                        distance_as_f32(event.distance, course_id, "move_lane_point.distance")?;
                    if explicit_move_lane_point.map_or(true, |(current, _)| point < current) {
                        explicit_move_lane_point = Some((point, direction == 1));
                    }
                }
            }
            2 => {
                match event_value(&event, 0, course_id, "straight marker")? {
                    1 => {
                        if pending_straight.is_some() {
                            bail!("course {course_id} started a straight before ending the previous one");
                        }
                        pending_straight = Some((
                            distance_as_f32(event.distance, course_id, "straight.start")?,
                            event_value_as_u8(&event, 1, course_id, "straight.front_type")?,
                        ));
                    }
                    2 => {
                        let Some((start, front_type)) = pending_straight.take() else {
                            bail!("course {course_id} ended a straight before starting one");
                        };
                        straights.push(CourseStraight {
                            start,
                            end: distance_as_f32(event.distance, course_id, "straight.end")?,
                            front_type,
                        });
                    }
                    marker => bail!("course {course_id} has unsupported straight marker {marker}"),
                }
            }
            11 => slopes.push(CourseSlope {
                start: distance_as_f32(event.distance, course_id, "slope.start")?,
                length: event_value_as_f32(&event, 1, course_id, "slope.length")?,
                slope: event_value_as_i16(&event, 0, course_id, "slope.slope")?,
            }),
            _ => {}
        }
    }

    if pending_straight.is_some() {
        bail!("course {course_id} has an unterminated straight");
    }

    corners.sort_by(|left, right| left.start.total_cmp(&right.start));
    lane_max_events.sort_by(|left, right| left.start.total_cmp(&right.start));
    straights.sort_by(|left, right| left.start.total_cmp(&right.start));
    slopes.sort_by(|left, right| left.start.total_cmp(&right.start));

    let (move_lane_point, first_move_lane_is_in) = explicit_move_lane_point.unwrap_or_else(|| {
        corners
            .first()
            .filter(|corner| corner.start > 0.0)
            .map(|corner| (corner.start, true))
            .unwrap_or((0.0, false))
    });

    Ok(CourseGeometry {
        lane_max_events,
        move_lane_point,
        first_move_lane_is_in,
        corners,
        straights,
        slopes,
    })
}

fn distance_type(distance: i64) -> u8 {
    match distance {
        ..=1400 => 1,
        1401..=1800 => 2,
        1801..=2499 => 3,
        2500.. => 4,
    }
}

fn event_value(event: &CourseEventParam, index: usize, course_id: i64, field: &str) -> Result<i64> {
    event
        .values
        .get(index)
        .copied()
        .ok_or_else(|| anyhow!("course {course_id} missing {field}"))
}

fn event_value_as_u8(
    event: &CourseEventParam,
    index: usize,
    course_id: i64,
    field: &str,
) -> Result<u8> {
    as_u8(event_value(event, index, course_id, field)?, field)
}

fn event_value_as_f32(
    event: &CourseEventParam,
    index: usize,
    course_id: i64,
    field: &str,
) -> Result<f32> {
    let value = event_value(event, index, course_id, field)? as f32;
    if !value.is_finite() {
        bail!("course {course_id} has non-finite {field} value {value}");
    }
    Ok(value)
}

fn event_value_as_i16(
    event: &CourseEventParam,
    index: usize,
    course_id: i64,
    field: &str,
) -> Result<i16> {
    i16::try_from(event_value(event, index, course_id, field)?)
        .with_context(|| format!("{field} is out of i16 range"))
}

fn distance_as_f32(distance: f64, course_id: i64, field: &str) -> Result<f32> {
    if !distance.is_finite() {
        bail!("course {course_id} has non-finite {field} distance {distance}");
    }
    Ok(distance as f32)
}

fn as_u8(value: i64, field: &str) -> Result<u8> {
    u8::try_from(value).with_context(|| format!("{field} value {value} is out of u8 range"))
}

fn as_u16(value: i64, field: &str) -> Result<u16> {
    u16::try_from(value).with_context(|| format!("{field} value {value} is out of u16 range"))
}

fn as_u32(value: i64, field: &str) -> Result<u32> {
    u32::try_from(value).with_context(|| format!("{field} value {value} is out of u32 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compete_fight_constants_match_decoded_global_asset() {
        let asset: serde_json::Value =
            serde_json::from_str(include_str!("../global_data/raceparams/10006800.json")).unwrap();
        let compete_fight = &asset["CompeteFight"];

        for (field, expected) in [
            ("DistanceGap", COMPETE_FIGHT_TARGET_DISTANCE_M),
            ("LaneGap", COMPETE_FIGHT_TARGET_LANE_COURSE_WIDTHS),
            ("TargetContinueTime", COMPETE_FIGHT_TARGET_CONTINUE_SECONDS),
            ("SpeedGap", COMPETE_FIGHT_TARGET_SPEED_GAP_MPS),
            ("TargetContinueDistance", COMPETE_FIGHT_EXIT_DISTANCE_M),
            ("AddParam1Coef1", COMPETE_FIGHT_TARGET_SPEED_BASE),
            ("AddParam1Coef2", COMPETE_FIGHT_TARGET_SPEED_POWER),
            ("AddParam1Coef3", COMPETE_FIGHT_TARGET_SPEED_SCALE),
            ("AddParam2Coef1", COMPETE_FIGHT_ACCELERATION_BASE),
            ("AddParam2Coef2", COMPETE_FIGHT_ACCELERATION_POWER),
            ("AddParam2Coef3", COMPETE_FIGHT_ACCELERATION_SCALE),
        ] {
            assert_eq!(
                compete_fight[field].as_f64().map(|value| value as f32),
                Some(expected as f32),
                "{field}"
            );
        }
        for (field, expected) in [
            (
                "TargetOrderPer",
                u64::from(COMPETE_FIGHT_TARGET_ORDER_PERCENT),
            ),
            ("HpPer", u64::from(COMPETE_FIGHT_END_HP_PERCENT)),
            ("HpPer2", u64::from(COMPETE_FIGHT_START_HP_PERCENT)),
        ] {
            assert_eq!(compete_fight[field].as_u64(), Some(expected), "{field}");
        }
    }

    #[test]
    fn base_motion_constants_match_decoded_global_asset() {
        let asset: serde_json::Value =
            serde_json::from_str(include_str!("../global_data/raceparams/10006800.json")).unwrap();

        for (field, expected) in [
            ("raceBaseSpeedBaseSpeed", RACE_BASE_SPEED),
            ("raceBaseSpeedBaseDistance", RACE_BASE_DISTANCE_M),
            ("raceBaseSpeedDistanceRate", RACE_BASE_DISTANCE_RATE_M),
            ("addSpeedParamCoef", PHASE_END_SPEED_SCALE),
        ] {
            assert_eq!(
                asset[field].as_f64().map(|value| value as f32),
                Some(expected as f32),
                "{field}"
            );
        }
        for (field, expected) in [
            (
                "BaseTargetSpeedRandomMinusVal1",
                BASE_TARGET_SPEED_RANDOM_MINUS_BASE_PERCENT,
            ),
            (
                "BaseTargetSpeedRandomMinusVal2",
                BASE_TARGET_SPEED_RANDOM_MINUS_WISDOM_DIVISOR,
            ),
            (
                "BaseTargetSpeedRandomPlusVal1",
                BASE_TARGET_SPEED_RANDOM_PLUS_WISDOM_DIVISOR,
            ),
            (
                "PhaseEndBaseTargetSpeedCoef",
                PHASE_END_SPEED_SQRT_COEFFICIENT,
            ),
        ] {
            assert_eq!(
                asset["BasetTargetSpeed"][field]
                    .as_f64()
                    .map(|value| value as f32),
                Some(expected as f32),
                "{field}"
            );
        }
        for (strategy, source_index) in (1..=4).zip(0..) {
            for (phase, field) in ["Start", "Middle", "End", "Last"].into_iter().enumerate() {
                assert_eq!(
                    asset["BasetTargetSpeed"]["PhaseBaseTargetSpeedPerArray"][source_index][field]
                        .as_f64()
                        .map(|value| value as f32),
                    Some(PHASE_TARGET_SPEED_PERCENT[strategy][phase] as f32),
                    "strategy {strategy} phase {field}"
                );
            }
        }
        for (phase, field) in ["Start", "Middle", "End", "Last"].into_iter().enumerate() {
            assert_eq!(
                asset["BasetTargetSpeed"]["PhaseBaseTargetSpeedPerExArray"][0][field]
                    .as_f64()
                    .map(|value| value as f32),
                Some(PHASE_TARGET_SPEED_PERCENT[5][phase] as f32),
                "Oonige phase {field}"
            );
        }
    }

    #[test]
    fn acceleration_constants_match_decoded_global_asset() {
        let asset: serde_json::Value =
            serde_json::from_str(include_str!("../global_data/raceparams/10006800.json")).unwrap();

        assert_eq!(
            asset["accelPowCoef"].as_f64().map(|value| value as f32),
            Some(ACCEL_POWER_COEFFICIENT as f32)
        );
        assert_eq!(
            asset["accelPowCoefUpSlope"]
                .as_f64()
                .map(|value| value as f32),
            Some(ACCEL_UPHILL_POWER_COEFFICIENT as f32)
        );
        assert_eq!(
            asset["AccelPowCoefSqrt"].as_f64().map(|value| value as f32),
            Some(ACCEL_POWER_SQRT_COEFFICIENT as f32)
        );
        assert_eq!(
            asset["StartAccelAdd"].as_f64().map(|value| value as f32),
            Some(ACCEL_START_DASH_ADD as f32)
        );
        for (strategy, source_index) in (1..=4).zip(0..) {
            for (phase, field) in ["Start", "Middle", "End"].into_iter().enumerate() {
                assert_eq!(
                    asset["Speed"]["PhaseAccelCoefArray"][source_index][field]
                        .as_f64()
                        .map(|value| value as f32),
                    Some(ACCEL_PHASE_COEFFICIENTS[strategy][phase] as f32)
                );
            }
        }
        for (phase, field) in ["Start", "Middle", "End"].into_iter().enumerate() {
            assert_eq!(
                asset["Speed"]["PhaseAccelCoefExArray"][0][field]
                    .as_f64()
                    .map(|value| value as f32),
                Some(ACCEL_PHASE_COEFFICIENTS[5][phase] as f32)
            );
        }
    }

    #[test]
    fn parses_and_sorts_lane_max_events() {
        let geometry = parse_course_event_params(
            99999,
            r#"{
                "courseParams": [
                    { "_paramType": 3, "_values": [7000], "_distance": 875.0 },
                    { "_paramType": 0, "_values": [1, 275], "_distance": 400.0 },
                    { "_paramType": 3, "_values": [10000], "_distance": 500.0 }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(
            geometry.lane_max_events,
            vec![
                CourseLaneMaxEvent {
                    start: 500.0,
                    lane_max: 10_000,
                },
                CourseLaneMaxEvent {
                    start: 875.0,
                    lane_max: 7_000,
                },
            ]
        );
        assert_eq!(geometry.move_lane_point, 400.0);
        assert!(geometry.first_move_lane_is_in);
    }

    #[test]
    fn explicit_move_lane_point_overrides_corner_fallback_and_keeps_direction() {
        let geometry = parse_course_event_params(
            99999,
            r#"{
                "courseParams": [
                    { "_paramType": 0, "_values": [1, 275], "_distance": 400.0 },
                    { "_paramType": 7, "_values": [2], "_distance": 30.0 }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(geometry.move_lane_point, 30.0);
        assert!(!geometry.first_move_lane_is_in);
    }

    #[test]
    fn global_course_event_override_replaces_jp_geometry() {
        let courses = load_course_event_params().unwrap();
        let geometry = courses.get(&10501).unwrap();

        assert_eq!(geometry.corners.len(), 2);
        assert_eq!(geometry.corners[0].start, 300.0);
        assert_eq!(geometry.corners[0].length, 350.0);
        assert_eq!(geometry.corners[1].start, 650.0);
        assert_eq!(geometry.corners[1].length, 240.0);
        assert_eq!(geometry.straights.len(), 1);
        assert_eq!(geometry.straights[0].start, 890.0);
        assert_eq!(geometry.straights[0].end, 1200.0);
    }

    #[test]
    fn emits_initial_lane_master_data() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE race_track (
                    id INTEGER PRIMARY KEY,
                    initial_lane_type INTEGER NOT NULL,
                    enable_half_gate INTEGER NOT NULL
                );
                CREATE TABLE race_course_set (
                    id INTEGER PRIMARY KEY,
                    race_track_id INTEGER NOT NULL,
                    distance INTEGER NOT NULL,
                    ground INTEGER NOT NULL,
                    inout INTEGER NOT NULL,
                    turn INTEGER NOT NULL,
                    float_lane_max INTEGER NOT NULL,
                    course_set_status_id INTEGER NOT NULL,
                    finish_time_min INTEGER NOT NULL,
                    finish_time_min_random_range INTEGER NOT NULL,
                    finish_time_max INTEGER NOT NULL,
                    finish_time_max_random_range INTEGER NOT NULL,
                    run_outside INTEGER NOT NULL
                );
                CREATE TABLE race_course_set_status (
                    course_set_status_id INTEGER PRIMARY KEY,
                    target_status_1 INTEGER NOT NULL,
                    target_status_2 INTEGER NOT NULL
                );
                INSERT INTO race_track VALUES (10001, 4, 1);
                INSERT INTO race_course_set
                    VALUES (10104, 10001, 2000, 1, 1, 1, 13500, 1, 1171000, 10000, 1230000, 10000, 1);
                INSERT INTO race_course_set_status VALUES (1, 3, 0);
                "#,
            )
            .unwrap();

        let generated = generate(&connection, "test").unwrap();
        let course = &generated.courses[0];

        assert_eq!(generated.schema_version, 4);
        assert_eq!(generated.race_parameters.schema_version, 22);
        assert_eq!(generated.race_parameters.start_delay_max_seconds, 0.1);
        assert_eq!(generated.race_parameters.target_speed_min, 13.0);
        assert_eq!(
            generated.race_parameters.compete_fight.target_speed_gap_mps,
            f64::from(0.6_f32)
        );
        assert_eq!(
            generated.race_parameters.compete_fight.target_speed_power,
            f64::from(0.708_f32)
        );
        assert_eq!(
            generated.race_parameters.compete_fight.acceleration_power,
            f64::from(0.59_f32)
        );
        assert_eq!(
            generated.race_parameters.acceleration.power_coefficient,
            f64::from(0.0006_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .acceleration
                .uphill_power_coefficient,
            f64::from(0.0004_f32)
        );
        assert_eq!(
            generated.race_parameters.acceleration.phase_coefficients[5],
            [
                f64::from(1.17_f32),
                f64::from(0.94_f32),
                f64::from(0.956_f32),
            ]
        );
        assert_eq!(
            generated
                .race_parameters
                .base_motion
                .phase_target_speed_percent[2],
            [f64::from(97.8_f32), f64::from(99.1_f32), 97.5, 97.5,]
        );
        assert_eq!(generated.race_parameters.temptation.lot_section_min, 2);
        assert_eq!(generated.race_parameters.temptation.lot_section_max, 9);
        assert_eq!(generated.race_parameters.temptation.force_end_seconds, 12.0);
        assert_eq!(
            generated.race_parameters.temptation.end_chance_percent,
            55.0
        );
        assert_eq!(
            generated
                .race_parameters
                .additional_activation
                .order_up_max_count,
            3
        );
        assert_eq!(
            generated
                .race_parameters
                .additional_activation
                .activate_any_skill_type_2_max_count,
            3
        );
        assert_eq!(
            generated
                .race_parameters
                .additional_activation
                .activate_any_skill_type_3_max_count,
            2
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_timing
                .ability_time_divide_distance_m,
            1000
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_timing
                .cooldown_time_divide_distance_m,
            1000
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_timing
                .order_up_add_ability_time_seconds,
            1.0
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_timing
                .order_up_add_ability_time_max_count,
            3
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_timing
                .duration_scaling
                .distance_diff_top_min_multiplier,
            0.800_000_011_920_929
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_timing
                .duration_scaling
                .remain_hp_type_1[3]
                .multiplier,
            2.200_000_047_683_716
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_timing
                .duration_scaling
                .blocked_time_type_1[3]
                .upper_bound,
            999.0
        );
        assert!(generated
            .race_parameters
            .skill_timing
            .duration_scaling
            .blocked_time_type_2
            .is_empty());
        assert_eq!(
            generated
                .race_parameters
                .skill_value_scaling
                .aoharu_team_total_status[0]
                .multiplier,
            f64::from(0.8_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_value_scaling
                .random_ability_value_type_1[2]
                .weight,
            10
        );
        assert_eq!(
            generated
                .race_parameters
                .skill_value_scaling
                .distance_diff_top_additive[1]
                .additive,
            f64::from(0.1_f32)
        );
        assert!(generated
            .race_parameters
            .skill_value_scaling
            .blocked_time_type_2
            .is_empty());
        assert_eq!(
            generated.race_parameters.near_horse.distance_threshold_m,
            3.0
        );
        assert_eq!(
            generated
                .race_parameters
                .near_horse
                .lane_distance_threshold_m,
            1.875
        );
        assert_eq!(
            generated
                .race_parameters
                .extra_move_lane
                .start_final_corner_rate,
            0.0
        );
        assert_eq!(
            generated
                .race_parameters
                .extra_move_lane
                .lane_normalize_course_widths,
            0.1
        );
        assert_eq!(
            generated
                .race_parameters
                .extra_move_lane
                .lane_coefficient_course_widths,
            0.5
        );
        assert_eq!(
            generated
                .race_parameters
                .extra_move_lane
                .lane_random_max_course_widths,
            0.1
        );
        assert_eq!(
            generated.race_parameters.lane_movement.speed_base,
            f64::from(0.02_f32)
        );
        assert_eq!(
            generated.race_parameters.lane_movement.speed_add,
            f64::from(0.3_f32)
        );
        assert_eq!(
            generated.race_parameters.lane_movement.power_coefficient,
            f64::from(0.001_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .lane_movement
                .outside_position_coefficient,
            f64::from(0.05_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .lane_movement
                .acceleration_base_coefficient,
            1.5
        );
        assert_eq!(
            generated
                .race_parameters
                .overtake_lane
                .final_in_lane_coefficient,
            1.0
        );
        assert_eq!(
            generated
                .race_parameters
                .overtake_lane
                .final_out_lane_coefficient,
            1.15
        );
        assert_eq!(
            generated
                .race_parameters
                .overtake_lane
                .target_cooldown_seconds,
            1.5
        );
        assert_eq!(
            generated
                .race_parameters
                .overtake_lane
                .order_change_count_cooldown_seconds,
            6.0
        );
        assert_eq!(
            generated
                .race_parameters
                .around_horse
                .block
                .front_lane_course_widths,
            f64::from(0.041_666_672_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .around_horse
                .surrounded
                .directional_lane_course_widths,
            f64::from(0.083_33_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .around_horse
                .congestion
                .lane_course_widths,
            f64::from(0.3_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .around_horse
                .congestion
                .horse_count_threshold,
            3
        );
        assert_eq!(
            generated
                .race_parameters
                .skill
                .infront_horse_near_distance_m,
            2.5
        );
        assert_eq!(
            generated
                .race_parameters
                .skill
                .behind_horse_near_lane_course_widths,
            f64::from(0.0556_f32)
        );
        assert_eq!(
            generated.race_parameters.skill.behind_near_parameter_sets,
            &[SimulatorNearLaneParameters {
                distance_m: 5.0,
                lane_course_widths: f64::from(0.15_f32),
            }]
        );
        assert_eq!(generated.race_parameters.deceleration.base, 1.0);
        assert_eq!(
            generated.race_parameters.deceleration.phase_rates,
            [1.2, 0.8, 1.0]
        );
        assert_eq!(generated.race_parameters.deceleration.hp_zero_rate, 1.2);
        assert_eq!(generated.race_parameters.deceleration.pace_down_rate, 0.5);
        assert_eq!(generated.race_parameters.minimum_speed.start_speed, 3.0);
        assert_eq!(
            generated.race_parameters.minimum_speed.base_speed_rate,
            f64::from(0.85_f32)
        );
        assert_eq!(
            generated
                .race_parameters
                .minimum_speed
                .guts_sqrt_coefficient,
            200.0
        );
        assert_eq!(
            generated.race_parameters.minimum_speed.guts_coefficient,
            f64::from(0.001_f32)
        );
        assert_eq!(
            generated.race_parameters.hp.max_hp_strategy_coefficients,
            [0.0, 0.95, 0.89, 1.0, 0.995, 0.86]
        );
        assert_eq!(generated.race_parameters.hp.consumption_base, 20.0);
        assert_eq!(generated.race_parameters.hp.speed_gap_offset, 12.0);
        assert_eq!(generated.race_parameters.hp.speed_gap_square_divisor, 144.0);
        assert_eq!(generated.race_parameters.hp.rushed_rate, 1.6);
        assert_eq!(generated.race_parameters.hp.pace_down_rate, 0.6);
        assert_eq!(
            generated.race_parameters.hp.ground_condition_rates[2],
            [0.0, 1.0, 1.0, 1.01, 1.02]
        );
        assert_eq!(
            generated
                .race_parameters
                .last_spurt
                .start_distance_max_from_goal_m,
            100.0
        );
        assert_eq!(
            generated
                .race_parameters
                .last_spurt
                .candidate_speed_delta_mps,
            f64::from(0.1_f32)
        );
        assert_eq!(generated.race_parameters.slope.check_interval_seconds, 1.0);
        assert_eq!(
            generated.race_parameters.slope.uphill_add_speed_value1,
            200.0
        );
        assert_eq!(
            generated.race_parameters.slope.downhill_add_speed_value1,
            0.3
        );
        assert_eq!(
            generated.race_parameters.slope.downhill_add_speed_value2,
            10.0
        );
        assert_eq!(
            generated
                .race_parameters
                .slope
                .downhill_start_wisdom_percent_coefficient,
            0.04
        );
        assert_eq!(
            generated.race_parameters.slope.downhill_end_chance_percent,
            20.0
        );
        assert_eq!(
            generated
                .race_parameters
                .force_in
                .lane_threshold_course_widths,
            0.12
        );
        assert_eq!(
            generated.race_parameters.force_in.target_speed_add_nige_mps,
            0.02
        );
        assert_eq!(
            generated
                .race_parameters
                .force_in
                .target_speed_add_random_range_mps,
            0.1
        );
        assert_eq!(
            generated
                .race_parameters
                .position_keep
                .check_interval_seconds,
            2.0
        );
        assert_eq!(
            generated.race_parameters.position_keep.cooldown_seconds,
            1.0
        );
        assert_eq!(generated.race_parameters.position_keep.start_section, 1);
        assert_eq!(generated.race_parameters.position_keep.end_section, 10);
        assert_eq!(
            generated
                .race_parameters
                .position_keep
                .oonige_continue_section,
            3
        );
        assert_eq!(
            generated
                .race_parameters
                .position_keep
                .speed_up_only_nige_end_distance_m,
            12.5
        );
        assert_eq!(
            generated
                .race_parameters
                .position_keep
                .oonige_overtake_end_distance_m,
            27.5
        );
        assert_eq!(
            generated
                .race_parameters
                .position_keep
                .pacemaker
                .forward_running_style_range_m,
            10.0
        );
        assert_eq!(
            generated
                .race_parameters
                .position_keep
                .pacemaker
                .most_forward_style_range_m,
            10.0
        );
        assert_eq!(
            generated
                .race_parameters
                .position_keep
                .pacemaker
                .top_not_most_forward_style_count,
            2
        );
        assert_eq!(
            generated.race_parameters.conserve_power.power_threshold,
            1200.0
        );
        assert_eq!(
            generated
                .race_parameters
                .conserve_power
                .strategy_distance_acceleration_coefficients[2][2],
            0.875
        );
        assert_eq!(
            generated
                .race_parameters
                .conserve_power
                .duration_distance_coefficients,
            [0.45, 1.0, 0.875, 0.8]
        );
        assert_eq!(course.initial_lane_type, 4);
        assert!(course.enable_half_gate);
        assert!(course.run_outside);
        assert_eq!(course.move_lane_point, 375.0);
        assert!(course.first_move_lane_is_in);
        assert_eq!(course.finish_time_min_random_range, 10_000);
        assert_eq!(course.finish_time_max_random_range, 10_000);
    }
}
