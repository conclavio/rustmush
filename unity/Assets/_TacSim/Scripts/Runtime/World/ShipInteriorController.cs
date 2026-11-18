using System;
using System.Collections.Generic;
using TacSim.Character;
using TacSim.Core;
using TacSim.Net;
using TMPro;
using UnityEngine;

namespace TacSim.World
{
    /// <summary>
    /// Builds and drives the walkable multi-deck ship interior
    /// (Airlock, Central Passage, Bridge, Crew Compartment, Cargo Hold, Engineering Access, and Dray-P Vehicle Bay)
    /// from crates/tacsim/content/units/*.json, with stairwell cutouts, normal-mapped URP bulkheads/decks/ceilings,
    /// panoramic Bridge canopy, live 3D Holo-Tank contacts, co-op crewmate avatars, and the Bridge Helm Couch.
    /// </summary>
    public sealed class ShipInteriorController : MonoBehaviour
    {
        public static readonly Vector3 ShipInteriorWorldOrigin = new Vector3(0f, 160f, 0f);

        public TacSimAssetCatalog Catalog;
        public ShipUnitTemplate Template { get; private set; }
        public string LoadedUnitId { get; private set; } = string.Empty;
        public Transform BridgeCameraAnchor { get; private set; }
        public Transform HelmSeatStandSpot { get; private set; }
        public string ActiveCompartmentId { get; private set; } = "airlock";

        public event Action<WorldInteractable> OnInteractableTriggered;
        public event Action<ExitPortalTrigger, string, string> OnPortalCrossed;

        private readonly Dictionary<string, TextMeshPro> _gaugeDisplays = new Dictionary<string, TextMeshPro>(StringComparer.Ordinal);
        private readonly Dictionary<string, GameObject> _crewActors = new Dictionary<string, GameObject>(StringComparer.Ordinal);
        private readonly Dictionary<string, Vector3> _crewTargetPositions = new Dictionary<string, Vector3>(StringComparer.Ordinal);
        private readonly Dictionary<string, WorldBillboard> _crewBillboards = new Dictionary<string, WorldBillboard>(StringComparer.Ordinal);
        private Transform _crewActorsContainer;
        private string _crewActorsCompartmentId;
        private Transform _holotankBlipsRoot;
        private Transform _cargoPalletsRoot;
        private WorldInteractable _airlockLeaveInteractable;
        private float _holoSpinDeg;

        // Companionway stairwell openings in ship local X/Z so climbing between Passage (Y=0),
        // Bridge (Y=+3.4m), and Cargo Hold (Y=-3.6m) has full head & capsule clearance.
        private static readonly Rect BridgeStairwellXZ = new Rect(-2.45f, 2.15f, 1.70f, 4.05f); // X [-2.45, -0.75], Z [2.15, 6.20]
        private static readonly Rect HoldStairwellXZ = new Rect(0.75f, -0.10f, 1.70f, 3.60f);   // X [0.75, 2.45],  Z [-0.10, 3.50]

        private bool TryGetCompartmentDef(string compartmentId, out ShipCompartmentDef comp)
        {
            comp = null;
            if (Template == null) return false;
            string key = string.IsNullOrEmpty(compartmentId) ? "airlock" : compartmentId;
            if (Template.Compartments.TryGetValue(key, out comp))
                return true;
            if (string.Equals(key, "corridor", StringComparison.OrdinalIgnoreCase) &&
                Template.Compartments.TryGetValue("passage", out comp))
                return true;
            if (string.Equals(key, "passage", StringComparison.OrdinalIgnoreCase) &&
                Template.Compartments.TryGetValue("corridor", out comp))
                return true;
            return false;
        }

        public Vector3 GetCompartmentSpawnWorldPos(string compartmentId)
        {
            if (TryGetCompartmentDef(compartmentId, out var comp))
            {
                return ShipInteriorWorldOrigin + new Vector3(comp.LocalCenter.x, comp.LocalCenter.y - comp.LocalSize.y * 0.5f + 0.2f, comp.LocalCenter.z);
            }
            return ShipInteriorWorldOrigin + new Vector3(0f, -1.4f, 13f);
        }

        public string TryFindContainingCompartment(Vector3 worldPos, string currentCompId, float hysteresisM = 0.35f)
        {
            if (Template == null) return ActiveCompartmentId;
            Vector3 lp = worldPos - ShipInteriorWorldOrigin;

            if (!string.IsNullOrEmpty(currentCompId) && hysteresisM > 0f &&
                TryGetCompartmentDef(currentCompId, out var cur))
            {
                Vector3 half = cur.LocalSize * 0.5f;
                if (Mathf.Abs(lp.x - cur.LocalCenter.x) <= half.x + hysteresisM &&
                    Mathf.Abs(lp.y - cur.LocalCenter.y) <= half.y + 0.9f &&
                    Mathf.Abs(lp.z - cur.LocalCenter.z) <= half.z + hysteresisM)
                {
                    return currentCompId;
                }
            }

            foreach (var kv in Template.Compartments)
            {
                var c = kv.Value;
                Vector3 half = c.LocalSize * 0.5f;
                if (Mathf.Abs(lp.x - c.LocalCenter.x) <= half.x &&
                    Mathf.Abs(lp.y - c.LocalCenter.y) <= half.y + 0.9f &&
                    Mathf.Abs(lp.z - c.LocalCenter.z) <= half.z)
                {
                    return kv.Key;
                }
            }
            return currentCompId ?? ActiveCompartmentId;
        }

        public List<(string command, string targetCompartment)> FindExitPath(string fromComp, string toComp)
        {
            if (Template == null) return new List<(string, string)>();
            string normFrom = TryGetCompartmentDef(fromComp, out var cFrom) ? cFrom.Id : fromComp;
            string normTo = TryGetCompartmentDef(toComp, out var cTo) ? cTo.Id : toComp;
            return Template.FindExitPath(normFrom, normTo);
        }

