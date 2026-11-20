// Doc 48 (`EX.1`–`EX.8`, `PL.1`–`PL.9`) Economy, 24-Week Price History, Open Jobs,
// Shipyards, Works, Bay & Parts Market helpers for the Web Corridor Port UI.

export interface EconomyGoodOption {
  code: string;
  label: string;
}

export interface IndustryRecipeOption {
  id: string;
  label: string;
  inputsSummary: string;
  outputSummary: string;
  heavyYardOnly?: boolean;
}

export interface PriceHistoryRow {
  when: string;
  askText: string;
  bidText: string;
  askMajor: number | null;
  bidMajor: number | null;
}

export interface ParsedPriceHistory {
  goodLabel: string | null;
  rowsNewestFirst: PriceHistoryRow[];
  askSparkline: string;
  bidSparkline: string;
  latestAsk: string;
  latestBid: string;
  askRange: string;
  bidRange: string;
}

export const HISTORY_GOODS: readonly EconomyGoodOption[] = [
  { code: "volatiles", label: "Volatiles & Ice" },
  { code: "metals", label: "Refined Metals" },
  { code: "structural_alloys", label: "Structural Alloys" },
  { code: "industrial_parts", label: "Industrial Parts" },
  { code: "carbon_feedstock", label: "Carbon Feedstock" },
  { code: "instrument_boards", label: "Instrument Boards" },
  { code: "propellant", label: "Reaction Mass" },
  { code: "provisions", label: "Crew Provisions" },
];

export const JOB_BOARD_FILTERS: readonly { id: string; label: string; cmd: string }[] = [
  { id: "all", label: "All Port Jobs", cmd: "+market/jobs" },
  { id: "haul", label: "Haulage", cmd: "+market/jobs haul" },
  { id: "escort", label: "Armed Escort", cmd: "+market/jobs escort" },
  { id: "survey", label: "Survey", cmd: "+market/jobs survey" },
  { id: "retrieval", label: "Salvage Retrieval", cmd: "+market/jobs retrieval" },
  { id: "passage", label: "Passenger", cmd: "+market/jobs passage" },
];

/**
 * Every works recipe in `crates/tacsim/content/industry/recipes-v2.json`, id for id, with
 * its label and its inputs and outputs (48e L8, clients:G11). `economy.test.ts` holds this
 * list to the file, so a recipe the content adds or renames fails there.
 */
