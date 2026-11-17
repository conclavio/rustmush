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
    /// Renders 3D Orbital Space Flight (player ship hull + fusion plume, rotating Ceres globe,
    /// targetable Zero-G Ore Asteroids &amp; Naval Contacts, 3D Mining Laser &amp; Collector Drones,
    /// Space Tactical Beam/Railgun/Torpedo FX) AND drives the 60 Hz 6-DOF Bow Docking Collar Mini-Game.
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
        public string SelectedRockId { get; private set; } = "A";
        public string LockedContactId { get; private set; } = "C1";

        public event Action<string> OnDockingTapeReadyToSubmit;
        public event Action OnDockingAborted;
        public event Action<string> OnSpaceAlert;
        public event Action<string, bool> OnOrbitalTargetClicked; // (designator, isAsteroid)

        private Transform _ceresBody;
        private Transform _mainDrivePlume;
        private Transform _particlePlumeInstance;
        private Transform _contactsContainer;
        private Transform _dockingCollarTarget;
        private readonly List<MeshRenderer> _gateRingRenderers = new List<MeshRenderer>();
        private readonly Dictionary<string, Transform> _namedOrbitalTargets = new Dictionary<string, Transform>(StringComparer.OrdinalIgnoreCase);
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
        private bool _clampSubmitted;

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

            // Fusion torch plume + commercial thruster VFX particle emitter
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

            // 2. Ceres Dwarf Planet & Sunlight
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

            _contactsContainer = new GameObject("OrbitalContacts").transform;
            _contactsContainer.SetParent(transform, false);
            BuildAmbientStarfieldAndAsteroids();

            // 3. Dedicated 60 Hz 6-DOF Bow Docking Collar Stage
            BuildBowDockingRig();
        }

        private void BuildAmbientStarfieldAndAsteroids()
        {
            _namedOrbitalTargets.Clear();
            _collectorDrones.Clear();

            Material rockMat = UrpMaterialFactory.GetOpaque("AsteroidOre", new Color(0.32f, 0.28f, 0.24f), 0.15f, 0.2f);
            Material veinMat = UrpMaterialFactory.GetEmissive("OreVein", new Color(0.22f, 0.72f, 0.85f), new Color(0.25f, 0.95f, 1.0f) * 1.4f);

            // 1. Three named Zero-G Prospecting Ore Pockets (A, B, C) within visual mining range!
            (string id, string label, Vector3 offset)[] oreRocks =
            {
                ("A", "ROCK A [Rich Ice/Volatiles]", new Vector3(-34f, 4f, 44f)),
                ("B", "ROCK B [Metal Concentrate]",  new Vector3(38f, -6f, 52f)),
                ("C", "ROCK C [Carbonaceous]",       new Vector3(6f, 12f, 68f))
            };

            for (int r = 0; r < oreRocks.Length; r++)
            {
                Vector3 pos = OrbitalStageOrigin + oreRocks[r].offset;
                var pocketRoot = new GameObject($"OreRock_{oreRocks[r].id}");
                pocketRoot.transform.SetParent(_contactsContainer, false);
                pocketRoot.transform.position = pos;

                var sc = pocketRoot.AddComponent<SphereCollider>();
                sc.radius = 6.5f;

                List<GameObject> orePool = (Catalog != null && Catalog.oreAsteroidPrefabs != null && Catalog.oreAsteroidPrefabs.Count > 0)
                    ? Catalog.oreAsteroidPrefabs
                    : (Catalog != null ? Catalog.asteroidPrefabs : null);

                if (orePool != null && orePool.Count > 0 && orePool[r % orePool.Count] != null)
                {
                    PrefabFittingHelper.SpawnCentered(orePool[r % orePool.Count], pocketRoot.transform, "OreMesh", pos, Quaternion.Euler(r * 35f, r * 62f, 0f), 11f, keepColliders: false);
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

                AddOrbitalTag(pocketRoot.transform, oreRocks[r].label, new Vector3(0f, 7.2f, 0f), new Color(0.55f, 1f, 0.85f));
                _namedOrbitalTargets[oreRocks[r].id] = pocketRoot.transform;
            }

            // 2. Named Naval Contacts: Customs Patrol Corvette (C1) & Belt Raider Contact (X1)
            SpawnNavalContact("C1", "C1 — Patrol Corvette", OrbitalStageOrigin + new Vector3(-62f, 14f, 92f), Quaternion.Euler(0f, 145f, 0f), new Color(0.45f, 0.82f, 1f), isHostile: false);
            SpawnNavalContact("X1", "X1 — Unmarked Raider", OrbitalStageOrigin + new Vector3(72f, -12f, 108f), Quaternion.Euler(0f, -145f, 0f), new Color(1f, 0.42f, 0.36f), isHostile: true);

            // 3. Two Collector Drones parked beside player's spine ready for +mine/drones sorties
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
                if (droneGo != null) _collectorDrones.Add(droneGo.transform);
            }

            // 4. Outer Belt Asteroid Ring
            for (int i = 0; i < 8; i++)
            {
                float ang = i * 0.785f;
                float dist = 125f + (i % 3) * 42f;
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
                    var rc = rock.GetComponent<Collider>();
                    if (rc != null) { if (Application.isPlaying) Destroy(rc); else DestroyImmediate(rc); }
                    rock.GetComponent<MeshRenderer>().sharedMaterial = rockMat;
                }
                rock.name = $"BeltRock_{i}";
            }

            // 5. Reusable 3D Beam Renderer & Torpedo Visual for Mining Laser & Naval Fire Control
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
            tmp.text = label;
            tmp.fontSize = 16f;
            tmp.color = color;
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.NoWrap;
            tmp.rectTransform.sizeDelta = new Vector2(45f, 6f);
            bb.NameplateText = tmp;
        }

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
            Transform target = ResolveOrbitalTarget(SelectedRockId);
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
        }

        public void TriggerNavalWeaponVfx(string weaponKind, string targetDesignator = null)
        {
            string tgtKey = !string.IsNullOrWhiteSpace(targetDesignator) ? targetDesignator : LockedContactId;
            Transform target = ResolveOrbitalTarget(tgtKey);
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

        private Transform ResolveOrbitalTarget(string id)
        {
            if (!string.IsNullOrEmpty(id) && _namedOrbitalTargets.TryGetValue(id, out var tr) && tr != null)
                return tr;
            foreach (var kv in _namedOrbitalTargets)
            {
                if (kv.Value != null) return kv.Value;
            }
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

            // Station cradle gantry backdrop surrounding target collar
            if (Catalog != null && Catalog.shipyardCradlePrefab != null)
            {
                PrefabFittingHelper.SpawnCentered(Catalog.shipyardCradlePrefab, dockRoot, "CeresDockingCradleBackdrop", DockingStageOrigin + new Vector3(0f, 0f, 14f), Quaternion.identity, 28f, keepColliders: false);
            }

            // Station cradle bulkhead surrounding target collar at Z = 0, facing -Z (towards approaching ship)
            CreateDockBox(_dockingCollarTarget, "StationBulkhead_Top", new Vector3(0f, 2.2f, 0.4f), new Vector3(8f, 2.4f, 0.8f), portBulkhead);
            CreateDockBox(_dockingCollarTarget, "StationBulkhead_Bot", new Vector3(0f, -2.2f, 0.4f), new Vector3(8f, 2.4f, 0.8f), portBulkhead);
            CreateDockBox(_dockingCollarTarget, "StationBulkhead_L", new Vector3(-2.6f, 0f, 0.4f), new Vector3(3.2f, 2.0f, 0.8f), portBulkhead);
            CreateDockBox(_dockingCollarTarget, "StationBulkhead_R", new Vector3(2.6f, 0f, 0.4f), new Vector3(3.2f, 2.0f, 0.8f), portBulkhead);

            // Octagonal collar ring & roll-index top fin (so pilot sees roll alignment clearly!)
            for (int i = 0; i < 8; i++)
            {
                float a = i * 45f * Mathf.Deg2Rad;
                var seg = CreateDockBox(_dockingCollarTarget, $"CollarClamp_{i}", new Vector3(Mathf.Cos(a) * 0.95f, Mathf.Sin(a) * 0.95f, 0f), new Vector3(0.32f, 0.78f, 0.35f), collarRim);
                seg.transform.localRotation = Quaternion.Euler(0f, 0f, i * 45f);
            }
            CreateDockBox(_dockingCollarTarget, "TopRollIndexChevron", new Vector3(0f, 1.18f, -0.1f), new Vector3(0.14f, 0.42f, 0.14f), crossMat);
            // Inner standoff crosshairs at Z = +0.6m behind collar for true 3D parallax attitude cue!
            CreateDockBox(_dockingCollarTarget, "StandoffCross_H", new Vector3(0f, 0f, 0.65f), new Vector3(0.75f, 0.04f, 0.04f), crossMat);
            CreateDockBox(_dockingCollarTarget, "StandoffCross_V", new Vector3(0f, 0f, 0.65f), new Vector3(0.04f, 0.75f, 0.04f), crossMat);

            // 5 Holographic Approach Tunnel Gates along -Z (0.5m, 1.0m, 1.5m, 2.0m, 2.5m)
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
            ShipRollDeg = cockpit.Float("roll_deg", ShipRollDeg);
            ThrottlePermille = cockpit.Float("thrust_permille", cockpit.Float("throttle_permille", ThrottlePermille));
            EmconPosture = cockpit.Str("emcon", EmconPosture);

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
            if (dockingFeed == null || !dockingFeed.IsObject || dockingFeed.Bool("closed", false))
            {
                ActiveDockingSession = null;
                return;
            }
            string sid = dockingFeed.Str("session_id", string.Empty);
            if (string.IsNullOrEmpty(sid)) return;
            if (ActiveDockingSession != null && ActiveDockingSession.SessionId == sid)
                return; // Keep stepping local 60 Hz session uninterrupted!

            ActiveDockingSession = DockingClientSession.FromFeed(dockingFeed);
            _dockingAccumulatorS = 0f;
            _clampSubmitted = false;
            GuidanceAssistEnabled = false;
            SyncBowDockingCameraPose();
        }

        public void StopDockingSession()
        {
            ActiveDockingSession = null;
            _clampSubmitted = false;
        }

        private void Update()
        {
            if (_ceresBody != null)
                _ceresBody.Rotate(Vector3.up, Time.deltaTime * 1.4f, UnityEngine.Space.World);

            UpdateOrbitalMiningAndWeaponAnimations();

            if (ActiveDockingSession == null || _clampSubmitted)
                return;

            if (Input.GetKeyDown(KeyCode.X))
                _pendingDampenerToggle = true;
            if (Input.GetKeyDown(KeyCode.G))
                GuidanceAssistEnabled = !GuidanceAssistEnabled;
            if (Input.GetKeyDown(KeyCode.Backspace))
                _pendingAbort = true;

            _dockingAccumulatorS += Mathf.Min(0.1f, Time.unscaledDeltaTime);
            const float StepDt = 1f / 60f;

            while (_dockingAccumulatorS >= StepDt && ActiveDockingSession != null && !_clampSubmitted)
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
                    _clampSubmitted = true;
                    SyncBowDockingCameraPose();
                    string tapeCmd = ActiveDockingSession.FormatTapeCommand();
                    OnDockingTapeReadyToSubmit?.Invoke(tapeCmd);
                    return;
                }

                if (ActiveDockingSession.IsCollarScrapeBounce)
                {
                    _clampSubmitted = true;
                    SyncBowDockingCameraPose();
                    OnSpaceAlert?.Invoke($"COLLAR SCRAPE BOUNCE at {ActiveDockingSession.VzMmps} mm/s! Submitting tape...");
                    OnDockingTapeReadyToSubmit?.Invoke(ActiveDockingSession.FormatTapeCommand());
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

            // Collector drone shuttle loop between Dray spine and SelectedRockId
            if (_collectorDrones.Count > 0 && PlayerShipTransform != null)
            {
                Transform rock = ResolveOrbitalTarget(SelectedRockId);
                for (int i = 0; i < _collectorDrones.Count; i++)
                {
                    var d = _collectorDrones[i];
                    if (d == null) continue;
                    if (_droneSortieActive && rock != null)
                    {
                        float phase = Mathf.PingPong(Time.time * 0.35f + i * 0.5f, 1f);
                        Vector3 shipBay = PlayerShipTransform.position + new Vector3(i == 0 ? -6.5f : 6.5f, 2f, 0f);
                        Vector3 rockFace = rock.position + new Vector3(i == 0 ? -4.5f : 4.5f, 2.5f, -4.5f);
                        Vector3 nextPos = Vector3.Lerp(shipBay, rockFace, phase) + Vector3.up * (Mathf.Sin(Time.time * 2.2f + i) * 1.2f);
                        Vector3 vel = nextPos - d.position;
                        d.position = nextPos;
                        if (vel.sqrMagnitude > 0.0001f)
                            d.rotation = Quaternion.Slerp(d.rotation, Quaternion.LookRotation(vel.normalized), 8f * Time.deltaTime);
                    }
                    else
                    {
                        Vector3 home = PlayerShipTransform.position + new Vector3(i == 0 ? -7.5f : 7.5f, 2.2f + Mathf.Sin(Time.time * 1.4f + i) * 0.35f, 2f);
                        d.position = Vector3.Lerp(d.position, home, 5f * Time.deltaTime);
                    }
                }
            }

            // Click-to-select 3D Ore Asteroid or Naval Contact in Orbital Exterior view
            if (ActiveDockingSession == null && Input.GetMouseButtonDown(0))
            {
                bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
                Camera cam = Camera.main;
                if (!overUi && cam != null && Vector3.Distance(cam.transform.position, OrbitalStageOrigin) < 350f)
                {
                    Ray ray = cam.ScreenPointToRay(Input.mousePosition);
                    if (Physics.Raycast(ray, out RaycastHit hit, 500f))
                    {
                        string hitName = hit.collider.gameObject.name;
                        if (hitName.StartsWith("OreRock_", StringComparison.OrdinalIgnoreCase))
                        {
                            string id = hitName.Substring("OreRock_".Length);
                            SelectRock(id);
                            OnOrbitalTargetClicked?.Invoke(id, true);
                        }
                        else if (hitName.StartsWith("NavalContact_", StringComparison.OrdinalIgnoreCase))
                        {
                            string id = hitName.Substring("NavalContact_".Length);
                            SetLockedContact(id);
                            OnOrbitalTargetClicked?.Invoke(id, false);
                        }
                    }
                }
            }
        }

        private static DockingControlInput SamplePilotDockingInput()
        {
            var inp = new DockingControlInput();
            if (Input.GetKey(KeyCode.D)) inp.TransX = 1;
            else if (Input.GetKey(KeyCode.A)) inp.TransX = -1;

            if (Input.GetKey(KeyCode.W)) inp.TransY = 1;
            else if (Input.GetKey(KeyCode.S)) inp.TransY = -1;

            if (Input.GetKey(KeyCode.R) || Input.GetKey(KeyCode.Space)) inp.ClosurePulse = 1;
            else if (Input.GetKey(KeyCode.F) || Input.GetKey(KeyCode.LeftControl)) inp.ClosurePulse = -1;

            if (Input.GetKey(KeyCode.UpArrow) || Input.GetKey(KeyCode.I)) inp.PitchTrim = 1;
            else if (Input.GetKey(KeyCode.DownArrow) || Input.GetKey(KeyCode.K)) inp.PitchTrim = -1;

            if (Input.GetKey(KeyCode.RightArrow) || Input.GetKey(KeyCode.L)) inp.YawTrim = 1;
            else if (Input.GetKey(KeyCode.LeftArrow) || Input.GetKey(KeyCode.J)) inp.YawTrim = -1;

            if (Input.GetKey(KeyCode.E)) inp.RollTrim = 1;
            else if (Input.GetKey(KeyCode.Q)) inp.RollTrim = -1;

            return inp;
        }

        private void SyncBowDockingCameraPose()
        {
            if (ActiveDockingSession == null || BowDockingCameraAnchor == null)
                return;

            // 1000 mm = 1.0 Unity metre; collar is at local (0, 0, 0) facing -Z; ship approaches along -Z -> 0
            float xM = ActiveDockingSession.DxMm / 1000f;
            float yM = ActiveDockingSession.DyMm / 1000f;
            float zM = -Mathf.Max(0.18f, ActiveDockingSession.DzMm / 1000f);

            float pitchDeg = ActiveDockingSession.PitchMdeg / 1000f;
            float yawDeg = ActiveDockingSession.YawMdeg / 1000f;
            float rollDeg = ActiveDockingSession.RollMdeg / 1000f;

            BowDockingCameraAnchor.localPosition = new Vector3(xM, yM, zM);
            BowDockingCameraAnchor.localRotation = Quaternion.Euler(-pitchDeg, yawDeg, -rollDeg);

            var gates = ActiveDockingSession.EvaluateGates();
            bool[] flags = { gates.LateralOk, gates.AttitudeOk, gates.RollOk, gates.ClosureOk, gates.RangeOk };
            for (int i = 0; i < _gateRingRenderers.Count && i < flags.Length; i++)
            {
                if (_gateRingRenderers[i] != null)
                {
                    _gateRingRenderers[i].sharedMaterial = flags[i]
                        ? UrpMaterialFactory.GetHologramUnlit("GateGreen", new Color(0.2f, 1.0f, 0.55f, 0.22f))
                        : UrpMaterialFactory.GetHologramUnlit("GateAmber", new Color(1.0f, 0.62f, 0.15f, 0.16f));
                }
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
            if (c != null) { if (Application.isPlaying) Destroy(c); else DestroyImmediate(c); }
            go.GetComponent<MeshRenderer>().sharedMaterial = mat;
            return go;
        }
    }
}