        public void BuildInterior(TacSimAssetCatalog catalog, string unitId = "tacsim.unit.dray")
        {
            Catalog = catalog;
            Template = TacSimContentLoader.LoadShipTemplate(unitId);
            LoadedUnitId = Template.UnitId;

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _gaugeDisplays.Clear();
            _crewActors.Clear();
            _crewTargetPositions.Clear();
            _crewBillboards.Clear();
            _crewActorsCompartmentId = null;

            transform.position = ShipInteriorWorldOrigin;

            _crewActorsContainer = new GameObject("LiveShipCrewActors").transform;
            _crewActorsContainer.SetParent(transform, false);

            Material rampMat = UrpMaterialFactory.GetTiledPbrSurface(
                "ShipLadderRamp", null, null, null,
                new Color(0.26f, 0.30f, 0.35f), new Vector2(1f, 1f),
                isFloor: true, smoothness: 0.32f, metallic: 0.55f);
            Material railMat = UrpMaterialFactory.GetOpaque("ShipHandrail", new Color(0.38f, 0.43f, 0.49f), 0.52f, 0.72f);

            foreach (var kv in Template.Compartments)
            {
                var comp = kv.Value;
                var cRoot = new GameObject($"Compartment_{comp.Id}").transform;
                cRoot.SetParent(transform, false);

                float floorY = comp.LocalCenter.y - comp.LocalSize.y * 0.5f;
                float ceilY = comp.LocalCenter.y + comp.LocalSize.y * 0.5f;
                float minX = comp.LocalCenter.x - comp.LocalSize.x * 0.5f;
                float maxX = comp.LocalCenter.x + comp.LocalSize.x * 0.5f;
                float minZ = comp.LocalCenter.z - comp.LocalSize.z * 0.5f;
                float maxZ = comp.LocalCenter.z + comp.LocalSize.z * 0.5f;

                Color deckTint = comp.Id == "hold" || comp.Id == "bay"
                    ? new Color(0.16f, 0.20f, 0.27f)
                    : new Color(0.20f, 0.24f, 0.29f);
                Material flMat = UrpMaterialFactory.GetTiledPbrSurface(
                    $"ShipDeck_{comp.Id}", null, null, null, deckTint,
                    new Vector2(Mathf.Max(1f, Mathf.Round(comp.LocalSize.x / 2.6f)), Mathf.Max(1f, Mathf.Round(comp.LocalSize.z / 2.6f))),
                    isFloor: true, smoothness: 0.36f, metallic: 0.48f);

                Material sideBulkheadMat = UrpMaterialFactory.GetTiledPbrSurface(
                    $"ShipSideBulkhead_{comp.Id}", null, null, null, new Color(0.26f, 0.30f, 0.35f),
                    new Vector2(Mathf.Max(1f, Mathf.Round(comp.LocalSize.z / 3.8f)), 1f),
                    isFloor: false, smoothness: 0.30f, metallic: 0.38f);

                Material endBulkheadMat = UrpMaterialFactory.GetTiledPbrSurface(
                    $"ShipEndBulkhead_{comp.Id}", null, null, null, new Color(0.26f, 0.30f, 0.35f),
                    new Vector2(Mathf.Max(1f, Mathf.Round(comp.LocalSize.x / 3.8f)), 1f),
                    isFloor: false, smoothness: 0.30f, metallic: 0.38f);

                // Deck plate with companionway stairwell cutouts (Passage -> Hold down-well; Bridge <- Passage up-well)
                Rect? floorCutout = null;
                if (comp.Id == "passage" || comp.Id == "corridor")
                    floorCutout = new Rect(HoldStairwellXZ.xMin, HoldStairwellXZ.yMin, HoldStairwellXZ.width, HoldStairwellXZ.height - 0.06f);
                else if (comp.Id == "bridge")
                    floorCutout = new Rect(BridgeStairwellXZ.xMin, minZ, BridgeStairwellXZ.width, (BridgeStairwellXZ.yMax - 0.06f) - minZ);

                CreateHorizontalSlabWithCutout(cRoot, "DeckPlate", floorY - 0.12f, 0.24f, minX, maxX, minZ, maxZ, floorCutout, flMat, collider: true);

                // Matte coffered compartment ceiling with companionway head-clearance cutouts
                Rect? ceilCutout = null;
                if (comp.Id == "passage" || comp.Id == "corridor")
                    ceilCutout = BridgeStairwellXZ;
                else if (comp.Id == "hold")
                    ceilCutout = HoldStairwellXZ;

                Material ceilMat = UrpMaterialFactory.GetStationCeilingMaterial(
                    $"ShipCeil_{comp.Id}", new Color(0.30f, 0.34f, 0.40f),
                    new Vector2(Mathf.Max(1f, Mathf.Round(comp.LocalSize.x / 3.2f)), Mathf.Max(1f, Mathf.Round(comp.LocalSize.z / 3.2f))));
                CreateHorizontalSlabWithCutout(cRoot, "CompartmentCeiling", ceilY + 0.06f, 0.12f, minX, maxX, minZ, maxZ, ceilCutout, ceilMat, collider: false);

                // Port & Starboard bulkheads
                CreateLocalBox(cRoot, "PortBulkhead", new Vector3(minX, comp.LocalCenter.y, comp.LocalCenter.z), new Vector3(0.22f, comp.LocalSize.y, comp.LocalSize.z), sideBulkheadMat, true);
                CreateLocalBox(cRoot, "StarboardBulkhead", new Vector3(maxX, comp.LocalCenter.y, comp.LocalCenter.z), new Vector3(0.22f, comp.LocalSize.y, comp.LocalSize.z), sideBulkheadMat, true);

                // Outer endcaps & panoramic Bridge forward canopy
                if (comp.Id == "airlock")
                {
                    CreateLocalBox(cRoot, "ForwardBulkhead", new Vector3(comp.LocalCenter.x, comp.LocalCenter.y, maxZ), new Vector3(comp.LocalSize.x, comp.LocalSize.y, 0.25f), endBulkheadMat, true);
                }
                else if (comp.Id == "bridge")
                {
                    BuildBridgeForwardCanopy(cRoot, comp, floorY, ceilY, maxZ, endBulkheadMat, railMat);
                    CreateLocalBox(cRoot, "AftBulkhead", new Vector3(comp.LocalCenter.x, comp.LocalCenter.y, minZ), new Vector3(comp.LocalSize.x, comp.LocalSize.y, 0.22f), endBulkheadMat, false);
                }
                else if (comp.Id == "hold")
                {
                    CreateLocalBox(cRoot, "ForwardBulkhead", new Vector3(comp.LocalCenter.x, comp.LocalCenter.y, maxZ), new Vector3(comp.LocalSize.x, comp.LocalSize.y, 0.25f), endBulkheadMat, true);
                }

                if (comp.Id == "crew" || (comp.Id == "engineering" && !Template.Compartments.ContainsKey("bay")) || comp.Id == "bay")
                {
                    CreateLocalBox(cRoot, "AftBulkhead", new Vector3(comp.LocalCenter.x, comp.LocalCenter.y, minZ), new Vector3(comp.LocalSize.x, comp.LocalSize.y, 0.25f), endBulkheadMat, true);
                }

                // Compartment lighting + recessed luminaire fixture matching dray.json prose
                Color lightCol = comp.Id == "hold" ? new Color(0.45f, 0.72f, 1.0f)
                               : (comp.Id == "crew" || comp.Id == "engineering" ? new Color(1.0f, 0.78f, 0.52f) : new Color(0.85f, 0.94f, 1.0f));

                var bezelMat = UrpMaterialFactory.GetOpaque("ShipLightBezel", new Color(0.12f, 0.14f, 0.18f), 0.35f, 0.55f);
                var diffMat = UrpMaterialFactory.GetEmissive($"ShipLightDiff_{comp.Id}", lightCol * 0.55f, lightCol * 2.0f);
                CreateLocalBox(cRoot, "CeilingFixtureHousing", new Vector3(comp.LocalCenter.x, ceilY - 0.03f, comp.LocalCenter.z), new Vector3(0.68f, 0.06f, 2.05f), bezelMat, false);
                CreateLocalBox(cRoot, "CeilingLightDiffuser", new Vector3(comp.LocalCenter.x, ceilY - 0.06f, comp.LocalCenter.z), new Vector3(0.44f, 0.03f, 1.82f), diffMat, false);

                var ltGo = new GameObject("CompartmentLight");
                ltGo.transform.SetParent(cRoot, false);
                ltGo.transform.localPosition = new Vector3(comp.LocalCenter.x, ceilY - 0.38f, comp.LocalCenter.z);
                var lt = ltGo.AddComponent<Light>();
                lt.type = LightType.Point;
                lt.range = 13f;
                lt.intensity = 2.85f;
                lt.color = lightCol;

                // Uniform-scale GaugeRoot so GaugeText is never stretched by GaugeSlate's (0.08, 1.15, 2.45) box scale!
                var gaugeRoot = new GameObject("GaugeRoot").transform;
                gaugeRoot.SetParent(cRoot, false);
                gaugeRoot.localPosition = new Vector3(minX + 0.16f, floorY + 1.65f, comp.LocalCenter.z);
                gaugeRoot.localScale = Vector3.one;

                CreateLocalBox(gaugeRoot, "GaugeSlate", Vector3.zero, new Vector3(0.06f, 1.15f, 2.45f),
                    UrpMaterialFactory.GetOpaque("Slate", new Color(0.05f, 0.09f, 0.14f), 0.55f, 0.3f), false);
                var tmp = CreateLocalTmp(gaugeRoot, new Vector3(0.045f, 0f, 0f), Quaternion.Euler(0f, -90f, 0f), $"{comp.Name.ToUpperInvariant()}\n<size=70%>{comp.ShortDesc}</size>", 1.42f);
                _gaugeDisplays[comp.Id] = tmp;

                DressShipCompartment(cRoot, comp, floorY);
            }

            // Walkable inclined companionway ramps + safety handrails connecting Passage <-> Bridge (+3.4m) and Passage <-> Hold (-3.6m)
            BuildCompanionwayRamp(transform, new Vector3(-1.6f, -1.6f, 2.5f), new Vector3(-1.6f, 1.8f, 6.2f), 1.6f, rampMat, railMat, "BridgeCompanionway");
            BuildCompanionwayRamp(transform, new Vector3(1.6f, -1.6f, 3.5f), new Vector3(1.6f, -5.3f, 0.2f), 1.6f, rampMat, railMat, "HoldCompanionway");

            // Ship ExitPortalTriggers matching dray.json exits
            BuildShipExitTriggers();

            // Airlock Outer Door (+ship/leave)
            BuildAirlockOuterHatch();

            // Bridge 3D Holo-Tank & Helm Couch
            BuildBridgeHoloAndHelm();
        }

