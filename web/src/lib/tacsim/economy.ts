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

export const WORKS_RECIPES: readonly IndustryRecipeOption[] = [
  {
    id: "metals",
    label: "Metal Ore Milling",
    inputsSummary: "10 metal_ore",
    outputSummary: "4 metals",
  },
  {
    id: "carbon_feedstock",
    label: "Carbon Feedstock Pressing",
    inputsSummary: "5 carbon_ore",
    outputSummary: "4 carbon_feedstock",
  },
  {
    id: "fittings",
    label: "Fittings Batch",
    inputsSummary: "5 structural_alloys + 1 carbon_ore",
    outputSummary: "5 industrial_parts",
  },
  {
    id: "ap_30",
    label: "30 mm Armor-Piercing Rounds",
    inputsSummary: "1 alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "500 ammo_ap_30",
  },
  {
    id: "he_105",
    label: "105 mm High-Explosive Rounds",
    inputsSummary: "1 alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "40 ammo_he_105",
  },
  {
    id: "missile",
    label: "Anti-Armor Missiles",
    inputsSummary: "1 alloys + 1 carbon_ore + 1 volatiles",
    outputSummary: "4 ammo_missile",
  },
  {
    id: "module_doctrine",
    label: "Doctrine Core Module",
    inputsSummary: "1 instrument_boards + 1 strategic_ore",
    outputSummary: "1 tacsim.part.core.doctrine",
  },
  {
    id: "module_command",
    label: "Command Core Module",
    inputsSummary: "4 instrument_boards + 2 strategic_ore",
    outputSummary: "1 tacsim.part.core.command",
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
export function parseMarketHistoryLines(lines: readonly string[]): ParsedPriceHistory {
  let goodLabel: string | null = null;
  const rows: PriceHistoryRow[] = [];

  for (const raw of lines) {
    const headMatch = raw.match(
      /House prices of (.+?) here, by IC week, newest first \(\d+\):/i,
    );
    if (headMatch) {
      goodLabel = headMatch[1]!.trim();
      rows.length = 0;
      continue;
    }

    const rowMatch = raw.match(
      /^\s*(this IC week|last IC week|\d+\s+IC weeks ago):\s*ask\s+([^,]+),\s*bid\s+(.+)$/i,
    );
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

export function formatEscortPostCommand(
  hullRef: string,
  destPort: string,
  feeCredits: number,
  commit = true,
): string {
  const h = hullRef.trim() || "ship";
  const p = destPort.trim() || "stickney";
  const f = Math.max(1, Math.trunc(feeCredits));
  return commit
    ? `+escort/post ${h} to=${p} fee=${f} go`
    : `+escort/post ${h} to=${p} fee=${f}`;
}
