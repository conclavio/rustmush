using System;
using System.Collections.Generic;
using TacSim.Arena;
using TacSim.Core;
using TacSim.Space;
using TacSim.Surface;
using TacSim.World;
using UnityEngine;
using UnityEngine.EventSystems;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// Unified WoW / EverQuest-style Corner Tactical Minimap supporting all 5 TacSim gameplay stages:
    /// - Station Walk: live multi-deck room blueprint + player arrow + deck switcher + click room to walk
    /// - Ship Interior: 6-compartment hull deckplan + active compartment highlight + click to move
    /// - Orbital Space: live 360° holo-radar of ships, asteroids, and torpedoes + click blip to lock/approach
    /// - Planetary Surface: live `site.map` terrain &amp; ore wash radar + click cell to drive rover
    /// - OOC Arena: live `arena.map` tactical battlefield radar + team pips + key spots
    ///
    /// Automatically shifts cleanly to Mid-Left (below Player Unit Frame, above CommsDrawer) whenever a
    /// top-right tactical card (DockingHud, SpaceFireControlCard, QuarryCard, ArenaContactsPanel) is active.
    /// </summary>
    public sealed class TacSimTacticalMinimap : MonoBehaviour
    {
        public event Action<string> OnStationRoomClicked;
        public event Action<string> OnShipCompartmentClicked;
        public event Action<string, bool> OnOrbitalBlipClicked;
        public event Action<int, int> OnSurfaceCellClicked;
        public event Action<string> OnArenaMinimapContactClicked;
        public event Action<int, int> OnArenaMinimapCellClicked;
        public event Action<int> OnReplayMinimapUnitClicked;
        public event Action OnOpenSolarChartRequested;

        public bool IsCollapsed { get; private set; }
        public bool IsDockedMidLeft { get; private set; }
        public bool IsVisible => _root != null && _root.activeSelf;
        public TacSimActiveStage ActiveStage { get; private set; } = TacSimActiveStage.StationWalk;
        public int InspectedStationFloor { get; private set; }
        public string HeaderText => _headerText != null ? _headerText.text : string.Empty;
        public int LastArenaMinimapContactBlipCount => _arenaContactHits.Count;
        public int LastReplayMinimapUnitCount => _replayUnitHits.Count;
        public bool LastArenaMinimapHasSensorRing { get; private set; }
        public bool LastArenaMinimapHasWeaponRangeRing { get; private set; }
        public bool LastArenaMinimapHasEcmRing { get; private set; }
        public bool LastArenaMinimapHasObjective { get; private set; }
        public bool LastArenaMinimapHasWaypoint { get; private set; }
        public bool LastArenaMinimapHasLockLine { get; private set; }
        public string LastArenaMinimapHoldOwner { get; private set; } = "NEUTRAL";
        public int LastReplayMinimapShotLineCount { get; private set; }
        public bool LastStationMinimapHasQuestBeacon { get; private set; }
        public int LastStationMinimapQuestFloorDelta { get; private set; }
        public string QuestObjectiveRoomEntry { get; private set; } = string.Empty;
        public bool LastShipMinimapHasQuestBeacon { get; private set; }
        public string QuestShipCompartment { get; private set; } = string.Empty;
        public Texture2D MinimapTexture => _mapTex;
        public Color32 SampleMinimapPixelForTests(int x, int y)
        {
            if (_pixels == null || (uint)x >= MapTexSize || (uint)y >= MapTexSize)
                return default;
            return _pixels[y * MapTexSize + x];
        }

        public void SetHiddenByHudPreset(bool hidden)
        {
            if (_root != null && _root.activeSelf == hidden)
                _root.SetActive(!hidden);
        }

        private const int MapTexSize = 196;

        private Font _font;
        private GameObject _root;
        private RectTransform _rootRect;
        private GameObject _bodyContainer;
        private Text _headerText;
        private Text _collapseBtnLabel;
        private RawImage _mapImage;
        private RectTransform _mapRect;
        private Texture2D _mapTex;
        private Color32[] _pixels;

        private StationWorldController _station;
        private ShipInteriorController _ship;
        private SpaceFlightController _space;
        private SurfaceMiningController _surface;
        private OocArenaController _arena;

        private float _zoom = 1.0f;
        private int _lastStationFloor = int.MinValue;
        private float _nextRedrawAt;
        private bool _questObjectiveIsTurnIn;
        private bool _questCurrentRoomHasOffer;

        // Cached clickable bounds in UV space `[0,1]` for Station rooms / Ship compartments / Orbital blips
        private readonly List<(Rect uvRect, string id)> _stationRoomHitRects = new List<(Rect, string)>();
        private readonly List<(Rect uvRect, string id)> _shipCompHitRects = new List<(Rect, string)>();
        private readonly List<(Vector2 uv, string id, bool isRock)> _orbitalBlipHits = new List<(Vector2, string, bool)>();
        private readonly List<(Vector2 uv, string designator, bool canLock)> _arenaContactHits = new List<(Vector2, string, bool)>();
        private readonly List<(Vector2 uv, int rosterIndex)> _replayUnitHits = new List<(Vector2, int)>();
        private float _arenaMinCellX;
        private float _arenaMinCellY;
        private float _arenaSpanCellsX = 64f;
        private float _arenaSpanCellsY = 64f;
        private int _surfaceOriginX;
        private int _surfaceOriginY;
        private int _surfaceSpanCells = 32;

        private sealed class MinimapClickRelay : MonoBehaviour, IPointerClickHandler
        {
            public TacSimTacticalMinimap Owner;
            public void OnPointerClick(PointerEventData eventData) => Owner?.HandleMapClick(eventData);
        }

        public void BuildMinimap(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            if (_root != null)
            {
                if (Application.isPlaying) Destroy(_root);
                else DestroyImmediate(_root);
                _root = null;
            }

            _root = CreatePanel(hudRoot, "TacticalMinimap", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-232f, -320f), new Vector2(-16f, -72f), new Color(0.03f, 0.06f, 0.10f, 0.91f));
            _rootRect = _root.GetComponent<RectTransform>();

            // Header bar (24px)
            var headerBar = CreatePanel(_root.transform, "MinimapHeader", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 0f), new Color(0.06f, 0.12f, 0.20f, 0.95f));
            SetRect(headerBar.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -24f), Vector2.zero);

            _headerText = CreateText(headerBar.transform, "DECK 1 — STATION", 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.52f, 0.90f, 1f));
            SetRect(_headerText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(8f, 0f), new Vector2(-30f, 0f));

            var colBtn = CreateSmallButton(headerBar.transform, "—", new Color(0.16f, 0.26f, 0.38f), ToggleCollapsed, out _collapseBtnLabel);
            SetRect(colBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-26f, 2f), new Vector2(-2f, -2f));

            // Collapsible body (196x196 map + 24px micro footer)
            _bodyContainer = CreatePanel(_root.transform, "MinimapBody", Vector2.zero, Vector2.one, new Vector2(0f, 0f), new Vector2(0f, -24f), Color.clear);

            var mapGo = new GameObject("MinimapCanvas", typeof(RectTransform), typeof(RawImage));
            mapGo.transform.SetParent(_bodyContainer.transform, false);
            _mapRect = mapGo.GetComponent<RectTransform>();
            SetRect(_mapRect, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(8f, 28f), new Vector2(-8f, -4f));

            _mapTex = new Texture2D(MapTexSize, MapTexSize, TextureFormat.RGBA32, false)
            {
                name = "TacSimTacticalMinimapTex",
                wrapMode = TextureWrapMode.Clamp,
                filterMode = FilterMode.Point
            };
            _pixels = new Color32[MapTexSize * MapTexSize];
            _mapImage = mapGo.GetComponent<RawImage>();
            _mapImage.texture = _mapTex;
            _mapImage.color = Color.white;
            _mapImage.raycastTarget = true;

            var relay = mapGo.AddComponent<MinimapClickRelay>();
            relay.Owner = this;

            // North compass badge at top-center of map
            var northLbl = CreateText(mapGo.transform, "N", 10, FontStyle.Bold, TextAnchor.UpperCenter, new Color(0.95f, 0.85f, 0.45f));
            SetRect(northLbl.rectTransform, new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-10f, -15f), new Vector2(10f, -1f));

            // Micro control footer
            var footer = CreatePanel(_bodyContainer.transform, "MinimapFooter", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(6f, 3f), new Vector2(-6f, 25f), Color.clear);
            var hlg = footer.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 4f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            CreateSmallButton(footer.transform, "▲", new Color(0.13f, 0.24f, 0.34f), () => StepInspectedFloor(+1), out _);
            CreateSmallButton(footer.transform, "▼", new Color(0.13f, 0.24f, 0.34f), () => StepInspectedFloor(-1), out _);
            CreateSmallButton(footer.transform, "-", new Color(0.13f, 0.24f, 0.34f), () => AdjustZoom(-0.25f), out _);
            CreateSmallButton(footer.transform, "+", new Color(0.13f, 0.24f, 0.34f), () => AdjustZoom(+0.25f), out _);
            CreateSmallButton(footer.transform, "MAP", new Color(0.18f, 0.38f, 0.52f), () => OnOpenSolarChartRequested?.Invoke(), out _);

            ApplyDockPlacement();
        }

        public void BindWorldSources(
            StationWorldController station,
            ShipInteriorController ship,
            SpaceFlightController space,
            SurfaceMiningController surface,
            OocArenaController arena)
        {
            _station = station;
            _ship = ship;
            _space = space;
            _surface = surface;
            _arena = arena;
        }

        public void ResetSessionState()
        {
            _lastStationFloor = int.MinValue;
            InspectedStationFloor = 0;
            _zoom = 1.0f;
            IsCollapsed = false;
            QuestObjectiveRoomEntry = string.Empty;
            _questObjectiveIsTurnIn = false;
            _questCurrentRoomHasOffer = false;
            LastStationMinimapHasQuestBeacon = false;
            LastStationMinimapQuestFloorDelta = 0;
            QuestShipCompartment = string.Empty;
            LastShipMinimapHasQuestBeacon = false;
            if (_bodyContainer != null) _bodyContainer.SetActive(true);
            if (_collapseBtnLabel != null) _collapseBtnLabel.text = "—";
            ApplyDockPlacement();
        }

        /// <summary>
        /// Highlights the active quest target compartment (`bridge`, `bay`, `hold`, `airlock`)
        /// on the Ship Interior Deckplan Minimap with a golden outline, beacon, and header tag.
        /// </summary>
        public void SetQuestShipCompartment(string compartmentId)
        {
            string normalized = compartmentId ?? string.Empty;
            if (string.Equals(QuestShipCompartment, normalized, StringComparison.OrdinalIgnoreCase))
                return;
            QuestShipCompartment = normalized;
            _nextRedrawAt = 0f;
        }

        /// <summary>
        /// Highlights the active quest objective room (or current-room quest offer) on the Station Minimap
        /// with a golden beacon and cross-deck elevation tag (`▲ D2!` / `▼ D1?`).
        /// </summary>
        public void SetQuestObjectiveRoom(string roomEntry, bool isTurnIn = false, bool currentRoomHasOffer = false)
        {
            string normalized = roomEntry ?? string.Empty;
            if (string.Equals(QuestObjectiveRoomEntry, normalized, StringComparison.OrdinalIgnoreCase) &&
                _questObjectiveIsTurnIn == isTurnIn &&
                _questCurrentRoomHasOffer == currentRoomHasOffer)
            {
                return;
            }
            QuestObjectiveRoomEntry = normalized;
            _questObjectiveIsTurnIn = isTurnIn;
            _questCurrentRoomHasOffer = currentRoomHasOffer;
            _nextRedrawAt = 0f;
        }

        public void ToggleCollapsed()
        {
            IsCollapsed = !IsCollapsed;
            if (_bodyContainer != null) _bodyContainer.SetActive(!IsCollapsed);
            if (_collapseBtnLabel != null) _collapseBtnLabel.text = IsCollapsed ? "+" : "—";
            ApplyDockPlacement();
        }

        public void SetDockMidLeft(bool dockMidLeft)
        {
            if (IsDockedMidLeft == dockMidLeft) return;
            IsDockedMidLeft = dockMidLeft;
            ApplyDockPlacement();
        }

        public void StepInspectedFloor(int delta)
        {
            if (_station?.Pack == null || _station.Pack.Rooms.Count == 0) return;
            int minF = int.MaxValue, maxF = int.MinValue;
            foreach (var r in _station.Pack.Rooms.Values)
            {
                int f = r.FloorIndex;
                if (f < minF) minF = f;
                if (f > maxF) maxF = f;
            }
            InspectedStationFloor = Mathf.Clamp(InspectedStationFloor + delta, minF, maxF);
            _nextRedrawAt = 0f;
        }

        public void AdjustZoom(float delta)
        {
            _zoom = Mathf.Clamp(_zoom + delta, 0.65f, 2.5f);
            _nextRedrawAt = 0f;
        }

        /// <summary>
        /// Refreshes the radar/minimap for the active stage at a clean ~8 Hz cadence (or immediately when forced).
        /// </summary>
        public void SyncMinimap(TacSimActiveStage stage, Vector3 playerWorldPos, float playerYawDeg, bool rightCornerOccupied, bool forceRedraw = false)
        {
            ActiveStage = stage;
            SetDockMidLeft(rightCornerOccupied);

            if (IsCollapsed || _mapTex == null) return;
            if (!forceRedraw && Application.isPlaying && Time.unscaledTime < _nextRedrawAt)
                return;
            _nextRedrawAt = Time.unscaledTime + 0.12f;

            switch (stage)
            {
                case TacSimActiveStage.StationWalk:
                    DrawStationMinimap(playerWorldPos, playerYawDeg);
                    break;
                case TacSimActiveStage.ShipInteriorWalk:
                    DrawShipInteriorMinimap(playerWorldPos, playerYawDeg);
                    break;
                case TacSimActiveStage.ShipHelmSeated:
                    DrawOrbitalRadar(playerYawDeg);
                    break;
                case TacSimActiveStage.PlanetarySurface:
                    DrawSurfaceMinimap(playerYawDeg);
                    break;
                case TacSimActiveStage.SimulatorArena:
                    DrawArenaMinimap();
                    break;
            }

            _mapTex.SetPixels32(_pixels);
            _mapTex.Apply(false, false);
        }

        public bool HandleClickAtUv(Vector2 uv)
        {
            if (uv.x < 0f || uv.x > 1f || uv.y < 0f || uv.y > 1f) return false;

            if (ActiveStage == TacSimActiveStage.StationWalk)
            {
                for (int i = 0; i < _stationRoomHitRects.Count; i++)
                {
                    if (_stationRoomHitRects[i].uvRect.Contains(uv))
                    {
                        OnStationRoomClicked?.Invoke(_stationRoomHitRects[i].id);
                        return true;
                    }
                }
            }
            else if (ActiveStage == TacSimActiveStage.ShipInteriorWalk)
            {
                for (int i = 0; i < _shipCompHitRects.Count; i++)
                {
                    if (_shipCompHitRects[i].uvRect.Contains(uv))
                    {
                        OnShipCompartmentClicked?.Invoke(_shipCompHitRects[i].id);
                        return true;
                    }
                }
            }
            else if (ActiveStage == TacSimActiveStage.ShipHelmSeated)
            {
                string bestId = null;
                bool bestRock = false;
                float bestSq = 0.085f * 0.085f;
                for (int i = 0; i < _orbitalBlipHits.Count; i++)
                {
                    float dx = _orbitalBlipHits[i].uv.x - uv.x;
                    float dy = _orbitalBlipHits[i].uv.y - uv.y;
                    float sq = dx * dx + dy * dy;
                    if (sq < bestSq)
                    {
                        bestSq = sq;
                        bestId = _orbitalBlipHits[i].id;
                        bestRock = _orbitalBlipHits[i].isRock;
                    }
                }
                if (!string.IsNullOrEmpty(bestId))
                {
                    OnOrbitalBlipClicked?.Invoke(bestId, bestRock);
                    return true;
                }
            }
            else if (ActiveStage == TacSimActiveStage.PlanetarySurface && _surface?.TerrainGrid != null)
            {
                int cx = _surfaceOriginX + Mathf.RoundToInt(uv.x * _surfaceSpanCells);
                int cy = _surfaceOriginY + Mathf.RoundToInt(uv.y * _surfaceSpanCells);
                OnSurfaceCellClicked?.Invoke(cx, cy);
                return true;
            }
            else if (ActiveStage == TacSimActiveStage.SimulatorArena && _arena?.TerrainGrid != null)
            {
                if (_arena.IsSpectatorTheaterActive && _replayUnitHits.Count > 0)
                {
                    int bestIdx = -1;
                    float bestReplaySq = 0.085f * 0.085f;
                    for (int i = 0; i < _replayUnitHits.Count; i++)
                    {
                        float dx = _replayUnitHits[i].uv.x - uv.x;
                        float dy = _replayUnitHits[i].uv.y - uv.y;
                        float sq = dx * dx + dy * dy;
                        if (sq < bestReplaySq)
                        {
                            bestReplaySq = sq;
                            bestIdx = _replayUnitHits[i].rosterIndex;
                        }
                    }
                    if (bestIdx >= 0)
                    {
                        OnReplayMinimapUnitClicked?.Invoke(bestIdx);
                        return true;
                    }
                    return false;
                }

                string bestDesig = null;
                float bestSq = 0.075f * 0.075f;
                for (int i = 0; i < _arenaContactHits.Count; i++)
                {
                    float dx = _arenaContactHits[i].uv.x - uv.x;
                    float dy = _arenaContactHits[i].uv.y - uv.y;
                    float sq = dx * dx + dy * dy;
                    if (sq < bestSq && (_arenaContactHits[i].canLock || bestDesig == null))
                    {
                        bestSq = sq;
                        bestDesig = _arenaContactHits[i].designator;
                    }
                }
                if (!string.IsNullOrEmpty(bestDesig))
                {
                    OnArenaMinimapContactClicked?.Invoke(bestDesig);
                    return true;
                }

                var grid = _arena.TerrainGrid;
                int span = Math.Max(1, grid.SpanCells);
                int maxCellX = grid.OriginCellX + Math.Max(1, grid.Width * span) - 1;
                int maxCellY = grid.OriginCellY + Math.Max(1, grid.Height * span) - 1;
                int cx = Mathf.Clamp(Mathf.FloorToInt(_arenaMinCellX + uv.x * _arenaSpanCellsX), grid.OriginCellX, maxCellX);
                int cy = Mathf.Clamp(Mathf.FloorToInt(_arenaMinCellY + uv.y * _arenaSpanCellsY), grid.OriginCellY, maxCellY);
                OnArenaMinimapCellClicked?.Invoke(cx, cy);
                return true;
            }
            return false;
        }

        private void HandleMapClick(PointerEventData eventData)
        {
            if (_mapRect == null || eventData == null) return;
            if (!RectTransformUtility.ScreenPointToLocalPointInRectangle(_mapRect, eventData.position, eventData.pressEventCamera, out Vector2 local))
                return;
            Rect r = _mapRect.rect;
            if (r.width <= 1f || r.height <= 1f) return;
            Vector2 uv = new Vector2((local.x - r.x) / r.width, (local.y - r.y) / r.height);
            HandleClickAtUv(uv);
        }

        private void ApplyDockPlacement()
        {
            if (_rootRect == null) return;
            float h = IsCollapsed ? 24f : 248f;
            if (IsDockedMidLeft)
            {
                // Dock neatly on Mid-Left below Player Unit Frame (`y = -164`) and well above CommsDrawer
                SetRect(_rootRect, new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(16f, -164f - h), new Vector2(232f, -164f));
            }
            else
            {
                SetRect(_rootRect, new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-232f, -72f - h), new Vector2(-16f, -72f));
            }
        }

        private void DrawStationMinimap(Vector3 playerWorldPos, float playerYawDeg)
        {
            ClearPixels(new Color32(8, 13, 22, 255));
            _stationRoomHitRects.Clear();
            LastStationMinimapHasQuestBeacon = false;
            LastStationMinimapQuestFloorDelta = 0;

            var pack = _station?.Pack;
            if (pack == null || pack.Rooms.Count == 0)
            {
                if (_headerText != null) _headerText.text = "STATION RADAR";
                return;
            }

            // Follow the player's deck automatically when the player changes floors
            int activeFloor = 0;
            AtlasRoomDef activeRoom = null;
            if (!string.IsNullOrEmpty(_station.ActiveRoomEntryId) &&
                pack.Rooms.TryGetValue(_station.ActiveRoomEntryId, out var matchedRoom))
            {
                activeRoom = matchedRoom;
                activeFloor = activeRoom.FloorIndex;
            }
            if (activeFloor != _lastStationFloor)
            {
                _lastStationFloor = activeFloor;
                InspectedStationFloor = activeFloor;
            }

            AtlasRoomDef questRoom = null;
            if (!string.IsNullOrEmpty(QuestObjectiveRoomEntry) &&
                pack.Rooms.TryGetValue(QuestObjectiveRoomEntry, out var matchedQuestRoom))
            {
                questRoom = matchedQuestRoom;
            }
            if (questRoom != null)
                LastStationMinimapQuestFloorDelta = questRoom.FloorIndex - InspectedStationFloor;

            if (_headerText != null)
            {
                string deckTag = InspectedStationFloor == activeFloor ? $"DECK {InspectedStationFloor + 1}" : $"DECK {InspectedStationFloor + 1} (VIEW)";
                string portName = !string.IsNullOrEmpty(pack.Name) ? pack.Name.ToUpperInvariant() : "STATION";
                string questTag = string.Empty;
                if (questRoom != null && LastStationMinimapQuestFloorDelta != 0)
                {
                    string arrow = LastStationMinimapQuestFloorDelta > 0 ? "▲" : "▼";
                    string sym = _questObjectiveIsTurnIn ? "?" : "!";
                    questTag = $" [{arrow} D{questRoom.FloorIndex + 1}{sym}]";
                }
                _headerText.text = $"{deckTag} — {portName}{questTag}";
            }

            float minX = float.MaxValue, maxX = float.MinValue, minY = float.MaxValue, maxY = float.MinValue;
            foreach (var r in pack.Rooms.Values)
            {
                if (r.FloorIndex != InspectedStationFloor) continue;
                if (r.MinX < minX) minX = r.MinX;
                if (r.MaxX > maxX) maxX = r.MaxX;
                if (r.MinY < minY) minY = r.MinY;
                if (r.MaxY > maxY) maxY = r.MaxY;
            }
            if (minX >= maxX || minY >= maxY) return;

            float cx = (minX + maxX) * 0.5f;
            float cy = (minY + maxY) * 0.5f;
            float span = Mathf.Max(Mathf.Max(maxX - minX, maxY - minY), 20f) * 1.12f / _zoom;

            // If zoomed in on active deck, center near active room
            if (_zoom > 1.1f && activeRoom != null && InspectedStationFloor == activeFloor)
            {
                cx = activeRoom.Center2D.x;
                cy = activeRoom.Center2D.y;
            }

            Vector2 AtlasToUv(float ax, float ay) => new Vector2(0.5f + (ax - cx) / span, 0.5f + (ay - cy) / span);

            // Draw room boxes on inspected deck
            foreach (var r in pack.Rooms.Values)
            {
                if (r.FloorIndex != InspectedStationFloor) continue;
                Vector2 u0 = AtlasToUv(r.MinX, r.MinY);
                Vector2 u1 = AtlasToUv(r.MaxX, r.MaxY);
                _stationRoomHitRects.Add((new Rect(u0.x, u0.y, u1.x - u0.x, u1.y - u0.y), r.EntryId));

                bool isCur = string.Equals(r.EntryId, _station.ActiveRoomEntryId, StringComparison.OrdinalIgnoreCase);
                bool isQuestRoom = questRoom != null && string.Equals(r.EntryId, questRoom.EntryId, StringComparison.OrdinalIgnoreCase);
                Color32 fill = isCur
                    ? new Color32(28, 84, 112, 255)
                    : (isQuestRoom
                        ? new Color32(62, 52, 22, 255)
                        : (r.Restricted ? new Color32(48, 28, 34, 255) : new Color32(20, 34, 52, 255)));
                Color32 border = isQuestRoom
                    ? new Color32(255, 214, 48, 255)
                    : (isCur ? new Color32(92, 226, 255, 255) : new Color32(68, 102, 134, 255));
                FillRectUv(u0, u1, fill);
                DrawRectOutlineUv(u0, u1, border);
            }

            // Draw glowing gold quest objective beacon on inspected deck
            AtlasRoomDef beaconRoom = null;
            if (questRoom != null && questRoom.FloorIndex == InspectedStationFloor)
                beaconRoom = questRoom;
            else if (questRoom == null && _questCurrentRoomHasOffer && activeRoom != null && activeRoom.FloorIndex == InspectedStationFloor)
                beaconRoom = activeRoom;

            if (beaconRoom != null)
            {
                Vector2 qUv = AtlasToUv(beaconRoom.Center2D.x, beaconRoom.Center2D.y);
                if (qUv.x >= 0f && qUv.x <= 1f && qUv.y >= 0f && qUv.y <= 1f)
                {
                    DrawQuestBeaconPx(UvToPx(qUv), _questObjectiveIsTurnIn);
                    LastStationMinimapHasQuestBeacon = true;
                }
            }

            // Draw player arrow on active deck
            if (InspectedStationFloor == activeFloor && activeRoom != null)
            {
                // Approximate player atlas position from activeRoom center + world delta
                Vector3 roomSpawn = _station.GetRoomSpawnPosition(activeRoom.EntryId);
                float pax = activeRoom.Center2D.x + (playerWorldPos.x - roomSpawn.x);
                float pay = activeRoom.Center2D.y + (playerWorldPos.z - roomSpawn.z);
                Vector2 pUv = AtlasToUv(pax, pay);
                DrawHeadingArrowUv(pUv, playerYawDeg, new Color32(110, 255, 180, 255));
            }
        }

        private void DrawQuestBeaconPx(Vector2 center, bool isTurnIn)
        {
            Color32 gold = isTurnIn ? new Color32(255, 226, 72, 255) : new Color32(255, 212, 42, 255);
            Color32 ring = new Color32(255, 240, 140, 255);
            int cx = Mathf.RoundToInt(center.x);
            int cy = Mathf.RoundToInt(center.y);
            const int r = 5;
            for (int dy = -r; dy <= r; dy++)
            {
                for (int dx = -r; dx <= r; dx++)
                {
                    if (Mathf.Abs(dx) + Mathf.Abs(dy) <= r)
                        Plot(cx + dx, cy + dy, gold);
                }
            }
            DrawCircleOutline(center, 8.5f, ring);
        }

        private void DrawShipInteriorMinimap(Vector3 playerWorldPos, float playerYawDeg)
        {
            ClearPixels(new Color32(8, 14, 24, 255));
            _shipCompHitRects.Clear();
            LastShipMinimapHasQuestBeacon = false;

            string activeId = _ship != null ? _ship.ActiveCompartmentId : "bridge";
            string effectiveTarget = QuestShipCompartment ?? string.Empty;
            if (!string.IsNullOrEmpty(effectiveTarget) &&
                string.Equals(effectiveTarget, "bay", StringComparison.OrdinalIgnoreCase) &&
                _ship?.Template != null &&
                !_ship.Template.Compartments.ContainsKey("bay") &&
                _ship.Template.Compartments.ContainsKey("hold"))
            {
                effectiveTarget = "hold";
            }

            if (_headerText != null)
            {
                string questTag = !string.IsNullOrEmpty(effectiveTarget) &&
                                  !string.Equals(effectiveTarget, activeId, StringComparison.OrdinalIgnoreCase)
                    ? $" [→ {effectiveTarget.ToUpperInvariant()}!]"
                    : string.Empty;
                _headerText.text = $"HULL DECKPLAN — {(activeId ?? "ABOARD").ToUpperInvariant()}{questTag}";
            }

            // Clean schematic layout of Dray / Corvette compartments
            var layout = new (string id, Rect r)[]
            {
                ("bridge",      new Rect(0.32f, 0.72f, 0.36f, 0.20f)),
                ("passage",     new Rect(0.36f, 0.44f, 0.28f, 0.24f)),
                ("crew",        new Rect(0.08f, 0.44f, 0.24f, 0.24f)),
                ("airlock",     new Rect(0.68f, 0.44f, 0.24f, 0.24f)),
                ("hold",        new Rect(0.28f, 0.16f, 0.44f, 0.24f)),
                ("bay",         new Rect(0.08f, 0.16f, 0.18f, 0.24f)),
                ("engineering", new Rect(0.74f, 0.16f, 0.18f, 0.24f)),
            };

            Vector2 activeCenter = new Vector2(0.5f, 0.5f);
            Vector2? beaconCenter = null;
            for (int i = 0; i < layout.Length; i++)
            {
                var item = layout[i];
                _shipCompHitRects.Add((item.r, item.id));
                bool isCur = string.Equals(item.id, activeId, StringComparison.OrdinalIgnoreCase);
                bool isQuest = !string.IsNullOrEmpty(effectiveTarget) &&
                               string.Equals(item.id, effectiveTarget, StringComparison.OrdinalIgnoreCase);
                Color32 fill = isCur
                    ? new Color32(28, 92, 118, 255)
                    : (isQuest ? new Color32(62, 52, 22, 255) : new Color32(18, 34, 52, 255));
                Color32 border = isQuest
                    ? new Color32(255, 214, 48, 255)
                    : (isCur ? new Color32(102, 234, 255, 255) : new Color32(66, 98, 128, 255));
                FillRectUv(item.r.min, item.r.max, fill);
                DrawRectOutlineUv(item.r.min, item.r.max, border);
                if (isCur) activeCenter = item.r.center;
                if (isQuest) beaconCenter = item.r.center;
            }
            if (beaconCenter.HasValue)
            {
                DrawQuestBeaconPx(UvToPx(beaconCenter.Value), isTurnIn: false);
                LastShipMinimapHasQuestBeacon = true;
            }
            DrawHeadingArrowUv(activeCenter, playerYawDeg, new Color32(110, 255, 180, 255));
        }

        private void DrawOrbitalRadar(float shipYawDeg)
        {
            ClearPixels(new Color32(6, 11, 19, 255));
            _orbitalBlipHits.Clear();

            double rangeKm = _space != null ? Math.Max(1.0, _space.RadarRangeKm / _zoom) : 1000.0;
            if (_headerText != null)
                _headerText.text = $"ORBITAL RADAR ({rangeKm:0} KM)";

            Vector2 center = new Vector2(MapTexSize * 0.5f, MapTexSize * 0.5f);
            Color32 ringCol = new Color32(34, 62, 86, 255);
            DrawCircleOutline(center, MapTexSize * 0.16f, ringCol);
            DrawCircleOutline(center, MapTexSize * 0.32f, ringCol);
            DrawCircleOutline(center, MapTexSize * 0.46f, ringCol);
            DrawLine(new Vector2(center.x, 6), new Vector2(center.x, MapTexSize - 6), ringCol);
            DrawLine(new Vector2(6, center.y), new Vector2(MapTexSize - 6, center.y), ringCol);

            // Own-ship chevron at center
            DrawHeadingArrowUv(new Vector2(0.5f, 0.5f), shipYawDeg, new Color32(82, 255, 178, 255));

            var blips = _space?.RadarBlips;
            if (blips == null) return;
            for (int i = 0; i < blips.Count; i++)
            {
                var b = blips[i];
                float nx = Mathf.Clamp((float)(b.OffsetYKm / rangeKm), -0.95f, 0.95f);
                float ny = Mathf.Clamp((float)(b.OffsetXKm / rangeKm), -0.95f, 0.95f);
                Vector2 uv = new Vector2(0.5f + nx * 0.46f, 0.5f + ny * 0.46f);
                Vector2 px = UvToPx(uv);

                Color32 col = b.IsTorpedo
                    ? new Color32(255, 140, 42, 255)
                    : (b.IsRock
                        ? new Color32(236, 194, 78, 255)
                        : (b.IsHostile
                            ? new Color32(255, 82, 76, 255)
                            : (b.IsFriendly ? new Color32(86, 232, 148, 255) : new Color32(102, 198, 248, 255))));

                DrawFilledCircle(px, b.IsTorpedo ? 2.5f : 3.5f, col);
                if (b.IsLocked)
                    DrawCircleOutline(px, 6.5f, new Color32(255, 224, 96, 255));

                if (!b.IsTorpedo && !string.IsNullOrEmpty(b.Id))
                    _orbitalBlipHits.Add((uv, b.Id, b.IsRock));
            }
        }

        private void DrawSurfaceMinimap(float roverYawDeg)
        {
            ClearPixels(new Color32(14, 16, 20, 255));
            if (_headerText != null)
                _headerText.text = $"QUARRY — {(_surface?.ActiveOreFamily ?? "VOLATILES").ToUpperInvariant()}";

            var grid = _surface?.TerrainGrid;
            if (grid == null || grid.Width <= 0 || grid.Height <= 0)
            {
                DrawHeadingArrowUv(new Vector2(0.5f, 0.5f), roverYawDeg, new Color32(110, 255, 180, 255));
                return;
            }

            _surfaceSpanCells = Mathf.Clamp(Mathf.RoundToInt(Mathf.Max(grid.Width, grid.Height) / _zoom), 12, Math.Max(grid.Width, grid.Height));
            _surfaceOriginX = Mathf.Clamp((_surface?.AuthoritativeSiteCellX ?? grid.Width / 2) - _surfaceSpanCells / 2, 0, Math.Max(0, grid.Width - _surfaceSpanCells));
            _surfaceOriginY = Mathf.Clamp((_surface?.AuthoritativeSiteCellY ?? grid.Height / 2) - _surfaceSpanCells / 2, 0, Math.Max(0, grid.Height - _surfaceSpanCells));

            for (int py = 0; py < MapTexSize; py++)
            {
                int gy = Mathf.Clamp(_surfaceOriginY + (py * _surfaceSpanCells) / MapTexSize, 0, grid.Height - 1);
                int row = py * MapTexSize;
                for (int px = 0; px < MapTexSize; px++)
                {
                    int gx = Mathf.Clamp(_surfaceOriginX + (px * _surfaceSpanCells) / MapTexSize, 0, grid.Width - 1);
                    float h = SampleGridElev(grid, gx, gy);
                    byte shade = (byte)Mathf.Clamp(Mathf.RoundToInt(26 + h * 18f), 18, 125);
                    _pixels[row + px] = new Color32((byte)(shade * 0.78f), (byte)(shade * 0.86f), shade, 255);
                }
            }

            float rU = ((_surface?.AuthoritativeSiteCellX ?? grid.Width / 2) - _surfaceOriginX) / (float)Math.Max(1, _surfaceSpanCells);
            float rV = ((_surface?.AuthoritativeSiteCellY ?? grid.Height / 2) - _surfaceOriginY) / (float)Math.Max(1, _surfaceSpanCells);
            DrawHeadingArrowUv(new Vector2(Mathf.Clamp01(rU), Mathf.Clamp01(rV)), roverYawDeg, new Color32(110, 255, 180, 255));
        }

        private static float SampleGridElev(DecodedTerrainGrid grid, int gx, int gy)
        {
            if (grid == null || grid.ElevMeters == null || grid.Width <= 0 || grid.Height <= 0)
                return 0f;
            int idx = gy * grid.Width + gx;
            return (uint)idx < (uint)grid.ElevMeters.Length ? grid.ElevMeters[idx] : 0f;
        }

        private static Color32 ClassifyArenaTerrainColor(TerrainCellClass cellClass, float elevM)
        {
            float relief = Mathf.Clamp(elevM * 0.045f, -0.20f, 0.35f);
            float mul = 1f + relief;
            Color32 baseCol;
            switch (cellClass)
            {
                case TerrainCellClass.WoodsLight:
                    baseCol = new Color32(34, 88, 48, 255);
                    break;
                case TerrainCellClass.WoodsHeavy:
                    baseCol = new Color32(22, 68, 34, 255);
                    break;
                case TerrainCellClass.Water:
                    baseCol = new Color32(22, 62, 108, 255);
                    break;
                case TerrainCellClass.WaterDeep:
                    baseCol = new Color32(14, 42, 82, 255);
                    break;
                case TerrainCellClass.Road:
                case TerrainCellClass.Paved:
                    baseCol = new Color32(84, 78, 66, 255);
                    break;
                case TerrainCellClass.Wall:
                case TerrainCellClass.Door:
                case TerrainCellClass.Platform:
                    baseCol = new Color32(114, 120, 132, 255);
                    break;
                case TerrainCellClass.Rock:
                case TerrainCellClass.Rubble:
                    baseCol = new Color32(78, 72, 62, 255);
                    break;
                case TerrainCellClass.Sand:
                    baseCol = new Color32(96, 86, 62, 255);
                    break;
                case TerrainCellClass.Ice:
                    baseCol = new Color32(86, 118, 134, 255);
                    break;
                case TerrainCellClass.Grass:
                    baseCol = new Color32(34, 58, 44, 255);
                    break;
                default:
                    baseCol = new Color32(30, 42, 56, 255);
                    break;
            }
            return new Color32(
                (byte)Mathf.Clamp(Mathf.RoundToInt(baseCol.r * mul), 8, 210),
                (byte)Mathf.Clamp(Mathf.RoundToInt(baseCol.g * mul), 8, 220),
                (byte)Mathf.Clamp(Mathf.RoundToInt(baseCol.b * mul), 8, 230),
                255);
        }

        private void DrawArenaMinimap()
        {
            ClearPixels(new Color32(10, 14, 22, 255));
            _arenaContactHits.Clear();
            _replayUnitHits.Clear();
            LastArenaMinimapHasSensorRing = false;
            LastArenaMinimapHasWeaponRangeRing = false;
            LastArenaMinimapHasEcmRing = false;
            LastArenaMinimapHasObjective = false;
            LastArenaMinimapHasWaypoint = false;
            LastArenaMinimapHasLockLine = false;
            LastArenaMinimapHoldOwner = "NEUTRAL";
            LastReplayMinimapShotLineCount = 0;

            bool isSpectator = _arena != null && _arena.IsSpectatorTheaterActive && _arena.ActiveReplayDoc != null;
            var st = _arena?.State;
            string holdOwner = _arena?.ActiveHoldZoneVisualOwner ?? "NEUTRAL";
            Color32 holdRingCol = holdOwner == "RED"
                ? new Color32(248, 74, 66, 255)
                : (holdOwner == "BLUE"
                    ? new Color32(66, 168, 255, 255)
                    : new Color32(255, 214, 66, 255));
            if (_headerText != null)
            {
                if (isSpectator)
                    _headerText.text = _arena.IsLiveObserverActive
                        ? $"OBSERVER — MATCH #{_arena.ActiveSpectatorMatchId}"
                        : $"REPLAY — MATCH #{_arena.ActiveSpectatorMatchId}";
                else
                    _headerText.text = st != null ? $"ARENA — {st.Callsign} ({st.Mode.ToUpperInvariant()})" : "ARENA RADAR";
            }

            var grid = _arena?.TerrainGrid;
            int gw = grid != null && grid.Width > 0 ? grid.Width : 64;
            int gh = grid != null && grid.Height > 0 ? grid.Height : 64;
            int span = grid != null ? Math.Max(1, grid.SpanCells) : 1;
            int originX = grid != null ? grid.OriginCellX : 0;
            int originY = grid != null ? grid.OriginCellY : 0;
            float cellSizeM = grid != null && grid.CellSizeM > 0.1f ? grid.CellSizeM : 10f;
            float siteCellM = cellSizeM / span;
            float totalCellsX = gw * span;
            float totalCellsY = gh * span;

            float ownCellX = originX + totalCellsX * 0.5f;
            float ownCellY = originY + totalCellsY * 0.5f;
            if (!isSpectator && st != null && siteCellM > 0.01f)
            {
                ownCellX = st.Xm / siteCellM;
                ownCellY = st.Zm / siteCellM;
            }
            else if (isSpectator && _arena.LatestReplayFrame != null &&
                     _arena.ReplayFocusUnitIndex >= 0 &&
                     _arena.ReplayFocusUnitIndex < _arena.LatestReplayFrame.Units.Length &&
                     _arena.LatestReplayFrame.Units[_arena.ReplayFocusUnitIndex].HasValue)
            {
                var fp = _arena.LatestReplayFrame.Units[_arena.ReplayFocusUnitIndex].Value;
                ownCellX = fp.CellX;
                ownCellY = fp.CellY;
            }

            float zoomClamped = Mathf.Clamp(_zoom, 1.0f, 2.5f);
            float winCellsX = totalCellsX / zoomClamped;
            float winCellsY = totalCellsY / zoomClamped;
            float minCellX = originX;
            float minCellY = originY;
            if (zoomClamped > 1.01f)
            {
                minCellX = Mathf.Clamp(ownCellX - winCellsX * 0.5f, originX, originX + Math.Max(0f, totalCellsX - winCellsX));
                minCellY = Mathf.Clamp(ownCellY - winCellsY * 0.5f, originY, originY + Math.Max(0f, totalCellsY - winCellsY));
            }
            _arenaMinCellX = minCellX;
            _arenaMinCellY = minCellY;
            _arenaSpanCellsX = Mathf.Max(1f, winCellsX);
            _arenaSpanCellsY = Mathf.Max(1f, winCellsY);

            Vector2 CellToUv(float cx, float cy) => new Vector2(
                (cx - _arenaMinCellX) / _arenaSpanCellsX,
                (cy - _arenaMinCellY) / _arenaSpanCellsY);

            if (grid != null && grid.Width > 0 && grid.Height > 0)
            {
                for (int py = 0; py < MapTexSize; py += 2)
                {
                    float cellY = _arenaMinCellY + ((py + 0.5f) / MapTexSize) * _arenaSpanCellsY;
                    int gy = Mathf.Clamp(Mathf.FloorToInt((cellY - originY) / span), 0, gh - 1);
                    for (int px = 0; px < MapTexSize; px += 2)
                    {
                        float cellX = _arenaMinCellX + ((px + 0.5f) / MapTexSize) * _arenaSpanCellsX;
                        int gx = Mathf.Clamp(Mathf.FloorToInt((cellX - originX) / span), 0, gw - 1);
                        int idx = gy * gw + gx;
                        TerrainCellClass cls = (grid.Classes != null && (uint)idx < (uint)grid.Classes.Length)
                            ? grid.Classes[idx]
                            : TerrainCellClass.Ground;
                        float elev = SampleGridElev(grid, gx, gy);
                        Color32 c = ClassifyArenaTerrainColor(cls, elev);
                        Plot(px, py, c); Plot(px + 1, py, c); Plot(px, py + 1, c); Plot(px + 1, py + 1, c);
                    }
                }

                // Draw surveyed KeySpots
                if (grid.KeySpots != null)
                {
                    for (int i = 0; i < grid.KeySpots.Count; i++)
                    {
                        var sp = grid.KeySpots[i];
                        Vector2 spUv = CellToUv(sp.CellX + 0.5f, sp.CellY + 0.5f);
                        if (spUv.x < 0f || spUv.x > 1f || spUv.y < 0f || spUv.y > 1f) continue;
                        bool isHold = string.Equals(sp.Letter, grid.HoldSpotLetter, StringComparison.OrdinalIgnoreCase);
                        Color32 spCol = isHold ? holdRingCol : new Color32(92, 224, 248, 255);
                        Vector2 spPx = UvToPx(spUv);
                        DrawFilledCircle(spPx, 2.5f, spCol);
                        if (isHold && (isSpectator || st == null || !st.HasDynamicObjective))
                        {
                            float holdRadPx = Mathf.Clamp((grid.HoldRadiusCells * span / _arenaSpanCellsX) * MapTexSize, 6f, MapTexSize * 0.4f);
                            DrawCircleOutline(spPx, holdRadPx, holdRingCol);
                            LastArenaMinimapHasObjective = true;
                            LastArenaMinimapHoldOwner = holdOwner;
                        }
                    }
                }
            }

            if (isSpectator)
            {
                var doc = _arena.ActiveReplayDoc;
                var frame = _arena.LatestReplayFrame;
                if (doc != null && frame != null)
                {
                    int n = Mathf.Min(doc.Roster.Count, frame.Units.Length);
                    // Draw live firefight shot tracers within [curMs - 1500ms .. curMs] underneath unit arrows
                    if (_arena.IsReplayModeActive && doc.Shots != null && doc.Shots.Count > 0)
                    {
                        float curMs = _arena.ReplayCursorMs;
                        float winStart = curMs - 1500f;
                        for (int sIdx = 0; sIdx < doc.Shots.Count; sIdx++)
                        {
                            var shot = doc.Shots[sIdx];
                            if (shot.TimeMs > curMs || shot.TimeMs < winStart) continue;
                            if (shot.ShooterIndex < 0 || shot.ShooterIndex >= n || shot.TargetIndex < 0 || shot.TargetIndex >= n) continue;
                            var shPose = frame.Units[shot.ShooterIndex];
                            var tgPose = frame.Units[shot.TargetIndex];
                            if (!shPose.HasValue || !tgPose.HasValue) continue;
                            Vector2 shUv = CellToUv(shPose.Value.CellX + 0.5f, shPose.Value.CellY + 0.5f);
                            Vector2 tgUv = CellToUv(tgPose.Value.CellX + 0.5f, tgPose.Value.CellY + 0.5f);
                            Color32 shotCol = string.Equals(shot.Outcome, "hit", StringComparison.OrdinalIgnoreCase)
                                ? new Color32(255, 216, 72, 255)
                                : (string.Equals(shot.Outcome, "flying", StringComparison.OrdinalIgnoreCase)
                                    ? new Color32(104, 224, 255, 255)
                                    : new Color32(152, 172, 190, 255));
                            DrawLine(UvToPx(new Vector2(Mathf.Clamp01(shUv.x), Mathf.Clamp01(shUv.y))),
                                     UvToPx(new Vector2(Mathf.Clamp01(tgUv.x), Mathf.Clamp01(tgUv.y))),
                                     shotCol);
                            LastReplayMinimapShotLineCount++;
                        }
                    }

                    for (int i = 0; i < n; i++)
                    {
                        var pose = frame.Units[i];
                        if (!pose.HasValue) continue;
                        Vector2 uv = CellToUv(pose.Value.CellX + 0.5f, pose.Value.CellY + 0.5f);
                        if (uv.x < 0f || uv.x > 1f || uv.y < 0f || uv.y > 1f) continue;
                        bool wrecked = i < frame.IsWrecked.Length && frame.IsWrecked[i];
                        Color32 col = wrecked
                            ? new Color32(115, 115, 122, 255)
                            : (doc.Roster[i].IsBlue ? new Color32(82, 184, 255, 255) : new Color32(255, 84, 78, 255));
                        Vector2 p = UvToPx(uv);
                        if (wrecked)
                            DrawFilledCircle(p, 3.0f, col);
                        else
                            DrawHeadingArrowUv(uv, pose.Value.HeadingDeg, col);

                        if (i == _arena.ReplayFocusUnitIndex)
                            DrawCircleOutline(p, 7.5f, new Color32(255, 224, 86, 255));

                        _replayUnitHits.Add((uv, i));
                    }
                }
                return;
            }

            if (st != null)
            {
                Vector2 ownUv = new Vector2(Mathf.Clamp01(CellToUv(ownCellX, ownCellY).x), Mathf.Clamp01(CellToUv(ownCellX, ownCellY).y));
                Vector2 ownPx = UvToPx(ownUv);

                // Dynamic Hold Objective perimeter (faction-tinted)
                if (st.HasDynamicObjective)
                {
                    Vector2 objUv = CellToUv(st.ObjectiveCellX + 0.5f, st.ObjectiveCellY + 0.5f);
                    if (objUv.x >= -0.1f && objUv.x <= 1.1f && objUv.y >= -0.1f && objUv.y <= 1.1f)
                    {
                        Vector2 objPx = UvToPx(objUv);
                        float radPx = Mathf.Clamp((st.ObjectiveRadiusCells * span / _arenaSpanCellsX) * MapTexSize, 7f, MapTexSize * 0.42f);
                        DrawCircleOutline(objPx, radPx, holdRingCol);
                        DrawFilledCircle(objPx, 2.5f, holdRingCol);
                        LastArenaMinimapHasObjective = true;
                        LastArenaMinimapHoldOwner = holdOwner;
                    }
                }

                // Active Autopilot Drive Waypoint + Route Line
                if (st.HasDriveWaypoint && !st.IsWreck)
                {
                    Vector2 wpUv = CellToUv(st.DriveWaypointCellX + 0.5f, st.DriveWaypointCellY + 0.5f);
                    Vector2 wpPx = UvToPx(new Vector2(Mathf.Clamp01(wpUv.x), Mathf.Clamp01(wpUv.y)));
                    DrawLine(ownPx, wpPx, new Color32(72, 230, 255, 255));
                    DrawCircleOutline(wpPx, 4.5f, new Color32(96, 245, 255, 255));
                    LastArenaMinimapHasWaypoint = true;
                }

                // Own SightMeters Sensor Ring colored by Scope
                float sightPx = 22f;
                if (!st.IsWreck && st.SightMeters > 5f && siteCellM > 0.01f)
                {
                    float sightCells = st.SightMeters / siteCellM;
                    sightPx = Mathf.Clamp((sightCells / _arenaSpanCellsX) * MapTexSize, 8f, MapTexSize * 0.48f);
                    Color32 ringCol = string.Equals(st.Scope, "hostile", StringComparison.OrdinalIgnoreCase)
                        ? new Color32(220, 86, 72, 255)
                        : (string.Equals(st.Scope, "unidentified", StringComparison.OrdinalIgnoreCase)
                            ? new Color32(228, 184, 64, 255)
                            : new Color32(58, 174, 206, 255));
                    DrawCircleOutline(ownPx, sightPx, ringCol);
                    LastArenaMinimapHasSensorRing = true;
                }

                // Primary Weapon Max-Range Envelope Ring (subtle emerald-gold outline)
                if (!st.IsWreck && siteCellM > 0.01f && _arena.TryGetPrimaryWeaponGunnery(out var primaryWpn) && primaryWpn.MaxRangeM > 15f)
                {
                    float wpnCells = primaryWpn.MaxRangeM / siteCellM;
                    float wpnPx = Mathf.Clamp((wpnCells / _arenaSpanCellsX) * MapTexSize, 10f, MapTexSize * 0.49f);
                    DrawCircleOutline(ownPx, wpnPx, new Color32(118, 212, 134, 255));
                    LastArenaMinimapHasWeaponRangeRing = true;
                }

                // Active ECM Jammer Bubble Ring (violet-magenta outline)
                if (st.IsEcmJammingActive && siteCellM > 0.01f)
                {
                    float ecmCells = Mathf.Max(20f, st.EcmRadiusMeters) / siteCellM;
                    float ecmPx = Mathf.Clamp((ecmCells / _arenaSpanCellsX) * MapTexSize, 7f, MapTexSize * 0.46f);
                    DrawCircleOutline(ownPx, ecmPx, new Color32(204, 118, 255, 255));
                    LastArenaMinimapHasEcmRing = true;
                }

                // Subtle Forward Sight Cone Wedge (±28° from own hull heading)
                float shownHeading = _arena != null ? _arena.PredictedHeadingDeg : st.HeadingDeg;
                if (!st.IsWreck)
                {
                    float coneLen = Mathf.Clamp(sightPx * 0.72f, 12f, 34f);
                    float leftRad = (shownHeading - 28f) * Mathf.Deg2Rad;
                    float rightRad = (shownHeading + 28f) * Mathf.Deg2Rad;
                    Color32 coneCol = new Color32(72, 196, 174, 255);
                    DrawLine(ownPx, ownPx + new Vector2(Mathf.Sin(leftRad) * coneLen, Mathf.Cos(leftRad) * coneLen), coneCol);
                    DrawLine(ownPx, ownPx + new Vector2(Mathf.Sin(rightRad) * coneLen, Mathf.Cos(rightRad) * coneLen), coneCol);
                }

                // Contacts + click-to-lock hit table + Locked Target Fire-Control Sightline Vector
                for (int i = 0; i < st.Contacts.Count; i++)
                {
                    var ct = st.Contacts[i];
                    Vector2 uv = CellToUv(ct.CellX + 0.5f, ct.CellY + 0.5f);
                    if (uv.x < 0f || uv.x > 1f || uv.y < 0f || uv.y > 1f) continue;
                    Color32 col = ct.IsWreck
                        ? new Color32(110, 110, 115, 255)
                        : (ct.Friendly ? new Color32(82, 184, 255, 255) : new Color32(255, 84, 78, 255));
                    Vector2 p = UvToPx(uv);
                    if (!st.IsWreck && !ct.IsWreck &&
                        !string.IsNullOrEmpty(st.LockedDesignator) &&
                        string.Equals(ct.Designator, st.LockedDesignator, StringComparison.OrdinalIgnoreCase))
                    {
                        DrawLine(ownPx, p, new Color32(255, 110, 68, 255));
                        LastArenaMinimapHasLockLine = true;
                    }
                    DrawFilledCircle(p, 3.5f, col);
                    if (string.Equals(ct.Designator, st.LockedDesignator, StringComparison.OrdinalIgnoreCase))
                        DrawCircleOutline(p, 6.5f, new Color32(255, 224, 96, 255));
                    if (!string.IsNullOrEmpty(ct.Designator))
                        _arenaContactHits.Add((uv, ct.Designator, !ct.Friendly && !ct.IsWreck));
                }

                DrawHeadingArrowUv(ownUv, shownHeading, new Color32(110, 255, 180, 255));
            }
        }

        private void OnDestroy()
        {
            if (_mapTex != null)
            {
                if (Application.isPlaying) Destroy(_mapTex);
                else DestroyImmediate(_mapTex);
                _mapTex = null;
            }
        }

        private void ClearPixels(Color32 c)
        {
            for (int i = 0; i < _pixels.Length; i++)
                _pixels[i] = c;
        }

        private static Vector2 UvToPx(Vector2 uv) => new Vector2(uv.x * (MapTexSize - 1), uv.y * (MapTexSize - 1));

        private void Plot(int x, int y, Color32 c)
        {
            if ((uint)x >= MapTexSize || (uint)y >= MapTexSize) return;
            _pixels[y * MapTexSize + x] = c;
        }

        private void FillRectUv(Vector2 u0, Vector2 u1, Color32 c)
        {
            int x0 = Mathf.Clamp(Mathf.RoundToInt(Mathf.Min(u0.x, u1.x) * (MapTexSize - 1)), 0, MapTexSize - 1);
            int x1 = Mathf.Clamp(Mathf.RoundToInt(Mathf.Max(u0.x, u1.x) * (MapTexSize - 1)), 0, MapTexSize - 1);
            int y0 = Mathf.Clamp(Mathf.RoundToInt(Mathf.Min(u0.y, u1.y) * (MapTexSize - 1)), 0, MapTexSize - 1);
            int y1 = Mathf.Clamp(Mathf.RoundToInt(Mathf.Max(u0.y, u1.y) * (MapTexSize - 1)), 0, MapTexSize - 1);
            for (int y = y0; y <= y1; y++)
            {
                int row = y * MapTexSize;
                for (int x = x0; x <= x1; x++)
                    _pixels[row + x] = c;
            }
        }

        private void DrawRectOutlineUv(Vector2 u0, Vector2 u1, Color32 c)
        {
            Vector2 p00 = UvToPx(new Vector2(u0.x, u0.y));
            Vector2 p10 = UvToPx(new Vector2(u1.x, u0.y));
            Vector2 p11 = UvToPx(new Vector2(u1.x, u1.y));
            Vector2 p01 = UvToPx(new Vector2(u0.x, u1.y));
            DrawLine(p00, p10, c);
            DrawLine(p10, p11, c);
            DrawLine(p11, p01, c);
            DrawLine(p01, p00, c);
        }

        private void DrawHeadingArrowUv(Vector2 uv, float yawDeg, Color32 col)
        {
            Vector2 p = UvToPx(uv);
            float rad = (90f - yawDeg) * Mathf.Deg2Rad;
            Vector2 dir = new Vector2(Mathf.Cos(rad), Mathf.Sin(rad));
            Vector2 perp = new Vector2(-dir.y, dir.x);
            Vector2 tip = p + dir * 7.5f;
            Vector2 left = p - dir * 4.0f + perp * 4.2f;
            Vector2 right = p - dir * 4.0f - perp * 4.2f;
            DrawLine(tip, left, col);
            DrawLine(tip, right, col);
            DrawLine(left, right, col);
            DrawFilledCircle(p, 2.2f, col);
        }

        private void DrawFilledCircle(Vector2 center, float radius, Color32 col)
        {
            int cx = Mathf.RoundToInt(center.x);
            int cy = Mathf.RoundToInt(center.y);
            int r = Mathf.CeilToInt(radius);
            float rSq = radius * radius;
            for (int dy = -r; dy <= r; dy++)
                for (int dx = -r; dx <= r; dx++)
                    if (dx * dx + dy * dy <= rSq)
                        Plot(cx + dx, cy + dy, col);
        }

        private void DrawCircleOutline(Vector2 center, float radius, Color32 col)
        {
            int segs = Mathf.Clamp(Mathf.RoundToInt(radius * 3f), 20, 96);
            Vector2 prev = Vector2.zero;
            for (int i = 0; i <= segs; i++)
            {
                float a = (Mathf.PI * 2f * i) / segs;
                Vector2 p = new Vector2(center.x + Mathf.Cos(a) * radius, center.y + Mathf.Sin(a) * radius);
                if (i > 0) DrawLine(prev, p, col);
                prev = p;
            }
        }

        private void DrawLine(Vector2 a, Vector2 b, Color32 col)
        {
            int x0 = Mathf.RoundToInt(a.x);
            int y0 = Mathf.RoundToInt(a.y);
            int x1 = Mathf.RoundToInt(b.x);
            int y1 = Mathf.RoundToInt(b.y);
            int dx = Mathf.Abs(x1 - x0);
            int dy = Mathf.Abs(y1 - y0);
            int sx = x0 < x1 ? 1 : -1;
            int sy = y0 < y1 ? 1 : -1;
            int err = dx - dy;
            int guard = MapTexSize * 3;
            while (guard-- > 0)
            {
                Plot(x0, y0, col);
                if (x0 == x1 && y0 == y1) break;
                int e2 = err * 2;
                if (e2 > -dy) { err -= dy; x0 += sx; }
                if (e2 < dx) { err += dx; y0 += sy; }
            }
        }

        private GameObject CreatePanel(Transform parent, string name, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), anchorMin, anchorMax, offsetMin, offsetMax);
            return go;
        }

        private Text CreateText(Transform parent, string content, int size, FontStyle style, TextAnchor align, Color color)
        {
            var go = new GameObject("Text", typeof(RectTransform), typeof(Text));
            go.transform.SetParent(parent, false);
            var t = go.GetComponent<Text>();
            t.font = _font;
            t.text = content;
            t.fontSize = size;
            t.fontStyle = style;
            t.alignment = align;
            t.color = color;
            t.supportRichText = false;
            t.raycastTarget = false;
            return t;
        }

        private Button CreateSmallButton(Transform parent, string label, Color bg, Action onClick, out Text labelOut)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = Color.white;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.colors = new ColorBlock
            {
                normalColor = bg,
                highlightedColor = new Color(Mathf.Clamp01(bg.r * 1.35f + 0.06f), Mathf.Clamp01(bg.g * 1.35f + 0.06f), Mathf.Clamp01(bg.b * 1.35f + 0.06f), 1f),
                pressedColor = new Color(Mathf.Clamp01(bg.r * 0.65f), Mathf.Clamp01(bg.g * 0.65f), Mathf.Clamp01(bg.b * 0.65f), 1f),
                selectedColor = bg,
                disabledColor = new Color(0.2f, 0.22f, 0.25f, 0.6f),
                colorMultiplier = 1f,
                fadeDuration = 0.05f
            };
            btn.onClick.AddListener(() => onClick?.Invoke());
            labelOut = CreateText(go.transform, label, 10, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(labelOut.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
            return btn;
        }

        private static void SetRect(RectTransform rt, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax)
        {
            rt.anchorMin = anchorMin;
            rt.anchorMax = anchorMax;
            rt.offsetMin = offsetMin;
            rt.offsetMax = offsetMax;
        }
    }
}