        private static void CreateHorizontalSlabWithCutout(
            Transform parent,
            string baseName,
            float yCenter,
            float thickness,
            float minX,
            float maxX,
            float minZ,
            float maxZ,
            Rect? cutoutXZ,
            Material mat,
            bool collider)
        {
            if (!cutoutXZ.HasValue)
            {
                CreateLocalBox(
                    parent,
                    baseName,
                    new Vector3((minX + maxX) * 0.5f, yCenter, (minZ + maxZ) * 0.5f),
                    new Vector3(maxX - minX, thickness, maxZ - minZ),
                    mat,
                    collider);
                return;
            }

            Rect cut = cutoutXZ.Value;
            float cMinX = Mathf.Clamp(cut.xMin, minX, maxX);
            float cMaxX = Mathf.Clamp(cut.xMax, minX, maxX);
            float cMinZ = Mathf.Clamp(cut.yMin, minZ, maxZ);
            float cMaxZ = Mathf.Clamp(cut.yMax, minZ, maxZ);

            if (cMaxX - cMinX < 0.05f || cMaxZ - cMinZ < 0.05f)
            {
                CreateLocalBox(
                    parent,
                    baseName,
                    new Vector3((minX + maxX) * 0.5f, yCenter, (minZ + maxZ) * 0.5f),
                    new Vector3(maxX - minX, thickness, maxZ - minZ),
                    mat,
                    collider);
                return;
            }

            if (cMinZ - minZ > 0.05f)
            {
                CreateLocalBox(parent, $"{baseName}_Aft",
                    new Vector3((minX + maxX) * 0.5f, yCenter, (minZ + cMinZ) * 0.5f),
                    new Vector3(maxX - minX, thickness, cMinZ - minZ), mat, collider);
            }
            if (maxZ - cMaxZ > 0.05f)
            {
                CreateLocalBox(parent, $"{baseName}_Fwd",
                    new Vector3((minX + maxX) * 0.5f, yCenter, (cMaxZ + maxZ) * 0.5f),
                    new Vector3(maxX - minX, thickness, maxZ - cMaxZ), mat, collider);
            }
            if (cMinX - minX > 0.05f)
            {
                CreateLocalBox(parent, $"{baseName}_Port",
                    new Vector3((minX + cMinX) * 0.5f, yCenter, (cMinZ + cMaxZ) * 0.5f),
                    new Vector3(cMinX - minX, thickness, cMaxZ - cMinZ), mat, collider);
            }
            if (maxX - cMaxX > 0.05f)
            {
                CreateLocalBox(parent, $"{baseName}_Stbd",
                    new Vector3((cMaxX + maxX) * 0.5f, yCenter, (cMinZ + cMaxZ) * 0.5f),
                    new Vector3(maxX - cMaxX, thickness, cMaxZ - cMinZ), mat, collider);
            }
        }

        private static void BuildBridgeForwardCanopy(
            Transform cRoot,
            ShipCompartmentDef comp,
            float floorY,
            float ceilY,
            float maxZ,
            Material bulkheadMat,
            Material frameMat)
        {
            float w = comp.LocalSize.x;
            float h = comp.LocalSize.y;
            float kneeH = 0.92f;
            float headerH = 0.48f;
            float glassH = Mathf.Max(1.0f, h - kneeH - headerH);
            float glassCenterY = floorY + kneeH + glassH * 0.5f;

            // Armored lower coaming, upper brow header, and physics barrier
            CreateLocalBox(cRoot, "BridgeForwardApron", new Vector3(comp.LocalCenter.x, floorY + kneeH * 0.5f, maxZ), new Vector3(w, kneeH, 0.26f), bulkheadMat, false);
            CreateLocalBox(cRoot, "BridgeForwardHeader", new Vector3(comp.LocalCenter.x, ceilY - headerH * 0.5f, maxZ), new Vector3(w, headerH, 0.26f), bulkheadMat, false);

            var glassMat = UrpMaterialFactory.GetArmourglass("BridgeCanopy", new Color(0.52f, 0.80f, 0.98f, 0.14f));
            var canopyGo = CreateLocalBox(cRoot, "BridgeCanopyGlass", new Vector3(comp.LocalCenter.x, glassCenterY, maxZ), new Vector3(w - 0.30f, glassH, 0.12f), glassMat, false);
            var barrier = canopyGo.AddComponent<BoxCollider>();
            barrier.size = new Vector3(1.05f, h / glassH, 2.0f);

            // Structural canopy mullions
            float[] mullionX = { -w * 0.48f, -w * 0.18f, w * 0.18f, w * 0.48f };
            for (int i = 0; i < mullionX.Length; i++)
            {
                CreateLocalBox(cRoot, $"BridgeCanopyMullion_{i}", new Vector3(comp.LocalCenter.x + mullionX[i], glassCenterY, maxZ - 0.02f), new Vector3(0.14f, glassH + 0.08f, 0.22f), frameMat, false);
            }

            // Exterior bow nose fairing + subtle deep-space starfield & navigational beacon vista beyond the Bridge glass
            var vistaRoot = new GameObject("BridgeExteriorVista").transform;
            vistaRoot.SetParent(cRoot, false);
            CreateLocalBox(vistaRoot, "BowForedeckCowling", new Vector3(comp.LocalCenter.x, floorY + 0.35f, maxZ + 2.4f), new Vector3(w * 0.68f, 0.45f, 4.2f), frameMat, false);

            Material starWarm = UrpMaterialFactory.GetEmissive("BridgeVistaStarWarm", Color.white, new Color(1.0f, 0.94f, 0.84f) * 2.2f);
            Material starCool = UrpMaterialFactory.GetEmissive("BridgeVistaStarCool", Color.white, new Color(0.75f, 0.90f, 1.0f) * 2.4f);
            for (int s = 0; s < 28; s++)
            {
                float sx = ((s * 37) % 44) - 22f;
                float sy = floorY - 2.0f + ((s * 19) % 18) * 0.85f;
                float sz = maxZ + 18f + ((s * 29) % 34);
                float dia = 0.06f + (s % 3) * 0.03f;
                CreateHoloSphere(vistaRoot, new Vector3(sx, sy, sz), dia, (s % 2 == 0) ? starCool : starWarm);
            }
        }

