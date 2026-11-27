using System;
using System.Collections.Generic;
using TacSim.Core;
using UnityEngine;

namespace TacSim.World
{
    public enum WayfinderScope
    {
        None = 0,
        Station = 1,
        Ship = 2
    }

    /// <summary>
    /// 3D Holographic Floor Wayfinder Ribbon &amp; Destination Beacon for multi-deck 3D Stations
    /// and multi-deck 3D Ship Interiors. Plots authoritative BFS exit routes from `AtlasNeighborhoodPack`
    /// or `ShipUnitTemplate`, renders an animated cyan deck-level `LineRenderer` ribbon plus pulsing
    /// destination pillar, tracks hop/distance progress, and clears automatically on arrival.
    /// </summary>
    public sealed class TacSimWayfinder3D : MonoBehaviour
    {
        public event Action<string> OnArrivedAtDestination;
        public event Action OnRouteChanged;

        public WayfinderScope Scope { get; private set; } = WayfinderScope.None;
        public bool HasActiveRoute => Scope != WayfinderScope.None && !string.IsNullOrEmpty(TargetId);
        public string TargetId { get; private set; } = string.Empty;
        public string TargetDisplayName { get; private set; } = string.Empty;
        public string TargetDeckLabel { get; private set; } = string.Empty;
        public int RemainingHops => _remainingPath.Count;
        public float DistanceMeters { get; private set; }
        public Vector3 DestinationWorldPos { get; private set; }
        public IReadOnlyList<(string command, string targetId)> RemainingPath => _remainingPath;
        public int RenderedPointCount => _ribbonPoints.Count;
        public bool IsQuestTrail { get; private set; }
        public Color ActiveTrailColor => _ribbon != null ? _ribbon.startColor : new Color(0.22f, 0.95f, 1.0f, 0.92f);

        private LineRenderer _ribbon;
        private Transform _beaconRoot;
        private Transform _beaconRing;
        private MeshRenderer _pillarRenderer;
        private MeshRenderer _ringRenderer;
        private readonly List<(string command, string targetId)> _remainingPath = new List<(string, string)>();
        private readonly List<Vector3> _waypointNodes = new List<Vector3>();
        private readonly List<Vector3> _ribbonPoints = new List<Vector3>();
        private float _pulsePhase;

        public void EnsureVisuals()
        {
            if (_ribbon == null)
            {
                var lineGo = new GameObject("WayfinderHoloRibbon");
                lineGo.transform.SetParent(transform, false);
                _ribbon = lineGo.AddComponent<LineRenderer>();
                _ribbon.useWorldSpace = true;
                _ribbon.startWidth = 0.28f;
                _ribbon.endWidth = 0.22f;
                _ribbon.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("WayfinderRibbonMat", new Color(0.22f, 0.92f, 1.0f, 0.88f));
                _ribbon.startColor = new Color(0.22f, 0.95f, 1.0f, 0.92f);
                _ribbon.endColor = new Color(0.32f, 1.0f, 0.72f, 0.95f);
                _ribbon.positionCount = 0;
                _ribbon.enabled = false;
            }

            if (_beaconRoot == null)
            {
                var bGo = new GameObject("WayfinderDestBeacon");
                bGo.transform.SetParent(transform, false);
                _beaconRoot = bGo.transform;

                var pillar = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
                pillar.name = "BeaconPillar";
                pillar.transform.SetParent(_beaconRoot, false);
                pillar.transform.localPosition = new Vector3(0f, 1.85f, 0f);
                pillar.transform.localScale = new Vector3(0.32f, 1.85f, 0.32f);
                var pc = pillar.GetComponent<Collider>();
                if (pc != null) { if (Application.isPlaying) Destroy(pc); else DestroyImmediate(pc); }
                _pillarRenderer = pillar.GetComponent<MeshRenderer>();
                _pillarRenderer.sharedMaterial =
                    UrpMaterialFactory.GetHologramUnlit("WayfinderPillarMat", new Color(0.24f, 0.96f, 0.82f, 0.45f));

                var ring = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
                ring.name = "BeaconFloorRing";
                ring.transform.SetParent(_beaconRoot, false);
                ring.transform.localPosition = new Vector3(0f, 0.08f, 0f);
                ring.transform.localScale = new Vector3(2.2f, 0.06f, 2.2f);
                var rc = ring.GetComponent<Collider>();
                if (rc != null) { if (Application.isPlaying) Destroy(rc); else DestroyImmediate(rc); }
                _ringRenderer = ring.GetComponent<MeshRenderer>();
                _ringRenderer.sharedMaterial =
                    UrpMaterialFactory.GetHologramUnlit("WayfinderRingMat", new Color(0.28f, 0.95f, 1.0f, 0.65f));
                _beaconRing = ring.transform;

                _beaconRoot.gameObject.SetActive(false);
            }
        }

