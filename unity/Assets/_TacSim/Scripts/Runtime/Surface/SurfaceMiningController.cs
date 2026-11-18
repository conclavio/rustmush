using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Text.RegularExpressions;
using TacSim.Core;
using TacSim.World;
using TMPro;
using UnityEngine;
using UnityEngine.EventSystems;

namespace TacSim.Surface
{
    public struct RoverLegWaypoint
    {
        public long XCm;
        public long YCm;
        public double AtSec;
    }

    /// <summary>
    /// Pure-online 3D Planetary Surface Heightfield, Landed Dray-P Pad, and
    /// Articulated Prospector Rover (`+tv`) driven by authoritative `mushd` `site.map`
    /// (`rows`, `site.grid`, `site.clock`, `site.rigs[]`, `readings[]`, `others[]`) and `tv.status` feeds.
    /// </summary>
    public sealed class SurfaceMiningController : MonoBehaviour
    {
        public static readonly Vector3 SurfaceStageOrigin = new Vector3(1200f, 0f, 0f);
        private static readonly Regex HopperPairRegex = new Regex(
            @"(\d+)\s*(?:of|/)\s*(\d+)\s*t\b",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        private static double MonotonicSeconds() =>
            Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency;

        public TacSimAssetCatalog Catalog;
        public DecodedTerrainGrid TerrainGrid { get; private set; }
        public SpawnedGroundVehicleRig ActiveRoverRig { get; private set; }
        public Transform RoverTransform => ActiveRoverRig?.Root != null ? ActiveRoverRig.Root.transform : null;
        public Transform RoverCupolaAnchor => ActiveRoverRig?.CupolaCameraAnchor;

        public bool IsExcursionActive { get; private set; }
        public bool KeyboardSuspended { get; set; }
        public string ActiveSiteSlug { get; private set; } = "occator_rim";
        public string ActiveSiteTitle { get; private set; } = "Occator Rim (Ceres)";
        public string ActiveOreFamily { get; private set; } = "volatiles";
        public int AddressedRigSlot { get; private set; } = 1;
        public bool RigOnGround { get; private set; }
        public int AuthoritativeSiteCellX { get; private set; } = 128;
        public int AuthoritativeSiteCellY { get; private set; } = 128;
        public int HopperLoadedTonnes { get; private set; }
        public int HopperCapacityTonnes { get; private set; } = 6;
        public int BatteryPct { get; private set; } = 100;
        public int HoldUnreservedTonnes { get; private set; } = 86;
        public string StatusSummaryLine { get; private set; } = "Rig 1 cradled in Dray-P Vehicle Bay.";

        public event Action<int, int> OnTerrainCellDriveClicked;
        public event Action OnSurfaceTelemetryChanged;
        public event Action<string> OnSurfaceLogEvent;

        private Transform _terrainRoot;
        private Transform _padRoot;
        private Transform _markersRoot;
        private Transform _otherRigsRoot;
        private Transform _scanPulseRing;
        private Transform _drillSparksGlow;
        private float _pulseScale = -1f;
        private Color _pulseColor = Color.cyan;
        private float _drillActiveUntil;

        private Vector3 _roverTargetLocalPos;
        private float _roverHeadingDeg = 180f;
        private float _roverSpeedKph;
        private bool _hasDriveWaypoint;
        private float _nextKeyboardDriveCommandAt;

        private readonly List<RoverLegWaypoint> _activeLegWaypoints = new List<RoverLegWaypoint>();
        private long _activeLegStartedIc;
        private double _clockBaseIcSec;
        private double _clockRateIcPerRealSec = 1.0;
        private double _clockBaseMonotonicSec;

        public IReadOnlyList<RoverLegWaypoint> ActiveLegWaypoints => _activeLegWaypoints;
        public long ActiveLegStartedIc => _activeLegStartedIc;

        private readonly List<string> _recentSiteLines = new List<string>();
        public IReadOnlyList<string> RecentSiteLines => _recentSiteLines;

        public void BuildSurfaceStage(TacSimAssetCatalog catalog, string siteSlug = "occator_rim")
        {
            Catalog = catalog;
            ActiveSiteSlug = string.IsNullOrWhiteSpace(siteSlug) ? "occator_rim" : siteSlug.Trim().ToLowerInvariant();

            if (_terrainRoot != null)
                TacSimTerrainDecoder.DestroyTerrainChunkResources(_terrainRoot);

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _terrainRoot = null;
            _padRoot = null;

            transform.position = SurfaceStageOrigin;
            // Do not invent a 64x64 procedural grid at startup — wait for authoritative `site.map` keyframes!
            TerrainGrid = null;
            ActiveSiteTitle = ActiveSiteSlug;

            _terrainRoot = new GameObject("PlanetaryTerrainMesh").transform;
            _terrainRoot.SetParent(transform, false);

            _markersRoot = new GameObject("OrePocketBeacons").transform;
            _markersRoot.SetParent(transform, false);

            _otherRigsRoot = new GameObject("OtherSurfaceRigs").transform;
            _otherRigsRoot.SetParent(transform, false);

            BuildLandedDrayCarrierOnPad();
            SpawnPlayerProspectorRover();
            BuildScanAndDrillVfx();
        }

        private void SyncPadPlacementFromGrid()
        {
            if (_padRoot == null || TerrainGrid == null) return;
            // Place carrier pad strictly from authoritative `site.marks.hull` (`[fx, fy]`) rather than glyph search!
            if (TerrainGrid.HasAuthoritativeHullCell)
            {
                _padRoot.gameObject.SetActive(true);
                _padRoot.position = SurfaceStageOrigin + TerrainGrid.CellToLocalPos(TerrainGrid.HullFrameX, TerrainGrid.HullFrameY, 0f);
            }
            else
            {
                _padRoot.gameObject.SetActive(false);
            }
        }

        private void RebuildLiveTerrainMesh(DecodedTerrainGrid liveGrid)
        {
            if (liveGrid == null) return;
            TerrainGrid = liveGrid;

            if (_terrainRoot != null)
            {
                TacSimTerrainDecoder.DestroyTerrainChunkResources(_terrainRoot);
                for (int i = _terrainRoot.childCount - 1; i >= 0; i--)
                {
                    var ch = _terrainRoot.GetChild(i).gameObject;
                    if (Application.isPlaying) Destroy(ch);
                    else DestroyImmediate(ch);
                }
            }
            else
            {
                _terrainRoot = new GameObject("PlanetaryTerrainMesh").transform;
                _terrainRoot.SetParent(transform, false);
            }

            TacSimTerrainDecoder.BuildChunkedTerrainMeshes(TerrainGrid, _terrainRoot, chunkCells: 32);
            SyncPadPlacementFromGrid();
        }

        private void BuildLandedDrayCarrierOnPad()
        {
            Vector3 padWorld = SurfaceStageOrigin + new Vector3(140f, 0f, 140f);

            _padRoot = new GameObject("LandedCarrierPad").transform;
            _padRoot.SetParent(transform, false);
            _padRoot.position = padWorld;
            _padRoot.gameObject.SetActive(false);

            Material apronMat = UrpMaterialFactory.GetOpaque("PadApron", new Color(0.24f, 0.27f, 0.31f), 0.38f, 0.55f);
            Material rampMat = UrpMaterialFactory.GetEmissive("SternRamp", new Color(0.45f, 0.36f, 0.14f), new Color(0.85f, 0.58f, 0.12f) * 0.65f);

            var apron = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            apron.name = "LandingApron";
            apron.transform.SetParent(_padRoot, false);
            apron.transform.localPosition = new Vector3(0f, 0.08f, 0f);
            apron.transform.localScale = new Vector3(26f, 0.12f, 26f);
            var ac = apron.GetComponent<Collider>();
            if (ac != null) { if (Application.isPlaying) Destroy(ac); else DestroyImmediate(ac); }
            apron.GetComponent<MeshRenderer>().sharedMaterial = apronMat;

            if (Catalog != null && Catalog.drayExteriorHullPrefab != null)
            {
                PrefabFittingHelper.SpawnGrounded(Catalog.drayExteriorHullPrefab, _padRoot, "SettledDrayPHull", padWorld + Vector3.up * 0.2f, Quaternion.identity, 8.5f, 26f, keepColliders: true);
            }
            else
            {
                Material hMat = UrpMaterialFactory.GetOpaque("PadDrayHull", new Color(0.48f, 0.45f, 0.40f), 0.4f, 0.55f);
                Material tMat = UrpMaterialFactory.GetOpaque("PadDrayTrim", new Color(0.22f, 0.26f, 0.32f), 0.45f, 0.7f);
                StationWorldController.BuildSculptedDrayExterior(_padRoot, padWorld + Vector3.up * 3.2f, hMat, tMat, "SettledDrayPHull");
            }

            var ramp = GameObject.CreatePrimitive(PrimitiveType.Cube);
            ramp.name = "LoweredSternRamp";
            ramp.transform.SetParent(_padRoot, false);
            ramp.transform.localPosition = new Vector3(0f, 0.65f, -12.8f);
            ramp.transform.localRotation = Quaternion.Euler(14f, 0f, 0f);
            ramp.transform.localScale = new Vector3(4.6f, 0.22f, 5.4f);
            ramp.GetComponent<MeshRenderer>().sharedMaterial = rampMat;

            var floodGo = new GameObject("PadFloodLight");
            floodGo.transform.SetParent(_padRoot, false);
            floodGo.transform.localPosition = new Vector3(0f, 14f, -10f);
            floodGo.transform.localRotation = Quaternion.Euler(50f, 0f, 0f);
            var lt = floodGo.AddComponent<Light>();
            lt.type = LightType.Spot;
            lt.spotAngle = 88f;
            lt.range = 65f;
            lt.intensity = 16f;
            lt.color = new Color(0.92f, 0.96f, 1f);
        }

        private void SpawnPlayerProspectorRover()
        {
            AuthoritativeSiteCellX = 128;
            AuthoritativeSiteCellY = 124;
            _roverTargetLocalPos = new Vector3(140f, 0f, 120f);
            Vector3 worldPos = SurfaceStageOrigin + _roverTargetLocalPos;
            ActiveRoverRig = TacSimVehicleVisualFactory.SpawnSurfaceRover(Catalog, transform, "RIG 1 [@]", worldPos, 180f, isEscort: false);
            _roverHeadingDeg = 180f;
            RigOnGround = true;
        }

        private void BuildScanAndDrillVfx()
        {
            var ring = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            ring.name = "ScanShockwaveRing";
            ring.transform.SetParent(transform, false);
            ring.transform.localScale = new Vector3(2f, 0.08f, 2f);
            var rc = ring.GetComponent<Collider>();
            if (rc != null) { if (Application.isPlaying) Destroy(rc); else DestroyImmediate(rc); }
            ring.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("SurfaceScanWave", new Color(0.25f, 0.92f, 1.0f, 0.55f));
            _scanPulseRing = ring.transform;
            _scanPulseRing.gameObject.SetActive(false);

            var sparks = GameObject.CreatePrimitive(PrimitiveType.Sphere);
            sparks.name = "CoreDrillGlow";
            sparks.transform.SetParent(transform, false);
            sparks.transform.localScale = Vector3.one * 1.35f;
            var sc = sparks.GetComponent<Collider>();
            if (sc != null) { if (Application.isPlaying) Destroy(sc); else DestroyImmediate(sc); }
            sparks.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetEmissive("DrillGlow", new Color(1f, 0.65f, 0.15f), new Color(1f, 0.72f, 0.2f) * 2.6f);
            _drillSparksGlow = sparks.transform;
            _drillSparksGlow.gameObject.SetActive(false);
        }

        public void EnterSurfaceMode()
        {
            if (TerrainGrid == null)
                BuildSurfaceStage(Catalog, ActiveSiteSlug);
            IsExcursionActive = true;
            RigOnGround = true;
            OnSurfaceTelemetryChanged?.Invoke();
        }

        public void ExitSurfaceMode()
        {
            IsExcursionActive = false;
            OnSurfaceTelemetryChanged?.Invoke();
        }

        public void SetOreFamilyFilter(string family)
        {
            if (string.IsNullOrWhiteSpace(family)) return;
            ActiveOreFamily = family.Trim().ToLowerInvariant();
            OnSurfaceTelemetryChanged?.Invoke();
        }

        public static bool TryParseHopperTonnes(string hopperText, out int loadedTonnes, out int capacityTonnes)
        {
            return TryParseHopperTonnes(hopperText, out loadedTonnes, out capacityTonnes, out _);
        }

        public static bool TryParseHopperTonnes(string hopperText, out int loadedTonnes, out int capacityTonnes, out bool hasExplicitCapacity)
        {
            loadedTonnes = 0;
            capacityTonnes = 6;
            hasExplicitCapacity = false;
            if (string.IsNullOrWhiteSpace(hopperText))
                return false;

            Match m = HopperPairRegex.Match(hopperText);
            if (m.Success &&
                int.TryParse(m.Groups[1].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out int ld) &&
                int.TryParse(m.Groups[2].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out int cap))
            {
                loadedTonnes = Mathf.Max(0, ld);
                capacityTonnes = Mathf.Max(1, cap);
                hasExplicitCapacity = true;
                return true;
            }

            if (hopperText.IndexOf("empty", StringComparison.OrdinalIgnoreCase) >= 0)
            {
                loadedTonnes = 0;
                return true;
            }
            return false;
        }

        public static bool ParseLegWaypoints(JsonNode legNode, out long startedIc, List<RoverLegWaypoint> outWaypoints)
        {
            startedIc = 0;
            outWaypoints?.Clear();
            if (legNode == null || !legNode.IsObject || outWaypoints == null)
                return false;

            startedIc = legNode.Long("started_ic", 0L);
            var wps = legNode["waypoints"].AsArray();
            if (wps == null || wps.Count == 0)
                return false;

            for (int i = 0; i < wps.Count; i++)
            {
                var wp = wps[i];
                if (wp.IsObject)
                {
                    if (!wp.ContainsKey("x_cm") || !wp.ContainsKey("y_cm"))
                        continue;
                    outWaypoints.Add(new RoverLegWaypoint
                    {
                        XCm = wp.Long("x_cm", 0L),
                        YCm = wp.Long("y_cm", 0L),
                        AtSec = Math.Max(0.0, wp.Double("at_s", 0.0))
                    });
                }
                else if (wp.IsArray && wp.AsArray().Count >= 2)
                {
                    var arr = wp.AsArray();
                    long rx = arr[0].AsLong(0L);
                    long ry = arr[1].AsLong(0L);
                    outWaypoints.Add(new RoverLegWaypoint
                    {
                        XCm = rx > 1024 ? rx : rx * 1000L + 500L,
                        YCm = ry > 1024 ? ry : ry * 1000L + 500L,
                        AtSec = arr.Count >= 3 ? Math.Max(0.0, arr[2].AsDouble(0.0)) : i * 10.0
                    });
                }
            }
            return outWaypoints.Count > 0;
        }

        /// <summary>
        /// Exact port of <c>tacsim::port::site_map::leg_position</c>: piecewise-linear interpolation
        /// across <c>{x_cm, y_cm, at_s}</c> waypoints by elapsed IC seconds since <c>started_ic</c>.
        /// </summary>
        public static bool EvaluateLegPositionCm(
            IReadOnlyList<RoverLegWaypoint> waypoints,
            double elapsedIcSec,
            out double xCm,
            out double yCm,
            out float headingDeg,
            out double segmentSpeedMps,
            out bool stillMoving)
        {
            xCm = 0.0;
            yCm = 0.0;
            headingDeg = 180f;
            segmentSpeedMps = 0.0;
            stillMoving = false;
            if (waypoints == null || waypoints.Count == 0)
                return false;

            var first = waypoints[0];
            if (elapsedIcSec <= first.AtSec)
            {
                xCm = first.XCm;
                yCm = first.YCm;
                if (waypoints.Count >= 2)
                {
                    var second = waypoints[1];
                    double dx0 = second.XCm - first.XCm;
                    double dy0 = second.YCm - first.YCm;
                    if (Math.Abs(dx0) > 0.01 || Math.Abs(dy0) > 0.01)
                        headingDeg = Mathf.Atan2((float)dx0, (float)dy0) * Mathf.Rad2Deg;
                    stillMoving = waypoints[waypoints.Count - 1].AtSec > elapsedIcSec;
                }
                return true;
            }

            for (int i = 0; i < waypoints.Count - 1; i++)
            {
                var a = waypoints[i];
                var b = waypoints[i + 1];
                if (elapsedIcSec <= b.AtSec)
                {
                    double dx = b.XCm - a.XCm;
                    double dy = b.YCm - a.YCm;
                    if (Math.Abs(dx) > 0.01 || Math.Abs(dy) > 0.01)
                        headingDeg = Mathf.Atan2((float)dx, (float)dy) * Mathf.Rad2Deg;

                    if (b.AtSec <= a.AtSec)
                    {
                        xCm = b.XCm;
                        yCm = b.YCm;
                        stillMoving = i + 1 < waypoints.Count - 1;
                        return true;
                    }

                    double span = b.AtSec - a.AtSec;
                    double t = (elapsedIcSec - a.AtSec) / span;
                    xCm = a.XCm + dx * t;
                    yCm = a.YCm + dy * t;
                    stillMoving = (i + 1 < waypoints.Count - 1) || (elapsedIcSec < b.AtSec);
                    segmentSpeedMps = stillMoving ? (Math.Sqrt(dx * dx + dy * dy) * 0.01) / span : 0.0;
                    return true;
                }
            }

            var last = waypoints[waypoints.Count - 1];
            xCm = last.XCm;
            yCm = last.YCm;
            if (waypoints.Count >= 2)
            {
                var prev = waypoints[waypoints.Count - 2];
                double dxl = last.XCm - prev.XCm;
                double dyl = last.YCm - prev.YCm;
                if (Math.Abs(dxl) > 0.01 || Math.Abs(dyl) > 0.01)
                    headingDeg = Mathf.Atan2((float)dxl, (float)dyl) * Mathf.Rad2Deg;
            }
            stillMoving = false;
            return true;
        }

        private void ApplyRigRowNode(JsonNode rig)
        {
            if (rig == null || !rig.IsObject) return;
            if (rig.ContainsKey("charge_pct") && !rig["charge_pct"].IsNull)
                BatteryPct = Mathf.Clamp(rig.Int("charge_pct", BatteryPct), 0, 100);

            if (rig.ContainsKey("load_t") || rig.ContainsKey("cap_t"))
            {
                HopperLoadedTonnes = Mathf.Max(0, rig.Int("load_t", HopperLoadedTonnes));
                HopperCapacityTonnes = Mathf.Max(1, rig.Int("cap_t", HopperCapacityTonnes));
            }
            else if (rig.ContainsKey("hopper"))
            {
                string hopStr = rig.Str("hopper", string.Empty);
                if (TryParseHopperTonnes(hopStr, out int ld, out int cap, out bool hasCap))
                {
                    HopperLoadedTonnes = ld;
                    if (hasCap)
                        HopperCapacityTonnes = cap;
                }
            }

            var cellArr = rig["cell"].AsArray();
            if (cellArr != null && cellArr.Count >= 2)
            {
                AuthoritativeSiteCellX = cellArr[0].AsInt(AuthoritativeSiteCellX);
                AuthoritativeSiteCellY = cellArr[1].AsInt(AuthoritativeSiteCellY);
            }
        }

        public void ApplyTvStatusFrame(JsonNode frame)
        {
            if (frame == null || !frame.IsObject) return;
            if (frame.ContainsKey("slot"))
                AddressedRigSlot = Mathf.Max(1, frame.Int("slot", AddressedRigSlot));
            ApplyRigRowNode(frame["rig"].IsObject ? frame["rig"] : frame);
            OnSurfaceTelemetryChanged?.Invoke();
        }

        public void ApplySiteMapFrame(JsonNode frame)
        {
            if (frame == null || !frame.IsObject) return;

            // 1. Decode live RLE terrain & site.grid coordinate frame (`x0`, `y0`, `span`, `width`, `height`)
            var decoded = TacSimTerrainDecoder.DecodeSiteMapFrame(frame);
            if (decoded != null)
            {
                bool shouldRebuildMesh = TerrainGrid == null ||
                                         !TerrainGrid.IsFromLiveServerFeed ||
                                         TerrainGrid.Width != decoded.Width ||
                                         TerrainGrid.Height != decoded.Height ||
                                         TerrainGrid.OriginCellX != decoded.OriginCellX ||
                                         TerrainGrid.OriginCellY != decoded.OriginCellY ||
                                         TerrainGrid.SpanCells != decoded.SpanCells;
                if (shouldRebuildMesh)
                {
                    RebuildLiveTerrainMesh(decoded);
                }
                else
                {
                    TerrainGrid = decoded;
                    SyncPadPlacementFromGrid();
                }
            }

            var siteBlock = frame["site"];
            bool positionedFromRigRow = false;
            if (siteBlock.IsObject)
            {
                ActiveSiteTitle = siteBlock.Str("site", ActiveSiteTitle);
                ActiveOreFamily = siteBlock.Str("family_word", siteBlock.Str("family", ActiveOreFamily));
                AddressedRigSlot = Mathf.Max(1, siteBlock.Int("addressed", AddressedRigSlot));
                var holdObj = siteBlock["hold"];
                if (holdObj.IsObject)
                    HoldUnreservedTonnes = holdObj.Int("room_t", HoldUnreservedTonnes);

                var clockObj = siteBlock["clock"];
                if (clockObj.IsObject && clockObj.ContainsKey("now_ic"))
                {
                    _clockBaseIcSec = clockObj.Double("now_ic", _clockBaseIcSec);
                    _clockRateIcPerRealSec = Math.Max(0.0, clockObj.Double("rate", 1.0));
                    _clockBaseMonotonicSec = MonotonicSeconds();
                }

                // 2. Read authoritative battery, hopper ("3 of 6 t"), cell, and active leg from `site.rigs[]`!
                var rigsArr = siteBlock["rigs"].AsArray();
                if (rigsArr != null && rigsArr.Count > 0)
                {
                    JsonNode matchedRig = JsonNode.Null;
                    for (int i = 0; i < rigsArr.Count; i++)
                    {
                        var r = rigsArr[i];
                        if (r.IsObject && r.Int("slot", 0) == AddressedRigSlot)
                        {
                            matchedRig = r;
                            break;
                        }
                    }
                    if (!matchedRig.IsObject)
                        matchedRig = rigsArr[0];

                    if (matchedRig.IsObject)
                    {
                        ApplyRigRowNode(matchedRig);
                        if (TerrainGrid != null && matchedRig["cell"].IsArray && matchedRig["cell"].AsArray().Count >= 2)
                        {
                            var rc = matchedRig["cell"].AsArray();
                            AuthoritativeSiteCellX = rc[0].AsInt(AuthoritativeSiteCellX);
                            AuthoritativeSiteCellY = rc[1].AsInt(AuthoritativeSiteCellY);
                            UpdateRoverTargetFromSiteCell(AuthoritativeSiteCellX, AuthoritativeSiteCellY, matchedRig["leg"]);
                            positionedFromRigRow = true;
                        }
                        else if (matchedRig.ContainsKey("cell") && matchedRig["cell"].IsNull)
                        {
                            _activeLegWaypoints.Clear();
                            _hasDriveWaypoint = false;
                            RigOnGround = false;
                            positionedFromRigRow = true;
                        }
                    }
                }

                _recentSiteLines.Clear();
                var linesArr = siteBlock["lines"].AsArray();
                if (linesArr != null)
                {
                    for (int i = 0; i < linesArr.Count; i++)
                    {
                        string ln = linesArr[i].AsString();
                        if (!string.IsNullOrWhiteSpace(ln)) _recentSiteLines.Add(ln);
                    }
                }
                var logArr = siteBlock["log"].AsArray();
                if (logArr != null && logArr.Count > 0)
                {
                    string latest = logArr[logArr.Count - 1].AsString(StatusSummaryLine);
                    if (!string.IsNullOrWhiteSpace(latest) && latest != StatusSummaryLine)
                    {
                        StatusSummaryLine = latest;
                        OnSurfaceLogEvent?.Invoke(latest);
                    }
                }
                else if (_recentSiteLines.Count > 0)
                {
                    StatusSummaryLine = _recentSiteLines[0];
                }

                RebuildLiveReadingsMarkers(siteBlock["readings"].AsArray());
                RebuildOtherRigsVisuals(siteBlock["others"].AsArray());
            }

            if (!positionedFromRigRow && TerrainGrid != null)
            {
                var youArr = frame["you"].AsArray();
                if (youArr != null && youArr.Count >= 2)
                {
                    int localFx = youArr[0].AsInt(0);
                    int localFy = youArr[1].AsInt(0);
                    AuthoritativeSiteCellX = TerrainGrid.LocalToSiteCellX(localFx);
                    AuthoritativeSiteCellY = TerrainGrid.LocalToSiteCellY(localFy);
                    UpdateRoverTargetFromSiteCell(AuthoritativeSiteCellX, AuthoritativeSiteCellY, JsonNode.Null);
                }
            }

            OnSurfaceTelemetryChanged?.Invoke();
        }

        private void UpdateRoverTargetFromSiteCell(int siteX, int siteY, JsonNode legNode)
        {
            if (TerrainGrid == null) return;

            _activeLegWaypoints.Clear();
            bool hasLeg = ParseLegWaypoints(legNode, out _activeLegStartedIc, _activeLegWaypoints);
            if (hasLeg && _clockBaseMonotonicSec <= 0.0)
            {
                _clockBaseIcSec = _activeLegStartedIc;
                _clockRateIcPerRealSec = 1.0;
                _clockBaseMonotonicSec = MonotonicSeconds();
            }

            Vector3 curLocalPos;
            if (hasLeg && EvaluateLegPositionCm(
                    _activeLegWaypoints,
                    Math.Max(0.0, _clockBaseIcSec - _activeLegStartedIc),
                    out double xCm,
                    out double yCm,
                    out float legHeading,
                    out _,
                    out bool moving))
            {
                curLocalPos = TerrainGrid.SiteCmToLocalPos(xCm, yCm, 0f);
                _roverTargetLocalPos = curLocalPos;
                _roverHeadingDeg = legHeading;
                _hasDriveWaypoint = moving;
            }
            else
            {
                curLocalPos = TerrainGrid.SiteCellToLocalPos(siteX, siteY, 0f);
                _roverTargetLocalPos = curLocalPos;
                _hasDriveWaypoint = true;
            }
            RigOnGround = true;

            if (ActiveRoverRig?.Root != null)
            {
                if (!hasLeg || Vector3.Distance(ActiveRoverRig.Root.transform.position - SurfaceStageOrigin, curLocalPos) > TerrainGrid.CellSizeM * 4f)
                {
                    ActiveRoverRig.Root.transform.position = SurfaceStageOrigin + curLocalPos;
                }
            }
        }

        private void RebuildLiveReadingsMarkers(List<JsonNode> readings)
        {
            if (_markersRoot == null || TerrainGrid == null) return;
            for (int i = _markersRoot.childCount - 1; i >= 0; i--)
            {
                var ch = _markersRoot.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }

            if (readings == null) return;
            for (int i = 0; i < readings.Count; i++)
            {
                var r = readings[i];
                var cell = r["cell"].AsArray();
                if (cell == null || cell.Count < 2) continue;
                int siteCx = cell[0].AsInt(AuthoritativeSiteCellX);
                int siteCy = cell[1].AsInt(AuthoritativeSiteCellY);
                if (!TerrainGrid.IsSiteCellInWindow(siteCx, siteCy))
                    continue;
                string txt = r.Str("text", "Ore Reading");
                bool isRich = txt.IndexOf("rich", StringComparison.OrdinalIgnoreCase) >= 0;
                SpawnBeaconPillarAtSiteCell(siteCx, siteCy, txt, isRich ? new Color(0.2f, 1f, 0.58f) : new Color(0.32f, 0.85f, 1f));
            }
        }

        private void RebuildOtherRigsVisuals(List<JsonNode> others)
        {
            if (_otherRigsRoot == null || TerrainGrid == null) return;
            for (int i = _otherRigsRoot.childCount - 1; i >= 0; i--)
            {
                var ch = _otherRigsRoot.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }

            if (others == null) return;
            for (int i = 0; i < others.Count; i++)
            {
                var o = others[i];
                var c = o["cell"].AsArray();
                if (c == null || c.Count < 2) continue;
                int siteCx = c[0].AsInt(AuthoritativeSiteCellX);
                int siteCy = c[1].AsInt(AuthoritativeSiteCellY);
                // Omit rigs outside the current drive/site window instead of clamping them to its border!
                if (!TerrainGrid.IsSiteCellInWindow(siteCx, siteCy))
                    continue;
                Vector3 wp = SurfaceStageOrigin + TerrainGrid.SiteCellToLocalPos(siteCx, siteCy, 0f);
                string tag = o.Bool("sounding", false) ? "OTHER RIG [SOUNDING!]" : "OTHER RIG [o]";
                TacSimVehicleVisualFactory.SpawnSurfaceRover(Catalog, _otherRigsRoot, tag, wp, 90f, isEscort: true);
            }
        }

        public void TriggerScanPulseVfx(bool seismicSounding)
        {
            if (ActiveRoverRig?.Root == null || _scanPulseRing == null) return;
            _pulseScale = 1f;
            _pulseColor = seismicSounding ? new Color(1.0f, 0.72f, 0.22f, 0.65f) : new Color(0.25f, 0.92f, 1.0f, 0.58f);
            _scanPulseRing.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit(seismicSounding ? "SeismicWave" : "RadarWave", _pulseColor);
            _scanPulseRing.position = ActiveRoverRig.Root.transform.position + Vector3.up * 0.4f;
            _scanPulseRing.gameObject.SetActive(true);
        }

        public void TriggerDrillVfx(float durationSec = 2.8f)
        {
            _drillActiveUntil = Time.time + durationSec;
            if (_drillSparksGlow != null && ActiveRoverRig?.Root != null)
            {
                _drillSparksGlow.position = ActiveRoverRig.Root.transform.position + ActiveRoverRig.Root.transform.forward * 3.1f + Vector3.up * 0.35f;
                _drillSparksGlow.gameObject.SetActive(true);
            }
        }

        private void SpawnBeaconPillarAtSiteCell(int siteCx, int siteCy, string label, Color color)
        {
            Vector3 wp = SurfaceStageOrigin + TerrainGrid.SiteCellToLocalPos(siteCx, siteCy, 0f);
            var root = new GameObject($"PocketBeacon_{siteCx}_{siteCy}");
            root.transform.SetParent(_markersRoot, false);
            root.transform.position = wp;

            var beam = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            beam.transform.SetParent(root.transform, false);
            beam.transform.localPosition = new Vector3(0f, 3.5f, 0f);
            beam.transform.localScale = new Vector3(0.45f, 3.5f, 0.45f);
            var bc = beam.GetComponent<Collider>();
            if (bc != null) { if (Application.isPlaying) Destroy(bc); else DestroyImmediate(bc); }
            beam.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"Beacon_{color}", new Color(color.r, color.g, color.b, 0.62f));

            var tagGo = new GameObject("PocketLabel");
            tagGo.transform.SetParent(root.transform, false);
            tagGo.transform.localPosition = new Vector3(0f, 7.6f, 0f);
            var bb = tagGo.AddComponent<WorldBillboard>();
            var tmpGo = new GameObject("Txt");
            tmpGo.transform.SetParent(tagGo.transform, false);
            var tmp = tmpGo.AddComponent<TextMeshPro>();
            tmp.text = label.Replace("<", "‹").Replace(">", "›");
            tmp.fontSize = 4.2f;
            tmp.color = color;
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.NoWrap;
            tmp.rectTransform.sizeDelta = new Vector2(18f, 3f);
            bb.NameplateText = tmp;
        }

