using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using TacSim.Core;
using UnityEngine;

namespace TacSim.Space
{
    public sealed class SolarChartVantage
    {
        public string Port = string.Empty;
        public string Body = string.Empty;
        public string Hull = string.Empty;
        public double PayloadKg;
        public int Occupants = 1;
    }

    public sealed class SolarChartBody
    {
        public string Id = string.Empty;
        public string Label = string.Empty;
        public string Glyph = "?";
        public string ParentId;
        public double RadiusAu;
        public double LonDeg;
        public double LatDeg;
        public double XAu;
        public double YAu;
        public double ZAu;
        public double PeriodDays;
        public bool Here;
        public double? FromHereAu;
        public double? LagSeconds;
    }

    public sealed class SolarChartLeg
    {
        public string From = string.Empty;
        public string To = string.Empty;
    }

    public sealed class SolarChartPassage
    {
        public double TotalSeconds;
        public double BurnTonnes;
        public double CrewDays;
    }

    public sealed class SolarChartDirect
    {
        public double Km;
        public double TotalSeconds;
        public double BurnTonnes;
    }

    public sealed class SolarChartRoute
    {
        public string To = string.Empty;
        public string Venue = string.Empty;
        public string Body = string.Empty;
        public readonly List<SolarChartLeg> Legs = new List<SolarChartLeg>();
        public SolarChartPassage Loaded;
        public SolarChartPassage Empty;
        public SolarChartDirect Direct;
    }

    public sealed class SolarChartWindowDay
    {
        public int OffsetDays;
        public double TotalSeconds;
        public double BurnTonnes;
    }

    public sealed class SolarChartWindow
    {
        public string To = string.Empty;
        public SolarChartPassage Now;
        public SolarChartWindowDay Fastest;
        public SolarChartWindowDay Cheapest;
        public SolarChartWindowDay Worst;
    }

    /// <summary>
    /// Decoded `sky.chart` solar system keyframe + server-parity logarithmic radial &amp; 3D tilted
    /// ecliptic projection math (mirrors `crates/tacsim/src/port/chart_panel.rs` and `web/src/lib/chart.ts`).
    /// </summary>
    public sealed class TacSimSolarChartFrame
    {
        public const double DefaultKneeR0Au = 0.3;
        public const double DefaultAuMax = 10.0;
        public const double AuPerLightMinute = 0.1202393;
        public const double KmPerAu = 149597871.0;
        public static readonly int[] LagRingMinutes = { 3, 6, 12, 24 };

        public string Date = string.Empty;
        public double IcNow;
        public string Model = string.Empty;
        public double RatePermille;
        public string ScaleKind = "log";
        public double ScaleAuMax = DefaultAuMax;
        public double ScaleR0Au = DefaultKneeR0Au;

        public SolarChartVantage Vantage = new SolarChartVantage();
        public readonly List<SolarChartBody> Bodies = new List<SolarChartBody>();
        public readonly List<SolarChartRoute> Routes = new List<SolarChartRoute>();
        public readonly List<SolarChartWindow> Windows = new List<SolarChartWindow>();

