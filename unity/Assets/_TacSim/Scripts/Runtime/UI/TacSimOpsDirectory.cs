using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Arena;
using TacSim.Core;
using TacSim.Net;
using TacSim.Space;
using TacSim.Surface;
using TacSim.World;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum OpsDirectoryTab
    {
        DeckDirectory = 0,
        RoomPresence = 1,
        ActiveOps = 2
    }

    /// <summary>
    /// WoW / EverQuest-style `[N]` Station &amp; Ship Directory, Live Room Cast/Presence Roster,
    /// Active Ops/Orders Tracker, and Compact Top-Center 3D Wayfinder Compass Banner.
    /// Drives `TacSimWayfinder3D` holographic trails and authoritative `mushd` navigation/social verbs.
    /// </summary>
    public sealed class TacSimOpsDirectory : MonoBehaviour
    {
        public event Action<string, bool> OnPlotStationRoomRequested;    // (roomEntryId, autoWalk)
        public event Action<string, bool> OnPlotShipCompartmentRequested; // (compId, autoWalk)
        public event Action OnStepWayfinderWalkRequested;
        public event Action OnClearWayfinderRequested;
        public event Action<string, string, bool> OnPresenceTargetRequested; // (id, name, isPlayer)
        public event Action<string> OnPrefillChatRequested;
        public event Action<string> OnCommandRequested;
        public event Action<string> OnArenaActionRequested;

        private const int MaxDirectoryRows = 24;
        private const int MaxPresenceRows = 16;

        private sealed class PooledDirectoryRow
        {
            public GameObject Root;
            public Image Background;
            public Text Label;
            public Button TrailButton;
            public Text TrailButtonLabel;
            public Button WalkButton;
            public Text WalkButtonLabel;
            public string BoundId;
            public bool BoundIsShip;
            public bool BoundIsArenaSpot;
        }

        private sealed class PooledPresenceRow
        {
            public GameObject Root;
            public Image Background;
            public Text Label;
            public Button TargetButton;
            public Button LookButton;
            public Button TalkButton;
            public string BoundId;
            public string BoundName;
            public bool BoundIsPlayer;
        }

        private Font _font;

        // Top-Center Compact 3D Wayfinder Compass Banner
        private GameObject _bannerRoot;
        private Text _bannerText;

        // [N] Modal
        private GameObject _modalRoot;
        private Text _modalTitleText;
        private Text _modalMetaText;
        private readonly List<Image> _tabImages = new List<Image>();
        private readonly GameObject[] _tabBodies = new GameObject[3];

        // Tab 0: Deck Directory
        private readonly List<PooledDirectoryRow> _dirPool = new List<PooledDirectoryRow>(MaxDirectoryRows);
        private Text _dirEmptyText;

        // Tab 1: Room Cast & Presence
        private readonly List<PooledPresenceRow> _presPool = new List<PooledPresenceRow>(MaxPresenceRows);
        private Text _presEmptyText;

        // Tab 2: Active Ops & Orders
        private Text _opsSummaryText;
        private GameObject _standardOpsActionsRow;
        private GameObject _arenaOpsActionsRow;
        private Button _arenaScanOpsBtn;

        // Bound live data sources
        private StationWorldController _station;
        private ShipInteriorController _ship;
        private TacSimWayfinder3D _wayfinder;
        private TacSimTradeAndShipState _trade;
        private SpaceFlightController _space;
        private SurfaceMiningController _surface;
        private OocArenaController _arena;
        private MushRoomView _lastRoomView;
        private TacSimActiveStage _activeStage = TacSimActiveStage.StationWalk;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public bool IsWayfinderBannerVisible => _bannerRoot != null && _bannerRoot.activeSelf;
        public string WayfinderBannerText => _bannerText != null ? _bannerText.text : string.Empty;
        public OpsDirectoryTab ActiveTab { get; private set; } = OpsDirectoryTab.DeckDirectory;
        public int VisibleDirectoryRowCount { get; private set; }
        public int VisiblePresenceRowCount { get; private set; }
        public string OpsSummarySnapshot => _opsSummaryText != null ? _opsSummaryText.text : string.Empty;

        public void BuildUi(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _tabImages.Clear();
            _dirPool.Clear();
            _presPool.Clear();

            BuildWayfinderBanner(hudRoot);
            BuildDirectoryModal(hudRoot);
        }

        private void BuildWayfinderBanner(Transform parent)
        {
            // Sits cleanly top-center below TopStrip (-64..-10) and between UnitFrames (left) and Minimap (right)
            _bannerRoot = CreatePanel(
                parent,
                "WayfinderCompassBanner",
                new Vector2(0.5f, 1f),
                new Vector2(0.5f, 1f),
                new Vector2(-276f, -104f),
                new Vector2(276f, -70f),
                new Color(0.03f, 0.09f, 0.14f, 0.92f));

            CreatePanel(_bannerRoot.transform, "Accent", new Vector2(0f, 0f), new Vector2(0f, 1f), Vector2.zero, new Vector2(3f, 0f), new Color(0.24f, 0.94f, 0.96f, 0.95f));

            _bannerText = CreateText(_bannerRoot.transform, "WAYFINDER ACTIVE", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.76f, 0.96f, 1f));
            SetRect(_bannerText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(12f, 0f), new Vector2(-132f, 0f));

            var walkBtn = CreateButton(_bannerRoot.transform, "WALK", new Color(0.14f, 0.46f, 0.42f), () => OnStepWayfinderWalkRequested?.Invoke());
            SetRect(walkBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-124f, 4f), new Vector2(-46f, -4f));

            var clearBtn = CreateButton(_bannerRoot.transform, "×", new Color(0.42f, 0.16f, 0.18f), () => OnClearWayfinderRequested?.Invoke());
            SetRect(clearBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-40f, 4f), new Vector2(-6f, -4f));

            _bannerRoot.SetActive(false);
        }

        private void BuildDirectoryModal(Transform parent)
        {
            _modalRoot = CreatePanel(
                parent,
                "OpsDirectoryModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-460f, -295f),
                new Vector2(460f, 295f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            CreatePanel(_modalRoot.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.24f, 0.92f, 0.86f, 0.95f));

            _modalTitleText = CreateText(_modalRoot.transform, "NAV DIRECTORY, ROOM ROSTER & ACTIVE OPS [N]", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.42f, 0.95f, 0.92f));
            SetRect(_modalTitleText.rectTransform, new Vector2(0f, 1f), new Vector2(0.68f, 1f), new Vector2(18f, -42f), new Vector2(0f, -8f));

            _modalMetaText = CreateText(_modalRoot.transform, "CERES FREEPORT", 12, FontStyle.Bold, TextAnchor.MiddleRight, new Color(0.72f, 0.86f, 0.96f));
            SetRect(_modalMetaText.rectTransform, new Vector2(0.55f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(-130f, -8f));

            var closeBtn = CreateButton(_modalRoot.transform, "CLOSE [ESC]", new Color(0.46f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -40f), new Vector2(-14f, -10f));

            var tabStrip = CreatePanel(_modalRoot.transform, "OpsTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -82f), new Vector2(-16f, -48f), Color.clear);
            var hlg = tabStrip.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 6f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            string[] labels = { "DECK DIRECTORY (3D TRAIL)", "ROOM CAST & PRESENCE", "ACTIVE OPS & ORDERS" };
            for (int i = 0; i < labels.Length; i++)
            {
                var captured = (OpsDirectoryTab)i;
                var tb = CreateButton(tabStrip.transform, labels[i], new Color(0.12f, 0.24f, 0.36f), () => SelectTab(captured));
                _tabImages.Add(tb.GetComponent<Image>());
            }

            BuildDeckDirectoryTab(_modalRoot.transform);
            BuildPresenceTab(_modalRoot.transform);
            BuildActiveOpsTab(_modalRoot.transform);

            SelectTab(OpsDirectoryTab.DeckDirectory);
            _modalRoot.SetActive(false);
        }

        private void BuildDeckDirectoryTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabDeckDirectory", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[0] = body;

            var hint = CreateText(body.transform, "Pick any deck/compartment to project a live 3D holographic floor ribbon (`3D TRAIL`) or step there automatically (`WALK`).", 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.74f, 0.88f, 0.96f));
            SetRect(hint.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(12f, -30f), new Vector2(-12f, -4f));

            var scrollContent = CreateScrollList(body.transform, "DirectoryScroll", new Vector2(10f, 10f), new Vector2(-10f, -34f));

            _dirEmptyText = CreateText(scrollContent, "No station or ship deck map loaded.", 13, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.65f, 0.76f, 0.86f));
            _dirEmptyText.rectTransform.sizeDelta = new Vector2(0f, 34f);

            for (int i = 0; i < MaxDirectoryRows; i++)
            {
                var row = new PooledDirectoryRow();
                var go = CreatePanel(scrollContent, $"DirRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 34f), new Color(0.07f, 0.12f, 0.19f, 0.92f));
                go.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 34f);
                row.Root = go;
                row.Background = go.GetComponent<Image>();

                row.Label = CreateText(go.transform, string.Empty, 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.88f, 0.95f, 1f));
                row.Label.supportRichText = true;
                SetRect(row.Label.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 0f), new Vector2(-202f, 0f));

                row.TrailButton = CreateButton(go.transform, "3D TRAIL", new Color(0.14f, 0.36f, 0.52f), () =>
                {
                    if (string.IsNullOrEmpty(row.BoundId)) return;
                    SetOpen(false);
                    if (row.BoundIsArenaSpot)
                    {
                        if (MushProtocol.TryValidateSafeCommandToken(row.BoundId, out string safeSpot, maxLen: 16))
                            OnArenaActionRequested?.Invoke($"{ResolveArenaAllOrderPrefix()}advance {safeSpot}");
                    }
                    else if (row.BoundIsShip) OnPlotShipCompartmentRequested?.Invoke(row.BoundId, false);
                    else OnPlotStationRoomRequested?.Invoke(row.BoundId, false);
                });
                row.TrailButtonLabel = row.TrailButton.GetComponentInChildren<Text>();
                SetRect(row.TrailButton.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-194f, 4f), new Vector2(-102f, -4f));

                row.WalkButton = CreateButton(go.transform, "WALK", new Color(0.14f, 0.48f, 0.38f), () =>
                {
                    if (string.IsNullOrEmpty(row.BoundId)) return;
                    SetOpen(false);
                    if (row.BoundIsArenaSpot)
                    {
                        if (MushProtocol.TryValidateSafeCommandToken(row.BoundId, out string safeSpot, maxLen: 16))
                            OnArenaActionRequested?.Invoke($"drive {safeSpot}");
                    }
                    else if (row.BoundIsShip) OnPlotShipCompartmentRequested?.Invoke(row.BoundId, true);
                    else OnPlotStationRoomRequested?.Invoke(row.BoundId, true);
                });
                row.WalkButtonLabel = row.WalkButton.GetComponentInChildren<Text>();
                SetRect(row.WalkButton.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-96f, 4f), new Vector2(-8f, -4f));

                go.SetActive(false);
                _dirPool.Add(row);
            }
        }

        private void BuildPresenceTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabPresence", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[1] = body;

            var hdr = CreateText(body.transform, "LIVE ROOM PRESENCE — CLICK TO TARGET, EXAMINE, OR OPEN DIRECT COMMS", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.55f, 0.92f, 0.82f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(12f, -30f), new Vector2(-124f, -4f));

            var whoBtn = CreateButton(body.transform, "Who Online", new Color(0.16f, 0.34f, 0.48f), () => OnCommandRequested?.Invoke("WHO"));
            SetRect(whoBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -30f), new Vector2(-10f, -4f));

            var scrollContent = CreateScrollList(body.transform, "PresenceScroll", new Vector2(10f, 10f), new Vector2(-10f, -36f));

            _presEmptyText = CreateText(scrollContent, "No other pilots or cast members detected in this room.", 13, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.65f, 0.76f, 0.86f));
            _presEmptyText.rectTransform.sizeDelta = new Vector2(0f, 34f);

            for (int i = 0; i < MaxPresenceRows; i++)
            {
                var row = new PooledPresenceRow();
                var go = CreatePanel(scrollContent, $"PresRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 34f), new Color(0.07f, 0.12f, 0.19f, 0.92f));
                go.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 34f);
                row.Root = go;
                row.Background = go.GetComponent<Image>();

                row.Label = CreateText(go.transform, string.Empty, 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.90f, 0.96f, 1f));
                row.Label.supportRichText = true;
                SetRect(row.Label.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 0f), new Vector2(-248f, 0f));

                row.TargetButton = CreateButton(go.transform, "TARGET", new Color(0.16f, 0.40f, 0.56f), () =>
                {
                    if (!string.IsNullOrEmpty(row.BoundName))
                        OnPresenceTargetRequested?.Invoke(row.BoundId, row.BoundName, row.BoundIsPlayer);
                });
                SetRect(row.TargetButton.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-242f, 4f), new Vector2(-166f, -4f));

                row.LookButton = CreateButton(go.transform, "LOOK", new Color(0.18f, 0.34f, 0.44f), () =>
                {
                    if (!string.IsNullOrEmpty(row.BoundName))
                    {
                        SetOpen(false);
                        OnCommandRequested?.Invoke($"look {row.BoundName}");
                    }
                });
                SetRect(row.LookButton.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-160f, 4f), new Vector2(-92f, -4f));

                row.TalkButton = CreateButton(go.transform, "COMMS", new Color(0.14f, 0.48f, 0.38f), () =>
                {
                    if (!string.IsNullOrEmpty(row.BoundName))
                    {
                        SetOpen(false);
                        string prefix = row.BoundIsPlayer ? $"page {row.BoundName}=" : $"say {row.BoundName}, ";
                        OnPrefillChatRequested?.Invoke(prefix);
                    }
                });
                SetRect(row.TalkButton.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-86f, 4f), new Vector2(-8f, -4f));

                go.SetActive(false);
                _presPool.Add(row);
            }
        }

        private void BuildActiveOpsTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabActiveOps", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[2] = body;

            _opsSummaryText = CreateText(body.transform, "No active operations logged.", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 0.98f));
            _opsSummaryText.supportRichText = true;
            SetRect(_opsSummaryText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 64f), new Vector2(-16f, -14f));

            _standardOpsActionsRow = CreatePanel(body.transform, "OpsActionsRow", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(14f, 12f), new Vector2(-14f, 54f), Color.clear);
            var hlg = _standardOpsActionsRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            CreateButton(_standardOpsActionsRow.transform, "JOBS (+jobs)", new Color(0.16f, 0.42f, 0.54f), () => OnCommandRequested?.Invoke("+jobs"));
            CreateButton(_standardOpsActionsRow.transform, "Escorts (+escort)", new Color(0.24f, 0.38f, 0.54f), () => OnCommandRequested?.Invoke("+escort"));
            CreateButton(_standardOpsActionsRow.transform, "Ship Orders", new Color(0.16f, 0.46f, 0.40f), () => OnCommandRequested?.Invoke("+ship/orders"));
            CreateButton(_standardOpsActionsRow.transform, "Co-op Ledger", new Color(0.42f, 0.32f, 0.14f), () => OnCommandRequested?.Invoke("+coop/statement"));
            CreateButton(_standardOpsActionsRow.transform, "Who Online", new Color(0.22f, 0.30f, 0.44f), () => OnCommandRequested?.Invoke("WHO"));

            _arenaOpsActionsRow = CreatePanel(body.transform, "ArenaOpsActionsRow", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(14f, 12f), new Vector2(-14f, 54f), Color.clear);
            var ahlg = _arenaOpsActionsRow.AddComponent<HorizontalLayoutGroup>();
            ahlg.spacing = 8f;
            ahlg.childControlWidth = true;
            ahlg.childControlHeight = true;
            ahlg.childForceExpandWidth = true;

            _arenaScanOpsBtn = CreateButton(_arenaOpsActionsRow.transform, "SCAN SECTOR", new Color(0.14f, 0.44f, 0.52f), () =>
            {
                SetOpen(false);
                OnArenaActionRequested?.Invoke("scan");
            });
            CreateButton(_arenaOpsActionsRow.transform, "SIM STATUS", new Color(0.18f, 0.38f, 0.52f), () => OnCommandRequested?.Invoke("sim status"));
            CreateButton(_arenaOpsActionsRow.transform, "MATCH HISTORY", new Color(0.26f, 0.34f, 0.50f), () => OnCommandRequested?.Invoke("sim history"));
            CreateButton(_arenaOpsActionsRow.transform, "WHO ONLINE", new Color(0.22f, 0.30f, 0.44f), () => OnCommandRequested?.Invoke("WHO"));
            _arenaOpsActionsRow.SetActive(false);
        }

        public void BindSources(
            StationWorldController station,
            ShipInteriorController ship,
            TacSimWayfinder3D wayfinder,
            TacSimTradeAndShipState trade,
            SpaceFlightController space,
            SurfaceMiningController surface,
            OocArenaController arena)
        {
            _station = station;
            _ship = ship;
            _wayfinder = wayfinder;
            _trade = trade;
            _space = space;
            _surface = surface;
            _arena = arena;
            SyncWayfinderBanner(_activeStage);
            if (IsOpen) RefreshModalContents();
        }

        public void UpdateLiveRoom(MushRoomView roomView, TacSimActiveStage stage)
        {
            _lastRoomView = roomView;
            _activeStage = stage;
            SyncWayfinderBanner(stage);
            if (IsOpen) RefreshModalContents();
        }

        public void SyncWayfinderBanner(TacSimActiveStage stage)
        {
            _activeStage = stage;
            if (_bannerRoot == null) return;
            bool show = _wayfinder != null &&
                        _wayfinder.HasActiveRoute &&
                        stage != TacSimActiveStage.SimulatorArena &&
                        ((_wayfinder.Scope == WayfinderScope.Station && stage == TacSimActiveStage.StationWalk) ||
                         (_wayfinder.Scope == WayfinderScope.Ship && stage == TacSimActiveStage.ShipInteriorWalk));
            _bannerRoot.SetActive(show);
            if (show && _bannerText != null)
            {
                int hops = Mathf.Max(1, _wayfinder.RemainingHops);
                _bannerText.text =
                    $"→ {_wayfinder.TargetDisplayName.ToUpperInvariant()} [{_wayfinder.TargetDeckLabel}] · {_wayfinder.DistanceMeters:0} m · {hops} HOP{(hops == 1 ? "" : "S")}";
            }
        }

        public void SetOpen(bool open)
        {
            if (_modalRoot == null) return;
            _modalRoot.SetActive(open);
            if (open) RefreshModalContents();
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void SelectTab(OpsDirectoryTab tab)
        {
            ActiveTab = tab;
            int idx = (int)tab;
            for (int i = 0; i < _tabBodies.Length; i++)
            {
                if (_tabBodies[i] != null)
                    _tabBodies[i].SetActive(i == idx);
            }
            for (int i = 0; i < _tabImages.Count; i++)
            {
                _tabImages[i].color = (i == idx)
                    ? new Color(0.18f, 0.56f, 0.74f, 1f)
                    : new Color(0.11f, 0.20f, 0.30f, 0.95f);
            }
            RefreshModalContents();
        }

        public void ResetSessionState()
        {
            _lastRoomView = null;
            _activeStage = TacSimActiveStage.StationWalk;
            if (_bannerRoot != null) _bannerRoot.SetActive(false);
            SetOpen(false);
        }

        public void RefreshModalContents()
        {
            RefreshDirectoryTab();
            RefreshPresenceTab();
            RefreshActiveOpsTab();
        }

        public string GetDirectoryRowLabel(int index) =>
            (index >= 0 && index < VisibleDirectoryRowCount && index < _dirPool.Count && _dirPool[index].Label != null)
                ? (_dirPool[index].Label.text ?? string.Empty)
                : string.Empty;

        public bool TriggerDirectoryRowAction(int index, bool secondary = false)
        {
            if (index < 0 || index >= VisibleDirectoryRowCount || index >= _dirPool.Count)
                return false;
            var row = _dirPool[index];
            var btn = secondary ? row.TrailButton : row.WalkButton;
            if (btn == null || !btn.interactable || !row.Root.activeSelf)
                return false;
            btn.onClick.Invoke();
            return true;
        }

        private string ResolveArenaAllOrderPrefix()
        {
            var det = _arena?.State?.Detachment;
            if (det != null && !_arena.State.DetachmentProjectionInvalid && !string.IsNullOrEmpty(det.AllOrderPrefill))
                return det.AllOrderPrefill;
            return "order all ";
        }

        private void RefreshDirectoryTab()
        {
            bool inArena = _activeStage == TacSimActiveStage.SimulatorArena && _arena != null && _arena.TerrainGrid != null;
            bool aboardShip = _activeStage == TacSimActiveStage.ShipInteriorWalk || _activeStage == TacSimActiveStage.ShipHelmSeated;
            int used = 0;

            if (inArena)
            {
                string arenaName = !string.IsNullOrEmpty(_arena.TerrainGrid.DisplayName)
                    ? _arena.TerrainGrid.DisplayName
                    : (!string.IsNullOrEmpty(_arena.TerrainGrid.Id) ? _arena.TerrainGrid.Id : "BATTLEFIELD");
                if (_modalMetaText != null) _modalMetaText.text = $"ARENA SECTOR: {arenaName.ToUpperInvariant()}";

                var spots = _arena.TerrainGrid.KeySpots;
                if (spots != null)
                {
                    string activeHold = _arena.State != null && _arena.State.HasDynamicObjective && !string.IsNullOrEmpty(_arena.State.ObjectiveLetter)
                        ? _arena.State.ObjectiveLetter
                        : (_arena.TerrainGrid.HoldSpotLetter ?? string.Empty);
                    Vector3 ownLocal = _arena.PlayerVehicleTransform != null
                        ? _arena.PlayerVehicleTransform.position - OocArenaController.ArenaStageOrigin
                        : (_arena.State != null ? _arena.ArenaWorldMetersToLocalPos(_arena.State.Xm, _arena.State.Zm) : Vector3.zero);

                    for (int i = 0; i < spots.Count && used < MaxDirectoryRows; i++)
                    {
                        var sp = spots[i];
                        if (sp == null || string.IsNullOrWhiteSpace(sp.Letter)) continue;
                        bool isObj = !string.IsNullOrEmpty(activeHold) &&
                                     string.Equals(sp.Letter, activeHold, StringComparison.OrdinalIgnoreCase);
                        Vector3 spotLocal = _arena.TerrainGrid.SiteCellToLocalPos(sp.CellX, sp.CellY, 0f);
                        float distM = Vector2.Distance(new Vector2(ownLocal.x, ownLocal.z), new Vector2(spotLocal.x, spotLocal.z));

                        var row = _dirPool[used++];
                        row.BoundId = sp.Letter.Trim();
                        row.BoundIsShip = false;
                        row.BoundIsArenaSpot = true;
                        string objTag = isObj ? " <color=#FFDD55>[OBJECTIVE]</color>" : string.Empty;
                        string spotTitle = string.IsNullOrWhiteSpace(sp.Name) ? $"Sector {row.BoundId}" : sp.Name;
                        row.Label.text = $"<color=#66EEFF>[{MushProtocol.EscapeUnityRichText(row.BoundId)}]</color> <b>{MushProtocol.EscapeUnityRichText(spotTitle)}</b>{objTag}   <color=#99CCEE>({sp.CellX},{sp.CellY}) · {distM:0}m</color>";
                        row.Background.color = isObj ? new Color(0.24f, 0.20f, 0.08f, 0.96f) : new Color(0.07f, 0.12f, 0.19f, 0.92f);
                        if (row.TrailButtonLabel != null) row.TrailButtonLabel.text = "SQUAD ADV";
                        if (row.WalkButtonLabel != null) row.WalkButtonLabel.text = "DRIVE";
                        row.TrailButton.interactable = true;
                        row.WalkButton.interactable = true;
                        row.Root.SetActive(true);
                    }
                }
            }
            else if (aboardShip && _ship?.Template != null)
            {
                if (_modalMetaText != null) _modalMetaText.text = $"SHIP DECKS ({_ship.LoadedUnitId.ToUpperInvariant()})";
                var comps = new List<ShipCompartmentDef>(_ship.Template.Compartments.Values);
                comps.Sort((a, b) => string.Compare(a.Id, b.Id, StringComparison.OrdinalIgnoreCase));

                for (int i = 0; i < comps.Count && used < MaxDirectoryRows; i++)
                {
                    var c = comps[i];
                    bool isHere = string.Equals(c.Id, _ship.ActiveCompartmentId, StringComparison.OrdinalIgnoreCase);
                    int hops = isHere ? 0 : _ship.FindExitPath(_ship.ActiveCompartmentId, c.Id).Count;
                    var row = _dirPool[used++];
                    row.BoundId = c.Id;
                    row.BoundIsShip = true;
                    row.BoundIsArenaSpot = false;
                    string tag = isHere ? "<color=#66FFAA>[HERE]</color>" : $"<color=#88CCFF>{hops} hop{(hops == 1 ? "" : "s")}</color>";
                    row.Label.text = $"[DECK] <b>{MushProtocol.EscapeUnityRichText(string.IsNullOrEmpty(c.Name) ? c.Id.ToUpperInvariant() : c.Name)}</b>   {tag}";
                    row.Background.color = isHere ? new Color(0.10f, 0.24f, 0.22f, 0.95f) : new Color(0.07f, 0.12f, 0.19f, 0.92f);
                    if (row.TrailButtonLabel != null) row.TrailButtonLabel.text = "3D TRAIL";
                    if (row.WalkButtonLabel != null) row.WalkButtonLabel.text = "WALK";
                    row.TrailButton.interactable = !isHere && hops > 0;
                    row.WalkButton.interactable = !isHere && hops > 0;
                    row.Root.SetActive(true);
                }
            }
            else if (_station?.Pack != null)
            {
                if (_modalMetaText != null) _modalMetaText.text = (_station.Pack.Name ?? "STATION").ToUpperInvariant();
                var rooms = new List<AtlasRoomDef>(_station.Pack.Rooms.Values);
                rooms.Sort((a, b) =>
                {
                    int f = a.FloorIndex.CompareTo(b.FloorIndex);
                    return f != 0 ? f : string.Compare(a.Name, b.Name, StringComparison.OrdinalIgnoreCase);
                });

                for (int i = 0; i < rooms.Count && used < MaxDirectoryRows; i++)
                {
                    var r = rooms[i];
                    bool isHere = string.Equals(r.EntryId, _station.ActiveRoomEntryId, StringComparison.OrdinalIgnoreCase);
                    int hops = isHere ? 0 : _station.FindExitPath(_station.ActiveRoomEntryId, r.EntryId).Count;
                    if (!isHere && hops == 0) continue; // Hide unreachable restricted rooms

                    var row = _dirPool[used++];
                    row.BoundId = r.EntryId;
                    row.BoundIsShip = false;
                    row.BoundIsArenaSpot = false;
                    string roleBadge = ClassifyRoomBadge(r.EntryId, r.Kind);
                    string hopStr = isHere ? "<color=#66FFAA>[HERE]</color>" : $"<color=#88CCFF>{hops} hop{(hops == 1 ? "" : "s")}</color>";
                    row.Label.text = $"<color=#FFCC66>[FL {r.FloorIndex + 1}]</color> <b>{MushProtocol.EscapeUnityRichText(r.Name ?? r.EntryId)}</b> {roleBadge}   {hopStr}";
                    row.Background.color = isHere ? new Color(0.10f, 0.24f, 0.22f, 0.95f) : new Color(0.07f, 0.12f, 0.19f, 0.92f);
                    if (row.TrailButtonLabel != null) row.TrailButtonLabel.text = "3D TRAIL";
                    if (row.WalkButtonLabel != null) row.WalkButtonLabel.text = "WALK";
                    row.TrailButton.interactable = !isHere;
                    row.WalkButton.interactable = !isHere;
                    row.Root.SetActive(true);
                }
            }

            VisibleDirectoryRowCount = used;
            for (int i = used; i < _dirPool.Count; i++)
                _dirPool[i].Root.SetActive(false);
            if (_dirEmptyText != null) _dirEmptyText.gameObject.SetActive(used == 0);
        }

        private void RefreshPresenceTab()
        {
            int used = 0;
            if (_lastRoomView != null && _lastRoomView.Contents != null)
            {
                for (int i = 0; i < _lastRoomView.Contents.Count && used < MaxPresenceRows; i++)
                {
                    var obj = _lastRoomView.Contents[i];
                    if (obj == null || string.IsNullOrWhiteSpace(obj.Name)) continue;

                    bool isPlayer = !string.IsNullOrEmpty(obj.Id) && _lastRoomView.PlayerIds.Contains(obj.Id);
                    bool isNpc = !string.IsNullOrEmpty(obj.Id) && _lastRoomView.NpcIds.Contains(obj.Id);
                    bool isDormant = !string.IsNullOrEmpty(obj.Id) && _lastRoomView.DormantNpcIds.Contains(obj.Id);
                    if (!isPlayer && !isNpc && !isDormant) continue;

                    var row = _presPool[used++];
                    row.BoundId = obj.Id ?? string.Empty;
                    row.BoundName = obj.Name.Trim();
                    row.BoundIsPlayer = isPlayer;

                    string badge = isPlayer
                        ? "<color=#66FFAA>[PILOT]</color>"
                        : (isDormant ? "<color=#99AABB>[CAST · IDLE]</color>" : "<color=#66CCFF>[NPC CAST]</color>");
                    row.Label.text = $"{badge} <b>{MushProtocol.EscapeUnityRichText(row.BoundName)}</b>";
                    row.Background.color = isPlayer ? new Color(0.08f, 0.18f, 0.16f, 0.94f) : new Color(0.07f, 0.12f, 0.19f, 0.92f);
                    row.Root.SetActive(true);
                }
            }

            VisiblePresenceRowCount = used;
            for (int i = used; i < _presPool.Count; i++)
                _presPool[i].Root.SetActive(false);
            if (_presEmptyText != null) _presEmptyText.gameObject.SetActive(used == 0);
        }

        private void RefreshActiveOpsTab()
        {
            if (_opsSummaryText == null) return;
            var sb = new StringBuilder(640);

            bool inArena = _arena != null && _arena.IsArenaActive && _arena.State != null;
            bool inSimLobby = _lastRoomView != null &&
                              _lastRoomView.Atlas != null &&
                              string.Equals(_lastRoomView.Atlas.Entry, "room.simulator_lobby", StringComparison.OrdinalIgnoreCase);

            if (_standardOpsActionsRow != null) _standardOpsActionsRow.SetActive(!inArena && !inSimLobby);
            if (_arenaOpsActionsRow != null) _arenaOpsActionsRow.SetActive(inArena || inSimLobby);
            if (_arenaScanOpsBtn != null) _arenaScanOpsBtn.gameObject.SetActive(inArena);

            if (inArena)
            {
                var st = _arena.State;
                var ti = _arena.CurrentTerrainIntel;
                int shots = st.MatchHits + st.MatchMisses;
                int accPct = shots > 0 ? Mathf.RoundToInt((st.MatchHits * 100f) / shots) : 0;
                string lockDesc = !string.IsNullOrEmpty(st.LockedDesignator)
                    ? $"LOCK {MushProtocol.EscapeUnityRichText(st.LockedDesignator)} · {st.LockRangeM:0}m · {st.LockHitPercent}%"
                    : "NO LOCK";
                string coverDesc = !string.IsNullOrEmpty(ti.CoverLabel) ? ti.CoverLabel : "OPEN";
                string losDesc = ti.HasTargetSample && ti.IsLineOfSightObstructed && !string.IsNullOrEmpty(ti.LineOfSightStatus)
                    ? $" · {ti.LineOfSightStatus} LOS"
                    : (ti.HasTargetSample ? " · CLEAR LOS" : string.Empty);

                sb.AppendLine("<color=#FF7766><b>ACTIVE ARENA BATTLEFIELD INTEL</b></color>");
                sb.AppendFormat("• MATCH #{0} — <b>{1}</b> ({2})  |  RED {3} vs BLUE {4}\n",
                    st.MatchId,
                    MushProtocol.EscapeUnityRichText(st.Callsign ?? "R1"),
                    MushProtocol.EscapeUnityRichText((st.Mode ?? "skirmish").ToUpperInvariant()),
                    st.RedAliveCount,
                    st.BlueAliveCount);
                sb.AppendFormat("• Combat: <b>{0}</b> kills · <b>{1}/{2}</b> hits ({3}%)  |  {4}\n",
                    st.MatchKills, st.MatchHits, shots, accPct, lockDesc);
                sb.AppendFormat("• Terrain &amp; Sensors: Cover <b>{0}</b>{1} · Sight {2:0}m · Scope {3}\n",
                    MushProtocol.EscapeUnityRichText(coverDesc),
                    MushProtocol.EscapeUnityRichText(losDesc),
                    st.SightMeters,
                    MushProtocol.EscapeUnityRichText((st.Scope ?? "clear").ToUpperInvariant()));

                if (st.StandingOrder != null)
                {
                    string ordTarget = !string.IsNullOrEmpty(st.StandingOrder.Target)
                        ? $" -> {MushProtocol.EscapeUnityRichText(st.StandingOrder.Target)}"
                        : (st.StandingOrder.HasToCell ? $" -> ({st.StandingOrder.ToCellX},{st.StandingOrder.ToCellY})" : string.Empty);
                    sb.AppendFormat("• Standing Order: <b>{0}</b> {1}{2}\n",
                        MushProtocol.EscapeUnityRichText((st.StandingOrder.Squad ?? "UNIT").ToUpperInvariant()),
                        MushProtocol.EscapeUnityRichText((st.StandingOrder.Objective ?? "HOLD").ToUpperInvariant()),
                        ordTarget);
                }
                if (st.Mind != null)
                {
                    string mindTitle = string.IsNullOrWhiteSpace(st.Mind.Name) ? "OWN MIND" : st.Mind.Name;
                    string prof = string.IsNullOrWhiteSpace(st.Mind.ActiveProfileName) ? "DEFAULT" : st.Mind.ActiveProfileName;
                    sb.AppendFormat("• Mind Doctrine: <b>{0}</b> (Profile: <b>{1}</b> · Rules: {2})\n",
                        MushProtocol.EscapeUnityRichText(mindTitle),
                        MushProtocol.EscapeUnityRichText(prof),
                        st.Rules.Count);
                }
                sb.AppendLine();
            }
            else if (inSimLobby)
            {
                sb.AppendLine("<color=#FFCC66><b>OOC SIMULATOR POD BAY</b></color>");
                sb.AppendLine("• Use the Sim Console (`sim status`, `sim history`) or step into a pod (`board`) to deploy.\n");
            }

            sb.AppendLine("<color=#66EEFF><b>ACTIVE WAYFINDER TRAIL</b></color>");
            if (_wayfinder != null && _wayfinder.HasActiveRoute)
            {
                sb.AppendFormat("• Destination: <b>{0}</b> [{1}] — {2:0} m ({3} hop{4} remaining)\n\n",
                    MushProtocol.EscapeUnityRichText(_wayfinder.TargetDisplayName),
                    MushProtocol.EscapeUnityRichText(_wayfinder.TargetDeckLabel),
                    _wayfinder.DistanceMeters,
                    _wayfinder.RemainingHops,
                    _wayfinder.RemainingHops == 1 ? "" : "s");
            }
            else
            {
                sb.AppendLine("• No active 3D wayfinder trail. Use the Deck Directory tab or click any room on the minimap.\n");
            }

            sb.AppendLine("<color=#FFCC66><b>SAILING PLAN &amp; CORRIDOR ORDERS</b></color>");
            if (_trade != null && _trade.SailingOrderLines.Count > 0)
            {
                int n = Mathf.Min(_trade.SailingOrderLines.Count, 6);
                for (int i = 0; i < n; i++)
                    sb.AppendLine("• " + MushProtocol.EscapeUnityRichText(_trade.SailingOrderLines[i]));
            }
            else
            {
                sb.AppendLine("• No active sailing orders. Open Solar Chart [M] or Sailing Planner to plot a corridor leg.");
            }

            sb.AppendLine("\n<color=#66FFAA><b>TACTICAL &amp; FIELD TELEMETRY</b></color>");
            if (_space != null && (_space.ActiveLock != null || !string.IsNullOrEmpty(_space.SelectedRockId)))
            {
                string lockDesc = _space.ActiveLock != null
                    ? $"Lock: {_space.ActiveLock.Label} ({_space.ActiveLock.RangeM / 1000.0:0.1} km)"
                    : $"Rock {_space.SelectedRockId}";
                sb.AppendLine("• Orbital Helm: " + MushProtocol.EscapeUnityRichText(lockDesc));
            }
            if (_surface != null && _surface.IsExcursionActive)
                sb.AppendFormat("• Surface Excursion: {0} — Rig #{1} ({2}/{3} t hopper, {4}% battery)\n",
                    MushProtocol.EscapeUnityRichText(_surface.ActiveSiteTitle),
                    _surface.AddressedRigSlot,
                    _surface.HopperLoadedTonnes,
                    _surface.HopperCapacityTonnes,
                    _surface.BatteryPct);
            if (!inArena && _arena != null && _arena.IsSpectatorTheaterActive)
                sb.AppendFormat("• Spectator Replay / Observer: Match #{0} active\n", _arena.ActiveSpectatorMatchId);

            _opsSummaryText.text = sb.ToString();
        }

        private static string ClassifyRoomBadge(string entryId, string kind)
        {
            string s = ((entryId ?? string.Empty) + " " + (kind ?? string.Empty)).ToLowerInvariant();
            if (s.Contains("exchange") || s.Contains("market") || s.Contains("chandl")) return "<color=#66FFAA>[TRADE]</color>";
            if (s.Contains("berth") || s.Contains("dock") || s.Contains("airlock")) return "<color=#66CCFF>[BERTH]</color>";
            if (s.Contains("sim")) return "<color=#FF7766>[ARENA]</color>";
            if (s.Contains("yard") || s.Contains("works") || s.Contains("foundry")) return "<color=#FFBB55>[YARD]</color>";
            if (s.Contains("arrival") || s.Contains("customs")) return "<color=#AADDFF>[HUB]</color>";
            return string.Empty;
        }

        private Transform CreateScrollList(Transform parent, string name, Vector2 offMin, Vector2 offMax)
        {
            var vp = CreatePanel(parent, name, Vector2.zero, Vector2.one, offMin, offMax, new Color(0.02f, 0.05f, 0.09f, 0.85f));
            vp.AddComponent<RectMask2D>();
            var scroll = vp.AddComponent<ScrollRect>();

            var content = new GameObject("Content", typeof(RectTransform), typeof(VerticalLayoutGroup), typeof(ContentSizeFitter));
            content.transform.SetParent(vp.transform, false);
            var crt = content.GetComponent<RectTransform>();
            crt.anchorMin = new Vector2(0f, 1f);
            crt.anchorMax = new Vector2(1f, 1f);
            crt.pivot = new Vector2(0.5f, 1f);
            crt.offsetMin = Vector2.zero;
            crt.offsetMax = Vector2.zero;

            var vlg = content.GetComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            var csf = content.GetComponent<ContentSizeFitter>();
            csf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            csf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;

            scroll.viewport = vp.GetComponent<RectTransform>();
            scroll.content = crt;
            scroll.horizontal = false;
            scroll.vertical = true;
            scroll.movementType = ScrollRect.MovementType.Clamped;
            scroll.scrollSensitivity = 28f;
            return content.transform;
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

        private Button CreateButton(Transform parent, string label, Color bg, Action onClick)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;

            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.colors = new ColorBlock
            {
                normalColor = Color.white,
                highlightedColor = new Color(1.18f, 1.18f, 1.18f, 1f),
                pressedColor = new Color(0.82f, 0.82f, 0.82f, 1f),
                selectedColor = Color.white,
                disabledColor = new Color(0.45f, 0.45f, 0.45f, 0.55f),
                colorMultiplier = 1f,
                fadeDuration = 0.06f
            };
            btn.onClick.AddListener(() => onClick?.Invoke());

            var txt = CreateText(go.transform, label, 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
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
