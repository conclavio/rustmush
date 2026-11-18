using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.Net;
using TMPro;
using UnityEngine;

namespace TacSim.World
{
    /// <summary>
    /// Builds and drives the walkable 6-compartment Dray ship interior
    /// (Airlock, Central Passage, Bridge, Crew Compartment, Cargo Hold, Engineering Access)
    /// from crates/tacsim/content/units/dray.json, plus Vattalus Fenris ship shell dressing,
    /// live 3D Bridge Holo-Tank contacts, Airlock disembark, and the Bridge Helm Couch.
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
        private Transform _holotankBlipsRoot;
        private Transform _cargoPalletsRoot;
        private WorldInteractable _airlockLeaveInteractable;
        private float _holoSpinDeg;

        public Vector3 GetCompartmentSpawnWorldPos(string compartmentId)
        {
            if (Template != null && Template.Compartments.TryGetValue(compartmentId ?? "airlock", out var comp))
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
                Template.Compartments.TryGetValue(currentCompId, out var cur))
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
            return Template.FindExitPath(fromComp, toComp);
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

            transform.position = ShipInteriorWorldOrigin;

            // Optional Vattalus Fenris Frigate exterior shell framing around our walkable 6-room Dray deck spine
            if (Catalog != null && Catalog.fenrisExteriorInteriorPrefab != null)
            {
                var shell = Instantiate(Catalog.fenrisExteriorInteriorPrefab, transform);
                shell.name = "FenrisVisualHullShell";
                shell.transform.localPosition = new Vector3(0f, -2.2f, 2f);
                // Disable interior colliders on decorative shell so our exact 6-compartment Dray deck triggers & stairs govern physics cleanly
                foreach (var c in shell.GetComponentsInChildren<Collider>(true))
                    c.enabled = false;
            }

            Material deckMat = UrpMaterialFactory.GetOpaque("ShipDeck", new Color(0.17f, 0.20f, 0.24f), 0.45f, 0.6f);
            Material bulkheadMat = UrpMaterialFactory.GetOpaque("ShipBulkhead", new Color(0.26f, 0.29f, 0.33f), 0.38f, 0.5f);
            Material holdMat = UrpMaterialFactory.GetOpaque("HoldBay", new Color(0.14f, 0.18f, 0.25f), 0.35f, 0.65f);
            Material rampMat = UrpMaterialFactory.GetOpaque("ShipLadderRamp", new Color(0.32f, 0.35f, 0.38f), 0.4f, 0.7f);

