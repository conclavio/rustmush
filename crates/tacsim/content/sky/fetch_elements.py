#!/usr/bin/env python3
"""The astronomy engine's sky, fetched from JPL Horizons (doc 19 §3; cut A.1).

What this is for
----------------
`crates/tacsim/content/lane/ceres_phobos.json` carries, for every `body`, an
**element table**: osculating Keplerian elements (heliocentric ecliptic J2000,
from Horizons ELEMENTS) at semi-annual epochs over the campaign window plus the
lane's own epoch, which `tacsim::sky::Sky::Kepler` propagates from the two rows
that bracket an instant and blends (doc 19 D-19-2). One row is exact; between
rows the blend holds every body within 0.1 permille of Horizons over sixty
years (Earth within 1.1 permille: the Moon's wobble), measured by this script
against the golden table it also writes (`crates/tacsim/content/sky/golden.json`:
Horizons heliocentric positions every 91 days over the window).

It prints ONE JSON object to stdout — the pack `def` per body and the
measurement (the maximum position error of the table against the golden rows,
per body) — and writes the golden table when asked (`--write-golden`). The pack
is edited by hand from the output and re-signed by its test.

No test runs this script. Nothing in the build reads it. It needs the network
(https://ssd.jpl.nasa.gov/api/horizons.api); `--cache <dir>` keeps the raw rows.

The calendar rule (doc 18 lead ruling R3; doc 19 D-19-3)
---------------------------------------------------------
The game's IC clock is `mush_core::CivilDate`: 365-day years, the non-leap
month lengths, year 0 at IC second 0. A real date maps to the game calendar BY
ITS LABEL (year, month, day): the sky a player sees on the game date 2351-05-13
is the real sky of 2351-05-13. So an instant's position is propagated over
REAL days — the Julian Day of its label — and the Rust does the same
(`tacsim::sky::julian_day`). February 29 has no game label and is never a row.

Usage
-----
    python -I crates/tacsim/content/sky/fetch_elements.py \
        [--epoch 2351-05-13] [--from 2340] [--to 2400] [--cache <dir>] \
        [--write-golden crates/tacsim/content/sky/golden.json]
"""

import argparse
import decimal
import json
import math
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

API = "https://ssd.jpl.nasa.gov/api/horizons.api"

# Horizons COMMAND per body (planet centres, giant barycentres — Horizons'
# planet-centre records for Jupiter and Saturn (599, 699) end at 2200 and the
# barycentres run on; at the sky's scale the two are one point — and `DES=`
# selects the asteroid record).
BODIES = {
    "mercury": "'199'",
    "venus": "'299'",
    "earth": "'399'",
    "mars": "'499'",
    "jupiter": "'5'",
    "saturn": "'6'",
    "ceres": "'DES=2000001;'",
    "pallas": "'DES=2000002;'",
    "juno": "'DES=2000003;'",
    "vesta": "'DES=2000004;'",
    "hygiea": "'DES=2000010;'",
}

# The moons (doc 19 cut A.7; Q-19-9): their ELEMENTS are asked about the PARENT
# (Horizons CENTER = the planet's body centre), so the pack's rows are
# planetocentric and `tacsim::sky` adds the parent's position; their golden
# VECTORS stay heliocentric like every other body's, which is what the Rust is
# measured against. `parent` is the pack id of the primary.
MOONS = {
    "luna": dict(command="'301'", center="'500@399'", parent="earth"),
    "phobos": dict(command="'401'", center="'500@499'", parent="mars"),
    "deimos": dict(command="'402'", center="'500@499'", parent="mars"),
    # Horizons' Titan ephemeris ends at 2250-01-05, a century before the window:
    # its rows are the last decade Horizons knows (2240–2249), and the Rust
    # propagates the last row alone past the table (two-body about Saturn — a
    # near-circular 15.9-day orbit whose plane precesses in centuries, so the
    # extrapolated sky is self-consistent rather than Horizons-true). No golden
    # rows and no measure: there is nothing to measure against.
    "titan": dict(command="'606'", center="'500@699'", parent="saturn", until=(2250, 1, 1)),
}

# Doc 35 cut SF.1 (D-35-4, D-35-13): outer solar system bodies, Galilean &
# Neptunian moons, and the four orbital asteroid field clusters.
OUTER_BODIES = {
    "uranus": dict(command="'7'", radius_km=25_362, surface_mg=887),
    "neptune": dict(command="'8'", radius_km=24_622, surface_mg=1_115),
}