export const WORKS_RECIPES: readonly IndustryRecipeOption[] = [
  {
    id: "ap_30",
    label: "30 mm AP rounds",
    inputsSummary: "1 structural_alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "500 ammo_ap_30",
  },
  {
    id: "ball_762",
    label: "7.62 mm ball, boxed",
    inputsSummary: "1 structural_alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "240 ammo_ball_762",
  },
  {
    id: "carbon_feedstock",
    label: "Carbon feedstock pressing",
    inputsSummary: "5 carbon_ore",
    outputSummary: "4 carbon_feedstock",
  },
  {
    id: "fittings",
    label: "Fittings batch",
    inputsSummary: "5 structural_alloys + 1 carbon_ore",
    outputSummary: "5 industrial_parts",
  },
  {
    id: "he_105",
    label: "105 mm HE rounds",
    inputsSummary: "1 structural_alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "40 ammo_he_105",
  },
  {
    id: "metals",
    label: "Metal ore milling",
    inputsSummary: "10 metal_ore",
    outputSummary: "4 metals",
  },
  {
    id: "missile",
    label: "Anti-armor missiles",
    inputsSummary: "1 structural_alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "4 ammo_missile",
  },
  {
    id: "module_command",
    label: "Command core module",
    inputsSummary: "4 instrument_boards + 2 strategic_ore",
    outputSummary: "1 \u00d7 Command core module",
  },
  {
    id: "module_doctrine",
    label: "Doctrine core module",
    inputsSummary: "1 instrument_boards + 1 strategic_ore",
    outputSummary: "1 \u00d7 Doctrine core module",
  },
  {
    id: "module_line",
    label: "Line core module",
    inputsSummary: "2 instrument_boards + 1 strategic_ore",
    outputSummary: "1 \u00d7 Line core module",
  },
  {
    id: "mount_105",
    label: "Main-gun mount",
    inputsSummary: "3 structural_alloys + 1 industrial_parts",
    outputSummary: "1 \u00d7 Main-gun mount",
  },
  {
    id: "mount_30",
    label: "Autocannon mount",
    inputsSummary: "1 structural_alloys + 1 industrial_parts",
    outputSummary: "1 \u00d7 Autocannon mount",
  },
  {
    id: "mount_laser",
    label: "Laser mount",
    inputsSummary: "2 structural_alloys + 2 industrial_parts + 1 instrument_boards",
    outputSummary: "1 \u00d7 Laser mount",
  },
  {
    id: "mount_mg",
    label: "Coaxial mounts",
    inputsSummary: "1 industrial_parts",
    outputSummary: "10 \u00d7 Coaxial mounts",
  },
  {
    id: "pd_box",
    label: "Point-defense rounds, boxed",
    inputsSummary: "1 structural_alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "24 pd_box",
  },
  {
    id: "plant_hauler",
    label: "Hauler plant",
    inputsSummary: "25 structural_alloys + 8 industrial_parts + 2 carbon_ore",
    outputSummary: "1 \u00d7 Hauler plant",
  },
  {
    id: "plant_light",
    label: "Light plants",
    inputsSummary: "1 industrial_parts + 1 carbon_ore",
    outputSummary: "5 \u00d7 Light plants",
  },
  {
    id: "plant_vehicle",
    label: "Vehicle plant",
    inputsSummary: "2 structural_alloys + 1 industrial_parts + 1 carbon_ore",
    outputSummary: "1 \u00d7 Vehicle plant",
  },
  {
    id: "plant_walker",
    label: "Walker plant",
    inputsSummary: "3 structural_alloys + 1 industrial_parts + 1 carbon_ore",
    outputSummary: "1 \u00d7 Walker plant",
  },
  {
    id: "pod_eccm",
    label: "ECCM pods",
    inputsSummary: "3 strategic_ore + 2 instrument_boards + 2 industrial_parts",
    outputSummary: "10 \u00d7 ECCM pods",
  },
  {
    id: "pod_ecm",
    label: "ECM pods",
    inputsSummary: "3 strategic_ore + 2 instrument_boards + 2 industrial_parts",
    outputSummary: "10 \u00d7 ECM pods",
  },
  {
    id: "rack_lr",
    label: "Long-range rack",
    inputsSummary: "1 structural_alloys + 1 industrial_parts",
    outputSummary: "1 \u00d7 Long-range rack",
  },
  {
    id: "sight_thermal",
    label: "Thermal sights",
    inputsSummary: "3 strategic_ore + 2 instrument_boards + 3 industrial_parts",
    outputSummary: "10 \u00d7 Thermal sights",
  },
  {
    id: "sight_visual",
    label: "Visual sights",
    inputsSummary: "2 strategic_ore + 1 instrument_boards + 3 industrial_parts",
    outputSummary: "10 \u00d7 Visual sights",
  },
  {
    id: "spinal_round",
    label: "Spinal gun rounds",
    inputsSummary: "1 structural_alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "10 spinal_round",
  },
  {
    id: "structural_alloys",
    label: "Alloy refining",
    inputsSummary: "5 metals",
    outputSummary: "4 structural_alloys",
  },
  {
    id: "torpedo",
    label: "Space torpedoes",
    inputsSummary: "1 structural_alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "2 torpedo",
  },
  {
    id: "volatiles",
    label: "Ice ore milling",
    inputsSummary: "20 ice_ore",
    outputSummary: "8 volatiles",
  },
];

export const YARD_RECIPES: readonly IndustryRecipeOption[] = [
  {
    id: "dray",
    label: "Dray Bulk Hauler",
    inputsSummary: "163 structural_alloys + 55 industrial_parts",
    outputSummary: "Ceres & Stickney Yards",
  },
  {
    id: "dray_p",
    label: "Dray-P Prospector Carrier",
    inputsSummary: "163 structural_alloys + 55 industrial_parts",
    outputSummary: "Ceres & Stickney Yards",
  },
  {
    id: "line_tank",
    label: "Line Tank Chassis",
    inputsSummary: "20 structural_alloys + 7 industrial_parts",
    outputSummary: "Stickney Heavy Yard",
    heavyYardOnly: true,
  },
  {
    id: "line_tank_laser",
    label: "Laser Line Tank Chassis",
    inputsSummary: "19 structural_alloys + 7 industrial_parts",
    outputSummary: "Stickney Heavy Yard",
    heavyYardOnly: true,
  },
  {
    id: "heavy_mk1",
    label: "Heavy Tank Mk1 Chassis",
    inputsSummary: "34 structural_alloys + 12 industrial_parts",
    outputSummary: "Stickney Heavy Yard",
    heavyYardOnly: true,
  },
  {
    id: "biped_mk1",
    label: "Biped Mech Mk1 Chassis",
    inputsSummary: "34 structural_alloys + 15 industrial_parts",
    outputSummary: "Stickney Heavy Yard",
    heavyYardOnly: true,
  },
  {
    id: "biped_mk2",
    label: "Biped Mech Mk2 Chassis",
    inputsSummary: "37 structural_alloys + 16 industrial_parts",
    outputSummary: "Stickney Heavy Yard",
    heavyYardOnly: true,
  },
];

