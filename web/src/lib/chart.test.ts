// The sky chart's pure half (doc 19 cut A.3b): the decoder over a grid frame
// with a `chart` block, the radial scale, the points and polylines the screen
// draws, and the table lines it prints. A hand-written frame in the server's
// shape; the committed fixture `fixtures/chart/sky_chart.json`, blessed by the
// Rust side (`TACSIM_BLESS=1 cargo test -p tacsim --test chart`), is decoded
// too when present, so a server that changes the shape fails here.

import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  AU_PER_LIGHT_MINUTE,
  LAG_RING_MINUTES,
  SKY_CHART_FEED,
  bodyLines,
  bodyPoints,
  chartHeading,
  lagRing,
  orbitRadius,
  parseChartFrame,
  radialFraction,
  routeLines,
  routePolyline,
  scaleCaption,
  windowLines,
} from "./chart";

const frame = {
  schema: { id: "sky.chart", shape: "grid", version: 1 },
  window: 0,
  revision: 0,
  tick: 0,
  title: "The sky on 2351-05-13 from Ceres Freeport on ceres (kepler; moving)",
  surface: "ecliptic",
  width: 6,
  height: 3,
  rows: [
    [[" ", 6]],
    [[" ", 2], ["*", 1], [" ", 1], ["C", 1], [" ", 1]],
    [["M", 1], [" ", 5]],
  ],
  legend: [
    ["*", "the Sun"],
    ["C", "ceres (here)"],
    ["M", "mars"],
  ],
  units: [{ id: "here", name: "Ceres Freeport", cell: [4, 1], glyph: "@", you: true }],
  you: [4, 1],
  cell_columns: 1,
  chart: {
    date: "2351-05-13",
    ic_now: 74152540800,
    model: "kepler",
    rate_permille: 1000,
    vantage: { port: "Ceres Freeport", body: "ceres", hull: "the Dray", payload_kg: 0, occupants: 1 },
    scale: { kind: "log", au_max: 9.6, r0_au: 0.3 },
    bodies: [
      {
        id: "ceres",
        label: "Ceres",
        glyph: "C",
        parent: null,
        r_au: 2.866,
        lon_deg: 0,
        lat_deg: -4.7,
        x_au: 2.866,
        y_au: 0,
        z_au: -0.23,
        period_days: 1681.9,
        here: true,
        from_here_au: null,
        lag_s: null,
        cell: [4, 1],
      },
      {
        id: "mars",
        label: "Mars",
        glyph: "M",
        parent: null,
        r_au: 1.445,
        lon_deg: 90,
        lat_deg: 1.2,
        x_au: 0,
        y_au: 1.445,
        z_au: 0.03,
        period_days: 687.0,
        here: false,
        from_here_au: 3.21,
        lag_s: 1602,
        cell: [0, 2],
      },
      {
        id: "phobos",
        label: "Phobos",
        glyph: "M",
        parent: "mars",
        r_au: 1.445,
        lon_deg: 90,
        lat_deg: 1.2,
        x_au: 0,
        y_au: 1.445,
        z_au: 0.03,
        period_days: 687.0,
        here: false,
        from_here_au: 3.21,
        lag_s: 1602,
        cell: [0, 2],
      },
    ],
    routes: [
      {
        to: "Stickney Yards",
        venue: "stickney",
        body: "phobos",
        legs: [
          { from: "ceres", to: "kirkwood" },
          { from: "kirkwood", to: "phobos" },
        ],
        loaded: { total_s: 1577842, burn_t: 51, crew_days: 19 },
        empty: { total_s: 1400000, burn_t: 40, crew_days: 17 },
        direct: { km: 480000000, total_s: 1500000, burn_t: 48 },
      },
    ],
    window: [
      {
        to: "Stickney Yards",
        now: { total_s: 1577842, burn_t: 51, crew_days: 19 },
        fastest: { offset_days: 30, total_s: 1500000, burn_t: 48 },
        cheapest: { offset_days: 30, total_s: 1500000, burn_t: 48 },
        worst: { offset_days: 200, total_s: 1900000, burn_t: 60 },
      },
    ],
  },
};