        private void ApplyTrailPalette(bool isQuestObjective)
        {
            if (_ribbon != null)
            {
                if (isQuestObjective)
                {
                    _ribbon.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("WayfinderQuestRibbonMat", new Color(1.0f, 0.84f, 0.24f, 0.92f));
                    _ribbon.startColor = new Color(1.0f, 0.84f, 0.24f, 0.96f);
                    _ribbon.endColor = new Color(1.0f, 0.94f, 0.56f, 0.98f);
                }
                else
                {
                    _ribbon.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("WayfinderRibbonMat", new Color(0.22f, 0.92f, 1.0f, 0.88f));
                    _ribbon.startColor = new Color(0.22f, 0.95f, 1.0f, 0.92f);
                    _ribbon.endColor = new Color(0.32f, 1.0f, 0.72f, 0.95f);
                }
            }

            if (_pillarRenderer != null)
            {
                _pillarRenderer.sharedMaterial = isQuestObjective
                    ? UrpMaterialFactory.GetHologramUnlit("WayfinderQuestPillarMat", new Color(1.0f, 0.84f, 0.24f, 0.56f))
                    : UrpMaterialFactory.GetHologramUnlit("WayfinderPillarMat", new Color(0.24f, 0.96f, 0.82f, 0.45f));
            }

            if (_ringRenderer != null)
            {
                _ringRenderer.sharedMaterial = isQuestObjective
                    ? UrpMaterialFactory.GetHologramUnlit("WayfinderQuestRingMat", new Color(1.0f, 0.88f, 0.34f, 0.78f))
                    : UrpMaterialFactory.GetHologramUnlit("WayfinderRingMat", new Color(0.28f, 0.95f, 1.0f, 0.65f));
            }
        }

        /// <summary>
        /// Builds the ordered 3D ribbon vertex chain from the player's current world position
        /// through each remaining waypoint node with a gentle vertical floor lift.
        /// </summary>
        public static List<Vector3> BuildRouteWorldPoints(Vector3 playerWorldPos, IReadOnlyList<Vector3> roomNodes, float floorLiftM = 0.28f)
        {
            var pts = new List<Vector3>(roomNodes != null ? roomNodes.Count + 1 : 1);
            pts.Add(playerWorldPos + Vector3.up * floorLiftM);
            if (roomNodes != null)
            {
                for (int i = 0; i < roomNodes.Count; i++)
                {
                    Vector3 p = roomNodes[i] + Vector3.up * floorLiftM;
                    if (Vector3.Distance(pts[pts.Count - 1], p) > 0.25f)
                        pts.Add(p);
                }
            }
            return pts;
        }

