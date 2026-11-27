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
    public enum CharacterSheetTab
    {
        Pilot = 0,
        ShipCargo = 1,
        SpaceFit = 2,
        GroundArena = 3,
        RulesetAndUnit = 4
    }

    /// <summary>
    /// WoW / EverQuest / EVE-style `[C]` Character, Ship, Stowage, Weapon Fit, Ruleset Sheet,
    /// Vitals &amp; Tactical Unit Chassis inspection sheet. Supports live 3D Ida Faber avatar swapping
    /// and authoritative `mushd` refresh queries (`Query::Sheet`, `*.vitals`, `*.unit`).
    /// </summary>
    public sealed class TacSimCharacterSheet : MonoBehaviour
    {
        public event Action<PlayerAvatarPreset> OnAvatarPresetSelected;
        public event Action<string> OnCommandRequested;
        public event Action<string> OnArenaActionRequested;
        public event Action OnOpenTradeTerminalRequested;
        public event Action OnOpenSailingPlanRequested;
        public event Action OnOpenSolarChartRequested;
        public event Action OnOpenQuestJournalRequested;
        public event Action OnRefreshSheetRequested;
        public event Action OnRefreshUnitAndVitalsRequested;

        private Font _font;
        private GameObject _modalRoot;
        private Text _titleText;
        private Text _subtitleText;
        private readonly List<Image> _tabImages = new List<Image>();
        private readonly GameObject[] _tabBodies = new GameObject[5];

        // Pilot tab
        private Text _pilotSummaryText;
        private readonly List<Image> _avatarButtons = new List<Image>();
        private PlayerAvatarPreset _selectedAvatar = PlayerAvatarPreset.StellarTrooperFemale;

        // Ship & Cargo tab
        private Text _shipIdentityText;
        private Image _massFill;
        private Text _massLabel;
        private Image _volFill;
        private Text _volLabel;
        private Image _tankFill;
        private Text _tankLabel;
        private Image _storesFill;
        private Text _storesLabel;
        private Text _shipOrdersText;

        // Space Fit tab
        private Text _spaceFitHeaderText;
        private Text _spaceMountsText;

        // Ground & Arena tab
        private Text _surfaceRigText;
        private Text _arenaRigText;
        private GameObject _standardGroundActionsRow;
        private GameObject _arenaRigActionsRow;

        // Ruleset Sheet, Vitals & Tactical Unit Chassis tab (`*.sheet`, `*.vitals`, `*.unit`)
        private Text _rulesetSheetText;
        private Text _unitChassisText;
        public LiveSheetDocument LiveSheet { get; private set; }
        public LiveVitalsStore LiveVitals { get; } = new LiveVitalsStore();
        public LiveUnitChassisView LiveUnitChassis { get; private set; }

        // Bound live data sources
        private TacSimTradeAndShipState _trade;
        private SpaceFlightController _space;
        private SurfaceMiningController _surface;
        private OocArenaController _arena;
        private TacSimQuestJournal _questJournal;

        private string _pilotName = "Unattached Pilot";
        private string _locationName = "Ceres Freeport";
        private string _hullBadge = "NO CHARTER";
        private TacSimActiveStage _activeStage = TacSimActiveStage.StationWalk;
        private float _nextAutoRefreshAt;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public CharacterSheetTab ActiveTab { get; private set; } = CharacterSheetTab.Pilot;
        public PlayerAvatarPreset SelectedAvatar => _selectedAvatar;
        public string PilotSummarySnapshot => _pilotSummaryText != null ? _pilotSummaryText.text : string.Empty;
        public string SpaceMountsSnapshot => _spaceMountsText != null ? _spaceMountsText.text : string.Empty;
        public string ArenaRigDossierSnapshot => _arenaRigText != null ? _arenaRigText.text : string.Empty;
        public string RulesetSheetSnapshot => _rulesetSheetText != null ? _rulesetSheetText.text : string.Empty;
        public string UnitChassisSnapshot => _unitChassisText != null ? _unitChassisText.text : string.Empty;

        public void BuildModal(Transform parent, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _tabImages.Clear();
            _avatarButtons.Clear();

            _modalRoot = CreatePanel(
                parent,
                "CharacterShipSheetModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-460f, -295f),
                new Vector2(460f, 295f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            // Top border accent
            CreatePanel(
                _modalRoot.transform,
                "TopAccent",
                new Vector2(0f, 1f),
                new Vector2(1f, 1f),
                new Vector2(0f, -3f),
                Vector2.zero,
                new Color(0.28f, 0.82f, 0.98f, 0.95f));

            _titleText = CreateText(_modalRoot.transform, "PILOT & VESSEL DOSSIER [C]", 19, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.92f, 1f));
            SetRect(_titleText.rectTransform, new Vector2(0f, 1f), new Vector2(0.65f, 1f), new Vector2(20f, -42f), new Vector2(0f, -8f));

            _subtitleText = CreateText(_modalRoot.transform, "STAGE: STATION WALK", 12, FontStyle.Bold, TextAnchor.MiddleRight, new Color(0.72f, 0.86f, 0.95f));
            SetRect(_subtitleText.rectTransform, new Vector2(0.50f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(-132f, -8f));

            var closeBtn = CreateButton(_modalRoot.transform, "CLOSE [ESC]", new Color(0.46f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-120f, -40f), new Vector2(-14f, -10f));

            // Mode Tabs
            var tabStrip = CreatePanel(_modalRoot.transform, "SheetTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -82f), new Vector2(-16f, -48f), Color.clear);
            var tabHlg = tabStrip.AddComponent<HorizontalLayoutGroup>();
            tabHlg.spacing = 6f;
            tabHlg.childControlWidth = true;
            tabHlg.childControlHeight = true;
            tabHlg.childForceExpandWidth = true;

            string[] tabLabels = { "PILOT & AVATAR", "SHIP & CARGO", "SPACE TACTICAL FIT", "GROUND & ARENA RIGS", "SHEET & CHASSIS" };
            for (int i = 0; i < tabLabels.Length; i++)
            {
                var captured = (CharacterSheetTab)i;
                var tb = CreateButton(tabStrip.transform, tabLabels[i], new Color(0.12f, 0.24f, 0.36f), () => SelectTab(captured));
                _tabImages.Add(tb.GetComponent<Image>());
            }

            BuildPilotTab(_modalRoot.transform);
            BuildShipCargoTab(_modalRoot.transform);
            BuildSpaceFitTab(_modalRoot.transform);
            BuildGroundArenaTab(_modalRoot.transform);
            BuildRulesetAndUnitTab(_modalRoot.transform);

            SelectTab(CharacterSheetTab.Pilot);
            _modalRoot.SetActive(false);
        }

        private void BuildPilotTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabPilot", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[0] = body;

            _pilotSummaryText = CreateText(body.transform, "Loading pilot telemetry...", 14, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 0.99f));
            _pilotSummaryText.supportRichText = true;
            SetRect(_pilotSummaryText.rectTransform, new Vector2(0f, 0.48f), new Vector2(1f, 1f), new Vector2(16f, 6f), new Vector2(-16f, -14f));

            var avHdr = CreateText(body.transform, "LIVE 3D AVATAR RIG (SWAPS INSTANTLY IN STATION / SHIP)", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.55f, 0.90f, 1f));
            SetRect(avHdr.rectTransform, new Vector2(0f, 0.38f), new Vector2(1f, 0.46f), new Vector2(16f, 0f), new Vector2(-16f, 0f));

            var avRow = CreatePanel(body.transform, "AvatarPresetRow", new Vector2(0f, 0.22f), new Vector2(1f, 0.36f), new Vector2(16f, 0f), new Vector2(-16f, 0f), Color.clear);
            var avHlg = avRow.AddComponent<HorizontalLayoutGroup>();
            avHlg.spacing = 8f;
            avHlg.childControlWidth = true;
            avHlg.childControlHeight = true;
            avHlg.childForceExpandWidth = true;

            string[] avLabels = { "Stellar Trooper (M)", "Stellar Trooper (F)", "Street Fighter (M)", "Street Fighter (F)" };
            for (int i = 0; i < avLabels.Length; i++)
            {
                var preset = (PlayerAvatarPreset)i;
                var btn = CreateButton(avRow.transform, avLabels[i], new Color(0.14f, 0.25f, 0.36f), () => ApplyAvatarChoice(preset, notify: true));
                _avatarButtons.Add(btn.GetComponent<Image>());
            }

            var actRow = CreatePanel(body.transform, "PilotActionsRow", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(16f, 14f), new Vector2(-16f, 56f), Color.clear);
            var actHlg = actRow.AddComponent<HorizontalLayoutGroup>();
            actHlg.spacing = 8f;
            actHlg.childControlWidth = true;
            actHlg.childControlHeight = true;
            actHlg.childForceExpandWidth = true;

            CreateButton(actRow.transform, "CREDITS", new Color(0.16f, 0.44f, 0.38f), () => OnCommandRequested?.Invoke("+credit"));
            CreateButton(actRow.transform, "CO-OP LEDGER", new Color(0.18f, 0.38f, 0.54f), () => OnCommandRequested?.Invoke("+coop/statement"));
            CreateButton(actRow.transform, "PORT EXCHANGE", new Color(0.20f, 0.46f, 0.52f), () =>
            {
                SetOpen(false);
                OnOpenTradeTerminalRequested?.Invoke();
            });
            CreateButton(actRow.transform, "SOLAR CHART", new Color(0.18f, 0.34f, 0.58f), () =>
            {
                SetOpen(false);
                OnOpenSolarChartRequested?.Invoke();
            });
            CreateButton(actRow.transform, "QUESTS [J]", new Color(0.56f, 0.40f, 0.12f), () =>
            {
                SetOpen(false);
                OnOpenQuestJournalRequested?.Invoke();
            });
            CreateButton(actRow.transform, "STEP OOC (+ooc)", new Color(0.32f, 0.24f, 0.46f), () =>
            {
                SetOpen(false);
                OnCommandRequested?.Invoke("+ooc");
            });
            CreateButton(actRow.transform, "RETURN IC (+ic)", new Color(0.18f, 0.46f, 0.32f), () =>
            {
                SetOpen(false);
                OnCommandRequested?.Invoke("+ic");
            });
        }

        private void BuildShipCargoTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabShipCargo", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[1] = body;

            _shipIdentityText = CreateText(body.transform, "HULL: UNCHARTERED", 14, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 1f));
            _shipIdentityText.supportRichText = true;
            SetRect(_shipIdentityText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -64f), new Vector2(-16f, -12f));

            var gauges = CreatePanel(body.transform, "ShipSheetGauges", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(14f, -134f), new Vector2(-14f, -70f), new Color(0.05f, 0.10f, 0.16f, 0.95f));
            BuildGaugeCell(gauges.transform, 0, "HOLD MASS", "0 / 100 t", new Color(0.22f, 0.72f, 0.95f), out _massFill, out _massLabel);
            BuildGaugeCell(gauges.transform, 1, "PACKED VOL", "0.0 / 120 m³", new Color(0.32f, 0.85f, 0.65f), out _volFill, out _volLabel);
            BuildGaugeCell(gauges.transform, 2, "REACTION TANK", "0 / 80 t", new Color(0.92f, 0.58f, 0.18f), out _tankFill, out _tankLabel);
            BuildGaugeCell(gauges.transform, 3, "CREW STORES", "0 / 120 d", new Color(0.68f, 0.55f, 0.95f), out _storesFill, out _storesLabel);

            _shipOrdersText = CreateText(body.transform, "No queued sailing orders.", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.80f, 0.90f, 0.96f));
            _shipOrdersText.supportRichText = true;
            SetRect(_shipOrdersText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 66f), new Vector2(-16f, -144f));

            var actRow = CreatePanel(body.transform, "ShipActionsRow", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(16f, 14f), new Vector2(-16f, 56f), Color.clear);
            var hlg = actRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            CreateButton(actRow.transform, "SHIP STATUS (+ship)", new Color(0.16f, 0.42f, 0.54f), () => OnCommandRequested?.Invoke("+ship"));
            CreateButton(actRow.transform, "CARGO HOLD (+ship/hold)", new Color(0.18f, 0.44f, 0.42f), () => OnCommandRequested?.Invoke("+ship/hold"));
            CreateButton(actRow.transform, "SAILING ORDERS", new Color(0.46f, 0.32f, 0.14f), () => OnCommandRequested?.Invoke("+ship/orders"));
            CreateButton(actRow.transform, "SAILING PLANNER", new Color(0.18f, 0.36f, 0.58f), () =>
            {
                SetOpen(false);
                OnOpenSailingPlanRequested?.Invoke();
            });
        }

        private void BuildSpaceFitTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabSpaceFit", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[2] = body;

            _spaceFitHeaderText = CreateText(body.transform, "ORBITAL FIRE CONTROL & HELM TELEMETRY", 14, FontStyle.Bold, TextAnchor.UpperLeft, new Color(1f, 0.82f, 0.44f));
            _spaceFitHeaderText.supportRichText = true;
            SetRect(_spaceFitHeaderText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -72f), new Vector2(-16f, -12f));

            _spaceMountsText = CreateText(body.transform, "Seat at the Bridge Helm in space to inspect live mount readiness, magazines, and lock solutions.", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.84f, 0.92f, 0.98f));
            _spaceMountsText.supportRichText = true;
            SetRect(_spaceMountsText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 66f), new Vector2(-16f, -80f));

            var actRow = CreatePanel(body.transform, "SpaceFitActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(16f, 14f), new Vector2(-16f, 56f), Color.clear);
            var hlg = actRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            CreateButton(actRow.transform, "TAC SCAN (+tac/scan)", new Color(0.16f, 0.46f, 0.40f), () => OnCommandRequested?.Invoke("+tac/scan"));
            CreateButton(actRow.transform, "EMCON GHOST", new Color(0.18f, 0.28f, 0.46f), () => OnCommandRequested?.Invoke("+helm/emcon ghost"));
            CreateButton(actRow.transform, "EMCON STANDARD", new Color(0.16f, 0.36f, 0.42f), () => OnCommandRequested?.Invoke("+helm/emcon standard"));
            CreateButton(actRow.transform, "SOLAR CHART [M]", new Color(0.22f, 0.34f, 0.56f), () =>
            {
                SetOpen(false);
                OnOpenSolarChartRequested?.Invoke();
            });
        }

        private void BuildGroundArenaTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabGroundArena", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[3] = body;

            var leftCard = CreatePanel(body.transform, "SurfaceRigCard", new Vector2(0f, 0f), new Vector2(0.49f, 1f), new Vector2(12f, 66f), new Vector2(-6f, -12f), new Color(0.05f, 0.10f, 0.15f, 0.92f));
            _surfaceRigText = CreateText(leftCard.transform, "SURFACE ROVER STATUS", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 0.96f));
            _surfaceRigText.supportRichText = true;
            SetRect(_surfaceRigText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));

            var rightCard = CreatePanel(body.transform, "ArenaRigCard", new Vector2(0.51f, 0f), new Vector2(1f, 1f), new Vector2(6f, 66f), new Vector2(-12f, -12f), new Color(0.06f, 0.09f, 0.15f, 0.92f));
            _arenaRigText = CreateText(rightCard.transform, "OOC ARENA COMBAT RIG", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.92f, 0.90f, 0.96f));
            _arenaRigText.supportRichText = true;
            SetRect(_arenaRigText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));

            _standardGroundActionsRow = CreatePanel(body.transform, "GroundArenaActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(12f, 14f), new Vector2(-12f, 56f), Color.clear);
            var hlg = _standardGroundActionsRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            CreateButton(_standardGroundActionsRow.transform, "ROVER STATUS (+tv)", new Color(0.44f, 0.34f, 0.14f), () =>
            {
                int slot = _surface != null ? Mathf.Max(1, _surface.AddressedRigSlot) : 1;
                OnCommandRequested?.Invoke($"+tv/{slot} status");
            });
            CreateButton(_standardGroundActionsRow.transform, "ROVER SCAN", new Color(0.16f, 0.44f, 0.52f), () => OnCommandRequested?.Invoke("surface:scan"));
            CreateButton(_standardGroundActionsRow.transform, "ARENA STATUS", new Color(0.46f, 0.20f, 0.24f), () => OnCommandRequested?.Invoke("status"));
            CreateButton(_standardGroundActionsRow.transform, "SIM LOBBY", new Color(0.24f, 0.34f, 0.52f), () => OnCommandRequested?.Invoke("sim status"));

            _arenaRigActionsRow = CreatePanel(body.transform, "ArenaRigActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(12f, 14f), new Vector2(-12f, 56f), Color.clear);
            var ahlg = _arenaRigActionsRow.AddComponent<HorizontalLayoutGroup>();
            ahlg.spacing = 8f;
            ahlg.childControlWidth = true;
            ahlg.childControlHeight = true;
            ahlg.childForceExpandWidth = true;

            CreateButton(_arenaRigActionsRow.transform, "SCAN SECTOR", new Color(0.14f, 0.44f, 0.52f), () => DispatchArenaDossierAction("scan"));
            CreateButton(_arenaRigActionsRow.transform, "EMCON TOGGLE", new Color(0.32f, 0.24f, 0.50f), () => DispatchArenaDossierAction("emcon"));
            CreateButton(_arenaRigActionsRow.transform, "MIND CARD", new Color(0.16f, 0.42f, 0.36f), () => DispatchArenaDossierAction("mind_card"));
            CreateButton(_arenaRigActionsRow.transform, "STATUS", new Color(0.46f, 0.22f, 0.22f), () => DispatchArenaDossierAction("status"));
            CreateButton(_arenaRigActionsRow.transform, "MATCH HISTORY", new Color(0.24f, 0.34f, 0.48f), () => OnCommandRequested?.Invoke("sim history"));
            _arenaRigActionsRow.SetActive(false);
        }

        public bool ClickArenaDossierActionForTests(string actionKey) => DispatchArenaDossierAction(actionKey);

        private bool DispatchArenaDossierAction(string actionKey)
        {
            string k = (actionKey ?? string.Empty).Trim().ToLowerInvariant();
            switch (k)
            {
                case "scan":
                    OnArenaActionRequested?.Invoke("scan");
                    return true;
                case "emcon":
                    bool silent = _arena?.State != null && _arena.State.EmconSilent;
                    OnArenaActionRequested?.Invoke(silent ? "emcon on" : "emcon off");
                    return true;
                case "mind_card":
                    OnArenaActionRequested?.Invoke("MIND CARD");
                    return true;
                case "status":
                    OnArenaActionRequested?.Invoke("status");
                    return true;
                default:
                    return false;
            }
        }

        private void BuildRulesetAndUnitTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabRulesetAndUnit", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -92f), new Color(0.04f, 0.08f, 0.13f, 0.85f));
            _tabBodies[4] = body;

            var leftCard = CreatePanel(body.transform, "RulesetSheetCard", new Vector2(0f, 0f), new Vector2(0.49f, 1f), new Vector2(12f, 66f), new Vector2(-6f, -12f), new Color(0.05f, 0.09f, 0.15f, 0.94f));
            _rulesetSheetText = CreateText(leftCard.transform, "RULESET CHARACTER SHEET\nWaiting for live `*.sheet` frame...", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.94f, 0.98f));
            _rulesetSheetText.supportRichText = true;
            SetRect(_rulesetSheetText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));

            var rightCard = CreatePanel(body.transform, "UnitChassisCard", new Vector2(0.51f, 0f), new Vector2(1f, 1f), new Vector2(6f, 66f), new Vector2(-12f, -12f), new Color(0.05f, 0.10f, 0.14f, 0.94f));
            _unitChassisText = CreateText(rightCard.transform, "TACTICAL UNIT CHASSIS & VITALS\nWaiting for live `*.unit` / `*.vitals` frames...", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.90f, 0.94f, 0.98f));
            _unitChassisText.supportRichText = true;
            SetRect(_unitChassisText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));

            var actRow = CreatePanel(body.transform, "RulesetUnitActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(12f, 14f), new Vector2(-12f, 56f), Color.clear);
            var hlg = actRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            CreateButton(actRow.transform, "REFRESH SHEET", new Color(0.16f, 0.44f, 0.56f), () => OnRefreshSheetRequested?.Invoke());
            CreateButton(actRow.transform, "REFRESH UNIT & VITALS", new Color(0.18f, 0.48f, 0.40f), () => OnRefreshUnitAndVitalsRequested?.Invoke());
        }

        public bool ApplySheetPanel(JsonNode raw)
        {
            if (raw == null || raw.IsNull)
            {
                LiveSheet = null;
                RefreshRulesetAndUnitTab();
                return true;
            }
            if (!LiveSheetDocument.TryDecode(raw, out var doc))
                return false;
            LiveSheet = doc;
            RefreshRulesetAndUnitTab();
            return true;
        }

        public bool ApplyVitalsPanel(string feed, JsonNode raw)
        {
            bool ok = LiveVitals.Apply(feed, raw);
            if (ok)
            {
                RefreshPilotTab();
                RefreshRulesetAndUnitTab();
            }
            return ok;
        }

        public void ResetVitalsWatermarks()
        {
            LiveVitals.ResetWatermarks();
        }

        public bool ApplyUnitChassisPanel(JsonNode raw)
        {
            if (raw == null || raw.IsNull)
            {
                LiveUnitChassis = null;
                RefreshRulesetAndUnitTab();
                return true;
            }
            if (!LiveUnitChassisView.TryDecode(raw, out var view))
                return false;
            LiveUnitChassis = view;
            RefreshRulesetAndUnitTab();
            return true;
        }

        public void BindSources(
            TacSimTradeAndShipState trade,
            SpaceFlightController space,
            SurfaceMiningController surface,
            OocArenaController arena)
        {
            _trade = trade;
            _space = space;
            _surface = surface;
            _arena = arena;
            RefreshContents();
        }

        public void BindQuestJournal(TacSimQuestJournal journal)
        {
            if (_questJournal == journal) return;
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated -= HandleQuestsUpdated;
            _questJournal = journal;
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated += HandleQuestsUpdated;
            if (IsOpen) RefreshPilotTab();
        }

        private void HandleQuestsUpdated()
        {
            if (IsOpen) RefreshPilotTab();
        }

        private void OnDestroy()
        {
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated -= HandleQuestsUpdated;
        }

        public void SetPilotContext(
            string pilotName,
            string locationName,
            string hullBadge,
            TacSimActiveStage stage,
            PlayerAvatarPreset? currentAvatar = null)
        {
            if (!string.IsNullOrWhiteSpace(pilotName)) _pilotName = pilotName.Trim();
            if (!string.IsNullOrWhiteSpace(locationName)) _locationName = locationName.Trim();
            if (!string.IsNullOrWhiteSpace(hullBadge)) _hullBadge = hullBadge.Trim();
            _activeStage = stage;
            if (currentAvatar.HasValue)
                ApplyAvatarChoice(currentAvatar.Value, notify: false);
            if (IsOpen)
                RefreshContents();
        }

        public void SetOpen(bool open)
        {
            if (_modalRoot == null) return;
            _modalRoot.SetActive(open);
            if (open)
            {
                if (ActiveTab == CharacterSheetTab.Pilot &&
                    (_activeStage == TacSimActiveStage.SimulatorArena || _activeStage == TacSimActiveStage.PlanetarySurface))
                {
                    SelectTab(CharacterSheetTab.GroundArena);
                    return;
                }
                RefreshContents();
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void ResetSessionState()
        {
            _pilotName = "Unattached Pilot";
            _locationName = "Ceres Freeport";
            _hullBadge = "NO CHARTER";
            _activeStage = TacSimActiveStage.StationWalk;
            LiveSheet = null;
            LiveUnitChassis = null;
            LiveVitals.Clear();
            SetOpen(false);
        }

        public void SelectTab(CharacterSheetTab tab)
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
                    ? new Color(0.20f, 0.56f, 0.78f, 1f)
                    : new Color(0.11f, 0.20f, 0.30f, 0.95f);
            }
            RefreshContents();
        }

        public void ApplyAvatarChoice(PlayerAvatarPreset preset, bool notify)
        {
            _selectedAvatar = preset;
            for (int i = 0; i < _avatarButtons.Count; i++)
            {
                _avatarButtons[i].color = (i == (int)preset)
                    ? new Color(0.22f, 0.64f, 0.86f, 1f)
                    : new Color(0.13f, 0.22f, 0.32f, 0.94f);
            }
            if (notify)
                OnAvatarPresetSelected?.Invoke(preset);
        }

        private void Update()
        {
            if (!IsOpen) return;
            if (Time.unscaledTime < _nextAutoRefreshAt) return;
            _nextAutoRefreshAt = Time.unscaledTime + 0.35f;
            RefreshContents();
        }

        public void RefreshContents()
        {
            if (_subtitleText != null)
                _subtitleText.text = $"STAGE: {FormatStageLabel(_activeStage)}";

            RefreshPilotTab();
            RefreshShipCargoTab();
            RefreshSpaceFitTab();
            RefreshGroundArenaTab();
            RefreshRulesetAndUnitTab();
        }

        private void RefreshPilotTab()
        {
            if (_pilotSummaryText == null) return;
            string credits = _trade != null ? _trade.CreditsText : "— cr";
            string venue = _trade != null && !string.IsNullOrEmpty(_trade.ActiveVenue) ? _trade.ActiveVenue : _locationName;
            string hullRef = _trade != null && !string.IsNullOrEmpty(_trade.ShipHullId) ? _trade.ShipHullId : "—";

            var sb = new StringBuilder(384);
            sb.AppendFormat("<color=#66EEFF><b>PILOT:</b></color> <b>{0}</b>\n", MushProtocol.EscapeUnityRichText(_pilotName));
            sb.AppendFormat("<color=#66EEFF><b>LOCATION:</b></color> {0}\n", MushProtocol.EscapeUnityRichText(venue));
            sb.AppendFormat("<color=#66EEFF><b>ACTIVE STAGE:</b></color> {0}\n", FormatStageLabel(_activeStage));
            sb.AppendFormat("<color=#66EEFF><b>CHARTER / HULL:</b></color> {0}  (Ref: {1})\n",
                MushProtocol.EscapeUnityRichText(_hullBadge),
                MushProtocol.EscapeUnityRichText(hullRef));
            sb.AppendFormat("<color=#66FFAA><b>AVAILABLE CREDITS:</b></color> <b>{0}</b>\n", MushProtocol.EscapeUnityRichText(credits));
            string vitalsCompact = LiveVitals.FormatRichSummary(compact: true);
            if (!string.IsNullOrEmpty(vitalsCompact))
                sb.Append(vitalsCompact).Append('\n');

            var qPanel = _questJournal?.LatestPanel;
            if (qPanel != null)
            {
                var activeQuest = _questJournal.GetTrackedActiveQuest();
                if (activeQuest != null)
                {
                    int done = 0;
                    for (int i = 0; i < activeQuest.Checkpoints.Count; i++)
                    {
                        if (activeQuest.Checkpoints[i].Checked || activeQuest.Checkpoints[i].LiveReady)
                            done++;
                    }
                    string prog = activeQuest.Checkpoints.Count > 0 ? $" ({done}/{activeQuest.Checkpoints.Count})" : string.Empty;
                    string next = !string.IsNullOrEmpty(activeQuest.NextStep) ? $" — {MushProtocol.EscapeUnityRichText(activeQuest.NextStep)}" : string.Empty;
                    sb.AppendFormat("<color=#FFDD44><b>ACTIVE QUEST:</b></color> <b>{0}</b>{1}{2}  |  Completed: {3}\n",
                        MushProtocol.EscapeUnityRichText(activeQuest.Title),
                        prog,
                        next,
                        qPanel.Completed.Count);
                }
                else if (qPanel.Available.Count > 0)
                {
                    sb.AppendFormat("<color=#FFDD44><b>QUEST AVAILABLE:</b></color> <b>{0}</b>  |  Completed: {1}\n",
                        MushProtocol.EscapeUnityRichText(qPanel.Available[0].Title),
                        qPanel.Completed.Count);
                }
                else if (qPanel.Completed.Count > 0)
                {
                    sb.AppendFormat("<color=#66FFAA><b>QUESTS COMPLETED:</b></color> {0}\n", qPanel.Completed.Count);
                }
            }

            sb.Append("<color=#88AAC8>Shortcuts: [Ctrl+K] Palette  [F1] Keys/Macros  [J] Quests  [C] Sheet  [M] Chart  [G] Ground  [N] Nav</color>");
            _pilotSummaryText.text = sb.ToString();
        }

        private void RefreshRulesetAndUnitTab()
        {
            if (_rulesetSheetText != null)
            {
                _rulesetSheetText.text = LiveSheet != null
                    ? LiveSheet.FormatRichSummary()
                    : "<color=#66EEFF><b>RULESET CHARACTER SHEET</b></color>\n\nNo live `*.sheet` payload received for this world yet.\nClick <b>REFRESH SHEET</b> to query the server.";
            }

            if (_unitChassisText != null)
            {
                var sb = new StringBuilder(512);
                if (LiveUnitChassis != null)
                {
                    sb.AppendLine(LiveUnitChassis.FormatRichSummary());
                }
                else
                {
                    sb.AppendLine("<color=#FFCC66><b>TACTICAL UNIT CHASSIS</b></color>");
                    sb.AppendLine("No tactical unit active: take a station or click <b>REFRESH UNIT &amp; VITALS</b>.");
                }

                string vitalsFull = LiveVitals.FormatRichSummary(compact: false);
                if (!string.IsNullOrEmpty(vitalsFull))
                {
                    sb.AppendLine();
                    sb.Append(vitalsFull);
                }
                _unitChassisText.text = sb.ToString().TrimEnd();
            }
        }

        private void RefreshShipCargoTab()
        {
            if (_shipIdentityText == null) return;
            string hullRef = _trade != null && !string.IsNullOrEmpty(_trade.ShipHullId) ? _trade.ShipHullId : "None";
            string portHint = _trade != null && !string.IsNullOrEmpty(_trade.AboardDockedPortHint) ? _trade.AboardDockedPortHint : "Undocked / Unknown";
            _shipIdentityText.text =
                $"<color=#66EEFF><b>ACTIVE HULL:</b></color> <b>{MushProtocol.EscapeUnityRichText(_hullBadge)}</b>   |   <b>DBREF:</b> {MushProtocol.EscapeUnityRichText(hullRef)}\n" +
                $"<color=#66EEFF><b>BERTH / PORT:</b></color> {MushProtocol.EscapeUnityRichText(portHint)}   |   <b>CREDITS:</b> {MushProtocol.EscapeUnityRichText(_trade?.CreditsText ?? "— cr")}";

            if (_trade != null)
            {
                SetGauge(_massFill, _massLabel, _trade.HoldMassTonnes, Mathf.Max(1, _trade.HoldMassLimitTonnes), $"{_trade.HoldMassTonnes} / {_trade.HoldMassLimitTonnes} t");
                SetGauge(_volFill, _volLabel, _trade.HoldVolumeM3, Mathf.Max(1f, _trade.HoldVolumeLimitM3), $"{_trade.HoldVolumeM3:0.0} / {_trade.HoldVolumeLimitM3:0} m³");
                SetGauge(_tankFill, _tankLabel, _trade.TankTonnes, Mathf.Max(1, _trade.TankLimitTonnes), $"{_trade.TankTonnes} / {_trade.TankLimitTonnes} t");
                SetGauge(_storesFill, _storesLabel, _trade.StoresCrewDays, Mathf.Max(1, _trade.StoresLimitCrewDays), $"{_trade.StoresCrewDays} / {_trade.StoresLimitCrewDays} d");
            }

            if (_shipOrdersText != null)
            {
                var sb = new StringBuilder(384);
                sb.AppendLine("<color=#66DDFF><b>ACTIVE SAILING PLAN & BERTH REGISTRY</b></color>");
                if (_trade != null && _trade.SailingOrderLines.Count > 0)
                {
                    int max = Mathf.Min(_trade.SailingOrderLines.Count, 7);
                    for (int i = 0; i < max; i++)
                        sb.AppendLine("• " + MushProtocol.EscapeUnityRichText(_trade.SailingOrderLines[i]));
                }
                else
                {
                    sb.AppendLine("• No queued sailing orders. Open Sailing Planner or Solar Chart [M] to dry-run burn & depart.");
                }

                if (_trade != null && _trade.BerthDockedHulls.Count > 0)
                {
                    sb.AppendLine();
                    sb.AppendFormat("<color=#FFCC66><b>LOCAL BERTH HULLS ({0}):</b></color> ", _trade.BerthDockedHulls.Count);
                    int shown = Mathf.Min(5, _trade.BerthDockedHulls.Count);
                    for (int i = 0; i < shown; i++)
                    {
                        if (i > 0) sb.Append("  |  ");
                        sb.Append(MushProtocol.EscapeUnityRichText(_trade.BerthDockedHulls[i].Name));
                    }
                }
                _shipOrdersText.text = sb.ToString();
            }
        }

        private void RefreshSpaceFitTab()
        {
            if (_spaceFitHeaderText == null || _spaceMountsText == null) return;
            if (_space == null)
            {
                _spaceFitHeaderText.text = "ORBITAL SPACE STAGE UNAVAILABLE";
                _spaceMountsText.text = string.Empty;
                return;
            }

            string emcon = string.IsNullOrEmpty(_space.EmconPosture) ? "STANDARD" : _space.EmconPosture.ToUpperInvariant();
            int throttlePct = Mathf.RoundToInt(Mathf.Clamp01(_space.ThrottlePermille / 1000f) * 100f);
            string lockStr = _space.ActiveLock != null && !string.IsNullOrEmpty(_space.ActiveLock.ShipId)
                ? $"<color=#FF7766><b>{MushProtocol.EscapeUnityRichText(string.IsNullOrEmpty(_space.ActiveLock.Label) ? _space.ActiveLock.ShipId : _space.ActiveLock.Label)}</b></color> ({_space.ActiveLock.RangeM / 1000.0:0.1} km, {_space.ActiveLock.Facet.ToUpperInvariant()})"
                : (!string.IsNullOrEmpty(_space.SelectedRockId)
                    ? $"<color=#FFCC55>ASTEROID {MushProtocol.EscapeUnityRichText(_space.SelectedRockId)}</color>"
                    : "NONE");

            _spaceFitHeaderText.text =
                $"TORCH: <b>{throttlePct}%</b>   |   EMCON: <b>{MushProtocol.EscapeUnityRichText(emcon)}</b>   |   CONTACTS: <b>{_space.RadarBlips.Count}</b>   |   TORPEDOES: <b>{_space.LiveTorpedoCount}</b>\n" +
                $"HELD LOCK / TARGET: {lockStr}";

            var sb = new StringBuilder(512);
            sb.AppendLine("<color=#66DDFF><b>SHIP WEAPON MOUNTS & FIRE-CONTROL TELEMETRY</b></color>");
            if (_space.FireMounts.Count == 0)
            {
                sb.AppendLine("• No live weapon mount frames received yet. Sit at the bridge helm and run `+tac/scan` or undock.");
            }
            else
            {
                for (int i = 0; i < _space.FireMounts.Count; i++)
                {
                    var m = _space.FireMounts[i];
                    string stCol = m.State == "ready" ? "#66FF99" : (m.State == "down" || m.State == "empty" ? "#FF6655" : "#FFCC55");
                    string ammo = m.Rounds >= 0 ? (m.RoundsMax > 0 ? $" | AMMO {m.Rounds}/{m.RoundsMax}" : $" | AMMO {m.Rounds}") : string.Empty;
                    string lockEst = m.DamageAtLock >= 0 ? $" | DMG@LOCK {m.DamageAtLock}" : (m.FlightMsAtLock >= 0 ? $" | TOF {m.FlightMsAtLock}ms" : string.Empty);
                    string cd = m.ReadyInMs > 0 ? $" ({m.ReadyInMs / 1000f:0.1}s)" : string.Empty;
                    sb.AppendFormat("• <b>{0}</b> [{1}] — <color={2}><b>{3}{4}</b></color>{5}{6}\n",
                        MushProtocol.EscapeUnityRichText(m.Id.ToUpperInvariant()),
                        MushProtocol.EscapeUnityRichText(m.Family.ToUpperInvariant()),
                        stCol,
                        MushProtocol.EscapeUnityRichText(m.State.ToUpperInvariant()),
                        cd,
                        ammo,
                        lockEst);
                }
            }
            _spaceMountsText.text = sb.ToString();
        }

        private void RefreshGroundArenaTab()
        {
            if (_surfaceRigText != null)
            {
                var sb = new StringBuilder(320);
                sb.AppendLine("<color=#FFCC66><b>SURFACE PROSPECTOR ROVER (+tv)</b></color>");
                if (_surface != null)
                {
                    sb.AppendFormat("Site: <b>{0}</b>\n", MushProtocol.EscapeUnityRichText(_surface.ActiveSiteTitle));
                    sb.AppendFormat("Rig Slot: <b>#{0}</b>  |  State: <b>{1}</b>\n",
                        _surface.AddressedRigSlot,
                        MushProtocol.EscapeUnityRichText((_surface.RigStateWord ?? "bay").ToUpperInvariant()));
                    sb.AppendFormat("Grid Cell: <b>({0}, {1})</b>  |  Hdg: <b>{2:000}°</b>\n",
                        _surface.AuthoritativeSiteCellX,
                        _surface.AuthoritativeSiteCellY,
                        (_surface.RoverHeadingDeg % 360f + 360f) % 360f);
                    sb.AppendFormat("Hopper: <b>{0} / {1} t</b>  |  Battery: <b>{2}%</b>\n",
                        _surface.HopperLoadedTonnes,
                        _surface.HopperCapacityTonnes,
                        _surface.BatteryPct);
                    sb.AppendFormat("Ore Filter: <b>{0}</b>  |  Hold Free: <b>{1} t</b>\n\n",
                        MushProtocol.EscapeUnityRichText(_surface.ActiveOreFamily.ToUpperInvariant()),
                        _surface.HoldUnreservedTonnes);
                    sb.Append(MushProtocol.EscapeUnityRichText(_surface.StatusSummaryLine));
                }
                else
                {
                    sb.Append("Surface Rover module inactive.");
                }
                _surfaceRigText.text = sb.ToString();
            }

            bool arenaLive = _arena != null && _arena.IsArenaActive && _arena.State != null;
            if (_standardGroundActionsRow != null) _standardGroundActionsRow.SetActive(!arenaLive);
            if (_arenaRigActionsRow != null) _arenaRigActionsRow.SetActive(arenaLive);

            if (_arenaRigText != null)
            {
                var sb = new StringBuilder(640);
                sb.AppendLine("<color=#FF7766><b>OOC SIMULATOR COMBAT RIG</b></color>");
                if (_arena != null && _arena.State != null)
                {
                    var st = _arena.State;
                    var ti = _arena.CurrentTerrainIntel;
                    string clockTxt = st.ArenaClockMs > 0 ? $" · {TacSimArenaReplayModel.FormatClockText(st.ArenaClockMs)}" : string.Empty;
                    string matchTag = st.MatchId > 0 ? $"MATCH #{st.MatchId}{clockTxt}  |  " : string.Empty;
                    sb.AppendFormat("{0}Callsign: <b>{1}</b> ({2})  |  Unit: <b>{3}</b>\n",
                        matchTag,
                        MushProtocol.EscapeUnityRichText(st.Callsign),
                        MushProtocol.EscapeUnityRichText(st.Party),
                        MushProtocol.EscapeUnityRichText((st.UnitType ?? string.Empty).Replace("tacsim.unit.", string.Empty)));

                    int shots = st.MatchHits + st.MatchMisses;
                    int accPct = shots > 0 ? Mathf.RoundToInt((st.MatchHits * 100f) / shots) : 0;
                    sb.AppendFormat("Mode: <b>{0}</b> (RED {1} vs BLUE {2})  |  Posture: <b>{3}</b>\n",
                        MushProtocol.EscapeUnityRichText((st.Mode ?? "skirmish").ToUpperInvariant()),
                        st.RedAliveCount,
                        st.BlueAliveCount,
                        MushProtocol.EscapeUnityRichText((st.Posture ?? "return").ToUpperInvariant()));
                    sb.AppendFormat("Power: <b>{0}</b>  |  Heat: <b>{1}/{2}</b>  |  Score: <b>{3}K</b> {4}/{5}H ({6}%)\n",
                        MushProtocol.EscapeUnityRichText((st.Power ?? "online").ToUpperInvariant()),
                        st.Heat,
                        st.HeatMax,
                        st.MatchKills,
                        st.MatchHits,
                        shots,
                        accPct);

                    string ecmTxt = !st.HasEcm
                        ? "NO ECM"
                        : (st.EmconSilent ? "ECM POD: SILENT" : $"ECM POD: ACTIVE ({st.EcmRadiusMeters:0}m)");
                    string rcvrTxt = st.HasReceiver ? $"  |  RCVR {st.ReceiverReachPercent}%" : string.Empty;
                    string coverTxt = !string.IsNullOrEmpty(ti.CoverLabel) ? ti.CoverLabel : "OPEN";
                    string losTxt = ti.HasTargetSample
                        ? (ti.IsLineOfSightObstructed ? $" · {ti.LineOfSightStatus} LOS" : " · CLEAR LOS")
                        : string.Empty;
                    sb.AppendFormat("Sensors: <b>{0:0}m</b> ({1})  |  {2}{3}\n",
                        st.SightMeters,
                        MushProtocol.EscapeUnityRichText((st.Scope ?? "clear").ToUpperInvariant()),
                        ecmTxt,
                        rcvrTxt);
                    sb.AppendFormat("Terrain: <b>{0}</b>{1}\n",
                        MushProtocol.EscapeUnityRichText(coverTxt),
                        MushProtocol.EscapeUnityRichText(losTxt));
                    sb.AppendFormat("Armor: <b>F {0}%</b> | <b>L {1}%</b> | <b>R {2}%</b> | <b>B {3}%</b> | <b>INT {4}%</b>\n",
                        st.ArmorFrontPct, st.ArmorLeftPct, st.ArmorRightPct, st.ArmorRearPct, st.InternalsPct);
                    if (!string.IsNullOrEmpty(st.LockedDesignator))
                        sb.AppendFormat("Locked Target: <color=#FFCC55><b>{0}</b></color> ({1:0} m, {2}% hit)\n",
                            MushProtocol.EscapeUnityRichText(st.LockedDesignator), st.LockRangeM, st.LockHitPercent);

                    if (st.Mind != null)
                    {
                        string mindTitle = string.IsNullOrWhiteSpace(st.Mind.Name) ? "OWN MIND" : st.Mind.Name;
                        string prof = string.IsNullOrWhiteSpace(st.Mind.ActiveProfileName) ? "DEFAULT" : st.Mind.ActiveProfileName;
                        sb.AppendFormat("Mind: <b>{0}</b> [Profile: <b>{1}</b> · Rules: {2}]\n",
                            MushProtocol.EscapeUnityRichText(mindTitle),
                            MushProtocol.EscapeUnityRichText(prof),
                            st.Rules.Count);
                    }

                    if (st.Weapons.Count > 0)
                    {
                        sb.AppendLine("\n<color=#88DDFF><b>WEAPONS:</b></color>");
                        int count = Mathf.Min(st.Weapons.Count, 4);
                        for (int i = 0; i < count; i++)
                        {
                            var w = st.Weapons[i];
                            string rdy = w.Ready ? "RDY" : $"{w.ReadyInMs / 1000f:0.1}s";
                            string bandInfo = w.MaxRangeM > 1f ? $" · {w.MaxRangeM:0}m" : string.Empty;
                            sb.AppendFormat("• {0}: {1} | Ammo {2}/{3} | Hit {4}%{5}\n",
                                MushProtocol.EscapeUnityRichText(w.Id ?? $"W{i + 1}"), rdy, w.Rounds, w.RoundsMax, w.HitPercent, bandInfo);
                        }
                    }
                }
                else
                {
                    sb.Append("OOC Arena simulator inactive.");
                }
                _arenaRigText.text = sb.ToString();
            }
        }

        private static string FormatStageLabel(TacSimActiveStage stage)
        {
            switch (stage)
            {
                case TacSimActiveStage.StationWalk: return "STATION WALK";
                case TacSimActiveStage.ShipInteriorWalk: return "SHIP DECKS";
                case TacSimActiveStage.ShipHelmSeated: return "ORBITAL HELM";
                case TacSimActiveStage.PlanetarySurface: return "SURFACE ROVER";
                case TacSimActiveStage.SimulatorArena: return "OOC ARENA";
                default: return "ACTIVE";
            }
        }

        private void BuildGaugeCell(Transform parent, int index, string title, string initialValue, Color fillColor, out Image fillImg, out Text valText)
        {
            float x0 = index * 0.25f;
            float x1 = x0 + 0.25f;
            var cell = CreatePanel(parent, "Gauge_" + title, new Vector2(x0, 0f), new Vector2(x1, 1f), new Vector2(8f, 6f), new Vector2(-8f, -6f), new Color(0.03f, 0.06f, 0.10f, 0.9f));

            var hdr = CreateText(cell.transform, title, 10, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.68f, 0.82f, 0.92f));
            SetRect(hdr.rectTransform, new Vector2(0f, 0.56f), new Vector2(1f, 1f), new Vector2(8f, 0f), new Vector2(-8f, -3f));

            var barBg = CreatePanel(cell.transform, "BarBg", new Vector2(0f, 0f), new Vector2(1f, 0.52f), new Vector2(8f, 5f), new Vector2(-8f, -2f), new Color(0.10f, 0.14f, 0.20f, 1f));
            var barFill = CreatePanel(barBg.transform, "BarFill", Vector2.zero, new Vector2(0.01f, 1f), Vector2.zero, Vector2.zero, fillColor);
            fillImg = barFill.GetComponent<Image>();

            valText = CreateText(barBg.transform, initialValue, 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(valText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
        }

        private static void SetGauge(Image fillImg, Text valText, float current, float max, string label)
        {
            if (fillImg != null)
            {
                float frac = Mathf.Clamp01(max > 0f ? current / max : 0f);
                fillImg.rectTransform.anchorMax = new Vector2(Mathf.Max(0.01f, frac), 1f);
            }
            if (valText != null)
                valText.text = label;
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
                disabledColor = new Color(0.5f, 0.5f, 0.5f, 0.6f),
                colorMultiplier = 1f,
                fadeDuration = 0.06f
            };
            btn.onClick.AddListener(() => onClick?.Invoke());

            var txt = CreateText(go.transform, label, 12, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
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
