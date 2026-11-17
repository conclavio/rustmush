using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.World;
using TMPro;
using UnityEngine;
using UnityEngine.EventSystems;

namespace TacSim.Surface
{
    /// <summary>
    /// Pure-online 3D Planetary Surface Heightfield, Landed Dray-P Pad, and
    /// Articulated Prospector Rover (`+tv`) driven by authoritative `mushd` `site.map` and `tv.status` feeds.
    /// </summary>
    public sealed class SurfaceMiningController : MonoBehaviour
    {
        public static readonly Vector3 SurfaceStageOrigin = new Vector3(1200f, 0f, 0f);

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
        public int HopperLoadedTonnes { get; private set; }
        public int HopperCapacityTonnes { get; private set; } = 6;
        public int BatteryPct { get; private set; } = 100;
        public int HoldUnreservedTonnes { get; private set; } = 86;
        public string StatusSummaryLine { get; private set; } = "Rig 1 cradled in Dray-P Vehicle Bay.";

        public event Action<int, int> OnTerrainCellDriveClicked;
        public event Action OnSurfaceTelemetryChanged;
        public event Action<string> OnSurfaceLogEvent;

        private Transform _terrainRoot;
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

        private readonly List<string> _recentSiteLines = new List<string>();
        public IReadOnlyList<string> RecentSiteLines => _recentSiteLines;

        public void BuildSurfaceStage(TacSimAssetCatalog catalog, string siteSlug = "occator_rim")
        {
            Catalog = catalog;
            ActiveSiteSlug = string.IsNullOrWhiteSpace(siteSlug) ? "occator_rim" : siteSlug.Trim().ToLowerInvariant();

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }

            transform.position = SurfaceStageOrigin;

            TerrainGrid = TacSimTerrainDecoder.BuildPlanetarySiteGrid(ActiveSiteSlug, side: 64);
            ActiveSiteTitle = TerrainGrid.DisplayName;

            _terrainRoot = new GameObject("PlanetaryTerrainMesh").transform;
            _terrainRoot.SetParent(transform, false);
            TacSimTerrainDecoder.BuildChunkedTerrainMeshes(TerrainGrid, _terrainRoot, chunkCells: 32);

            _markersRoot = new GameObject("OrePocketBeacons").transform;
            _markersRoot.SetParent(transform, false);

            _otherRigsRoot = new GameObject("OtherSurfaceRigs").transform;
            _otherRigsRoot.SetParent(transform, false);

            BuildLandedDrayCarrierOnPad();
            SpawnPlayerProspectorRover();
            BuildScanAndDrillVfx();
        }