        private void DressShipCompartment(Transform cRoot, ShipCompartmentDef comp, float floorY)
        {
            if (Catalog == null) return;
            Vector3 worldFloorCenter = ShipInteriorWorldOrigin + new Vector3(comp.LocalCenter.x, floorY, comp.LocalCenter.z);

            switch (comp.Id)
            {
                case "hold":
                    _cargoPalletsRoot = new GameObject("DynamicCargoPallets").transform;
                    _cargoPalletsRoot.SetParent(cRoot, false);
                    // Overhead crane rail & baseline stowage bay markers
                    if (Catalog.stationPipeProp != null)
                        PrefabFittingHelper.SpawnCentered(Catalog.stationPipeProp, cRoot, "HoldOverheadRail", worldFloorCenter + new Vector3(0f, comp.LocalSize.y - 0.35f, 0f), Quaternion.Euler(0f, 90f, 0f), 8.5f, false, scaleUpToTarget: false);
                    UpdateCargoVisualFill(20); // Default light dunnage until live +ship/hold arrives
                    break;

                case "engineering":
                    if (Catalog.stationPowerConsoleProp != null)
                    {
                        PrefabFittingHelper.SpawnGrounded(Catalog.stationPowerConsoleProp, cRoot, "ReactorControlBus_P", worldFloorCenter + new Vector3(-2.6f, 0f, -2.2f), Quaternion.Euler(0f, 90f, 0f), 2.1f, 2.2f, false);
                        PrefabFittingHelper.SpawnGrounded(Catalog.stationPowerConsoleProp, cRoot, "ReactorControlBus_S", worldFloorCenter + new Vector3(2.6f, 0f, -2.2f), Quaternion.Euler(0f, -90f, 0f), 2.1f, 2.2f, false);
                    }
                    if (Catalog.stationPipeProp != null)
                    {
                        PrefabFittingHelper.SpawnCentered(Catalog.stationPipeProp, cRoot, "CoolantManifold", worldFloorCenter + new Vector3(0f, 2.6f, -3.2f), Quaternion.identity, 6.2f, false, scaleUpToTarget: false);
                    }
                    if (Catalog.shipVentMistPrefab != null)
                    {
                        var mist = Instantiate(Catalog.shipVentMistPrefab, worldFloorCenter + new Vector3(0f, 0.35f, -3.4f), Quaternion.identity, cRoot);
                        mist.name = "EngineeringCoolantMist";
                    }
                    break;

                case "crew":
                    // Port & Starboard crew bunks + galley table
                    Material bunkMat = UrpMaterialFactory.GetOpaque("CrewBunk", new Color(0.22f, 0.27f, 0.34f), 0.35f, 0.25f);
                    CreateLocalBox(cRoot, "CrewBunk_Lower", new Vector3(comp.LocalCenter.x + 2.5f, floorY + 0.45f, comp.LocalCenter.z - 1.2f), new Vector3(1.4f, 0.45f, 2.4f), bunkMat, true);
                    CreateLocalBox(cRoot, "CrewBunk_Upper", new Vector3(comp.LocalCenter.x + 2.5f, floorY + 1.55f, comp.LocalCenter.z - 1.2f), new Vector3(1.4f, 0.30f, 2.4f), bunkMat, false);
                    if (Catalog.stationCrateProp != null)
                        PrefabFittingHelper.SpawnGrounded(Catalog.stationCrateProp, cRoot, "ProvisionsLocker", worldFloorCenter + new Vector3(-2.4f, 0f, -2.2f), Quaternion.Euler(0f, 20f, 0f), 0.95f, 1.4f, true);
                    break;

                case "bay":
                    // Dray-P 2-Rig Stern Vehicle Bay: RIG 1 & RIG 2 cradles + stern ramp launch console
                    Material cautionMat = UrpMaterialFactory.GetEmissive("RampCaution", new Color(0.75f, 0.52f, 0.10f), new Color(0.95f, 0.62f, 0.12f) * 1.1f);
                    CreateLocalBox(cRoot, "SternRampThreshold", new Vector3(comp.LocalCenter.x, floorY + 0.05f, comp.LocalCenter.z - comp.LocalSize.z * 0.5f + 0.65f), new Vector3(6.8f, 0.10f, 0.55f), cautionMat, false);

                    for (int rSlot = 1; rSlot <= 2; rSlot++)
                    {
                        float rx = rSlot == 1 ? -2.25f : 2.25f;
                        Vector3 rigWorld = ShipInteriorWorldOrigin + new Vector3(rx, floorY, comp.LocalCenter.z - 0.6f);
                        var rover = TacSimVehicleVisualFactory.SpawnSurfaceRover(Catalog, cRoot, $"RIG {rSlot}", rigWorld, 180f, isEscort: rSlot == 2);
                        var col = rover.Root.AddComponent<BoxCollider>();
                        col.size = new Vector3(2.8f, 2.4f, 5.0f);
                        col.center = new Vector3(0f, 1.2f, 0f);

                        var rWi = rover.Root.AddComponent<WorldInteractable>();
                        rWi.kind = InteractableKind.VehicleBayRig;
                        rWi.promptTitle = $"Operate Rig {rSlot} (Dray-P Cradle)";
                        rWi.promptSubtitle = "Launch down stern ramp, lease rig, or open Surface Quarry HUD";
                        rWi.commandOrKey = $"rig:{rSlot}";
                        rWi.maxInteractDistance = 4.4f;
                        rWi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
                    }
                    break;
            }
        }

        public void UpdateCargoVisualFill(int loadedTonnes)
        {
            if (_cargoPalletsRoot == null || Catalog == null || !Template.Compartments.TryGetValue("hold", out var hold))
                return;

            for (int i = _cargoPalletsRoot.childCount - 1; i >= 0; i--)
            {
                var ch = _cargoPalletsRoot.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }

            int stacks = Mathf.Clamp((loadedTonnes + 11) / 12, 0, 8);
            float floorY = hold.LocalCenter.y - hold.LocalSize.y * 0.5f;
            for (int i = 0; i < stacks; i++)
            {
                float sideX = (i % 2 == 0) ? -2.85f : 2.85f;
                float zOff = -3.6f + (i / 2) * 2.4f;
                Vector3 palletWorld = ShipInteriorWorldOrigin + new Vector3(sideX, floorY, hold.LocalCenter.z + zOff);
                GameObject prefab = (i % 3 == 2 && Catalog.stationBarrelProp != null) ? Catalog.stationBarrelProp : Catalog.stationCrateProp;
                if (prefab != null)
                {
                    PrefabFittingHelper.SpawnGrounded(prefab, _cargoPalletsRoot, $"CargoLot_{i}", palletWorld, Quaternion.Euler(0f, (i * 25f) % 90f, 0f), 1.15f, 1.85f, keepColliders: true);
                }
            }
        }

