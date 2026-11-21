using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.Net;
using TacSim.World;
using TMPro;
using UnityEngine;
using UnityEngine.EventSystems;

namespace TacSim.Space
{
    /// <summary>
    /// Authoritative Doc 49 (`CC.9`) per-mount telemetry row from `space.cockpit.fire.mounts[]`.
    /// </summary>
    public sealed class SpaceMountTelemetry
    {
        public string Id = string.Empty;
        public string Family = string.Empty; // "laser" | "tubes" | "spinal" | "point_defence"
        public string State = "ready";       // "down" | "empty" | "cycling" | "too_hot" | "past_reach" | "ready"
        public int ReadyInMs;
        public string Mode = string.Empty;   // normalized US: "offense" | "defense" | ""
        public int Rounds = -1;              // -1 when mount has no magazine (e.g. laser)
        public int RoundsMax = -1;           // -1 when drum max is omitted (e.g. rotary PDC)
        public int DamageAtLock = -1;        // deterministic laser pulse damage at held lock range
        public int FlightMsAtLock = -1;      // deterministic spinal slug flight time (ms) at held lock range
    }

    /// <summary>
    /// Authoritative Doc 49 (`CC.8` / `CC.9`) held target lock &amp; 5-band hull aspect from `space.cockpit.fire.lock`.
    /// </summary>
    public sealed class SpaceLockTelemetry
    {
        public string ShipId = string.Empty;
        public string Label = string.Empty;
        public long RangeM;
        public string Facet = "front";       // "front" | "left" | "right" | "rear" | "top"
    }

    /// <summary>
    /// Renders authoritative 3D Orbital Space Flight (player ship hull + fusion plume, rotating body,
    /// server-perceived Zero-G Ore Asteroids &amp; Naval Contacts from `space.holotank` / `space.cockpit`,
    /// 3D Mining Laser &amp; Collector Drones) AND drives the 60 Hz 6-DOF Bow Docking Collar Mini-Game
    /// with bounded RLE submission and authoritative `space.docking` outcome reconciliation.
    /// </summary>
    public sealed class SpaceFlightController : MonoBehaviour
    {
        public static readonly Vector3 OrbitalStageOrigin = new Vector3(0f, 620f, 0f);
        public static readonly Vector3 DockingStageOrigin = new Vector3(450f, 620f, 0f);

        public TacSimAssetCatalog Catalog;

        public Transform PlayerShipTransform { get; private set; }
        public Transform BowDockingCameraAnchor { get; private set; }

        public bool IsDockingActive => ActiveDockingSession != null;
        public DockingClientSession ActiveDockingSession { get; private set; }
        public bool GuidanceAssistEnabled { get; set; }
        public bool KeyboardSuspended { get; set; }
        public int LiveTorpedoCount => _liveTorpedoMarkers.Count;
        public bool HasPendingDockingAbort => _pendingAbort;
        public bool HasPendingDampenerToggle => _pendingDampenerToggle;

        public float ShipYawDeg { get; private set; }
        public float ShipPitchDeg { get; private set; }
        public float ShipRollDeg { get; private set; }
        public float ThrottlePermille { get; private set; }
        public string EmconPosture { get; private set; } = "standard";
        public string ActiveSiteId { get; private set; } = string.Empty;
        public string SelectedRockId { get; private set; } = string.Empty;
        public string LockedContactId { get; private set; } = string.Empty;
        public IReadOnlyList<SpaceMountTelemetry> FireMounts => _fireMounts;
        public SpaceLockTelemetry ActiveLock { get; private set; }

        /// <summary>
        /// Fired when the local 60 Hz integrator reaches a terminal contact (`(tapeCommand, isClampCandidate)`).
        /// Final success/failure is decided only when the server replies with `space.docking` (`closed: true, outcome`).
        /// </summary>
        public event Action<string, bool> OnDockingTapeReadyToSubmit;
        public event Action OnDockingAborted;
        public event Action<string> OnDockingEndedByServer;
        public event Action<string> OnSpaceAlert;
        public event Action<string, bool> OnOrbitalTargetClicked; // (designator, isAsteroid)
        public event Action OnSpaceTelemetryChanged;

