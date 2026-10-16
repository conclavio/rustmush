#!/usr/bin/env python3
"""The trade run's lane sky, fetched from JPL Horizons (doc 18 §4.8, D-18-48).

What this is for
----------------
`crates/tacsim/content/lane/ceres_phobos.json` (pack `tacsim.lane.ceres_phobos`)
places Ceres, Mars and Pallas at their TRUE heliocentric ecliptic longitudes and
distances on one real calendar date, the date the circular sky (`tacsim::sky::Sky::
Circular`) holds them at (D-18-48), plus Kirkwood on the Ceres-Mars chord at
2.06 AU. This script is the committed source of those numbers: it picks the
date (lead ruling R4), reads the positions, and prints the pack fields and the
legs the Rust derives from them. It prints ONE JSON object to stdout and writes
nothing; the pack is edited by hand from its output and re-signed by the test.

No test runs this script. Nothing in the build reads it. It needs the network
(https://ssd.jpl.nasa.gov/api/horizons.api); without it, or with --offline, it
falls back to Kepler propagation from the osculating element sets below.

The calendar rule (lead ruling R3)
----------------------------------
The game's IC clock is `mush_core::CivilDate`: 365-day years, the non-leap
month lengths, year 0 at IC second 0. So a date's `epoch_ic` is
`(year * 365 + day_of_year_0_based) * 86_400` -- NOT a Unix/Gregorian count --
and Horizons is queried for the same Y-M-D label (Gregorian, 00:00 TDB).
February 29 has no game-calendar label, so it is never a candidate.

The date (lead ruling R4)
-------------------------
Within the window (default 2350-01-01 .. 2352-12-31), among the dates whose
Ceres and Mars true longitudes are within --max-dl degrees, take the one that
minimizes the Ceres-Mars chord under the circular model's own formula
sqrt(r1^2 + r2^2 - 2 r1 r2 cos dl), ties to the earlier date. No candidate:
widen the window by one year each side ONCE and say so in the output.

Fallback
--------
--offline, or any network error: solve Kepler's equation (12 Newton steps) from
the heliocentric ecliptic-J2000 osculating elements at JD 2579876.5 TDB
(2351-05-13) fetched from Horizons ELEMENTS. The output's "source" says which.

Usage
-----
    python -I crates/tacsim/content/sky/fetch_horizons.py \
        [--from 2350-01-01] [--to 2352-12-31] [--max-dl 11.0] [--offline]
"""

import argparse
import json
import math
import urllib.error
import urllib.parse
import urllib.request

API = "https://ssd.jpl.nasa.gov/api/horizons.api"

# Horizons COMMAND for each body. Stickney is on Phobos, which the lane places
# at Mars (a moon's offset is nothing a corridor distance reads).
BODIES = {
    "ceres": "'DES=2000001;'",
    "mars": "'499'",
    "pallas": "'DES=2000002;'",
}

AU_KM = 149_597_871  # tacsim::ephemeris::AU_KM
KIRKWOOD_AU = 2.06  # lead note 4: on the chord at 2.06 AU
KIRKWOOD_PERIOD_DAYS = 1080.0  # placeholder (Q-18-36)
COAST_KMS = 226  # tacsim.unit.dray's drive.law.coast.kms (D-18-29, re-fitted to the real-date chord by the IC3 lead ruling)
PAYLOAD_CEILING_KG = 181_000  # tacsim.unit.dray's payload_ceiling_kg
REPORT_PAYLOADS_KG = [142_184, 180_680]  # full outbound; brimmed hull
WORLD_RATE = 24  # the report's real-time rate
D18_30_SECONDS = 950_000  # D-18-30's leg sum, for the alternative coast rate

MONTH_LENGTHS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]

# Heliocentric ecliptic J2000 osculating elements at JD 2579876.5 TDB
# (2351-05-13), from Horizons ELEMENTS (lead ruling R4; these supersede the
# lead note's a/e/i-only mean elements, which cannot place a body).
ELEMENTS_EPOCH_JD = 2579876.5
ELEMENTS = {
    "ceres": dict(a=2.767755414, e=0.075862114, i=10.550296325, om=75.280015105,
                  w=84.918513982, ma=114.238750294, n=0.214048659, pr=1681.860576),
    "pallas": dict(a=2.772452768, e=0.212782605, i=35.260534190, om=169.753529154,
                   w=313.057255126, ma=49.925994268, n=0.213504897, pr=1686.143998),
    "mars": dict(a=1.523771877, e=0.093674616, i=1.820714486, om=48.516913391,
                 w=289.069252598, ma=307.768495125, n=0.523991465, pr=687.034091),
}


# ---------------------------------------------------------------------------
# Dates
# ---------------------------------------------------------------------------

def parse_date(text):
    y, m, d = (int(part) for part in text.split("-"))
    return (y, m, d)