        private void Update()
        {
            float dt = Mathf.Max(0.0001f, Time.deltaTime);

            if (_pulseScale >= 0f && _scanPulseRing != null)
            {
                _pulseScale += dt * 48f;
                _scanPulseRing.localScale = new Vector3(_pulseScale, 0.08f, _pulseScale);
                if (_pulseScale > 95f)
                {
                    _pulseScale = -1f;
                    _scanPulseRing.gameObject.SetActive(false);
                }
            }

            bool drilling = Time.time < _drillActiveUntil;
            if (_drillSparksGlow != null && _drillSparksGlow.gameObject.activeSelf != drilling)
                _drillSparksGlow.gameObject.SetActive(drilling);

            if (!IsExcursionActive || ActiveRoverRig?.Root == null || TerrainGrid == null)
                return;

            // Route keyboard WASD / Arrow driving through authoritative `+tv/<slot> drive <x>,<y> go`
            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (!KeyboardSuspended && Time.unscaledTime >= _nextKeyboardDriveCommandAt)
            {
                int dx = 0;
                int dy = 0;
                if (Input.GetKey(KeyCode.W) || Input.GetKey(KeyCode.UpArrow)) dy = 1;
                else if (Input.GetKey(KeyCode.S) || Input.GetKey(KeyCode.DownArrow)) dy = -1;
                if (Input.GetKey(KeyCode.D) || Input.GetKey(KeyCode.RightArrow)) dx = 1;
                else if (Input.GetKey(KeyCode.A) || Input.GetKey(KeyCode.LeftArrow)) dx = -1;

                if (dx != 0 || dy != 0)
                {
                    int step = Mathf.Max(2, TerrainGrid.SpanCells * 3);
                    int nextSiteX = Mathf.Max(0, AuthoritativeSiteCellX + dx * step);
                    int nextSiteY = Mathf.Max(0, AuthoritativeSiteCellY + dy * step);
                    _nextKeyboardDriveCommandAt = Time.unscaledTime + 0.50f;
                    OnTerrainCellDriveClicked?.Invoke(nextSiteX, nextSiteY);
                }
            }

            if (!overUi && Input.GetMouseButtonDown(0))
            {
                Camera cam = Camera.main;
                if (cam != null && Vector3.Distance(cam.transform.position, ActiveRoverRig.Root.transform.position) < 340f)
                {
                    Ray ray = cam.ScreenPointToRay(Input.mousePosition);
                    if (Physics.Raycast(ray, out RaycastHit hit, 600f))
                    {
                        Vector3 hitLocal = hit.point - SurfaceStageOrigin;
                        int cx = Mathf.Clamp(Mathf.FloorToInt(hitLocal.x / TerrainGrid.CellSizeM), 0, TerrainGrid.Width - 1);
                        int cy = Mathf.Clamp(Mathf.FloorToInt(hitLocal.z / TerrainGrid.CellSizeM), 0, TerrainGrid.Height - 1);
                        int siteX = TerrainGrid.LocalToSiteCellX(cx);
                        int siteY = TerrainGrid.LocalToSiteCellY(cy);
                        OnTerrainCellDriveClicked?.Invoke(siteX, siteY);
                    }
                }
            }

            // Follow authoritative `{x_cm, y_cm, at_s}` leg waypoints on the IC clock when present,
            // or hold at the authoritative server cell otherwise.
            Vector3 localPos = ActiveRoverRig.Root.transform.position - SurfaceStageOrigin;
            StepRoverKinematics(ref localPos, dt, MonotonicSeconds());

            ActiveRoverRig.Root.transform.position = SurfaceStageOrigin + localPos;
            ActiveRoverRig.Root.transform.rotation = Quaternion.Euler(0f, _roverHeadingDeg, 0f);
            ActiveRoverRig.UpdateArticulation(dt, _roverSpeedKph, null);
        }