        private void BuildCompanionwayRamp(Transform parent, Vector3 localBottom, Vector3 localTop, float width, Material mat, Material railMat, string name)
        {
            var root = new GameObject(name).transform;
            root.SetParent(parent, false);
            int steps = 12;
            for (int i = 0; i < steps; i++)
            {
                float t = (i + 0.5f) / steps;
                Vector3 p = Vector3.Lerp(localBottom, localTop, t);
                CreateLocalBox(root, $"Step_{i}", p, new Vector3(width, 0.26f, 0.55f), mat, true);
            }

            // Port & Starboard sloped safety handrails + stanchions (visual only so narrow companionways never snag capsules)
            Vector3 railBottom = localBottom + new Vector3(0f, 0.96f, 0f);
            Vector3 railTop = localTop + new Vector3(0f, 0.96f, 0f);
            Vector3 delta = railTop - railBottom;
            float railLen = delta.magnitude;
            if (railLen > 0.2f)
            {
                Quaternion railRot = Quaternion.LookRotation(delta.normalized, Vector3.up);
                Vector3 mid = (railBottom + railTop) * 0.5f;
                float sideX = width * 0.5f + 0.04f;

                var railL = CreateLocalBox(root, "RampRail_L", mid + new Vector3(-sideX, 0f, 0f), new Vector3(0.05f, 0.05f, railLen), railMat, false);
                railL.transform.localRotation = railRot;
                var railR = CreateLocalBox(root, "RampRail_R", mid + new Vector3(sideX, 0f, 0f), new Vector3(0.05f, 0.05f, railLen), railMat, false);
                railR.transform.localRotation = railRot;

                for (int pIdx = 0; pIdx <= 2; pIdx++)
                {
                    float pt = pIdx * 0.5f;
                    Vector3 basePt = Vector3.Lerp(localBottom, localTop, pt);
                    CreateLocalBox(root, $"RampStanchion_L_{pIdx}", basePt + new Vector3(-sideX, 0.48f, 0f), new Vector3(0.04f, 0.96f, 0.04f), railMat, false);
                    CreateLocalBox(root, $"RampStanchion_R_{pIdx}", basePt + new Vector3(sideX, 0.48f, 0f), new Vector3(0.04f, 0.96f, 0.04f), railMat, false);
                }
            }
        }

        private void BuildShipExitTriggers()
        {
            Material hatchGlow = UrpMaterialFactory.GetEmissive("ShipHatchGlow", new Color(0.1f, 0.4f, 0.55f), new Color(0.2f, 0.85f, 1f) * 0.9f);
            foreach (var ex in Template.Exits)
            {
                if (!Template.Compartments.TryGetValue(ex.From, out var cA) || !Template.Compartments.TryGetValue(ex.To, out var cB))
                    continue;

                Vector3 mid = (cA.LocalCenter + cB.LocalCenter) * 0.5f;
                Vector3 axis = (cB.LocalCenter - cA.LocalCenter).normalized;

                var go = new GameObject($"ShipPortal_{ex.From}_{ex.To}");
                go.transform.SetParent(transform, false);
                go.transform.localPosition = mid;

                var bc = go.AddComponent<BoxCollider>();
                bc.isTrigger = true;
                bc.size = new Vector3(3.2f, 2.8f, 1.8f);

                var trig = go.AddComponent<ExitPortalTrigger>();
                trig.roomA = ex.From;
                trig.roomB = ex.To;
                trig.commandAToB = ex.ForwardCommand;
                trig.commandBToA = ex.ReverseCommand;
                trig.labelAToB = ex.ForwardLabel;
                trig.labelBToA = ex.ReverseLabel;
                trig.axisAToB = axis;
                trig.OnPortalCrossed = (p, target, cmd) => OnPortalCrossed?.Invoke(p, target, cmd);

                CreateLocalBox(go.transform, "HatchStrip", Vector3.zero, new Vector3(2.8f, 0.08f, 0.18f), hatchGlow, false);

                // Frame flat spine bulkheads (airlock <-> passage, passage <-> crew, hold <-> engineering) with Vattalus BlastDoor_300cm
                if (Catalog != null && Catalog.shipBlastDoorPrefab != null && Mathf.Abs(cA.LocalCenter.y - cB.LocalCenter.y) < 0.5f)
                {
                    float flY = cA.LocalCenter.y - cA.LocalSize.y * 0.5f;
                    Vector3 worldDoorPos = ShipInteriorWorldOrigin + new Vector3(mid.x, flY, mid.z);
                    PrefabFittingHelper.SpawnGrounded(Catalog.shipBlastDoorPrefab, go.transform, "BulkheadFrame", worldDoorPos, Quaternion.identity, 2.95f, 3.8f, keepColliders: false);
                }
            }
        }

        private void BuildAirlockOuterHatch()
        {
            if (!Template.Compartments.TryGetValue("airlock", out var airlock)) return;
            float floorY = airlock.LocalCenter.y - airlock.LocalSize.y * 0.5f;
            Vector3 localPos = new Vector3(0f, floorY + 1.5f, airlock.LocalCenter.z + airlock.LocalSize.z * 0.5f - 0.25f);

            var hatch = CreateLocalBox(transform, "OuterAirlockDoor", localPos, new Vector3(2.2f, 2.6f, 0.35f),
                UrpMaterialFactory.GetEmissive("AirlockSeal", new Color(0.18f, 0.32f, 0.22f), new Color(0.15f, 0.9f, 0.45f) * 1.2f), true);

            if (Catalog != null && Catalog.shipAirlockDoorPrefab != null)
            {
                Vector3 doorWorldFloor = ShipInteriorWorldOrigin + new Vector3(0f, floorY, airlock.LocalCenter.z + airlock.LocalSize.z * 0.5f - 0.32f);
                PrefabFittingHelper.SpawnGrounded(Catalog.shipAirlockDoorPrefab, hatch.transform, "VattalusOuterLock", doorWorldFloor, Quaternion.identity, 2.95f, 3.4f, keepColliders: false);
            }

            _airlockLeaveInteractable = hatch.AddComponent<WorldInteractable>();
            _airlockLeaveInteractable.kind = InteractableKind.AirlockDisembark;
            _airlockLeaveInteractable.promptTitle = "Cycle Outer Lock Ashore";
            _airlockLeaveInteractable.promptSubtitle = "Step out into the Berth Gallery (+ship/leave)";
            _airlockLeaveInteractable.commandOrKey = "+ship/leave";
            _airlockLeaveInteractable.maxInteractDistance = 3.8f;
            _airlockLeaveInteractable.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
        }