        public bool PlotStationRoute(StationWorldController station, string fromRoomEntry, string toRoomEntry, Vector3 playerWorldPos, bool isQuestObjective = false)
        {
            EnsureVisuals();
            if (station == null || station.Pack == null || string.IsNullOrWhiteSpace(toRoomEntry))
                return false;
            if (!station.Pack.Rooms.TryGetValue(toRoomEntry, out var destRoom))
                return false;

            string startRoom = !string.IsNullOrEmpty(fromRoomEntry) ? fromRoomEntry : station.ActiveRoomEntryId;
            if (string.Equals(startRoom, toRoomEntry, StringComparison.OrdinalIgnoreCase))
            {
                ClearRoute();
                return false;
            }

            var path = station.FindExitPath(startRoom, toRoomEntry);
            if (path == null || path.Count == 0)
                return false;

            Scope = WayfinderScope.Station;
            TargetId = toRoomEntry;
            TargetDisplayName = string.IsNullOrWhiteSpace(destRoom.Name) ? toRoomEntry : destRoom.Name;
            TargetDeckLabel = $"FL {destRoom.FloorIndex + 1}";
            IsQuestTrail = isQuestObjective;
            ApplyTrailPalette(isQuestObjective);

            _remainingPath.Clear();
            _waypointNodes.Clear();
            for (int i = 0; i < path.Count; i++)
            {
                _remainingPath.Add(path[i]);
                _waypointNodes.Add(station.GetRoomSpawnPosition(path[i].targetRoom));
            }
            DestinationWorldPos = _waypointNodes[_waypointNodes.Count - 1];
            RebuildRibbonGeometry(playerWorldPos);
            OnRouteChanged?.Invoke();
            return true;
        }

        public bool PlotShipRoute(ShipInteriorController ship, string fromCompId, string toCompId, Vector3 playerWorldPos, bool isQuestObjective = false)
        {
            EnsureVisuals();
            if (ship == null || ship.Template == null || string.IsNullOrWhiteSpace(toCompId))
                return false;

            string startComp = !string.IsNullOrEmpty(fromCompId) ? fromCompId : ship.ActiveCompartmentId;
            if (string.Equals(startComp, toCompId, StringComparison.OrdinalIgnoreCase))
            {
                ClearRoute();
                return false;
            }

            var path = ship.FindExitPath(startComp, toCompId);
            if (path == null || path.Count == 0)
                return false;

            Scope = WayfinderScope.Ship;
            TargetId = toCompId;
            TargetDisplayName = ship.Template.Compartments.TryGetValue(toCompId, out var cDef) && !string.IsNullOrWhiteSpace(cDef.Name)
                ? cDef.Name
                : toCompId.ToUpperInvariant();
            TargetDeckLabel = "SHIP DECK";
            IsQuestTrail = isQuestObjective;
            ApplyTrailPalette(isQuestObjective);

            _remainingPath.Clear();
            _waypointNodes.Clear();
            for (int i = 0; i < path.Count; i++)
            {
                _remainingPath.Add(path[i]);
                _waypointNodes.Add(ship.GetCompartmentSpawnWorldPos(path[i].targetCompartment));
            }
            DestinationWorldPos = _waypointNodes[_waypointNodes.Count - 1];
            RebuildRibbonGeometry(playerWorldPos);
            OnRouteChanged?.Invoke();
            return true;
        }