def date_str(date):
    return "%04d-%02d-%02d" % date


def is_leap(y):
    return y % 4 == 0 and (y % 100 != 0 or y % 400 == 0)


def next_day(date):
    y, m, d = date
    lengths = list(MONTH_LENGTHS)
    if is_leap(y):
        lengths[1] = 29
    d += 1
    if d > lengths[m - 1]:
        d = 1
        m += 1
        if m > 12:
            m = 1
            y += 1
    return (y, m, d)


def add_years(date, years):
    y, m, d = date
    if m == 2 and d == 29:
        d = 28
    return (y + years, m, d)


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


MONTHS = {name: i + 1 for i, name in enumerate(
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"])}


def horizons_date(text):
    """'A.D. 2351-May-13 00:00:00.0000' -> (2351, 5, 13)."""
    text = text.strip()
    if text.startswith("A.D."):
        text = text[4:].strip()
    day_part = text.split()[0]
    y, mon, d = day_part.split("-")
    return (int(y), MONTHS[mon], int(d))


# ---------------------------------------------------------------------------
# Arithmetic exactly as the Rust does it
# ---------------------------------------------------------------------------

def rround(x):
    """f64::round: half away from zero."""
    return math.floor(x + 0.5) if x >= 0 else -math.floor(-x + 0.5)


def wrap180(deg):
    return (deg + 180.0) % 360.0 - 180.0


def chord_au(r1, r2, dl_deg):
    return math.sqrt(max(0.0, r1 * r1 + r2 * r2 - 2.0 * r1 * r2 * math.cos(math.radians(dl_deg))))


def uau(au):
    return rround(au * 1e6)


def mdeg(deg):
    return rround(deg * 1000.0) % 360_000


def separation_km(uau_a, uau_b, delta_mdeg):
    """tacsim::ephemeris::separation_km, step for step."""
    r1 = uau_a * AU_KM / 1e6
    r2 = uau_b * AU_KM / 1e6
    theta = math.radians(delta_mdeg / 1000.0)
    squared = r1 * r1 + r2 * r2 - 2.0 * r1 * r2 * math.cos(theta)
    return rround(math.sqrt(max(0.0, squared)))


def ceil_div(a, b):
    return (a + b - 1) // b


# ---------------------------------------------------------------------------
# Horizons
# ---------------------------------------------------------------------------

def horizons(params):
    query = dict(format="json", OBJ_DATA="'NO'", MAKE_EPHEM="'YES'", CENTER="'500@10'",
                 REF_PLANE="'ECLIPTIC'", OUT_UNITS="'AU-D'", CSV_FORMAT="'YES'")
    query.update(params)
    url = API + "?" + urllib.parse.urlencode(query)
    with urllib.request.urlopen(url, timeout=60) as response:
        body = json.loads(response.read().decode("utf-8"))
    if "error" in body:
        raise urllib.error.URLError(body["error"])
    return body["result"]


def table(result):
    """The CSV rows between $$SOE and $$EOE, keyed by the header line's names."""
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


def vectors(command, first, last):
    """Daily heliocentric ecliptic-J2000 positions: {date: (x, y, z)}."""
    result = horizons(dict(COMMAND=command, EPHEM_TYPE="'VECTORS'",
                           START_TIME="'%s'" % date_str(first), STOP_TIME="'%s'" % date_str(last),
                           STEP_SIZE="'1d'", VEC_TABLE="'1'"))
    out = {}
    for row in table(result):
        out[horizons_date(row["Calendar Date (TDB)"])] = (
            float(row["X"]), float(row["Y"]), float(row["Z"]))
    return out


def period_days(command, date):
    """The osculating sidereal period at the date (Horizons ELEMENTS PR)."""
    result = horizons(dict(COMMAND=command, EPHEM_TYPE="'ELEMENTS'",
                           START_TIME="'%s'" % date_str(date),
                           STOP_TIME="'%s'" % date_str(next_day(date)), STEP_SIZE="'1d'"))
    return float(table(result)[0]["PR"])


# ---------------------------------------------------------------------------
# The fallback: Kepler from the element sets
# ---------------------------------------------------------------------------

def kepler_xyz(body, date):
    el = ELEMENTS[body]
    dt = julian_day(date) - ELEMENTS_EPOCH_JD
    m = math.radians((el["ma"] + el["n"] * dt) % 360.0)
    e = el["e"]
    big_e = m if e < 0.8 else math.pi
    for _ in range(12):
        big_e -= (big_e - e * math.sin(big_e) - m) / (1.0 - e * math.cos(big_e))
    a = el["a"]
    xp = a * (math.cos(big_e) - e)
    yp = a * math.sqrt(1.0 - e * e) * math.sin(big_e)
    w, om, inc = (math.radians(el[k]) for k in ("w", "om", "i"))
    cw, sw, co, so, ci, si = (math.cos(w), math.sin(w), math.cos(om), math.sin(om),
                              math.cos(inc), math.sin(inc))
    x = (co * cw - so * sw * ci) * xp + (-co * sw - so * cw * ci) * yp
    y = (so * cw + co * sw * ci) * xp + (-so * sw + co * cw * ci) * yp
    z = (sw * si) * xp + (cw * si) * yp
    return (x, y, z)


def kepler_vectors(body, first, last):
    out = {}
    date = first
    while date <= last:
        out[date] = kepler_xyz(body, date)
        date = next_day(date)
    return out


# ---------------------------------------------------------------------------
# The search and the block
# ---------------------------------------------------------------------------

def polar(xyz):
    x, y, z = xyz
    r = math.sqrt(x * x + y * y + z * z)
    lam = math.degrees(math.atan2(y, x)) % 360.0
    return r, lam


def candidates(ceres, mars, max_dl):
    out = []
    for date in sorted(ceres):
        if date not in mars or (date[1] == 2 and date[2] == 29):
            continue
        r1, l1 = polar(ceres[date])
        r2, l2 = polar(mars[date])
        dl = wrap180(l1 - l2)
        if abs(dl) <= max_dl:
            out.append((date, chord_au(r1, r2, dl), dl))
    return out


def chord_minimum(found):
    best = None
    for date, chord, dl in found:  # sorted by date: a tie keeps the earlier
        if best is None or chord < best[1]:
            best = (date, chord, dl)
    return best


def first_season(found):
    """The first contiguous run of qualifying dates."""
    season = []
    for item in found:
        if season and item[0] != next_day(season[-1][0]) and not (
                season[-1][0][1] == 2 and season[-1][0][2] == 28 and item[0][1] == 3
                and item[0][2] == 1):
            break
        season.append(item)
    return season


def kirkwood(ceres_rl, mars_rl):
    """The point on the Ceres-Mars segment (circular-model xy positions) at
    exactly KIRKWOOD_AU from the Sun, by bisection; its longitude."""
    (r1, l1), (r2, l2) = ceres_rl, mars_rl
    p1 = (r1 * math.cos(math.radians(l1)), r1 * math.sin(math.radians(l1)))
    p2 = (r2 * math.cos(math.radians(l2)), r2 * math.sin(math.radians(l2)))

    def at(s):
        return (p1[0] + (p2[0] - p1[0]) * s, p1[1] + (p2[1] - p1[1]) * s)

    def f(s):
        x, y = at(s)
        return math.hypot(x, y) - KIRKWOOD_AU

    lo, hi = 0.0, 1.0
    if f(lo) * f(hi) > 0:
        raise ValueError("Kirkwood's radius is not on the chord")
    for _ in range(200):
        mid = (lo + hi) / 2.0
        if f(lo) * f(mid) <= 0:
            hi = mid
        else:
            lo = mid
    x, y = at((lo + hi) / 2.0)
    return math.degrees(math.atan2(y, x)) % 360.0


def legs_block(fields, kms):
    """The two legs as the Rust derives them, at a coast rate."""
    def km(a, b):
        fa, fb = fields[a], fields[b]
        return separation_km(uau(fa["orbit_au"]), uau(fb["orbit_au"]),
                             mdeg(fa["phase_deg_at_epoch"]) - mdeg(fb["phase_deg_at_epoch"]))

    pairs = [("ceres", "kirkwood"), ("kirkwood", "mars")]
    legs = []
    for a, b in pairs:
        dist = km(a, b)
        legs.append(dict(leg="%s-%s" % (a, b), km=dist, au=round(dist / AU_KM, 4),
                         base_s=ceil_div(dist, kms)))
    chord = km("ceres", "mars")
    laden = {}
    for payload in REPORT_PAYLOADS_KG:
        secs = [ceil_div(leg["base_s"] * (PAYLOAD_CEILING_KG + payload), PAYLOAD_CEILING_KG)
                for leg in legs]
        laden[str(payload)] = dict(
            leg_s=secs, total_s=sum(secs),
            hours_at_rate=round(sum(secs) / WORLD_RATE / 3600.0, 2),
            crew_days_one_occupant=sum(ceil_div(s, 86_400) for s in secs))
    return dict(kms=kms, chord_km=chord, chord_au=round(chord / AU_KM, 4), legs=legs,
                laden_at_rate_24=laden)


def block(date, raw, periods, kms, source):
    """Everything one candidate date yields: the pack fields, the legs."""
    ic = epoch_ic(date)
    bodies = {}
    fields = {}
    for name in ("ceres", "mars", "pallas"):
        r, lam = polar(raw[name])
        x, y, z = raw[name]
        bodies[name] = dict(x_au=x, y_au=y, z_au=z, r_au=round(r, 6), lambda_deg=round(lam, 3))
        fields[name] = dict(orbit_au=round(r, 6), period_days=round(periods[name], 3),
                            phase_deg_at_epoch=round(lam, 3), epoch_ic=ic)
    k_lam = kirkwood(polar(raw["ceres"]), polar(raw["mars"]))
    bodies["kirkwood"] = dict(r_au=KIRKWOOD_AU, lambda_deg=round(k_lam, 3))
    fields["kirkwood"] = dict(orbit_au=KIRKWOOD_AU, period_days=KIRKWOOD_PERIOD_DAYS,
                              phase_deg_at_epoch=round(k_lam, 3), epoch_ic=ic)
    dl = wrap180(polar(raw["ceres"])[1] - polar(raw["mars"])[1])
    out = dict(date=date_str(date), epoch_ic=ic, source=source, delta_lambda_deg=round(dl, 3),
               bodies=bodies, pack_fields=fields)
    out.update(legs_block(fields, kms))
    return out


def fetch_all(names, first, last, offline):
    if offline:
        return {n: kepler_vectors(n, first, last) for n in names}, "elements"
    try:
        return {n: vectors(BODIES[n], first, last) for n in names}, "horizons"
    except (urllib.error.URLError, OSError, ValueError, KeyError):
        return {n: kepler_vectors(n, first, last) for n in names}, "elements"


def periods_at(date, source):
    if source == "horizons":
        try:
            return {n: period_days(BODIES[n], date) for n in ("ceres", "mars", "pallas")}
        except (urllib.error.URLError, OSError, ValueError, KeyError):
            pass
    return {n: ELEMENTS[n]["pr"] for n in ("ceres", "mars", "pallas")}


def pallas_at(date, source):
    if source == "horizons":
        try:
            return vectors(BODIES["pallas"], date, date)[date]
        except (urllib.error.URLError, OSError, ValueError, KeyError):
            pass
    return kepler_xyz("pallas", date)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--from", dest="first", default="2350-01-01")
    parser.add_argument("--to", dest="last", default="2352-12-31")
    parser.add_argument("--max-dl", dest="max_dl", type=float, default=11.0)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    first, last = parse_date(args.first), parse_date(args.last)
    notes = []

    series, source = fetch_all(("ceres", "mars"), first, last, args.offline)
    found = candidates(series["ceres"], series["mars"], args.max_dl)
    if not found:
        notes.append("no qualifying date in %s..%s; the window was widened by one year each "
                     "side, once" % (date_str(first), date_str(last)))
        first, last = add_years(first, -1), add_years(last, 1)
        series, source = fetch_all(("ceres", "mars"), first, last, args.offline)
        found = candidates(series["ceres"], series["mars"], args.max_dl)
    if not found:
        print(json.dumps(dict(error="no qualifying date", notes=notes), indent=2))
        return
    chosen = chord_minimum(found)[0]
    season = first_season([item for item in found if item[0] >= found[0][0]])
    raw = dict(ceres=series["ceres"][chosen], mars=series["mars"][chosen],
               pallas=pallas_at(chosen, source))
    periods = periods_at(chosen, source)
    out = block(chosen, raw, periods, COAST_KMS, source)
    out["window"] = [date_str(first), date_str(last)]
    out["max_dl_deg"] = args.max_dl
    out["qualifying_dates"] = [date_str(d) for d, _, _ in found]
    out["first_season"] = [date_str(season[0][0]), date_str(season[-1][0])]
    out["notes"] = notes

    # 5a (i): the chord minimum of the NEXT qualifying season after the window.
    after = next_day(last)
    horizon = add_years(last, 6)
    later, later_source = fetch_all(("ceres", "mars"), after, horizon, args.offline)
    later_found = candidates(later["ceres"], later["mars"], args.max_dl)
    if later_found:
        next_season = first_season(later_found)
        nd = chord_minimum(next_season)[0]
        nraw = dict(ceres=later["ceres"][nd], mars=later["mars"][nd],
                    pallas=pallas_at(nd, later_source))
        nblock = block(nd, nraw, periods_at(nd, later_source), COAST_KMS, later_source)
        nblock["season"] = [date_str(next_season[0][0]), date_str(next_season[-1][0])]
        if later_source == "elements":
            nblock["approximate"] = True
        out["next_season"] = nblock
    else:
        out["next_season"] = dict(error="no qualifying date through %s" % date_str(horizon))

    # 5a (ii): the chosen date at the coast rate nearest D-18-30's leg sum.
    leg_km = sum(leg["km"] for leg in out["legs"])
    kms = rround(leg_km / D18_30_SECONDS)
    alt = dict(date=out["date"], epoch_ic=out["epoch_ic"], pack_fields=out["pack_fields"])
    alt.update(legs_block(out["pack_fields"], kms))
    out["kms_matching_d18_30"] = alt

    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