OUTER_MOONS = {
    "io": dict(command="'501'", center="'500@599'", parent="jupiter",
               radius_km=1_822, surface_mg=180),
    "europa": dict(command="'502'", center="'500@599'", parent="jupiter",
                   radius_km=1_561, surface_mg=131),
    "ganymede": dict(command="'503'", center="'500@599'", parent="jupiter",
                     radius_km=2_631, surface_mg=143),
    "callisto": dict(command="'504'", center="'500@599'", parent="jupiter",
                     radius_km=2_410, surface_mg=124),
    # Horizons' Triton ephemeris ends at 2199-12-30; like Titan, its table holds
    # the last decade Horizons knows (2189–2198) and the solver propagates the
    # last row past the cutoff.
    "triton": dict(command="'801'", center="'500@899'", parent="neptune",
                   until=(2199, 12, 1), radius_km=1_353, surface_mg=78),
}

ASTEROID_CLUSTERS = {
    "ceres_halo": dict(
        base="ceres",
        trail_deg=0.3,
        zone="core",
        radius_km=500,
        spectral_classes=["c_type", "s_type"],
        ore_families=["ice_ore", "carbon_ore", "metal_ore"],
    ),
    "kirkwood_gap_3_1": dict(
        base="ceres",
        trail_deg=2.4,
        zone="frontier",
        radius_km=800,
        spectral_classes=["s_type", "m_type"],
        ore_families=["metal_ore", "strategic_ore"],
    ),
    "pallas_inclined_belt": dict(
        base="pallas",
        trail_deg=0.6,
        zone="frontier",
        radius_km=600,
        spectral_classes=["m_type", "s_type"],
        ore_families=["strategic_ore", "metal_ore"],
    ),
    "saturn_b_ring_edge": dict(
        parent="saturn",
        a_au=0.000_786_000,
        e=0.000_300_000,
        i_deg=0.300_000,
        node_deg=169.530_000,
        peri_deg=42.000_000,
        mean_anomaly_deg=180.000_000,
        mean_motion_deg_day=742.268_041_237,
        period_days=0.485,
        zone="frontier",
        radius_km=400,
        spectral_classes=["c_type"],
        ore_families=["ice_ore", "carbon_ore"],
    ),
}

SUN_CENTER = "'500@10'"

# Kirkwood Depot (doc 18 Q-18-36; doc 19 D-19-6): Ceres's own rows, trailing by
# this many degrees of mean anomaly.
KIRKWOOD_TRAIL_DEG = 2.0

AU_KM = 149_597_871  # tacsim::ephemeris::AU_KM
GOLDEN_STEP = "91d"

