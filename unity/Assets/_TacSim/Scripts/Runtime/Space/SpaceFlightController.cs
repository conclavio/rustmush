using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.World;
using TMPro;
using UnityEngine;
using UnityEngine.EventSystems;

namespace TacSim.Space
{
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

        public float ShipYawDeg { get; private set; }
        public float ShipPitchDeg { get; private set; }
        public float ShipRollDeg { get; private set; }
        public float ThrottlePermille { get; private set; }
        public string EmconPosture { get; private set; } = "standard";
        public string SelectedRockId { get; private set; } = string.Empty;
        public string LockedContactId { get; private set; } = string.Empty;

        /// <summary>
        /// Fired when the local 60 Hz integrator reaches a terminal contact (`(tapeCommand, isClampCandidate)`).
        /// Final success/failure is decided only when the server replies with `space.docking` (`closed: true, outcome`).
        /// </summary>
        public event Action<string, bool> OnDockingTapeReadyToSubmit;
        public event Action OnDockingAborted;
        public event Action<string> OnDockingEndedByServer;
        public event Action<string> OnSpaceAlert;
        public event Action<string, bool> OnOrbitalTargetClicked; // (designator, isAsteroid)

        private Transform _ceresBody;
        private Transform _mainDrivePlume;
        private Transform _particlePlumeInstance;
        private Transform _contactsContainer;
        private Transform _dockingCollarTarget;
        private readonly List<MeshRenderer> _gateRingRenderers = new List<MeshRenderer>();
        private readonly Dictionary<string, Transform> _namedOrbitalTargets = new Dictionary<string, Transform>(StringComparer.OrdinalIgnoreCase);
        private readonly HashSet<string> _knownRockIds = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        private readonly List<Transform> _collectorDrones = new List<Transform>();

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