const SPARK_BLOCKS = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"] as const;

function parseMajorNumber(priceText: string): number | null {
  const trimmed = priceText.trim();
  if (!trimmed || trimmed.toLowerCase() === "none") return null;
  const m = trimmed.replace(/,/g, "").match(/(-?\d+(?:\.\d+)?)/);
  if (!m) return null;
  const val = Number.parseFloat(m[1]!);
  return Number.isFinite(val) ? val : null;
}

export function buildSparkline(valuesOldestFirst: readonly (number | null)[]): string {
  const nums = valuesOldestFirst.filter((v): v is number => v !== null);
  if (nums.length === 0) return "—";
  const min = Math.min(...nums);
  const max = Math.max(...nums);
  const span = max - min;
  return valuesOldestFirst
    .map((v) => {
      if (v === null) return "·";
      if (span <= 0.0001) return SPARK_BLOCKS[4];
      const idx = Math.max(
        0,
        Math.min(7, Math.round(((v - min) / span) * 7)),
      );
      return SPARK_BLOCKS[idx];
    })
    .join("");
}

/**
 * Parses canonical `mushd` `+market/history tacsim:<good>` lines:
 *   `House prices of Volatiles here, by IC week, newest first (4):`
 *   `  this IC week: ask 108.00 cr, bid 90.00 cr`
 *   `  last IC week: ask 102.00 cr, bid 86.00 cr`
 */
const HISTORY_HEAD_RE = /House prices of (.+?) here, by IC week, newest first \((\d+)\):/i;
const HISTORY_ROW_RE =
  /^\s*(this IC week|last IC week|\d+\s+IC weeks ago):\s*ask\s+([^,]+),\s*bid\s+(.+)$/i;

/**
 * Folds one server line into the `+market/history` reply accumulator (48e L8,
 * clients:G5). A heading starts a fresh reply; a week row joins the reply under
 * way until the heading's count is reached. Any other line leaves it alone, so
 * the reply is whole however long the series and whatever else the log shows.
 * Returns the new accumulator, or `null` when the line is not part of a reply.
 */
export function foldMarketHistoryLine(acc: readonly string[], line: string): string[] | null {
  if (HISTORY_HEAD_RE.test(line)) return [line];
  if (acc.length === 0 || !HISTORY_ROW_RE.test(line)) return null;
  const head = acc[0]!.match(HISTORY_HEAD_RE);
  const want = head ? Number(head[2]) : 0;
  if (acc.length - 1 >= want) return null;
  return [...acc, line];
}

export function parseMarketHistoryLines(lines: readonly string[]): ParsedPriceHistory {
  let goodLabel: string | null = null;
  const rows: PriceHistoryRow[] = [];

  for (const raw of lines) {
    const headMatch = raw.match(HISTORY_HEAD_RE);
    if (headMatch) {
      goodLabel = headMatch[1]!.trim();
      rows.length = 0;
      continue;
    }

    const rowMatch = raw.match(HISTORY_ROW_RE);
    if (rowMatch) {
      const when = rowMatch[1]!.trim();
      const askText = rowMatch[2]!.trim();
      const bidText = rowMatch[3]!.trim();
      rows.push({
        when,
        askText,
        bidText,
        askMajor: parseMajorNumber(askText),
        bidMajor: parseMajorNumber(bidText),
      });
    }
  }

  const oldestFirst = rows.slice().reverse();
  const askValues = oldestFirst.map((r) => r.askMajor);
  const bidValues = oldestFirst.map((r) => r.bidMajor);

  const validAsks = askValues.filter((v): v is number => v !== null);
  const validBids = bidValues.filter((v): v is number => v !== null);

  return {
    goodLabel,
    rowsNewestFirst: rows,
    askSparkline: buildSparkline(askValues),
    bidSparkline: buildSparkline(bidValues),
    latestAsk: rows[0]?.askText ?? "—",
    latestBid: rows[0]?.bidText ?? "—",
    askRange:
      validAsks.length > 0
        ? `${Math.min(...validAsks).toFixed(2)} – ${Math.max(...validAsks).toFixed(2)}`
        : "—",
    bidRange:
      validBids.length > 0
        ? `${Math.min(...validBids).toFixed(2)} – ${Math.max(...validBids).toFixed(2)}`
        : "—",
  };
}