describe("parseChartFrame", () => {
  it("decodes the grid half and the chart half", () => {
    const f = parseChartFrame(frame);
    expect(f).not.toBeNull();
    expect(f!.grid.width).toBe(6);
    expect(f!.grid.legend).toEqual(frame.legend);
    expect(f!.grid.you).toEqual([4, 1]);
    expect(f!.chart.vantage.port).toBe("Ceres Freeport");
    expect(f!.chart.bodies.map((b) => b.id)).toEqual(["ceres", "mars", "phobos"]);
    expect(f!.chart.bodies[2]!.parent).toBe("mars");
    expect(f!.chart.routes[0]!.legs).toHaveLength(2);
    expect(f!.chart.window[0]!.worst?.offset_days).toBe(200);
  });

  it("refuses a frame that is not a grid or has no chart block", () => {
    expect(parseChartFrame({ schema: { shape: "list" } })).toBeNull();
    const { chart: _chart, ...bare } = frame;
    expect(parseChartFrame(bare)).toBeNull();
  });

  it("tolerates missing keys", () => {
    const f = parseChartFrame({ ...frame, chart: { bodies: [{ id: "x" }] } });
    expect(f).not.toBeNull();
    expect(f!.chart.bodies[0]).toMatchObject({ id: "x", label: "x", glyph: "?", parent: null, r_au: 0 });
    expect(f!.chart.scale.au_max).toBe(10);
    expect(f!.chart.routes).toEqual([]);
  });

  it("decodes the committed fixture when the Rust side has blessed one", () => {
    const path = resolve(process.cwd(), "src/lib/fixtures/chart/sky_chart.json");
    if (!existsSync(path)) {
      return;
    }
    const f = parseChartFrame(JSON.parse(readFileSync(path, "utf8")));
    expect(f).not.toBeNull();
    expect(f!.grid.width).toBeGreaterThan(0);
    expect(f!.chart.bodies.length).toBeGreaterThanOrEqual(16);
    expect(f!.chart.bodies.some((b) => b.here)).toBe(true);
    expect(f!.chart.routes.length).toBeGreaterThan(0);
  });
});

describe("the picture", () => {
  it("places a body on the logarithmic scale along its longitude", () => {
    expect(radialFraction(9.6, 9.6)).toBeCloseTo(1);
    // ln(1 + 2.4 / 0.3) / ln(1 + 9.6 / 0.3) = ln 9 / ln 33.
    expect(radialFraction(2.4, 9.6)).toBeCloseTo(Math.log(9) / Math.log(33));
    expect(radialFraction(1, 9.6)).toBeGreaterThan(radialFraction(0.39, 9.6));
    expect(radialFraction(0, 9.6)).toBe(0);
    expect(radialFraction(100, 9.6)).toBe(1);
    const f = parseChartFrame(frame)!;
    const pts = bodyPoints(f.chart, 640);
    const ceres = pts.find((p) => p.id === "ceres")!;
    const mars = pts.find((p) => p.id === "mars")!;
    // Ceres at longitude 0 lies to the right of the Sun on the centre line.
    expect(ceres.y).toBeCloseTo(320);
    expect(ceres.x).toBeGreaterThan(320);
    // Mars at longitude 90 lies straight up (screen y grows downward).
    expect(mars.x).toBeCloseTo(320);
    expect(mars.y).toBeLessThan(320);
    // Farther out, farther from the centre.
    expect(ceres.x - 320).toBeGreaterThan(320 - mars.y);
    expect(orbitRadius(f.chart.bodies[0]!, f.chart, 640)).toBeCloseTo(ceres.x - 320);
  });

  it("draws a route through its legs and skips a body not on the chart", () => {
    const f = parseChartFrame(frame)!;
    const pts = bodyPoints(f.chart, 640);
    const line = routePolyline(f.chart.routes[0]!, pts);
    // ceres -> kirkwood (absent) -> phobos: ceres, then phobos.
    expect(line.map((p) => p.id)).toEqual(["ceres", "phobos"]);
  });
});