        public static TacSimSolarChartFrame FromJson(JsonNode raw)
        {
            if (raw == null || !raw.IsObject)
                return null;

            JsonNode c = raw["chart"].IsObject ? raw["chart"] : raw;
            if (!c.IsObject)
                return null;

            // Accept any object containing a `bodies` array or `vantage` block.
            if (!c["bodies"].IsArray && !c["vantage"].IsObject)
                return null;

            var frame = new TacSimSolarChartFrame
            {
                Date = c.Str("date", string.Empty),
                IcNow = c.Double("ic_now", 0.0),
                Model = c.Str("model", string.Empty),
                RatePermille = c.Double("rate_permille", 0.0)
            };

            JsonNode v = c["vantage"];
            if (v.IsObject)
            {
                frame.Vantage = new SolarChartVantage
                {
                    Port = v.Str("port", string.Empty),
                    Body = v.Str("body", string.Empty),
                    Hull = v.Str("hull", string.Empty),
                    PayloadKg = v.Double("payload_kg", 0.0),
                    Occupants = Math.Max(1, v.Int("occupants", 1))
                };
            }

            JsonNode sc = c["scale"];
            if (sc.IsObject)
            {
                frame.ScaleKind = sc.Str("kind", "log");
                double auMax = sc.Double("au_max", DefaultAuMax);
                double r0 = sc.Double("r0_au", DefaultKneeR0Au);
                frame.ScaleAuMax = auMax > 0.0 ? auMax : DefaultAuMax;
                frame.ScaleR0Au = r0 > 0.0 ? r0 : DefaultKneeR0Au;
            }

            var bodiesArr = c["bodies"].AsArray();
            if (bodiesArr != null)
            {
                for (int i = 0; i < bodiesArr.Count; i++)
                {
                    JsonNode bn = bodiesArr[i];
                    if (!bn.IsObject) continue;
                    string id = bn.Str("id", string.Empty);
                    if (string.IsNullOrEmpty(id)) continue;

                    string glyph = bn.Str("glyph", "?");
                    if (glyph.Length > 1) glyph = glyph.Substring(0, 1);

                    string parent = bn.ContainsKey("parent") && bn["parent"].Kind == JsonKind.String
                        ? bn.Str("parent", string.Empty)
                        : null;
                    if (string.IsNullOrEmpty(parent)) parent = null;

                    double? fromHere = bn.ContainsKey("from_here_au") && !bn["from_here_au"].IsNull
                        ? (double?)bn.Double("from_here_au", 0.0)
                        : null;
                    double? lagS = bn.ContainsKey("lag_s") && !bn["lag_s"].IsNull
                        ? (double?)bn.Double("lag_s", 0.0)
                        : null;

                    frame.Bodies.Add(new SolarChartBody
                    {
                        Id = id,
                        Label = bn.Str("label", id),
                        Glyph = string.IsNullOrEmpty(glyph) ? "?" : glyph,
                        ParentId = parent,
                        RadiusAu = bn.Double("r_au", 0.0),
                        LonDeg = bn.Double("lon_deg", 0.0),
                        LatDeg = bn.Double("lat_deg", 0.0),
                        XAu = bn.Double("x_au", 0.0),
                        YAu = bn.Double("y_au", 0.0),
                        ZAu = bn.Double("z_au", 0.0),
                        PeriodDays = bn.Double("period_days", 0.0),
                        Here = bn.Bool("here", false),
                        FromHereAu = fromHere,
                        LagSeconds = lagS
                    });
                }
            }

            var routesArr = c["routes"].AsArray();
            if (routesArr != null)
            {
                for (int i = 0; i < routesArr.Count; i++)
                {
                    JsonNode rn = routesArr[i];
                    if (!rn.IsObject) continue;
                    string to = rn.Str("to", string.Empty);
                    if (string.IsNullOrEmpty(to)) continue;

                    var r = new SolarChartRoute
                    {
                        To = to,
                        Venue = rn.Str("venue", string.Empty),
                        Body = rn.Str("body", string.Empty),
                        Loaded = ParsePassage(rn["loaded"]),
                        Empty = ParsePassage(rn["empty"])
                    };

                    JsonNode dir = rn["direct"];
                    if (dir.IsObject)
                    {
                        r.Direct = new SolarChartDirect
                        {
                            Km = dir.Double("km", 0.0),
                            TotalSeconds = dir.Double("total_s", 0.0),
                            BurnTonnes = dir.Double("burn_t", 0.0)
                        };
                    }

                    var legsArr = rn["legs"].AsArray();
                    if (legsArr != null)
                    {
                        for (int k = 0; k < legsArr.Count; k++)
                        {
                            JsonNode ln = legsArr[k];
                            if (!ln.IsObject) continue;
                            string from = ln.Str("from", string.Empty);
                            string legTo = ln.Str("to", string.Empty);
                            if (!string.IsNullOrEmpty(from) && !string.IsNullOrEmpty(legTo))
                                r.Legs.Add(new SolarChartLeg { From = from, To = legTo });
                        }
                    }

                    frame.Routes.Add(r);
                }
            }

            var winArr = c["window"].AsArray();
            if (winArr != null)
            {
                for (int i = 0; i < winArr.Count; i++)
                {
                    JsonNode wn = winArr[i];
                    if (!wn.IsObject) continue;
                    string to = wn.Str("to", string.Empty);
                    var now = ParsePassage(wn["now"]);
                    if (string.IsNullOrEmpty(to) || now == null) continue;

                    frame.Windows.Add(new SolarChartWindow
                    {
                        To = to,
                        Now = now,
                        Fastest = ParseWindowDay(wn["fastest"]),
                        Cheapest = ParseWindowDay(wn["cheapest"]),
                        Worst = ParseWindowDay(wn["worst"])
                    });
                }
            }

            return frame;
        }