        private void BuildBridgeHoloAndHelm()
        {
            if (!Template.Compartments.TryGetValue("bridge", out var bridge)) return;
            float floorY = bridge.LocalCenter.y - bridge.LocalSize.y * 0.5f;

            // Central 3D Holo-Tank Emitter
            var holoPedestal = CreateLocalBox(transform, "HoloTankPedestal", new Vector3(0f, floorY + 0.55f, bridge.LocalCenter.z + 0.5f), new Vector3(1.8f, 1.1f, 1.8f),
                UrpMaterialFactory.GetOpaque("HoloBase", new Color(0.12f, 0.16f, 0.22f), 0.6f, 0.6f), true);

            var holoDisc = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            holoDisc.name = "HoloProjectorDisc";
            holoDisc.transform.SetParent(holoPedestal.transform, false);
            holoDisc.transform.localPosition = new Vector3(0f, 0.62f, 0f);
            holoDisc.transform.localScale = new Vector3(1.5f, 0.05f, 1.5f);
            var discCol = holoDisc.GetComponent<Collider>();
            if (discCol != null) { if (Application.isPlaying) Destroy(discCol); else DestroyImmediate(discCol); }
            holoDisc.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("HoloRing", new Color(0.15f, 0.72f, 1.0f, 0.35f));

            _holotankBlipsRoot = new GameObject("HoloTankContacts").transform;
            _holotankBlipsRoot.SetParent(holoPedestal.transform, false);
            _holotankBlipsRoot.localPosition = new Vector3(0f, 1.05f, 0f);

            // Seed default holo sun/Ceres + ship blip until live space.holotank arrives
            PopulateHoloBlips(null);

            // Forward Acceleration Couch (Helm Station) + Vattalus VA_Seat01_Red_Prefab dual command/ops seats & forward consoles
            var helmCouch = CreateLocalBox(transform, "HelmAccelerationCouch", new Vector3(0f, floorY + 0.42f, bridge.LocalCenter.z + 2.5f), new Vector3(1.1f, 0.82f, 1.1f),
                UrpMaterialFactory.GetEmissive("HelmSeat", new Color(0.18f, 0.26f, 0.36f), new Color(0.12f, 0.55f, 0.9f) * 0.45f), true);

            if (Catalog != null)
            {
                Vector3 helmFloorWorld = ShipInteriorWorldOrigin + new Vector3(0f, floorY, bridge.LocalCenter.z + 2.5f);
                if (Catalog.shipHelmSeatPrefab != null)
                {
                    var mr = helmCouch.GetComponent<MeshRenderer>();
                    if (mr != null) mr.enabled = false; // Hide placeholder cube when commercial Vattalus crash couch is present
                    PrefabFittingHelper.SpawnGrounded(Catalog.shipHelmSeatPrefab, transform, "CommandCrashCouch", helmFloorWorld, Quaternion.identity, 1.32f, 1.5f, keepColliders: false);
                    PrefabFittingHelper.SpawnGrounded(Catalog.shipHelmSeatPrefab, transform, "OpsCrashCouch", helmFloorWorld + new Vector3(-1.65f, 0f, -0.15f), Quaternion.Euler(0f, 10f, 0f), 1.32f, 1.5f, keepColliders: false);
                }
                if (Catalog.stationConsoleProp != null)
                {
                    PrefabFittingHelper.SpawnGrounded(Catalog.stationConsoleProp, transform, "HelmFlightDesk", helmFloorWorld + new Vector3(0f, 0f, 1.15f), Quaternion.Euler(0f, 180f, 0f), 1.05f, 1.6f, keepColliders: false);
                    PrefabFittingHelper.SpawnGrounded(Catalog.stationConsoleProp, transform, "OpsSensorDesk", helmFloorWorld + new Vector3(-1.65f, 0f, 1.0f), Quaternion.Euler(0f, 190f, 0f), 1.05f, 1.6f, keepColliders: false);
                }
            }

            HelmSeatStandSpot = new GameObject("HelmStandSpot").transform;
            HelmSeatStandSpot.SetParent(transform, false);
            HelmSeatStandSpot.localPosition = new Vector3(0f, floorY + 0.15f, bridge.LocalCenter.z + 2.5f);

            BridgeCameraAnchor = new GameObject("BridgeHoloCameraAnchor").transform;
            BridgeCameraAnchor.SetParent(transform, false);
            BridgeCameraAnchor.localPosition = new Vector3(2.4f, floorY + 2.4f, bridge.LocalCenter.z - 1.2f);
            BridgeCameraAnchor.localRotation = Quaternion.Euler(24f, -32f, 0f);

            var helmWi = helmCouch.AddComponent<WorldInteractable>();
            helmWi.kind = InteractableKind.BridgeHelmSeat;
            helmWi.promptTitle = "Take Bridge Helm & Orbital Canopy";
            helmWi.promptSubtitle = "Switch to 3D Space Flight / Undock / 6-DOF Docking";
            helmWi.commandOrKey = "seat:helm";
            helmWi.maxInteractDistance = 3.8f;
            helmWi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
        }

        private void Update()
        {
            if (_holotankBlipsRoot != null)
            {
                _holoSpinDeg = (_holoSpinDeg + Time.deltaTime * 16f) % 360f;
                _holotankBlipsRoot.localRotation = Quaternion.Euler(0f, _holoSpinDeg, 0f);
            }

            float dt = Mathf.Max(0.001f, Time.deltaTime);
            foreach (var kv in _crewActors)
            {
                var actorGo = kv.Value;
                if (actorGo == null || !_crewTargetPositions.TryGetValue(kv.Key, out Vector3 targetPos))
                    continue;

                Vector3 curPos = actorGo.transform.position;
                Vector3 delta = targetPos - curPos;
                float dist = delta.magnitude;
                var spawner = actorGo.GetComponent<CharacterVisualSpawner>();

                if (dist > 0.02f)
                {
                    float stepSpeed = 3.2f;
                    actorGo.transform.position = Vector3.MoveTowards(curPos, targetPos, stepSpeed * dt);
                    Vector3 flat = new Vector3(delta.x, 0f, delta.z);
                    if (flat.sqrMagnitude > 0.001f)
                    {
                        Quaternion face = Quaternion.LookRotation(flat.normalized, Vector3.up);
                        actorGo.transform.rotation = Quaternion.Slerp(actorGo.transform.rotation, face, dt * 8f);
                    }
                    spawner?.UpdateLocomotion(stepSpeed, false, false, dt);
                }
                else
                {
                    actorGo.transform.position = targetPos;
                    spawner?.UpdateLocomotion(0f, false, false, dt);
                }
            }
        }

        private static string NormalizeUnitId(string unitId)
        {
            if (string.IsNullOrWhiteSpace(unitId))
                return "tacsim.unit.dray";
            string trimmed = unitId.Trim().ToLowerInvariant();
            return trimmed.StartsWith("tacsim.unit.", StringComparison.Ordinal)
                ? trimmed
                : "tacsim.unit." + trimmed;
        }

        public bool TryResolveCompartmentFromRoomView(MushRoomView rv, out string compartmentId)
        {
            compartmentId = null;
            if (rv == null) return false;
            if (rv.TacSim != null && rv.TacSim.Mode == "interior" && !string.IsNullOrEmpty(rv.TacSim.RoomTemplateId))
            {
                if (!string.IsNullOrEmpty(rv.TacSim.Unit))
                {
                    string normalizedUnit = NormalizeUnitId(rv.TacSim.Unit);
                    if (!string.Equals(NormalizeUnitId(LoadedUnitId), normalizedUnit, StringComparison.OrdinalIgnoreCase))
                    {
                        BuildInterior(Catalog, normalizedUnit);
                    }
                }
                compartmentId = rv.TacSim.RoomTemplateId;
                return true;
            }

            // Fallback match by Dray compartment name/description for servers not yet publishing ambient["tacsim"]
            if (Template != null && !string.IsNullOrEmpty(rv.Name))
            {
                string trimmed = rv.Name.Trim();
                foreach (var kv in Template.Compartments)
                {
                    if (string.Equals(kv.Value.Name, trimmed, StringComparison.OrdinalIgnoreCase) ||
                        trimmed.EndsWith(" - " + kv.Value.Name, StringComparison.OrdinalIgnoreCase) ||
                        trimmed.EndsWith(" — " + kv.Value.Name, StringComparison.OrdinalIgnoreCase))
                    {
                        compartmentId = kv.Key;
                        return true;
                    }
                }
            }
            return false;
        }

        public void ApplyRoomView(MushRoomView rv)
        {
            ApplyRoomView(rv, null);
        }

        public void ApplyRoomView(MushRoomView rv, string selfActorId)
        {
            if (!TryResolveCompartmentFromRoomView(rv, out string resolvedComp))
                return;

            ActiveCompartmentId = resolvedComp;
            if (_airlockLeaveInteractable != null)
            {
                if (rv.TacSim != null)
                {
                    _airlockLeaveInteractable.promptTitle = rv.TacSim.Docked
                        ? $"Disembark {rv.TacSim.HullName} to {rv.TacSim.Port}"
                        : "Outer Airlock Sealed (Under Way)";
                }
                else
                {
                    _airlockLeaveInteractable.promptTitle = "Cycle Outer Airlock (+ship/leave)";
                }
            }

            SyncCompartmentOccupants(rv, selfActorId, resolvedComp);
        }