        private Transform _ceresBody;
        private Transform _mainDrivePlume;
        private Transform _particlePlumeInstance;
        private Transform _contactsContainer;
        private Transform _dockingCollarTarget;
        private readonly List<MeshRenderer> _gateRingRenderers = new List<MeshRenderer>();
        private readonly Dictionary<string, Transform> _namedOrbitalTargets = new Dictionary<string, Transform>(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<string, TextMeshPro> _targetLabelTexts = new Dictionary<string, TextMeshPro>(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<string, string> _contactStyleKeys = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<string, Transform> _liveTorpedoMarkers = new Dictionary<string, Transform>(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<string, TextMeshPro> _torpedoLabelTexts = new Dictionary<string, TextMeshPro>(StringComparer.OrdinalIgnoreCase);
        private readonly HashSet<string> _knownRockIds = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        private readonly List<Transform> _collectorDrones = new List<Transform>();
        private readonly List<SpaceMountTelemetry> _fireMounts = new List<SpaceMountTelemetry>(8);
        private float _lastHolotankBurnPermille;
        private bool _hasLastCockpitSpeed;
        private float _lastCockpitSpeedMps;

        private Transform _lockAspectIndicator;
        private Transform _lockAspectSectorPlate;
        private TextMeshPro _lockAspectLabel;

        private LineRenderer _activeBeamRenderer;
        private Transform _activeBeamEndGlow;
        private Transform _activeTorpedoVisual;
        private Vector3 _torpStart;
        private Vector3 _torpEnd;
        private float _torpProgress = -1f;
        private float _beamExpireTime;
        private bool _droneSortieActive;

        private float _dockingAccumulatorS;
        private bool _pendingDampenerToggle;
        private bool _pendingAbort;
        private bool _tapeSubmittedAwaitingServer;
        private float _tapeSubmittedAtUnscaledTime;
        private const float DockingSubmitTimeoutSec = 10.0f;

        public bool HasTargetableRock(string rockId) => !string.IsNullOrEmpty(rockId) && _knownRockIds.Contains(rockId.Trim().ToUpperInvariant());
        public bool HasPerceivedContact(string designator)
        {
            if (string.IsNullOrEmpty(designator)) return false;
            return ResolveContactKey(designator, out string key) && !_knownRockIds.Contains(key);
        }

        public bool IsDroneSortieActive => _droneSortieActive;
        public string OwnShipHolotankId { get; private set; } = string.Empty;
        public bool HasClusterMiningTelemetry { get; private set; }
        public string AuthoritativeMatchedRockId { get; private set; } = string.Empty;
        public string AuthoritativeApproachingRockId { get; private set; } = string.Empty;
        public bool IsAuthoritativeMiningLaserActive { get; private set; }

        public bool IsVelocityMatchedToRock(string rockId) =>
            !string.IsNullOrWhiteSpace(rockId) &&
            string.Equals(AuthoritativeMatchedRockId, rockId.Trim(), StringComparison.OrdinalIgnoreCase);

        public bool TryGetTargetPosition(string id, out Vector3 worldPos)
        {
            if (ResolveContactKey(id, out string key) && _namedOrbitalTargets.TryGetValue(key, out var tr) && tr != null)
            {
                worldPos = tr.position;
                return true;
            }
            worldPos = Vector3.zero;
            return false;
        }

        private bool ResolveContactKey(string id, out string resolvedKey)
        {
            resolvedKey = null;
            if (string.IsNullOrWhiteSpace(id)) return false;
            string trimmed = id.Trim();
            if (_namedOrbitalTargets.ContainsKey(trimmed))
            {
                resolvedKey = trimmed;
                return true;
            }
            foreach (var k in _namedOrbitalTargets.Keys)
            {
                if (string.Equals(k, trimmed, StringComparison.OrdinalIgnoreCase))
                {
                    resolvedKey = k;
                    return true;
                }
            }
            return false;
        }

        public void BuildSpaceStage(TacSimAssetCatalog catalog)
        {
            Catalog = catalog;
            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _gateRingRenderers.Clear();
            _namedOrbitalTargets.Clear();
            _targetLabelTexts.Clear();
            _contactStyleKeys.Clear();
            _liveTorpedoMarkers.Clear();
            _torpedoLabelTexts.Clear();
            _knownRockIds.Clear();
            _collectorDrones.Clear();
            _fireMounts.Clear();
            ActiveLock = null;
            SelectedRockId = string.Empty;
            LockedContactId = string.Empty;
            OwnShipHolotankId = string.Empty;
            HasClusterMiningTelemetry = false;
            AuthoritativeMatchedRockId = string.Empty;
            AuthoritativeApproachingRockId = string.Empty;
            IsAuthoritativeMiningLaserActive = false;
            _droneSortieActive = false;
            _lastHolotankBurnPermille = 0f;
            _hasLastCockpitSpeed = false;
            _lastCockpitSpeedMps = 0f;

            transform.position = OrbitalStageOrigin;

            // 1. Player's Orbital Ship Hull + Main Fusion Plume
            Material hullMat = UrpMaterialFactory.GetOpaque("SpaceDrayHull", new Color(0.50f, 0.48f, 0.44f), 0.42f, 0.6f);
            Material trimMat = UrpMaterialFactory.GetOpaque("SpaceDrayTrim", new Color(0.24f, 0.28f, 0.34f), 0.5f, 0.7f);

            if (Catalog != null && Catalog.drayExteriorHullPrefab != null)
            {
                var inst = PrefabFittingHelper.SpawnCentered(Catalog.drayExteriorHullPrefab, transform, "PlayerOrbitalShip", OrbitalStageOrigin, Quaternion.identity, 26f, keepColliders: false);
                PlayerShipTransform = inst != null ? inst.transform : StationWorldController.BuildSculptedDrayExterior(transform, OrbitalStageOrigin, hullMat, trimMat, "PlayerOrbitalShip").transform;
            }
            else
            {
                PlayerShipTransform = StationWorldController.BuildSculptedDrayExterior(transform, OrbitalStageOrigin, hullMat, trimMat, "PlayerOrbitalShip").transform;
            }

            var plumeGo = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            plumeGo.name = "FusionDrivePlume";
            plumeGo.transform.SetParent(PlayerShipTransform, false);
            plumeGo.transform.position = OrbitalStageOrigin + new Vector3(0f, 0f, -13.5f);
            plumeGo.transform.rotation = Quaternion.Euler(90f, 0f, 0f);
            plumeGo.transform.localScale = new Vector3(2.0f, 4.2f, 2.0f);
            var pCol = plumeGo.GetComponent<Collider>();
            if (pCol != null) { if (Application.isPlaying) Destroy(pCol); else DestroyImmediate(pCol); }
            plumeGo.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("DrivePlume", new Color(0.35f, 0.78f, 1.0f, 0.75f));
            _mainDrivePlume = plumeGo.transform;
            _mainDrivePlume.gameObject.SetActive(false);

            if (Catalog != null && Catalog.thrusterPlumePrefab != null)
            {
                var fx = Instantiate(Catalog.thrusterPlumePrefab, OrbitalStageOrigin + new Vector3(0f, 0f, -13.2f), Quaternion.Euler(0f, 180f, 0f), PlayerShipTransform);
                fx.name = "FusionThrusterParticles";
                _particlePlumeInstance = fx.transform;
                _particlePlumeInstance.gameObject.SetActive(false);
            }

            // 2. Distant Planet Backdrop & Sunlight
            var sunGo = new GameObject("SolarLight");
            sunGo.transform.SetParent(transform, false);
            sunGo.transform.position = OrbitalStageOrigin + new Vector3(200f, 120f, -180f);
            sunGo.transform.rotation = Quaternion.Euler(22f, -42f, 0f);

            if (Catalog != null && Catalog.ceresPlanetPrefab != null)
            {
                var p = PrefabFittingHelper.SpawnCentered(
                    Catalog.ceresPlanetPrefab,
                    transform,
                    "CeresGlobe",
                    OrbitalStageOrigin + new Vector3(160f, -55f, 340f),
                    Quaternion.identity,
                    maxDimensionM: 150f,
                    keepColliders: false);
                if (p != null)
                    _ceresBody = p.transform;
            }
            else
            {
                var ceres = GameObject.CreatePrimitive(PrimitiveType.Sphere);
                ceres.name = "CeresGlobe";
                ceres.transform.SetParent(transform, true);
                ceres.transform.position = OrbitalStageOrigin + new Vector3(160f, -55f, 340f);
                ceres.transform.localScale = Vector3.one * 150f;
                var cc = ceres.GetComponent<Collider>();
                if (cc != null) { if (Application.isPlaying) Destroy(cc); else DestroyImmediate(cc); }
                ceres.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetOpaque("CeresRegolith", new Color(0.36f, 0.35f, 0.33f), 0.12f, 0.05f);
                _ceresBody = ceres.transform;
            }

            // Authoritative contacts & cluster rocks container (starts empty until server telemetry arrives!)
            _contactsContainer = new GameObject("OrbitalContacts").transform;
            _contactsContainer.SetParent(transform, false);
            BuildAmbientSceneryAndVfxPool();

            // 3. Dedicated 60 Hz 6-DOF Bow Docking Collar Stage
            BuildBowDockingRig();
        }

        private void BuildAmbientSceneryAndVfxPool()
        {
            Material rockMat = UrpMaterialFactory.GetOpaque("AsteroidOre", new Color(0.32f, 0.28f, 0.24f), 0.15f, 0.2f);
            Material veinMat = UrpMaterialFactory.GetEmissive("OreVein", new Color(0.22f, 0.72f, 0.85f), new Color(0.25f, 0.95f, 1.0f) * 1.4f);

            // Collector drone visual pool (hidden until server reports active drones in flight)
            for (int d = 0; d < 2; d++)
            {
                Vector3 dPos = OrbitalStageOrigin + new Vector3(d == 0 ? -7.5f : 7.5f, 2.2f, 2f);
                GameObject droneGo;
                if (Catalog != null && Catalog.collectorDronePrefab != null)
                {
                    droneGo = PrefabFittingHelper.SpawnCentered(Catalog.collectorDronePrefab, transform, $"MiningDrone_{d + 1}", dPos, Quaternion.identity, 3.6f, keepColliders: false);
                }
                else
                {
                    droneGo = CreateDockBox(transform, $"MiningDrone_{d + 1}", Vector3.zero, new Vector3(1.8f, 1.1f, 2.4f), veinMat);
                    droneGo.transform.position = dPos;
                }
                if (droneGo != null)
                {
                    droneGo.SetActive(false);
                    _collectorDrones.Add(droneGo.transform);
                }
            }

            // Non-interactive distant belt scenery ring (no colliders, no target IDs)
            for (int i = 0; i < 8; i++)
            {
                float ang = i * 0.785f;
                float dist = 165f + (i % 3) * 42f;
                Vector3 pos = OrbitalStageOrigin + new Vector3(Mathf.Cos(ang) * dist, ((i % 5) - 2) * 16f, Mathf.Sin(ang) * dist);
                GameObject rock;
                if (Catalog != null && Catalog.asteroidPrefabs != null && Catalog.asteroidPrefabs.Count > 0 && Catalog.asteroidPrefabs[i % Catalog.asteroidPrefabs.Count] != null)
                {
                    rock = Instantiate(Catalog.asteroidPrefabs[i % Catalog.asteroidPrefabs.Count], pos, Quaternion.Euler(i * 23f, i * 47f, 0f), transform);
                }
                else
                {
                    rock = GameObject.CreatePrimitive(PrimitiveType.Sphere);
                    rock.transform.SetParent(transform, true);
                    rock.transform.position = pos;
                    rock.transform.localScale = new Vector3(6f + (i % 4) * 2f, 4.5f, 5.5f);
                    rock.GetComponent<MeshRenderer>().sharedMaterial = rockMat;
                }
                var rc = rock.GetComponent<Collider>();
                if (rc != null) { if (Application.isPlaying) Destroy(rc); else DestroyImmediate(rc); }
                rock.name = $"DistantSceneryRock_{i}";
            }

            var beamGo = new GameObject("OrbitalBeamVfx");
            beamGo.transform.SetParent(transform, false);
            _activeBeamRenderer = beamGo.AddComponent<LineRenderer>();
            _activeBeamRenderer.positionCount = 2;
            _activeBeamRenderer.startWidth = 0.55f;
            _activeBeamRenderer.endWidth = 0.32f;
            _activeBeamRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("MiningBeamMat", new Color(0.25f, 0.95f, 1.0f, 0.92f));
            _activeBeamRenderer.enabled = false;

            var glowGo = GameObject.CreatePrimitive(PrimitiveType.Sphere);
            glowGo.name = "BeamImpactSparks";
            glowGo.transform.SetParent(transform, false);
            glowGo.transform.localScale = Vector3.one * 2.8f;
            var gc = glowGo.GetComponent<Collider>();
            if (gc != null) { if (Application.isPlaying) Destroy(gc); else DestroyImmediate(gc); }
            glowGo.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetEmissive("SlagGlow", new Color(1f, 0.65f, 0.2f), new Color(1f, 0.72f, 0.25f) * 2.5f);
            _activeBeamEndGlow = glowGo.transform;
            _activeBeamEndGlow.gameObject.SetActive(false);

            var torpGo = GameObject.CreatePrimitive(PrimitiveType.Capsule);
            torpGo.name = "TorpedoStreak";
            torpGo.transform.SetParent(transform, false);
            torpGo.transform.localScale = new Vector3(0.65f, 1.8f, 0.65f);
            var tc = torpGo.GetComponent<Collider>();
            if (tc != null) { if (Application.isPlaying) Destroy(tc); else DestroyImmediate(tc); }
            torpGo.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetEmissive("TorpFlare", new Color(1f, 0.45f, 0.15f), new Color(1f, 0.55f, 0.2f) * 2.8f);
            _activeTorpedoVisual = torpGo.transform;
            _activeTorpedoVisual.gameObject.SetActive(false);

            BuildLockAspectIndicator();
        }

        private void BuildLockAspectIndicator()
        {
            var root = new GameObject("LockedAspectIndicator");
            root.transform.SetParent(transform, false);
            _lockAspectIndicator = root.transform;

            var ring = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            ring.name = "AspectBaseRing";
            ring.transform.SetParent(_lockAspectIndicator, false);
            ring.transform.localScale = new Vector3(26f, 0.12f, 26f);
            var rc = ring.GetComponent<Collider>();
            if (rc != null) { if (Application.isPlaying) Destroy(rc); else DestroyImmediate(rc); }
            ring.GetComponent<MeshRenderer>().sharedMaterial =
                UrpMaterialFactory.GetHologramUnlit("LockAspectRing", new Color(1.0f, 0.62f, 0.18f, 0.24f));

            var sector = GameObject.CreatePrimitive(PrimitiveType.Cube);
            sector.name = "StrikeBandArc";
            sector.transform.SetParent(_lockAspectIndicator, false);
            sector.transform.localPosition = new Vector3(0f, 0.35f, 12.5f);
            sector.transform.localScale = new Vector3(9.5f, 1.4f, 1.6f);
            var sc = sector.GetComponent<Collider>();
            if (sc != null) { if (Application.isPlaying) Destroy(sc); else DestroyImmediate(sc); }
            sector.GetComponent<MeshRenderer>().sharedMaterial =
                UrpMaterialFactory.GetEmissive("StrikeBandGlow", new Color(1.0f, 0.36f, 0.22f), new Color(1.0f, 0.45f, 0.25f) * 2.4f);
            _lockAspectSectorPlate = sector.transform;

            var bbGo = new GameObject("AspectCallout");
            bbGo.transform.SetParent(_lockAspectIndicator, false);
            bbGo.transform.localPosition = new Vector3(0f, 12.8f, 0f);
            var bb = bbGo.AddComponent<WorldBillboard>();
            var txtGo = new GameObject("Label");
            txtGo.transform.SetParent(bbGo.transform, false);
            _lockAspectLabel = txtGo.AddComponent<TextMeshPro>();
            _lockAspectLabel.richText = false;
            _lockAspectLabel.fontSize = 14f;
            _lockAspectLabel.color = new Color(1.0f, 0.78f, 0.36f);
            _lockAspectLabel.alignment = TextAlignmentOptions.Center;
            _lockAspectLabel.textWrappingMode = TextWrappingModes.NoWrap;
            _lockAspectLabel.rectTransform.sizeDelta = new Vector2(60f, 6f);
            bb.NameplateText = _lockAspectLabel;

            _lockAspectIndicator.gameObject.SetActive(false);
        }

        /// <summary>
        /// Clears all session-scoped orbital contacts, locks, weapons VFX, and active docking state on disconnect/re-login.
        /// </summary>
        public void ResetSessionState()
        {
            StopDockingSession();
            StopMiningLaser();
            ToggleDroneSortie(false);
            _pendingDampenerToggle = false;
            _pendingAbort = false;
            GuidanceAssistEnabled = false;
            KeyboardSuspended = false;
            ShipYawDeg = 0f;
            ShipPitchDeg = 0f;
            ShipRollDeg = 0f;
            ThrottlePermille = 0f;
            EmconPosture = "standard";
            ActiveSiteId = string.Empty;
            SelectedRockId = string.Empty;
            LockedContactId = string.Empty;
            OwnShipHolotankId = string.Empty;
            ActiveLock = null;
            _fireMounts.Clear();
            HasClusterMiningTelemetry = false;
            AuthoritativeMatchedRockId = string.Empty;
            AuthoritativeApproachingRockId = string.Empty;
            _lastHolotankBurnPermille = 0f;
            _hasLastCockpitSpeed = false;
            _lastCockpitSpeedMps = 0f;
            _torpProgress = -1f;
            if (_activeTorpedoVisual != null)
                _activeTorpedoVisual.gameObject.SetActive(false);

            if (_contactsContainer != null)
            {
                for (int i = _contactsContainer.childCount - 1; i >= 0; i--)
                {
                    var ch = _contactsContainer.GetChild(i).gameObject;
                    if (Application.isPlaying) Destroy(ch);
                    else DestroyImmediate(ch);
                }
            }
            _namedOrbitalTargets.Clear();
            _targetLabelTexts.Clear();
            _contactStyleKeys.Clear();
            _liveTorpedoMarkers.Clear();
            _torpedoLabelTexts.Clear();
            _knownRockIds.Clear();

            if (PlayerShipTransform != null)
                PlayerShipTransform.rotation = Quaternion.identity;
            UpdateDrivePlume();
            SyncLockedAspectIndicator();
            OnSpaceTelemetryChanged?.Invoke();
        }

        /// <summary>
        /// Computes Unity ship rotation matching `crates/tacsim/src/space/bubble.rs:951-991`:
        /// positive `rollDeg` tilts starboard (`+X`) UP toward `+Y` (CCW looking forward along `+Z`).
        /// </summary>
        public static Quaternion ComputePlayerShipRotation(float yawDeg, float pitchDeg, float rollDeg)
        {
            return Quaternion.Euler(-pitchDeg, yawDeg, rollDeg);
        }

        public static Quaternion ComputeContactRotation(JsonNode c, Vector3 contactStagePos, Vector3 observerStagePos)
        {
            if (c != null && c.IsObject)
            {
                if (c.ContainsKey("yaw_deg") || c.ContainsKey("pitch_deg") || c.ContainsKey("roll_deg"))
                {
                    return ComputePlayerShipRotation(
                        c.Float("yaw_deg", 0f),
                        c.Float("pitch_deg", 0f),
                        c.Float("roll_deg", 0f));
                }

                if (c.ContainsKey("vx_kms") || c.ContainsKey("vy_kms") || c.ContainsKey("vz_kms"))
                {
                    // Same bubble -> Unity axis mapping as ComputeRelativeStageOffset: +x->+Z, +y->+X, +z->+Y
                    Vector3 velUnity = new Vector3(
                        c.Float("vy_kms", 0f),
                        c.Float("vz_kms", 0f),
                        c.Float("vx_kms", 0f));
                    if (velUnity.sqrMagnitude > 1e-8f)
                        return Quaternion.LookRotation(velUnity.normalized, Vector3.up);
                }
            }

            Vector3 towardObserver = observerStagePos - contactStagePos;
            if (towardObserver.sqrMagnitude > 1e-4f)
                return Quaternion.LookRotation(towardObserver.normalized, Vector3.up);
            return Quaternion.Euler(0f, 180f, 0f);
        }

        /// <summary>
        /// Reconciles 3D orbital contacts, live torpedoes, and asteroid cluster rocks strictly from authoritative
        /// `space.holotank` JSON (`x_km`/`y_km`/`z_km`, filtering `you: true` as the observer's own hull reference origin).
        /// Reuses existing GameObjects by ID to avoid per-frame destroy/create churn.
        /// </summary>
        public void ApplyHolotankFeed(JsonNode holotank)
        {
            if (holotank == null || !holotank.IsObject || holotank.Bool("refused", false) || _contactsContainer == null)
                return;

            var seenTargets = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var seenTorpedoes = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            _knownRockIds.Clear();
            OwnShipHolotankId = string.Empty;

            Material rockMat = UrpMaterialFactory.GetOpaque("AsteroidOre", new Color(0.32f, 0.28f, 0.24f), 0.15f, 0.2f);
            Material veinMat = UrpMaterialFactory.GetEmissive("OreVein", new Color(0.22f, 0.72f, 0.85f), new Color(0.25f, 0.95f, 1.0f) * 1.4f);

            double obsXKm = 0.0;
            double obsYKm = 0.0;
            double obsZKm = 0.0;
            var contactsArr = holotank["contacts"].AsArray() ?? holotank["blips"].AsArray();
            if (contactsArr != null)
            {
                for (int i = 0; i < contactsArr.Count; i++)
                {
                    var c = contactsArr[i];
                    if (c.IsObject && c.Bool("you", false))
                    {
                        OwnShipHolotankId = c.Str("id", string.Empty).Trim();
                        obsXKm = c.Double("x_km", 0.0);
                        obsYKm = c.Double("y_km", 0.0);
                        obsZKm = c.Double("z_km", 0.0);
                        double burnMag = Math.Abs(c.Double("ax_mps2", 0.0)) +
                                         Math.Abs(c.Double("ay_mps2", 0.0)) +
                                         Math.Abs(c.Double("az_mps2", 0.0));
                        _lastHolotankBurnPermille = burnMag > 0.01
                            ? Mathf.Clamp((float)(burnMag * 100.0), 150f, 1000f)
                            : 0f;
                        ThrottlePermille = _lastHolotankBurnPermille;
                        UpdateDrivePlume();
                        break;
                    }
                }
            }

            const int MaxOrbitalRocks = 32;
            const int MaxOrbitalContacts = 64;
            const int MaxLiveTorpedoes = 64;

            var rocksArr = holotank["rocks"].AsArray() ?? holotank["cluster"]?["rocks"].AsArray();
            if (rocksArr != null)
            {
                int rocksSpawned = 0;
                for (int r = 0; r < rocksArr.Count && rocksSpawned < MaxOrbitalRocks; r++)
                {
                    var rk = rocksArr[r];
                    if (!rk.IsObject) continue;
                    string rawRockId = rk.Str("id", rk.Str("letter", string.Empty));
                    if (!MushProtocol.TryValidateSafeCommandToken(rawRockId, out string validRockId)) continue;
                    string rId = validRockId.ToUpperInvariant();
                    string spectral = rk.Str("spectral", rk.Str("label", "Pocket"));
                    Vector3 offset = ComputeRelativeStageOffset(rk, obsXKm, obsYKm, obsZKm, defaultRadius: 48f, index: r);
                    UpsertOreRockTarget(rId, $"ROCK {rId} [{spectral}]", OrbitalStageOrigin + offset, r, rockMat, veinMat);
                    seenTargets.Add(rId);
                    rocksSpawned++;
                }
            }

            if (contactsArr != null)
            {
                int contactsSpawned = 0;
                for (int i = 0; i < contactsArr.Count && contactsSpawned < MaxOrbitalContacts; i++)
                {
                    var c = contactsArr[i];
                    if (!c.IsObject) continue;
                    // Never spawn the observer's own hull (`you: true`) as an external targetable contact!
                    if (c.Bool("you", false))
                        continue;

                    string kind = c.Str("kind", string.Empty);
                    string rawId = c.Str("designator", c.Str("id", string.Empty));
                    if (!MushProtocol.TryValidateSafeCommandToken(rawId, out string id)) continue;

                    if (string.Equals(kind, "rock", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(kind, "asteroid", StringComparison.OrdinalIgnoreCase))
                    {
                        string rKey = id.ToUpperInvariant();
                        string lbl = c.Str("label", c.Str("name", $"ROCK {rKey}"));
                        Vector3 offset = ComputeRelativeStageOffset(c, obsXKm, obsYKm, obsZKm, defaultRadius: 50f, index: i);
                        UpsertOreRockTarget(rKey, lbl, OrbitalStageOrigin + offset, i, rockMat, veinMat);
                        seenTargets.Add(rKey);
                        contactsSpawned++;
                        if (c.Bool("locked", false))
                            SelectedRockId = rKey;
                        continue;
                    }

                    string relation = c.Str("relation", c.Str("iff", "neutral"));
                    bool hostile = relation.IndexOf("hostile", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                   relation.IndexOf("enemy", StringComparison.OrdinalIgnoreCase) >= 0;
                    bool friendly = !hostile && (relation.IndexOf("friend", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                                 relation.IndexOf("ally", StringComparison.OrdinalIgnoreCase) >= 0);
                    string name = c.Str("name", c.Str("label", id));
                    Vector3 pos = OrbitalStageOrigin + ComputeRelativeStageOffset(c, obsXKm, obsYKm, obsZKm, defaultRadius: 85f, index: i);
                    Quaternion rot = ComputeContactRotation(c, pos, OrbitalStageOrigin);
                    Color col = hostile ? new Color(1f, 0.42f, 0.36f)
                              : (friendly ? new Color(0.45f, 1.0f, 0.65f) : new Color(0.45f, 0.82f, 1f));
                    UpsertNavalContact(id, $"{id} — {name}", pos, rot, col, hostile, friendly);
                    seenTargets.Add(id);
                    contactsSpawned++;
                    if (c.Bool("locked", false))
                        LockedContactId = id;
                }
            }

            // Render live server-perceived torpedoes (`space.holotank.torpedoes[]`)
            var torpsArr = holotank["torpedoes"].AsArray();
            if (torpsArr != null)
            {
                int torpCap = Math.Min(torpsArr.Count, MaxLiveTorpedoes);
                for (int t = 0; t < torpCap; t++)
                {
                    var tp = torpsArr[t];
                    if (!tp.IsObject) continue;
                    string tid = tp.Str("id", t.ToString());
                    if (string.IsNullOrEmpty(tid)) continue;

                    Vector3 torpPos = OrbitalStageOrigin + ComputeRelativeStageOffset(tp, obsXKm, obsYKm, obsZKm, defaultRadius: 65f, index: t);
                    Vector3 aimTargetPos = OrbitalStageOrigin;
                    if (tp.ContainsKey("target_x_km") || tp.ContainsKey("target_y_km") || tp.ContainsKey("target_z_km"))
                    {
                        var tgtNode = JsonNode.NewObject();
                        tgtNode["x_km"] = JsonNode.FromDouble(tp.Double("target_x_km", obsXKm));
                        tgtNode["y_km"] = JsonNode.FromDouble(tp.Double("target_y_km", obsYKm));
                        tgtNode["z_km"] = JsonNode.FromDouble(tp.Double("target_z_km", obsZKm));
                        aimTargetPos = OrbitalStageOrigin + ComputeRelativeStageOffset(tgtNode, obsXKm, obsYKm, obsZKm, defaultRadius: 65f, index: t);
                    }
                    int ttiS = tp.Int("tti_s", -1);
                    string tLabel = ttiS >= 0 ? $"TORP-{tid} ({ttiS}s)" : $"TORP-{tid}";
                    UpsertLiveTorpedoMarker(tid, tLabel, torpPos, aimTargetPos);
                    seenTorpedoes.Add(tid);
                }
            }

            PruneStalePooledTargets(seenTargets);
            PruneStalePooledTorpedoes(seenTorpedoes);

            if (!string.IsNullOrEmpty(SelectedRockId) && !_knownRockIds.Contains(SelectedRockId))
                SelectedRockId = string.Empty;
            if (string.IsNullOrEmpty(SelectedRockId))
            {
                foreach (string rid in _knownRockIds) { SelectedRockId = rid; break; }
            }

            if (!string.IsNullOrEmpty(LockedContactId) && !HasPerceivedContact(LockedContactId))
                LockedContactId = string.Empty;

            SyncLockedAspectIndicator();
            OnSpaceTelemetryChanged?.Invoke();
        }

        private void PruneStalePooledTargets(HashSet<string> seenTargets)
        {
            List<string> stale = null;
            foreach (var kv in _namedOrbitalTargets)
            {
                if (!seenTargets.Contains(kv.Key))
                {
                    if (stale == null) stale = new List<string>();
                    stale.Add(kv.Key);
                }
            }
            if (stale == null) return;
            for (int i = 0; i < stale.Count; i++)
            {
                string key = stale[i];
                if (_namedOrbitalTargets.TryGetValue(key, out var tr) && tr != null)
                {
                    if (Application.isPlaying) Destroy(tr.gameObject);
                    else DestroyImmediate(tr.gameObject);
                }
                _namedOrbitalTargets.Remove(key);
                _targetLabelTexts.Remove(key);
                _contactStyleKeys.Remove(key);
            }
        }

        private void PruneStalePooledTorpedoes(HashSet<string> seenTorpedoes)
        {
            List<string> stale = null;
            foreach (var kv in _liveTorpedoMarkers)
            {
                if (!seenTorpedoes.Contains(kv.Key))
                {
                    if (stale == null) stale = new List<string>();
                    stale.Add(kv.Key);
                }
            }
            if (stale == null) return;
            for (int i = 0; i < stale.Count; i++)
            {
                string key = stale[i];
                if (_liveTorpedoMarkers.TryGetValue(key, out var tr) && tr != null)
                {
                    if (Application.isPlaying) Destroy(tr.gameObject);
                    else DestroyImmediate(tr.gameObject);
                }
                _liveTorpedoMarkers.Remove(key);
                _torpedoLabelTexts.Remove(key);
            }
        }

        public static Vector3 ComputeRelativeStageOffset(
            JsonNode node,
            double obsXKm,
            double obsYKm,
            double obsZKm,
            float defaultRadius,
            int index)
        {
            // 1. Canonical `mushd` `space.holotank` 3D coordinates (`x_km`, `y_km`, `z_km`)
            if (node != null && node.IsObject &&
                (node.ContainsKey("x_km") || node.ContainsKey("y_km") || node.ContainsKey("z_km")))
            {
                double dxKm = node.Double("x_km", 0.0) - obsXKm;
                double dyKm = node.Double("y_km", 0.0) - obsYKm;
                double dzKm = node.Double("z_km", 0.0) - obsZKm;
                // Bubble axes: +x = bearing 0° (Unity +Z), +y = bearing 90° (Unity +X), +z = elevation up (Unity +Y)
                Vector3 deltaMeters = new Vector3(
                    (float)(dyKm * 1000.0),
                    (float)(dzKm * 1000.0),
                    (float)(dxKm * 1000.0));
                float distMeters = deltaMeters.magnitude;
                if (distMeters <= 0.01f)
                    return new Vector3(0f, 0f, 18f);
                float visualRadius = Mathf.Clamp(22f + 16f * Mathf.Log(1f + distMeters / 250f), 18f, 220f);
                return (deltaMeters / distMeters) * visualRadius;
            }

            // 2. Explicit metre offset arrays (`rel_pos_m` / `pos_m`)
            var rel = node?["rel_pos_m"].AsArray() ?? node?["pos_m"].AsArray();
            if (rel != null && rel.Count >= 3)
            {
                Vector3 deltaMeters = new Vector3(rel[1].AsFloat(0f), rel[2].AsFloat(0f), rel[0].AsFloat(0f));
                float distMeters = deltaMeters.magnitude;
                if (distMeters > 0.01f)
                {
                    float visualRadius = Mathf.Clamp(22f + 16f * Mathf.Log(1f + distMeters / 250f), 18f, 220f);
                    return (deltaMeters / distMeters) * visualRadius;
                }
            }

            // 3. Spherical `bearing_deg` / `pitch_deg` / `range_m` (e.g. `space.cockpit` contact projection)
            float bearingDeg = node != null ? node.Float("bearing_deg", index * 55f) : index * 55f;
            float pitchDeg = node != null ? node.Float("pitch_deg", 0f) : 0f;
            float rangeM = node != null ? node.Float("range_m", defaultRadius * 25f) : defaultRadius * 25f;
            float scaledDist = Mathf.Clamp(22f + 16f * Mathf.Log(1f + Mathf.Max(1f, rangeM) / 250f), 24f, 180f);
            float bRad = bearingDeg * Mathf.Deg2Rad;
            float pRad = pitchDeg * Mathf.Deg2Rad;
            float cosP = Mathf.Cos(pRad);
            return new Vector3(
                Mathf.Sin(bRad) * cosP * scaledDist,
                Mathf.Sin(pRad) * scaledDist,
                Mathf.Cos(bRad) * cosP * scaledDist);
        }

        private void UpsertOreRockTarget(string id, string label, Vector3 pos, int idx, Material rockMat, Material veinMat)
        {
            if (_namedOrbitalTargets.TryGetValue(id, out var existing) && existing != null &&
                _contactStyleKeys.TryGetValue(id, out string styleKey) && styleKey == "rock")
            {
                existing.position = pos;
                if (_targetLabelTexts.TryGetValue(id, out var existingTmp) && existingTmp != null)
                    existingTmp.text = MushProtocolLiteEscape(label);
                _knownRockIds.Add(id);
                return;
            }
            if (existing != null)
            {
                if (Application.isPlaying) Destroy(existing.gameObject);
                else DestroyImmediate(existing.gameObject);
            }

            var pocketRoot = new GameObject($"OreRock_{id}");
            pocketRoot.transform.SetParent(_contactsContainer, false);
            pocketRoot.transform.position = pos;

            var sc = pocketRoot.AddComponent<SphereCollider>();
            sc.radius = 6.5f;

            List<GameObject> orePool = (Catalog != null && Catalog.oreAsteroidPrefabs != null && Catalog.oreAsteroidPrefabs.Count > 0)
                ? Catalog.oreAsteroidPrefabs
                : (Catalog != null ? Catalog.asteroidPrefabs : null);

            if (orePool != null && orePool.Count > 0 && orePool[idx % orePool.Count] != null)
            {
                PrefabFittingHelper.SpawnCentered(orePool[idx % orePool.Count], pocketRoot.transform, "OreMesh", pos, Quaternion.Euler(idx * 35f, idx * 62f, 0f), 11f, keepColliders: false);
            }
            else
            {
                var sp = GameObject.CreatePrimitive(PrimitiveType.Sphere);
                sp.transform.SetParent(pocketRoot.transform, false);
                sp.transform.localScale = new Vector3(9.5f, 7.2f, 8.4f);
                var col = sp.GetComponent<Collider>();
                if (col != null) { if (Application.isPlaying) Destroy(col); else DestroyImmediate(col); }
                sp.GetComponent<MeshRenderer>().sharedMaterial = rockMat;
                CreateDockBox(pocketRoot.transform, "OreSeam", new Vector3(0f, 1.2f, -3.2f), new Vector3(4.5f, 0.65f, 2.2f), veinMat);
            }

            var tmp = AddOrbitalTag(pocketRoot.transform, label, new Vector3(0f, 7.2f, 0f), new Color(0.55f, 1f, 0.85f));
            _namedOrbitalTargets[id] = pocketRoot.transform;
            _targetLabelTexts[id] = tmp;
            _contactStyleKeys[id] = "rock";
            _knownRockIds.Add(id);
        }

        private void UpsertNavalContact(string id, string label, Vector3 pos, Quaternion rot, Color tagColor, bool isHostile, bool isFriendly)
        {
            string factionKey = isHostile ? "ship_hostile" : (isFriendly ? "ship_friendly" : "ship_neutral");
            if (_namedOrbitalTargets.TryGetValue(id, out var existing) && existing != null &&
                _contactStyleKeys.TryGetValue(id, out string styleKey) && styleKey == factionKey)
            {
                existing.position = pos;
                existing.rotation = rot;
                if (_targetLabelTexts.TryGetValue(id, out var existingTmp) && existingTmp != null)
                {
                    existingTmp.text = MushProtocolLiteEscape(label);
                    existingTmp.color = tagColor;
                }
                return;
            }
            if (existing != null)
            {
                if (Application.isPlaying) Destroy(existing.gameObject);
                else DestroyImmediate(existing.gameObject);
            }

            var cRoot = new GameObject($"NavalContact_{id}");
            cRoot.transform.SetParent(_contactsContainer, false);
            cRoot.transform.position = pos;
            cRoot.transform.rotation = rot;

            var bc = cRoot.AddComponent<BoxCollider>();
            bc.size = new Vector3(10f, 8f, 24f);

            GameObject prefab = Catalog != null
                ? (Catalog.corvetteExteriorPrefab != null ? Catalog.corvetteExteriorPrefab : Catalog.visitingFreighterPrefab)
                : null;
            if (prefab != null)
            {
                PrefabFittingHelper.SpawnCentered(prefab, cRoot.transform, "HullMesh", pos, rot, 24f, keepColliders: false);
            }
            else
            {
                string matKey = isHostile ? "ContactHull_Hostile" : (isFriendly ? "ContactHull_Friendly" : "ContactHull_Neutral");
                Color hullCol = isHostile ? new Color(0.38f, 0.22f, 0.22f)
                              : (isFriendly ? new Color(0.24f, 0.38f, 0.30f) : new Color(0.32f, 0.38f, 0.46f));
                Material cHull = UrpMaterialFactory.GetOpaque(matKey, hullCol, 0.5f, 0.65f);
                Material cTrim = UrpMaterialFactory.GetOpaque("ContactTrim", new Color(0.18f, 0.20f, 0.24f), 0.5f, 0.7f);
                var sculpted = StationWorldController.BuildSculptedDrayExterior(cRoot.transform, pos, cHull, cTrim, "CutterSilhouette");
                if (sculpted != null)
                {
                    sculpted.transform.localPosition = Vector3.zero;
                    sculpted.transform.localRotation = Quaternion.identity;
                }
            }

            var tmp = AddOrbitalTag(cRoot.transform, label, new Vector3(0f, 8.5f, 0f), tagColor);
            _namedOrbitalTargets[id] = cRoot.transform;
            _targetLabelTexts[id] = tmp;
            _contactStyleKeys[id] = factionKey;
        }

        private void UpsertLiveTorpedoMarker(string torpId, string label, Vector3 pos, Vector3 aimTargetPos)
        {
            Vector3 dir = aimTargetPos - pos;
            Quaternion rot = dir.sqrMagnitude > 1e-4f
                ? Quaternion.LookRotation(dir.normalized, Vector3.up) * Quaternion.Euler(90f, 0f, 0f)
                : Quaternion.Euler(90f, 0f, 0f);

            if (_liveTorpedoMarkers.TryGetValue(torpId, out var existing) && existing != null)
            {
                existing.position = pos;
                existing.rotation = rot;
                if (_torpedoLabelTexts.TryGetValue(torpId, out var existingTmp) && existingTmp != null)
                    existingTmp.text = MushProtocolLiteEscape(label);
                return;
            }

            var torpRoot = new GameObject($"LiveTorpedo_{torpId}");
            torpRoot.transform.SetParent(_contactsContainer, false);
            torpRoot.transform.position = pos;
            torpRoot.transform.rotation = rot;

            var meshGo = GameObject.CreatePrimitive(PrimitiveType.Capsule);
            meshGo.name = "TorpedoBody";
            meshGo.transform.SetParent(torpRoot.transform, false);
            meshGo.transform.localScale = new Vector3(0.85f, 2.2f, 0.85f);
            var col = meshGo.GetComponent<Collider>();
            if (col != null) { if (Application.isPlaying) Destroy(col); else DestroyImmediate(col); }
            meshGo.GetComponent<MeshRenderer>().sharedMaterial =
                UrpMaterialFactory.GetEmissive("LiveTorpFlare", new Color(1.0f, 0.38f, 0.14f), new Color(1.0f, 0.50f, 0.18f) * 2.8f);

            var tmp = AddOrbitalTag(torpRoot.transform, label, new Vector3(0f, 4.0f, 0f), new Color(1.0f, 0.55f, 0.25f));
            _liveTorpedoMarkers[torpId] = torpRoot.transform;
            _torpedoLabelTexts[torpId] = tmp;
        }

        private static TextMeshPro AddOrbitalTag(Transform parent, string label, Vector3 localOffset, Color color)
        {
            var bbGo = new GameObject("OrbitalBillboard");
            bbGo.transform.SetParent(parent, false);
            bbGo.transform.localPosition = localOffset;
            var bb = bbGo.AddComponent<WorldBillboard>();
            var txtGo = new GameObject("Label");
            txtGo.transform.SetParent(bbGo.transform, false);
            var tmp = txtGo.AddComponent<TextMeshPro>();
            tmp.richText = false;
            tmp.text = MushProtocolLiteEscape(label);
            tmp.fontSize = 16f;
            tmp.color = color;
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.NoWrap;
            tmp.rectTransform.sizeDelta = new Vector2(45f, 6f);
            bb.NameplateText = tmp;
            return tmp;
        }

        private static string MushProtocolLiteEscape(string s) =>
            string.IsNullOrEmpty(s) ? string.Empty : s.Replace("<", "‹").Replace(">", "›");

        public void SelectRock(string rockId)
        {
            if (!string.IsNullOrWhiteSpace(rockId))
                SelectedRockId = rockId.Trim().ToUpperInvariant();
        }

        public void NotifyApproachOrdered(string rockId)
        {
            if (string.IsNullOrWhiteSpace(rockId)) return;
            string r = rockId.Trim().ToUpperInvariant();
            SelectedRockId = r;
            // `+helm/approach` clears `session.matched_rock = None`; wait for `space.cockpit` to report authoritative approach.
            AuthoritativeMatchedRockId = string.Empty;
        }

        public void SetLockedContact(string designator)
        {
            if (!string.IsNullOrWhiteSpace(designator))
                LockedContactId = designator.Trim();
        }

        public void TriggerMiningLaser(string rockId = null, float durationSec = 4.2f)
        {
            if (!string.IsNullOrWhiteSpace(rockId)) SelectedRockId = rockId.Trim().ToUpperInvariant();
            Transform target = ResolveExactTarget(SelectedRockId);
            if (target == null || PlayerShipTransform == null || _activeBeamRenderer == null) return;

            Vector3 dir = (target.position - PlayerShipTransform.position).normalized;
            if (dir.sqrMagnitude > 0.01f)
                PlayerShipTransform.rotation = Quaternion.LookRotation(dir, Vector3.up);

            _activeBeamRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("MiningBeamCyan", new Color(0.20f, 0.96f, 0.85f, 0.95f));
            _activeBeamRenderer.startWidth = 0.65f;
            _activeBeamRenderer.endWidth = 0.42f;
            _activeBeamRenderer.SetPosition(0, PlayerShipTransform.position + PlayerShipTransform.forward * 11f);
            _activeBeamRenderer.SetPosition(1, target.position);
            _activeBeamRenderer.enabled = true;

            if (_activeBeamEndGlow != null)
            {
                _activeBeamEndGlow.position = target.position - dir * 4.2f;
                _activeBeamEndGlow.gameObject.SetActive(true);
            }
            _beamExpireTime = Time.time + durationSec;
        }

        public void StopMiningLaser()
        {
            IsAuthoritativeMiningLaserActive = false;
            _beamExpireTime = 0f;
            if (_activeBeamRenderer != null)
                _activeBeamRenderer.enabled = false;
            if (_activeBeamEndGlow != null)
                _activeBeamEndGlow.gameObject.SetActive(false);
        }

        public void ToggleDroneSortie(bool active)
        {
            _droneSortieActive = active;
            for (int i = 0; i < _collectorDrones.Count; i++)
            {
                if (_collectorDrones[i] != null)
                    _collectorDrones[i].gameObject.SetActive(active);
            }
        }

        public void TriggerNavalWeaponVfx(string weaponKind, string targetDesignator = null)
        {
            string tgtKey = !string.IsNullOrWhiteSpace(targetDesignator) ? targetDesignator : LockedContactId;
            Transform target = ResolveExactTarget(tgtKey);
            if (target == null || PlayerShipTransform == null) return;

            Vector3 muzzle = PlayerShipTransform.position + PlayerShipTransform.forward * 12f;
            string wk = (weaponKind ?? "beam").ToLowerInvariant();
            if (wk.Contains("torp"))
            {
                _torpStart = muzzle;
                _torpEnd = target.position;
                _torpProgress = 0f;
                if (_activeTorpedoVisual != null)
                {
                    _activeTorpedoVisual.position = muzzle;
                    _activeTorpedoVisual.gameObject.SetActive(true);
                }
                return;
            }

            if (_activeBeamRenderer == null)
                return;

            Color beamCol = wk.Contains("rail") ? new Color(0.55f, 0.85f, 1.0f, 0.98f) : new Color(1.0f, 0.24f, 0.32f, 0.95f);
            _activeBeamRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"Weapon_{wk}", beamCol);
            _activeBeamRenderer.startWidth = wk.Contains("rail") ? 0.28f : 0.52f;
            _activeBeamRenderer.endWidth = wk.Contains("rail") ? 0.18f : 0.35f;
            _activeBeamRenderer.SetPosition(0, muzzle);
            _activeBeamRenderer.SetPosition(1, target.position);
            _activeBeamRenderer.enabled = true;

            if (_activeBeamEndGlow != null)
            {
                _activeBeamEndGlow.position = target.position;
                _activeBeamEndGlow.gameObject.SetActive(true);
            }
            _beamExpireTime = Time.time + (wk.Contains("rail") ? 0.45f : 1.6f);
        }

        private Transform ResolveExactTarget(string id)
        {
            if (ResolveContactKey(id, out string key) && _namedOrbitalTargets.TryGetValue(key, out var tr) && tr != null)
                return tr;
            return null;
        }

        private void BuildBowDockingRig()
        {
            var dockRoot = new GameObject("BowDockingRig").transform;
            dockRoot.SetParent(transform, true);
            dockRoot.position = DockingStageOrigin;

            _dockingCollarTarget = new GameObject("TargetCollarPort").transform;
            _dockingCollarTarget.SetParent(dockRoot, false);
            _dockingCollarTarget.localPosition = Vector3.zero;

            Material portBulkhead = UrpMaterialFactory.GetOpaque("DockPortWall", new Color(0.16f, 0.19f, 0.23f), 0.4f, 0.65f);
            Material collarRim = UrpMaterialFactory.GetOpaque("CollarRim", new Color(0.35f, 0.38f, 0.42f), 0.55f, 0.8f);
            Material crossMat = UrpMaterialFactory.GetEmissive("AlignmentCross", new Color(0.9f, 0.7f, 0.15f), new Color(1.0f, 0.75f, 0.2f) * 1.6f);

            if (Catalog != null && Catalog.shipyardCradlePrefab != null)
            {
                PrefabFittingHelper.SpawnCentered(Catalog.shipyardCradlePrefab, dockRoot, "CeresDockingCradleBackdrop", DockingStageOrigin + new Vector3(0f, 0f, 14f), Quaternion.identity, 28f, keepColliders: false);
            }

            CreateDockBox(_dockingCollarTarget, "StationBulkhead_Top", new Vector3(0f, 2.2f, 0.4f), new Vector3(8f, 2.4f, 0.8f), portBulkhead);
            CreateDockBox(_dockingCollarTarget, "StationBulkhead_Bot", new Vector3(0f, -2.2f, 0.4f), new Vector3(8f, 2.4f, 0.8f), portBulkhead);
            CreateDockBox(_dockingCollarTarget, "StationBulkhead_L", new Vector3(-2.6f, 0f, 0.4f), new Vector3(3.2f, 2.0f, 0.8f), portBulkhead);
            CreateDockBox(_dockingCollarTarget, "StationBulkhead_R", new Vector3(2.6f, 0f, 0.4f), new Vector3(3.2f, 2.0f, 0.8f), portBulkhead);

            for (int i = 0; i < 8; i++)
            {
                float a = i * 45f * Mathf.Deg2Rad;
                var seg = CreateDockBox(_dockingCollarTarget, $"CollarClamp_{i}", new Vector3(Mathf.Cos(a) * 0.95f, Mathf.Sin(a) * 0.95f, 0f), new Vector3(0.32f, 0.78f, 0.35f), collarRim);
                seg.transform.localRotation = Quaternion.Euler(0f, 0f, i * 45f);
            }
            CreateDockBox(_dockingCollarTarget, "TopRollIndexChevron", new Vector3(0f, 1.18f, -0.1f), new Vector3(0.14f, 0.42f, 0.14f), crossMat);
            CreateDockBox(_dockingCollarTarget, "StandoffCross_H", new Vector3(0f, 0f, 0.65f), new Vector3(0.75f, 0.04f, 0.04f), crossMat);
            CreateDockBox(_dockingCollarTarget, "StandoffCross_V", new Vector3(0f, 0f, 0.65f), new Vector3(0.04f, 0.75f, 0.04f), crossMat);

            for (int g = 0; g < 5; g++)
            {
                var ring = GameObject.CreatePrimitive(PrimitiveType.Cube);
                ring.name = $"ApproachGate_{g}";
                ring.transform.SetParent(_dockingCollarTarget, false);
                ring.transform.localPosition = new Vector3(0f, 0f, -(g + 1) * 0.48f);
                ring.transform.localScale = new Vector3(0.55f + g * 0.14f, 0.55f + g * 0.14f, 0.02f);
                var rc = ring.GetComponent<Collider>();
                if (rc != null) { if (Application.isPlaying) Destroy(rc); else DestroyImmediate(rc); }
                var mr = ring.GetComponent<MeshRenderer>();
                mr.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("DockGate_Amber", new Color(1.0f, 0.58f, 0.16f, 0.18f));
                _gateRingRenderers.Add(mr);
            }

            var flood = new GameObject("BowSpotLight");
            flood.transform.SetParent(dockRoot, false);
            flood.transform.localPosition = new Vector3(0f, 1.5f, -4.5f);
            var lt = flood.AddComponent<Light>();
            lt.type = LightType.Point;
            lt.range = 18f;
            lt.intensity = 6.5f;
            lt.color = new Color(0.88f, 0.95f, 1f);

            BowDockingCameraAnchor = new GameObject("BowCollarCamera").transform;
            BowDockingCameraAnchor.SetParent(dockRoot, false);
            BowDockingCameraAnchor.localPosition = new Vector3(0f, 0f, -2.4f);
            BowDockingCameraAnchor.localRotation = Quaternion.identity;
        }

        public static string NormalizeLaserModeWord(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw)) return string.Empty;
            string lower = raw.Trim().ToLowerInvariant();
            if (lower == "offence" || lower == "offense") return "offense";
            if (lower == "defence" || lower == "defense" || lower == "point") return "defense";
            return lower;
        }

        public static void ParseSpaceFireTelemetry(
            JsonNode fireObj,
            List<SpaceMountTelemetry> mountsOut,
            out SpaceLockTelemetry lockOut)
        {
            mountsOut?.Clear();
            lockOut = null;
            if (fireObj == null || !fireObj.IsObject)
                return;

            var mountsArr = fireObj["mounts"].AsArray();
            if (mountsArr != null && mountsOut != null)
            {
                for (int i = 0; i < mountsArr.Count; i++)
                {
                    var m = mountsArr[i];
                    if (!m.IsObject) continue;
                    string id = m.Str("id", string.Empty).Trim();
                    if (string.IsNullOrEmpty(id)) continue;

                    mountsOut.Add(new SpaceMountTelemetry
                    {
                        Id = id,
                        Family = m.Str("family", "laser").Trim().ToLowerInvariant(),
                        State = m.Str("state", "ready").Trim().ToLowerInvariant(),
                        ReadyInMs = Math.Max(0, m.Int("ready_in_ms", 0)),
                        Mode = NormalizeLaserModeWord(m.Str("mode", string.Empty)),
                        Rounds = m.ContainsKey("rounds") && !m["rounds"].IsNull ? m.Int("rounds", -1) : -1,
                        RoundsMax = m.ContainsKey("rounds_max") && !m["rounds_max"].IsNull ? m.Int("rounds_max", -1) : -1,
                        DamageAtLock = m.ContainsKey("damage") && !m["damage"].IsNull ? m.Int("damage", -1) : -1,
                        FlightMsAtLock = m.ContainsKey("flight_ms") && !m["flight_ms"].IsNull ? m.Int("flight_ms", -1) : -1,
                    });
                }
            }

            var lockNode = fireObj["lock"];
            if (lockNode.IsObject)
            {
                string lockId = lockNode.Str("id", string.Empty).Trim();
                if (!string.IsNullOrEmpty(lockId))
                {
                    string facet = lockNode.Str("facet", "front").Trim().ToLowerInvariant();
                    if (string.IsNullOrEmpty(facet)) facet = "front";
                    lockOut = new SpaceLockTelemetry
                    {
                        ShipId = lockId,
                        Label = lockNode.Str("label", lockId).Trim(),
                        RangeM = Math.Max(0L, lockNode.Long("range_m", 0L)),
                        Facet = facet,
                    };
                }
            }
        }

        public static bool ExtractLockedContactIdFromFireNode(JsonNode fireObj, out string lockId, out bool explicitClear)
        {
            lockId = string.Empty;
            explicitClear = false;
            if (fireObj == null || !fireObj.IsObject)
                return false;

            var lockNode = fireObj["lock"];
            string serverLock = lockNode.IsObject
                ? lockNode.Str("id", string.Empty)
                : fireObj.Str("lock_id", fireObj.Str("lock", fireObj.Str("locked", string.Empty)));
            if (!string.IsNullOrEmpty(serverLock))
            {
                lockId = serverLock.Trim();
                return true;
            }
            if ((fireObj.ContainsKey("lock_id") && fireObj["lock_id"].IsNull) ||
                (fireObj.ContainsKey("lock") && fireObj["lock"].IsNull))
            {
                explicitClear = true;
                return true;
            }
            return false;
        }

        public static void ParseClusterMiningState(
            JsonNode cockpit,
            out bool hasMiningBlock,
            out string matchedRock,
            out string approachingRock,
            out int activeDrones,
            out string activeLaserRock)
        {
            hasMiningBlock = false;
            matchedRock = string.Empty;
            approachingRock = string.Empty;
            activeDrones = -1;
            activeLaserRock = string.Empty;
            if (cockpit == null || !cockpit.IsObject)
                return;

            var miningObj = cockpit["mining"];
            if (miningObj.IsObject)
            {
                hasMiningBlock = true;
                if (miningObj.ContainsKey("matched_rock") && !miningObj["matched_rock"].IsNull)
                    matchedRock = miningObj.Str("matched_rock", string.Empty).Trim().ToUpperInvariant();
                if (miningObj.ContainsKey("approaching_rock") && !miningObj["approaching_rock"].IsNull)
                    approachingRock = miningObj.Str("approaching_rock", string.Empty).Trim().ToUpperInvariant();
                activeDrones = miningObj.Int("active_drones", 0);
                if (miningObj["laser"].IsObject)
                    activeLaserRock = miningObj["laser"].Str("rock", matchedRock).Trim().ToUpperInvariant();
                return;
            }

            if (cockpit.ContainsKey("matched_rock") && !cockpit["matched_rock"].IsNull)
                matchedRock = cockpit.Str("matched_rock", string.Empty).Trim().ToUpperInvariant();
            activeDrones = cockpit.Int("active_drones", -1);
        }

        public void ApplyCockpitFeed(JsonNode cockpit)
        {
            if (cockpit == null || !cockpit.IsObject || cockpit.Bool("refused", false)) return;
            ShipYawDeg = cockpit.Float("yaw_deg", ShipYawDeg);
            ShipPitchDeg = cockpit.Float("pitch_deg", ShipPitchDeg);
            ShipRollDeg = cockpit.Float("roll_deg", ShipRollDeg);

            if (cockpit.ContainsKey("thrust_permille") || cockpit.ContainsKey("throttle_permille"))
            {
                ThrottlePermille = Mathf.Clamp(cockpit.Float("thrust_permille", cockpit.Float("throttle_permille", 0f)), 0f, 1000f);
            }
            else if (cockpit.ContainsKey("accel_mps2"))
            {
                ThrottlePermille = Mathf.Clamp(Mathf.Abs(cockpit.Float("accel_mps2", 0f)) * 100f, 0f, 1000f);
            }
            else if (cockpit.ContainsKey("thrust"))
            {
                ThrottlePermille = Mathf.Clamp(Mathf.Abs(cockpit.Float("thrust", 0f)) * 1000f, 0f, 1000f);
            }
            else if (cockpit.ContainsKey("burning"))
            {
                ThrottlePermille = cockpit.Bool("burning", false) ? 350f : 0f;
            }
            else if (_lastHolotankBurnPermille > 0f)
            {
                ThrottlePermille = _lastHolotankBurnPermille;
            }
            else if (cockpit.ContainsKey("speed_mps"))
            {
                float speedMps = cockpit.Float("speed_mps", 0f);
                if (_hasLastCockpitSpeed && Mathf.Abs(speedMps - _lastCockpitSpeedMps) > 0.25f)
                    ThrottlePermille = 350f;
                else if (_hasLastCockpitSpeed)
                    ThrottlePermille = 0f;
                _lastCockpitSpeedMps = speedMps;
                _hasLastCockpitSpeed = true;
            }

            EmconPosture = cockpit.Str("emcon", EmconPosture);
            string siteId = cockpit.Str("site_id", string.Empty).Trim();
            if (!string.IsNullOrEmpty(siteId))
                ActiveSiteId = siteId;

            // Fallback contact/torpedo projection from `space.cockpit.contacts[]` when `space.holotank` has not populated contacts yet
            var cockpitContacts = cockpit["contacts"].AsArray();
            if (cockpitContacts != null && cockpitContacts.Count > 0 && _contactsContainer != null &&
                _namedOrbitalTargets.Count == 0 && _liveTorpedoMarkers.Count == 0)
            {
                int fallbackCap = Math.Min(cockpitContacts.Count, 64);
                for (int i = 0; i < fallbackCap; i++)
                {
                    var c = cockpitContacts[i];
                    if (!c.IsObject) continue;
                    string rawCid = c.Str("id", string.Empty);
                    if (!MushProtocol.TryValidateSafeCommandToken(rawCid, out string cid)) continue;
                    string kind = c.Str("kind", "ship");
                    string label = c.Str("label", cid);
                    Vector3 pos = OrbitalStageOrigin + ComputeRelativeStageOffset(c, 0.0, 0.0, 0.0, defaultRadius: 85f, index: i);
                    if (string.Equals(kind, "torpedo", StringComparison.OrdinalIgnoreCase))
                    {
                        int tti = c.Int("tti_s", -1);
                        UpsertLiveTorpedoMarker(cid, tti >= 0 ? $"{label} ({tti}s)" : label, pos, OrbitalStageOrigin);
                    }
                    else
                    {
                        string relation = c.Str("relation", "neutral");
                        bool hostile = relation.IndexOf("hostile", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                       relation.IndexOf("enemy", StringComparison.OrdinalIgnoreCase) >= 0;
                        bool friendly = !hostile && (relation.IndexOf("friend", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                                     relation.IndexOf("ally", StringComparison.OrdinalIgnoreCase) >= 0);
                        Color col = hostile ? new Color(1f, 0.42f, 0.36f)
                                  : (friendly ? new Color(0.45f, 1.0f, 0.65f) : new Color(0.45f, 0.82f, 1f));
                        UpsertNavalContact(cid, $"{cid} — {label}", pos, ComputeContactRotation(c, pos, OrbitalStageOrigin), col, hostile, friendly);
                        if (c.Bool("locked", false))
                            LockedContactId = cid;
                    }
                }
            }

            var fireNode = cockpit["fire"];
            if (fireNode.IsObject)
            {
                ParseSpaceFireTelemetry(fireNode, _fireMounts, out var parsedLock);
                ActiveLock = parsedLock;
                if (ExtractLockedContactIdFromFireNode(fireNode, out string serverLock, out bool clearLock))
                {
                    if (!string.IsNullOrEmpty(serverLock))
                        LockedContactId = serverLock;
                    else if (clearLock)
                        LockedContactId = string.Empty;
                }
            }
            else
            {
                _fireMounts.Clear();
                ActiveLock = null;
            }

            ParseClusterMiningState(
                cockpit,
                out bool hasMining,
                out string matchedRock,
                out string approachingRock,
                out int activeDrones,
                out string activeLaserRock);

            HasClusterMiningTelemetry = hasMining;
            AuthoritativeMatchedRockId = matchedRock;
            AuthoritativeApproachingRockId = approachingRock;

            if (!hasMining)
            {
                // Left asteroid cluster: clear stale drone sortie & mining laser state!
                SelectedRockId = string.Empty;
                ToggleDroneSortie(false);
                StopMiningLaser();
            }
            else
            {
                if (!string.IsNullOrEmpty(matchedRock))
                    SelectedRockId = matchedRock;
                else if (!string.IsNullOrEmpty(approachingRock))
                    SelectedRockId = approachingRock;

                if (activeDrones >= 0)
                    ToggleDroneSortie(activeDrones > 0);

                if (!string.IsNullOrEmpty(activeLaserRock))
                {
                    IsAuthoritativeMiningLaserActive = true;
                    SelectedRockId = activeLaserRock;
                    TriggerMiningLaser(activeLaserRock, durationSec: 8.0f);
                }
                else if (IsAuthoritativeMiningLaserActive)
                {
                    StopMiningLaser();
                }
            }

            if (PlayerShipTransform != null)
                PlayerShipTransform.rotation = ComputePlayerShipRotation(ShipYawDeg, ShipPitchDeg, ShipRollDeg);
            UpdateDrivePlume();
            SyncLockedAspectIndicator();
            OnSpaceTelemetryChanged?.Invoke();
        }

        private void SyncLockedAspectIndicator()
        {
            if (_lockAspectIndicator == null)
                return;

            string lockTargetId = ActiveLock != null && !string.IsNullOrEmpty(ActiveLock.ShipId)
                ? ActiveLock.ShipId
                : LockedContactId;
            Transform targetTr = ResolveExactTarget(lockTargetId);
            if (targetTr == null)
            {
                _lockAspectIndicator.gameObject.SetActive(false);
                return;
            }

            _lockAspectIndicator.position = targetTr.position;
            _lockAspectIndicator.rotation = targetTr.rotation;

            string facet = ActiveLock != null && !string.IsNullOrEmpty(ActiveLock.Facet)
                ? ActiveLock.Facet
                : "front";

            if (_lockAspectSectorPlate != null)
            {
                switch (facet)
                {
                    case "rear":
                        _lockAspectSectorPlate.localPosition = new Vector3(0f, 0.35f, -12.5f);
                        _lockAspectSectorPlate.localRotation = Quaternion.identity;
                        _lockAspectSectorPlate.localScale = new Vector3(9.5f, 1.4f, 1.6f);
                        break;
                    case "left":
                        _lockAspectSectorPlate.localPosition = new Vector3(-12.5f, 0.35f, 0f);
                        _lockAspectSectorPlate.localRotation = Quaternion.Euler(0f, 90f, 0f);
                        _lockAspectSectorPlate.localScale = new Vector3(9.5f, 1.4f, 1.6f);
                        break;
                    case "right":
                        _lockAspectSectorPlate.localPosition = new Vector3(12.5f, 0.35f, 0f);
                        _lockAspectSectorPlate.localRotation = Quaternion.Euler(0f, 90f, 0f);
                        _lockAspectSectorPlate.localScale = new Vector3(9.5f, 1.4f, 1.6f);
                        break;
                    case "top":
                        _lockAspectSectorPlate.localPosition = new Vector3(0f, 6.4f, 0f);
                        _lockAspectSectorPlate.localRotation = Quaternion.identity;
                        _lockAspectSectorPlate.localScale = new Vector3(9.0f, 0.65f, 9.0f);
                        break;
                    default: // "front"
                        _lockAspectSectorPlate.localPosition = new Vector3(0f, 0.35f, 12.5f);
                        _lockAspectSectorPlate.localRotation = Quaternion.identity;
                        _lockAspectSectorPlate.localScale = new Vector3(9.5f, 1.4f, 1.6f);
                        break;
                }
            }

            if (_lockAspectLabel != null)
            {
                if (ActiveLock != null)
                {
                    string rangeStr = ActiveLock.RangeM >= 1000
                        ? $"{ActiveLock.RangeM / 1000f:0.0} KM"
                        : $"{ActiveLock.RangeM} M";
                    string lbl = string.IsNullOrEmpty(ActiveLock.Label) ? ActiveLock.ShipId : ActiveLock.Label;
                    _lockAspectLabel.text = MushProtocolLiteEscape(
                        $"LOCK: {lbl.ToUpperInvariant()} · {rangeStr} · [{facet.ToUpperInvariant()} BAND]");
                }
                else
                {
                    _lockAspectLabel.text = MushProtocolLiteEscape($"LOCK: {lockTargetId.ToUpperInvariant()}");
                }
            }

            _lockAspectIndicator.gameObject.SetActive(true);
        }

        public void SetLocalHelmPreview(float yawDeg, float pitchDeg, float throttlePermille)
        {
            ShipYawDeg = (yawDeg % 360f + 360f) % 360f;
            ShipPitchDeg = Mathf.Clamp(pitchDeg, -85f, 85f);
            ThrottlePermille = Mathf.Clamp(throttlePermille, 0f, 1000f);
            if (PlayerShipTransform != null)
                PlayerShipTransform.rotation = ComputePlayerShipRotation(ShipYawDeg, ShipPitchDeg, ShipRollDeg);
            UpdateDrivePlume();
        }

        private void UpdateDrivePlume()
        {
            bool burning = ThrottlePermille > 5f;
            float t = Mathf.Clamp01(ThrottlePermille / 1000f);

            if (_mainDrivePlume != null)
            {
                _mainDrivePlume.gameObject.SetActive(burning);
                if (burning)
                    _mainDrivePlume.localScale = new Vector3(1.4f + t * 1.1f, 1.8f + t * 5.2f, 1.4f + t * 1.1f);
            }

            if (_particlePlumeInstance != null)
            {
                _particlePlumeInstance.gameObject.SetActive(burning);
                if (burning)
                    _particlePlumeInstance.localScale = Vector3.one * (0.65f + t * 0.95f);
            }
        }

        public void StartDockingFromFeed(JsonNode dockingFeed)
        {
            if (dockingFeed == null || !dockingFeed.IsObject || dockingFeed.Bool("closed", false) || dockingFeed.Bool("refused", false))
            {
                bool hadSession = ActiveDockingSession != null || _tapeSubmittedAwaitingServer;
                ActiveDockingSession = null;
                _tapeSubmittedAwaitingServer = false;
                if (hadSession)
                {
                    string outcome = dockingFeed != null && dockingFeed.IsObject
                        ? dockingFeed.Str("outcome", "closed")
                        : "closed";
                    OnDockingEndedByServer?.Invoke(outcome);
                }
                return;
            }
            string sid = dockingFeed.Str("session_id", string.Empty);
            if (!MushProtocol.TryValidateSafeCommandToken(sid, out string safeSid)) return;
            if (ActiveDockingSession != null && ActiveDockingSession.SessionId == safeSid)
                return;

            ActiveDockingSession = DockingClientSession.FromFeed(dockingFeed);
            _dockingAccumulatorS = 0f;
            _tapeSubmittedAwaitingServer = false;
            GuidanceAssistEnabled = false;
            SyncBowDockingCameraPose();
        }

        public void StopDockingSession()
        {
            ActiveDockingSession = null;
            _tapeSubmittedAwaitingServer = false;
        }

        private void Update()
        {
            if (_ceresBody != null)
                _ceresBody.Rotate(Vector3.up, Time.deltaTime * 1.4f, UnityEngine.Space.World);

            UpdateOrbitalMiningAndWeaponAnimations();

            if (_tapeSubmittedAwaitingServer)
            {
                if (Time.unscaledTime - _tapeSubmittedAtUnscaledTime > DockingSubmitTimeoutSec)
                {
                    ActiveDockingSession = null;
                    _tapeSubmittedAwaitingServer = false;
                    OnSpaceAlert?.Invoke("Docking verification timed out waiting for server response.");
                    OnDockingEndedByServer?.Invoke("timeout");
                }
                return;
            }
            if (ActiveDockingSession == null)
                return;

            ProcessDockingHotkeys(
                Input.GetKeyDown(KeyCode.X),
                Input.GetKeyDown(KeyCode.G),
                Input.GetKeyDown(KeyCode.Backspace));

            _dockingAccumulatorS += Mathf.Min(0.1f, Time.unscaledDeltaTime);
            const float StepDt = 1f / 60f;

            while (_dockingAccumulatorS >= StepDt && ActiveDockingSession != null && !_tapeSubmittedAwaitingServer)
            {
                _dockingAccumulatorS -= StepDt;
                DockingControlInput input = GuidanceAssistEnabled
                    ? ActiveDockingSession.ComputeGuidanceAssistInput()
                    : SamplePilotDockingInput();

                if (_pendingDampenerToggle)
                {
                    input.ToggleDampeners = true;
                    _pendingDampenerToggle = false;
                }
                if (_pendingAbort)
                {
                    input.Abort = true;
                    _pendingAbort = false;
                    ActiveDockingSession.Step60Fps(input);
                    ActiveDockingSession = null;
                    OnDockingAborted?.Invoke();
                    return;
                }

                ActiveDockingSession.Step60Fps(input);

                if (ActiveDockingSession.IsClamped)
                {
                    string tapeCmd = ActiveDockingSession.FormatTapeCommand();
                    if (string.IsNullOrEmpty(tapeCmd) || tapeCmd.Length > DockingClientSession.MaxSafeCommandChars)
                    {
                        ActiveDockingSession = null;
                        OnSpaceAlert?.Invoke("Docking tape invalid or exceeded safe command length — aborting session; re-initiate approach.");
                        OnDockingAborted?.Invoke();
                        return;
                    }
                    _tapeSubmittedAwaitingServer = true;
                    _tapeSubmittedAtUnscaledTime = Time.unscaledTime;
                    SyncBowDockingCameraPose();
                    OnDockingTapeReadyToSubmit?.Invoke(tapeCmd, true);
                    return;
                }

                if (ActiveDockingSession.IsCollarScrapeBounce)
                {
                    string tapeCmd = ActiveDockingSession.FormatTapeCommand();
                    if (string.IsNullOrEmpty(tapeCmd) || tapeCmd.Length > DockingClientSession.MaxSafeCommandChars)
                    {
                        ActiveDockingSession = null;
                        OnDockingAborted?.Invoke();
                        return;
                    }
                    _tapeSubmittedAwaitingServer = true;
                    _tapeSubmittedAtUnscaledTime = Time.unscaledTime;
                    SyncBowDockingCameraPose();
                    OnSpaceAlert?.Invoke($"Collar contact out of gate at {ActiveDockingSession.VzMmps} mm/s — awaiting server bounce check...");
                    OnDockingTapeReadyToSubmit?.Invoke(tapeCmd, false);
                    return;
                }

                if (ActiveDockingSession.IsTapeBudgetExhausted)
                {
                    ActiveDockingSession = null;
                    OnSpaceAlert?.Invoke("Docking approach reached 60s / RLE frame limit — aborting approach.");
                    OnDockingAborted?.Invoke();
                    return;
                }
            }

            SyncBowDockingCameraPose();
        }

        private void UpdateOrbitalMiningAndWeaponAnimations()
        {
            if (_activeBeamRenderer != null && _activeBeamRenderer.enabled && Time.time > _beamExpireTime)
            {
                _activeBeamRenderer.enabled = false;
                if (_activeBeamEndGlow != null) _activeBeamEndGlow.gameObject.SetActive(false);
            }

            if (_torpProgress >= 0f && _activeTorpedoVisual != null)
            {
                _torpProgress += Time.deltaTime * 1.35f;
                if (_torpProgress >= 1f)
                {
                    _torpProgress = -1f;
                    _activeTorpedoVisual.gameObject.SetActive(false);
                    if (_activeBeamEndGlow != null)
                    {
                        _activeBeamEndGlow.position = _torpEnd;
                        _activeBeamEndGlow.gameObject.SetActive(true);
                        _beamExpireTime = Time.time + 0.65f;
                    }
                }
                else
                {
                    _activeTorpedoVisual.position = Vector3.Lerp(_torpStart, _torpEnd, _torpProgress);
                    Vector3 fwd = (_torpEnd - _torpStart).normalized;
                    if (fwd.sqrMagnitude > 0.01f)
                        _activeTorpedoVisual.rotation = Quaternion.LookRotation(fwd) * Quaternion.Euler(90f, 0f, 0f);
                }
            }

            if (_droneSortieActive && _collectorDrones.Count > 0 && PlayerShipTransform != null)
            {
                Transform rock = ResolveExactTarget(SelectedRockId);
                Vector3 dst = rock != null ? rock.position : OrbitalStageOrigin + new Vector3(0f, 0f, 45f);
                Vector3 src = PlayerShipTransform.position + new Vector3(0f, 2f, 0f);
                for (int i = 0; i < _collectorDrones.Count; i++)
                {
                    var d = _collectorDrones[i];
                    if (d == null || !d.gameObject.activeSelf) continue;
                    float phase = Mathf.PingPong(Time.time * 0.28f + i * 0.5f, 1f);
                    Vector3 lateral = new Vector3(i == 0 ? -4.5f : 4.5f, 1.5f + Mathf.Sin(Time.time * 2f + i) * 0.8f, 0f);
                    d.position = Vector3.Lerp(src + lateral, dst + lateral * 0.5f, phase);
                    d.LookAt(phase < 0.5f ? dst : src);
                }
            }

            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (!overUi && Input.GetMouseButtonDown(0) && !IsDockingActive)
            {
                Camera cam = Camera.main;
                if (cam != null && Vector3.Distance(cam.transform.position, OrbitalStageOrigin) < 280f)
                {
                    Ray ray = cam.ScreenPointToRay(Input.mousePosition);
                    if (Physics.Raycast(ray, out RaycastHit hit, 350f))
                    {
                        foreach (var kv in _namedOrbitalTargets)
                        {
                            if (kv.Value != null && (hit.transform == kv.Value || hit.transform.IsChildOf(kv.Value)))
                            {
                                if (!MushProtocol.TryValidateSafeCommandToken(kv.Key, out string safeKey))
                                    break;
                                bool isRock = _knownRockIds.Contains(kv.Key);
                                if (isRock) SelectedRockId = safeKey;
                                else LockedContactId = safeKey;
                                OnOrbitalTargetClicked?.Invoke(safeKey, isRock);
                                break;
                            }
                        }
                    }
                }
            }
        }

        public bool ProcessDockingHotkeys(bool keyXDown, bool keyGDown, bool keyBackspaceDown)
        {
            if (KeyboardSuspended || ActiveDockingSession == null)
                return false;
            bool handled = false;
            if (keyXDown)
            {
                _pendingDampenerToggle = true;
                handled = true;
            }
            if (keyGDown)
            {
                GuidanceAssistEnabled = !GuidanceAssistEnabled;
                handled = true;
            }
            if (keyBackspaceDown)
            {
                _pendingAbort = true;
                handled = true;
            }
            return handled;
        }

        public DockingControlInput SamplePilotDockingInput()
        {
            if (KeyboardSuspended)
                return default;

            var inp = new DockingControlInput();
            if (Input.GetKey(KeyCode.D)) inp.TransX = 1;
            else if (Input.GetKey(KeyCode.A)) inp.TransX = -1;

            if (Input.GetKey(KeyCode.W)) inp.TransY = 1;
            else if (Input.GetKey(KeyCode.S)) inp.TransY = -1;

            if (Input.GetKey(KeyCode.R) || Input.GetKey(KeyCode.Space)) inp.ClosurePulse = 1;
            else if (Input.GetKey(KeyCode.F) || Input.GetKey(KeyCode.LeftControl)) inp.ClosurePulse = -1;

            if (Input.GetKey(KeyCode.UpArrow)) inp.PitchTrim = 1;
            else if (Input.GetKey(KeyCode.DownArrow)) inp.PitchTrim = -1;

            if (Input.GetKey(KeyCode.RightArrow)) inp.YawTrim = 1;
            else if (Input.GetKey(KeyCode.LeftArrow)) inp.YawTrim = -1;

            if (Input.GetKey(KeyCode.E)) inp.RollTrim = 1;
            else if (Input.GetKey(KeyCode.Q)) inp.RollTrim = -1;

            return inp;
        }

        private void SyncBowDockingCameraPose()
        {
            if (ActiveDockingSession == null || BowDockingCameraAnchor == null)
                return;

            float xM = ActiveDockingSession.DxMm * 0.001f;
            float yM = ActiveDockingSession.DyMm * 0.001f;
            float zM = -Mathf.Max(0.18f, ActiveDockingSession.DzMm * 0.001f);

            float pitchDeg = ActiveDockingSession.PitchMdeg * 0.001f;
            float yawDeg = ActiveDockingSession.YawMdeg * 0.001f;
            float rollDeg = ActiveDockingSession.RollMdeg * 0.001f;

            BowDockingCameraAnchor.localPosition = new Vector3(xM, yM, zM);
            BowDockingCameraAnchor.localRotation = Quaternion.Euler(-pitchDeg, yawDeg, -rollDeg);

            var gates = ActiveDockingSession.EvaluateGates();
            Material gateMat = gates.AllGreen
                ? UrpMaterialFactory.GetHologramUnlit("DockGate_Green", new Color(0.2f, 1.0f, 0.55f, 0.34f))
                : (gates.LateralOk && gates.AttitudeOk && gates.RollOk
                    ? UrpMaterialFactory.GetHologramUnlit("DockGate_Cyan", new Color(0.35f, 0.88f, 1.0f, 0.26f))
                    : UrpMaterialFactory.GetHologramUnlit("DockGate_Amber", new Color(1.0f, 0.58f, 0.16f, 0.18f)));

            for (int i = 0; i < _gateRingRenderers.Count; i++)
            {
                if (_gateRingRenderers[i] != null && _gateRingRenderers[i].sharedMaterial != gateMat)
                    _gateRingRenderers[i].sharedMaterial = gateMat;
            }
        }

        private static GameObject CreateDockBox(Transform parent, string name, Vector3 localPos, Vector3 scale, Material mat)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Cube);
            go.name = name;
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localScale = scale;
            var c = go.GetComponent<Collider>();
            if (c != null)
            {
                if (Application.isPlaying) Destroy(c);
                else DestroyImmediate(c);
            }
            go.GetComponent<MeshRenderer>().sharedMaterial = mat;
            return go;
        }
    }
}