        public void SyncProgress(
            TacSimActiveStage stage,
            StationWorldController station,
            ShipInteriorController ship,
            Vector3 playerWorldPos,
            float dt)
        {
            if (!HasActiveRoute) return;

            bool scopeMatches = (Scope == WayfinderScope.Station && stage == TacSimActiveStage.StationWalk) ||
                                (Scope == WayfinderScope.Ship && stage == TacSimActiveStage.ShipInteriorWalk);
            if (!scopeMatches)
            {
                SetVisualsActive(false);
                return;
            }

            string currentId = Scope == WayfinderScope.Station
                ? (station != null ? station.ActiveRoomEntryId : string.Empty)
                : (ship != null ? ship.ActiveCompartmentId : string.Empty);

            if (string.Equals(currentId, TargetId, StringComparison.OrdinalIgnoreCase))
            {
                string arrivedName = TargetDisplayName;
                ClearRoute();
                OnArrivedAtDestination?.Invoke(arrivedName);
                return;
            }

            // Re-slice remaining hops cleanly whenever the player enters a different room along the path
            if (!string.IsNullOrEmpty(currentId))
            {
                bool shouldRecompute = _remainingPath.Count == 0 ||
                                       string.Equals(_remainingPath[0].targetId, currentId, StringComparison.OrdinalIgnoreCase);
                if (shouldRecompute)
                {
                    if (Scope == WayfinderScope.Station && station != null)
                    {
                        var nextPath = station.FindExitPath(currentId, TargetId);
                        if (nextPath != null && nextPath.Count > 0)
                        {
                            _remainingPath.Clear();
                            _waypointNodes.Clear();
                            for (int i = 0; i < nextPath.Count; i++)
                            {
                                _remainingPath.Add(nextPath[i]);
                                _waypointNodes.Add(station.GetRoomSpawnPosition(nextPath[i].targetRoom));
                            }
                            OnRouteChanged?.Invoke();
                        }
                    }
                    else if (Scope == WayfinderScope.Ship && ship != null)
                    {
                        var nextPath = ship.FindExitPath(currentId, TargetId);
                        if (nextPath != null && nextPath.Count > 0)
                        {
                            _remainingPath.Clear();
                            _waypointNodes.Clear();
                            for (int i = 0; i < nextPath.Count; i++)
                            {
                                _remainingPath.Add(nextPath[i]);
                                _waypointNodes.Add(ship.GetCompartmentSpawnWorldPos(nextPath[i].targetCompartment));
                            }
                            OnRouteChanged?.Invoke();
                        }
                    }
                }
            }

            RebuildRibbonGeometry(playerWorldPos);

            _pulsePhase += Mathf.Max(0.001f, dt) * 3.4f;
            if (_beaconRing != null)
            {
                float s = 2.0f + 0.55f * Mathf.Sin(_pulsePhase);
                _beaconRing.localScale = new Vector3(s, 0.06f, s);
            }
        }

        public void ClearRoute()
        {
            bool had = HasActiveRoute;
            Scope = WayfinderScope.None;
            IsQuestTrail = false;
            TargetId = string.Empty;
            TargetDisplayName = string.Empty;
            TargetDeckLabel = string.Empty;
            DistanceMeters = 0f;
            _remainingPath.Clear();
            _waypointNodes.Clear();
            _ribbonPoints.Clear();
            SetVisualsActive(false);
            if (had)
                OnRouteChanged?.Invoke();
        }

        private void RebuildRibbonGeometry(Vector3 playerWorldPos)
        {
            _ribbonPoints.Clear();
            var pts = BuildRouteWorldPoints(playerWorldPos, _waypointNodes);
            _ribbonPoints.AddRange(pts);

            float totalDist = 0f;
            for (int i = 1; i < _ribbonPoints.Count; i++)
                totalDist += Vector3.Distance(_ribbonPoints[i - 1], _ribbonPoints[i]);
            DistanceMeters = totalDist;

            if (_ribbon != null)
            {
                _ribbon.positionCount = _ribbonPoints.Count;
                for (int i = 0; i < _ribbonPoints.Count; i++)
                    _ribbon.SetPosition(i, _ribbonPoints[i]);
                _ribbon.enabled = _ribbonPoints.Count >= 2;
            }

            if (_beaconRoot != null)
            {
                _beaconRoot.position = DestinationWorldPos;
                _beaconRoot.gameObject.SetActive(_ribbonPoints.Count >= 2);
            }
        }

        private void SetVisualsActive(bool active)
        {
            if (_ribbon != null) _ribbon.enabled = active;
            if (_beaconRoot != null) _beaconRoot.gameObject.SetActive(active);
        }
    }
}
