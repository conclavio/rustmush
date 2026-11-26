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
        public event Action OnOpenSolarChartRequested;

        public bool IsCollapsed { get; private set; }
        public bool IsDockedMidLeft { get; private set; }
        public TacSimActiveStage ActiveStage { get; private set; } = TacSimActiveStage.StationWalk;
        public int InspectedStationFloor { get; private set; }
        public string HeaderText => _headerText != null ? _headerText.text : string.Empty;

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

        // Cached clickable bounds in UV space `[0,1]` for Station rooms / Ship compartments / Orbital blips
        private readonly List<(Rect uvRect, string id)> _stationRoomHitRects = new List<(Rect, string)>();
        private readonly List<(Rect uvRect, string id)> _shipCompHitRects = new List<(Rect, string)>();
        private readonly List<(Vector2 uv, string id, bool isRock)> _orbitalBlipHits = new List<(Vector2, string, bool)>();
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
            if (_bodyContainer != null) _bodyContainer.SetActive(true);
            if (_collapseBtnLabel != null) _collapseBtnLabel.text = "—";
            ApplyDockPlacement();
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

            if (_headerText != null)
            {
                string deckTag = InspectedStationFloor == activeFloor ? $"DECK {InspectedStationFloor + 1}" : $"DECK {InspectedStationFloor + 1} (VIEW)";
                string portName = !string.IsNullOrEmpty(pack.Name) ? pack.Name.ToUpperInvariant() : "STATION";
                _headerText.text = $"{deckTag} — {portName}";
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
                Color32 fill = isCur
                    ? new Color32(28, 84, 112, 255)
                    : (r.Restricted ? new Color32(48, 28, 34, 255) : new Color32(20, 34, 52, 255));
                Color32 border = isCur
                    ? new Color32(92, 226, 255, 255)
                    : new Color32(68, 102, 134, 255);
                FillRectUv(u0, u1, fill);
                DrawRectOutlineUv(u0, u1, border);
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

        private void DrawShipInteriorMinimap(Vector3 playerWorldPos, float playerYawDeg)
        {
            ClearPixels(new Color32(8, 14, 24, 255));
            _shipCompHitRects.Clear();

            string activeId = _ship != null ? _ship.ActiveCompartmentId : "bridge";
            if (_headerText != null)
                _headerText.text = $"HULL DECKPLAN — {(activeId ?? "ABOARD").ToUpperInvariant()}";

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
            for (int i = 0; i < layout.Length; i++)
            {
                var item = layout[i];
                _shipCompHitRects.Add((item.r, item.id));
                bool isCur = string.Equals(item.id, activeId, StringComparison.OrdinalIgnoreCase);
                Color32 fill = isCur ? new Color32(28, 92, 118, 255) : new Color32(18, 34, 52, 255);
                Color32 border = isCur ? new Color32(102, 234, 255, 255) : new Color32(66, 98, 128, 255);
                FillRectUv(item.r.min, item.r.max, fill);
                DrawRectOutlineUv(item.r.min, item.r.max, border);
                if (isCur) activeCenter = item.r.center;
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

        private void DrawArenaMinimap()
        {
            ClearPixels(new Color32(10, 14, 22, 255));
            var st = _arena?.State;
            if (_headerText != null)
                _headerText.text = st != null ? $"ARENA — {st.Callsign} ({st.Mode.ToUpperInvariant()})" : "ARENA RADAR";

            var grid = _arena?.TerrainGrid;
            int gw = grid != null && grid.Width > 0 ? grid.Width : 64;
            int gh = grid != null && grid.Height > 0 ? grid.Height : 64;

            if (grid != null && grid.Width > 0 && grid.Height > 0)
            {
                for (int py = 0; py < MapTexSize; py += 2)
                {
                    int gy = Mathf.Clamp((py * gh) / MapTexSize, 0, gh - 1);
                    for (int px = 0; px < MapTexSize; px += 2)
                    {
                        int gx = Mathf.Clamp((px * gw) / MapTexSize, 0, gw - 1);
                        byte s = (byte)Mathf.Clamp(Mathf.RoundToInt(22 + SampleGridElev(grid, gx, gy) * 14f), 16, 110);
                        Color32 c = new Color32((byte)(s * 0.8f), (byte)(s * 0.9f), s, 255);
                        Plot(px, py, c); Plot(px + 1, py, c); Plot(px, py + 1, c); Plot(px + 1, py + 1, c);
                    }
                }
            }

            if (st != null)
            {
                for (int i = 0; i < st.Contacts.Count; i++)
                {
                    var ct = st.Contacts[i];
                    Vector2 uv = new Vector2(Mathf.Clamp01(ct.CellX / (float)Math.Max(1, gw)), Mathf.Clamp01(ct.CellY / (float)Math.Max(1, gh)));
                    Color32 col = ct.IsWreck
                        ? new Color32(110, 110, 115, 255)
                        : (ct.Friendly ? new Color32(82, 184, 255, 255) : new Color32(255, 84, 78, 255));
                    Vector2 p = UvToPx(uv);
                    DrawFilledCircle(p, 3.5f, col);
                    if (string.Equals(ct.Designator, st.LockedDesignator, StringComparison.OrdinalIgnoreCase))
                        DrawCircleOutline(p, 6.5f, new Color32(255, 224, 96, 255));
                }

                float pxNorm = Mathf.Clamp01((st.Xm / 40f) / Math.Max(1, gw));
                float pzNorm = Mathf.Clamp01((st.Zm / 40f) / Math.Max(1, gh));
                DrawHeadingArrowUv(new Vector2(pxNorm, pzNorm), st.HeadingDeg, new Color32(110, 255, 180, 255));
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