describe("the tables", () => {
  it("prints the heading, the sky, the passages and the window", () => {
    const f = parseChartFrame(frame)!;
    expect(chartHeading(f.chart)).toBe(
      "The sky on 2351-05-13 (kepler; moving, rate 1000), from Ceres Freeport on ceres",
    );
    const sky = bodyLines(f.chart);
    expect(sky[0]).toMatch(/^mars {6}r {2}1\.445 AU {2}lon {3}90\.0 {2}lat \+1\.2 {2}3\.210 AU from here, 26m 42s light$/);
    expect(sky[1]).toContain("moon of mars");
    expect(sky[2]).toMatch(/^ceres {5}r {2}2\.866 AU {2}lon {4}0\.0 {2}lat -4\.7 {2}here$/);
    expect(routeLines(f.chart)).toEqual([
      "Stickney Yards: 438.3 h at this load (0 t aboard the Dray: burn 51 t, 19 crew-days); empty 388.9 h and 40 t; the direct chord 3.209 AU would be 416.7 h and 48 t.",
    ]);
    expect(windowLines(f.chart)).toEqual([
      "Stickney Yards: now 438.3 h, burn 51 t; fastest in 30 IC days: 416.7 h, burn 48 t (-21.6 h and -3 t); worst in 200 IC days: 527.8 h, burn 60 t (+89.5 h and +9 t).",
    ]);
  });

  it("says every day is today under a constant sky", () => {
    const f = parseChartFrame({
      ...frame,
      chart: { ...frame.chart, rate_permille: 0, window: [{ to: "Stickney Yards", now: frame.chart.window[0]!.now }] },
    })!;
    expect(chartHeading(f.chart)).toContain("constant");
    expect(windowLines(f.chart)[0]).toContain("every day is today");
  });

  it("names the feed the server registers", () => {
    expect(SKY_CHART_FEED).toBe("sky.chart");
  });
});

// Doc 21 cut G.5c: the light-lag rings about the viewer's body, as the
// terminal's picture draws them, and the caption that names the scale drawn.
describe("the light-lag rings", () => {
  const fixture = () => {
    const f = parseChartFrame(
      JSON.parse(readFileSync(resolve(process.cwd(), "src/lib/fixtures/chart/sky_chart.json"), "utf8")),
    );
    expect(f).not.toBeNull();
    return f!.chart;
  };

  it("are 3, 6, 12 and 24 light-minutes, a light-minute 0.1202 AU", () => {
    expect([...LAG_RING_MINUTES]).toEqual([3, 6, 12, 24]);
    expect(AU_PER_LIGHT_MINUTE).toBeCloseTo((299_792_458 * 60) / 149_597_870_700, 6);
  });

  it("are closed curves about the viewer's body, widening with the minutes", () => {
    const chart = fixture();
    const here = bodyPoints(chart, 640).find((p) => chart.bodies.find((b) => b.id === p.id)?.here)!;
    const spread = (minutes: number) => {
      const ring = lagRing(chart, minutes, 640);
      expect(ring).toHaveLength(181);
      const [first, last] = [ring[0]!, ring[ring.length - 1]!];
      expect(Math.hypot(first.x - last.x, first.y - last.y)).toBeLessThan(1e-6);
      return ring.reduce((sum, p) => sum + Math.hypot(p.x - here.x, p.y - here.y), 0) / ring.length;
    };
    const [a, b, c, d] = [spread(3), spread(6), spread(12), spread(24)];
    expect(a).toBeLessThan(b);
    expect(b).toBeLessThan(c);
    expect(c).toBeLessThan(d);
  });

  it("are none on a chart with nowhere to stand", () => {
    const chart = fixture();
    expect(lagRing({ ...chart, bodies: chart.bodies.map((b) => ({ ...b, here: false })) }, 3, 640)).toEqual([]);
  });

  it("are named in a caption that says the scale is logarithmic", () => {
    expect(scaleCaption(fixture())).toBe(
      "scale: logarithmic past 0.3 AU, the edge at 10.3 AU; rings: light-minutes from here",
    );
  });
});
