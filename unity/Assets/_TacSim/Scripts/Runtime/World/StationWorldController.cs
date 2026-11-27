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
    /// Builds and drives the 3-deck Ceres Freeport 3D station directly from
    /// crates/tacsim/content/atlas/neighborhood.ceres_freeport.json, including seamless doorway/stair
    /// ExitPortalTriggers, the floodlit Visiting Berth Gallery overlooking docked hulls,
    /// world-space boards, and Ida Faber NPC cast spawners.
    /// </summary>
    public sealed class StationWorldController : MonoBehaviour
    {
        public TacSimAssetCatalog Catalog;
        public AtlasNeighborhoodPack Pack { get; private set; }
        public string LoadedPackId { get; private set; } = string.Empty;
        public string ActiveRoomEntryId { get; private set; } = "room.c03_arrivals";

        public event Action<WorldInteractable> OnInteractableTriggered;
        public event Action<ExitPortalTrigger, string, string> OnPortalCrossed;

        private readonly Dictionary<string, Transform> _roomRoots = new Dictionary<string, Transform>(StringComparer.Ordinal);
        private readonly Dictionary<string, Light> _roomLights = new Dictionary<string, Light>(StringComparer.Ordinal);
        private readonly Dictionary<string, TextMeshPro> _roomBoards = new Dictionary<string, TextMeshPro>(StringComparer.Ordinal);
        private readonly Dictionary<string, List<string>> _roomBoardLines = new Dictionary<string, List<string>>(StringComparer.Ordinal);
        private readonly Dictionary<string, GameObject> _spawnedActors = new Dictionary<string, GameObject>(StringComparer.Ordinal);
        private readonly Dictionary<string, CharacterVisualSpawner> _actorSpawners = new Dictionary<string, CharacterVisualSpawner>(StringComparer.Ordinal);
        private readonly Dictionary<string, string> _actorDisplayNames = new Dictionary<string, string>(StringComparer.Ordinal);
        private readonly Dictionary<string, Vector3> _actorTargetPositions = new Dictionary<string, Vector3>(StringComparer.Ordinal);
        private readonly Dictionary<string, WorldBillboard> _actorBillboards = new Dictionary<string, WorldBillboard>(StringComparer.Ordinal);
        private readonly Dictionary<string, TextMeshPro> _actorQuestMarkers = new Dictionary<string, TextMeshPro>(StringComparer.Ordinal);
        private readonly Dictionary<string, MushNpcQuestMarker> _actorQuestMarkerStates = new Dictionary<string, MushNpcQuestMarker>(StringComparer.Ordinal);
        private readonly List<TacSimDockedHullRef> _lastBerthTradeHulls = new List<TacSimDockedHullRef>();
        private bool _hasAuthoritativeBerthHulls;
        private string _spawnedActorsRoomEntry;

        public int ActiveQuestMarkerCount => _actorQuestMarkers.Count;

        public MushNpcQuestMarker GetNpcQuestMarker(string npcId)
        {
            if (string.IsNullOrEmpty(npcId)) return null;
            return _actorQuestMarkerStates.TryGetValue(npcId, out var m) ? m : null;
        }

        private sealed class ExteriorPortShuttle
        {
            public Transform Root;
            public float StartX;
            public float EndX;
            public float BaseY;
            public float BaseZ;
            public float SpeedMps;
            public float PhaseOffset;
        }

        private Transform _actorsContainer;
        private Transform _dockedShipVisual;
        private readonly List<(Transform tr, float baseY)> _berthDrones = new List<(Transform, float)>();
        private readonly List<ExteriorPortShuttle> _exteriorTraffic = new List<ExteriorPortShuttle>();
        private readonly List<TextMeshPro> _cradleHullSigns = new List<TextMeshPro>();
        private readonly List<GameObject> _cradleHullObjects = new List<GameObject>();
        private WorldInteractable _airlockBoardInteractable;
        private string _builtGeometrySignature;

        public int DoorwaySignCount { get; private set; }
        public int ExteriorTrafficShuttleCount => _exteriorTraffic.Count;

        public const int MaxViewportMullions = 64;
        public const int MaxViewingStanchions = 64;
        public const int MaxWallSlotScanSteps = 512;
        public const int MaxSpawnedRoomActors = 64;
        public const int MaxMooredCradleHulls = 16;

        // Shift atlas coordinates near Unity origin (0, y, 0) for crisp float precision across any port pack
        private float _originOffsetX = -160f;
        private float _originOffsetZ = -150f;

        public Vector3 AtlasToUnity(float atlasX, float elevationY, float atlasY)
        {
            return new Vector3(atlasX + _originOffsetX, elevationY, atlasY + _originOffsetZ);
        }

        public Vector3 GetRoomSpawnPosition(string roomEntryId)
        {
            if (Pack != null && Pack.Rooms.TryGetValue(roomEntryId, out var room))
            {
                float elev = Pack.GetFloorElevationY(room.FloorIndex);
                Vector2 c = room.Center2D;
                if (roomEntryId == "room.c03_arrivals")
                    return AtlasToUnity(c.x + 5f, elev + 0.15f, c.y + 4f);
                if (roomEntryId == "room.simulator_lobby")
                    return AtlasToUnity(c.x - 2f, elev + 0.15f, c.y);
                return AtlasToUnity(c.x, elev + 0.15f, c.y);
            }
            return new Vector3(0f, 6.15f, 0f);
        }

        public string FindContainingRoomEntry(Vector3 worldPos)
        {
            return TryFindContainingRoomWithHysteresis(worldPos, ActiveRoomEntryId, 0f);
        }

        public string TryFindContainingRoomWithHysteresis(Vector3 worldPos, string currentRoomEntryId, float hysteresisM = 0.85f)
        {
            if (Pack == null) return ActiveRoomEntryId;
            float ax = worldPos.x - _originOffsetX;
            float ay = worldPos.z - _originOffsetZ;

            // Keep current room within a generous doorway & stair threshold band (1.70 m doorway deadband, 1.0 m stair deadband)
            // so crossing a threshold or stepping up/down inter-floor stairs never oscillates between rooms.
            if (!string.IsNullOrEmpty(currentRoomEntryId) && hysteresisM > 0f &&
                Pack.Rooms.TryGetValue(currentRoomEntryId, out var cur))
            {
                float curElev = Pack.GetFloorElevationY(cur.FloorIndex);
                if (ax >= cur.MinX - hysteresisM && ax <= cur.MaxX + hysteresisM &&
                    ay >= cur.MinY - hysteresisM && ay <= cur.MaxY + hysteresisM &&
                    Mathf.Abs(worldPos.y - curElev) < 3.6f)
                {
                    return currentRoomEntryId;
                }
            }

            string best = null;
            float bestVert = float.MaxValue;
            float vertCaptureLimit = (!string.IsNullOrEmpty(currentRoomEntryId) && hysteresisM > 0f) ? 2.6f : 3.4f;
            foreach (var kv in Pack.Rooms)
            {
                var r = kv.Value;
                if (ax >= r.MinX && ax <= r.MaxX && ay >= r.MinY && ay <= r.MaxY)
                {
                    float elev = Pack.GetFloorElevationY(r.FloorIndex);
                    float dy = Mathf.Abs(worldPos.y - elev);
                    if (dy < vertCaptureLimit && dy < bestVert)
                    {
                        bestVert = dy;
                        best = r.EntryId;
                    }
                }
            }
            return best ?? currentRoomEntryId ?? ActiveRoomEntryId;
        }

        public List<(string command, string targetRoom)> FindExitPath(string fromRoomEntry, string toRoomEntry)
        {
            if (Pack == null) return new List<(string, string)>();
            return Pack.FindExitPath(fromRoomEntry, toRoomEntry);
        }

        public bool AreRoomsConnectedWithinHops(string roomA, string roomB, int maxHops = 3)
        {
            if (string.IsNullOrEmpty(roomA) || string.IsNullOrEmpty(roomB)) return false;
            if (string.Equals(roomA, roomB, StringComparison.Ordinal)) return true;
            var path = FindExitPath(roomA, roomB);
            return path.Count > 0 && path.Count <= maxHops;
        }

        public bool RebuildStationIfContentChanged(TacSimAssetCatalog catalog, string packId)
        {
            var candidate = TacSimContentLoader.LoadNeighborhoodPackById(packId);
            string newSig = candidate.ComputeGeometrySignature();
            if (Pack != null && string.Equals(_builtGeometrySignature, newSig, StringComparison.Ordinal))
            {
                // Geometry & exits are identical (e.g. post-login sync confirming local pack); refresh metadata/cast in-place without tearing down 3D colliders or lights!
                Pack = candidate;
                LoadedPackId = candidate.PackId;
                return false;
            }
            BuildLoadedStationPack(catalog, candidate, newSig);
            return true;
        }

        public void BuildStation(TacSimAssetCatalog catalog, string packId = "neighborhood.ceres_freeport")
        {
            var loaded = TacSimContentLoader.LoadNeighborhoodPackById(packId);
            BuildLoadedStationPack(catalog, loaded, loaded.ComputeGeometrySignature());
        }

        private void BuildLoadedStationPack(TacSimAssetCatalog catalog, AtlasNeighborhoodPack loadedPack, string geometrySignature)
        {
            Catalog = catalog;
            Pack = loadedPack;
            LoadedPackId = Pack.PackId;
            _builtGeometrySignature = geometrySignature;

            Vector2 center = Pack.GetBoundsCenter2D();
            _originOffsetX = -center.x;
            _originOffsetZ = -center.y;
            ActiveRoomEntryId = Pack.GetDefaultSpawnRoomEntry();

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _roomRoots.Clear();
            _roomLights.Clear();
            _roomBoards.Clear();
            _roomBoardLines.Clear();
            _spawnedActors.Clear();
            _actorSpawners.Clear();
            _actorDisplayNames.Clear();
            _actorTargetPositions.Clear();
            _actorBillboards.Clear();
            _spawnedActorsRoomEntry = null;
            _berthDrones.Clear();
            _exteriorTraffic.Clear();
            DoorwaySignCount = 0;
            _cradleHullSigns.Clear();
            _cradleHullObjects.Clear();
            _airlockBoardInteractable = null;
            _dockedShipVisual = null;

            _actorsContainer = new GameObject("LiveStationActors").transform;
            _actorsContainer.SetParent(transform, false);

            // Map shared wall doorways, non-adjacent transit portals, & inter-floor stairs/ladders across ANY corridor port pack
            var horizontalDoors = new List<(int floor, float yWall, float centerX, AtlasExitDef exit)>();
            var verticalDoors = new List<(int floor, float xWall, float centerY, AtlasExitDef exit)>();
            var interFloorExits = new List<AtlasExitDef>();
            var nonAdjacentExits = new List<AtlasExitDef>();

            foreach (var ex in Pack.Exits)
            {
                if (ex.Hidden)
                    continue;

                if (!Pack.Rooms.TryGetValue(ex.FromRoom, out var rA) || !Pack.Rooms.TryGetValue(ex.ToRoom, out var rB))
                    continue;

                if (rA.FloorIndex != rB.FloorIndex)
                {
                    if (!ex.Hidden && !ex.Locked)
                        interFloorExits.Add(ex);
                    continue;
                }

                // Shared horizontal wall (Y constant)
                bool matchedWall = false;
                if (Mathf.Abs(rA.MaxY - rB.MinY) < 0.25f || Mathf.Abs(rB.MaxY - rA.MinY) < 0.25f)
                {
                    float yShared = Mathf.Abs(rA.MaxY - rB.MinY) < 0.25f ? rA.MaxY : rA.MinY;
                    float xOverlapMin = Mathf.Max(rA.MinX, rB.MinX);
                    float xOverlapMax = Mathf.Min(rA.MaxX, rB.MaxX);
                    if (xOverlapMax - xOverlapMin >= 2.2f)
                    {
                        horizontalDoors.Add((rA.FloorIndex, yShared, (xOverlapMin + xOverlapMax) * 0.5f, ex));
                        matchedWall = true;
                    }
                }
                // Shared vertical wall (X constant)
                else if (Mathf.Abs(rA.MaxX - rB.MinX) < 0.25f || Mathf.Abs(rB.MaxX - rA.MinX) < 0.25f)
                {
                    float xShared = Mathf.Abs(rA.MaxX - rB.MinX) < 0.25f ? rA.MaxX : rA.MinX;
                    float yOverlapMin = Mathf.Max(rA.MinY, rB.MinY);
                    float yOverlapMax = Mathf.Min(rA.MaxY, rB.MaxY);
                    if (yOverlapMax - yOverlapMin >= 2.2f)
                    {
                        verticalDoors.Add((rA.FloorIndex, xShared, (yOverlapMin + yOverlapMax) * 0.5f, ex));
                        matchedWall = true;
                    }
                }

                if (!matchedWall && !ex.Locked)
                {
                    nonAdjacentExits.Add(ex);
                }
            }

            foreach (var kv in Pack.Rooms)
            {
                BuildRoomGeometry(kv.Value, horizontalDoors, verticalDoors, interFloorExits);
            }

            BuildPortalTriggers(horizontalDoors, verticalDoors, interFloorExits, nonAdjacentExits);
            BuildCountersAndServiceDesks();
            BuildOocSimulatorCentreProps();
            if (LoadedPackId != "neighborhood.ooc_area")
            {
                BuildExteriorBerthCradles();
            }
        }

        private void BuildRoomGeometry(
            AtlasRoomDef room,
            List<(int floor, float yWall, float centerX, AtlasExitDef exit)> hDoors,
            List<(int floor, float xWall, float centerY, AtlasExitDef exit)> vDoors,
            List<AtlasExitDef> interFloorExits)
        {
            var root = new GameObject(room.EntryId).transform;
            root.SetParent(transform, false);
            _roomRoots[room.EntryId] = root;

            float elev = Pack.GetFloorElevationY(room.FloorIndex);
            float height = Pack.GetFloorHeight(room.FloorIndex);
            Vector2 center2d = room.Center2D;
            Vector2 size2d = room.Size2D;

            bool isStickneyOrRock = LoadedPackId.Contains("stickney") || LoadedPackId.Contains("kirkwood") || LoadedPackId.Contains("voltaire");
            bool isLunar = LoadedPackId.Contains("aitken");

            Color wallTint = room.EntryId == "room.sim_red" ? new Color(0.46f, 0.24f, 0.25f)
                           : room.EntryId == "room.sim_blue" ? new Color(0.24f, 0.34f, 0.48f)
                           : isStickneyOrRock ? new Color(0.42f, 0.35f, 0.30f)
                           : isLunar ? new Color(0.46f, 0.50f, 0.55f)
                           : new Color(0.36f, 0.40f, 0.45f);
            Material trimMat = UrpMaterialFactory.GetOpaque("StationTrim", new Color(0.32f, 0.36f, 0.42f), 0.45f, 0.55f);

            // Check if an inter-floor stair/ladder opens down through this upper room's floor
            if (room.EntryId == "room.c03_arrivals")
            {
                CreateTexturedFloorSlab(root, "Floor_West", room.EntryId, room.MinX, room.MinY, 122f, room.MaxY, elev);
                CreateTexturedFloorSlab(root, "Floor_East", room.EntryId, 128f, room.MinY, room.MaxX, room.MaxY, elev);
                CreateTexturedFloorSlab(root, "Floor_South", room.EntryId, 122f, room.MinY, 128f, 143f, elev);
                CreateTexturedFloorSlab(root, "Floor_North", room.EntryId, 122f, 154f, 128f, room.MaxY, elev);
            }
            else if (TryGetUpperStairOpening(room, interFloorExits, out float hx0, out float hy0, out float hx1, out float hy1))
            {
                if (hx0 > room.MinX + 0.3f) CreateTexturedFloorSlab(root, "Floor_West", room.EntryId, room.MinX, room.MinY, hx0, room.MaxY, elev);
                if (room.MaxX > hx1 + 0.3f) CreateTexturedFloorSlab(root, "Floor_East", room.EntryId, hx1, room.MinY, room.MaxX, room.MaxY, elev);
                if (hy0 > room.MinY + 0.3f) CreateTexturedFloorSlab(root, "Floor_South", room.EntryId, hx0, room.MinY, hx1, hy0, elev);
                if (room.MaxY > hy1 + 0.3f) CreateTexturedFloorSlab(root, "Floor_North", room.EntryId, hx0, hy1, hx1, room.MaxY, elev);
            }
            else
            {
                CreateTexturedFloorSlab(root, "Floor", room.EntryId, room.MinX, room.MinY, room.MaxX, room.MaxY, elev);
            }

            // Overhead ceiling beams (kept open enough for 3rd-person camera comfort while framing the architecture)
            CreateBox(root, "CeilingRim_N", AtlasToUnity(center2d.x, elev + height - 0.15f, room.MaxY - 0.3f), new Vector3(size2d.x, 0.3f, 0.6f), trimMat, false);
            CreateBox(root, "CeilingRim_S", AtlasToUnity(center2d.x, elev + height - 0.15f, room.MinY + 0.3f), new Vector3(size2d.x, 0.3f, 0.6f), trimMat, false);

            // Berth Gallery in any port has panoramic framed armourglass overlooking floodlit cradles & the dark!
            bool isBerthGallery = room.EntryId.IndexOf("berth_gallery", StringComparison.OrdinalIgnoreCase) >= 0;
            if (isBerthGallery)
            {
                Material glassMat = UrpMaterialFactory.GetArmourglass("BerthGlass", new Color(0.32f, 0.68f, 0.90f, 0.11f));
                Material sillMat = GetWallMaterialForSpan(room.EntryId, wallTint, size2d.x, 0.6f, isStickneyOrRock);

                // Knee-high bulkhead sill + framed clear armourglass + vertical steel mullions every 8 m
                float sillH = 0.55f;
                CreateBox(root, "ViewportSill", AtlasToUnity(center2d.x, elev + sillH * 0.5f, room.MinY), new Vector3(size2d.x, sillH, 0.34f), sillMat, true);
                CreateBox(root, "Armourglass_Viewport", AtlasToUnity(center2d.x, elev + sillH + (height - sillH) * 0.5f, room.MinY), new Vector3(size2d.x, height - sillH, 0.08f), glassMat, true);

                float safeSpanX = (float.IsNaN(size2d.x) || float.IsInfinity(size2d.x)) ? 8.0f : Mathf.Clamp(size2d.x, 1.0f, TacSimContentLoader.MaxRoomSpanMeters);
                int mullionCount = Mathf.Clamp(Mathf.RoundToInt(safeSpanX / 8.0f), 2, MaxViewportMullions);
                for (int m = 0; m <= mullionCount; m++)
                {
                    float mx = Mathf.Lerp(room.MinX + 0.15f, room.MaxX - 0.15f, m / (float)mullionCount);
                    CreateBox(root, $"ViewportMullion_{m}", AtlasToUnity(mx, elev + height * 0.5f, room.MinY), new Vector3(0.24f, height, 0.28f), trimMat, false);
                }

                // Grounded brushed-steel observation guardrail with stanchion posts + flush muted deck safety strip (no floating yellow bar!)
                Material railSteel = UrpMaterialFactory.GetOpaque("ObservationHandrail", new Color(0.26f, 0.30f, 0.35f), 0.48f, 0.72f);
                Material safetyLineMat = UrpMaterialFactory.GetOpaque("DeckSafetyLine", new Color(0.48f, 0.36f, 0.14f), 0.18f, 0.12f);
                float railSpanX = Mathf.Clamp(safeSpanX - 4f, 2f, TacSimContentLoader.MaxRoomSpanMeters);
                float railAtlasY = room.MinY + 0.92f;
                CreateBox(root, "ViewingHandrail", AtlasToUnity(center2d.x, elev + 0.96f, railAtlasY), new Vector3(railSpanX, 0.055f, 0.065f), railSteel, false);
                CreateBox(root, "ViewingKneeRail", AtlasToUnity(center2d.x, elev + 0.52f, railAtlasY), new Vector3(railSpanX, 0.030f, 0.030f), railSteel, false);
                CreateBox(root, "DeckSafetyInlay", AtlasToUnity(center2d.x, elev + 0.004f, railAtlasY + 0.22f), new Vector3(railSpanX, 0.006f, 0.065f), safetyLineMat, false);

                int stanchions = Mathf.Clamp(Mathf.RoundToInt(railSpanX / 6.0f), 2, MaxViewingStanchions);
                for (int st = 0; st <= stanchions; st++)
                {
                    float sx = Mathf.Lerp(center2d.x - railSpanX * 0.5f, center2d.x + railSpanX * 0.5f, st / (float)stanchions);
                    CreateBox(root, $"ViewingStanchion_{st}", AtlasToUnity(sx, elev + 0.48f, railAtlasY), new Vector3(0.055f, 0.96f, 0.055f), railSteel, false);
                }
            }
            else
            {
                BuildHorizontalWallIfCanonical(root, room, room.MinY, isSouthOfRoom: true, elev, height, wallTint, isStickneyOrRock, hDoors);
            }

            BuildHorizontalWallIfCanonical(root, room, room.MaxY, isSouthOfRoom: false, elev, height, wallTint, isStickneyOrRock, hDoors);
            BuildVerticalWallIfCanonical(root, room, room.MinX, isWestOfRoom: true, elev, height, wallTint, isStickneyOrRock, vDoors);
            BuildVerticalWallIfCanonical(root, room, room.MaxX, isWestOfRoom: false, elev, height, wallTint, isStickneyOrRock, vDoors);

            // Overhead matte coffered ceiling when no upper-deck floor slab sits directly above this room
            if (!HasUpperFloorSlabCovering(room))
            {
                Vector2 ceilTiling = new Vector2(Mathf.Max(1f, Mathf.Round(size2d.x / 4.0f)), Mathf.Max(1f, Mathf.Round(size2d.y / 4.0f)));
                Material ceilMat = UrpMaterialFactory.GetStationCeilingMaterial($"Roof_{room.EntryId}", wallTint, ceilTiling);
                CreateBox(root, "RoofCeiling", AtlasToUnity(center2d.x, elev + height + 0.03f, center2d.y), new Vector3(size2d.x, 0.06f, size2d.y), ceilMat, withCollider: false);
            }

            // Recessed architectural ceiling luminaire + soft diffused room light (no bare glaring ceiling hotspot!)
            Color lightColor = GetRoomLightColor(room.EntryId, isStickneyOrRock);
            float fixtureLenX = Mathf.Clamp(size2d.x * 0.28f, 1.8f, 4.4f);
            Material housingMat = UrpMaterialFactory.GetOpaque("CeilingFixtureHousing", new Color(0.14f, 0.17f, 0.21f), 0.22f, 0.45f);
            Material diffuserMat = UrpMaterialFactory.GetEmissive("CeilingDiffuser_" + room.EntryId, lightColor * 0.85f, lightColor * 1.15f);
            CreateBox(root, "CeilingFixtureHousing", AtlasToUnity(center2d.x, elev + height - 0.06f, center2d.y), new Vector3(fixtureLenX, 0.10f, 0.62f), housingMat, withCollider: false);
            CreateBox(root, "CeilingLightDiffuser", AtlasToUnity(center2d.x, elev + height - 0.11f, center2d.y), new Vector3(fixtureLenX - 0.16f, 0.03f, 0.44f), diffuserMat, withCollider: false);

            var lightGo = new GameObject("RoomLight");
            lightGo.transform.SetParent(root, false);
            lightGo.transform.position = AtlasToUnity(center2d.x, elev + height - 1.25f, center2d.y);
            var lt = lightGo.AddComponent<Light>();
            lt.type = LightType.Point;
            lt.color = lightColor;
            lt.range = Mathf.Max(size2d.x, size2d.y) * 1.05f;
            lt.intensity = room.Restricted ? 1.25f : 2.15f;
            _roomLights[room.EntryId] = lt;

            // Floating architectural sector header
            Color signCol = room.EntryId == "room.sim_red" ? new Color(1f, 0.42f, 0.38f)
                          : room.EntryId == "room.sim_blue" ? new Color(0.42f, 0.78f, 1f)
                          : new Color(0.65f, 0.88f, 1.0f);
            CreateWorldSign(root, room.Name.ToUpperInvariant(), AtlasToUnity(center2d.x, elev + 3.55f, room.MaxY - 0.28f), Quaternion.identity, 2.1f, signCol);

            // World-space live status / market board across all ports & Simulator Lobby
            if (isBerthGallery ||
                room.EntryId.IndexOf("concourse", StringComparison.OrdinalIgnoreCase) >= 0 ||
                room.EntryId.IndexOf("registry", StringComparison.OrdinalIgnoreCase) >= 0 ||
                room.EntryId.IndexOf("exchange", StringComparison.OrdinalIgnoreCase) >= 0 ||
                room.EntryId.IndexOf("chandlery", StringComparison.OrdinalIgnoreCase) >= 0 ||
                room.EntryId.IndexOf("commissary", StringComparison.OrdinalIgnoreCase) >= 0 ||
                room.EntryId.IndexOf("dockmaster", StringComparison.OrdinalIgnoreCase) >= 0 ||
                room.EntryId.IndexOf("harbourmaster", StringComparison.OrdinalIgnoreCase) >= 0 ||
                room.EntryId == "room.simulator_lobby")
            {
                float boardX = SelectClearHorizontalWallX(room, room.MaxY, hDoors, room.MinX + 4.2f, halfClearanceM: 2.8f);
                Vector3 boardPos = AtlasToUnity(boardX, elev + 2.10f, room.MaxY - 0.18f);
                var boardTmp = CreateDiegeticBoard(root, room, boardPos, Quaternion.identity);
                _roomBoards[room.EntryId] = boardTmp;
            }

            // Spawn up to 3 authored room detail plaques as grounded, non-overlapping [E] inspection terminals:
            // enforce >= 3.6m separation along the south wall, spilling onto clear East / West wall spans in narrow offices!
            var placedSouthXs = new List<float>();
            var placedEastYs = new List<float>();
            var placedWestYs = new List<float>();
            int detailIdx = 0;
            foreach (var det in room.Details)
            {
                if (detailIdx >= 3) break;
                float desiredX = Mathf.Lerp(room.MinX + 3.4f, room.MaxX - 3.4f, (detailIdx + 1f) / (Mathf.Min(3, room.Details.Count) + 1f));
                if (TrySelectNonOverlappingHorizontalWallX(room, room.MinY, hDoors, placedSouthXs, desiredX, halfClearanceM: 2.2f, minPlaqueSpacingM: 3.6f, out float southX))
                {
                    placedSouthXs.Add(southX);
                    float pz = isBerthGallery ? room.MinY + 1.65f : room.MinY + 0.68f;
                    CreateDetailPlaque(root, det.Key, det.Value, AtlasToUnity(southX, elev, pz), inwardYawDeg: 0f);
                }
                else if (TrySelectNonOverlappingVerticalWallY(room, room.MaxX, vDoors, placedEastYs, room.Center2D.y, halfClearanceM: 2.2f, minPlaqueSpacingM: 3.4f, out float eastY))
                {
                    placedEastYs.Add(eastY);
                    CreateDetailPlaque(root, det.Key, det.Value, AtlasToUnity(room.MaxX - 0.72f, elev, eastY), inwardYawDeg: -90f);
                }
                else if (TrySelectNonOverlappingVerticalWallY(room, room.MinX, vDoors, placedWestYs, room.Center2D.y, halfClearanceM: 2.2f, minPlaqueSpacingM: 3.4f, out float westY))
                {
                    placedWestYs.Add(westY);
                    CreateDetailPlaque(root, det.Key, det.Value, AtlasToUnity(room.MinX + 0.72f, elev, westY), inwardYawDeg: 90f);
                }
                else
                {
                    float fallbackX = SelectClearHorizontalWallX(room, room.MinY, hDoors, desiredX, halfClearanceM: 2.0f);
                    float pz = isBerthGallery ? room.MinY + 1.65f : room.MinY + 0.68f;
                    CreateDetailPlaque(root, det.Key, det.Value, AtlasToUnity(fallbackX, elev, pz), inwardYawDeg: 0f);
                }
                detailIdx++;
            }

            // Heavy Station Kit & Gllow3D architectural dressing per room role
            DressRoomInterior(root, room, elev, height);
        }

        private bool TryGetUpperStairOpening(AtlasRoomDef room, List<AtlasExitDef> interFloorExits, out float hx0, out float hy0, out float hx1, out float hy1)
        {
            hx0 = hy0 = hx1 = hy1 = 0f;
            if (interFloorExits == null) return false;
            for (int i = 0; i < interFloorExits.Count; i++)
            {
                var ex = interFloorExits[i];
                if (!Pack.Rooms.TryGetValue(ex.FromRoom, out var rA) || !Pack.Rooms.TryGetValue(ex.ToRoom, out var rB))
                    continue;
                AtlasRoomDef upper = rA.FloorIndex > rB.FloorIndex ? rA : rB;
                if (upper != room) continue;

                ComputeInterFloorStairGeometry(ex, rA, rB, out Vector2 topXY, out Vector2 botXY, out _, out _, out bool runAlongX, out float stairWidth);
                float halfW = stairWidth * 0.5f + 0.25f;
                if (runAlongX)
                {
                    hx0 = Mathf.Max(room.MinX + 0.4f, Mathf.Min(topXY.x, botXY.x) - 0.7f);
                    hx1 = Mathf.Min(room.MaxX - 0.4f, Mathf.Max(topXY.x, botXY.x) + 0.7f);
                    hy0 = Mathf.Max(room.MinY + 0.4f, topXY.y - halfW);
                    hy1 = Mathf.Min(room.MaxY - 0.4f, topXY.y + halfW);
                }
                else
                {
                    hx0 = Mathf.Max(room.MinX + 0.4f, topXY.x - halfW);
                    hx1 = Mathf.Min(room.MaxX - 0.4f, topXY.x + halfW);
                    hy0 = Mathf.Max(room.MinY + 0.4f, Mathf.Min(topXY.y, botXY.y) - 0.7f);
                    hy1 = Mathf.Min(room.MaxY - 0.4f, Mathf.Max(topXY.y, botXY.y) + 0.7f);
                }
                return (hx1 - hx0 > 1.5f) && (hy1 - hy0 > 1.5f);
            }
            return false;
        }

        private void ComputeInterFloorStairGeometry(
            AtlasExitDef ex,
            AtlasRoomDef rA,
            AtlasRoomDef rB,
            out Vector2 topXY,
            out Vector2 botXY,
            out float elevTop,
            out float elevBot,
            out bool runAlongX,
            out float stairWidth)
        {
            if (LoadedPackId == "neighborhood.ceres_freeport" && (rA.EntryId == "room.c03_arrivals" || rB.EntryId == "room.c03_arrivals"))
            {
                topXY = new Vector2(125f, 153.5f);
                botXY = new Vector2(125f, 143.5f);
                elevTop = 5.85f;
                elevBot = 0.15f;
                runAlongX = false;
                stairWidth = 4.8f;
                return;
            }

            AtlasRoomDef upper = rA.FloorIndex > rB.FloorIndex ? rA : rB;
            AtlasRoomDef lower = rA.FloorIndex > rB.FloorIndex ? rB : rA;
            elevTop = Pack.GetFloorElevationY(upper.FloorIndex) - 0.15f;
            elevBot = Pack.GetFloorElevationY(lower.FloorIndex) + 0.15f;

            // Clamp stair strictly inside the horizontal intersection of upper & lower rooms so stairs NEVER pierce adjacent walls/floors!
            float ovMinX = Mathf.Max(upper.MinX, lower.MinX);
            float ovMaxX = Mathf.Min(upper.MaxX, lower.MaxX);
            float ovMinY = Mathf.Max(upper.MinY, lower.MinY);
            float ovMaxY = Mathf.Min(upper.MaxY, lower.MaxY);

            bool hasOverlap = (ovMaxX - ovMinX) > 2.0f && (ovMaxY - ovMinY) > 2.0f;
            float boundMinX = hasOverlap ? ovMinX : upper.MinX;
            float boundMaxX = hasOverlap ? ovMaxX : upper.MaxX;
            float boundMinY = hasOverlap ? ovMinY : upper.MinY;
            float boundMaxY = hasOverlap ? ovMaxY : upper.MaxY;

            float spanX = boundMaxX - boundMinX;
            float spanY = boundMaxY - boundMinY;

            float anchorX = ex.HasExplicitPortal ? ex.PortalA.x : (boundMinX + boundMaxX) * 0.5f;
            float anchorY = ex.HasExplicitPortal ? ex.PortalA.z : (boundMinY + boundMaxY) * 0.5f;

            // Orient stair along the longer overlap axis when the Y depth is too shallow (e.g. Aitken Highport l07_exchange <-> l08_receiving).
            runAlongX = spanY < 9.0f && spanX > spanY;
            if (runAlongX)
            {
                stairWidth = Mathf.Clamp(spanY - 1.2f, 2.2f, 4.4f);
                float halfRun = Mathf.Min(4.2f, Mathf.Max(1.4f, spanX * 0.5f - 0.8f));
                float safeY = Mathf.Clamp(anchorY, boundMinY + stairWidth * 0.5f + 0.4f, boundMaxY - stairWidth * 0.5f - 0.4f);
                float safeCx = Mathf.Clamp(anchorX, boundMinX + halfRun + 0.5f, boundMaxX - halfRun - 0.5f);
                topXY = new Vector2(safeCx + halfRun, safeY);
                botXY = new Vector2(safeCx - halfRun, safeY);
            }
            else
            {
                stairWidth = Mathf.Clamp(spanX - 1.2f, 2.2f, 4.8f);
                float halfRun = Mathf.Min(4.2f, Mathf.Max(1.4f, spanY * 0.5f - 0.8f));
                float safeX = Mathf.Clamp(anchorX, boundMinX + stairWidth * 0.5f + 0.4f, boundMaxX - stairWidth * 0.5f - 0.4f);
                float safeCy = Mathf.Clamp(anchorY, boundMinY + halfRun + 0.5f, boundMaxY - halfRun - 0.5f);
                topXY = new Vector2(safeX, safeCy + halfRun);
                botXY = new Vector2(safeX, safeCy - halfRun);
            }
        }

        private void DressRoomInterior(Transform root, AtlasRoomDef room, float elev, float height)
        {
            if (Catalog == null) return;

            // 1. Structural corner support ribs (clear of doorways & central concourse paths)
            if (Catalog.stationSupportProp != null)
            {
                float inset = 0.65f;
                PrefabFittingHelper.SpawnGrounded(Catalog.stationSupportProp, root, "Rib_SW", AtlasToUnity(room.MinX + inset, elev, room.MinY + inset), Quaternion.Euler(0f, 45f, 0f), height - 0.2f, 2.2f, false);
                PrefabFittingHelper.SpawnGrounded(Catalog.stationSupportProp, root, "Rib_SE", AtlasToUnity(room.MaxX - inset, elev, room.MinY + inset), Quaternion.Euler(0f, -45f, 0f), height - 0.2f, 2.2f, false);
                PrefabFittingHelper.SpawnGrounded(Catalog.stationSupportProp, root, "Rib_NW", AtlasToUnity(room.MinX + inset, elev, room.MaxY - inset), Quaternion.Euler(0f, 135f, 0f), height - 0.2f, 2.2f, false);
                PrefabFittingHelper.SpawnGrounded(Catalog.stationSupportProp, root, "Rib_NE", AtlasToUnity(room.MaxX - inset, elev, room.MaxY - inset), Quaternion.Euler(0f, -135f, 0f), height - 0.2f, 2.2f, false);
            }

            // 2. Overhead utility pipe conduits
            if (Catalog.stationPipeProp != null)
            {
                float pipeY = elev + height - 0.45f;
                PrefabFittingHelper.SpawnCentered(Catalog.stationPipeProp, root, "OverheadPipe_N", AtlasToUnity(room.Center2D.x, pipeY, room.MaxY - 0.75f), Quaternion.identity, 6.2f, false, scaleUpToTarget: false);
                if (room.EntryId == "room.c02_pressure" || room.EntryId == "room.c14_dockmaster" || room.EntryId == "room.c15_maintenance")
                {
                    PrefabFittingHelper.SpawnCentered(Catalog.stationPipeProp, root, "OverheadPipe_S", AtlasToUnity(room.Center2D.x, pipeY, room.MinY + 0.85f), Quaternion.identity, 6.2f, false, scaleUpToTarget: false);
                }
            }

            // 3. Wall power console near NE corner
            if (Catalog.stationPowerConsoleProp != null && !room.Restricted)
            {
                PrefabFittingHelper.SpawnGrounded(Catalog.stationPowerConsoleProp, root, "WallPowerUnit", AtlasToUnity(room.MaxX - 1.4f, elev, room.MaxY - 0.7f), Quaternion.Euler(0f, 180f, 0f), 2.0f, 2.2f, false);
            }

            // 4. Room-specific commercial set dressing
            switch (room.EntryId)
            {
                case "room.c01_berth_gallery":
                    // Stacked ready-cargo & batteries along east gallery + static Gllow3D dockworkers watching the cradles
                    SpawnPropCluster(root, AtlasToUnity(room.MaxX - 3.2f, elev, room.MinY + 3.4f), includeBarrels: true, includeBattery: true);
                    SpawnAmbientDockWorker(root, 0, AtlasToUnity(146f, elev, room.MinY + 2.1f), Quaternion.Euler(0f, 180f, 0f), "BerthSpotter_01");
                    SpawnAmbientDockWorker(root, 1, AtlasToUnity(174f, elev, room.MinY + 2.2f), Quaternion.Euler(0f, 195f, 0f), "BerthEngineer_02");
                    break;

                case "room.c03_arrivals":
                    SpawnAmbientDockWorker(root, 2, AtlasToUnity(room.MaxX - 2.4f, elev, room.MinY + 2.8f), Quaternion.Euler(0f, -90f, 0f), "PortWatchGuard");
                    break;

                case "room.c04_concourse":
                    // Central concourse holo-directory pillar
                    Material pillarMat = UrpMaterialFactory.GetOpaque("ConcourseKiosk", new Color(0.14f, 0.20f, 0.28f), 0.55f, 0.5f);
                    Vector3 kioskPos = AtlasToUnity(room.Center2D.x, elev, room.Center2D.y);
                    CreateBox(root, "DirectoryBase", kioskPos + Vector3.up * 0.55f, new Vector3(1.8f, 1.1f, 1.8f), pillarMat, true);
                    if (Catalog.stationDisplayProp != null)
                        PrefabFittingHelper.SpawnGrounded(Catalog.stationDisplayProp, root, "ConcourseHoloDisplay", kioskPos + Vector3.up * 1.1f, Quaternion.identity, 1.1f, 1.6f, false);
                    break;

                case "room.c07_exchange":
                    if (Catalog.stationDisplayProp != null)
                    {
                        PrefabFittingHelper.SpawnGrounded(Catalog.stationDisplayProp, root, "ExchangeTicker_W", AtlasToUnity(room.MinX + 2.4f, elev + 1.1f, room.MaxY - 0.6f), Quaternion.Euler(0f, 180f, 0f), 1.0f, 1.8f, false);
                        PrefabFittingHelper.SpawnGrounded(Catalog.stationDisplayProp, root, "ExchangeTicker_E", AtlasToUnity(room.MaxX - 3.4f, elev + 1.1f, room.MaxY - 0.6f), Quaternion.Euler(0f, 180f, 0f), 1.0f, 1.8f, false);
                    }
                    SpawnAmbientDockWorker(root, 3, AtlasToUnity(room.MaxX - 2.6f, elev, room.MinY + 3.0f), Quaternion.Euler(0f, -45f, 0f), "ExchangeBroker");
                    break;

                case "room.c08_chandlery":
                case "room.c14_dockmaster":
                    SpawnPropCluster(root, AtlasToUnity(room.MaxX - 2.6f, elev, room.MinY + 2.6f), includeBarrels: true, includeBattery: true);
                    SpawnPropCluster(root, AtlasToUnity(room.MinX + 2.6f, elev, room.MinY + 2.6f), includeBarrels: true, includeBattery: false);
                    break;

                case "room.c10_slack_line":
                    // Crew bar booths & warm amber tables
                    Material boothMat = UrpMaterialFactory.GetOpaque("SlackBooth", new Color(0.28f, 0.17f, 0.13f), 0.35f, 0.2f);
                    Material tableMat = UrpMaterialFactory.GetEmissive("SlackTable", new Color(0.22f, 0.18f, 0.15f), new Color(0.85f, 0.45f, 0.12f) * 0.35f);
                    for (int b = 0; b < 2; b++)
                    {
                        Vector3 bPos = AtlasToUnity(room.MinX + 4.5f + b * 7.5f, elev, room.MinY + 3.6f);
                        CreateBox(root, $"BoothTable_{b}", bPos + Vector3.up * 0.42f, new Vector3(1.6f, 0.84f, 1.1f), tableMat, true);
                        CreateBox(root, $"BoothBenchL_{b}", bPos + new Vector3(-1.35f, 0.28f, 0f), new Vector3(0.65f, 0.56f, 1.4f), boothMat, true);
                        CreateBox(root, $"BoothBenchR_{b}", bPos + new Vector3(1.35f, 0.28f, 0f), new Vector3(0.65f, 0.56f, 1.4f), boothMat, true);
                    }
                    SpawnPropCluster(root, AtlasToUnity(room.MaxX - 2.3f, elev, room.MaxY - 2.3f), includeBarrels: true, includeBattery: false);
                    break;

                default:
                    if (!room.Restricted)
                        SpawnPropCluster(root, AtlasToUnity(room.MaxX - 2.3f, elev, room.MinY + 2.3f), includeBarrels: false, includeBattery: false);
                    break;
            }
        }

        private void SpawnPropCluster(Transform parent, Vector3 basePos, bool includeBarrels, bool includeBattery)
        {
            if (Catalog.stationCrateProp != null)
            {
                PrefabFittingHelper.SpawnGrounded(Catalog.stationCrateProp, parent, "FreightCrate_A", basePos, Quaternion.Euler(0f, 18f, 0f), 1.05f, 1.8f, true);
                PrefabFittingHelper.SpawnGrounded(Catalog.stationCrateProp, parent, "FreightCrate_B", basePos + new Vector3(-1.25f, 0f, 0.35f), Quaternion.Euler(0f, -12f, 0f), 0.85f, 1.5f, true);
            }
            if (includeBarrels && Catalog.stationBarrelProp != null)
            {
                PrefabFittingHelper.SpawnGrounded(Catalog.stationBarrelProp, parent, "Drum_A", basePos + new Vector3(0.95f, 0f, -0.35f), Quaternion.identity, 0.95f, 0.95f, false);
                PrefabFittingHelper.SpawnGrounded(Catalog.stationBarrelProp, parent, "Drum_B", basePos + new Vector3(1.55f, 0f, 0.15f), Quaternion.Euler(0f, 40f, 0f), 0.95f, 0.95f, false);
            }
            if (includeBattery && Catalog.stationBatteryProp != null)
            {
                PrefabFittingHelper.SpawnGrounded(Catalog.stationBatteryProp, parent, "PowerCell", basePos + new Vector3(-0.3f, 0f, -1.05f), Quaternion.Euler(0f, 90f, 0f), 0.85f, 1.3f, false);
            }
        }

        private void SpawnAmbientDockWorker(Transform parent, int index, Vector3 floorPos, Quaternion rot, string label)
        {
            if (Catalog == null || Catalog.ambientWorkerModels == null || Catalog.ambientWorkerModels.Count == 0)
                return;
            int pick = index % Catalog.ambientWorkerModels.Count;
            GameObject mdl = Catalog.ambientWorkerModels[pick];
            if (mdl == null) return;
            Texture albedo = (Catalog.ambientWorkerAlbedos != null && pick < Catalog.ambientWorkerAlbedos.Count)
                ? Catalog.ambientWorkerAlbedos[pick]
                : null;
            PrefabFittingHelper.SpawnGrounded(mdl, parent, label, floorPos, rot, targetHeightM: 1.78f, maxDimensionM: 2.0f, keepColliders: false, overrideAlbedo: albedo);
        }

        private static List<(float start, float end)> SubtractIntervals(float min, float max, List<(float start, float end)> cuts)
        {
            var result = new List<(float start, float end)> { (min, max) };
            if (cuts == null || cuts.Count == 0)
                return result;

            var sorted = new List<(float start, float end)>(cuts);
            sorted.Sort((a, b) => a.start.CompareTo(b.start));

            for (int c = 0; c < sorted.Count; c++)
            {
                float cutStart = sorted[c].start;
                float cutEnd = sorted[c].end;
                if (cutEnd <= cutStart) continue;

                var next = new List<(float start, float end)>(result.Count + 1);
                for (int i = 0; i < result.Count; i++)
                {
                    float s = result[i].start;
                    float e = result[i].end;
                    if (cutEnd <= s || cutStart >= e)
                    {
                        next.Add((s, e));
                    }
                    else
                    {
                        if (cutStart > s + 0.05f) next.Add((s, cutStart));
                        if (cutEnd < e - 0.05f) next.Add((cutEnd, e));
                    }
                }
                result = next;
                if (result.Count == 0) break;
            }
            return result;
        }

        private void BuildHorizontalWallIfCanonical(
            Transform parent,
            AtlasRoomDef room,
            float yCoord,
            bool isSouthOfRoom,
            float elev,
            float height,
            Color wallTint,
            bool isRockCut,
            List<(int floor, float yWall, float centerX, AtlasExitDef exit)> hDoors)
        {
            // Subtract ONLY the exact spans owned by adjacent south neighbors (prevents both double walls AND partial-overlap voids!).
            var neighborCuts = new List<(float start, float end)>();
            if (isSouthOfRoom)
            {
                foreach (var other in Pack.Rooms.Values)
                {
                    if (other != room && other.FloorIndex == room.FloorIndex &&
                        Mathf.Abs(other.MaxY - yCoord) < 0.2f)
                    {
                        float ovMin = Mathf.Max(room.MinX, other.MinX);
                        float ovMax = Mathf.Min(room.MaxX, other.MaxX);
                        if (ovMax - ovMin > 0.2f)
                            neighborCuts.Add((ovMin, ovMax));
                    }
                }
            }

            var ownedSpans = SubtractIntervals(room.MinX, room.MaxX, neighborCuts);
            if (ownedSpans.Count == 0)
                return;

            // Collect ALL doorways along this horizontal wall edge (multi-door walls like Stickney Concourse cut every door!).
            float doorHeight = 3.1f;
            var doorIntervals = new List<(float start, float end)>();
            foreach (var d in hDoors)
            {
                if (d.floor == room.FloorIndex && Mathf.Abs(d.yWall - yCoord) < 0.25f &&
                    d.centerX > room.MinX + 0.5f && d.centerX < room.MaxX - 0.5f)
                {
                    float halfW = (d.exit.Mode == "open" ? 7.5f : 4.2f) * 0.5f;
                    float ds = Mathf.Max(room.MinX + 0.1f, d.centerX - halfW);
                    float de = Mathf.Min(room.MaxX - 0.1f, d.centerX + halfW);
                    if (de - ds > 0.5f)
                        doorIntervals.Add((ds, de));
                }
            }

            int segIdx = 0;
            int lintelIdx = 0;
            float lintelH = Mathf.Max(0.3f, height - doorHeight);

            for (int s = 0; s < ownedSpans.Count; s++)
            {
                float spanMin = ownedSpans[s].start;
                float spanMax = ownedSpans[s].end;
                var spanDoors = new List<(float start, float end)>();
                for (int d = 0; d < doorIntervals.Count; d++)
                {
                    float ds = Mathf.Max(spanMin, doorIntervals[d].start);
                    float de = Mathf.Min(spanMax, doorIntervals[d].end);
                    if (de - ds > 0.4f)
                        spanDoors.Add((ds, de));
                }

                var solidSegs = SubtractIntervals(spanMin, spanMax, spanDoors);
                for (int k = 0; k < solidSegs.Count; k++)
                {
                    float w = solidSegs[k].end - solidSegs[k].start;
                    if (w <= 0.15f) continue;
                    string segName = solidSegs.Count == 1 && spanDoors.Count == 0 ? "Wall_H" : $"Wall_H_Seg_{segIdx++}";
                    CreateBox(parent, segName, AtlasToUnity((solidSegs[k].start + solidSegs[k].end) * 0.5f, elev + height * 0.5f, yCoord),
                        new Vector3(w, height, 0.32f), GetWallMaterialForSpan(room.EntryId, wallTint, w, height, isRockCut), true);
                }

                for (int d = 0; d < spanDoors.Count; d++)
                {
                    float dw = spanDoors[d].end - spanDoors[d].start;
                    float dcx = (spanDoors[d].start + spanDoors[d].end) * 0.5f;
                    string lintelName = lintelIdx == 0 ? "Wall_H_Lintel" : $"Wall_H_Lintel_{lintelIdx}";
                    lintelIdx++;
                    CreateBox(parent, lintelName, AtlasToUnity(dcx, elev + doorHeight + lintelH * 0.5f, yCoord),
                        new Vector3(dw, lintelH, 0.32f), GetWallMaterialForSpan(room.EntryId, wallTint, dw, lintelH, isRockCut), true);
                }
            }
        }

        private void BuildVerticalWallIfCanonical(
            Transform parent,
            AtlasRoomDef room,
            float xCoord,
            bool isWestOfRoom,
            float elev,
            float height,
            Color wallTint,
            bool isRockCut,
            List<(int floor, float xWall, float centerY, AtlasExitDef exit)> vDoors)
        {
            // Subtract ONLY the exact spans owned by adjacent west neighbors.
            var neighborCuts = new List<(float start, float end)>();
            if (isWestOfRoom)
            {
                foreach (var other in Pack.Rooms.Values)
                {
                    if (other != room && other.FloorIndex == room.FloorIndex &&
                        Mathf.Abs(other.MaxX - xCoord) < 0.2f)
                    {
                        float ovMin = Mathf.Max(room.MinY, other.MinY);
                        float ovMax = Mathf.Min(room.MaxY, other.MaxY);
                        if (ovMax - ovMin > 0.2f)
                            neighborCuts.Add((ovMin, ovMax));
                    }
                }
            }

            var ownedSpans = SubtractIntervals(room.MinY, room.MaxY, neighborCuts);
            if (ownedSpans.Count == 0)
                return;

            // Collect ALL doorways along this vertical wall edge.
            float doorHeight = 3.1f;
            var doorIntervals = new List<(float start, float end)>();
            foreach (var d in vDoors)
            {
                if (d.floor == room.FloorIndex && Mathf.Abs(d.xWall - xCoord) < 0.25f &&
                    d.centerY > room.MinY + 0.5f && d.centerY < room.MaxY - 0.5f)
                {
                    float halfW = (d.exit.Mode == "open" ? 8.0f : 4.2f) * 0.5f;
                    float ds = Mathf.Max(room.MinY + 0.1f, d.centerY - halfW);
                    float de = Mathf.Min(room.MaxY - 0.1f, d.centerY + halfW);
                    if (de - ds > 0.5f)
                        doorIntervals.Add((ds, de));
                }
            }

            int segIdx = 0;
            int lintelIdx = 0;
            float lintelH = Mathf.Max(0.3f, height - doorHeight);

            for (int s = 0; s < ownedSpans.Count; s++)
            {
                float spanMin = ownedSpans[s].start;
                float spanMax = ownedSpans[s].end;
                var spanDoors = new List<(float start, float end)>();
                for (int d = 0; d < doorIntervals.Count; d++)
                {
                    float ds = Mathf.Max(spanMin, doorIntervals[d].start);
                    float de = Mathf.Min(spanMax, doorIntervals[d].end);
                    if (de - ds > 0.4f)
                        spanDoors.Add((ds, de));
                }

                var solidSegs = SubtractIntervals(spanMin, spanMax, spanDoors);
                for (int k = 0; k < solidSegs.Count; k++)
                {
                    float len = solidSegs[k].end - solidSegs[k].start;
                    if (len <= 0.15f) continue;
                    string segName = solidSegs.Count == 1 && spanDoors.Count == 0 ? "Wall_V" : $"Wall_V_Seg_{segIdx++}";
                    CreateBox(parent, segName, AtlasToUnity(xCoord, elev + height * 0.5f, (solidSegs[k].start + solidSegs[k].end) * 0.5f),
                        new Vector3(0.32f, height, len), GetWallMaterialForSpan(room.EntryId, wallTint, len, height, isRockCut), true);
                }

                for (int d = 0; d < spanDoors.Count; d++)
                {
                    float dw = spanDoors[d].end - spanDoors[d].start;
                    float dcy = (spanDoors[d].start + spanDoors[d].end) * 0.5f;
                    string lintelName = lintelIdx == 0 ? "Wall_V_Lintel" : $"Wall_V_Lintel_{lintelIdx}";
                    lintelIdx++;
                    CreateBox(parent, lintelName, AtlasToUnity(xCoord, elev + doorHeight + lintelH * 0.5f, dcy),
                        new Vector3(0.32f, lintelH, dw), GetWallMaterialForSpan(room.EntryId, wallTint, dw, lintelH, isRockCut), true);
                }
            }
        }

        private void BuildPortalTriggers(
            List<(int floor, float yWall, float centerX, AtlasExitDef exit)> hDoors,
            List<(int floor, float xWall, float centerY, AtlasExitDef exit)> vDoors,
            List<AtlasExitDef> interFloorExits,
            List<AtlasExitDef> nonAdjacentExits)
        {
            var portalsRoot = new GameObject("ExitPortals").transform;
            portalsRoot.SetParent(transform, false);
            Material portalMat = UrpMaterialFactory.GetEmissive("DoorFrameGlow", new Color(0.1f, 0.35f, 0.5f), new Color(0.15f, 0.75f, 1.0f) * 1.1f);

            foreach (var d in hDoors)
            {
                if (!Pack.Rooms.TryGetValue(d.exit.FromRoom, out var rA) || !Pack.Rooms.TryGetValue(d.exit.ToRoom, out var rB))
                    continue;
                float elev = Pack.GetFloorElevationY(d.floor);
                Vector3 pos = AtlasToUnity(d.centerX, elev + 1.5f, d.yWall);
                Vector3 axis = rB.Center2D.y > rA.Center2D.y ? Vector3.forward : Vector3.back;
                CreatePortalTrigger(portalsRoot, d.exit, pos, new Vector3(4.0f, 3.0f, 1.6f), axis, portalMat, horizontal: true);
            }

            foreach (var d in vDoors)
            {
                if (!Pack.Rooms.TryGetValue(d.exit.FromRoom, out var rA) || !Pack.Rooms.TryGetValue(d.exit.ToRoom, out var rB))
                    continue;
                float elev = Pack.GetFloorElevationY(d.floor);
                Vector3 pos = AtlasToUnity(d.xWall, elev + 1.5f, d.centerY);
                Vector3 axis = rB.Center2D.x > rA.Center2D.x ? Vector3.right : Vector3.left;
                CreatePortalTrigger(portalsRoot, d.exit, pos, new Vector3(1.6f, 3.0f, 4.0f), axis, portalMat, horizontal: false);
            }

            // Spawn explicit transit portal arches for non-adjacent same-floor exits (e.g. OOC exit.hall_simulator_lobby)
            if (nonAdjacentExits != null)
            {
                for (int i = 0; i < nonAdjacentExits.Count; i++)
                {
                    var ex = nonAdjacentExits[i];
                    if (!Pack.Rooms.TryGetValue(ex.FromRoom, out var rA) || !Pack.Rooms.TryGetValue(ex.ToRoom, out var rB))
                        continue;
                    SpawnTransitPortalArch(portalsRoot, ex, rA, rB, isForward: true, portalMat);
                    SpawnTransitPortalArch(portalsRoot, ex, rB, rA, isForward: false, portalMat);
                }
            }

            // Build walkable 3D steel staircases / spoke ramps for every inter-floor exit in the pack
            Material stairMat = UrpMaterialFactory.GetOpaque("SteelStair", new Color(0.28f, 0.30f, 0.33f), 0.5f, 0.7f);
            if (interFloorExits != null)
            {
                for (int s = 0; s < interFloorExits.Count; s++)
                {
                    var ex = interFloorExits[s];
                    if (!Pack.Rooms.TryGetValue(ex.FromRoom, out var rA) || !Pack.Rooms.TryGetValue(ex.ToRoom, out var rB))
                        continue;

                    ComputeInterFloorStairGeometry(ex, rA, rB, out Vector2 topXY, out Vector2 botXY, out float elevTop, out float elevBot, out bool runAlongX, out float stairWidth);
                    var stairRoot = new GameObject($"InterFloorStair_{ex.EntryId}").transform;
                    stairRoot.SetParent(transform, false);
                    int steps = 18;
                    Vector3 stepScale = runAlongX ? new Vector3(0.72f, 0.34f, stairWidth) : new Vector3(stairWidth, 0.34f, 0.72f);
                    for (int i = 0; i < steps; i++)
                    {
                        float t = (i + 0.5f) / steps;
                        float y = Mathf.Lerp(elevTop, elevBot, t);
                        float ax = Mathf.Lerp(topXY.x, botXY.x, t);
                        float ay = Mathf.Lerp(topXY.y, botXY.y, t);
                        CreateBox(stairRoot, $"Step_{i}", AtlasToUnity(ax, y, ay), stepScale, stairMat, true);
                    }

                    // Architectural dark-steel stair side stringers & handrails (no glowing cyan doorway posts on stairs!)
                    Material railMat = UrpMaterialFactory.GetOpaque("StairRailSteel", new Color(0.20f, 0.23f, 0.27f), 0.32f, 0.65f);
                    Vector3 topMid = AtlasToUnity(topXY.x, elevTop + 0.85f, topXY.y);
                    Vector3 botMid = AtlasToUnity(botXY.x, elevBot + 0.85f, botXY.y);
                    Vector3 spanDelta = botMid - topMid;
                    float spanLen = spanDelta.magnitude;
                    if (spanLen > 0.5f)
                    {
                        Quaternion slopeRot = Quaternion.LookRotation(spanDelta.normalized, Vector3.up);
                        Vector3 midCenter = (topMid + botMid) * 0.5f;
                        Vector3 sideOffset = runAlongX
                            ? new Vector3(0f, 0f, stairWidth * 0.5f + 0.06f)
                            : new Vector3(stairWidth * 0.5f + 0.06f, 0f, 0f);
                        var railL = CreateBox(stairRoot, "StairRail_L", midCenter - sideOffset, new Vector3(0.10f, 0.12f, spanLen), railMat, false);
                        railL.transform.rotation = slopeRot;
                        var railR = CreateBox(stairRoot, "StairRail_R", midCenter + sideOffset, new Vector3(0.10f, 0.12f, spanLen), railMat, false);
                        railR.transform.rotation = slopeRot;
                    }

                    Vector2 midXY = (topXY + botXY) * 0.5f;
                    Vector3 trigAxis = (botMid - topMid).sqrMagnitude > 0.01f ? (botMid - topMid).normalized : (Vector3.down + Vector3.back).normalized;
                    Vector3 trigSize = runAlongX ? new Vector3(3.0f, 3.0f, stairWidth) : new Vector3(stairWidth, 3.0f, 3.0f);
                    CreatePortalTrigger(
                        portalsRoot,
                        ex,
                        AtlasToUnity(midXY.x, (elevTop + elevBot) * 0.5f, midXY.y),
                        trigSize,
                        trigAxis,
                        portalMat,
                        horizontal: !runAlongX);
                }
            }
        }

        private void SpawnTransitPortalArch(Transform parent, AtlasExitDef ex, AtlasRoomDef srcRoom, AtlasRoomDef dstRoom, bool isForward, Material portalMat)
        {
            float elev = Pack.GetFloorElevationY(srcRoom.FloorIndex);
            Vector2 delta = dstRoom.Center2D - srcRoom.Center2D;
            float px, py;
            bool onVerticalWall = Mathf.Abs(delta.x) >= Mathf.Abs(delta.y);
            if (onVerticalWall)
            {
                px = delta.x >= 0f ? srcRoom.MaxX - 1.15f : srcRoom.MinX + 1.15f;
                py = srcRoom.Center2D.y;
            }
            else
            {
                px = srcRoom.Center2D.x;
                py = delta.y >= 0f ? srcRoom.MaxY - 1.15f : srcRoom.MinY + 1.15f;
            }

            Vector3 floorPos = AtlasToUnity(px, elev, py);
            string suffix = isForward ? "Fwd" : "Rev";
            var root = new GameObject($"TransitPortal_{ex.EntryId}_{suffix}");
            root.transform.SetParent(parent, false);
            root.transform.position = floorPos;
            root.transform.localScale = Vector3.one;

            var bc = root.AddComponent<BoxCollider>();
            bc.isTrigger = true;
            bc.center = new Vector3(0f, 1.5f, 0f);
            bc.size = onVerticalWall ? new Vector3(1.6f, 3.0f, 3.2f) : new Vector3(3.2f, 3.0f, 1.6f);

            // Visual arch posts + lintel + destination sign
            Vector3 postOff = onVerticalWall ? new Vector3(0f, 1.5f, 1.4f) : new Vector3(1.4f, 1.5f, 0f);
            Vector3 postMirror = onVerticalWall ? new Vector3(0f, 1.5f, -1.4f) : new Vector3(-1.4f, 1.5f, 0f);
            CreateBox(root.transform, "TransitPost_A", floorPos + postOff, new Vector3(0.18f, 3.0f, 0.18f), portalMat, false);
            CreateBox(root.transform, "TransitPost_B", floorPos + postMirror, new Vector3(0.18f, 3.0f, 0.18f), portalMat, false);
            Vector3 lintelScale = onVerticalWall ? new Vector3(0.22f, 0.22f, 3.0f) : new Vector3(3.0f, 0.22f, 0.22f);
            CreateBox(root.transform, "TransitLintel", floorPos + new Vector3(0f, 3.05f, 0f), lintelScale, portalMat, false);

            string label = isForward ? ex.ForwardLabel : ex.ReverseLabel;
            string cmd = TacSimContentLoader.FormatSafeExitCommand(isForward ? ex.ForwardCommand : ex.ReverseCommand);
            string revCmd = TacSimContentLoader.FormatSafeExitCommand(isForward ? ex.ReverseCommand : ex.ForwardCommand);
            float signYaw = onVerticalWall ? (delta.x >= 0f ? 90f : -90f) : (delta.y >= 0f ? 0f : 180f);
            CreateWorldSign(root.transform, $"TRANSIT -> {(label ?? dstRoom.Name).ToUpperInvariant()}", floorPos + new Vector3(0f, 3.45f, 0f), Quaternion.Euler(0f, signYaw, 0f), 1.9f, new Color(0.45f, 0.95f, 1f));

            var trig = root.AddComponent<ExitPortalTrigger>();
            trig.roomA = srcRoom.EntryId;
            trig.roomB = dstRoom.EntryId;
            trig.commandAToB = cmd ?? string.Empty;
            trig.commandBToA = revCmd ?? string.Empty;
            trig.labelAToB = label;
            trig.labelBToA = isForward ? ex.ReverseLabel : ex.ForwardLabel;
            trig.axisAToB = new Vector3(delta.x, 0f, delta.y).normalized;
            trig.isLocked = ex.Locked;
            trig.OnPortalCrossed = (p, target, c) => OnPortalCrossed?.Invoke(p, target, c);

            if (!string.IsNullOrEmpty(cmd))
            {
                var wi = root.AddComponent<WorldInteractable>();
                wi.kind = InteractableKind.GenericCommand;
                wi.promptTitle = $"Transit to {dstRoom.Name}";
                wi.promptSubtitle = $"Step through transit arch ({cmd})";
                wi.commandOrKey = cmd;
                wi.maxInteractDistance = 4.2f;
                wi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
            }
        }

        private void CreatePortalTrigger(Transform parent, AtlasExitDef ex, Vector3 pos, Vector3 boxSize, Vector3 axisAToB, Material frameMat, bool horizontal)
        {
            var go = new GameObject($"Portal_{ex.EntryId}");
            go.transform.SetParent(parent, false);
            go.transform.position = pos;

            var bc = go.AddComponent<BoxCollider>();
            bc.isTrigger = true;
            bc.size = boxSize;

            var trig = go.AddComponent<ExitPortalTrigger>();
            trig.roomA = ex.FromRoom;
            trig.roomB = ex.ToRoom;
            trig.commandAToB = TacSimContentLoader.FormatSafeExitCommand(ex.ForwardCommand) ?? string.Empty;
            trig.commandBToA = TacSimContentLoader.FormatSafeExitCommand(ex.ReverseCommand) ?? string.Empty;
            trig.labelAToB = ex.ForwardLabel;
            trig.labelBToA = ex.ReverseLabel;
            trig.axisAToB = axisAToB;
            trig.isLocked = ex.Locked;
            trig.OnPortalCrossed = (p, target, cmd) => OnPortalCrossed?.Invoke(p, target, cmd);

            bool isStairOrLadder = ex.Mode == "stair" || ex.Mode == "ladder";
            bool hasCommercialDoorPrefab = !isStairOrLadder && Catalog != null &&
                                           ((ex.Locked && Catalog.stationLockedDoor != null) ||
                                            (!ex.Locked && Catalog.stationDoorFrame != null));

            if (!isStairOrLadder && !hasCommercialDoorPrefab)
            {
                Material postMat = ex.Locked
                    ? UrpMaterialFactory.GetEmissive("LockedDoorGlow", new Color(0.55f, 0.14f, 0.12f), new Color(1.0f, 0.25f, 0.18f) * 1.1f)
                    : frameMat;
                Vector3 stripOffset = horizontal ? new Vector3(boxSize.x * 0.5f, 0f, 0f) : new Vector3(0f, 0f, boxSize.z * 0.5f);
                CreateBox(go.transform, "DoorPost_A", pos - stripOffset, new Vector3(0.14f, boxSize.y, 0.14f), postMat, false);
                CreateBox(go.transform, "DoorPost_B", pos + stripOffset, new Vector3(0.14f, boxSize.y, 0.14f), postMat, false);
            }

            // DOTTEAM Heavy Station Kit portal arch / locked bulkhead door dressing (auto-aligned flush in wall plane!)
            if (!isStairOrLadder)
            {
                Vector3 floorCenter = new Vector3(pos.x, pos.y - boxSize.y * 0.5f, pos.z);
                if (ex.Locked)
                {
                    // Always place a solid physics bulkhead blocker across locked doorways so the player avatar cannot walk into a restricted room
                    Vector3 blockerSize = horizontal ? new Vector3(4.3f, 3.2f, 0.36f) : new Vector3(0.36f, 3.2f, 4.3f);
                    Material sealedMat = UrpMaterialFactory.GetOpaque("SealedBulkheadLeaf", new Color(0.24f, 0.20f, 0.20f), new Color(0.24f, 0.20f, 0.20f).r, 0.65f);
                    CreateBox(go.transform, "SealedBulkheadBlocker", floorCenter + Vector3.up * 1.6f, blockerSize, sealedMat, withCollider: true);
                    if (Catalog != null && Catalog.stationLockedDoor != null)
                        SpawnAlignedDoorwayPrefab(Catalog.stationLockedDoor, go.transform, "LockedBulkheadDoor", floorCenter, horizontal, 3.1f, 4.2f, keepColliders: true);
                }
                else if (Catalog != null && Catalog.stationDoorFrame != null)
                {
                    SpawnAlignedDoorwayPrefab(Catalog.stationDoorFrame, go.transform, "HeavyPortalArch", floorCenter, horizontal, 3.35f, 5.4f, keepColliders: false);
                }

                SpawnDoorwayDestinationLintels(go.transform, ex, pos, boxSize, axisAToB, horizontal);
            }
        }

        private void SpawnDoorwayDestinationLintels(Transform portalRoot, AtlasExitDef ex, Vector3 pos, Vector3 boxSize, Vector3 axisAToB, bool horizontal)
        {
            if (Pack == null || ex == null) return;
            float lintelY = pos.y + boxSize.y * 0.5f + 0.34f;
            Vector3 normal = axisAToB.sqrMagnitude > 0.01f
                ? new Vector3(axisAToB.x, 0f, axisAToB.z).normalized
                : (horizontal ? Vector3.forward : Vector3.right);
            float backingWidth = horizontal ? 3.2f : 0.14f;
            float backingDepth = horizontal ? 0.14f : 3.2f;
            Material backingMat = UrpMaterialFactory.GetOpaque("DoorLintelBacking", new Color(0.05f, 0.08f, 0.12f), 0.25f, 0.45f);

            if (ex.Locked)
            {
                Color redCol = new Color(1.0f, 0.36f, 0.30f);
                Quaternion rotA = Quaternion.LookRotation(normal, Vector3.up);
                Quaternion rotB = Quaternion.LookRotation(-normal, Vector3.up);
                CreateBox(portalRoot, "LockedLintelStrip", new Vector3(pos.x, lintelY, pos.z), new Vector3(backingWidth, 0.42f, backingDepth), backingMat, false);
                CreateWorldSign(portalRoot, "RESTRICTED — SEALED", new Vector3(pos.x, lintelY, pos.z) - normal * 0.24f, rotA, 1.35f, redCol);
                CreateWorldSign(portalRoot, "RESTRICTED — SEALED", new Vector3(pos.x, lintelY, pos.z) + normal * 0.24f, rotB, 1.35f, redCol);
                DoorwaySignCount += 2;
                return;
            }

            string nameB = Pack.Rooms.TryGetValue(ex.ToRoom, out var roomB) && !string.IsNullOrWhiteSpace(roomB.Name) ? roomB.Name : ex.ForwardLabel;
            string nameA = Pack.Rooms.TryGetValue(ex.FromRoom, out var roomA) && !string.IsNullOrWhiteSpace(roomA.Name) ? roomA.Name : ex.ReverseLabel;
            Color signCol = new Color(0.45f, 0.94f, 1.0f);

            CreateBox(portalRoot, "DoorLintelBacking", new Vector3(pos.x, lintelY, pos.z), new Vector3(backingWidth, 0.42f, backingDepth), backingMat, false);

            // Side facing Room A points toward Room B; side facing Room B points toward Room A
            if (!string.IsNullOrWhiteSpace(nameB))
            {
                Quaternion rotFaceA = Quaternion.LookRotation(normal, Vector3.up);
                CreateWorldSign(portalRoot, $"-> {nameB.ToUpperInvariant()}", new Vector3(pos.x, lintelY, pos.z) - normal * 0.24f, rotFaceA, 1.45f, signCol);
                DoorwaySignCount++;
            }
            if (!string.IsNullOrWhiteSpace(nameA))
            {
                Quaternion rotFaceB = Quaternion.LookRotation(-normal, Vector3.up);
                CreateWorldSign(portalRoot, $"-> {nameA.ToUpperInvariant()}", new Vector3(pos.x, lintelY, pos.z) + normal * 0.24f, rotFaceB, 1.45f, signCol);
                DoorwaySignCount++;
            }
        }

        private static void SpawnAlignedDoorwayPrefab(
            GameObject prefab,
            Transform parent,
            string instanceName,
            Vector3 floorCenter,
            bool horizontalWallAlongX,
            float targetHeightM,
            float maxDimensionM,
            bool keepColliders)
        {
            var inst = PrefabFittingHelper.SpawnGrounded(prefab, parent, instanceName, floorCenter, Quaternion.identity, targetHeightM, maxDimensionM, keepColliders);
            if (inst == null) return;

            if (PrefabFittingHelper.TryComputeRendererBounds(inst, out Bounds b))
            {
                // A doorway frame/leaf is wider across the doorway opening than it is thick through the wall.
                // Align its wider horizontal axis with the wall tangent (X for horizontal walls, Z for vertical walls)!
                bool wideAlongZ = b.size.z > b.size.x;
                bool needsQuarterTurn = (horizontalWallAlongX && wideAlongZ) || (!horizontalWallAlongX && !wideAlongZ);
                if (needsQuarterTurn)
                {
                    inst.transform.rotation = Quaternion.Euler(0f, 90f, 0f);
                    if (PrefabFittingHelper.TryComputeRendererBounds(inst, out Bounds bRot))
                    {
                        inst.transform.position += new Vector3(floorCenter.x - bRot.center.x, floorCenter.y - bRot.min.y, floorCenter.z - bRot.center.z);
                    }
                }
            }
        }

        private void BuildCountersAndServiceDesks()
        {
            Material counterMat = UrpMaterialFactory.GetOpaque("Counter", new Color(0.26f, 0.22f, 0.19f), 0.55f, 0.35f);
            var roomsWithCounter = new HashSet<string>(StringComparer.Ordinal);
            foreach (var feat in Pack.Features)
            {
                if (feat.FeatureKind != "counter") continue;
                roomsWithCounter.Add(feat.RoomId);
                float elev = Pack.GetFloorElevationY(feat.FloorIndex);
                Vector3 a = AtlasToUnity(feat.PointA.x, elev + feat.HeightM * 0.5f, feat.PointA.y);
                Vector3 b = AtlasToUnity(feat.PointB.x, elev + feat.HeightM * 0.5f, feat.PointB.y);
                Vector3 mid = (a + b) * 0.5f;
                float len = Vector3.Distance(a, b);
                var counterGo = CreateBox(transform, $"Counter_{feat.EntryId}", mid, new Vector3(Mathf.Max(1.5f, len * 0.45f), feat.HeightM, 1.05f), counterMat, true);

                if (Catalog != null)
                {
                    Vector3 counterTop = new Vector3(mid.x, elev + feat.HeightM, mid.z);
                    if (Catalog.stationConsoleProp != null)
                        SpawnWallAlignedConsoleProp(Catalog.stationConsoleProp, transform, $"DeskConsole_{feat.EntryId}", counterTop + new Vector3(-0.45f, 0f, 0f), inwardYawDeg: 180f, targetHeightM: 0.68f, maxDimensionM: 1.1f);
                    if (Catalog.stationDisplayProp != null)
                        SpawnWallAlignedConsoleProp(Catalog.stationDisplayProp, transform, $"DeskDisplay_{feat.EntryId}", counterTop + new Vector3(0.55f, 0f, 0f), inwardYawDeg: 180f, targetHeightM: 0.58f, maxDimensionM: 0.95f);
                }

                AttachServiceDeskInteractable(counterGo, feat.RoomId);
            }

            // Ensure service rooms in multi-port packs (Stickney, Aitken, Voltaire, Ligeia) without polyline features still get an interactive desk!
            foreach (var kv in Pack.Rooms)
            {
                if (roomsWithCounter.Contains(kv.Key) || kv.Value.Restricted) continue;
                if (!HasServiceDeskRole(kv.Key)) continue;

                var r = kv.Value;
                float elev = Pack.GetFloorElevationY(r.FloorIndex);
                Vector3 deskPos = AtlasToUnity(r.Center2D.x, elev + 0.55f, r.MaxY - 2.4f);
                var counterGo = CreateBox(transform, $"PortDesk_{r.EntryId}", deskPos, new Vector3(3.4f, 1.1f, 1.05f), counterMat, true);
                AttachServiceDeskInteractable(counterGo, r.EntryId);
            }

            // Berth Gallery Airlock Boarding Hatch across ANY corridor port!
            string galleryEntry = Pack.GetBerthGalleryRoomEntry();
            if (!string.IsNullOrEmpty(galleryEntry) && Pack.Rooms.TryGetValue(galleryEntry, out var gallery))
            {
                float elev = Pack.GetFloorElevationY(gallery.FloorIndex);
                float lockAtlasX = galleryEntry == "room.c01_berth_gallery" ? 125f : gallery.MinX + Mathf.Min(15f, gallery.Size2D.x * 0.3f);
                Vector3 lockPos = AtlasToUnity(lockAtlasX, elev, gallery.MinY + 1.2f);
                var lockRoot = new GameObject("BerthAirlockBoardingGate");
                lockRoot.transform.SetParent(transform, false);
                lockRoot.transform.position = lockPos;

                Material lockMat = UrpMaterialFactory.GetOpaque("AirlockRing", new Color(0.25f, 0.30f, 0.36f), 0.5f, 0.7f);
                Material greenBeacon = UrpMaterialFactory.GetEmissive("AirlockReady", new Color(0.1f, 0.5f, 0.25f), new Color(0.2f, 1.0f, 0.45f) * 2.0f);
                CreateBox(lockRoot.transform, "CollarFrame", lockPos + Vector3.up * 1.6f, new Vector3(2.8f, 3.2f, 0.6f), lockMat, true);
                if (Catalog != null && Catalog.shipAirlockDoorPrefab != null)
                {
                    PrefabFittingHelper.SpawnGrounded(Catalog.shipAirlockDoorPrefab, lockRoot.transform, "VattalusAirlockHatch", lockPos + new Vector3(0f, 0f, 0.22f), Quaternion.identity, 3.15f, 3.6f, keepColliders: false);
                }
                CreateBox(lockRoot.transform, "SealBeacon", lockPos + new Vector3(0f, 3.35f, 0.25f), new Vector3(1.8f, 0.22f, 0.22f), greenBeacon, false);
                CreateWorldSign(lockRoot.transform, "CRADLE 01 — UMBILICAL LOCK", lockPos + new Vector3(0f, 3.75f, 0.3f), Quaternion.Euler(0f, 180f, 0f), 2.5f, new Color(0.35f, 1f, 0.65f));

                _airlockBoardInteractable = lockRoot.AddComponent<WorldInteractable>();
                _airlockBoardInteractable.kind = InteractableKind.AirlockBoard;
                _airlockBoardInteractable.promptTitle = "Cycle Airlock & Board Ship";
                _airlockBoardInteractable.promptSubtitle = "Step aboard your chartered hull (+ship/board)";
                _airlockBoardInteractable.commandOrKey = "+ship/board";
                _airlockBoardInteractable.maxInteractDistance = 4.5f;
                _airlockBoardInteractable.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
            }
        }

        private static bool HasServiceDeskRole(string roomId)
        {
            if (string.IsNullOrEmpty(roomId)) return false;
            string low = roomId.ToLowerInvariant();
            return low.Contains("registry") || low.Contains("exchange") || low.Contains("chandlery") ||
                   low.Contains("commissary") || low.Contains("victualling") || low.Contains("clearing") ||
                   low.Contains("dockmaster") || low.Contains("harbourmaster") || low.Contains("service");
        }

        private void AttachServiceDeskInteractable(GameObject counterGo, string roomId)
        {
            string title = null;
            string subtitle = null;
            string cmd = null;
            string low = (roomId ?? string.Empty).ToLowerInvariant();

            if (low.Contains("registry"))
            {
                title = "Registry & Co-op Desk";
                subtitle = "Charter terms, prospecting licences (+licence), or hull enrolment";
                cmd = "desk:coop";
            }
            else if (low.Contains("exchange") || low.Contains("receiving"))
            {
                title = $"{Pack.Name} Commodity Exchange";
                subtitle = "Browse live volatiles, concentrate, ore & alloy lots";
                cmd = "desk:exchange";
            }
            else if (low.Contains("chandlery") || low.Contains("commissary") || low.Contains("victualling"))
            {
                title = "Stores, Chandlery & Bunkering";
                subtitle = "Provisions (crew-days) & reaction mass schedule";
                cmd = "desk:chandlery";
            }
            else if (low.Contains("clearing"))
            {
                title = "Clearing House Teller";
                subtitle = "Check Luna-mirrored credit balance (+credit)";
                cmd = "+credit";
            }
            else if (low.Contains("dockmaster") || low.Contains("harbourmaster"))
            {
                title = "Harbourmaster / Sailing-Plan Counter";
                subtitle = "Review corridor departures & captain's standing orders";
                cmd = "desk:dockmaster";
            }
            else if (low.Contains("service"))
            {
                title = "Yard Refit & Service Desk";
                subtitle = "Refit Dray-P Prospector (+coop/refit prospector), yard works (+works) & rig status (+tv)";
                cmd = "desk:refit";
            }

            if (title == null) return;
            var wi = counterGo.AddComponent<WorldInteractable>();
            wi.kind = InteractableKind.StationServiceDesk;
            wi.promptTitle = title;
            wi.promptSubtitle = subtitle;
            wi.commandOrKey = cmd;
            wi.maxInteractDistance = 4.2f;
            wi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
        }

        private void BuildOocSimulatorCentreProps()
        {
            if (LoadedPackId != "neighborhood.ooc_area") return;

            // 1. Simulator Lobby: Red vs Blue Match Selector Console + Return to IC Port Gate (unscaled (1,1,1) roots so TMP signs never stretch!)
            if (Pack.Rooms.TryGetValue("room.simulator_lobby", out var lobby))
            {
                float elev = Pack.GetFloorElevationY(lobby.FloorIndex);
                Vector3 selPos = AtlasToUnity(lobby.Center2D.x + 2.5f, elev, lobby.Center2D.y);

                Material consoleMat = UrpMaterialFactory.GetOpaque("SimLobbyDesk", new Color(0.14f, 0.18f, 0.24f), 0.6f, 0.5f);
                Material redGlow = UrpMaterialFactory.GetEmissive("SimRedBtn", new Color(0.85f, 0.18f, 0.16f), new Color(1.0f, 0.25f, 0.20f) * 2.0f);
                Material blueGlow = UrpMaterialFactory.GetEmissive("SimBlueBtn", new Color(0.16f, 0.45f, 0.92f), new Color(0.22f, 0.58f, 1.0f) * 2.0f);

                var selDesk = new GameObject("SimulatorSelectorConsole");
                selDesk.transform.SetParent(transform, false);
                selDesk.transform.position = selPos;
                selDesk.transform.localScale = Vector3.one;
                CreateBox(selDesk.transform, "SelectorDeskBody", selPos + Vector3.up * 0.58f, new Vector3(3.4f, 1.16f, 1.4f), consoleMat, true);
                CreateBox(selDesk.transform, "RedTeamBeacon", selPos + new Vector3(-0.9f, 1.24f, 0f), new Vector3(0.9f, 0.16f, 0.6f), redGlow, false);
                CreateBox(selDesk.transform, "BlueTeamBeacon", selPos + new Vector3(0.9f, 1.24f, 0f), new Vector3(0.9f, 0.16f, 0.6f), blueGlow, false);
                CreateWorldSign(selDesk.transform, "RED vs BLUE SIMULATOR SELECTOR", selPos + new Vector3(0f, 2.05f, 0f), Quaternion.Euler(0f, 180f, 0f), 2.4f, new Color(1f, 0.88f, 0.45f));
                var selCol = selDesk.AddComponent<BoxCollider>();
                selCol.center = new Vector3(0f, 0.58f, 0f);
                selCol.size = new Vector3(3.4f, 1.16f, 1.4f);

                var wi = selDesk.AddComponent<WorldInteractable>();
                wi.kind = InteractableKind.StationServiceDesk;
                wi.promptTitle = "Open Red vs Blue Simulator Console";
                wi.promptSubtitle = "Pick Red/Blue Team, Tank/Biped Unit, AI Wingmen & Start Match";
                wi.commandOrKey = "desk:sim_lobby";
                wi.maxInteractDistance = 4.6f;
                wi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);

                // Return to IC Port kiosk (unscaled (1,1,1) root)
                Vector3 icPos = AtlasToUnity(lobby.MinX + 2.4f, elev, lobby.MinY + 2.4f);
                var icStand = new GameObject("ReturnToIcKiosk");
                icStand.transform.SetParent(transform, false);
                icStand.transform.position = icPos;
                icStand.transform.localScale = Vector3.one;
                CreateBox(icStand.transform, "IcKioskBody", icPos + Vector3.up * 0.9f, new Vector3(1.2f, 1.8f, 0.6f),
                    UrpMaterialFactory.GetEmissive("IcGate", new Color(0.12f, 0.42f, 0.34f), new Color(0.20f, 0.90f, 0.65f) * 1.3f), true);
                CreateWorldSign(icStand.transform, "RETURN TO IC PORT (+IC)", icPos + new Vector3(0f, 2.25f, 0f), Quaternion.identity, 2.0f, new Color(0.45f, 1f, 0.78f));
                var icCol = icStand.AddComponent<BoxCollider>();
                icCol.center = new Vector3(0f, 0.9f, 0f);
                icCol.size = new Vector3(1.2f, 1.8f, 0.6f);
                var icWi = icStand.AddComponent<WorldInteractable>();
                icWi.kind = InteractableKind.GenericCommand;
                icWi.promptTitle = "Return to IC Corridor Port";
                icWi.promptSubtitle = "Step back to your stamped IC berth (+ic)";
                icWi.commandOrKey = "+ic";
                icWi.maxInteractDistance = 4.0f;
                icWi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
            }

            // 2. Spawn 6 Red Sim Pods in room.sim_red & 6 Blue Sim Pods in room.sim_blue!
            SpawnTeamSimPods("room.sim_red", isRed: true);
            SpawnTeamSimPods("room.sim_blue", isRed: false);
        }

        private void SpawnTeamSimPods(string roomEntryId, bool isRed)
        {
            if (!Pack.Rooms.TryGetValue(roomEntryId, out var room)) return;
            float elev = Pack.GetFloorElevationY(room.FloorIndex);

            Color teamCol = isRed ? new Color(0.86f, 0.20f, 0.18f) : new Color(0.18f, 0.50f, 0.94f);
            Material podShell = UrpMaterialFactory.GetOpaque(isRed ? "RedPodShell" : "BluePodShell", new Color(0.19f, 0.22f, 0.26f), 0.52f, 0.65f);
            Material stripeMat = UrpMaterialFactory.GetEmissive(isRed ? "RedPodStripe" : "BluePodStripe", teamCol, teamCol * 2.1f);
            string teamWord = isRed ? "RED" : "BLUE";

            // Bold team wall stripe across back wall
            float stripeY = isRed ? room.MaxY - 0.22f : room.MinY + 0.22f;
            CreateBox(transform, $"{teamWord}_WallBanner", AtlasToUnity(room.Center2D.x, elev + 2.4f, stripeY), new Vector3(Mathf.Max(4f, room.Size2D.x - 1.5f), 0.45f, 0.12f), stripeMat, false);

            for (int i = 0; i < 6; i++)
            {
                int slot = i + 1;
                int colIdx = i % 3;
                int rowIdx = i / 3;
                float px = Mathf.Lerp(room.MinX + 4.2f, room.MaxX - 4.2f, colIdx / 2f);
                float py = Mathf.Lerp(room.MinY + 4.5f, room.MaxY - 4.5f, rowIdx == 0 ? 0.22f : 0.78f);
                Vector3 podPos = AtlasToUnity(px, elev, py);

                var podGo = new GameObject($"{teamWord}_Pod_{slot}");
                podGo.transform.SetParent(transform, false);
                podGo.transform.position = podPos;
                podGo.transform.localScale = Vector3.one;
                CreateBox(podGo.transform, "PodShell", podPos + Vector3.up * 1.35f, new Vector3(2.1f, 2.7f, 2.3f), podShell, true);
                CreateBox(podGo.transform, "TeamChevron", podPos + new Vector3(0f, 2.45f, 1.18f), new Vector3(1.65f, 0.28f, 0.08f), stripeMat, false);
                CreateWorldSign(podGo.transform, $"{teamWord} {slot}", podPos + new Vector3(0f, 3.05f, 0f), Quaternion.identity, 2.6f, teamCol);
                var podCol = podGo.AddComponent<BoxCollider>();
                podCol.center = new Vector3(0f, 1.35f, 0f);
                podCol.size = new Vector3(2.1f, 2.7f, 2.3f);

                var wi = podGo.AddComponent<WorldInteractable>();
                wi.kind = InteractableKind.SimulatorPodBoard;
                wi.promptTitle = $"Climb Into {teamWord} Pod {slot}";
                wi.promptSubtitle = "Board armored cockpit pod & launch 3D Arena (BOARD)";
                wi.commandOrKey = $"pod:{teamWord.ToLowerInvariant()}:{slot}";
                wi.maxInteractDistance = 3.8f;
                wi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
            }
        }

        private void BuildExteriorBerthCradles()
        {
            string galleryEntry = Pack.GetBerthGalleryRoomEntry();
            float galleryMinY = 110f;
            float galleryMinX = 110f;
            float galleryMaxX = 210f;
            float galleryElev = 4.5f;
            if (!string.IsNullOrEmpty(galleryEntry) && Pack.Rooms.TryGetValue(galleryEntry, out var gal))
            {
                galleryMinY = gal.MinY;
                galleryMinX = gal.MinX;
                galleryMaxX = gal.MaxX;
                galleryElev = Pack.GetFloorElevationY(gal.FloorIndex) - 0.6f;
            }

            var berthsRoot = new GameObject("ExteriorFloodlitCradles").transform;
            berthsRoot.SetParent(transform, false);

            Material cradleMat = UrpMaterialFactory.GetOpaque("CradleTruss", new Color(0.20f, 0.23f, 0.28f), 0.38f, 0.72f);
            Material darkCradleMat = UrpMaterialFactory.GetOpaque("UnlitSquadronTruss", new Color(0.06f, 0.08f, 0.11f), 0.25f, 0.6f);
            Material hullMat = UrpMaterialFactory.GetOpaque("DrayHullPlate", new Color(0.48f, 0.46f, 0.42f), 0.38f, 0.55f);
            Material beaconRed = UrpMaterialFactory.GetEmissive("BerthNavLight", new Color(0.9f, 0.2f, 0.15f), new Color(1f, 0.25f, 0.18f) * 2.2f);
            Material starMat = UrpMaterialFactory.GetEmissive("BerthBackdropStar", Color.white, new Color(0.85f, 0.92f, 1.0f) * 2.4f);

            // Backdrop starfield pinpoints beyond the cradles so the window clearly opens onto deep space
            var starsRoot = new GameObject("ExteriorVacuumStarfield").transform;
            starsRoot.SetParent(berthsRoot, false);
            var rng = new System.Random(2350);
            for (int s = 0; s < 65; s++)
            {
                float sx = Mathf.Lerp(galleryMinX - 35f, galleryMaxX + 35f, (float)rng.NextDouble());
                float sy = galleryElev - 6f + (float)rng.NextDouble() * 24f;
                float sz = galleryMinY - 42f - (float)rng.NextDouble() * 55f;
                float r = 0.09f + (float)rng.NextDouble() * 0.14f;
                CreateBox(starsRoot, $"Star_{s}", AtlasToUnity(sx, sy, sz), Vector3.one * r, starMat, false);
            }

            string[] defaultNames = { "CRADLE 01 — SLACK WATER", "CRADLE 02 — COLD COMFORT", "CRADLE 03 — NINTH LEDGER", "CRADLE 04 — PATIENCE WEN", "CRADLE 05 — TALLY HO" };
            Material boomSteel = UrpMaterialFactory.GetOpaque("MooringBoomSteel", new Color(0.16f, 0.19f, 0.23f), 0.35f, 0.72f);
            int cradleCount = 5;
            for (int i = 0; i < cradleCount; i++)
            {
                float cx = Mathf.Lerp(galleryMinX + 12f, galleryMaxX - 16f, i / (float)(cradleCount - 1));
                Vector3 cPos = AtlasToUnity(cx, galleryElev + 1.8f, galleryMinY - 22.0f);
                if (Catalog != null && Catalog.shipyardCradlePrefab != null)
                {
                    PrefabFittingHelper.SpawnCentered(Catalog.shipyardCradlePrefab, berthsRoot, $"ShipyardCradle_{i}", cPos, Quaternion.identity, 26f, keepColliders: false);
                }
                else
                {
                    // Open skeletal mooring cradle frame when commercial shipyard prefabs are absent
                    CreateBox(berthsRoot, $"CradleKeel_{i}", cPos + new Vector3(0f, -4.2f, 0f), new Vector3(1.2f, 0.6f, 22f), cradleMat, false);
                    CreateBox(berthsRoot, $"CradleRibFore_{i}", cPos + new Vector3(0f, -1.2f, 7.5f), new Vector3(11.5f, 0.5f, 0.7f), cradleMat, false);
                    CreateBox(berthsRoot, $"CradleRibAft_{i}", cPos + new Vector3(0f, -1.2f, -7.5f), new Vector3(11.5f, 0.5f, 0.7f), cradleMat, false);
                }

                // Sleek dark-steel docking collar boom connecting station exterior bulkhead to cradle bow (no giant orange blocks!)
                Vector3 stationAnchor = AtlasToUnity(cx, galleryElev + 0.8f, galleryMinY - 5.2f);
                CreateBox(berthsRoot, $"DockingBoom_{i}", stationAnchor, new Vector3(0.42f, 0.34f, 9.6f), boomSteel, false);
                CreateBox(berthsRoot, $"DockingCollarRing_{i}", stationAnchor + new Vector3(0f, 0f, -4.8f), new Vector3(1.35f, 1.15f, 0.35f), cradleMat, false);
                CreateBox(berthsRoot, $"CradleBeacon_{i}", stationAnchor + new Vector3(-0.62f, 0.58f, -4.8f), new Vector3(0.14f, 0.14f, 0.14f), beaconRed, false);

                // Hard floodlight illuminating the moored hull so it stands out brightly through the glass!
                var floodGo = new GameObject($"CradleFloodlight_{i}");
                floodGo.transform.SetParent(berthsRoot, false);
                floodGo.transform.position = cPos + new Vector3(3.5f, 8.5f, 11.5f);
                var pt = floodGo.AddComponent<Light>();
                pt.type = LightType.Point;
                pt.range = 36f;
                pt.intensity = 7.5f;
                pt.color = new Color(0.86f, 0.94f, 1.0f);

                GameObject hullPrefab = (i % 2 == 0 && Catalog != null && Catalog.drayExteriorHullPrefab != null)
                    ? Catalog.drayExteriorHullPrefab
                    : (i == 4 && Catalog != null && Catalog.corvetteExteriorPrefab != null)
                    ? Catalog.corvetteExteriorPrefab
                    : (Catalog != null ? Catalog.visitingFreighterPrefab : null);

                Quaternion shipYaw = Quaternion.Euler(0f, (i % 2 == 0) ? -14f : 12f, 0f);
                GameObject shipGo = hullPrefab != null
                    ? PrefabFittingHelper.SpawnCentered(hullPrefab, berthsRoot, $"CradledHull_{i + 1}", cPos, shipYaw, 20f, keepColliders: false)
                    : BuildSculptedDrayExterior(berthsRoot, cPos, hullMat, cradleMat, $"CradledDray_{i + 1}");

                if (i == 0 && shipGo != null) _dockedShipVisual = shipGo.transform;
                _cradleHullObjects.Add(shipGo);

                // Hull callsign sign framed cleanly above each moored ship
                var signTmp = CreateTmp3D(berthsRoot, $"CradleHullSign_{i}", Vector3.zero, defaultNames[i], 2.7f, new Color(0.72f, 0.94f, 1f));
                signTmp.rectTransform.sizeDelta = new Vector2(16f, 2.2f);
                signTmp.transform.position = cPos + new Vector3(0f, 7.4f, 9.5f);
                signTmp.transform.rotation = Quaternion.Euler(0f, 180f, 0f);
                _cradleHullSigns.Add(signTmp);

                if (i < 2 && Catalog != null && Catalog.collectorDronePrefab != null)
                {
                    var drone = PrefabFittingHelper.SpawnCentered(
                        Catalog.collectorDronePrefab,
                        berthsRoot,
                        $"YardTenderDrone_{i}",
                        cPos + new Vector3(7.4f, 3.6f, 6.0f),
                        Quaternion.Euler(0f, -125f, 0f),
                        3.6f,
                        keepColliders: false);
                    if (drone != null) _berthDrones.Add((drone.transform, drone.transform.position.y));
                }
            }

            // "The dark": distant unlit working & Squadron berths continuing beyond the floodlit cradles (using real shipyard gantries, never flat grey slabs!)
            for (int u = 0; u < 2; u++)
            {
                Vector3 darkPos = AtlasToUnity(galleryMaxX + 16f + u * 24f, galleryElev + 1.0f, galleryMinY - 28f - u * 10f);
                if (Catalog != null && Catalog.shipyardCradlePrefab != null)
                {
                    PrefabFittingHelper.SpawnCentered(Catalog.shipyardCradlePrefab, berthsRoot, $"UnlitSquadronCradle_{u}", darkPos, Quaternion.Euler(0f, -10f, 0f), 24f, keepColliders: false);
                }
                else
                {
                    CreateBox(berthsRoot, $"UnlitTrussSpine_{u}", darkPos + new Vector3(0f, -3.5f, 0f), new Vector3(0.8f, 0.5f, 18f), darkCradleMat, false);
                    CreateBox(berthsRoot, $"UnlitTrussArch_{u}", darkPos + new Vector3(0f, 2.5f, 4f), new Vector3(9.5f, 0.45f, 0.45f), darkCradleMat, false);
                }
            }

            // Animated 3D Exterior Port Traffic Shuttles / Dockyard Tugs transiting the deep-space orbital lane beyond the cradles
            BuildExteriorPortTrafficShuttles(berthsRoot, galleryMinX, galleryMaxX, galleryMinY, galleryElev);
        }

        private void BuildExteriorPortTrafficShuttles(Transform berthsRoot, float galleryMinX, float galleryMaxX, float galleryMinY, float galleryElev)
        {
            var trafficRoot = new GameObject("ExteriorPortTraffic").transform;
            trafficRoot.SetParent(berthsRoot, false);

            Material shuttleHull = UrpMaterialFactory.GetOpaque("PortShuttleHull", new Color(0.56f, 0.60f, 0.66f), 0.45f, 0.60f);
            Material shuttleTrim = UrpMaterialFactory.GetOpaque("PortShuttleTrim", new Color(0.22f, 0.26f, 0.32f), 0.50f, 0.70f);
            Material driveGlow = UrpMaterialFactory.GetEmissive("PortShuttlePlume", new Color(0.32f, 0.88f, 1.0f), new Color(0.40f, 0.96f, 1.0f) * 2.4f);
            Material portRed = UrpMaterialFactory.GetEmissive("PortShuttleNavRed", new Color(1f, 0.22f, 0.18f), new Color(1f, 0.3f, 0.22f) * 2.2f);
            Material stbdGreen = UrpMaterialFactory.GetEmissive("PortShuttleNavGrn", new Color(0.20f, 1f, 0.42f), new Color(0.28f, 1f, 0.50f) * 2.2f);

            float laneMinX = galleryMinX - 55f;
            float laneMaxX = galleryMaxX + 55f;
            for (int i = 0; i < 3; i++)
            {
                bool eastbound = (i % 2) == 0;
                float startX = eastbound ? laneMinX : laneMaxX;
                float endX = eastbound ? laneMaxX : laneMinX;
                float baseY = galleryElev + 3.2f + i * 3.8f;
                float baseZ = galleryMinY - 38f - i * 8.5f;

                var root = new GameObject($"PortTrafficShuttle_{i + 1}").transform;
                root.SetParent(trafficRoot, false);
                Vector3 initPos = AtlasToUnity(Mathf.Lerp(startX, endX, i * 0.33f), baseY, baseZ);
                root.position = initPos;
                root.rotation = Quaternion.identity;

                float tailSign = eastbound ? -1f : 1f;
                CreateBox(root, "HullCore", initPos, new Vector3(4.6f, 1.0f, 1.5f), shuttleHull, false);
                CreateBox(root, "WingPods", initPos, new Vector3(1.8f, 0.42f, 3.4f), shuttleTrim, false);
                CreateBox(root, "DrivePlume", initPos + new Vector3(tailSign * 2.8f, 0f, 0f), new Vector3(1.4f, 0.52f, 0.62f), driveGlow, false);
                CreateBox(root, "NavLight_Port", initPos + new Vector3(0f, 0.2f, eastbound ? 1.8f : -1.8f), Vector3.one * 0.22f, portRed, false);
                CreateBox(root, "NavLight_Stbd", initPos + new Vector3(0f, 0.2f, eastbound ? -1.8f : 1.8f), Vector3.one * 0.22f, stbdGreen, false);

                _exteriorTraffic.Add(new ExteriorPortShuttle
                {
                    Root = root,
                    StartX = startX,
                    EndX = endX,
                    BaseY = baseY,
                    BaseZ = baseZ,
                    SpeedMps = 6.5f + i * 2.4f,
                    PhaseOffset = i * 0.33f
                });
            }
        }

        private void Update()
        {
            for (int i = 0; i < _berthDrones.Count; i++)
            {
                var entry = _berthDrones[i];
                if (entry.tr == null) continue;
                Vector3 p = entry.tr.position;
                p.y = entry.baseY + Mathf.Sin(Time.time * 1.35f + i * 2.1f) * 0.14f;
                entry.tr.position = p;
            }

            for (int i = 0; i < _exteriorTraffic.Count; i++)
            {
                var sh = _exteriorTraffic[i];
                if (sh?.Root == null) continue;
                float span = Mathf.Max(20f, Mathf.Abs(sh.EndX - sh.StartX));
                float u = Mathf.Repeat(sh.PhaseOffset + (Time.time * sh.SpeedMps) / span, 1f);
                float curX = Mathf.Lerp(sh.StartX, sh.EndX, u);
                float curY = sh.BaseY + Mathf.Sin(Time.time * 0.65f + i * 1.9f) * 0.65f;
                sh.Root.position = AtlasToUnity(curX, curY, sh.BaseZ);
            }

            float dt = Mathf.Max(0.001f, Time.deltaTime);
            foreach (var kv in _spawnedActors)
            {
                var actorGo = kv.Value;
                if (actorGo == null || !_actorTargetPositions.TryGetValue(kv.Key, out Vector3 targetPos))
                    continue;

                Vector3 curPos = actorGo.transform.position;
                Vector3 delta = targetPos - curPos;
                float dist = delta.magnitude;
                _actorSpawners.TryGetValue(kv.Key, out var spawner);

                if (dist > 0.02f)
                {
                    float stepSpeed = 3.4f;
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

            if (_actorQuestMarkers.Count > 0)
            {
                int idx = 0;
                foreach (var kv in _actorQuestMarkers)
                {
                    if (kv.Value != null)
                    {
                        float bob = Mathf.Sin(Time.time * 3.0f + idx * 0.9f) * 0.08f;
                        kv.Value.transform.localPosition = new Vector3(0f, 0.78f + bob, 0f);
                    }
                    idx++;
                }
            }
        }

        public static GameObject BuildSculptedDrayExterior(Transform parent, Vector3 pos, Material hullMat, Material trimMat, string name)
        {
            var root = new GameObject(name);
            root.transform.SetParent(parent, false);
            root.transform.position = pos;
            root.transform.localRotation = Quaternion.identity;

            // Build all structural sub-parts in LOCAL space so rotating parent/cRoot rotates the entire Dray cleanly!
            CreateLocalBox(root.transform, "HullSpine", Vector3.zero, new Vector3(6.2f, 5.2f, 20f), hullMat, false);
            CreateLocalBox(root.transform, "BowCollar", new Vector3(0f, 0.2f, 11.2f), new Vector3(3.8f, 3.8f, 2.6f), trimMat, false);
            CreateLocalBox(root.transform, "PortPod", new Vector3(-4.6f, -0.3f, -1f), new Vector3(3.0f, 4.2f, 13f), hullMat, false);
            CreateLocalBox(root.transform, "StarboardPod", new Vector3(4.6f, -0.3f, -1f), new Vector3(3.0f, 4.2f, 13f), hullMat, false);
            CreateLocalBox(root.transform, "BridgeSuperstructure", new Vector3(0f, 3.4f, 5.5f), new Vector3(4.6f, 2.0f, 5.6f), trimMat, false);
            CreateLocalBox(root.transform, "DriveBell", new Vector3(0f, 0f, -11.8f), new Vector3(4.2f, 4.2f, 3.6f), trimMat, false);
            return root;
        }

        public bool TryMatchStationRoom(MushRoomView rv, out string entryId)
        {
            entryId = null;
            if (rv == null) return false;

            // Dynamically stream any corridor port OR OOC Simulator Centre neighborhood pack when RoomView arrives!
            string incomingPack = rv.Atlas != null && !string.IsNullOrEmpty(rv.Atlas.Pack)
                ? rv.Atlas.Pack
                : (rv.Atlas != null ? rv.Atlas.NeighborhoodPack : null);

            if (string.IsNullOrEmpty(incomingPack) && !string.IsNullOrEmpty(rv.Name))
            {
                string rn = rv.Name.Trim();
                if (string.Equals(rn, "Simulator Lobby", StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(rn, "Red Room", StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(rn, "Blue Room", StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(rn, "Welcome Hall", StringComparison.OrdinalIgnoreCase))
                {
                    incomingPack = "neighborhood.ooc_area";
                }
            }

            if (!string.IsNullOrEmpty(incomingPack) &&
                incomingPack.StartsWith("neighborhood.", StringComparison.OrdinalIgnoreCase) &&
                !string.Equals(LoadedPackId, incomingPack, StringComparison.OrdinalIgnoreCase))
            {
                BuildStation(Catalog, incomingPack);
            }

            if (rv.Atlas != null && !string.IsNullOrEmpty(rv.Atlas.Entry) &&
                (Pack == null || Pack.Rooms.ContainsKey(rv.Atlas.Entry)))
            {
                entryId = rv.Atlas.Entry;
                return true;
            }

            // Fallback name match for servers not yet publishing ambient["atlas"]
            if (Pack != null && !string.IsNullOrEmpty(rv.Name))
            {
                string trimmed = rv.Name.Trim();
                foreach (var kv in Pack.Rooms)
                {
                    if (string.Equals(kv.Value.Name, trimmed, StringComparison.OrdinalIgnoreCase))
                    {
                        entryId = kv.Key;
                        return true;
                    }
                }
            }
            return false;
        }

        public string ResolveEntryIdForRoomView(MushRoomView rv)
        {
            return TryMatchStationRoom(rv, out string matched) ? matched : ActiveRoomEntryId;
        }

        public void HighlightActiveRoom(string roomEntryId)
        {
            if (string.IsNullOrEmpty(roomEntryId)) return;
            ActiveRoomEntryId = roomEntryId;
            foreach (var kv in _roomLights)
            {
                bool active = kv.Key == ActiveRoomEntryId;
                kv.Value.intensity = active ? 4.4f : 2.1f;
            }
        }

        public void ApplyRoomView(MushRoomView rv, string selfActorId = null, bool updateHighlight = true)
        {
            if (rv == null) return;
            bool matched = TryMatchStationRoom(rv, out string resolvedEntry);
            if (matched && updateHighlight)
                HighlightActiveRoom(resolvedEntry);

            // Update Berth Gallery airlock prompt with player's live chartered hull name!
            if (rv.TacSim != null && rv.TacSim.Mode == "berth_gallery" && _airlockBoardInteractable != null)
            {
                if (!string.IsNullOrEmpty(rv.TacSim.OwnHullName))
                {
                    _airlockBoardInteractable.promptTitle = rv.TacSim.OwnHullDockedHere
                        ? $"Board {rv.TacSim.OwnHullName}"
                        : $"{rv.TacSim.OwnHullName} (Not Cradled Here)";
                    _airlockBoardInteractable.promptSubtitle = rv.TacSim.OwnHullDockedHere
                        ? $"Cradled at {rv.TacSim.PortEntry} — Press [E] to cycle airlock (+ship/board)"
                        : "Enrol or recall your hull at the Co-op Desk";
                }
                if (_dockedShipVisual != null)
                    _dockedShipVisual.gameObject.SetActive(rv.TacSim.DockedCount > 0 || rv.TacSim.OwnHullDockedHere);
            }

            // Sync visible NPCs & remote players in the room `rv` actually describes (filtering out disabled/dormant placeholders)
            SyncRoomOccupants(rv, selfActorId, matched ? resolvedEntry : ActiveRoomEntryId);
            ApplyQuestMarkers(rv.QuestMarkers);
        }

        /// <summary>
        /// Renders or clears floating WoW-style 3D overhead quest markers above live NPCs:
        /// Gold '!' (available), Gold '?' (ready for turn-in), Silver '?' (in progress).
        /// Also updates the NPC's `[E]` interaction banner prompt so players immediately see quest actions.
        /// </summary>
        public void ApplyQuestMarkers(IReadOnlyList<MushNpcQuestMarker> markers)
        {
            var incoming = new Dictionary<string, MushNpcQuestMarker>(StringComparer.Ordinal);
            if (markers != null)
            {
                for (int i = 0; i < markers.Count; i++)
                {
                    var m = markers[i];
                    if (m != null && !string.IsNullOrEmpty(m.NpcId))
                        incoming[m.NpcId] = m;
                }
            }

            foreach (var kv in _spawnedActors)
            {
                string actorId = kv.Key;
                GameObject actorGo = kv.Value;
                if (actorGo == null) continue;

                incoming.TryGetValue(actorId, out var marker);
                if (marker == null && markers != null && _actorDisplayNames.TryGetValue(actorId, out string dispName) && !string.IsNullOrEmpty(dispName))
                {
                    for (int i = 0; i < markers.Count; i++)
                    {
                        if (markers[i] != null && string.Equals(markers[i].NpcName, dispName, StringComparison.OrdinalIgnoreCase))
                        {
                            marker = markers[i];
                            break;
                        }
                    }
                }

                var wi = actorGo.GetComponent<WorldInteractable>();
                string rawActorName = _actorDisplayNames.TryGetValue(actorId, out string n) ? n : string.Empty;
                string safeActorName = MushProtocol.EscapeUnityRichText(rawActorName);

                if (marker != null && !string.IsNullOrEmpty(marker.Symbol))
                {
                    _actorQuestMarkerStates[actorId] = marker;
                    if (_actorBillboards.TryGetValue(actorId, out var bb) && bb != null)
                    {
                        if (!_actorQuestMarkers.TryGetValue(actorId, out var tmp) || tmp == null)
                        {
                            tmp = CreateTmp3D(bb.transform, "QuestMarker", new Vector3(0f, 0.78f, 0f), marker.Symbol, 3.2f, Color.yellow, allowRichText: false);
                            tmp.fontStyle = FontStyles.Bold;
                            tmp.rectTransform.sizeDelta = new Vector2(1.4f, 1.4f);
                            _actorQuestMarkers[actorId] = tmp;
                        }
                        bool isGold = string.Equals(marker.Marker, "available", StringComparison.OrdinalIgnoreCase) ||
                                      string.Equals(marker.Marker, "turn_in", StringComparison.OrdinalIgnoreCase);
                        tmp.text = marker.Symbol;
                        tmp.color = isGold ? new Color(1.0f, 0.84f, 0.16f, 1f) : new Color(0.74f, 0.82f, 0.90f, 1f);
                        tmp.gameObject.SetActive(true);
                    }

                    if (wi != null)
                    {
                        string roleTag = string.Equals(marker.Marker, "turn_in", StringComparison.OrdinalIgnoreCase)
                            ? "Turn In Quest"
                            : string.Equals(marker.Marker, "available", StringComparison.OrdinalIgnoreCase)
                                ? "Quest Available"
                                : "Quest In Progress";
                        wi.promptTitle = $"[{marker.Symbol}] {safeActorName} — {roleTag}";
                        if (!string.IsNullOrEmpty(marker.QuestTitle))
                            wi.promptSubtitle = MushProtocol.EscapeUnityRichText(marker.QuestTitle);
                    }
                }
                else
                {
                    bool hadMarker = _actorQuestMarkerStates.Remove(actorId);
                    if (_actorQuestMarkers.TryGetValue(actorId, out var oldTmp))
                    {
                        if (oldTmp != null)
                        {
                            if (Application.isPlaying) Destroy(oldTmp.gameObject);
                            else DestroyImmediate(oldTmp.gameObject);
                        }
                        _actorQuestMarkers.Remove(actorId);
                    }
                    if (hadMarker && wi != null && !string.IsNullOrEmpty(safeActorName))
                    {
                        wi.promptTitle = $"Talk to {safeActorName}";
                    }
                }
            }
        }

        public void ResetSessionState()
        {
            ClearAllSpawnedActors();
            _spawnedActorsRoomEntry = null;
            _lastBerthTradeHulls.Clear();
            _hasAuthoritativeBerthHulls = false;
            _roomBoardLines.Clear();
            ApplyCradleLabels(Array.Empty<string>());
        }

        public void ApplyBerthDockedHulls(
            IReadOnlyList<TacSimDockedHullRef> hulls,
            string ownHullId = null,
            string ownHullName = null,
            bool ownHullDockedHere = false)
        {
            _hasAuthoritativeBerthHulls = true;
            _lastBerthTradeHulls.Clear();
            if (hulls != null)
            {
                for (int i = 0; i < hulls.Count; i++)
                {
                    if (hulls[i] != null)
                        _lastBerthTradeHulls.Add(hulls[i]);
                }
            }

            var labels = new List<string>();
            if (ownHullDockedHere && !string.IsNullOrEmpty(ownHullName))
                labels.Add($"CRADLE 01 — {MushProtocol.EscapeUnityRichText(ownHullName.ToUpperInvariant())} [YOUR HULL]");

            for (int i = 0; i < _lastBerthTradeHulls.Count; i++)
            {
                var dh = _lastBerthTradeHulls[i];
                if (string.IsNullOrEmpty(dh.Name)) continue;
                string upper = MushProtocol.EscapeUnityRichText(dh.Name.ToUpperInvariant());
                bool dup = false;
                for (int m = 0; m < labels.Count; m++)
                {
                    if (labels[m].IndexOf(upper, StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        dup = true;
                        break;
                    }
                }
                if (!dup)
                {
                    string markSuffix = !string.IsNullOrEmpty(dh.Mark) ? $" ({MushProtocol.EscapeUnityRichText(dh.Mark)})" : string.Empty;
                    bool isOwn = !string.IsNullOrEmpty(ownHullId) && string.Equals(dh.Id, ownHullId, StringComparison.Ordinal);
                    string ownTag = isOwn ? " [YOUR HULL]" : string.Empty;
                    labels.Add($"CRADLE {labels.Count + 1:D2} — {upper}{markSuffix}{ownTag}");
                }
            }

            ApplyCradleLabels(labels);
        }

        private void ApplyCradleLabels(IReadOnlyList<string> mooredLabelsForCradles)
        {
            int count = mooredLabelsForCradles != null ? mooredLabelsForCradles.Count : 0;
            for (int c = 0; c < _cradleHullSigns.Count; c++)
            {
                bool hasShip = c < count;
                if (_cradleHullObjects.Count > c && _cradleHullObjects[c] != null)
                    _cradleHullObjects[c].SetActive(hasShip);
                if (_cradleHullSigns[c] != null)
                {
                    _cradleHullSigns[c].gameObject.SetActive(hasShip);
                    if (hasShip) _cradleHullSigns[c].text = MushProtocol.EscapeUnityRichText(mooredLabelsForCradles[c]);
                }
            }
            if (_dockedShipVisual != null)
                _dockedShipVisual.gameObject.SetActive(count > 0);
        }

        public void AppendBoardLine(string line)
        {
            if (string.IsNullOrWhiteSpace(line)) return;
            if (_roomBoards.TryGetValue(ActiveRoomEntryId, out var tmp) && tmp != null)
            {
                if (!_roomBoardLines.TryGetValue(ActiveRoomEntryId, out var lines))
                {
                    lines = new List<string>();
                    _roomBoardLines[ActiveRoomEntryId] = lines;
                }
                lines.Add(MushProtocol.EscapeUnityRichText(line.Trim()));
                while (lines.Count > 8)
                    lines.RemoveAt(0);
                tmp.text = string.Join("\n", lines);
            }
        }

        public void ShowActorSpeech(string actorId, string actorName, string text)
        {
            string safeText = MushProtocol.EscapeUnityRichText(text);
            if (!string.IsNullOrEmpty(actorId) && _actorBillboards.TryGetValue(actorId, out var bb) && bb != null)
            {
                bb.ShowSpeech(safeText);
                return;
            }
            if (!string.IsNullOrWhiteSpace(actorName))
            {
                string trimmed = actorName.Trim();
                foreach (var kv in _actorDisplayNames)
                {
                    if (string.Equals(kv.Value, trimmed, StringComparison.OrdinalIgnoreCase))
                    {
                        if (_actorBillboards.TryGetValue(kv.Key, out var b2) && b2 != null)
                            b2.ShowSpeech(safeText);
                        break;
                    }
                }
            }
        }

        public void SetActorComposing(string actorId, string actorName, bool composing)
        {
            SetActorTransientBubble(actorId, actorName, composing, "...", 8.5f);
        }

        public void SetActorThinking(string actorId, string actorName, bool thinking)
        {
            SetActorTransientBubble(actorId, actorName, thinking, "thinking...", 18.0f);
        }

        public string GetActorSpeechText(string actorId)
        {
            if (!string.IsNullOrEmpty(actorId) &&
                _actorBillboards.TryGetValue(actorId, out var bb) &&
                bb != null && bb.IsSpeechActive)
            {
                return bb.CurrentSpeechText;
            }
            return string.Empty;
        }

        private void SetActorTransientBubble(string actorId, string actorName, bool active, string marker, float durationSec)
        {
            if (!string.IsNullOrEmpty(actorId) && _actorBillboards.TryGetValue(actorId, out var bb) && bb != null)
            {
                if (active)
                    bb.ShowSpeech(marker, durationSec);
                else if (bb.SpeechBubbleText != null && bb.SpeechBubbleText.text == marker)
                    bb.SpeechBubbleText.gameObject.SetActive(false);
                return;
            }
            if (!string.IsNullOrWhiteSpace(actorName))
            {
                string trimmed = actorName.Trim();
                foreach (var kv in _actorDisplayNames)
                {
                    if (string.Equals(kv.Value, trimmed, StringComparison.OrdinalIgnoreCase))
                    {
                        if (_actorBillboards.TryGetValue(kv.Key, out var b2) && b2 != null)
                        {
                            if (active) b2.ShowSpeech(marker, durationSec);
                            else if (b2.SpeechBubbleText != null && b2.SpeechBubbleText.text == marker) b2.SpeechBubbleText.gameObject.SetActive(false);
                        }
                        break;
                    }
                }
            }
        }

        private void ClearAllSpawnedActors()
        {
            foreach (var kv in _spawnedActors)
            {
                if (kv.Value != null)
                {
                    if (Application.isPlaying) Destroy(kv.Value);
                    else DestroyImmediate(kv.Value);
                }
            }
            _spawnedActors.Clear();
            _actorSpawners.Clear();
            _actorDisplayNames.Clear();
            _actorTargetPositions.Clear();
            _actorBillboards.Clear();
            _actorQuestMarkers.Clear();
            _actorQuestMarkerStates.Clear();
        }

        private void SyncRoomOccupants(MushRoomView rv, string selfActorId, string targetRoomEntry)
        {
            string lookupEntry = !string.IsNullOrEmpty(targetRoomEntry) ? targetRoomEntry : ActiveRoomEntryId;
            if (!Pack.Rooms.TryGetValue(lookupEntry, out var room))
            {
                Pack.Rooms.TryGetValue("room.c03_arrivals", out room);
                if (room == null)
                {
                    ClearAllSpawnedActors();
                    return;
                }
            }

            if (!string.Equals(_spawnedActorsRoomEntry, lookupEntry, StringComparison.Ordinal))
            {
                ClearAllSpawnedActors();
                _spawnedActorsRoomEntry = lookupEntry;
            }

            bool isBerthGallery = lookupEntry.IndexOf("berth_gallery", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                  (rv.TacSim != null && rv.TacSim.Mode == "berth_gallery");

            var dockedHullIds = new HashSet<string>(StringComparer.Ordinal);
            var dockedHullNames = ExtractBerthBoardHullNames(rv);
            var mooredLabelsForCradles = new List<string>();

            if (rv.TacSim != null)
            {
                if (rv.TacSim.OwnHullDockedHere && !string.IsNullOrEmpty(rv.TacSim.OwnHullName))
                {
                    string safeOwnHull = MushProtocol.EscapeUnityRichText(rv.TacSim.OwnHullName.ToUpperInvariant());
                    mooredLabelsForCradles.Add($"CRADLE 01 — {safeOwnHull} [YOUR HULL]");
                    if (!string.IsNullOrEmpty(rv.TacSim.OwnHull))
                        dockedHullIds.Add(rv.TacSim.OwnHull);
                }

                // Prefer structured `ambient.tacsim.docked_hulls` or live `port.trade.berth.docked_hulls` from mushd!
                IReadOnlyList<TacSimDockedHullRef> sourceHulls = rv.TacSim.DockedHulls.Count > 0
                    ? (IReadOnlyList<TacSimDockedHullRef>)rv.TacSim.DockedHulls
                    : (_hasAuthoritativeBerthHulls ? (IReadOnlyList<TacSimDockedHullRef>)_lastBerthTradeHulls : rv.TacSim.DockedHulls);
                int hullCap = Math.Min(sourceHulls.Count, MaxMooredCradleHulls);
                for (int h = 0; h < hullCap; h++)
                {
                    var dh = sourceHulls[h];
                    if (!string.IsNullOrEmpty(dh.Id)) dockedHullIds.Add(dh.Id);
                    if (!string.IsNullOrEmpty(dh.Name))
                    {
                        dockedHullNames.Add(dh.Name);
                        string upper = MushProtocol.EscapeUnityRichText(dh.Name.ToUpperInvariant());
                        bool dup = false;
                        for (int m = 0; m < mooredLabelsForCradles.Count; m++)
                        {
                            if (mooredLabelsForCradles[m].IndexOf(upper, StringComparison.OrdinalIgnoreCase) >= 0)
                            {
                                dup = true;
                                break;
                            }
                        }
                        if (!dup && mooredLabelsForCradles.Count < MaxMooredCradleHulls)
                        {
                            string markSuffix = !string.IsNullOrEmpty(dh.Mark) ? $" ({MushProtocol.EscapeUnityRichText(dh.Mark)})" : string.Empty;
                            mooredLabelsForCradles.Add($"CRADLE {mooredLabelsForCradles.Count + 1:D2} — {upper}{markSuffix}");
                        }
                    }
                }
            }

            bool hasAuthoritativeThingTyping = rv.ThingIds.Count > 0 || rv.PlayerIds.Count > 0;
            float elev = Pack.GetFloorElevationY(room.FloorIndex);
            var seenActorIds = new HashSet<string>(StringComparer.Ordinal);
            int actorIdx = 0;
            for (int i = 0; i < rv.Contents.Count; i++)
            {
                var item = rv.Contents[i];
                if (!string.IsNullOrEmpty(selfActorId) && item.Id == selfActorId)
                    continue;

                if (rv.DormantNpcIds.Contains(item.Id))
                    continue;

                var seedNpc = Pack.FindSeedNpcByName(item.Name);
                if (seedNpc != null && !seedNpc.Enabled)
                    continue;

                bool isNpc = rv.NpcIds.Contains(item.Id) || (seedNpc != null && seedNpc.Enabled);
                bool isExplicitThing = rv.ThingIds.Contains(item.Id);
                bool isExplicitPlayer = rv.PlayerIds.Contains(item.Id);
                bool isDockedShip = !isNpc && !isExplicitPlayer && (
                    dockedHullIds.Contains(item.Id) ||
                    dockedHullNames.Contains(item.Name) ||
                    (rv.TacSim != null && !string.IsNullOrEmpty(rv.TacSim.OwnHull) && item.Id == rv.TacSim.OwnHull) ||
                    LooksLikeShipOrVehicleThing(item.Name));

                if (isDockedShip)
                {
                    if (isBerthGallery && mooredLabelsForCradles.Count < MaxMooredCradleHulls)
                    {
                        string upper = MushProtocol.EscapeUnityRichText((item.Name ?? string.Empty).ToUpperInvariant());
                        bool alreadyAdded = false;
                        for (int m = 0; m < mooredLabelsForCradles.Count; m++)
                        {
                            if (mooredLabelsForCradles[m].IndexOf(upper, StringComparison.OrdinalIgnoreCase) >= 0)
                            {
                                alreadyAdded = true;
                                break;
                            }
                        }
                        if (!alreadyAdded)
                            mooredLabelsForCradles.Add($"CRADLE {mooredLabelsForCradles.Count + 1:D2} — {upper}");
                    }
                    continue;
                }

                // Never spawn inanimate Things (sim pods, selector consoles, hulls, plaques) as humanoid avatars!
                if (isExplicitThing)
                    continue;
                if (!isNpc && !isExplicitPlayer && (hasAuthoritativeThingTyping || isBerthGallery || LooksLikeStaticRoomThing(item.Name, room)))
                    continue;

                if (actorIdx >= MaxSpawnedRoomActors)
                    break;

                seenActorIds.Add(item.Id);
                Vector3 pos = ComputeNpcStandSpot(room, elev, actorIdx);
                actorIdx++;
                _actorTargetPositions[item.Id] = pos;

                // Reuse existing GameObject when still present in this room so `look` / `RoomView` updates
                // never destroy/recreate characters, flash T-poses, or drop active speech bubbles!
                if (_spawnedActors.TryGetValue(item.Id, out var existingGo) && existingGo != null)
                    continue;

                var npcGo = new GameObject($"Actor_{item.Id}_{item.Name}");
                npcGo.transform.SetParent(_actorsContainer, false);
                npcGo.transform.position = pos;
                npcGo.transform.rotation = Quaternion.Euler(0f, 180f, 0f);

                var cap = npcGo.AddComponent<CapsuleCollider>();
                cap.height = 1.8f;
                cap.radius = 0.38f;
                cap.center = new Vector3(0f, 0.9f, 0f);

                var spawner = npcGo.AddComponent<CharacterVisualSpawner>();
                bool isMale = seedNpc != null ? seedNpc.IsMale : TacSimContentLoader.InferIsMale(item.Name, null);
                spawner.BuildNpcVisual(Catalog, item.Name, isRemotePlayer: !isNpc, isMale: isMale);

                var bbGo = new GameObject("Billboard");
                bbGo.transform.SetParent(npcGo.transform, false);
                bbGo.transform.localPosition = new Vector3(0f, 2.08f, 0f);
                var bb = bbGo.AddComponent<WorldBillboard>();
                bb.AnchorLocalY = 2.08f;
                Vector3 roomMinW = AtlasToUnity(room.MinX, elev, room.MinY);
                Vector3 roomMaxW = AtlasToUnity(room.MaxX, elev, room.MaxY);
                bb.SetRoomSafeBounds(roomMinW.x, roomMaxW.x, roomMinW.z, roomMaxW.z, wallMarginM: 1.50f);

                string safeActorName = MushProtocol.EscapeUnityRichText(item.Name);
                bb.NameplateText = CreateTmp3D(bbGo.transform, "Nameplate", Vector3.zero, isNpc ? $"{safeActorName}\n<size=68%><color=#88CCFF>[NPC]</color></size>" : safeActorName, 1.32f, Color.white, allowRichText: isNpc);
                bb.NameplateText.rectTransform.sizeDelta = new Vector2(2.60f, 0.58f);

                // Bottom-anchored compact speech bubble so multi-line dialogue grows UPWARD above the nameplate without clipping into walls!
                bb.SpeechBubbleText = CreateTmp3D(bbGo.transform, "SpeechBubble", new Vector3(0f, 0.34f, 0f), "", 1.05f, new Color(1f, 0.95f, 0.65f), allowRichText: false);
                bb.SpeechBubbleText.rectTransform.pivot = new Vector2(0.5f, 0.0f);
                bb.SpeechBubbleText.rectTransform.sizeDelta = new Vector2(2.50f, 2.20f);
                bb.SpeechBubbleText.alignment = TextAlignmentOptions.Bottom;
                bb.SpeechBubbleText.gameObject.SetActive(false);

                var wi = npcGo.AddComponent<WorldInteractable>();
                wi.kind = InteractableKind.NpcActor;
                wi.isRemotePlayer = !isNpc;
                wi.actorId = item.Id ?? string.Empty;
                wi.promptTitle = $"Talk to {MushProtocol.EscapeUnityRichText(item.Name)}";
                wi.promptSubtitle = isNpc
                    ? (!string.IsNullOrEmpty(seedNpc?.ShortDesc) ? seedNpc.ShortDesc : "Speak in-character or ask about the port")
                    : "Greet pilot";
                wi.detailBody = seedNpc?.ShortDesc ?? string.Empty;
                wi.commandOrKey = MushProtocol.TryValidateSafeKeywordOrName(item.Name, out string safeNpcName) ? safeNpcName : string.Empty;
                wi.maxInteractDistance = 4.2f;
                wi.OnInteracted = target => OnInteractableTriggered?.Invoke(target);

                _spawnedActors[item.Id] = npcGo;
                _actorSpawners[item.Id] = spawner;
                _actorDisplayNames[item.Id] = item.Name ?? string.Empty;
                _actorBillboards[item.Id] = bb;
            }

            if (_spawnedActors.Count > seenActorIds.Count)
            {
                var departedIds = new List<string>();
                foreach (var kv in _spawnedActors)
                {
                    if (!seenActorIds.Contains(kv.Key))
                        departedIds.Add(kv.Key);
                }
                for (int i = 0; i < departedIds.Count; i++)
                {
                    string depId = departedIds[i];
                    if (_spawnedActors.TryGetValue(depId, out var depGo) && depGo != null)
                    {
                        if (Application.isPlaying) Destroy(depGo);
                        else DestroyImmediate(depGo);
                    }
                    _spawnedActors.Remove(depId);
                    _actorSpawners.Remove(depId);
                    _actorDisplayNames.Remove(depId);
                    _actorTargetPositions.Remove(depId);
                    _actorBillboards.Remove(depId);
                    _actorQuestMarkers.Remove(depId);
                    _actorQuestMarkerStates.Remove(depId);
                }
            }

            // Always update exterior 3D ship cradles when in Berth Gallery (including clearing all placeholder cradles on 0 hulls!).
            if (isBerthGallery && (rv.TacSim != null || _hasAuthoritativeBerthHulls || mooredLabelsForCradles.Count > 0))
            {
                ApplyCradleLabels(mooredLabelsForCradles);
            }
        }

        /// <summary>
        /// Returns all active targetable interactables in the current station room (live NPCs &amp; remote pilots first,
        /// followed by room terminals/plaques) for WoW/EQ [Tab] target cycling.
        /// </summary>
        public List<WorldInteractable> GetActiveRoomTargetables()
        {
            var result = new List<WorldInteractable>();
            var seen = new HashSet<WorldInteractable>();

            foreach (var kv in _spawnedActors)
            {
                if (kv.Value == null || !kv.Value.activeInHierarchy) continue;
                var wi = kv.Value.GetComponent<WorldInteractable>();
                if (wi != null && wi.isActiveAndEnabled && seen.Add(wi))
                    result.Add(wi);
            }

            if (!string.IsNullOrEmpty(ActiveRoomEntryId) && _roomRoots.TryGetValue(ActiveRoomEntryId, out var roomRoot) && roomRoot != null)
            {
                var roomItems = roomRoot.GetComponentsInChildren<WorldInteractable>(false);
                for (int i = 0; i < roomItems.Length; i++)
                {
                    if (roomItems[i] != null && roomItems[i].isActiveAndEnabled && seen.Add(roomItems[i]))
                        result.Add(roomItems[i]);
                }
            }
            return result;
        }

        private static HashSet<string> ExtractBerthBoardHullNames(MushRoomView rv)
        {
            var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            if (rv == null || string.IsNullOrEmpty(rv.Desc)) return names;

            string[] lines = rv.Desc.Split('\n');
            bool inBoard = false;
            for (int i = 0; i < lines.Length; i++)
            {
                string ln = lines[i].Trim();
                if (ln.StartsWith("Berth board:", StringComparison.OrdinalIgnoreCase) ||
                    ln.StartsWith("Departures board:", StringComparison.OrdinalIgnoreCase))
                {
                    inBoard = true;
                    continue;
                }
                if (inBoard)
                {
                    int slash = ln.IndexOf('/');
                    if (slash > 0)
                    {
                        string hullName = ln.Substring(0, slash).Trim();
                        if (!string.IsNullOrEmpty(hullName))
                            names.Add(hullName);
                    }
                    else if (string.IsNullOrEmpty(ln))
                    {
                        inBoard = false;
                    }
                }
            }
            return names;
        }

        private static bool LooksLikeShipOrVehicleThing(string name)
        {
            if (string.IsNullOrWhiteSpace(name)) return false;
            string low = name.ToLowerInvariant();
            return low.Contains("lifter") || low.Contains("dray") || low.Contains("corvette") ||
                   low.Contains("crawler") || low.Contains("prospector") || low.Contains("tender") ||
                   low.Contains("freighter") || low.Contains("hauler") || low.Contains("halcyon") ||
                   low == "slack water" || low == "cold comfort" || low == "ninth ledger" ||
                   low == "patience wen" || low == "kirkwood lamp" || low == "long division" ||
                   low == "dry season" || low == "tally ho";
        }

        private static bool LooksLikeStaticRoomThing(string name, AtlasRoomDef room)
        {
            if (string.IsNullOrWhiteSpace(name)) return true;
            if (room != null && room.Details.ContainsKey(name)) return true;
            string low = name.ToLowerInvariant();
            return low.StartsWith("the ") ||
                   low.Contains("cage") || low.Contains("manifold") || low.Contains("board") ||
                   low.Contains("pod ") || low.Contains("selector") || low.Contains("pamphlet") ||
                   low.Contains("box") || low.Contains("console") || low.Contains("terminal") ||
                   low.Contains("counter") || low.Contains("plaque") || low.Contains("ledger") ||
                   low.Contains("screen") || low.Contains("rack") || low.Contains("door") ||
                   low.Contains("window") || low.Contains("glass") || low.Contains("cradle");
        }

        private Vector3 ComputeNpcStandSpot(AtlasRoomDef room, float elev, int idx)
        {
            float safeMinX = room.MinX + 1.75f;
            float safeMaxX = room.MaxX - 1.75f;
            float safeMinY = room.MinY + 1.65f;
            float safeMaxY = room.MaxY - 1.85f; // keep clear of north-wall StatusBoardRoot

            // Stand behind the room's authored counter if one exists!
            foreach (var f in Pack.Features)
            {
                if (f.FeatureKind == "counter" && f.RoomId == room.EntryId)
                {
                    float x0 = Mathf.Max(safeMinX, Mathf.Min(f.PointA.x, f.PointB.x) + 1.4f);
                    float x1 = Mathf.Min(safeMaxX, Mathf.Max(f.PointA.x, f.PointB.x) - 1.4f);
                    // Bias first clerk toward east half of counter so they don't stand directly in front of the west-side StatusBoard!
                    float t = idx == 0 ? 0.68f : (((idx * 37) % 100) / 100f);
                    float cx = x1 > x0 ? Mathf.Lerp(x0, x1, t) : room.Center2D.x;
                    float behindY = f.PointA.y > room.Center2D.y ? f.PointA.y + 1.05f : f.PointA.y - 1.05f;
                    behindY = safeMinY < safeMaxY ? Mathf.Clamp(behindY, safeMinY, safeMaxY) : room.Center2D.y;
                    return AtlasToUnity(cx, elev, behindY);
                }
            }
            Vector2 c = room.Center2D;
            float angle = idx * 1.35f;
            float rx = safeMinX < safeMaxX ? Mathf.Clamp(c.x + Mathf.Cos(angle) * 2.8f, safeMinX, safeMaxX) : c.x;
            float ry = safeMinY < safeMaxY ? Mathf.Clamp(c.y + Mathf.Sin(angle) * 2.2f, safeMinY, safeMaxY) : c.y;
            return AtlasToUnity(rx, elev, ry);
        }

        private Material GetWallMaterialForSpan(string entryId, Color wallTint, float spanM, float heightM, bool isRockCut)
        {
            bool isFreight = entryId.IndexOf("dockmaster", StringComparison.OrdinalIgnoreCase) >= 0 ||
                             entryId.IndexOf("maintenance", StringComparison.OrdinalIgnoreCase) >= 0 ||
                             entryId.IndexOf("chandlery", StringComparison.OrdinalIgnoreCase) >= 0 ||
                             entryId.IndexOf("pressure", StringComparison.OrdinalIgnoreCase) >= 0;

            // Note: DOTTEAM Heavy Station Kit's `B2_Walls*_A.png` and `B2_Floors_A.png` are non-tileable UV-unwrap trim atlases
            // authored for custom 3D mesh UVs; use seamless 2K rock textures only on rock-cut ports, and use seamless procedural
            // architectural bulkhead/deck PBR + normal maps on pressurized station walls & floors!
            Texture alb = null;
            Texture nrm = null;
            if (isRockCut && Catalog != null && Catalog.stationRockAlbedo != null)
            {
                alb = Catalog.stationRockAlbedo;
                nrm = Catalog.stationRockNormal;
            }

            Color tinted = isFreight && !isRockCut
                ? new Color(wallTint.r * 0.98f, wallTint.g * 0.92f, wallTint.b * 0.84f, 1f)
                : wallTint;

            Vector2 tiling = new Vector2(Mathf.Max(1f, Mathf.Round(spanM / 4.2f)), Mathf.Max(1f, Mathf.Round(heightM / 4.5f)));
            return UrpMaterialFactory.GetTiledPbrSurface(
                $"Wall_{LoadedPackId}_{entryId}",
                alb,
                nrm,
                null,
                tinted,
                tiling,
                isFloor: false,
                smoothness: isRockCut ? 0.18f : 0.26f,
                metallic: isRockCut ? 0.08f : 0.36f);
        }

        private Material GetFloorMaterialForSlab(string entryId, float widthX, float depthZ)
        {
            Color floorTint;
            float smooth = 0.22f;
            float metal = 0.34f;
            switch (entryId)
            {
                case "room.c06_clearing":
                    floorTint = new Color(0.40f, 0.44f, 0.49f);
                    smooth = 0.28f;
                    metal = 0.28f;
                    break;
                case "room.c09_tycho":
                case "room.c16_authority":
                    floorTint = new Color(0.34f, 0.38f, 0.45f);
                    smooth = 0.18f;
                    metal = 0.16f;
                    break;
                case "room.c10_slack_line":
                    floorTint = new Color(0.32f, 0.26f, 0.24f);
                    smooth = 0.20f;
                    metal = 0.25f;
                    break;
                case "room.c14_dockmaster":
                case "room.c15_maintenance":
                    floorTint = new Color(0.34f, 0.32f, 0.29f);
                    smooth = 0.22f;
                    metal = 0.40f;
                    break;
                default:
                    floorTint = new Color(0.30f, 0.35f, 0.41f);
                    break;
            }

            Vector2 tiling = new Vector2(Mathf.Max(1f, Mathf.Round(widthX / 4.0f)), Mathf.Max(1f, Mathf.Round(depthZ / 4.0f)));
            return UrpMaterialFactory.GetTiledPbrSurface($"Floor_{entryId}", null, null, null, floorTint, tiling, isFloor: true, smoothness: smooth, metallic: metal);
        }

        private static Color GetRoomLightColor(string entryId, bool isRockCutPort = false)
        {
            if (entryId == "room.sim_red") return new Color(1.0f, 0.55f, 0.52f);
            if (entryId == "room.sim_blue") return new Color(0.52f, 0.78f, 1.0f);
            if (entryId == "room.simulator_lobby") return new Color(0.92f, 0.94f, 1.0f);
            if (isRockCutPort) return new Color(1.0f, 0.80f, 0.55f);

            switch (entryId)
            {
                case "room.c01_berth_gallery": return new Color(0.75f, 0.88f, 1.0f);
                case "room.c03_arrivals":      return new Color(1.0f, 0.92f, 0.76f);
                case "room.c10_slack_line":    return new Color(1.0f, 0.72f, 0.48f);
                case "room.c14_dockmaster":    return new Color(1.0f, 0.68f, 0.32f);
                default:                       return new Color(0.86f, 0.93f, 1.0f);
            }
        }

        private bool HasUpperFloorSlabCovering(AtlasRoomDef room)
        {
            if (Pack == null) return false;
            Vector2 c = room.Center2D;
            foreach (var other in Pack.Rooms.Values)
            {
                if (other.FloorIndex == room.FloorIndex + 1 &&
                    c.x >= other.MinX && c.x <= other.MaxX &&
                    c.y >= other.MinY && c.y <= other.MaxY)
                {
                    return true;
                }
            }
            return false;
        }

        private static bool TrySelectNonOverlappingHorizontalWallX(
            AtlasRoomDef room,
            float yWall,
            List<(int floor, float yWall, float centerX, AtlasExitDef exit)> hDoors,
            List<float> existingXs,
            float desiredX,
            float halfClearanceM,
            float minPlaqueSpacingM,
            out float chosenX)
        {
            chosenX = desiredX;
            float minSafe = room.MinX + halfClearanceM;
            float maxSafe = room.MaxX - halfClearanceM;
            if (minSafe >= maxSafe)
                return false;

            float clamped = Mathf.Clamp(desiredX, minSafe, maxSafe);
            if (IsHorizontalSlotClear(room.FloorIndex, yWall, clamped, halfClearanceM + 2.2f, hDoors, existingXs, minPlaqueSpacingM))
            {
                chosenX = clamped;
                return true;
            }

            float span = maxSafe - minSafe;
            if (float.IsNaN(span) || float.IsInfinity(span) || span <= 0f)
                return false;
            int maxSteps = Math.Min(MaxWallSlotScanSteps, (int)Math.Ceiling(Math.Min(span, MaxWallSlotScanSteps)));
            for (int step = 1; step <= maxSteps; step++)
            {
                float offset = step * 1.0f;
                float right = clamped + offset;
                if (right <= maxSafe && IsHorizontalSlotClear(room.FloorIndex, yWall, right, halfClearanceM + 2.2f, hDoors, existingXs, minPlaqueSpacingM))
                {
                    chosenX = right;
                    return true;
                }
                float left = clamped - offset;
                if (left >= minSafe && IsHorizontalSlotClear(room.FloorIndex, yWall, left, halfClearanceM + 2.2f, hDoors, existingXs, minPlaqueSpacingM))
                {
                    chosenX = left;
                    return true;
                }
            }
            return false;
        }

        private static bool IsHorizontalSlotClear(
            int floor,
            float yWall,
            float candidateX,
            float doorDistM,
            List<(int floor, float yWall, float centerX, AtlasExitDef exit)> hDoors,
            List<float> existingXs,
            float minPlaqueSpacingM)
        {
            if (ConflictsWithHorizontalDoor(floor, yWall, candidateX, doorDistM, hDoors))
                return false;
            if (existingXs != null)
            {
                for (int i = 0; i < existingXs.Count; i++)
                {
                    if (Mathf.Abs(existingXs[i] - candidateX) < minPlaqueSpacingM)
                        return false;
                }
            }
            return true;
        }

        private static bool TrySelectNonOverlappingVerticalWallY(
            AtlasRoomDef room,
            float xWall,
            List<(int floor, float xWall, float centerY, AtlasExitDef exit)> vDoors,
            List<float> existingYs,
            float desiredY,
            float halfClearanceM,
            float minPlaqueSpacingM,
            out float chosenY)
        {
            chosenY = desiredY;
            float minSafe = room.MinY + halfClearanceM;
            float maxSafe = room.MaxY - halfClearanceM;
            if (minSafe >= maxSafe)
                return false;

            float clamped = Mathf.Clamp(desiredY, minSafe, maxSafe);
            if (IsVerticalSlotClear(room.FloorIndex, xWall, clamped, halfClearanceM + 2.2f, vDoors, existingYs, minPlaqueSpacingM))
            {
                chosenY = clamped;
                return true;
            }

            float span = maxSafe - minSafe;
            if (float.IsNaN(span) || float.IsInfinity(span) || span <= 0f)
                return false;
            int maxSteps = Math.Min(MaxWallSlotScanSteps, (int)Math.Ceiling(Math.Min(span, MaxWallSlotScanSteps)));
            for (int step = 1; step <= maxSteps; step++)
            {
                float offset = step * 1.0f;
                float north = clamped + offset;
                if (north <= maxSafe && IsVerticalSlotClear(room.FloorIndex, xWall, north, halfClearanceM + 2.2f, vDoors, existingYs, minPlaqueSpacingM))
                {
                    chosenY = north;
                    return true;
                }
                float south = clamped - offset;
                if (south >= minSafe && IsVerticalSlotClear(room.FloorIndex, xWall, south, halfClearanceM + 2.2f, vDoors, existingYs, minPlaqueSpacingM))
                {
                    chosenY = south;
                    return true;
                }
            }
            return false;
        }

        private static bool IsVerticalSlotClear(
            int floor,
            float xWall,
            float candidateY,
            float doorDistM,
            List<(int floor, float xWall, float centerY, AtlasExitDef exit)> vDoors,
            List<float> existingYs,
            float minPlaqueSpacingM)
        {
            if (vDoors != null)
            {
                for (int i = 0; i < vDoors.Count; i++)
                {
                    var d = vDoors[i];
                    if (d.floor == floor && Mathf.Abs(d.xWall - xWall) < 0.35f && Mathf.Abs(d.centerY - candidateY) < doorDistM)
                        return false;
                }
            }
            if (existingYs != null)
            {
                for (int i = 0; i < existingYs.Count; i++)
                {
                    if (Mathf.Abs(existingYs[i] - candidateY) < minPlaqueSpacingM)
                        return false;
                }
            }
            return true;
        }

        private static float SelectClearHorizontalWallX(
            AtlasRoomDef room,
            float yWall,
            List<(int floor, float yWall, float centerX, AtlasExitDef exit)> hDoors,
            float desiredX,
            float halfClearanceM)
        {
            float minSafe = room.MinX + halfClearanceM;
            float maxSafe = room.MaxX - halfClearanceM;
            if (minSafe >= maxSafe)
                return (room.MinX + room.MaxX) * 0.5f;

            float clamped = Mathf.Clamp(desiredX, minSafe, maxSafe);
            if (!ConflictsWithHorizontalDoor(room.FloorIndex, yWall, clamped, halfClearanceM + 2.3f, hDoors))
                return clamped;

            // Scan left and right along the wall for the nearest non-doorway span (bounded integer steps to avoid float infinite loops).
            float span = maxSafe - minSafe;
            if (float.IsNaN(span) || float.IsInfinity(span) || span <= 0f)
                return clamped;
            int maxSteps = Math.Min(MaxWallSlotScanSteps, (int)Math.Ceiling(Math.Min(span, MaxWallSlotScanSteps * 1.5f) / 1.5f));
            for (int step = 1; step <= maxSteps; step++)
            {
                float offset = step * 1.5f;
                float right = clamped + offset;
                if (right <= maxSafe && !ConflictsWithHorizontalDoor(room.FloorIndex, yWall, right, halfClearanceM + 2.3f, hDoors))
                    return right;
                float left = clamped - offset;
                if (left >= minSafe && !ConflictsWithHorizontalDoor(room.FloorIndex, yWall, left, halfClearanceM + 2.3f, hDoors))
                    return left;
            }
            return clamped;
        }

        private static bool ConflictsWithHorizontalDoor(
            int floor,
            float yWall,
            float candidateX,
            float minDistM,
            List<(int floor, float yWall, float centerX, AtlasExitDef exit)> hDoors)
        {
            if (hDoors == null) return false;
            for (int i = 0; i < hDoors.Count; i++)
            {
                var d = hDoors[i];
                if (d.floor == floor && Mathf.Abs(d.yWall - yWall) < 0.35f && Mathf.Abs(d.centerX - candidateX) < minDistM)
                    return true;
            }
            return false;
        }

        private static GameObject SpawnWallAlignedConsoleProp(
            GameObject prefab,
            Transform parent,
            string instanceName,
            Vector3 floorPos,
            float inwardYawDeg,
            float targetHeightM,
            float maxDimensionM)
        {
            // Spawn grounded at identity first to inspect the vendor mesh's unrotated wide desk axis, then rotate by `inwardYawDeg`
            // (plus -90 deg if the mesh's wide desk axis is Z rather than X) so the console screen always faces into the room!
            var inst = PrefabFittingHelper.SpawnGrounded(prefab, parent, instanceName, floorPos, Quaternion.identity, targetHeightM, maxDimensionM, keepColliders: false);
            if (inst == null) return null;

            if (PrefabFittingHelper.TryComputeRendererBounds(inst, out Bounds b))
            {
                bool wideAlongZ = b.size.z > b.size.x * 1.05f;
                float baseOffsetYaw = wideAlongZ ? -90f : 0f;
                inst.transform.rotation = Quaternion.Euler(0f, inwardYawDeg + baseOffsetYaw, 0f);
                if (PrefabFittingHelper.TryComputeRendererBounds(inst, out Bounds bAligned))
                {
                    inst.transform.position += new Vector3(
                        floorPos.x - bAligned.center.x,
                        floorPos.y - bAligned.min.y,
                        floorPos.z - bAligned.center.z);
                }
            }
            return inst;
        }

        private TextMeshPro CreateDiegeticBoard(Transform parent, AtlasRoomDef room, Vector3 pos, Quaternion rot)
        {
            // Keep StatusBoardRoot at uniform scale (1, 1, 1) so BoardText NEVER inherits non-uniform cube scaling across doorways!
            var boardRoot = new GameObject("StatusBoardRoot");
            boardRoot.transform.SetParent(parent, false);
            boardRoot.transform.SetPositionAndRotation(pos, rot);

            var backing = CreateBox(boardRoot.transform, "StatusBoardBacking", pos, new Vector3(4.6f, 1.85f, 0.10f),
                UrpMaterialFactory.GetOpaque("BoardFrame", new Color(0.06f, 0.09f, 0.13f), 0.6f, 0.4f), true);
            backing.transform.localRotation = Quaternion.identity;

            string safeTitle = MushProtocol.EscapeUnityRichText(room.Name.ToUpperInvariant());
            string safeShort = MushProtocol.EscapeUnityRichText(room.ShortDesc);
            string summary = $"{safeTitle}\n<color=#88CCFF>{safeShort}</color>";
            var tmp = CreateTmp3D(boardRoot.transform, "BoardText", new Vector3(0f, 0f, -0.07f), summary, 1.45f, new Color(0.82f, 0.95f, 1f), allowRichText: true);
            tmp.rectTransform.sizeDelta = new Vector2(4.30f, 1.65f);
            tmp.alignment = TextAlignmentOptions.TopLeft;
            tmp.overflowMode = TextOverflowModes.Ellipsis;
            _roomBoardLines[room.EntryId] = new List<string> { safeTitle, $"<color=#88CCFF>{safeShort}</color>" };

            var wi = backing.AddComponent<WorldInteractable>();
            wi.kind = InteractableKind.RoomDetailPlaque;
            wi.promptTitle = $"Read {room.Name} Board";
            wi.promptSubtitle = "Inspect live port postings";
            wi.detailBody = room.Desc;
            wi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
            return tmp;
        }

        private void CreateDetailPlaque(Transform parent, string keyword, string textBody, Vector3 floorPos, float inwardYawDeg = 0f)
        {
            var plaqueRoot = new GameObject($"Plaque_{keyword}");
            plaqueRoot.transform.SetParent(parent, false);
            plaqueRoot.transform.SetPositionAndRotation(floorPos, Quaternion.Euler(0f, inwardYawDeg, 0f));

            var bc = plaqueRoot.AddComponent<BoxCollider>();
            bc.size = new Vector3(0.95f, 1.35f, 0.75f);
            bc.center = new Vector3(0f, 0.68f, 0f);

            if (Catalog != null && Catalog.stationConsoleProp != null)
            {
                SpawnWallAlignedConsoleProp(Catalog.stationConsoleProp, plaqueRoot.transform, "ObservationConsole", floorPos, inwardYawDeg, targetHeightM: 1.12f, maxDimensionM: 1.25f);
            }
            else
            {
                Material postMat = UrpMaterialFactory.GetOpaque("LecternPost", new Color(0.16f, 0.20f, 0.25f), 0.45f, 0.65f);
                Material screenMat = UrpMaterialFactory.GetEmissive("LecternScreen", new Color(0.08f, 0.22f, 0.30f), new Color(0.15f, 0.65f, 0.85f) * 0.8f);
                var ped = CreateBox(plaqueRoot.transform, "LecternPedestal", floorPos + new Vector3(0f, 0.48f, 0f), new Vector3(0.22f, 0.96f, 0.22f), postMat, false);
                ped.transform.localRotation = Quaternion.identity;
                var face = CreateBox(plaqueRoot.transform, "LecternPlate", floorPos + plaqueRoot.transform.forward * 0.04f + new Vector3(0f, 0.98f, 0f), new Vector3(0.68f, 0.08f, 0.42f), screenMat, false);
                face.transform.localRotation = Quaternion.Euler(28f, 0f, 0f);
            }

            // Orient toward a viewer inside the room AND attach WorldBillboard so the label is never mirrored!
            var labelGo = new GameObject("PlaqueLabelBillboard");
            labelGo.transform.SetParent(plaqueRoot.transform, false);
            labelGo.transform.localPosition = new Vector3(0f, 1.44f, 0f);
            labelGo.transform.localRotation = Quaternion.Euler(0f, 180f, 0f);
            var bb = labelGo.AddComponent<WorldBillboard>();
            string safeDisplayKeyword = MushProtocol.EscapeUnityRichText((keyword ?? string.Empty).ToUpperInvariant());
            var labelTmp = CreateTmp3D(labelGo.transform, "PlaqueTitle", Vector3.zero, $"[{safeDisplayKeyword}]", 1.08f, new Color(0.55f, 0.90f, 1f), allowRichText: false);
            labelTmp.rectTransform.sizeDelta = new Vector2(2.40f, 0.48f);
            bb.NameplateText = labelTmp;

            bool validKw = MushProtocol.TryValidateSafeKeywordOrName(keyword, out string safeCmdKeyword);
            var wi = plaqueRoot.AddComponent<WorldInteractable>();
            wi.kind = InteractableKind.RoomDetailPlaque;
            wi.promptTitle = $"Inspect {MushProtocol.EscapeUnityRichText(keyword)}";
            wi.promptSubtitle = validKw ? $"look {safeCmdKeyword}" : "Inspect plaque";
            wi.commandOrKey = validKw ? $"look {safeCmdKeyword}" : string.Empty;
            wi.detailBody = textBody;
            wi.OnInteracted = item => OnInteractableTriggered?.Invoke(item);
        }

        private void CreateTexturedFloorSlab(Transform parent, string name, string entryId, float minX, float minY, float maxX, float maxY, float elev)
        {
            float w = Mathf.Max(0.2f, maxX - minX);
            float d = Mathf.Max(0.2f, maxY - minY);
            float cx = (minX + maxX) * 0.5f + _originOffsetX;
            float cz = (minY + maxY) * 0.5f + _originOffsetZ;
            Material mat = GetFloorMaterialForSlab(entryId, w, d);
            CreateBox(parent, name, new Vector3(cx, elev - 0.14f, cz), new Vector3(w, 0.28f, d), mat, true);

            // Attach a matte coffered ceiling soffit underneath any upper-deck floor slab so rooms below see a true
            // architectural ceiling (preserving all staircase cutouts!) rather than upside-down floor tiles!
            if (elev > 0.5f)
            {
                Vector2 ceilTiling = new Vector2(Mathf.Max(1f, Mathf.Round(w / 4.0f)), Mathf.Max(1f, Mathf.Round(d / 4.0f)));
                Material ceilMat = UrpMaterialFactory.GetStationCeilingMaterial($"Soffit_{entryId}", new Color(0.32f, 0.36f, 0.42f), ceilTiling);
                CreateBox(parent, name + "_CeilingSoffit", new Vector3(cx, elev - 0.30f, cz), new Vector3(w, 0.04f, d), ceilMat, withCollider: false);
            }
        }

        private static GameObject CreateLocalBox(Transform parent, string name, Vector3 localPos, Vector3 scale, Material mat, bool withCollider)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Cube);
            go.name = name;
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localRotation = Quaternion.identity;
            go.transform.localScale = scale;
            if (!withCollider)
            {
                var col = go.GetComponent<Collider>();
                if (col != null)
                {
                    if (Application.isPlaying) Destroy(col);
                    else DestroyImmediate(col);
                }
            }
            var mr = go.GetComponent<MeshRenderer>();
            if (mr != null) mr.sharedMaterial = mat;
            return go;
        }

        private static GameObject CreateBox(Transform parent, string name, Vector3 worldPos, Vector3 scale, Material mat, bool withCollider)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Cube);
            go.name = name;
            go.transform.SetParent(parent, true);
            go.transform.position = worldPos;
            go.transform.localScale = scale;
            if (!withCollider)
            {
                var col = go.GetComponent<Collider>();
                if (col != null)
                {
                    if (Application.isPlaying) Destroy(col);
                    else DestroyImmediate(col);
                }
            }
            var mr = go.GetComponent<MeshRenderer>();
            if (mr != null) mr.sharedMaterial = mat;
            return go;
        }

        private static void CreateWorldSign(Transform parent, string label, Vector3 worldPos, Quaternion rot, float fontSize, Color color)
        {
            string safeLabel = MushProtocol.EscapeUnityRichText(label);
            var tmp = CreateTmp3D(parent, "Sign_" + safeLabel, Vector3.zero, safeLabel, fontSize, color, allowRichText: false);
            tmp.transform.position = worldPos;
            tmp.transform.rotation = rot;
        }

        private static TextMeshPro CreateTmp3D(Transform parent, string name, Vector3 localPos, string text, float fontSize, Color color, bool allowRichText = false)
        {
            var go = new GameObject(name);
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            var tmp = go.AddComponent<TextMeshPro>();
            tmp.richText = allowRichText;
            tmp.text = text;
            tmp.fontSize = fontSize;
            tmp.color = color;
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.Normal;
            tmp.rectTransform.sizeDelta = new Vector2(6f, 1.6f);
            return tmp;
        }
    }
}