        private void BuildLandedDrayCarrierOnPad()
        {
            int padCx = TerrainGrid.Width / 2;
            int padCy = TerrainGrid.Height / 2;
            Vector3 padLocal = TerrainGrid.CellToLocalPos(padCx, padCy, 0f);
            Vector3 padWorld = SurfaceStageOrigin + padLocal;

            var padRoot = new GameObject("LandedCarrierPad").transform;
            padRoot.SetParent(transform, false);
            padRoot.position = padWorld;

            Material apronMat = UrpMaterialFactory.GetOpaque("PadApron", new Color(0.24f, 0.27f, 0.31f), 0.38f, 0.55f);
            Material rampMat = UrpMaterialFactory.GetEmissive("SternRamp", new Color(0.45f, 0.36f, 0.14f), new Color(0.85f, 0.58f, 0.12f) * 0.65f);

            var apron = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            apron.name = "LandingApron";
            apron.transform.SetParent(padRoot, false);
            apron.transform.localPosition = new Vector3(0f, 0.08f, 0f);
            apron.transform.localScale = new Vector3(26f, 0.12f, 26f);
            var ac = apron.GetComponent<Collider>();
            if (ac != null) { if (Application.isPlaying) Destroy(ac); else DestroyImmediate(ac); }
            apron.GetComponent<MeshRenderer>().sharedMaterial = apronMat;

            if (Catalog != null && Catalog.drayExteriorHullPrefab != null)
            {
                PrefabFittingHelper.SpawnGrounded(Catalog.drayExteriorHullPrefab, padRoot, "SettledDrayPHull", padWorld + Vector3.up * 0.2f, Quaternion.identity, 8.5f, 26f, keepColliders: true);
            }
            else
            {
                Material hMat = UrpMaterialFactory.GetOpaque("PadDrayHull", new Color(0.48f, 0.45f, 0.40f), 0.4f, 0.55f);
                Material tMat = UrpMaterialFactory.GetOpaque("PadDrayTrim", new Color(0.22f, 0.26f, 0.32f), 0.45f, 0.7f);
                StationWorldController.BuildSculptedDrayExterior(padRoot, padWorld + Vector3.up * 3.2f, hMat, tMat, "SettledDrayPHull");
            }

            var ramp = GameObject.CreatePrimitive(PrimitiveType.Cube);
            ramp.name = "LoweredSternRamp";
            ramp.transform.SetParent(padRoot, false);
            ramp.transform.localPosition = new Vector3(0f, 0.65f, -12.8f);
            ramp.transform.localRotation = Quaternion.Euler(14f, 0f, 0f);
            ramp.transform.localScale = new Vector3(4.6f, 0.22f, 5.4f);
            ramp.GetComponent<MeshRenderer>().sharedMaterial = rampMat;

            var floodGo = new GameObject("PadFloodLight");
            floodGo.transform.SetParent(padRoot, false);
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
            int cx = TerrainGrid.Width / 2;
            int cy = TerrainGrid.Height / 2 - 2;
            _roverTargetLocalPos = TerrainGrid.CellToLocalPos(cx, cy, 0f);
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

        public void ApplyTvStatusFrame(JsonNode frame)
        {
            if (frame == null || !frame.IsObject) return;
            var rig = frame["rig"];
            if (rig.IsObject)
            {
                BatteryPct = Mathf.Clamp(rig.Int("charge_pct", BatteryPct), 0, 100);
                HopperLoadedTonnes = Mathf.Max(0, rig.Int("load_t", HopperLoadedTonnes));
                HopperCapacityTonnes = Mathf.Max(1, rig.Int("cap_t", HopperCapacityTonnes));
                OnSurfaceTelemetryChanged?.Invoke();
            }
        }

        public void ApplySiteMapFrame(JsonNode frame)
        {
            if (frame == null || !frame.IsObject) return;

            var siteBlock = frame["site"];
            if (siteBlock.IsObject)
            {
                ActiveSiteTitle = siteBlock.Str("site", ActiveSiteTitle);
                ActiveOreFamily = siteBlock.Str("family_word", siteBlock.Str("family", ActiveOreFamily));
                AddressedRigSlot = Mathf.Max(1, siteBlock.Int("addressed", AddressedRigSlot));
                var holdObj = siteBlock["hold"];
                if (holdObj.IsObject)
                    HoldUnreservedTonnes = holdObj.Int("room_t", HoldUnreservedTonnes);

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

            var youArr = frame["you"].AsArray();
            var origArr = frame["origin_cell"].AsArray();
            if (youArr != null && youArr.Count >= 2 && TerrainGrid != null)
            {
                int ox = (origArr != null && origArr.Count >= 2) ? origArr[0].AsInt(0) : 0;
                int oy = (origArr != null && origArr.Count >= 2) ? origArr[1].AsInt(0) : 0;
                int siteX = ox + youArr[0].AsInt(0);
                int siteY = oy + youArr[1].AsInt(0);

                int localCx = Mathf.Clamp(siteX - 128 + TerrainGrid.Width / 2, 2, TerrainGrid.Width - 3);
                int localCy = Mathf.Clamp(siteY - 128 + TerrainGrid.Height / 2, 2, TerrainGrid.Height - 3);
                _roverTargetLocalPos = TerrainGrid.CellToLocalPos(localCx, localCy, 0f);
                _hasDriveWaypoint = true;
                RigOnGround = true;
            }

            OnSurfaceTelemetryChanged?.Invoke();
        }

        private void RebuildLiveReadingsMarkers(List<JsonNode> readings)
        {
            if (_markersRoot == null || TerrainGrid == null) return;
            for (int i = _markersRoot.childCount - 1; i >= 0; i--)
                Destroy(_markersRoot.GetChild(i).gameObject);

            if (readings == null) return;
            for (int i = 0; i < readings.Count; i++)
            {
                var r = readings[i];
                var cell = r["cell"].AsArray();
                if (cell == null || cell.Count < 2) continue;
                int localCx = Mathf.Clamp(cell[0].AsInt(128) - 128 + TerrainGrid.Width / 2, 2, TerrainGrid.Width - 3);
                int localCy = Mathf.Clamp(cell[1].AsInt(128) - 128 + TerrainGrid.Height / 2, 2, TerrainGrid.Height - 3);
                string txt = r.Str("text", "Ore Reading");
                bool isRich = txt.IndexOf("rich", StringComparison.OrdinalIgnoreCase) >= 0;
                SpawnBeaconPillar(localCx, localCy, txt, isRich ? new Color(0.2f, 1f, 0.58f) : new Color(0.32f, 0.85f, 1f));
            }
        }

        private void RebuildOtherRigsVisuals(List<JsonNode> others)
        {
            if (_otherRigsRoot == null || TerrainGrid == null) return;
            for (int i = _otherRigsRoot.childCount - 1; i >= 0; i--)
                Destroy(_otherRigsRoot.GetChild(i).gameObject);

            if (others == null) return;
            for (int i = 0; i < others.Count; i++)
            {
                var o = others[i];
                var c = o["cell"].AsArray();
                if (c == null || c.Count < 2) continue;
                int localCx = Mathf.Clamp(c[0].AsInt(128) - 128 + TerrainGrid.Width / 2, 2, TerrainGrid.Width - 3);
                int localCy = Mathf.Clamp(c[1].AsInt(128) - 128 + TerrainGrid.Height / 2, 2, TerrainGrid.Height - 3);
                Vector3 wp = SurfaceStageOrigin + TerrainGrid.CellToLocalPos(localCx, localCy, 0f);
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

        private void SpawnBeaconPillar(int cx, int cy, string label, Color color)
        {
            Vector3 wp = SurfaceStageOrigin + TerrainGrid.CellToLocalPos(cx, cy, 0f);
            var root = new GameObject($"PocketBeacon_{cx}_{cy}");
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
            tmp.text = label;
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

            float throttle = 0f;
            float steer = 0f;
            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (!KeyboardSuspended)
            {
                if (Input.GetKey(KeyCode.W) || Input.GetKey(KeyCode.UpArrow)) throttle = 1f;
                else if (Input.GetKey(KeyCode.S) || Input.GetKey(KeyCode.DownArrow)) throttle = -0.65f;
                if (Input.GetKey(KeyCode.D) || Input.GetKey(KeyCode.RightArrow)) steer = 1f;
                else if (Input.GetKey(KeyCode.A) || Input.GetKey(KeyCode.LeftArrow)) steer = -1f;
            }
            if (!overUi && Input.GetMouseButtonDown(0))
            {
                Camera cam = Camera.main;
                if (cam != null && Vector3.Distance(cam.transform.position, ActiveRoverRig.Root.transform.position) < 260f)
                {
                    Ray ray = cam.ScreenPointToRay(Input.mousePosition);
                    if (Physics.Raycast(ray, out RaycastHit hit, 500f))
                    {
                        Vector3 hitLocal = hit.point - SurfaceStageOrigin;
                        int cx = Mathf.Clamp(Mathf.FloorToInt(hitLocal.x / TerrainGrid.CellSizeM), 2, TerrainGrid.Width - 3);
                        int cy = Mathf.Clamp(Mathf.FloorToInt(hitLocal.z / TerrainGrid.CellSizeM), 2, TerrainGrid.Height - 3);
                        _roverTargetLocalPos = TerrainGrid.CellToLocalPos(cx, cy, 0f);
                        _hasDriveWaypoint = true;

                        int siteX = cx - TerrainGrid.Width / 2 + 128;
                        int siteY = cy - TerrainGrid.Height / 2 + 128;
                        OnTerrainCellDriveClicked?.Invoke(siteX, siteY);
                    }
                }
            }

            Vector3 localPos = ActiveRoverRig.Root.transform.position - SurfaceStageOrigin;
            float maxMetersPerSec = 14f;
            if (Mathf.Abs(throttle) > 0.01f || Mathf.Abs(steer) > 0.01f)
            {
                _hasDriveWaypoint = false;
                _roverHeadingDeg = (_roverHeadingDeg + steer * 68f * dt + 360f) % 360f;
                Vector3 fwd = Quaternion.Euler(0f, _roverHeadingDeg, 0f) * Vector3.forward;
                localPos += fwd * (throttle * maxMetersPerSec * dt);
                _roverSpeedKph = throttle * 42f;
            }
            else if (_hasDriveWaypoint)
            {
                Vector3 toTarget = _roverTargetLocalPos - localPos;
                toTarget.y = 0f;
                if (toTarget.magnitude > 0.75f)
                {
                    float desiredYaw = Mathf.Atan2(toTarget.x, toTarget.z) * Mathf.Rad2Deg;
                    _roverHeadingDeg = Mathf.MoveTowardsAngle(_roverHeadingDeg, desiredYaw, 110f * dt);
                    Vector3 step = toTarget.normalized * Mathf.Min(toTarget.magnitude, maxMetersPerSec * dt);
                    localPos += step;
                    _roverSpeedKph = 38f;
                }
                else
                {
                    _hasDriveWaypoint = false;
                    _roverSpeedKph = 0f;
                }
            }
            else
            {
                _roverSpeedKph = Mathf.MoveTowards(_roverSpeedKph, 0f, 40f * dt);
            }

            float maxCoord = (TerrainGrid.Width - 2) * TerrainGrid.CellSizeM;
            localPos.x = Mathf.Clamp(localPos.x, 12f, maxCoord);
            localPos.z = Mathf.Clamp(localPos.z, 12f, maxCoord);
            localPos.y = TerrainGrid.SampleElevationMeters(localPos.x, localPos.z);

            ActiveRoverRig.Root.transform.position = SurfaceStageOrigin + localPos;
            ActiveRoverRig.Root.transform.rotation = Quaternion.Euler(0f, _roverHeadingDeg, 0f);
            ActiveRoverRig.UpdateArticulation(dt, _roverSpeedKph, null);
        }
    }
}