        private void ClearCrewActors()
        {
            foreach (var kv in _crewActors)
            {
                if (kv.Value != null)
                {
                    if (Application.isPlaying) Destroy(kv.Value);
                    else DestroyImmediate(kv.Value);
                }
            }
            _crewActors.Clear();
            _crewTargetPositions.Clear();
            _crewBillboards.Clear();
        }

        private void SyncCompartmentOccupants(MushRoomView rv, string selfActorId, string compartmentId)
        {
            if (rv == null || !TryGetCompartmentDef(compartmentId, out var comp))
                return;

            if (_crewActorsContainer == null)
            {
                _crewActorsContainer = new GameObject("LiveShipCrewActors").transform;
                _crewActorsContainer.SetParent(transform, false);
            }

            if (!string.Equals(_crewActorsCompartmentId, comp.Id, StringComparison.Ordinal))
            {
                ClearCrewActors();
                _crewActorsCompartmentId = comp.Id;
            }

            bool hasAuthoritativeTyping = rv.ThingIds.Count > 0 || rv.PlayerIds.Count > 0 || rv.NpcIds.Count > 0;
            var seenIds = new HashSet<string>(StringComparer.Ordinal);
            int slotIdx = 0;

            for (int i = 0; i < rv.Contents.Count; i++)
            {
                var item = rv.Contents[i];
                if (string.IsNullOrEmpty(item.Id)) continue;
                if (!string.IsNullOrEmpty(selfActorId) && item.Id == selfActorId) continue;
                if (rv.DormantNpcIds.Contains(item.Id) || rv.ThingIds.Contains(item.Id)) continue;

                bool isPlayer = rv.PlayerIds.Contains(item.Id);
                bool isNpc = rv.NpcIds.Contains(item.Id);
                if (!isPlayer && !isNpc)
                {
                    if (hasAuthoritativeTyping || LooksLikeShipFixture(item.Name))
                        continue;
                }

                seenIds.Add(item.Id);
                Vector3 targetPos = ComputeCrewStandSpot(comp, slotIdx);
                slotIdx++;
                _crewTargetPositions[item.Id] = targetPos;

                if (_crewActors.TryGetValue(item.Id, out var existingGo) && existingGo != null)
                    continue;

                var actorGo = new GameObject($"ShipActor_{item.Id}_{item.Name}");
                actorGo.transform.SetParent(_crewActorsContainer, true);
                actorGo.transform.position = targetPos;

                Vector3 compFloorWorld = ShipInteriorWorldOrigin + new Vector3(comp.LocalCenter.x, targetPos.y - ShipInteriorWorldOrigin.y, comp.LocalCenter.z);
                Vector3 toCenter = compFloorWorld - targetPos;
                toCenter.y = 0f;
                actorGo.transform.rotation = toCenter.sqrMagnitude > 0.01f
                    ? Quaternion.LookRotation(toCenter.normalized, Vector3.up)
                    : Quaternion.Euler(0f, 180f, 0f);

                var cap = actorGo.AddComponent<CapsuleCollider>();
                cap.height = 1.8f;
                cap.radius = 0.36f;
                cap.center = new Vector3(0f, 0.9f, 0f);

                var spawner = actorGo.AddComponent<CharacterVisualSpawner>();
                bool isMale = TacSimContentLoader.InferIsMale(item.Name, null);
                spawner.BuildNpcVisual(Catalog, item.Name, isRemotePlayer: !isNpc, isMale: isMale);

                var bbGo = new GameObject("Billboard");
                bbGo.transform.SetParent(actorGo.transform, false);
                bbGo.transform.localPosition = new Vector3(0f, 2.04f, 0f);
                var bb = bbGo.AddComponent<WorldBillboard>();
                bb.AnchorLocalY = 2.04f;
                Vector3 minW = ShipInteriorWorldOrigin + comp.LocalCenter - comp.LocalSize * 0.5f;
                Vector3 maxW = ShipInteriorWorldOrigin + comp.LocalCenter + comp.LocalSize * 0.5f;
                bb.SetRoomSafeBounds(minW.x, maxW.x, minW.z, maxW.z, wallMarginM: 1.05f);

                string safeActorName = MushProtocol.EscapeUnityRichText(item.Name);
                string roleBadge = isNpc ? "<color=#88CCFF>[CREW / NPC]</color>" : "<color=#66EEAA>[CREW]</color>";
                bb.NameplateText = CreateCrewBillboardTmp(bbGo.transform, "Nameplate", Vector3.zero, $"{safeActorName}\n<size=68%>{roleBadge}</size>", 1.26f, Color.white);
                bb.NameplateText.rectTransform.sizeDelta = new Vector2(2.40f, 0.56f);

                bb.SpeechBubbleText = CreateCrewBillboardTmp(bbGo.transform, "SpeechBubble", new Vector3(0f, 0.32f, 0f), string.Empty, 1.02f, new Color(1f, 0.95f, 0.65f));
                bb.SpeechBubbleText.rectTransform.pivot = new Vector2(0.5f, 0.0f);
                bb.SpeechBubbleText.rectTransform.sizeDelta = new Vector2(2.35f, 2.00f);
                bb.SpeechBubbleText.alignment = TextAlignmentOptions.Bottom;
                bb.SpeechBubbleText.gameObject.SetActive(false);

                var wi = actorGo.AddComponent<WorldInteractable>();
                wi.kind = InteractableKind.NpcActor;
                wi.promptTitle = $"Talk to {item.Name}";
                wi.promptSubtitle = isNpc ? "Speak with crew member" : "Speak with co-op shipmate";
                wi.commandOrKey = item.Name;
                wi.maxInteractDistance = 3.8f;
                wi.OnInteracted = target => OnInteractableTriggered?.Invoke(target);

                _crewActors[item.Id] = actorGo;
                _crewBillboards[item.Id] = bb;
            }

            if (_crewActors.Count > seenIds.Count)
            {
                var departed = new List<string>();
                foreach (var kv in _crewActors)
                {
                    if (!seenIds.Contains(kv.Key))
                        departed.Add(kv.Key);
                }
                for (int i = 0; i < departed.Count; i++)
                {
                    string depId = departed[i];
                    if (_crewActors.TryGetValue(depId, out var depGo) && depGo != null)
                    {
                        if (Application.isPlaying) Destroy(depGo);
                        else DestroyImmediate(depGo);
                    }
                    _crewActors.Remove(depId);
                    _crewTargetPositions.Remove(depId);
                    _crewBillboards.Remove(depId);
                }
            }
        }

        private static Vector3 ComputeCrewStandSpot(ShipCompartmentDef comp, int idx)
        {
            float floorY = comp.LocalCenter.y - comp.LocalSize.y * 0.5f;
            float maxX = Mathf.Max(0.85f, comp.LocalSize.x * 0.5f - 0.95f);
            float maxZ = Mathf.Max(0.85f, comp.LocalSize.z * 0.5f - 1.10f);

            float sideSign = (idx % 2 == 0) ? 1f : -1f;
            float xOff = Mathf.Clamp(sideSign * (1.35f + (idx / 2) * 0.45f), -maxX, maxX);
            float zOff = Mathf.Clamp(((idx % 3) - 1) * 1.15f, -maxZ, maxZ);

            // Keep Ops crash couch spot clear on Bridge
            if (comp.Id == "bridge" && idx == 0)
            {
                xOff = -1.65f;
                zOff = 1.45f;
            }

            return ShipInteriorWorldOrigin + new Vector3(comp.LocalCenter.x + xOff, floorY, comp.LocalCenter.z + zOff);
        }