        private static SolarChartPassage ParsePassage(JsonNode node)
        {
            if (node == null || !node.IsObject) return null;
            return new SolarChartPassage
            {
                TotalSeconds = node.Double("total_s", 0.0),
                BurnTonnes = node.Double("burn_t", 0.0),
                CrewDays = node.Double("crew_days", 0.0)
            };
        }

        private static SolarChartWindowDay ParseWindowDay(JsonNode node)
        {
            if (node == null || !node.IsObject) return null;
            return new SolarChartWindowDay
            {
                OffsetDays = node.Int("offset_days", 0),
                TotalSeconds = node.Double("total_s", 0.0),
                BurnTonnes = node.Double("burn_t", 0.0)
            };
        }

        public SolarChartBody FindBody(string id)
        {
            if (string.IsNullOrEmpty(id)) return null;
            for (int i = 0; i < Bodies.Count; i++)
            {
                if (string.Equals(Bodies[i].Id, id, StringComparison.OrdinalIgnoreCase))
                    return Bodies[i];
            }
            return null;
        }

        public SolarChartBody FindHereBody()
        {
            for (int i = 0; i < Bodies.Count; i++)
            {
                if (Bodies[i].Here) return Bodies[i];
            }
            return FindBody(Vantage.Body);
        }

        public SolarChartRoute FindRouteForBodyOrPort(string bodyOrPort)
        {
            if (string.IsNullOrEmpty(bodyOrPort)) return null;
            for (int i = 0; i < Routes.Count; i++)
            {
                if (string.Equals(Routes[i].To, bodyOrPort, StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(Routes[i].Body, bodyOrPort, StringComparison.OrdinalIgnoreCase))
                    return Routes[i];
            }
            return null;
        }

        public SolarChartWindow FindWindowForPort(string portId)
        {
            if (string.IsNullOrEmpty(portId)) return null;
            for (int i = 0; i < Windows.Count; i++)
            {
                if (string.Equals(Windows[i].To, portId, StringComparison.OrdinalIgnoreCase))
                    return Windows[i];
            }
            return null;
        }

        /// <summary>
        /// Server-parity logarithmic radial scale (`ln(1 + r/r0) / ln(1 + au_max/r0)` clamped to `[0, 1]`).
        /// </summary>
        public static double RadialFraction(double rAu, double auMax, double r0Au = DefaultKneeR0Au)
        {
            if (auMax <= 0.0 || rAu <= 0.0 || r0Au <= 0.0)
                return 0.0;
            double denom = Math.Log(1.0 + auMax / r0Au);
            if (denom <= 1e-9) return 0.0;
            return Math.Min(1.0, Math.Log(1.0 + rAu / r0Au) / denom);
        }

        /// <summary>
        /// Projects an ecliptic point `(rAu, lonDeg, latDeg)` onto a normalized `[0, 1] x [0, 1]` orrery canvas
        /// with optional 3D perspective tilt (`tiltDeg` in `0..68`), orbit yaw (`yawDeg`), zoom, and pan.
        /// Also outputs the un-lifted ecliptic foot point (`footUv`) so 3D view can draw vertical inclination stems.
        /// </summary>
        public Vector2 ProjectEclipticToUv(
            double rAu,
            double lonDeg,
            double latDeg,
            float zoom,
            Vector2 panUv,
            float tiltDeg,
            float yawDeg,
            out Vector2 footUv)
        {
            float f = (float)RadialFraction(rAu, ScaleAuMax, ScaleR0Au);
            float theta = (float)((lonDeg + yawDeg) * Math.PI / 180.0);
            float xPlane = f * Mathf.Cos(theta);
            float yPlane = f * Mathf.Sin(theta);

            float tiltRad = Mathf.Clamp(tiltDeg, 0f, 68f) * Mathf.Deg2Rad;
            float cosTilt = Mathf.Cos(tiltRad);
            float sinTilt = Mathf.Sin(tiltRad);

            // Subtle depth foreshortening when tilted into 3D holo-table view
            float persp = 1.0f / (1.0f - yPlane * sinTilt * 0.22f);
            float scale = 0.45f * Mathf.Clamp(zoom, 0.55f, 3.5f) * persp;

            float fx = 0.5f + panUv.x + xPlane * scale;
            float fy = 0.5f + panUv.y + yPlane * cosTilt * scale;
            footUv = new Vector2(fx, fy);

            // Exaggerate small ecliptic latitudes gently in 3D tilted mode so inclined orbits (Mercury/Ceres) read clearly
            float zLift = f * Mathf.Sin((float)(latDeg * Math.PI / 180.0)) * 2.2f * sinTilt * scale;
            return new Vector2(fx, fy + zLift);
        }

        public Vector2 ProjectBodyToUv(
            SolarChartBody body,
            float zoom,
            Vector2 panUv,
            float tiltDeg,
            float yawDeg,
            out Vector2 footUv)
        {
            if (body == null)
            {
                footUv = new Vector2(0.5f, 0.5f);
                return footUv;
            }
            return ProjectEclipticToUv(body.RadiusAu, body.LonDeg, body.LatDeg, zoom, panUv, tiltDeg, yawDeg, out footUv);
        }

        /// <summary>
        /// Projects a heliocentric body into the 3D Bridge Holo-Tank cylinder (`maxRadiusM` horizontal, `maxVertM` vertical).
        /// </summary>
        public Vector3 ProjectBodyToBridgeHoloTank(SolarChartBody body, float maxRadiusM = 0.68f, float maxVertM = 0.30f)
        {
            if (body == null) return Vector3.zero;
            float f = (float)RadialFraction(body.RadiusAu, ScaleAuMax, ScaleR0Au);
            float theta = (float)(body.LonDeg * Math.PI / 180.0);
            float x = f * Mathf.Cos(theta) * maxRadiusM;
            float z = f * Mathf.Sin(theta) * maxRadiusM;
            float y = Mathf.Clamp(f * Mathf.Sin((float)(body.LatDeg * Math.PI / 180.0)) * 2.4f * maxRadiusM, -maxVertM, maxVertM);
            return new Vector3(x, y, z);
        }

        /// <summary>
        /// Samples a closed light-lag ring `minutes` of light away from the current `here` body,
        /// warped through the logarithmic scale and active 3D tilt/yaw/zoom/pan transform.
        /// </summary>
        public List<Vector2> ComputeLagRingUv(int minutes, float zoom, Vector2 panUv, float tiltDeg, float yawDeg, int steps = 96)
        {
            var pts = new List<Vector2>(steps + 1);
            var here = FindHereBody();
            if (here == null || minutes <= 0) return pts;

            double lonRad = here.LonDeg * Math.PI / 180.0;
            double hx = here.RadiusAu * Math.Cos(lonRad);
            double hy = here.RadiusAu * Math.Sin(lonRad);
            double radiusAu = minutes * AuPerLightMinute;

            for (int i = 0; i <= steps; i++)
            {
                double a = (2.0 * Math.PI * i) / steps;
                double wx = hx + radiusAu * Math.Cos(a);
                double wy = hy + radiusAu * Math.Sin(a);
                double r = Math.Sqrt(wx * wx + wy * wy);
                double lon = Math.Atan2(wy, wx) * 180.0 / Math.PI;
                pts.Add(ProjectEclipticToUv(r, lon, 0.0, zoom, panUv, tiltDeg, yawDeg, out _));
            }
            return pts;
        }

        public List<Vector2> ComputeRoutePolylineUv(SolarChartRoute route, float zoom, Vector2 panUv, float tiltDeg, float yawDeg)
        {
            var pts = new List<Vector2>();
            if (route == null) return pts;
            for (int i = 0; i < route.Legs.Count; i++)
            {
                var leg = route.Legs[i];
                var fromBody = FindBody(leg.From);
                var toBody = FindBody(leg.To);
                if (i == 0 && fromBody != null)
                    pts.Add(ProjectBodyToUv(fromBody, zoom, panUv, tiltDeg, yawDeg, out _));
                if (toBody != null)
                    pts.Add(ProjectBodyToUv(toBody, zoom, panUv, tiltDeg, yawDeg, out _));
            }
            return pts;
        }

        public SolarChartBody FindNearestPrimaryBody(Vector2 clickUv, float zoom, Vector2 panUv, float tiltDeg, float yawDeg, float maxDistUv = 0.06f)
        {
            SolarChartBody best = null;
            float bestDistSq = maxDistUv * maxDistUv;
            for (int i = 0; i < Bodies.Count; i++)
            {
                var b = Bodies[i];
                if (!string.IsNullOrEmpty(b.ParentId)) continue;
                Vector2 uv = ProjectBodyToUv(b, zoom, panUv, tiltDeg, yawDeg, out _);
                float dx = uv.x - clickUv.x;
                float dy = uv.y - clickUv.y;
                float dSq = dx * dx + dy * dy;
                if (dSq < bestDistSq)
                {
                    bestDistSq = dSq;
                    best = b;
                }
            }
            return best;
        }

        public static string FormatHours(double totalSeconds)
        {
            return (totalSeconds / 3600.0).ToString("0.0", CultureInfo.InvariantCulture) + " h";
        }

        public static string FormatLightLag(double seconds)
        {
            if (seconds < 0.0) seconds = 0.0;
            int m = (int)Math.Floor(seconds / 60.0);
            int s = (int)Math.Floor(seconds % 60.0);
            return $"{m}m {s:00}s";
        }

        public string BuildHeadingText()
        {
            string motion = Math.Abs(RatePermille) < 0.001
                ? "constant"
                : $"moving ({RatePermille.ToString("0.#", CultureInfo.InvariantCulture)}‰)";
            string portPart = !string.IsNullOrEmpty(Vantage.Port) ? Vantage.Port : "orbit";
            string bodyPart = !string.IsNullOrEmpty(Vantage.Body) ? Vantage.Body : "sol";
            return $"SOLAR CHART  //  {Date}  [{Model}; {motion}]  —  Vantage: {portPart.ToUpperInvariant()} ({bodyPart.ToUpperInvariant()})";
        }

        public string BuildBodyAndRouteDossier(SolarChartBody body, SolarChartRoute route)
        {
            var sb = new StringBuilder(384);
            if (body != null)
            {
                sb.AppendFormat(CultureInfo.InvariantCulture, "<b>{0}</b> ({1})\n", body.Label.ToUpperInvariant(), body.Id);
                sb.AppendFormat(CultureInfo.InvariantCulture, "Orbit: <b>{0:0.000} AU</b>  |  Lon {1:0.1}°  Lat {2:+0.1;-0.1;0.0}°\n",
                    body.RadiusAu, body.LonDeg, body.LatDeg);
                if (body.PeriodDays > 0.0)
                    sb.AppendFormat(CultureInfo.InvariantCulture, "Period: {0:0.0} d\n", body.PeriodDays);

                if (body.Here || !body.FromHereAu.HasValue)
                {
                    sb.Append("<color=#F2C94C>Status: CURRENT VANTAGE BODY</color>\n");
                }
                else
                {
                    string lag = body.LagSeconds.HasValue ? FormatLightLag(body.LagSeconds.Value) : "—";
                    sb.AppendFormat(CultureInfo.InvariantCulture, "Range: <b>{0:0.000} AU</b>  |  Light Lag: <b>{1}</b>\n",
                        body.FromHereAu.Value, lag);
                }

                // List any moons orbiting this body
                var moons = new List<string>();
                for (int i = 0; i < Bodies.Count; i++)
                {
                    if (string.Equals(Bodies[i].ParentId, body.Id, StringComparison.OrdinalIgnoreCase))
                        moons.Add(Bodies[i].Label);
                }
                if (moons.Count > 0)
                    sb.Append("Moons: ").Append(string.Join(", ", moons)).Append('\n');
            }

            if (route != null)
            {
                sb.AppendFormat(CultureInfo.InvariantCulture, "\n<color=#F2C94C><b>CLEARED PASSAGE -> {0}</b></color>\n", route.To.ToUpperInvariant());
                if (route.Legs.Count > 0)
                {
                    var hops = new List<string>(route.Legs.Count + 1) { route.Legs[0].From };
                    for (int i = 0; i < route.Legs.Count; i++) hops.Add(route.Legs[i].To);
                    sb.Append("Legs: ").Append(string.Join(" -> ", hops)).Append('\n');
                }
                if (route.Loaded != null)
                {
                    sb.AppendFormat(CultureInfo.InvariantCulture,
                        "Loaded ({0} t on {1}): <b>{2}</b> | Burn <b>{3:0.0} t</b> | {4:0.0} crew-d\n",
                        Math.Floor(Vantage.PayloadKg / 1000.0),
                        string.IsNullOrEmpty(Vantage.Hull) ? "hull" : Vantage.Hull,
                        FormatHours(route.Loaded.TotalSeconds),
                        route.Loaded.BurnTonnes,
                        route.Loaded.CrewDays);
                }
                else
                {
                    sb.Append("<color=#FF8866>Not plannable at current payload mass.</color>\n");
                }
                if (route.Empty != null)
                {
                    sb.AppendFormat(CultureInfo.InvariantCulture,
                        "Empty hull: {0} | Burn {1:0.0} t\n",
                        FormatHours(route.Empty.TotalSeconds),
                        route.Empty.BurnTonnes);
                }
                if (route.Direct != null)
                {
                    sb.AppendFormat(CultureInfo.InvariantCulture,
                        "Direct chord ({0:0.000} AU): {1} | Burn {2:0.0} t\n",
                        route.Direct.Km / KmPerAu,
                        FormatHours(route.Direct.TotalSeconds),
                        route.Direct.BurnTonnes);
                }

                var win = FindWindowForPort(route.To);
                if (win != null)
                {
                    sb.Append("\n<color=#66DDFF><b>1-YEAR LAUNCH WINDOW</b></color>\n");
                    if (win.Fastest == null)
                    {
                        sb.Append("Constant sky — launch cost is identical any day.\n");
                    }
                    else
                    {
                        sb.AppendFormat(CultureInfo.InvariantCulture, "Fastest (+{0}d): {1} | {2:0.0} t burn\n",
                            win.Fastest.OffsetDays, FormatHours(win.Fastest.TotalSeconds), win.Fastest.BurnTonnes);
                        if (win.Cheapest != null)
                        {
                            sb.AppendFormat(CultureInfo.InvariantCulture, "Cheapest (+{0}d): {1} | {2:0.0} t burn\n",
                                win.Cheapest.OffsetDays, FormatHours(win.Cheapest.TotalSeconds), win.Cheapest.BurnTonnes);
                        }
                        if (win.Worst != null)
                        {
                            sb.AppendFormat(CultureInfo.InvariantCulture, "Worst (+{0}d): {1} | {2:0.0} t burn\n",
                                win.Worst.OffsetDays, FormatHours(win.Worst.TotalSeconds), win.Worst.BurnTonnes);
                        }
                    }
                }
            }

            return sb.ToString().TrimEnd();
        }
    }
}