        public void StepRoverKinematics(ref Vector3 localPos, float dt, double nowMonotonicSec)
        {
            if (TerrainGrid == null) return;
            double nowIc = _clockBaseIcSec + Math.Max(0.0, nowMonotonicSec - _clockBaseMonotonicSec) * _clockRateIcPerRealSec;
            double elapsedIc = nowIc - _activeLegStartedIc;
            int cellX = AuthoritativeSiteCellX;
            int cellY = AuthoritativeSiteCellY;
            AdvanceRoverKinematicsStep(
                TerrainGrid,
                _activeLegWaypoints,
                elapsedIc,
                _clockRateIcPerRealSec,
                dt,
                ref localPos,
                ref _roverTargetLocalPos,
                ref _hasDriveWaypoint,
                ref _roverHeadingDeg,
                ref _roverSpeedKph,
                ref cellX,
                ref cellY);
            AuthoritativeSiteCellX = cellX;
            AuthoritativeSiteCellY = cellY;
        }

        public static void AdvanceRoverKinematicsStep(
            DecodedTerrainGrid grid,
            List<RoverLegWaypoint> activeLegWaypoints,
            double elapsedIcSec,
            double clockRateIcPerRealSec,
            float dt,
            ref Vector3 localPos,
            ref Vector3 roverTargetLocalPos,
            ref bool hasDriveWaypoint,
            ref float roverHeadingDeg,
            ref float roverSpeedKph,
            ref int authoritativeSiteCellX,
            ref int authoritativeSiteCellY)
        {
            if (grid == null) return;

            if (activeLegWaypoints != null && activeLegWaypoints.Count > 0)
            {
                if (EvaluateLegPositionCm(
                        activeLegWaypoints,
                        elapsedIcSec,
                        out double xCm,
                        out double yCm,
                        out float legHeading,
                        out double segMps,
                        out bool moving))
                {
                    authoritativeSiteCellX = (int)Math.Floor(xCm / 1000.0);
                    authoritativeSiteCellY = (int)Math.Floor(yCm / 1000.0);
                    localPos = grid.SiteCmToLocalPos(xCm, yCm, 0f);
                    roverHeadingDeg = Mathf.MoveTowardsAngle(roverHeadingDeg, legHeading, 180f * dt);
                    roverSpeedKph = moving ? (float)(segMps * Math.Max(0.25, clockRateIcPerRealSec) * 3.6) : 0f;
                    if (!moving)
                    {
                        // Anchor fallback destination at the finished endpoint and clear hasDriveWaypoint
                        // so subsequent frames hold position instead of reversing toward the leg's origin!
                        roverTargetLocalPos = localPos;
                        hasDriveWaypoint = false;
                        activeLegWaypoints.Clear();
                    }
                }
            }
            else if (hasDriveWaypoint)
            {
                Vector3 toTarget = roverTargetLocalPos - localPos;
                toTarget.y = 0f;
                if (toTarget.magnitude > 0.35f)
                {
                    float desiredYaw = Mathf.Atan2(toTarget.x, toTarget.z) * Mathf.Rad2Deg;
                    roverHeadingDeg = Mathf.MoveTowardsAngle(roverHeadingDeg, desiredYaw, 150f * dt);
                    Vector3 step = toTarget.normalized * Mathf.Min(toTarget.magnitude, 14f * dt);
                    localPos += step;
                    roverSpeedKph = 24f;
                }
                else
                {
                    localPos = roverTargetLocalPos;
                    hasDriveWaypoint = false;
                    roverSpeedKph = 0f;
                }
            }
            else
            {
                roverSpeedKph = Mathf.MoveTowards(roverSpeedKph, 0f, 40f * dt);
            }

            float maxCoordX = Mathf.Max(grid.CellSizeM, grid.Width * grid.CellSizeM);
            float maxCoordZ = Mathf.Max(grid.CellSizeM, grid.Height * grid.CellSizeM);
            localPos.x = Mathf.Clamp(localPos.x, 0f, maxCoordX);
            localPos.z = Mathf.Clamp(localPos.z, 0f, maxCoordZ);
            localPos.y = grid.SampleElevationMeters(localPos.x, localPos.z);
        }
    }
}