            foreach (var kv in Template.Compartments)
            {
                var comp = kv.Value;
                var cRoot = new GameObject($"Compartment_{comp.Id}").transform;
                cRoot.SetParent(transform, false);

                float floorY = comp.LocalCenter.y - comp.LocalSize.y * 0.5f;
                Material flMat = comp.Id == "hold" ? holdMat : deckMat;

                // Floor & side bulkheads (open along spine connections & ramps)
                CreateLocalBox(cRoot, "DeckPlate", new Vector3(comp.LocalCenter.x, floorY - 0.12f, comp.LocalCenter.z), new Vector3(comp.LocalSize.x, 0.24f, comp.LocalSize.z), flMat, true);
                CreateLocalBox(cRoot, "PortBulkhead", new Vector3(comp.LocalCenter.x - comp.LocalSize.x * 0.5f, comp.LocalCenter.y, comp.LocalCenter.z), new Vector3(0.22f, comp.LocalSize.y, comp.LocalSize.z), bulkheadMat, true);
                CreateLocalBox(cRoot, "StarboardBulkhead", new Vector3(comp.LocalCenter.x + comp.LocalSize.x * 0.5f, comp.LocalCenter.y, comp.LocalCenter.z), new Vector3(0.22f, comp.LocalSize.y, comp.LocalSize.z), bulkheadMat, true);

                // Outer endcaps
                if (comp.Id == "airlock" || comp.Id == "bridge")
                {
                    CreateLocalBox(cRoot, "ForwardBulkhead", new Vector3(comp.LocalCenter.x, comp.LocalCenter.y, comp.LocalCenter.z + comp.LocalSize.z * 0.5f), new Vector3(comp.LocalSize.x, comp.LocalSize.y, 0.25f), bulkheadMat, true);
                }
                if (comp.Id == "crew" || (comp.Id == "engineering" && !Template.Compartments.ContainsKey("bay")) || comp.Id == "bay")
                {
                    CreateLocalBox(cRoot, "AftBulkhead", new Vector3(comp.LocalCenter.x, comp.LocalCenter.y, comp.LocalCenter.z - comp.LocalSize.z * 0.5f), new Vector3(comp.LocalSize.x, comp.LocalSize.y, 0.25f), bulkheadMat, true);
                }

                // Compartment lighting matching dray.json prose (blue work light in Hold, warm in Crew/Engineering, cool Bridge)
                var ltGo = new GameObject("CompartmentLight");
                ltGo.transform.SetParent(cRoot, false);
                ltGo.transform.localPosition = comp.LocalCenter + new Vector3(0f, comp.LocalSize.y * 0.35f, 0f);
                var lt = ltGo.AddComponent<Light>();
                lt.type = LightType.Point;
                lt.range = 12f;
                lt.intensity = 3.2f;
                lt.color = comp.Id == "hold" ? new Color(0.45f, 0.72f, 1.0f)
                         : (comp.Id == "crew" || comp.Id == "engineering" ? new Color(1.0f, 0.78f, 0.52f) : new Color(0.85f, 0.94f, 1.0f));

                // Live gauge slate
                var gaugeGo = CreateLocalBox(cRoot, "GaugeSlate", new Vector3(comp.LocalCenter.x - comp.LocalSize.x * 0.45f, floorY + 1.65f, comp.LocalCenter.z), new Vector3(0.08f, 1.1f, 2.4f),
                    UrpMaterialFactory.GetOpaque("Slate", new Color(0.05f, 0.09f, 0.14f), 0.7f, 0.3f), false);
                var tmp = CreateLocalTmp(gaugeGo.transform, new Vector3(0.6f, 0f, 0f), Quaternion.Euler(0f, -90f, 0f), $"{comp.Name.ToUpperInvariant()}\n<size=70%>{comp.ShortDesc}</size>", 1.5f);
                _gaugeDisplays[comp.Id] = tmp;

                DressShipCompartment(cRoot, comp, floorY);
            }

            // Walkable inclined companionway ramps connecting Passage <-> Bridge (+3.4m) and Passage <-> Hold (-3.6m)
            BuildCompanionwayRamp(transform, new Vector3(-1.6f, -1.6f, 2.5f), new Vector3(-1.6f, 1.8f, 6.2f), 1.6f, rampMat, "BridgeCompanionway");
            BuildCompanionwayRamp(transform, new Vector3(1.6f, -1.6f, 3.5f), new Vector3(1.6f, -5.3f, 0.2f), 1.6f, rampMat, "HoldCompanionway");

            // Ship ExitPortalTriggers matching dray.json exits
            BuildShipExitTriggers();

            // Airlock Outer Door (+ship/leave)
            BuildAirlockOuterHatch();

            // Bridge 3D Holo-Tank & Helm Couch
            BuildBridgeHoloAndHelm();
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

        private void BuildCompanionwayRamp(Transform parent, Vector3 localBottom, Vector3 localTop, float width, Material mat, string name)
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
        }

        public bool TryResolveCompartmentFromRoomView(MushRoomView rv, out string compartmentId)
        {
            compartmentId = null;
            if (rv == null) return false;
            if (rv.TacSim != null && rv.TacSim.Mode == "interior" && !string.IsNullOrEmpty(rv.TacSim.RoomTemplateId))
            {
                if (!string.IsNullOrEmpty(rv.TacSim.Unit) &&
                    !string.Equals(LoadedUnitId, rv.TacSim.Unit, StringComparison.OrdinalIgnoreCase))
                {
                    BuildInterior(Catalog, rv.TacSim.Unit);
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
        }

        public void UpdateGaugeText(string line)
        {
            if (string.IsNullOrWhiteSpace(line)) return;
            if (_gaugeDisplays.TryGetValue(ActiveCompartmentId, out var tmp) && tmp != null)
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
            tmp.rectTransform.sizeDelta = new Vector2(2.2f, 1.0f);
            return tmp;
        }
    }
}