        public bool HasTargetableRock(string rockId) => !string.IsNullOrEmpty(rockId) && _knownRockIds.Contains(rockId);
        public bool HasPerceivedContact(string designator) =>
            !string.IsNullOrEmpty(designator) && _namedOrbitalTargets.ContainsKey(designator) && !_knownRockIds.Contains(designator);

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
            _knownRockIds.Clear();
            _collectorDrones.Clear();
            SelectedRockId = string.Empty;
            LockedContactId = string.Empty;
            _droneSortieActive = false;

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
        }

        /// <summary>
        /// Reconciles 3D orbital contacts and asteroid cluster rocks strictly from authoritative `space.holotank` JSON.
        /// </summary>
        public void ApplyHolotankFeed(JsonNode holotank)
        {
            if (holotank == null || !holotank.IsObject || _contactsContainer == null)
                return;

            for (int i = _contactsContainer.childCount - 1; i >= 0; i--)
            {
                var ch = _contactsContainer.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _namedOrbitalTargets.Clear();
            _knownRockIds.Clear();

            Material rockMat = UrpMaterialFactory.GetOpaque("AsteroidOre", new Color(0.32f, 0.28f, 0.24f), 0.15f, 0.2f);
            Material veinMat = UrpMaterialFactory.GetEmissive("OreVein", new Color(0.22f, 0.72f, 0.85f), new Color(0.25f, 0.95f, 1.0f) * 1.4f);

            var rocksArr = holotank["rocks"].AsArray() ?? holotank["cluster"]?["rocks"].AsArray();
            if (rocksArr != null)
            {
                for (int r = 0; r < rocksArr.Count; r++)
                {
                    var rk = rocksArr[r];
                    if (!rk.IsObject) continue;
                    string rId = rk.Str("id", rk.Str("letter", string.Empty)).Trim().ToUpperInvariant();
                    if (string.IsNullOrEmpty(rId)) continue;
                    string spectral = rk.Str("spectral", rk.Str("label", "Pocket"));
                    Vector3 offset = ParseRelativeOffset(rk, defaultRadius: 48f, index: r);
                    SpawnOreRockTarget(rId, $"ROCK {rId} [{spectral}]", OrbitalStageOrigin + offset, r, rockMat, veinMat);
                }
            }

            var contactsArr = holotank["contacts"].AsArray() ?? holotank["blips"].AsArray();
            if (contactsArr != null)
            {
                for (int i = 0; i < contactsArr.Count; i++)
                {
                    var c = contactsArr[i];
                    if (!c.IsObject) continue;
                    string kind = c.Str("kind", string.Empty);
                    string id = c.Str("designator", c.Str("id", string.Empty)).Trim();
                    if (string.IsNullOrEmpty(id)) continue;

                    if (string.Equals(kind, "rock", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(kind, "asteroid", StringComparison.OrdinalIgnoreCase))
                    {
                        string lbl = c.Str("label", c.Str("name", $"ROCK {id}"));
                        Vector3 offset = ParseRelativeOffset(c, defaultRadius: 50f, index: i);
                        SpawnOreRockTarget(id.ToUpperInvariant(), lbl, OrbitalStageOrigin + offset, i, rockMat, veinMat);
                        continue;
                    }

                    string relation = c.Str("relation", c.Str("iff", "neutral"));
                    bool hostile = relation.IndexOf("hostile", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                   relation.IndexOf("enemy", StringComparison.OrdinalIgnoreCase) >= 0;
                    string name = c.Str("name", c.Str("label", id));
                    Vector3 pos = OrbitalStageOrigin + ParseRelativeOffset(c, defaultRadius: 85f, index: i);
                    Color col = hostile ? new Color(1f, 0.42f, 0.36f) : new Color(0.45f, 0.82f, 1f);
                    SpawnNavalContact(id, $"{id} — {name}", pos, Quaternion.Euler(0f, 180f, 0f), col, hostile);
                }
            }

            if (!string.IsNullOrEmpty(SelectedRockId) && !_knownRockIds.Contains(SelectedRockId))
                SelectedRockId = string.Empty;
            if (string.IsNullOrEmpty(SelectedRockId))
            {
                foreach (string rid in _knownRockIds) { SelectedRockId = rid; break; }
            }

            if (!string.IsNullOrEmpty(LockedContactId) && !HasPerceivedContact(LockedContactId))
                LockedContactId = string.Empty;
        }

        private static Vector3 ParseRelativeOffset(JsonNode node, float defaultRadius, int index)
        {
            var rel = node["rel_pos_m"].AsArray() ?? node["pos_m"].AsArray();
            if (rel != null && rel.Count >= 3)
            {
                Vector3 raw = new Vector3(rel[0].AsFloat(0f), rel[2].AsFloat(0f), rel[1].AsFloat(0f)) * 0.04f;
                float mag = raw.magnitude;
                if (mag > 0.1f)
                    return raw.normalized * Mathf.Clamp(mag, 26f, 140f);
            }
            float bearingDeg = node.Float("bearing_deg", index * 55f);
            float rangeM = node.Float("range_m", defaultRadius * 20f);
            float scaledDist = Mathf.Clamp(rangeM * 0.04f, 28f, 135f);
            float rad = bearingDeg * Mathf.Deg2Rad;
            return new Vector3(Mathf.Sin(rad) * scaledDist, ((index % 3) - 1) * 8f, Mathf.Cos(rad) * scaledDist);
        }

        private void SpawnOreRockTarget(string id, string label, Vector3 pos, int idx, Material rockMat, Material veinMat)
        {
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

            AddOrbitalTag(pocketRoot.transform, label, new Vector3(0f, 7.2f, 0f), new Color(0.55f, 1f, 0.85f));
            _namedOrbitalTargets[id] = pocketRoot.transform;
            _knownRockIds.Add(id);
        }

        private void SpawnNavalContact(string id, string label, Vector3 pos, Quaternion rot, Color tagColor, bool isHostile)
        {
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
                Material cHull = UrpMaterialFactory.GetOpaque($"ContactHull_{id}", isHostile ? new Color(0.38f, 0.22f, 0.22f) : new Color(0.32f, 0.38f, 0.46f), 0.5f, 0.65f);
                Material cTrim = UrpMaterialFactory.GetOpaque("ContactTrim", new Color(0.18f, 0.20f, 0.24f), 0.5f, 0.7f);
                StationWorldController.BuildSculptedDrayExterior(cRoot.transform, pos, cHull, cTrim, "CutterSilhouette");
            }

            AddOrbitalTag(cRoot.transform, label, new Vector3(0f, 8.5f, 0f), tagColor);
            _namedOrbitalTargets[id] = cRoot.transform;
        }

        private static void AddOrbitalTag(Transform parent, string label, Vector3 localOffset, Color color)
        {
            var bbGo = new GameObject("OrbitalBillboard");
            bbGo.transform.SetParent(parent, false);
            bbGo.transform.localPosition = localOffset;
            var bb = bbGo.AddComponent<WorldBillboard>();
            var txtGo = new GameObject("Label");
            txtGo.transform.SetParent(bbGo.transform, false);
            var tmp = txtGo.AddComponent<TextMeshPro>();
            tmp.text = MushProtocolLiteEscape(label);
            tmp.fontSize = 16f;
            tmp.color = color;
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.NoWrap;
            tmp.rectTransform.sizeDelta = new Vector2(45f, 6f);
            bb.NameplateText = tmp;
        }

        private static string MushProtocolLiteEscape(string s) =>
            string.IsNullOrEmpty(s) ? string.Empty : s.Replace("<", "‹").Replace(">", "›");

        public void SelectRock(string rockId)
        {
            if (!string.IsNullOrWhiteSpace(rockId))
                SelectedRockId = rockId.Trim().ToUpperInvariant();
        }

        public void SetLockedContact(string designator)
        {
            if (!string.IsNullOrWhiteSpace(designator))
                LockedContactId = designator.Trim().ToUpperInvariant();
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
            if (!string.IsNullOrEmpty(id) && _namedOrbitalTargets.TryGetValue(id, out var tr) && tr != null)
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
                mr.sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"GateMat_{g}", new Color(1.0f, 0.65f, 0.15f, 0.16f));
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

        public void ApplyCockpitFeed(JsonNode cockpit)
        {
            if (cockpit == null || !cockpit.IsObject) return;
            ShipYawDeg = cockpit.Float("yaw_deg", ShipYawDeg);
            ShipPitchDeg = cockpit.Float("pitch_deg", ShipPitchDeg);
            ShipRollDeg = cockpit.Float("roll_deg", ShipPitchDeg);
            ShipRollDeg = cockpit.Float("roll_deg", 0f);
            ThrottlePermille = cockpit.Float("thrust_permille", cockpit.Float("throttle_permille", ThrottlePermille));
            EmconPosture = cockpit.Str("emcon", EmconPosture);

            var fireObj = cockpit["fire"];
            if (fireObj.IsObject)
            {
                string serverLock = fireObj.Str("lock", fireObj.Str("locked", string.Empty));
                if (!string.IsNullOrEmpty(serverLock))
                    LockedContactId = serverLock;
            }

            string matchedRock = cockpit.Str("matched_rock", string.Empty);
            if (!string.IsNullOrEmpty(matchedRock))
                SelectedRockId = matchedRock.Trim().ToUpperInvariant();

            int activeDrones = cockpit.Int("active_drones", -1);
            if (activeDrones >= 0)
                ToggleDroneSortie(activeDrones > 0);

            if (PlayerShipTransform != null)
                PlayerShipTransform.rotation = Quaternion.Euler(-ShipPitchDeg, ShipYawDeg, -ShipRollDeg);
            UpdateDrivePlume();
        }

        public void SetLocalHelmPreview(float yawDeg, float pitchDeg, float throttlePermille)
        {
            ShipYawDeg = (yawDeg % 360f + 360f) % 360f;
            ShipPitchDeg = Mathf.Clamp(pitchDeg, -85f, 85f);
            ThrottlePermille = Mathf.Clamp(throttlePermille, 0f, 1000f);
            if (PlayerShipTransform != null)
                PlayerShipTransform.rotation = Quaternion.Euler(-ShipPitchDeg, ShipYawDeg, -ShipRollDeg);
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
            if (string.IsNullOrEmpty(sid)) return;
            if (ActiveDockingSession != null && ActiveDockingSession.SessionId == sid)
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

            if (ActiveDockingSession == null || _tapeSubmittedAwaitingServer)
                return;

            if (Input.GetKeyDown(KeyCode.X))
                _pendingDampenerToggle = true;
            if (Input.GetKeyDown(KeyCode.G))
                GuidanceAssistEnabled = !GuidanceAssistEnabled;
            if (Input.GetKeyDown(KeyCode.Backspace))
                _pendingAbort = true;

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
                    if (tapeCmd.Length > DockingClientSession.MaxSafeCommandChars)
                    {
                        ActiveDockingSession = null;
                        OnSpaceAlert?.Invoke("Docking tape exceeded safe command length — aborting session; re-initiate approach.");
                        OnDockingAborted?.Invoke();
                        return;
                    }
                    _tapeSubmittedAwaitingServer = true;
                    SyncBowDockingCameraPose();
                    OnDockingTapeReadyToSubmit?.Invoke(tapeCmd, true);
                    return;
                }

                if (ActiveDockingSession.IsCollarScrapeBounce)
                {
                    string tapeCmd = ActiveDockingSession.FormatTapeCommand();
                    if (tapeCmd.Length > DockingClientSession.MaxSafeCommandChars)
                    {
                        ActiveDockingSession = null;
                        OnDockingAborted?.Invoke();
                        return;
                    }
                    _tapeSubmittedAwaitingServer = true;
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
                                bool isRock = _knownRockIds.Contains(kv.Key);
                                if (isRock) SelectedRockId = kv.Key;
                                else LockedContactId = kv.Key;
                                OnOrbitalTargetClicked?.Invoke(kv.Key, isRock);
                                break;
                            }
                        }
                    }
                }
            }
        }

        private DockingControlInput SamplePilotDockingInput()
        {
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
            Color ringCol = gates.AllGreen
                ? new Color(0.2f, 1.0f, 0.55f, 0.34f)
                : (gates.LateralOk && gates.AttitudeOk && gates.RollOk
                    ? new Color(0.35f, 0.88f, 1.0f, 0.26f)
                    : new Color(1.0f, 0.58f, 0.16f, 0.18f));

            for (int i = 0; i < _gateRingRenderers.Count; i++)
            {
                if (_gateRingRenderers[i] != null)
                    _gateRingRenderers[i].sharedMaterial.color = ringCol;
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