/**
 * The chassis the simulator lobby's `SIM UNIT` takes (48e L8, clients:G8): unit
 * type ids under `crates/tacsim/content/units/`, the same list Unity offers
 * (`TacSimMissionOverlays.cs`). `scout`, `missile_carrier` and `heavy_mech`
 * name no unit and the lobby refused them.
 */
export const SIM_UNIT_CHASSIS: ReadonlyArray<{ id: string; label: string }> = [
  { id: "line_tank", label: "Line Tank" },
  { id: "line_tank_laser", label: "Laser Tank" },
  { id: "heavy_mk1", label: "Heavy Mk1" },
  { id: "biped_mk1", label: "Biped Mk1" },
  { id: "biped_mk2", label: "Biped Mk2" },
  { id: "scout_mk1", label: "Scout Mk1" },
];

/**
 * A job line on the id the player typed (48e L8, clients:G6): `verb id` or
 * `verb id go`. With no id there is no line: the old placeholder `1` named no
 * job, so the server only answered with its usage line.
 */
export function formatJobCommand(verb: string, jobId: string, go: boolean): string | null {
  const id = jobId.trim();
  if (!/^\S+$/.test(id)) return null;
  return go ? `${verb} ${id} go` : `${verb} ${id}`;
}

export function formatMarketHistoryCommand(goodCode: string): string {
  const clean = goodCode.trim().replace(/^tacsim:/i, "") || "volatiles";
  return `+market/history tacsim:${clean}`;
}

export function formatMarketDepthCommand(goodCode: string): string {
  const clean = goodCode.trim().replace(/^tacsim:/i, "") || "volatiles";
  return `+market/depth tacsim:${clean}`;
}

export function formatHaulPostCommand(
  qty: number,
  goodCode: string,
  destPort: string,
  feeCredits: number,
  commit = true,
): string {
  const q = Math.max(1, Math.trunc(qty));
  const g = goodCode.trim() || "volatiles";
  const p = destPort.trim() || "stickney";
  const f = Math.max(1, Math.trunc(feeCredits));
  return commit
    ? `+haul/post ${q} ${g} to=${p} fee=${f} go`
    : `+haul/post ${q} ${g} to=${p} fee=${f}`;
}

export const SURVEY_OFFERS: readonly { id: string; label: string }[] = [
  { id: "ahuna-a", label: "Ahuna Flank · Block A (ahuna-a)" },
  { id: "kerwan-f", label: "Kerwan Floor · Block F (kerwan-f)" },
  { id: "juling-k", label: "Juling Shadow · Block K (juling-k)" },
  { id: "occator-p", label: "Occator Rim · Block P (occator-p)" },
];

export const SIMULATOR_CHASSIS_IDS: readonly { id: string; label: string }[] = SIM_UNIT_CHASSIS;

export function isValidHullDbref(hullRef: string | null | undefined): boolean {
  return typeof hullRef === "string" && /^#\d+$/.test(hullRef.trim());
}

export function formatSurveyAcceptCommand(surveyId: string, commit = true): string {
  const clean = surveyId.trim() || SURVEY_OFFERS[0]!.id;
  return commit ? `+survey/accept ${clean} go` : `+survey/accept ${clean}`;
}

/**
 * `+escort/post` takes the hull as one word, its dbref (`#123`): never a name
 * or `ship` (48e L8, clients:G4; `tacsim::services::escort::hull_word`).
 * Returns `null` when `hullDbref` is not a dbref, so the caller disables the
 * button rather than send a line the server can only refuse.
 */
export function formatEscortPostCommand(
  hullDbref: string | null | undefined,
  destPort: string,
  feeCredits: number,
  commit = true,
): string | null {
  const h = (hullDbref ?? "").trim();
  if (!isValidHullDbref(h)) return null;
  const p = destPort.trim() || "stickney";
  const f = Math.max(1, Math.trunc(feeCredits));
  return commit
    ? `+escort/post ${h} to=${p} fee=${f} go`
    : `+escort/post ${h} to=${p} fee=${f}`;
}