MONTH_LENGTHS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
MONTHS = {name: i + 1 for i, name in enumerate(
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"])}

# The decimals the pack holds a row at (tacsim::pack::ElementRow's integer
# scales: nano-AU, nano for e, micro-degrees, nano-degrees per day).
DECIMALS = dict(a_au=9, e=9, i_deg=6, node_deg=6, peri_deg=6, mean_anomaly_deg=6,
                mean_motion_deg_day=9)


# ---------------------------------------------------------------------------
# Dates
# ---------------------------------------------------------------------------

def parse_date(text):
    y, m, d = (int(part) for part in text.split("-"))
    return (y, m, d)


def date_str(date):
    return "%04d-%02d-%02d" % date


def epoch_ic(date):
    """The game calendar's IC seconds at 00:00 (mush_core::CivilDate)."""
    y, m, d = date
    day = sum(MONTH_LENGTHS[: m - 1]) + (d - 1)
    return (y * 365 + day) * 86_400


def julian_day(date):
    """JD at 00:00 of a Gregorian date (the standard formula)."""
    y, m, d = date
    a = (14 - m) // 12
    yy = y + 4800 - a
    mm = m + 12 * a - 3
    jdn = d + (153 * mm + 2) // 5 + 365 * yy + yy // 4 - yy // 100 + yy // 400 - 32045
    return jdn - 0.5


def horizons_date(text):
    """'A.D. 2351-May-13 00:00:00.0000' -> (2351, 5, 13)."""
    text = text.strip()
    if text.startswith("A.D."):
        text = text[4:].strip()
    y, mon, d = text.split()[0].split("-")
    return (int(y), MONTHS[mon], int(d))


def row_dates(first_year, last_year, epoch):
    """January 1 and July 1 of every year in the window, plus the epoch, sorted."""
    dates = set()
    for year in range(first_year, last_year + 1):
        dates.add((year, 1, 1))
        if year < last_year:
            dates.add((year, 7, 1))
    dates.add(epoch)
    return sorted(dates)


# ---------------------------------------------------------------------------
# Horizons
# ---------------------------------------------------------------------------

CACHE = {"dir": None}


def horizons(params, center=SUN_CENTER):
    query = dict(format="json", OBJ_DATA="'NO'", MAKE_EPHEM="'YES'", CENTER=center,
                 REF_PLANE="'ECLIPTIC'", REF_SYSTEM="'J2000'", CSV_FORMAT="'YES'")
    query.update(params)
    url = API + "?" + urllib.parse.urlencode(query)
    with urllib.request.urlopen(url, timeout=120) as response:
        body = json.loads(response.read().decode("utf-8"))
    if "error" in body:
        raise urllib.error.URLError(body["error"])
    return body["result"]


def table(result):
    lines = result.splitlines()
    start = lines.index("$$SOE")
    end = lines.index("$$EOE")
    header = None
    for line in reversed(lines[:start]):
        if "JDTDB" in line:
            header = [name.strip() for name in line.split(",")]
            break
    rows = []
    for line in lines[start + 1 : end]:
        cells = [cell.strip() for cell in line.split(",")]
        rows.append(dict(zip(header, cells)))
    return rows


def cached(key, fetch):
    """The raw rows of one query, from `--cache <dir>` when present."""
    if not CACHE["dir"]:
        return fetch()
    path = os.path.join(CACHE["dir"], key + ".json")
    if os.path.exists(path):
        with open(path, encoding="utf-8") as handle:
            rows = json.load(handle)
        for row in rows:
            row["date"] = tuple(row["date"])
        return rows
    rows = fetch()
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(rows, handle)
    return rows


# Horizons answers at most this many TLIST epochs in one query (it answered 80 of
# 122 and said nothing), so the dates go in chunks.
TLIST_CHUNK = 40


def elements_at(command, dates, center=SUN_CENTER):
    """Osculating elements at each date (00:00 TDB) about `center`: date, JD, a, e,
    i, om, w, ma, n, pr."""
    out = []
    for start in range(0, len(dates), TLIST_CHUNK):
        chunk = dates[start : start + TLIST_CHUNK]
        tlist = " ".join("%.1f" % julian_day(date) for date in chunk)
        result = horizons(dict(COMMAND=command, EPHEM_TYPE="'ELEMENTS'", OUT_UNITS="'AU-D'",
                               TLIST="'%s'" % tlist), center)
        rows = table(result)
        if len(rows) != len(chunk):
            raise ValueError("Horizons answered %d of %d epochs" % (len(rows), len(chunk)))
        for row in rows:
            out.append(dict(date=horizons_date(row["Calendar Date (TDB)"]),
                            jd=float(row["JDTDB"]), a=float(row["A"]), e=float(row["EC"]),
                            i=float(row["IN"]), om=float(row["OM"]), w=float(row["W"]),
                            ma=float(row["MA"]), n=float(row["N"]), pr=float(row["PR"])))
    return out


def vectors(command, first, last, step):
    """Heliocentric ecliptic-J2000 positions per row, km: date, JD, x, y, z."""
    result = horizons(dict(COMMAND=command, EPHEM_TYPE="'VECTORS'", OUT_UNITS="'KM-S'",
                           VEC_TABLE="'1'", START_TIME="'%s'" % date_str(first),
                           STOP_TIME="'%s'" % date_str(last), STEP_SIZE="'%s'" % step))
    out = []
    for row in table(result):
        out.append(dict(date=horizons_date(row["Calendar Date (TDB)"]), jd=float(row["JDTDB"]),
                        x=float(row["X"]), y=float(row["Y"]), z=float(row["Z"])))
    return out


# ---------------------------------------------------------------------------
# Propagation, exactly as the Rust does it (tacsim::sky, Kepler)
# ---------------------------------------------------------------------------

def quantize(row):
    """A Horizons row rounded to the pack's decimals: the numbers the Rust holds."""
    return dict(epoch_ic=epoch_ic(row["date"]), jd=julian_day(row["date"]),
                a_au=round(row["a"], DECIMALS["a_au"]), e=round(row["e"], DECIMALS["e"]),
                i_deg=round(row["i"], DECIMALS["i_deg"]),
                node_deg=round(row["om"] % 360.0, DECIMALS["node_deg"]),
                peri_deg=round(row["w"] % 360.0, DECIMALS["peri_deg"]),
                mean_anomaly_deg=round(row["ma"] % 360.0, DECIMALS["mean_anomaly_deg"]),
                mean_motion_deg_day=round(row["n"], DECIMALS["mean_motion_deg_day"]))


def kepler_xyz(row, dt_days):
    """Heliocentric ecliptic-J2000 position, km, `dt_days` after the row's epoch,
    by two-body propagation of the row's elements."""
    a = row["a_au"]
    e = min(max(row["e"], 0.0), 0.99)
    inc = math.radians(row["i_deg"])
    om = math.radians(row["node_deg"])
    w = math.radians(row["peri_deg"])
    m = math.radians((row["mean_anomaly_deg"] + row["mean_motion_deg_day"] * dt_days) % 360.0)
    big_e = m if e < 0.8 else math.pi
    for _ in range(16):
        big_e -= (big_e - e * math.sin(big_e) - m) / (1.0 - e * math.cos(big_e))
    xp = a * (math.cos(big_e) - e)
    yp = a * math.sqrt(1.0 - e * e) * math.sin(big_e)
    cw, sw, co, so, ci, si = (math.cos(w), math.sin(w), math.cos(om), math.sin(om),
                              math.cos(inc), math.sin(inc))
    x = (co * cw - so * sw * ci) * xp + (-co * sw - so * cw * ci) * yp
    y = (so * cw + co * sw * ci) * xp + (-so * sw + co * cw * ci) * yp
    z = (sw * si) * xp + (cw * si) * yp
    return (x * AU_KM, y * AU_KM, z * AU_KM)


def position(rows, jd):
    """The table's position at `jd`: the bracketing rows' propagations blended
    linearly in time; outside the table, the nearest end row's propagation."""
    if jd <= rows[0]["jd"]:
        return kepler_xyz(rows[0], jd - rows[0]["jd"])
    if jd >= rows[-1]["jd"]:
        return kepler_xyz(rows[-1], jd - rows[-1]["jd"])
    lo = max(k for k in range(len(rows) - 1) if rows[k]["jd"] <= jd)
    r0, r1 = rows[lo], rows[lo + 1]
    u = (jd - r0["jd"]) / (r1["jd"] - r0["jd"])
    p0 = kepler_xyz(r0, jd - r0["jd"])
    p1 = kepler_xyz(r1, jd - r1["jd"])
    return tuple((1.0 - u) * a + u * b for a, b in zip(p0, p1))


def blended_elements_position(rows, jd):
    """A moon table's position at `jd` with elements blended in element space,
    matching `tacsim::kepler::blended_elements_position`."""
    if jd <= rows[0]["jd"]:
        return kepler_xyz(rows[0], jd - rows[0]["jd"])
    if jd >= rows[-1]["jd"]:
        return kepler_xyz(rows[-1], jd - rows[-1]["jd"])
    lo = max(k for k in range(len(rows) - 1) if rows[k]["jd"] <= jd)
    r0, r1 = rows[lo], rows[lo + 1]
    u = (jd - r0["jd"]) / (r1["jd"] - r0["jd"])
    def lerp(a, b):
        return a + (b - a) * u
    def lerp_angle(a, b):
        diff = (b - a) % 360.0
        if diff > 180.0:
            diff -= 360.0
        return (a + diff * u) % 360.0
    def adv(r):
        return (r["mean_anomaly_deg"] + r["mean_motion_deg_day"] * (jd - r["jd"])) % 360.0
    peri = lerp_angle(r0["peri_deg"], r1["peri_deg"])
    lon = lerp_angle((r0["peri_deg"] + adv(r0)) % 360.0, (r1["peri_deg"] + adv(r1)) % 360.0)
    blended = dict(
        a_au=lerp(r0["a_au"], r1["a_au"]),
        e=lerp(r0["e"], r1["e"]),
        i_deg=lerp(r0["i_deg"], r1["i_deg"]),
        node_deg=lerp_angle(r0["node_deg"], r1["node_deg"]),
        peri_deg=peri,
        mean_anomaly_deg=(lon - peri) % 360.0,
        mean_motion_deg_day=lerp(r0["mean_motion_deg_day"], r1["mean_motion_deg_day"]),
    )
    return kepler_xyz(blended, 0.0)


def polar(xyz):
    x, y, z = xyz
    r = math.sqrt(x * x + y * y + z * z)
    lam = math.degrees(math.atan2(y, x)) % 360.0
    beta = math.degrees(math.asin(z / r)) if r else 0.0
    return r, lam, beta


def measure(rows, golden):
    """Max and RMS relative position error of the table against the golden rows."""
    worst = (0.0, None)
    sq = 0.0
    for g in golden:
        pred = position(rows, g["jd"])
        err = math.dist(pred, (g["x"], g["y"], g["z"]))
        rel = err / math.hypot(g["x"], g["y"], g["z"])
        sq += rel * rel
        if rel > worst[0]:
            worst = (rel, date_str(g["date"]))
    return dict(max_permille=round(worst[0] * 1000, 4), worst_date=worst[1],
                rms_permille=round(math.sqrt(sq / len(golden)) * 1000, 4))


def pack_rows(rows):
    """The `elements` array of a body's `def`: one compact row per epoch."""
    return [[r["epoch_ic"], r["a_au"], r["e"], r["i_deg"], r["node_deg"], r["peri_deg"],
             r["mean_anomaly_deg"], r["mean_motion_deg_day"]] for r in rows]


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def write_outer_pack(args, epoch, first, last, dates, ic):
    """Fetch outer solar system bodies, Galilean/Triton moons, and four orbital
    asteroid clusters (doc 35 cut SF.1) and write `args.outer_path`."""
    outer = dict(
        note="Doc 35 cut SF.1 outer solar system bodies, Galilean & Neptunian moons, and "
             "orbital asteroid clusters (JPL Horizons ecliptic J2000 osculating elements "
             "over %s..%s, epoch %s)" % (date_str(first), date_str(last), date_str(epoch)),
        epoch=date_str(epoch),
        epoch_ic=ic,
        bodies={},
        golden={},
        measure={},
    )
    parent_rows = {}
    for parent_name in ("jupiter", "saturn", "ceres", "pallas"):
        cmd = BODIES[parent_name]
        print("fetching base %s ..." % parent_name, file=sys.stderr)
        praw = cached("elements-%s-%d-%d-%s" % (parent_name, args.first, args.last, date_str(epoch)),
                      lambda cmd=cmd: elements_at(cmd, dates))
        parent_rows[parent_name] = (praw, [quantize(r) for r in praw])

    for name, spec in OUTER_BODIES.items():
        command = spec["command"]
        print("fetching outer planet %s ..." % name, file=sys.stderr)
        raw = cached("elements-%s-%d-%d-%s" % (name, args.first, args.last, date_str(epoch)),
                     lambda command=command: elements_at(command, dates))
        vecs = cached("vectors-%s-%d-%d-%s" % (name, args.first, args.last, GOLDEN_STEP),
                      lambda command=command: vectors(command, first, last, GOLDEN_STEP))
        rows = [quantize(r) for r in raw]
        parent_rows[name] = (raw, rows)
        at_epoch = position(rows, julian_day(epoch))
        r, lam, beta = polar(at_epoch)
        at_row = [row for row in raw if row["date"] == epoch][0]
        outer["bodies"][name] = dict(
            kind="planet",
            orbit_uau=int(round(r / AU_KM * 1_000_000)),
            period_mdays=int(round(at_row["pr"] * 1_000)),
            phase_mdeg=int(round(lam * 1_000)) % 360_000,
            epoch_ic=ic,
            radius_km=spec["radius_km"],
            surface_mg=spec["surface_mg"],
            elements=pack_rows(rows),
        )
        outer["measure"][name] = measure(rows, vecs)
        outer["golden"][name] = [
            [epoch_ic(g["date"]), round(g["x"], 1), round(g["y"], 1), round(g["z"], 1)]
            for g in vecs if not (g["date"][1] == 2 and g["date"][2] == 29)
        ]

    for name, spec in OUTER_MOONS.items():
        parent = spec["parent"]
        print("fetching outer moon %s about %s ..." % (name, parent), file=sys.stderr)
        until = spec.get("until")
        if until:
            mdates = [d for d in row_dates(until[0] - 10, until[0] - 1, epoch) if d < until]
            raw = cached("elements-%s-%d-%d-%s" % (name, until[0] - 10, until[0] - 1, date_str(epoch)),
                         lambda spec=spec, mdates=mdates: elements_at(spec["command"], mdates, spec["center"]))
            vecs = []
        else:
            raw = cached("elements-%s-%d-%d-%s" % (name, args.first, args.last, date_str(epoch)),
                         lambda spec=spec: elements_at(spec["command"], dates, spec["center"]))
            vecs = cached("vectors-%s-%d-%d-%s" % (name, args.first, args.last, GOLDEN_STEP),
                          lambda spec=spec: vectors(spec["command"], first, last, GOLDEN_STEP))
        rows = [quantize(r) for r in raw]
        at_epoch = blended_elements_position(rows, julian_day(epoch))
        r, lam, beta = polar(at_epoch)
        at_row = ([row for row in raw if row["date"] == epoch] or [raw[-1]])[0]
        outer["bodies"][name] = dict(
            kind="moon",
            parent=parent,
            orbit_uau=max(1, int(round(r / AU_KM * 1_000_000))),
            period_mdays=max(1, int(round(at_row["pr"] * 1_000))),
            phase_mdeg=int(round(lam * 1_000)) % 360_000,
            epoch_ic=ic,
            radius_km=spec["radius_km"],
            surface_mg=spec["surface_mg"],
            elements=pack_rows(rows),
        )
        if not until:
            prow = parent_rows[parent][1]
            worst = (0.0, None)
            sq = 0.0
            for g in vecs:
                p = position(prow, g["jd"])
                m = blended_elements_position(rows, g["jd"])
                pred = (p[0] + m[0], p[1] + m[1], p[2] + m[2])
                err = math.dist(pred, (g["x"], g["y"], g["z"]))
                rel = err / math.hypot(g["x"], g["y"], g["z"])
                sq += rel * rel
                if rel > worst[0]:
                    worst = (rel, date_str(g["date"]))
            outer["measure"][name] = dict(
                max_permille=round(worst[0] * 1000, 4),
                worst_date=worst[1],
                rms_permille=round(math.sqrt(sq / len(vecs)) * 1000, 4),
                about=parent,
            )
            outer["golden"][name] = [
                [epoch_ic(g["date"]), round(g["x"], 1), round(g["y"], 1), round(g["z"], 1)]
                for g in vecs if not (g["date"][1] == 2 and g["date"][2] == 29)
            ]
        else:
            outer["measure"][name] = dict(about=parent, note="extrapolated past %04d-%02d-%02d" % until)

    for cname, cspec in ASTEROID_CLUSTERS.items():
        if "base" in cspec:
            base_raw, base_rows = parent_rows[cspec["base"]]
            trail = cspec["trail_deg"]
            rows = [
                dict(r, mean_anomaly_deg=round((r["mean_anomaly_deg"] - trail) % 360.0,
                                               DECIMALS["mean_anomaly_deg"]))
                for r in base_rows
            ]
            at_row = [row for row in base_raw if row["date"] == epoch][0]
            r, lam, _ = polar(position(rows, julian_day(epoch)))
            outer["bodies"][cname] = dict(
                kind="asteroid_cluster",
                orbit_uau=int(round(r / AU_KM * 1_000_000)),
                period_mdays=int(round(at_row["pr"] * 1_000)),
                phase_mdeg=int(round(lam * 1_000)) % 360_000,
                epoch_ic=ic,
                radius_km=cspec["radius_km"],
                surface_mg=1,
                zone=cspec["zone"],
                spectral_classes=cspec["spectral_classes"],
                ore_families=cspec["ore_families"],
                elements=pack_rows(rows),
            )
        else:
            row = dict(
                epoch_ic=ic,
                jd=julian_day(epoch),
                a_au=cspec["a_au"],
                e=cspec["e"],
                i_deg=cspec["i_deg"],
                node_deg=cspec["node_deg"],
                peri_deg=cspec["peri_deg"],
                mean_anomaly_deg=cspec["mean_anomaly_deg"],
                mean_motion_deg_day=cspec["mean_motion_deg_day"],
            )
            r, lam, _ = polar(kepler_xyz(row, 0.0))
            outer["bodies"][cname] = dict(
                kind="asteroid_cluster",
                parent=cspec["parent"],
                orbit_uau=max(1, int(round(r / AU_KM * 1_000_000))),
                period_mdays=max(1, int(round(cspec["period_days"] * 1_000))),
                phase_mdeg=int(round(lam * 1_000)) % 360_000,
                epoch_ic=ic,
                radius_km=cspec["radius_km"],
                surface_mg=1,
                zone=cspec["zone"],
                spectral_classes=cspec["spectral_classes"],
                ore_families=cspec["ore_families"],
                elements=pack_rows([row]),
            )

    text = json.dumps(outer, separators=(",", ":"))
    text = re.sub(
        r"-?\d+(?:\.\d+)?[eE][+-]?\d+",
        lambda m: format(decimal.Decimal(repr(float(m.group(0)))), "f"),
        text,
    )
    with open(args.outer_path, "w", encoding="utf-8") as handle:
        handle.write(text + "\n")
    print(json.dumps(dict(outer_written=args.outer_path, measure=outer["measure"]), indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--epoch", default="2351-05-13")
    parser.add_argument("--from", dest="first", type=int, default=2340)
    parser.add_argument("--to", dest="last", type=int, default=2400)
    parser.add_argument("--write-golden", dest="golden_path", default=None)
    parser.add_argument("--write-outer", dest="outer_path", default=None,
                        help="write the outer solar system & asteroid cluster table (doc 35 SF.1)")
    parser.add_argument("--bodies", default=",".join(BODIES))
    parser.add_argument("--moons", default=",".join(MOONS),
                        help="the moons to fetch about their parents (doc 19 cut A.7)")
    parser.add_argument("--cache", default=None, help="a directory for the raw Horizons rows")
    args = parser.parse_args()
    CACHE["dir"] = args.cache
    epoch = parse_date(args.epoch)
    first, last = (args.first, 1, 1), (args.last, 1, 1)
    dates = row_dates(args.first, args.last, epoch)
    ic = epoch_ic(epoch)
    if args.outer_path:
        write_outer_pack(args, epoch, first, last, dates, ic)
        return
    names = [n for n in args.bodies.split(",") if n]
    moons = [n for n in args.moons.split(",") if n]

    out = dict(epoch=date_str(epoch), epoch_ic=ic, window=[date_str(first), date_str(last)],
               rows=len(dates), golden_step=GOLDEN_STEP, bodies={}, measure={})
    golden = dict(note="JPL Horizons heliocentric ecliptic J2000 positions, km, every %s over "
                       "%s..%s; ic is the game calendar's IC second of the row's date label "
                       "(mush_core::CivilDate)" % (GOLDEN_STEP, date_str(first), date_str(last)),
                  epoch=date_str(epoch), epoch_ic=ic, bodies={})
    ceres_rows = None
    for name in names:
        command = BODIES[name]
        print("fetching %s ..." % name, file=sys.stderr)
        raw = cached("elements-%s-%d-%d-%s" % (name, args.first, args.last, date_str(epoch)),
                     lambda: elements_at(command, dates))
        vecs = cached("vectors-%s-%d-%d-%s" % (name, args.first, args.last, GOLDEN_STEP),
                      lambda: vectors(command, first, last, GOLDEN_STEP))
        rows = [quantize(r) for r in raw]
        if name == "ceres":
            ceres_rows = rows
        at_epoch = position(rows, julian_day(epoch))
        r, lam, beta = polar(at_epoch)
        at_row = [row for row in raw if row["date"] == epoch][0]
        out["bodies"][name] = dict(
            orbit_au=round(r / AU_KM, 6), period_days=round(at_row["pr"], 3),
            phase_deg_at_epoch=round(lam, 3), epoch_ic=ic, elements=pack_rows(rows),
            latitude_deg_at_epoch=round(beta, 3))
        out["measure"][name] = measure(rows, vecs)
        golden["bodies"][name] = [
            [epoch_ic(g["date"]), round(g["x"], 1), round(g["y"], 1), round(g["z"], 1)]
            for g in vecs if not (g["date"][1] == 2 and g["date"][2] == 29)]

    # The moons (doc 19 cut A.7): elements about the parent, golden vectors about
    # the Sun. The pack's four numbers are the planetocentric distance, period and
    # longitude at the epoch; the measure composes the parent's table (fetched
    # above, or now) with the moon's and compares the sum with Horizons.
    parent_rows = {}
    for name in moons:
        spec = MOONS[name]
        print("fetching %s about %s ..." % (name, spec["parent"]), file=sys.stderr)
        until = spec.get("until")
        if until:
            # The last decade Horizons knows, Jan 1 and Jul 1 rows, before the cutoff.
            mdates = [d for d in row_dates(until[0] - 10, until[0] - 1, epoch) if d < until]
            raw = cached("elements-%s-%d-%d-%s" % (name, until[0] - 10, until[0] - 1,
                                                    date_str(epoch)),
                         lambda: elements_at(spec["command"], mdates, spec["center"]))
            vecs = []
        else:
            raw = cached("elements-%s-%d-%d-%s" % (name, args.first, args.last, date_str(epoch)),
                         lambda: elements_at(spec["command"], dates, spec["center"]))
            vecs = cached("vectors-%s-%d-%d-%s" % (name, args.first, args.last, GOLDEN_STEP),
                          lambda: vectors(spec["command"], first, last, GOLDEN_STEP))
        parent = spec["parent"]
        if parent not in parent_rows:
            praw = cached("elements-%s-%d-%d-%s" % (parent, args.first, args.last, date_str(epoch)),
                          lambda: elements_at(BODIES[parent], dates))
            parent_rows[parent] = [quantize(r) for r in praw]
        rows = [quantize(r) for r in raw]
        at_epoch = position(rows, julian_day(epoch))
        r, lam, beta = polar(at_epoch)
        at_row = ([row for row in raw if row["date"] == epoch] or [raw[-1]])[0]
        out["bodies"][name] = dict(
            orbit_au=round(r / AU_KM, 9), period_days=round(at_row["pr"], 3),
            phase_deg_at_epoch=round(lam, 3), epoch_ic=ic, elements=pack_rows(rows),
            parent=parent, latitude_deg_at_epoch=round(beta, 3))
        if until:
            out["bodies"][name]["extrapolated_from"] = date_str(raw[-1]["date"])
            out["measure"][name] = dict(about=parent, note="no Horizons ephemeris in the window")
            continue
        composed = [dict(g, **{"x": g["x"], "y": g["y"], "z": g["z"]}) for g in vecs]
        prow = parent_rows[parent]
        def composed_position(jd, rows=rows, prow=prow):
            p = position(prow, jd)
            m = position(rows, jd)
            return (p[0] + m[0], p[1] + m[1], p[2] + m[2])
        worst = (0.0, None)
        sq = 0.0
        for g in composed:
            pred = composed_position(g["jd"])
            err = math.dist(pred, (g["x"], g["y"], g["z"]))
            rel = err / math.hypot(g["x"], g["y"], g["z"])
            sq += rel * rel
            if rel > worst[0]:
                worst = (rel, date_str(g["date"]))
        out["measure"][name] = dict(max_permille=round(worst[0] * 1000, 4), worst_date=worst[1],
                                    rms_permille=round(math.sqrt(sq / len(composed)) * 1000, 4),
                                    about=parent)
        golden["bodies"][name] = [
            [epoch_ic(g["date"]), round(g["x"], 1), round(g["y"], 1), round(g["z"], 1)]
            for g in vecs if not (g["date"][1] == 2 and g["date"][2] == 29)]

    # Kirkwood: Ceres's rows, trailing by KIRKWOOD_TRAIL_DEG of mean anomaly.
    if ceres_rows:
        rows = [dict(r, mean_anomaly_deg=round((r["mean_anomaly_deg"] - KIRKWOOD_TRAIL_DEG)
                                                 % 360.0, DECIMALS["mean_anomaly_deg"]))
                for r in ceres_rows]
        r, lam, beta = polar(position(rows, julian_day(epoch)))
        out["bodies"]["kirkwood"] = dict(
            orbit_au=round(r / AU_KM, 6), period_days=out["bodies"]["ceres"]["period_days"],
            phase_deg_at_epoch=round(lam, 3), epoch_ic=ic, elements=pack_rows(rows),
            latitude_deg_at_epoch=round(beta, 3),
            trails="ceres by %.1f deg of mean anomaly" % KIRKWOOD_TRAIL_DEG)

    if args.golden_path:
        with open(args.golden_path, "w", encoding="utf-8") as handle:
            json.dump(golden, handle, separators=(",", ":"))
            handle.write("\n")
        out["golden_written"] = args.golden_path
    # serde_json prints a small float as a plain decimal (`0.000062686`), never
    # Python's `6.2686e-05`, and a build with its `arbitrary_precision` feature
    # (mush-bot's) digests the authored spelling, so the pack's canonical digest only
    # agrees when the spellings do (tests/lane.rs checks every number of the lane).
    text = json.dumps(out, indent=1)
    text = re.sub(
        r"-?\d+(?:\.\d+)?[eE][+-]?\d+",
        lambda m: format(decimal.Decimal(repr(float(m.group(0)))), "f"),
        text,
    )
    print(text)


if __name__ == "__main__":
    main()