        private static bool LooksLikeShipFixture(string rawName)
        {
            if (string.IsNullOrWhiteSpace(rawName)) return true;
            string lower = rawName.ToLowerInvariant();
            return lower.Contains("slate") || lower.Contains("locker") || lower.Contains("bunk") ||
                   lower.Contains("couch") || lower.Contains("console") || lower.Contains("hatch") ||
                   lower.Contains("door") || lower.Contains("rig ") || lower.StartsWith("rig", StringComparison.Ordinal) ||
                   lower.Contains("tank") || lower.Contains("rom") || lower.Contains("still");
        }

        public void ShowActorSpeech(string actorId, string actorName, string text)
        {
            string safeText = MushProtocol.EscapeUnityRichText(text);
            if (!string.IsNullOrEmpty(actorId) && _crewBillboards.TryGetValue(actorId, out var bb) && bb != null)
            {
                bb.ShowSpeech(safeText);
                return;
            }
            if (!string.IsNullOrEmpty(actorName))
            {
                foreach (var kv in _crewActors)
                {
                    if (kv.Value != null && kv.Value.name.IndexOf(actorName, StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        if (_crewBillboards.TryGetValue(kv.Key, out var b2) && b2 != null)
                            b2.ShowSpeech(safeText);
                        break;
                    }
                }
            }
        }

        public void SetActorComposing(string actorId, string actorName, bool composing)
        {
            if (!string.IsNullOrEmpty(actorId) && _crewBillboards.TryGetValue(actorId, out var bb) && bb != null)
            {
                if (composing)
                    bb.ShowSpeech("...");
                else if (bb.SpeechBubbleText != null && bb.SpeechBubbleText.text == "...")
                    bb.SpeechBubbleText.gameObject.SetActive(false);
                return;
            }
            if (!string.IsNullOrEmpty(actorName))
            {
                foreach (var kv in _crewActors)
                {
                    if (kv.Value != null && kv.Value.name.IndexOf(actorName, StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        if (_crewBillboards.TryGetValue(kv.Key, out var b2) && b2 != null)
                        {
                            if (composing) b2.ShowSpeech("...");
                            else if (b2.SpeechBubbleText != null && b2.SpeechBubbleText.text == "...") b2.SpeechBubbleText.gameObject.SetActive(false);
                        }
                        break;
                    }
                }
            }
        }

        public void UpdateGaugeText(string line)
        {
            if (string.IsNullOrWhiteSpace(line)) return;
            string key = TryGetCompartmentDef(ActiveCompartmentId, out var comp) ? comp.Id : ActiveCompartmentId;
            if (_gaugeDisplays.TryGetValue(key, out var tmp) && tmp != null)
            {
                string cur = tmp.text ?? string.Empty;
                if (cur.Length > 340) cur = cur.Substring(cur.Length - 240);
                tmp.text = cur + "\n" + line.Trim();
            }
        }

        public void PopulateHoloBlips(JsonNode holoFeed)
        {
            if (_holotankBlipsRoot == null) return;
            for (int i = _holotankBlipsRoot.childCount - 1; i >= 0; i--)
            {
                var ch = _holotankBlipsRoot.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }

            Material selfMat = UrpMaterialFactory.GetHologramUnlit("BlipSelf", new Color(0.25f, 1.0f, 0.7f, 0.85f));
            Material friendlyMat = UrpMaterialFactory.GetHologramUnlit("BlipFriend", new Color(0.35f, 0.8f, 1.0f, 0.8f));
            Material hostileMat = UrpMaterialFactory.GetHologramUnlit("BlipHostile", new Color(1.0f, 0.35f, 0.3f, 0.9f));
            Material rockMat = UrpMaterialFactory.GetHologramUnlit("BlipRock", new Color(1.0f, 0.78f, 0.3f, 0.85f));

            // Center own-ship marker
            CreateHoloSphere(_holotankBlipsRoot, Vector3.zero, 0.11f, selfMat);

            var contacts = holoFeed != null && holoFeed.IsObject ? holoFeed["contacts"].AsArray() : null;
            double rangeKm = holoFeed != null && holoFeed.IsObject ? Math.Max(1.0, holoFeed.Double("range_km", 1000.0)) : 1000.0;

            if (contacts != null && contacts.Count > 0)
            {
                for (int i = 0; i < contacts.Count; i++)
                {
                    var c = contacts[i];
                    if (c.Bool("you", false)) continue;
                    float x = (float)(c.Double("x_km", 0.0) / rangeKm) * 0.7f;
                    float z = (float)(c.Double("y_km", 0.0) / rangeKm) * 0.7f;
                    float y = (float)(c.Double("z_km", 0.0) / rangeKm) * 0.5f;
                    string kind = c.Str("kind", "ship");
                    string rel = c.Str("relation", "neutral");
                    Material m = (kind == "asteroid" || kind == "rock") ? rockMat : (rel == "hostile" ? hostileMat : friendlyMat);
                    CreateHoloSphere(_holotankBlipsRoot, new Vector3(x, y, z), 0.08f, m);
                }
            }
            else
            {
                // Ambient Ceres / waypoint markers when docked
                CreateHoloSphere(_holotankBlipsRoot, new Vector3(0.42f, 0.12f, 0.28f), 0.14f, friendlyMat);
                CreateHoloSphere(_holotankBlipsRoot, new Vector3(-0.35f, -0.08f, 0.38f), 0.07f, rockMat);
            }
        }

        private static void CreateHoloSphere(Transform parent, Vector3 localPos, float diameter, Material mat)
        {
            var s = GameObject.CreatePrimitive(PrimitiveType.Sphere);
            s.transform.SetParent(parent, false);
            s.transform.localPosition = localPos;
            s.transform.localScale = Vector3.one * diameter;
            var col = s.GetComponent<Collider>();
            if (col != null) { if (Application.isPlaying) Destroy(col); else DestroyImmediate(col); }
            s.GetComponent<MeshRenderer>().sharedMaterial = mat;
        }

        private static GameObject CreateLocalBox(Transform parent, string name, Vector3 localPos, Vector3 scale, Material mat, bool collider)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Cube);
            go.name = name;
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localScale = scale;
            if (!collider)
            {
                var c = go.GetComponent<Collider>();
                if (c != null) { if (Application.isPlaying) Destroy(c); else DestroyImmediate(c); }
            }
            go.GetComponent<MeshRenderer>().sharedMaterial = mat;
            return go;
        }

        private static TextMeshPro CreateLocalTmp(Transform parent, Vector3 localPos, Quaternion localRot, string text, float size)
        {
            var go = new GameObject("GaugeText");
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localRotation = localRot;
            var tmp = go.AddComponent<TextMeshPro>();
            tmp.text = text;
            tmp.fontSize = size;
            tmp.color = new Color(0.78f, 0.95f, 1.0f);
            tmp.alignment = TextAlignmentOptions.TopLeft;
            tmp.overflowMode = TextOverflowModes.Ellipsis;
            tmp.rectTransform.sizeDelta = new Vector2(2.25f, 1.0f);
            return tmp;
        }

        private static TextMeshPro CreateCrewBillboardTmp(Transform parent, string name, Vector3 localPos, string text, float fontSize, Color color)
        {
            var go = new GameObject(name);
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            var tmp = go.AddComponent<TextMeshPro>();
            tmp.text = text;
            tmp.fontSize = fontSize;
            tmp.color = color;
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.Normal;
            tmp.rectTransform.sizeDelta = new Vector2(2.4f, 0.8f);
            return tmp;
        }
    }
}
