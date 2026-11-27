using System;
using System.Collections.Generic;
using TacSim.Arena;
using TacSim.Core;
using TacSim.Net;
using TacSim.Space;
using TacSim.Surface;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum TacticalOverviewFilter
    {
        All = 0,
        Primary = 1,   // Space: Ships | Surface: Rich Ore | Arena: Hostiles
        Secondary = 2  // Space: Rocks | Surface: All Readings | Arena: Friendlies
    }

    public readonly struct OverviewRowEntry
    {
        public readonly string Id;
        public readonly string DisplayName;
        public readonly string DistanceText;
        public readonly float SortDistance;
        public readonly string Badge;
        public readonly Color AccentColor;
        public readonly bool IsLocked;
        public readonly bool IsRock;
        public readonly int SurfaceCellX;
        public readonly int SurfaceCellY;

        public OverviewRowEntry(
            string id,
            string displayName,
            string distanceText,
            float sortDistance,
            string badge,
            Color accentColor,
            bool isLocked = false,
            bool isRock = false,
            int surfaceCellX = -1,
            int surfaceCellY = -1)
        {
            Id = id ?? string.Empty;
            DisplayName = displayName ?? string.Empty;
            DistanceText = distanceText ?? string.Empty;
            SortDistance = sortDistance;
            Badge = badge ?? string.Empty;
            AccentColor = accentColor;
            IsLocked = isLocked;
            IsRock = isRock;
            SurfaceCellX = surfaceCellX;
            SurfaceCellY = surfaceCellY;
        }
    }

    /// <summary>
    /// EVE Online / Homeworld-style `[T]` Live Sortable Tactical Overview Drawer.
    /// Docks cleanly on the left edge beneath `UnitFrames` (`y = -428..-172`) with zero overlap
    /// against Minimap, CommsDrawer, SpaceFireCard, or Docking HUD.
    /// - Orbital Space: nearest-sorted Ships, Hostiles, Asteroids &amp; Torpedoes with 1-click Lock / Approach.
    /// - Planetary Surface: nearest-sorted Scanned Ore Pockets with 1-click Rover Drive-To (`+tv/drive`).
    /// - OOC Arena: nearest-sorted Enemy/Friendly Vehicles with Hit % and 1-click Lock.
    /// </summary>
    public sealed class TacSimTacticalOverview : MonoBehaviour
    {
        public event Action<string, bool> OnSpaceRowClicked;       // (id, isRock)
        public event Action<string, bool> OnSpaceRowSecondaryAct;  // (id, isRock -> match vel / beam)
        public event Action<int, int> OnSurfaceBeaconDriveClicked; // (siteX, siteY)
        public event Action<string> OnArenaContactLockClicked;     // (designator)
        public event Action<string> OnArenaContactScanClicked;     // (designator)

        private const int MaxVisibleRows = 7;

        private sealed class PooledRow
        {
            public GameObject Root;
            public Image Bg;
            public Button RowBtn;
            public Text Label;
            public Button ActBtn;
            public Text ActLabel;
            public OverviewRowEntry BoundEntry;
        }

        private Font _font;
        private GameObject _root;
        private RectTransform _rootRect;
        private GameObject _body;
        private Text _headerText;
        private Text _collapseBtnLabel;
        private Text _sortBtnLabel;
        private Button _lockBestBtn;
        private readonly Image[] _filterImages = new Image[3];
        private readonly Text[] _filterLabels = new Text[3];
        private readonly List<PooledRow> _rowPool = new List<PooledRow>(MaxVisibleRows);
        private readonly List<OverviewRowEntry> _entries = new List<OverviewRowEntry>(32);
        private Text _emptyLabel;

        private SpaceFlightController _space;
        private SurfaceMiningController _surface;
        private OocArenaController _arena;
        private TacSimActiveStage _activeStage = TacSimActiveStage.StationWalk;
        private bool _userEnabled = true;
        private bool _collapsed;
        private bool _avoidMidLeftMinimap;
        private bool _sortArenaByOdds;
        private float _nextSyncAt;

        public TacticalOverviewFilter ActiveFilter { get; private set; } = TacticalOverviewFilter.All;
        public bool IsVisible => _root != null && _root.activeSelf;
        public bool IsUserEnabled => _userEnabled;
        public bool IsCollapsed => _collapsed;
        public bool IsAvoidingMidLeftMinimap => _avoidMidLeftMinimap;
        public bool SortArenaByOdds => _sortArenaByOdds;
        public int VisibleRowCount { get; private set; }
        public int TotalEntryCount => _entries.Count;
        public string FirstRowId => _entries.Count > 0 ? _entries[0].Id : string.Empty;

        public void BuildDrawer(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _rowPool.Clear();

            // Left edge below UnitFrames (shifts right of TacticalMinimap when minimap docks Mid-Left)
            _root = CreatePanel(
                hudRoot,
                "TacticalOverviewDrawer",
                new Vector2(0f, 1f),
                new Vector2(0f, 1f),
                new Vector2(16f, -430f),
                new Vector2(316f, -172f),
                new Color(0.03f, 0.06f, 0.10f, 0.90f));
            _rootRect = _root.GetComponent<RectTransform>();

            var hdr = CreatePanel(_root.transform, "OverviewHeader", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -24f), Vector2.zero, new Color(0.06f, 0.13f, 0.21f, 0.96f));
            _headerText = CreateText(hdr.transform, "TACTICAL OVERVIEW [T]", 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.48f, 0.92f, 1f));
            SetRect(_headerText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-120f, 0f));

            _lockBestBtn = CreateSmallButton(hdr.transform, "BEST", new Color(0.56f, 0.20f, 0.16f), () => LockBestArenaTarget(), out _);
            SetRect(_lockBestBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-116f, 2f), new Vector2(-76f, -2f));
            _lockBestBtn.gameObject.SetActive(false);

            var sortBtn = CreateSmallButton(hdr.transform, "DIST", new Color(0.14f, 0.28f, 0.42f), ToggleSortMode, out _sortBtnLabel);
            SetRect(sortBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-72f, 2f), new Vector2(-30f, -2f));

            var colBtn = CreateSmallButton(hdr.transform, "—", new Color(0.16f, 0.28f, 0.40f), ToggleCollapsed, out _collapseBtnLabel);
            SetRect(colBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-26f, 2f), new Vector2(-2f, -2f));

            _body = CreatePanel(_root.transform, "OverviewBody", Vector2.zero, Vector2.one, Vector2.zero, new Vector2(0f, -24f), Color.clear);

            // Filter bar
            var fBar = CreatePanel(_body.transform, "FilterBar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(6f, -24f), new Vector2(-6f, -3f), Color.clear);
            var hlg = fBar.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 4f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            string[] defFilters = { "ALL", "SHIPS", "ROCKS" };
            for (int i = 0; i < 3; i++)
            {
                var captured = (TacticalOverviewFilter)i;
                var fb = CreateSmallButton(fBar.transform, defFilters[i], new Color(0.11f, 0.20f, 0.30f), () => SetFilter(captured), out _filterLabels[i]);
                _filterImages[i] = fb.GetComponent<Image>();
            }

            // Rows container
            var rowsBox = CreatePanel(_body.transform, "OverviewRows", Vector2.zero, Vector2.one, new Vector2(6f, 6f), new Vector2(-6f, -28f), new Color(0.02f, 0.04f, 0.08f, 0.85f));
            var vlg = rowsBox.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 3f;
            vlg.padding = new RectOffset(4, 4, 4, 4);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxVisibleRows; i++)
            {
                var rowGo = CreatePanel(rowsBox.transform, $"OvRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 25f), new Color(0.06f, 0.11f, 0.18f, 0.92f));
                rowGo.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 25f);
                var img = rowGo.GetComponent<Image>();
                img.raycastTarget = true;
                var rowBtn = rowGo.AddComponent<Button>();
                rowBtn.targetGraphic = img;

                var lbl = CreateText(rowGo.transform, string.Empty, 10, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
                lbl.supportRichText = true;
                SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 0f), new Vector2(-56f, 0f));

                var row = new PooledRow { Root = rowGo, Bg = img, RowBtn = rowBtn, Label = lbl };
                rowBtn.onClick.AddListener(() => HandlePrimaryRowClick(row.BoundEntry));

                row.ActBtn = CreateSmallButton(rowGo.transform, "ACT", new Color(0.16f, 0.42f, 0.54f), () => HandleSecondaryRowClick(row.BoundEntry), out row.ActLabel);
                SetRect(row.ActBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-52f, 2f), new Vector2(-3f, -2f));

                rowGo.SetActive(false);
                _rowPool.Add(row);
            }

            _emptyLabel = CreateText(rowsBox.transform, "No contacts / readings in sensor range.", 11, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.60f, 0.74f, 0.86f));
            SetRect(_emptyLabel.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 6f), new Vector2(-6f, -6f));

            RefreshFilterVisuals();
            _root.SetActive(false);
        }

        public void BindControllers(SpaceFlightController space, SurfaceMiningController surface, OocArenaController arena)
        {
            _space = space;
            _surface = surface;
            _arena = arena;
        }

        public void ResetSessionState()
        {
            _entries.Clear();
            VisibleRowCount = 0;
            ActiveFilter = TacticalOverviewFilter.All;
            _userEnabled = true;
            _collapsed = false;
            _avoidMidLeftMinimap = false;
            _sortArenaByOdds = false;
            if (_sortBtnLabel != null) _sortBtnLabel.text = "DIST";
            ApplyDrawerBounds();
            if (_root != null) _root.SetActive(false);
        }

        public void ToggleVisible()
        {
            _userEnabled = !_userEnabled;
            SyncOverview(_activeStage, forceImmediate: true);
        }

        public void SetFilter(TacticalOverviewFilter filter)
        {
            ActiveFilter = filter;
            RefreshFilterVisuals();
            SyncOverview(_activeStage, forceImmediate: true);
        }

        public void ToggleSortMode()
        {
            _sortArenaByOdds = !_sortArenaByOdds;
            if (_sortBtnLabel != null)
                _sortBtnLabel.text = _sortArenaByOdds ? "ODDS" : "DIST";
            SyncOverview(_activeStage, forceImmediate: true);
        }

        public void SetAvoidMidLeftMinimap(bool avoid)
        {
            if (_avoidMidLeftMinimap == avoid) return;
            _avoidMidLeftMinimap = avoid;
            ApplyDrawerBounds();
        }

        public void ToggleCollapsed()
        {
            _collapsed = !_collapsed;
            if (_body != null) _body.SetActive(!_collapsed);
            if (_collapseBtnLabel != null) _collapseBtnLabel.text = _collapsed ? "+" : "—";
            ApplyDrawerBounds();
        }

        private void ApplyDrawerBounds()
        {
            if (_rootRect == null) return;
            float xLeft = _avoidMidLeftMinimap ? 242f : 16f;
            float xRight = xLeft + 300f;
            float yBot = _collapsed ? -196f : -412f;
            SetRect(_rootRect, new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xLeft, yBot), new Vector2(xRight, -172f));
        }

        public void SyncOverview(TacSimActiveStage stage, bool hideForOverlay = false, bool forceImmediate = false)
        {
            _activeStage = stage;
            if (_root == null) return;

            bool stageEligible =
                (stage == TacSimActiveStage.ShipHelmSeated && (_space == null || !_space.IsDockingActive)) ||
                stage == TacSimActiveStage.PlanetarySurface ||
                stage == TacSimActiveStage.SimulatorArena;

            bool shouldShow = _userEnabled && stageEligible && !hideForOverlay;
            if (_root.activeSelf != shouldShow)
                _root.SetActive(shouldShow);
            if (!shouldShow)
            {
                VisibleRowCount = 0;
                return;
            }

            if (!forceImmediate && Application.isPlaying && Time.unscaledTime < _nextSyncAt)
                return;
            _nextSyncAt = Time.unscaledTime + 0.22f;

            RebuildEntriesForStage(stage);
            RefreshStageFilterLabels(stage);
            RenderRows(stage);
        }

        public bool ClickRowByIndex(int index)
        {
            if (index < 0 || index >= VisibleRowCount || index >= _rowPool.Count) return false;
            HandlePrimaryRowClick(_rowPool[index].BoundEntry);
            return true;
        }

        public bool ClickRowSecondaryByIndex(int index)
        {
            if (index < 0 || index >= VisibleRowCount || index >= _rowPool.Count) return false;
            HandleSecondaryRowClick(_rowPool[index].BoundEntry);
            return true;
        }

        public string GetFilterTabLabelForTests(int index) =>
            (index >= 0 && index < _filterLabels.Length && _filterLabels[index] != null)
                ? (_filterLabels[index].text ?? string.Empty)
                : string.Empty;

        public bool LockBestArenaTarget()
        {
            var st = _arena?.State;
            if (st == null || st.Contacts == null || st.Contacts.Count == 0) return false;
            ArenaContactState best = null;
            for (int i = 0; i < st.Contacts.Count; i++)
            {
                var ct = st.Contacts[i];
                if (ct == null || ct.IsWreck || ct.Friendly || string.IsNullOrEmpty(ct.Designator)) continue;
                if (best == null ||
                    ct.HitOddsPct > best.HitOddsPct ||
                    (ct.HitOddsPct == best.HitOddsPct && ct.RangeM < best.RangeM))
                {
                    best = ct;
                }
            }
            if (best == null) return false;
            OnArenaContactLockClicked?.Invoke(best.Designator);
            return true;
        }

        private void RebuildEntriesForStage(TacSimActiveStage stage)
        {
            _entries.Clear();
            switch (stage)
            {
                case TacSimActiveStage.ShipHelmSeated:
                {
                    if (_space == null) break;
                    var blips = _space.RadarBlips;
                    for (int i = 0; i < blips.Count; i++)
                    {
                        var b = blips[i];
                        if (b.IsTorpedo || string.IsNullOrEmpty(b.Id)) continue;
                        if (ActiveFilter == TacticalOverviewFilter.Primary && b.IsRock) continue;
                        if (ActiveFilter == TacticalOverviewFilter.Secondary && !b.IsRock) continue;

                        string distStr = b.RangeKm >= 1.0 ? $"{b.RangeKm:0.1}km" : $"{b.RangeKm * 1000.0:0}m";
                        Color col = b.IsLocked
                            ? new Color(1.0f, 0.34f, 0.24f)
                            : (b.IsRock ? new Color(1.0f, 0.80f, 0.30f) : (b.IsHostile ? new Color(1.0f, 0.44f, 0.36f) : new Color(0.36f, 0.90f, 1.0f)));
                        string badge = b.IsLocked ? "LOCK" : (b.IsRock ? "ORE" : (b.IsHostile ? "HOST" : "SHIP"));
                        _entries.Add(new OverviewRowEntry(
                            b.Id,
                            string.IsNullOrEmpty(b.Label) ? b.Id : b.Label,
                            distStr,
                            (float)b.RangeKm,
                            badge,
                            col,
                            b.IsLocked,
                            b.IsRock));
                    }
                    break;
                }
                case TacSimActiveStage.PlanetarySurface:
                {
                    if (_surface == null) break;
                    Vector3 roverPos = _surface.RoverTransform != null ? _surface.RoverTransform.position : Vector3.zero;
                    var beacons = _surface.OreBeacons;
                    for (int i = 0; i < beacons.Count; i++)
                    {
                        var bc = beacons[i];
                        if (ActiveFilter == TacticalOverviewFilter.Primary && !bc.IsRich) continue;
                        if (ActiveFilter == TacticalOverviewFilter.Secondary && bc.IsRich) continue;

                        float distM = Vector3.Distance(roverPos, bc.WorldPos);
                        _entries.Add(new OverviewRowEntry(
                            $"{bc.SiteCellX},{bc.SiteCellY}",
                            string.IsNullOrEmpty(bc.Label) ? $"Cell ({bc.SiteCellX},{bc.SiteCellY})" : bc.Label,
                            $"{distM:0}m",
                            distM,
                            bc.IsRich ? "RICH" : "ORE",
                            bc.IsRich ? new Color(0.28f, 1.0f, 0.58f) : new Color(0.40f, 0.88f, 1.0f),
                            isLocked: false,
                            isRock: true,
                            surfaceCellX: bc.SiteCellX,
                            surfaceCellY: bc.SiteCellY));
                    }
                    break;
                }
                case TacSimActiveStage.SimulatorArena:
                {
                    var st = _arena?.State;
                    if (st == null) break;
                    for (int i = 0; i < st.Contacts.Count; i++)
                    {
                        var ct = st.Contacts[i];
                        if (ct.IsWreck || string.IsNullOrEmpty(ct.Designator)) continue;
                        if (ActiveFilter == TacticalOverviewFilter.Primary && ct.Friendly) continue;
                        if (ActiveFilter == TacticalOverviewFilter.Secondary && !ct.Friendly) continue;

                        bool isLocked = string.Equals(ct.Designator, st.LockedDesignator, StringComparison.OrdinalIgnoreCase);
                        Color col = isLocked
                            ? new Color(1.0f, 0.30f, 0.22f)
                            : (ct.Friendly ? new Color(0.30f, 0.90f, 0.68f) : new Color(1.0f, 0.56f, 0.24f));
                        string oddsTag = ct.HitOddsPct > 0 ? $"{ct.HitOddsPct}%" : (ct.Friendly ? "ALLY" : "FOE");
                        string aspectTag = !string.IsNullOrEmpty(ct.Aspect) && !string.Equals(ct.Aspect, "front", StringComparison.OrdinalIgnoreCase)
                            ? $" [{ct.Aspect.ToUpperInvariant()}]"
                            : string.Empty;
                        string dmgTag = string.Equals(ct.HealthWord, "crippled", StringComparison.OrdinalIgnoreCase)
                            ? " CRIT"
                            : (string.Equals(ct.HealthWord, "scarred", StringComparison.OrdinalIgnoreCase) ? " SCAR" : string.Empty);
                        string moveDot = ct.Moving ? " •" : string.Empty;
                        float sortKey = _sortArenaByOdds
                            ? (ct.Friendly ? 10000f + ct.RangeM : (1000f - ct.HitOddsPct * 10f) + ct.RangeM * 0.001f)
                            : ct.RangeM;
                        _entries.Add(new OverviewRowEntry(
                            ct.Designator,
                            $"{ct.Designator} {(string.IsNullOrEmpty(ct.ClassLabel) ? ct.Party : ct.ClassLabel)}{aspectTag}{dmgTag}{moveDot}",
                            $"{ct.RangeM:0}m",
                            sortKey,
                            isLocked ? "LOCK" : oddsTag,
                            col,
                            isLocked: isLocked));
                    }
                    break;
                }
            }

            _entries.Sort((a, b) => a.SortDistance.CompareTo(b.SortDistance));
        }

        private void RefreshStageFilterLabels(TacSimActiveStage stage)
        {
            bool isArena = stage == TacSimActiveStage.SimulatorArena;
            if (_lockBestBtn != null && _lockBestBtn.gameObject.activeSelf != isArena)
                _lockBestBtn.gameObject.SetActive(isArena);

            if (_headerText != null)
            {
                _headerText.text = stage == TacSimActiveStage.PlanetarySurface
                    ? $"ORE SCANNER ({_entries.Count}) [T]"
                    : (isArena
                        ? $"ARENA TARGETS ({_entries.Count}) [T]"
                        : $"ORBITAL OVERVIEW ({_entries.Count}) [T]");
            }

            if (isArena && _arena?.State?.Contacts != null)
            {
                int totalLive = 0, hostileLive = 0, allyLive = 0;
                var contacts = _arena.State.Contacts;
                for (int i = 0; i < contacts.Count; i++)
                {
                    var ct = contacts[i];
                    if (ct == null || ct.IsWreck || string.IsNullOrEmpty(ct.Designator)) continue;
                    totalLive++;
                    if (ct.Friendly) allyLive++;
                    else hostileLive++;
                }
                if (_filterLabels[0] != null) _filterLabels[0].text = $"ALL ({totalLive})";
                if (_filterLabels[1] != null) _filterLabels[1].text = $"FOES ({hostileLive})";
                if (_filterLabels[2] != null) _filterLabels[2].text = $"ALLY ({allyLive})";
            }
            else
            {
                if (_filterLabels[0] != null) _filterLabels[0].text = "ALL";
                if (_filterLabels[1] != null)
                    _filterLabels[1].text = stage == TacSimActiveStage.PlanetarySurface ? "RICH" : "SHIPS";
                if (_filterLabels[2] != null)
                    _filterLabels[2].text = stage == TacSimActiveStage.PlanetarySurface ? "STD" : "ROCKS";
            }
        }

        private void RenderRows(TacSimActiveStage stage)
        {
            int shown = Mathf.Min(MaxVisibleRows, _entries.Count);
            VisibleRowCount = shown;
            if (_emptyLabel != null) _emptyLabel.gameObject.SetActive(shown == 0);

            for (int i = 0; i < _rowPool.Count; i++)
            {
                var row = _rowPool[i];
                if (i < shown)
                {
                    var e = _entries[i];
                    row.BoundEntry = e;
                    row.Bg.color = e.IsLocked
                        ? new Color(0.32f, 0.14f, 0.14f, 0.96f)
                        : new Color(0.06f, 0.12f, 0.19f, 0.92f);
                    string hex = ToHexRgb(e.AccentColor);
                    row.Label.text = $"<color=#{hex}>[{e.Badge}]</color> {MushProtocol.EscapeUnityRichText(e.DisplayName)}  <color=#88BBDD>{e.DistanceText}</color>";
                    if (row.ActLabel != null)
                    {
                        row.ActLabel.text = stage == TacSimActiveStage.PlanetarySurface
                            ? "DRIVE"
                            : (stage == TacSimActiveStage.SimulatorArena
                                ? "SCAN"
                                : (e.IsRock ? "MATCH" : "BEAM"));
                    }
                    row.Root.SetActive(true);
                }
                else
                {
                    row.Root.SetActive(false);
                }
            }
        }

        private void HandlePrimaryRowClick(OverviewRowEntry e)
        {
            if (string.IsNullOrEmpty(e.Id)) return;
            if (_activeStage == TacSimActiveStage.PlanetarySurface)
            {
                if (e.SurfaceCellX >= 0 && e.SurfaceCellY >= 0)
                    OnSurfaceBeaconDriveClicked?.Invoke(e.SurfaceCellX, e.SurfaceCellY);
            }
            else if (_activeStage == TacSimActiveStage.SimulatorArena)
            {
                OnArenaContactLockClicked?.Invoke(e.Id);
            }
            else if (_activeStage == TacSimActiveStage.ShipHelmSeated)
            {
                OnSpaceRowClicked?.Invoke(e.Id, e.IsRock);
            }
        }

        private void HandleSecondaryRowClick(OverviewRowEntry e)
        {
            if (string.IsNullOrEmpty(e.Id)) return;
            if (_activeStage == TacSimActiveStage.PlanetarySurface)
            {
                if (e.SurfaceCellX >= 0 && e.SurfaceCellY >= 0)
                    OnSurfaceBeaconDriveClicked?.Invoke(e.SurfaceCellX, e.SurfaceCellY);
            }
            else if (_activeStage == TacSimActiveStage.SimulatorArena)
            {
                OnArenaContactScanClicked?.Invoke(e.Id);
            }
            else if (_activeStage == TacSimActiveStage.ShipHelmSeated)
            {
                OnSpaceRowSecondaryAct?.Invoke(e.Id, e.IsRock);
            }
        }

        private void RefreshFilterVisuals()
        {
            for (int i = 0; i < _filterImages.Length; i++)
            {
                if (_filterImages[i] == null) continue;
                _filterImages[i].color = i == (int)ActiveFilter
                    ? new Color(0.20f, 0.56f, 0.76f, 1f)
                    : new Color(0.10f, 0.19f, 0.28f, 0.94f);
            }
        }

        private GameObject CreatePanel(Transform parent, string name, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), aMin, aMax, oMin, oMax);
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

        private Button CreateSmallButton(Transform parent, string label, Color bg, Action onClick, out Text labelTxt)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.onClick.AddListener(() => onClick?.Invoke());
            labelTxt = CreateText(go.transform, label, 9, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(labelTxt.rectTransform, Vector2.zero, Vector2.one, new Vector2(2f, 1f), new Vector2(-2f, -1f));
            return btn;
        }

        private static string ToHexRgb(Color c)
        {
            int r = Mathf.Clamp(Mathf.RoundToInt(c.r * 255f), 0, 255);
            int g = Mathf.Clamp(Mathf.RoundToInt(c.g * 255f), 0, 255);
            int b = Mathf.Clamp(Mathf.RoundToInt(c.b * 255f), 0, 255);
            return $"{r:X2}{g:X2}{b:X2}";
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
