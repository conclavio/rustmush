using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Character;
using TacSim.Core;
using TacSim.Net;
using TacSim.Space;
using TacSim.World;
using UnityEngine;
using UnityEngine.EventSystems;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// Self-constructing diegetic Sci-Fi HUD using built-in uGUI.
    /// Always rebuilds runtime listeners on Play Mode entry (IsInitialized is non-serialized) so
    /// saved scene hierarchies never suffer from stripped runtime UnityEvent closures.
    /// </summary>
    public sealed class TacSimHudController : MonoBehaviour
    {
        private const string PrefLastUrl = "TacSim.LastServerUrl";
        private const string PrefLastAccount = "TacSim.LastAccount";

        public event Action<string, string, string, string, bool, PlayerAvatarPreset> OnAuthSubmitted;
        public event Action<MushAuthSession, MushCharacterSlot, PlayerAvatarPreset> OnCharacterSelected;
        public event Action<string> OnCommandSubmitted;
        public event Action<bool> OnComposingChanged;
        public event Action OnRefreshTradeRequested;
        public event Action<ulong> OnLoadMoreTradeRequested;
        public event Action<MarketListingRow, int, bool> OnMarketBuyRequested;
        public event Action<MarketListingRow, int, bool> OnMarketSellRequested;
        public event Action<MushServerRequest, string> OnRequestChoiceSubmitted;
        public event Action<CameraRigMode> OnCameraModeRequested;
        public event Action OnStandFromHelmRequested;

        /// <summary>
        /// Non-serialized flag: always false when entering Play Mode from a saved scene so BuildHud()
        /// wires fresh runtime Button.onClick closures and InputField references.
        /// </summary>
        public bool IsInitialized { get; private set; }
        public TacSimMissionOverlays Overlays { get; private set; }
        public bool IsModalOpen =>
            (_modalPanel != null && _modalPanel.activeSelf) ||
            (_tradePanel != null && _tradePanel.activeSelf) ||
            (Overlays != null && Overlays.IsLobbyModalOpen);

        public bool IsTextInputFocused =>
            (_chatInput != null && _chatInput.isFocused) ||
            (_npcSayInput != null && _npcSayInput.isFocused) ||
            (_loginPanel != null && _loginPanel.activeSelf);

        private Font _uiFont;
        private Canvas _canvas;

        // Login / Avatar panel
        private GameObject _loginPanel;
        private GameObject _credentialsGroup;
        private GameObject _charSelectGroup;
        private Transform _charSelectListRoot;
        private InputField _urlInput;
        private InputField _accountInput;
        private InputField _passwordInput;
        private InputField _charNameInput;
        private Text _loginStatusText;
        private PlayerAvatarPreset _selectedAvatar = PlayerAvatarPreset.StellarTrooperFemale;
        private readonly List<Image> _avatarButtons = new List<Image>();

        // Top bar
        private GameObject _hudRoot;
        private Text _locationText;
        private Text _netBadgeText;
        private Text _shipAndClockText;

        // Composing typing indicator debounce
        private bool _composingActive;
        private float _composingIdleAt;

        // [E] Interaction prompt
        private GameObject _interactBanner;
        private Text _interactTitleText;
        private Text _interactSubText;

        // Contextual Quick Actions strip & Bridge Helm Flight Deck bar
        private GameObject _quickActionsStripPanel;
        private Transform _quickActionsContainer;
        private GameObject _helmFlightBar;
        private Text _helmStatusLabel;

        // Slice 1C: Visual Commodity Exchange, Dual Hold Bars & Sailing Plan Terminal
        private GameObject _tradePanel;
        private TacSimTradeAndShipState _boundTradeState;
        private bool _tradeSailingTabActive;
        private Text _tradeHeaderText;
        private Text _tradeCreditsText;
        private Image _massBarFill;
        private Text _massBarLabel;
        private Image _volBarFill;
        private Text _volBarLabel;
        private Image _tankBarFill;
        private Text _tankBarLabel;
        private Image _storesBarFill;
        private Text _storesBarLabel;
        private GameObject _exchangeTabBody;
        private GameObject _sailingTabBody;
        private Image _exchangeTabBtnImg;
        private Image _sailingTabBtnImg;
        private Transform _listingsContentRoot;
        private readonly List<Image> _qtyPresetImages = new List<Image>();
        private readonly List<Image> _destPortImages = new List<Image>();
        private Text _sailingPlanOutputText;
        private readonly List<string> _sailingEchoLines = new List<string>();

        // Modal dialog
        private GameObject _modalPanel;
        private Text _modalHeaderText;
        private Text _modalBodyText;
        private ScrollRect _modalScrollRect;
        private Transform _modalButtonsRoot;
        private InputField _npcSayInput;
        private string _activeNpcTargetName;
        private string _modalCommandPrefix;
        private string _modalBaseIntro = string.Empty;
        private readonly List<string> _modalResponseLines = new List<string>();

        // 6-DOF Docking Telemetry Panel
        private GameObject _dockingPanel;
        private Text _dockingTelemetryText;
        private readonly Image[] _gatePills = new Image[5];
        private readonly Text[] _gateLabels = new Text[5];

        // Comms drawer
        private Text _commsLogText;
        private ScrollRect _commsScrollRect;
        private InputField _chatInput;
        private readonly Queue<string> _logLines = new Queue<string>();
        private const int MaxLogLines = 45;

        public void BuildHud()
        {
            _uiFont = Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            if (_uiFont == null)
                _uiFont = Font.CreateDynamicFontFromOSFont("Consolas", 14);

            EnsureEventSystem();

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _avatarButtons.Clear();
            _qtyPresetImages.Clear();
            _destPortImages.Clear();
            _lastQuickActionKey = null;
            _lastQuickActionSeated = null;

            var canvasGo = new GameObject("TacSimCanvas");
            canvasGo.transform.SetParent(transform, false);
            _canvas = canvasGo.AddComponent<Canvas>();
            _canvas.renderMode = RenderMode.ScreenSpaceOverlay;
            _canvas.sortingOrder = 20;

            var scaler = canvasGo.AddComponent<CanvasScaler>();
            scaler.uiScaleMode = CanvasScaler.ScaleMode.ScaleWithScreenSize;
            scaler.referenceResolution = new Vector2(1920f, 1080f);
            scaler.matchWidthOrHeight = 0.5f;
            canvasGo.AddComponent<GraphicRaycaster>();

            BuildActiveGameplayHud(canvasGo.transform);
            BuildLoginAndAvatarOverlay(canvasGo.transform);

            Overlays = gameObject.GetComponent<TacSimMissionOverlays>() ?? gameObject.AddComponent<TacSimMissionOverlays>();
            Overlays.BuildOverlays(_hudRoot.transform, _uiFont);

            IsInitialized = true;
            ShowLoginOverlay(true, "Sign in to mushd (e.g. https://dev1.conclav.io) to enter the 2350 Corridor.");
        }

        private static void EnsureEventSystem()
        {
            var es = FindAnyObjectByType<EventSystem>();
            if (es == null)
            {
                var esGo = new GameObject("EventSystem");
                es = esGo.AddComponent<EventSystem>();
            }
            if (es.GetComponent<BaseInputModule>() == null)
            {
                es.gameObject.AddComponent<StandaloneInputModule>();
            }
        }

        private void BuildActiveGameplayHud(Transform root)
        {
            _hudRoot = CreatePanel(root, "GameplayHud", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, Color.clear);

            // 1. Top Telemetry Strip
            var topBar = CreatePanel(_hudRoot.transform, "TopStrip", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -64f), new Vector2(-16f, -10f), new Color(0.04f, 0.07f, 0.11f, 0.86f));
            _locationText = CreateText(topBar.transform, "CERES FREEPORT — PORT WATCH ARRIVALS [PUBLIC DECK]", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.82f, 0.94f, 1f));
            SetRect(_locationText.rectTransform, new Vector2(0f, 0f), new Vector2(0.50f, 1f), new Vector2(18f, 0f), Vector2.zero);

            _netBadgeText = CreateText(topBar.transform, "[DISCONNECTED]", 13, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.62f, 0.72f, 0.82f));
            SetRect(_netBadgeText.rectTransform, new Vector2(0.50f, 0f), new Vector2(0.61f, 1f), Vector2.zero, Vector2.zero);

            _shipAndClockText = CreateText(topBar.transform, "IC 08:00 UTC | HULL: UNCHARTERED | RMB: Orbit Camera", 15, FontStyle.Normal, TextAnchor.MiddleRight, new Color(0.62f, 0.84f, 0.96f));
            SetRect(_shipAndClockText.rectTransform, new Vector2(0.61f, 0f), new Vector2(1f, 1f), Vector2.zero, new Vector2(-18f, 0f));

            // 2. Center-Bottom [E] Contextual Interaction Banner
            _interactBanner = CreatePanel(_hudRoot.transform, "InteractBanner", new Vector2(0.5f, 0f), new Vector2(0.5f, 0f), new Vector2(-240f, 128f), new Vector2(240f, 198f), new Color(0.05f, 0.12f, 0.18f, 0.92f));
            _interactTitleText = CreateText(_interactBanner.transform, "[E] Inspect", 18, FontStyle.Bold, TextAnchor.UpperCenter, new Color(0.35f, 0.95f, 0.72f));
            SetRect(_interactTitleText.rectTransform, new Vector2(0f, 0.45f), new Vector2(1f, 1f), new Vector2(12f, -6f), new Vector2(-12f, 0f));
            _interactSubText = CreateText(_interactBanner.transform, "", 14, FontStyle.Normal, TextAnchor.LowerCenter, new Color(0.78f, 0.88f, 0.95f));
            SetRect(_interactSubText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0.5f), new Vector2(12f, 6f), new Vector2(-12f, 0f));
            _interactBanner.SetActive(false);

            // 3. Bottom-Right Diegetic Quick Action Strip + Seated Bridge Flight Strip
            // Anchored strictly to the right of CommsDrawer (`x = 452 .. -16`) so narrow aspect ratios (e.g. 1100x756) never overlap!
            _quickActionsStripPanel = CreatePanel(_hudRoot.transform, "QuickActionStrip", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 16f), new Vector2(-16f, 64f), new Color(0.04f, 0.07f, 0.11f, 0.86f));
            var hlg = _quickActionsStripPanel.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 7f;
            hlg.padding = new RectOffset(10, 10, 7, 7);
            hlg.childAlignment = TextAnchor.MiddleCenter;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;
            _quickActionsContainer = _quickActionsStripPanel.transform;

            BuildHelmFlightDeckBar(_hudRoot.transform);

            // 4. Bottom-Left Compact Comms & Narrative Drawer (ScrollRect + RectMask2D pinned to newest bottom line)
            var commsPanel = CreatePanel(_hudRoot.transform, "CommsDrawer", new Vector2(0f, 0f), new Vector2(0f, 0f), new Vector2(14f, 16f), new Vector2(436f, 248f), new Color(0.03f, 0.05f, 0.08f, 0.88f));
            var commsViewport = CreatePanel(commsPanel.transform, "CommsViewport", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(12f, 44f), new Vector2(-12f, -10f), new Color(0f, 0f, 0f, 0.02f));
            commsViewport.AddComponent<RectMask2D>();
            _commsScrollRect = commsViewport.AddComponent<ScrollRect>();

            _commsLogText = CreateText(commsViewport.transform, "<color=#66BBFF>[COMMS]</color> Welcome to Ceres Freeport, 2350.", 13, FontStyle.Normal, TextAnchor.LowerLeft, new Color(0.84f, 0.90f, 0.95f));
            _commsLogText.supportRichText = true;
            _commsLogText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _commsLogText.verticalOverflow = VerticalWrapMode.Overflow;
            var commsFitter = _commsLogText.gameObject.AddComponent<ContentSizeFitter>();
            commsFitter.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            commsFitter.verticalFit = ContentSizeFitter.FitMode.PreferredSize;
            _commsLogText.rectTransform.anchorMin = new Vector2(0f, 0f);
            _commsLogText.rectTransform.anchorMax = new Vector2(1f, 0f);
            _commsLogText.rectTransform.pivot = new Vector2(0.5f, 0f);
            _commsLogText.rectTransform.offsetMin = Vector2.zero;
            _commsLogText.rectTransform.offsetMax = Vector2.zero;

            _commsScrollRect.viewport = commsViewport.GetComponent<RectTransform>();
            _commsScrollRect.content = _commsLogText.rectTransform;
            _commsScrollRect.horizontal = false;
            _commsScrollRect.vertical = true;
            _commsScrollRect.movementType = ScrollRect.MovementType.Clamped;
            _commsScrollRect.scrollSensitivity = 28f;

            _chatInput = CreateInputField(commsPanel.transform, "Enter command (+coop, look, say hi)...", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(10f, 8f), new Vector2(-78f, 38f));
            _chatInput.onSubmit.AddListener(_ => SubmitChatInput());
            _chatInput.onValueChanged.AddListener(val => HandleInputComposingEdited(val, isNpcSayBox: false));

            var sendBtn = CreateButton(commsPanel.transform, "SEND", new Color(0.14f, 0.44f, 0.52f), SubmitChatInput);
            SetRect(sendBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-72f, 8f), new Vector2(-10f, 38f));

            // 5. 60 Hz 6-DOF Bow Docking Collar HUD
            BuildDockingHudOverlay(_hudRoot.transform);

            // 6. Modal Inspector / NPC Conversation / Station Desk Window
            BuildModalInspector(_hudRoot.transform);

            // 7. Slice 1C: Visual Commodity Exchange, Dual Hold Capacity Bars & Sailing Plan Builder
            BuildTradeAndSailingTerminal(_hudRoot.transform);
        }

        private void BuildHelmFlightDeckBar(Transform parent)
        {
            _helmFlightBar = CreatePanel(parent, "HelmFlightDeckBar", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 70f), new Vector2(-16f, 118f), new Color(0.03f, 0.09f, 0.15f, 0.90f));
            _helmStatusLabel = CreateText(_helmFlightBar.transform, "TORCH 0% | EMCON STANDARD", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.92f, 1f));
            SetRect(_helmStatusLabel.rectTransform, new Vector2(0f, 0f), new Vector2(0.24f, 1f), new Vector2(12f, 0f), Vector2.zero);

            var controlsRow = CreatePanel(_helmFlightBar.transform, "HelmControlsRow", new Vector2(0.24f, 0f), new Vector2(1f, 1f), new Vector2(6f, 6f), new Vector2(-8f, -6f), Color.clear);
            var hlg = controlsRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 6f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            CreateButton(controlsRow.transform, "THRUST 0%", new Color(0.22f, 0.24f, 0.28f), () => OnCommandSubmitted?.Invoke("helm:thrust:0"));
            CreateButton(controlsRow.transform, "35%", new Color(0.14f, 0.36f, 0.48f), () => OnCommandSubmitted?.Invoke("helm:thrust:350"));
            CreateButton(controlsRow.transform, "70%", new Color(0.16f, 0.46f, 0.56f), () => OnCommandSubmitted?.Invoke("helm:thrust:700"));
            CreateButton(controlsRow.transform, "100%", new Color(0.58f, 0.34f, 0.12f), () => OnCommandSubmitted?.Invoke("helm:thrust:1000"));
            CreateButton(controlsRow.transform, "GHOST", new Color(0.18f, 0.28f, 0.42f), () => OnCommandSubmitted?.Invoke("+helm/emcon ghost"));
            CreateButton(controlsRow.transform, "STD", new Color(0.16f, 0.34f, 0.38f), () => OnCommandSubmitted?.Invoke("+helm/emcon standard"));
            CreateButton(controlsRow.transform, "SCAN", new Color(0.16f, 0.44f, 0.34f), () => OnCommandSubmitted?.Invoke("+tac/scan"));

            _helmFlightBar.SetActive(false);
        }

        private void BuildTradeAndSailingTerminal(Transform parent)
        {
            _tradePanel = CreatePanel(parent, "TradeAndSailingTerminal", new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-475f, -305f), new Vector2(475f, 305f), new Color(0.03f, 0.07f, 0.12f, 0.99f));

            _tradeHeaderText = CreateText(_tradePanel.transform, "CERES PORT EXCHANGE & SHIP LOGISTICS", 19, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.42f, 0.92f, 1f));
            SetRect(_tradeHeaderText.rectTransform, new Vector2(0f, 1f), new Vector2(0.62f, 1f), new Vector2(20f, -44f), new Vector2(0f, -8f));

            _tradeCreditsText = CreateText(_tradePanel.transform, "CREDITS: — cr", 16, FontStyle.Bold, TextAnchor.MiddleRight, new Color(0.45f, 1.0f, 0.72f));
            SetRect(_tradeCreditsText.rectTransform, new Vector2(0.62f, 1f), new Vector2(1f, 1f), new Vector2(0f, -44f), new Vector2(-132f, -8f));

            var closeBtn = CreateButton(_tradePanel.transform, "CLOSE [ESC]", new Color(0.45f, 0.16f, 0.18f), () => _tradePanel.SetActive(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -42f), new Vector2(-16f, -10f));

            // Dual Dray Hold Mass / Volume + Tank + Stores Gauges Strip
            var gaugesStrip = CreatePanel(_tradePanel.transform, "HoldGaugesStrip", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -106f), new Vector2(-16f, -48f), new Color(0.05f, 0.11f, 0.17f, 0.96f));
            BuildGaugeCell(gaugesStrip.transform, 0, "HOLD MASS", "0 / 100 t", new Color(0.22f, 0.72f, 0.95f), out _massBarFill, out _massBarLabel);
            BuildGaugeCell(gaugesStrip.transform, 1, "PACKED VOL", "0.0 / 120 m³", new Color(0.32f, 0.85f, 0.65f), out _volBarFill, out _volBarLabel);
            BuildGaugeCell(gaugesStrip.transform, 2, "REACTION TANK", "0 / 80 t", new Color(0.92f, 0.58f, 0.18f), out _tankBarFill, out _tankBarLabel);
            BuildGaugeCell(gaugesStrip.transform, 3, "CREW STORES", "0 / 120 d", new Color(0.68f, 0.55f, 0.95f), out _storesBarFill, out _storesBarLabel);

            // Mode Tabs: [1] Commodity Exchange & Chandlery | [2] Sailing Plan & Corridor Route
            var tabStrip = CreatePanel(_tradePanel.transform, "TerminalTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -146f), new Vector2(-16f, -112f), Color.clear);
            var exBtn = CreateButton(tabStrip.transform, "COMMODITY EXCHANGE & CHANDLERY", new Color(0.16f, 0.46f, 0.58f), () => SwitchTradeTab(sailingTab: false));
            SetRect(exBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0.5f, 1f), Vector2.zero, new Vector2(-4f, 0f));
            _exchangeTabBtnImg = exBtn.GetComponent<Image>();

            var sailBtn = CreateButton(tabStrip.transform, "SAILING PLAN & CORRIDOR ROUTE", new Color(0.12f, 0.24f, 0.34f), () => SwitchTradeTab(sailingTab: true));
            SetRect(sailBtn.GetComponent<RectTransform>(), new Vector2(0.5f, 0f), new Vector2(1f, 1f), new Vector2(4f, 0f), Vector2.zero);
            _sailingTabBtnImg = sailBtn.GetComponent<Image>();

            // ---- TAB 1: EXCHANGE & CHANDLERY ----
            _exchangeTabBody = CreatePanel(_tradePanel.transform, "ExchangeTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -154f), Color.clear);

            var qtyStrip = CreatePanel(_exchangeTabBody.transform, "QuantitySelectorBar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -38f), Vector2.zero, new Color(0.05f, 0.10f, 0.16f, 0.9f));
            var qLabel = CreateText(qtyStrip.transform, "LOT QUANTITY:", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.75f, 0.88f, 0.96f));
            SetRect(qLabel.rectTransform, new Vector2(0f, 0f), new Vector2(0f, 1f), new Vector2(12f, 0f), new Vector2(128f, 0f));

            int[] presets = { 5, 10, 25, 50, -1 };
            string[] presetLabels = { "5", "10", "25", "50", "MAX FIT / ALL" };
            for (int i = 0; i < presets.Length; i++)
            {
                int capturedIdx = i;
                int val = presets[i];
                float x0 = 132f + i * 116f;
                float w = (i == 4) ? 148f : 108f;
                var qb = CreateButton(qtyStrip.transform, presetLabels[i], new Color(0.14f, 0.26f, 0.38f), () => SelectTradeQuantityPreset(capturedIdx, val));
                SetRect(qb.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0f, 1f), new Vector2(x0, 4f), new Vector2(x0 + w, -4f));
                _qtyPresetImages.Add(qb.GetComponent<Image>());
            }

            var refreshBtn = CreateButton(qtyStrip.transform, "REFRESH BOARD", new Color(0.15f, 0.42f, 0.48f), () =>
            {
                _boundTradeState?.NotifyMarketRefreshRequested();
                if (OnRefreshTradeRequested != null)
                {
                    OnRefreshTradeRequested.Invoke();
                }
                else
                {
                    OnCommandSubmitted?.Invoke("+market/list");
                    OnCommandSubmitted?.Invoke("+ship");
                    OnCommandSubmitted?.Invoke("+credit");
                }
            });
            SetRect(refreshBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-156f, 4f), new Vector2(-6f, -4f));

            var listViewport = CreatePanel(_exchangeTabBody.transform, "ListingsViewport", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(0f, 0f), new Vector2(0f, -46f), new Color(0.02f, 0.05f, 0.09f, 0.85f));
            listViewport.AddComponent<RectMask2D>();
            var listScroll = listViewport.AddComponent<ScrollRect>();

            var contentGo = new GameObject("ListingsContent", typeof(RectTransform), typeof(VerticalLayoutGroup), typeof(ContentSizeFitter));
            contentGo.transform.SetParent(listViewport.transform, false);
            var contentRt = contentGo.GetComponent<RectTransform>();
            contentRt.anchorMin = new Vector2(0f, 1f);
            contentRt.anchorMax = new Vector2(1f, 1f);
            contentRt.pivot = new Vector2(0.5f, 1f);
            contentRt.offsetMin = Vector2.zero;
            contentRt.offsetMax = Vector2.zero;

            var vlg = contentGo.GetComponent<VerticalLayoutGroup>();
            vlg.spacing = 6f;
            vlg.padding = new RectOffset(8, 8, 8, 8);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            var csf = contentGo.GetComponent<ContentSizeFitter>();
            csf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            csf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;

            listScroll.viewport = listViewport.GetComponent<RectTransform>();
            listScroll.content = contentRt;
            listScroll.horizontal = false;
            listScroll.vertical = true;
            listScroll.movementType = ScrollRect.MovementType.Clamped;
            listScroll.scrollSensitivity = 28f;
            _listingsContentRoot = contentGo.transform;

            // ---- TAB 2: SAILING PLAN & CORRIDOR ROUTE BUILDER ----
            _sailingTabBody = CreatePanel(_tradePanel.transform, "SailingTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -154f), Color.clear);

            var portStrip = CreatePanel(_sailingTabBody.transform, "CorridorPortBar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -40f), Vector2.zero, new Color(0.05f, 0.10f, 0.16f, 0.9f));
            var pLbl = CreateText(portStrip.transform, "CORRIDOR PORT:", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.75f, 0.88f, 0.96f));
            SetRect(pLbl.rectTransform, new Vector2(0f, 0f), new Vector2(0f, 1f), new Vector2(12f, 0f), new Vector2(142f, 0f));

            (string code, string label)[] ports =
            {
                ("stickney", "Stickney Yards (Phobos)"),
                ("aitken", "Aitken Highport (Luna)"),
                ("occator", "Occator Rim"),
                ("rheasilvia", "Rheasilvia (Vesta)")
            };
            for (int i = 0; i < ports.Length; i++)
            {
                int idx = i;
                string pCode = ports[i].code;
                float x0 = 148f + i * 190f;
                var pb = CreateButton(portStrip.transform, ports[i].label, new Color(0.14f, 0.26f, 0.38f), () => SelectDestinationPort(idx, pCode));
                SetRect(pb.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0f, 1f), new Vector2(x0, 4f), new Vector2(x0 + 184f, -4f));
                _destPortImages.Add(pb.GetComponent<Image>());
            }

            // 2 rows of 1-click Captain's Standing Order & Passage Actions (exact crates/tacsim/src/port/orders.rs syntax)
            var row1 = CreatePanel(_sailingTabBody.transform, "OrderRow1", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -88f), new Vector2(0f, -46f), Color.clear);
            var r1Hlg = row1.AddComponent<HorizontalLayoutGroup>();
            r1Hlg.spacing = 8f; r1Hlg.childControlWidth = true; r1Hlg.childControlHeight = true; r1Hlg.childForceExpandWidth = true;

            CreateButton(row1.transform, "Dry-Run +ship/plan", new Color(0.14f, 0.42f, 0.54f), () =>
                OnCommandSubmitted?.Invoke($"+ship/plan {(_boundTradeState != null ? _boundTradeState.SelectedDestinationPort : "stickney")}"));
            CreateButton(row1.transform, "+ Bunker Fuel (fill)", new Color(0.48f, 0.32f, 0.12f), () =>
                OnCommandSubmitted?.Invoke("+ship/order buy propellant fill"));
            CreateButton(row1.transform, "+ Provisions (fill)", new Color(0.32f, 0.26f, 0.48f), () =>
                OnCommandSubmitted?.Invoke("+ship/order buy provisions fill"));
            CreateButton(row1.transform, "+ Buy Volatiles", new Color(0.14f, 0.44f, 0.36f), () =>
                OnCommandSubmitted?.Invoke($"+ship/order buy volatiles {GetOrderTradeQty()}"));
            CreateButton(row1.transform, "+ Buy Metals", new Color(0.14f, 0.44f, 0.36f), () =>
                OnCommandSubmitted?.Invoke($"+ship/order buy metals {GetOrderTradeQty()}"));

            var row2 = CreatePanel(_sailingTabBody.transform, "OrderRow2", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -136f), new Vector2(0f, -94f), Color.clear);
            var r2Hlg = row2.AddComponent<HorizontalLayoutGroup>();
            r2Hlg.spacing = 8f; r2Hlg.childControlWidth = true; r2Hlg.childControlHeight = true; r2Hlg.childForceExpandWidth = true;

            CreateButton(row2.transform, "+ Sell Volatiles (all)", new Color(0.46f, 0.34f, 0.14f), () =>
                OnCommandSubmitted?.Invoke("+ship/order sell volatiles all"));
            CreateButton(row2.transform, "+ Sell Alloys (all)", new Color(0.46f, 0.34f, 0.14f), () =>
                OnCommandSubmitted?.Invoke("+ship/order sell structural_alloys all"));
            CreateButton(row2.transform, "+ Queue Depart", new Color(0.16f, 0.50f, 0.42f), () =>
                OnCommandSubmitted?.Invoke($"+ship/order depart {(_boundTradeState != null ? _boundTradeState.SelectedDestinationPort : "stickney")}"));
            CreateButton(row2.transform, "View +ship/orders", new Color(0.16f, 0.32f, 0.44f), () =>
                OnCommandSubmitted?.Invoke("+ship/orders"));
            CreateButton(row2.transform, "Clear Queue", new Color(0.44f, 0.18f, 0.20f), () =>
                OnCommandSubmitted?.Invoke("+ship/order clear"));

            var sailViewport = CreatePanel(_sailingTabBody.transform, "SailingOutputViewport", new Vector2(0f, 0f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, -144f), new Color(0.02f, 0.05f, 0.09f, 0.90f));
            sailViewport.AddComponent<RectMask2D>();
            var sailScroll = sailViewport.AddComponent<ScrollRect>();
            _sailingPlanOutputText = CreateText(sailViewport.transform, "Select a corridor destination above to dry-run burn/stores (+ship/plan) or queue automated captain's orders (+ship/order).", 14, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 1f));
            _sailingPlanOutputText.supportRichText = true;
            _sailingPlanOutputText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _sailingPlanOutputText.verticalOverflow = VerticalWrapMode.Overflow;
            var sFitter = _sailingPlanOutputText.gameObject.AddComponent<ContentSizeFitter>();
            sFitter.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            sFitter.verticalFit = ContentSizeFitter.FitMode.PreferredSize;
            _sailingPlanOutputText.rectTransform.anchorMin = new Vector2(0f, 1f);
            _sailingPlanOutputText.rectTransform.anchorMax = new Vector2(1f, 1f);
            _sailingPlanOutputText.rectTransform.pivot = new Vector2(0.5f, 1f);
            _sailingPlanOutputText.rectTransform.offsetMin = new Vector2(12f, 0f);
            _sailingPlanOutputText.rectTransform.offsetMax = new Vector2(-12f, 0f);
            sailScroll.viewport = sailViewport.GetComponent<RectTransform>();
            sailScroll.content = _sailingPlanOutputText.rectTransform;
            sailScroll.horizontal = false;
            sailScroll.vertical = true;
            sailScroll.movementType = ScrollRect.MovementType.Clamped;

            _sailingTabBody.SetActive(false);
            _tradePanel.SetActive(false);
        }

        private void BuildGaugeCell(Transform strip, int index, string title, string initialVal, Color barColor, out Image fillImg, out Text valText)
        {
            float a0 = index * 0.25f;
            float a1 = (index + 1) * 0.25f;
            var cell = CreatePanel(strip, $"Gauge_{title}", new Vector2(a0, 0f), new Vector2(a1, 1f), new Vector2(8f, 6f), new Vector2(-8f, -6f), Color.clear);

            var hdr = CreateText(cell.transform, title, 11, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.68f, 0.82f, 0.92f));
            SetRect(hdr.rectTransform, new Vector2(0f, 0.5f), new Vector2(0.52f, 1f), Vector2.zero, Vector2.zero);

            valText = CreateText(cell.transform, initialVal, 12, FontStyle.Bold, TextAnchor.UpperRight, Color.white);
            SetRect(valText.rectTransform, new Vector2(0.48f, 0.5f), new Vector2(1f, 1f), Vector2.zero, Vector2.zero);

            var track = CreatePanel(cell.transform, "BarTrack", new Vector2(0f, 0f), new Vector2(1f, 0.42f), new Vector2(0f, 2f), new Vector2(0f, -2f), new Color(0.08f, 0.14f, 0.20f, 1f));
            var fillGo = CreatePanel(track.transform, "BarFill", new Vector2(0f, 0f), new Vector2(0f, 1f), Vector2.zero, Vector2.zero, barColor);
            fillImg = fillGo.GetComponent<Image>();
        }

        private void BuildDockingHudOverlay(Transform parent)
        {
            _dockingPanel = CreatePanel(parent, "DockingHud", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-460f, -380f), new Vector2(-16f, -76f), new Color(0.03f, 0.08f, 0.13f, 0.92f));
            var hdr = CreateText(_dockingPanel.transform, "60 Hz 6-DOF BOW COLLAR ALIGNMENT", 16, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.35f, 0.95f, 0.78f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(14f, -34f), new Vector2(-14f, -8f));

            string[] names = { "LAT", "ATT", "ROLL", "CLOS", "RNG" };
            for (int i = 0; i < 5; i++)
            {
                float x0 = 14f + i * 84f;
                var pill = CreatePanel(_dockingPanel.transform, $"Gate_{i}", new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(x0, -72f), new Vector2(x0 + 78f, -40f), new Color(0.45f, 0.28f, 0.08f, 0.95f));
                _gatePills[i] = pill.GetComponent<Image>();
                _gateLabels[i] = CreateText(pill.transform, names[i], 13, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
                SetRect(_gateLabels[i].rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
            }

            _dockingTelemetryText = CreateText(_dockingPanel.transform, "", 14, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.95f, 1.0f));
            _dockingTelemetryText.supportRichText = true;
            SetRect(_dockingTelemetryText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(14f, 10f), new Vector2(-14f, -82f));
            _dockingPanel.SetActive(false);
        }

        private void BuildModalInspector(Transform parent)
        {
            _modalPanel = CreatePanel(parent, "ModalInspector", new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-390f, -255f), new Vector2(390f, 265f), new Color(0.04f, 0.08f, 0.13f, 0.99f));
            _modalHeaderText = CreateText(_modalPanel.transform, "TERMINAL", 20, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.42f, 0.90f, 1.0f));
            SetRect(_modalHeaderText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -46f), new Vector2(-120f, -10f));

            var closeBtn = CreateButton(_modalPanel.transform, "CLOSE [ESC]", new Color(0.45f, 0.16f, 0.18f), () => _modalPanel.SetActive(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-115f, -44f), new Vector2(-14f, -10f));

            var modalViewport = CreatePanel(_modalPanel.transform, "ModalViewport", new Vector2(0f, 0.28f), new Vector2(1f, 1f), new Vector2(20f, 8f), new Vector2(-20f, -54f), new Color(0f, 0f, 0f, 0.02f));
            modalViewport.AddComponent<RectMask2D>();
            _modalScrollRect = modalViewport.AddComponent<ScrollRect>();

            _modalBodyText = CreateText(modalViewport.transform, "", 14, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.93f, 0.97f));
            _modalBodyText.supportRichText = true;
            _modalBodyText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _modalBodyText.verticalOverflow = VerticalWrapMode.Overflow;
            var modalFitter = _modalBodyText.gameObject.AddComponent<ContentSizeFitter>();
            modalFitter.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            modalFitter.verticalFit = ContentSizeFitter.FitMode.PreferredSize;
            _modalBodyText.rectTransform.anchorMin = new Vector2(0f, 1f);
            _modalBodyText.rectTransform.anchorMax = new Vector2(1f, 1f);
            _modalBodyText.rectTransform.pivot = new Vector2(0.5f, 1f);
            _modalBodyText.rectTransform.offsetMin = Vector2.zero;
            _modalBodyText.rectTransform.offsetMax = Vector2.zero;

            _modalScrollRect.viewport = modalViewport.GetComponent<RectTransform>();
            _modalScrollRect.content = _modalBodyText.rectTransform;
            _modalScrollRect.horizontal = false;
            _modalScrollRect.vertical = true;
            _modalScrollRect.movementType = ScrollRect.MovementType.Clamped;
            _modalScrollRect.scrollSensitivity = 28f;

            _npcSayInput = CreateInputField(_modalPanel.transform, "Speak directly to NPC (Enter)...", new Vector2(0f, 0.18f), new Vector2(1f, 0.27f), new Vector2(20f, 0f), new Vector2(-20f, 0f));
            _npcSayInput.onSubmit.AddListener(_ => SubmitNpcInput());
            _npcSayInput.onValueChanged.AddListener(val => HandleInputComposingEdited(val, isNpcSayBox: true));

            var btnBar = CreatePanel(_modalPanel.transform, "ModalActionButtons", new Vector2(0f, 0f), new Vector2(1f, 0.16f), new Vector2(16f, 12f), new Vector2(-16f, -2f), Color.clear);
            var hlg = btnBar.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 10f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;
            _modalButtonsRoot = btnBar.transform;

            _modalPanel.SetActive(false);
        }

        private void BuildLoginAndAvatarOverlay(Transform root)
        {
            _loginPanel = CreatePanel(root, "LoginOverlay", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.02f, 0.04f, 0.07f, 0.90f));
            var card = CreatePanel(_loginPanel.transform, "LoginCard", new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-350f, -218f), new Vector2(350f, 218f), new Color(0.06f, 0.10f, 0.16f, 0.98f));

            var title = CreateText(card.transform, "RUSTMUSH // TACSIM 2350 — 3D CLIENT", 22, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.45f, 0.90f, 1f));
            SetRect(title.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -50f), new Vector2(-20f, -10f));

            _credentialsGroup = CreatePanel(card.transform, "CredentialsGroup", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, Color.clear);

            string savedUrl = Application.isPlaying ? PlayerPrefs.GetString(PrefLastUrl, "https://dev1.conclav.io") : "https://dev1.conclav.io";
            string savedAcct = Application.isPlaying ? PlayerPrefs.GetString(PrefLastAccount, "nick") : "nick";

            _urlInput = CreateInputField(_credentialsGroup.transform, "Server URL (e.g. https://dev1.conclav.io)", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(28f, -98f), new Vector2(-28f, -58f));
            _urlInput.text = savedUrl;

            _accountInput = CreateInputField(_credentialsGroup.transform, "Account Name", new Vector2(0f, 1f), new Vector2(0.5f, 1f), new Vector2(28f, -146f), new Vector2(-8f, -106f));
            _accountInput.text = savedAcct;

            _passwordInput = CreateInputField(_credentialsGroup.transform, "Password", new Vector2(0.5f, 1f), new Vector2(1f, 1f), new Vector2(8f, -146f), new Vector2(-28f, -106f));
            _passwordInput.contentType = InputField.ContentType.Password;
            _passwordInput.text = "";

            _charNameInput = CreateInputField(_credentialsGroup.transform, "Character Name (optional for Login; required for Register)", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(28f, -194f), new Vector2(-28f, -154f));
            _charNameInput.text = "";

            var avHdr = CreateText(_credentialsGroup.transform, "SELECT 3RD-PERSON PLAYER AVATAR (IDA FABER RIGS)", 14, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(avHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(28f, -230f), new Vector2(-28f, -204f));

            string[] avNames = { "Trooper (M)", "Trooper (F)", "Street Fighter (M)", "Street Fighter (F)" };
            for (int i = 0; i < 4; i++)
            {
                int captured = i;
                float xMin = 28f + i * 162f;
                var btn = CreateButton(_credentialsGroup.transform, avNames[i], new Color(0.14f, 0.25f, 0.36f), () => SelectAvatar((PlayerAvatarPreset)captured));
                SetRect(btn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xMin, -278f), new Vector2(xMin + 154f, -236f));
                _avatarButtons.Add(btn.GetComponent<Image>());
            }
            SelectAvatar(PlayerAvatarPreset.StellarTrooperFemale);

            var loginBtn = CreateButton(_credentialsGroup.transform, "LOGIN TO MUSHD", new Color(0.14f, 0.52f, 0.40f), () => SubmitAuth(isRegister: false));
            SetRect(loginBtn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0.5f, 1f), new Vector2(28f, -342f), new Vector2(-8f, -294f));

            var regBtn = CreateButton(_credentialsGroup.transform, "REGISTER ACCOUNT", new Color(0.16f, 0.40f, 0.62f), () => SubmitAuth(isRegister: true));
            SetRect(regBtn.GetComponent<RectTransform>(), new Vector2(0.5f, 1f), new Vector2(1f, 1f), new Vector2(8f, -342f), new Vector2(-28f, -294f));

            // Character selection view (shown if an account has multiple characters)
            _charSelectGroup = CreatePanel(card.transform, "CharacterSelectGroup", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, Color.clear);
            var csHdr = CreateText(_charSelectGroup.transform, "SELECT CHARACTER TO ATTACH", 17, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.75f, 0.92f, 1f));
            SetRect(csHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(28f, -92f), new Vector2(-28f, -58f));

            var listPanel = CreatePanel(_charSelectGroup.transform, "CharacterButtons", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(40f, -292f), new Vector2(-40f, -100f), Color.clear);
            var vlg = listPanel.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 10f;
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;
            _charSelectListRoot = listPanel.transform;

            var backBtn = CreateButton(_charSelectGroup.transform, "<- BACK TO LOGIN", new Color(0.28f, 0.22f, 0.26f), () =>
            {
                _charSelectGroup.SetActive(false);
                _credentialsGroup.SetActive(true);
            });
            SetRect(backBtn.GetComponent<RectTransform>(), new Vector2(0.25f, 1f), new Vector2(0.75f, 1f), new Vector2(0f, -342f), new Vector2(0f, -298f));
            _charSelectGroup.SetActive(false);

            _loginStatusText = CreateText(card.transform, "", 14, FontStyle.Normal, TextAnchor.MiddleCenter, new Color(0.95f, 0.85f, 0.55f));
            _loginStatusText.supportRichText = true;
            SetRect(_loginStatusText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(28f, 10f), new Vector2(-28f, -352f));
        }

        private void SubmitAuth(bool isRegister)
        {
            string url = _urlInput != null ? _urlInput.text.Trim() : "https://dev1.conclav.io";
            string acct = _accountInput != null ? _accountInput.text.Trim() : "";
            string pass = _passwordInput != null ? _passwordInput.text : "";
            string cname = _charNameInput != null ? _charNameInput.text.Trim() : "";

            if (Application.isPlaying)
            {
                PlayerPrefs.SetString(PrefLastUrl, url);
                PlayerPrefs.SetString(PrefLastAccount, acct);
                PlayerPrefs.Save();
            }

            OnAuthSubmitted?.Invoke(url, acct, pass, cname, isRegister, _selectedAvatar);
        }

        public void ShowCharacterPicker(MushAuthSession session)
        {
            if (_credentialsGroup == null || _charSelectGroup == null) return;
            _credentialsGroup.SetActive(false);
            _charSelectGroup.SetActive(true);

            for (int i = _charSelectListRoot.childCount - 1; i >= 0; i--)
                DestroySafe(_charSelectListRoot.GetChild(i).gameObject);

            foreach (var slot in session.Characters)
            {
                var captured = slot;
                var btn = CreateButton(_charSelectListRoot, $"ENTER WORLD AS  {captured.Name.ToUpperInvariant()}  (#{captured.Key})", new Color(0.14f, 0.46f, 0.42f), () =>
                    OnCharacterSelected?.Invoke(session, captured, _selectedAvatar));
                btn.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 46f);
            }

            SetLoginStatus($"Authenticated as <b>{session.Account}</b> ({session.Characters.Count} characters available).");
        }

        private void SelectAvatar(PlayerAvatarPreset preset)
        {
            _selectedAvatar = preset;
            for (int i = 0; i < _avatarButtons.Count; i++)
            {
                _avatarButtons[i].color = (i == (int)preset)
                    ? new Color(0.22f, 0.65f, 0.88f, 1f)
                    : new Color(0.14f, 0.22f, 0.30f, 0.92f);
            }
        }

        private void Update()
        {
            if (_composingActive && Time.unscaledTime >= _composingIdleAt)
            {
                SetComposingState(false);
            }

            if (Input.GetKeyDown(KeyCode.Escape))
            {
                if (_tradePanel != null && _tradePanel.activeSelf)
                    _tradePanel.SetActive(false);
                if (_modalPanel != null && _modalPanel.activeSelf)
                    _modalPanel.SetActive(false);
                if (Overlays != null && Overlays.IsLobbyModalOpen)
                    Overlays.ShowArenaLobbyModal(false);
            }

            if (Input.GetKeyDown(KeyCode.Return) || Input.GetKeyDown(KeyCode.KeypadEnter))
            {
                // Pressing Enter on Password or Account field submits Login directly!
                if (_loginPanel != null && _loginPanel.activeSelf && _credentialsGroup != null && _credentialsGroup.activeSelf)
                {
                    if ((_passwordInput != null && !string.IsNullOrEmpty(_passwordInput.text)) &&
                        (_accountInput != null && !string.IsNullOrWhiteSpace(_accountInput.text)))
                    {
                        SubmitAuth(isRegister: false);
                        return;
                    }
                }

                if (_modalPanel != null && _modalPanel.activeSelf && _npcSayInput != null && _npcSayInput.gameObject.activeSelf && !string.IsNullOrWhiteSpace(_npcSayInput.text))
                {
                    SubmitNpcInput();
                }
                else if (_chatInput != null && !string.IsNullOrWhiteSpace(_chatInput.text))
                {
                    SubmitChatInput();
                }
            }
        }

        private void HandleInputComposingEdited(string rawValue, bool isNpcSayBox)
        {
            if (isNpcSayBox && !string.IsNullOrEmpty(_modalCommandPrefix))
                return;

            string trimmed = rawValue != null ? rawValue.TrimStart() : string.Empty;
            bool shouldCompose = !string.IsNullOrEmpty(trimmed) &&
                (isNpcSayBox || (trimmed[0] != '+' && trimmed[0] != '@' && trimmed[0] != '/'));

            if (shouldCompose)
            {
                _composingIdleAt = Time.unscaledTime + 4.0f;
                SetComposingState(true);
            }
            else if (_composingActive)
            {
                SetComposingState(false);
            }
        }

        private void SetComposingState(bool active)
        {
            if (_composingActive == active) return;
            _composingActive = active;
            OnComposingChanged?.Invoke(active);
        }

        public string GetModalInputValue() => _npcSayInput != null ? _npcSayInput.text.Trim() : string.Empty;

        private void SubmitNpcInput()
        {
            if (_npcSayInput == null || string.IsNullOrWhiteSpace(_npcSayInput.text)) return;
            string spoken = _npcSayInput.text.Trim();
            SetComposingState(false);

            if (!string.IsNullOrEmpty(_modalCommandPrefix))
            {
                OnCommandSubmitted?.Invoke($"{_modalCommandPrefix} {spoken}");
                return;
            }

            _npcSayInput.text = string.Empty;
            if (!string.IsNullOrEmpty(_activeNpcTargetName))
                OnCommandSubmitted?.Invoke($"say {_activeNpcTargetName}, {spoken}");
            else
                OnCommandSubmitted?.Invoke($"say {spoken}");
        }

        private void SubmitChatInput()
        {
            if (_chatInput == null || string.IsNullOrWhiteSpace(_chatInput.text)) return;
            string line = _chatInput.text.Trim();
            _chatInput.text = string.Empty;
            SetComposingState(false);
            OnCommandSubmitted?.Invoke(line);
        }

        public void ShowLoginOverlay(bool visible, string statusMessage = null)
        {
            if (_loginPanel != null) _loginPanel.SetActive(visible);
            if (visible && _credentialsGroup != null && _charSelectGroup != null)
            {
                _credentialsGroup.SetActive(true);
                _charSelectGroup.SetActive(false);
            }
            if (_hudRoot != null) _hudRoot.SetActive(!visible);
            if (statusMessage != null && _loginStatusText != null)
                _loginStatusText.text = statusMessage;
        }

        public void SetLoginStatus(string statusMessage)
        {
            if (_loginStatusText != null) _loginStatusText.text = statusMessage;
        }

        public void SetConnectionBadge(MushWsConnectionState state)
        {
            if (_netBadgeText == null) return;
            switch (state)
            {
                case MushWsConnectionState.Online:
                    _netBadgeText.text = "[ONLINE]";
                    _netBadgeText.color = new Color(0.36f, 0.96f, 0.68f);
                    break;
                case MushWsConnectionState.Connecting:
                    _netBadgeText.text = "[CONNECTING…]";
                    _netBadgeText.color = new Color(0.55f, 0.85f, 1.0f);
                    break;
                case MushWsConnectionState.Reconnecting:
                    _netBadgeText.text = "[RECONNECTING…]";
                    _netBadgeText.color = new Color(1.0f, 0.78f, 0.35f);
                    break;
                case MushWsConnectionState.SessionEnded:
                    _netBadgeText.text = "[SESSION ENDED]";
                    _netBadgeText.color = new Color(1.0f, 0.45f, 0.42f);
                    break;
                default:
                    _netBadgeText.text = "[DISCONNECTED]";
                    _netBadgeText.color = new Color(0.62f, 0.72f, 0.82f);
                    break;
            }
        }

        public void UpdateTopBar(string locationLabel, string clockAndHullLabel)
        {
            if (_locationText != null) _locationText.text = locationLabel;
            if (_shipAndClockText != null) _shipAndClockText.text = clockAndHullLabel;
        }

        public void SetFocusedInteractable(WorldInteractable wi)
        {
            if (_interactBanner == null) return;
            if (wi == null)
            {
                _interactBanner.SetActive(false);
                return;
            }
            _interactBanner.SetActive(true);
            _interactTitleText.text = $"[E]  {wi.promptTitle}";
            _interactSubText.text = wi.promptSubtitle ?? string.Empty;
        }

        public void AppendLogLine(string richLine)
        {
            if (string.IsNullOrWhiteSpace(richLine)) return;
            _logLines.Enqueue(richLine);
            while (_logLines.Count > MaxLogLines) _logLines.Dequeue();
            if (_commsLogText != null)
            {
                _commsLogText.text = string.Join("\n", _logLines);
                LayoutRebuilder.ForceRebuildLayoutImmediate(_commsLogText.rectTransform);
                if (_commsScrollRect != null)
                    _commsScrollRect.verticalNormalizedPosition = 0f;
            }
        }

        private string _lastQuickActionKey;
        private bool? _lastQuickActionSeated;

        private static void DestroySafe(UnityEngine.Object obj)
        {
            if (obj == null) return;
            if (Application.isPlaying) Destroy(obj);
            else DestroyImmediate(obj);
        }

        public bool IsWalkQuickActionsVisible => _quickActionsStripPanel != null && _quickActionsStripPanel.activeSelf;

        public void SetWalkQuickActionsVisible(bool visible)
        {
            if (_quickActionsStripPanel != null && _quickActionsStripPanel.activeSelf != visible)
                _quickActionsStripPanel.SetActive(visible);
        }

        public void RefreshContextualQuickActions(string roomOrModeKey, bool isSeatedAtHelm)
        {
            if (_quickActionsContainer == null) return;
            string normalizedKey = roomOrModeKey ?? string.Empty;
            if (_lastQuickActionKey == normalizedKey && _lastQuickActionSeated == isSeatedAtHelm)
            {
                if (_helmFlightBar != null)
                    _helmFlightBar.SetActive(isSeatedAtHelm);
                Overlays?.SetHelmSeatedVisible(isSeatedAtHelm);
                return;
            }
            _lastQuickActionKey = normalizedKey;
            _lastQuickActionSeated = isSeatedAtHelm;

            for (int i = _quickActionsContainer.childCount - 1; i >= 0; i--)
                DestroySafe(_quickActionsContainer.GetChild(i).gameObject);

            if (_helmFlightBar != null)
                _helmFlightBar.SetActive(isSeatedAtHelm);
            Overlays?.SetHelmSeatedVisible(isSeatedAtHelm);

            if (isSeatedAtHelm)
            {
                AddQuickBtn("Stand Up", () => OnStandFromHelmRequested?.Invoke());
                AddQuickBtn("Bridge Holo", () => OnCameraModeRequested?.Invoke(CameraRigMode.ShipBridgeHolo));
                AddQuickBtn("Orbital View", () => OnCameraModeRequested?.Invoke(CameraRigMode.OrbitalExterior));
                AddQuickBtn("Sailing Plan", () => OnCommandSubmitted?.Invoke("ui:open_sailing"));
                AddQuickBtn("Undock Port", () => OnCommandSubmitted?.Invoke("action:undock_selected"));
                AddQuickBtn("Decouple 3D", () => OnCommandSubmitted?.Invoke("+helm/decouple"));
                AddQuickBtn("6-DOF Dock", () => OnCommandSubmitted?.Invoke("action:start_docking"));
                return;
            }

            switch (roomOrModeKey)
            {
                case "room.welcome_hall":
                    AddQuickBtn("Simulator Annexe", () => OnCommandSubmitted?.Invoke("simulator"));
                    AddQuickBtn("Tutorial East", () => OnCommandSubmitted?.Invoke("east"));
                    AddQuickBtn("OOC Lounge", () => OnCommandSubmitted?.Invoke("south"));
                    AddQuickBtn("Return IC (+ic)", () => OnCommandSubmitted?.Invoke("+ic"));
                    break;
                case "room.simulator_lobby":
                    AddQuickBtn("Simulator Console", () => Overlays?.ShowArenaLobbyModal(true));
                    AddQuickBtn("Red Bay (North)", () => OnCommandSubmitted?.Invoke("red"));
                    AddQuickBtn("Blue Bay (South)", () => OnCommandSubmitted?.Invoke("blue"));
                    AddQuickBtn("Sim Status", () => OnCommandSubmitted?.Invoke("sim status"));
                    AddQuickBtn("Return IC (+ic)", () => OnCommandSubmitted?.Invoke("+ic"));
                    break;
                case "room.sim_red":
                case "room.sim_blue":
                    AddQuickBtn("Board Sim Pod", () => OnCommandSubmitted?.Invoke("board"));
                    AddQuickBtn("Simulator Console", () => Overlays?.ShowArenaLobbyModal(true));
                    AddQuickBtn("Sim Status", () => OnCommandSubmitted?.Invoke("sim status"));
                    AddQuickBtn("Back to Lobby", () => OnCommandSubmitted?.Invoke("lobby"));
                    AddQuickBtn("Return IC (+ic)", () => OnCommandSubmitted?.Invoke("+ic"));
                    break;
                case "room.p11_service_counter":
                    AddQuickBtn("Refit Dray-P", () => OnCommandSubmitted?.Invoke("+coop/refit prospector"));
                    AddQuickBtn("Lease +tv Rig", () => OnCommandSubmitted?.Invoke("+tv/lease go"));
                    AddQuickBtn("Ship Status", () => OnCommandSubmitted?.Invoke("+ship"));
                    break;
                case "room.c05_registry":
                case "room.l05_registry":
                    AddQuickBtn("Co-op Terms", () => OnCommandSubmitted?.Invoke("+coop"));
                    AddQuickBtn("Enrol Charter Hull", () => OnCommandSubmitted?.Invoke("ui:open_coop_enrol"));
                    AddQuickBtn("Statement", () => OnCommandSubmitted?.Invoke("+coop/statement"));
                    break;
                case "room.c07_exchange":
                case "room.c08_chandlery":
                case "room.p08_exchange":
                case "room.p09_commissary":
                case "room.l07_exchange":
                    AddQuickBtn("Visual Trade Board", () => OnCommandSubmitted?.Invoke("ui:open_exchange"));
                    AddQuickBtn("Credits", () => OnCommandSubmitted?.Invoke("+credit"));
                    AddQuickBtn("Ship Hold", () => OnCommandSubmitted?.Invoke("+ship/hold"));
                    break;
                case "room.c14_dockmaster":
                case "room.p05_harbourmaster":
                case "room.l12_dockmaster":
                    AddQuickBtn("Sailing & Route Plan", () => OnCommandSubmitted?.Invoke("ui:open_sailing"));
                    AddQuickBtn("Visual Trade Board", () => OnCommandSubmitted?.Invoke("ui:open_exchange"));
                    AddQuickBtn("Ship Status", () => OnCommandSubmitted?.Invoke("+ship"));
                    break;
                case "room.c01_berth_gallery":
                case "room.p01_berth_gallery":
                case "room.l01_berth_gallery":
                    AddQuickBtn("Board Ship", () => OnCommandSubmitted?.Invoke("+ship/board"));
                    AddQuickBtn("Ship Status", () => OnCommandSubmitted?.Invoke("+ship"));
                    AddQuickBtn("Sailing Plan", () => OnCommandSubmitted?.Invoke("ui:open_sailing"));
                    break;
                case "interior:airlock":
                    AddQuickBtn("Cycle Ashore", () => OnCommandSubmitted?.Invoke("+ship/leave"));
                    AddQuickBtn("Ship Status", () => OnCommandSubmitted?.Invoke("+ship"));
                    break;
                case "interior:hold":
                    AddQuickBtn("Hold Load Cells", () => OnCommandSubmitted?.Invoke("+ship/hold"));
                    AddQuickBtn("Trade Summary", () => OnCommandSubmitted?.Invoke("ui:open_exchange"));
                    AddQuickBtn("Vehicle Bay", () => OnCommandSubmitted?.Invoke("+tv/bay in"));
                    break;
                case "interior:bay":
                    AddQuickBtn("Launch Rig 1", () => OnCommandSubmitted?.Invoke("surface:launch"));
                    AddQuickBtn("Lease Rig", () => OnCommandSubmitted?.Invoke("+tv/lease go"));
                    AddQuickBtn("Back to Hold", () => OnCommandSubmitted?.Invoke("+tv/bay out"));
                    break;
                case "interior:bridge":
                    AddQuickBtn("Take Helm", () => OnCommandSubmitted?.Invoke("seat:helm"));
                    AddQuickBtn("Sailing & Route Plan", () => OnCommandSubmitted?.Invoke("ui:open_sailing"));
                    AddQuickBtn("Dry-Run +ship/plan", () => OnCommandSubmitted?.Invoke("+ship/plan"));
                    break;
                case "interior:passage":
                case "interior:crew":
                case "interior:engineering":
                    AddQuickBtn("Look Around", () => OnCommandSubmitted?.Invoke("look"));
                    AddQuickBtn("Ship Status", () => OnCommandSubmitted?.Invoke("+ship"));
                    AddQuickBtn("Ship Hold", () => OnCommandSubmitted?.Invoke("+ship/hold"));
                    break;
                default:
                    AddQuickBtn("Look Around", () => OnCommandSubmitted?.Invoke("look"));
                    AddQuickBtn("Ship Status", () => OnCommandSubmitted?.Invoke("+ship"));
                    AddQuickBtn("Credits", () => OnCommandSubmitted?.Invoke("+credit"));
                    AddQuickBtn("OOC Portal (ooc)", () => OnCommandSubmitted?.Invoke("ooc"));
                    break;
            }
        }

        public void UpdateHelmStatus(float throttlePermille, string emconPosture)
        {
            if (_helmStatusLabel == null) return;
            int pct = Mathf.RoundToInt(Mathf.Clamp(throttlePermille, 0f, 1000f) * 0.1f);
            string em = string.IsNullOrEmpty(emconPosture) ? "STD" : emconPosture.ToUpperInvariant();
            _helmStatusLabel.text = $"TORCH {pct}% | EMCON {em}";
        }

        public void BindTradeState(TacSimTradeAndShipState state)
        {
            if (_boundTradeState != null)
                _boundTradeState.OnStateChanged -= RefreshTradeTerminal;
            _boundTradeState = state;
            if (_boundTradeState != null)
                _boundTradeState.OnStateChanged += RefreshTradeTerminal;
            SelectTradeQuantityPreset(1, 10);
            SelectDestinationPort(0, "stickney");
        }

        public void OpenTradeAndSailingModal(string header, bool openSailingTab)
        {
            if (_tradePanel == null) return;
            if (_modalPanel != null) _modalPanel.SetActive(false);
            _tradePanel.SetActive(true);
            if (_tradeHeaderText != null && !string.IsNullOrEmpty(header))
                _tradeHeaderText.text = header;
            SwitchTradeTab(openSailingTab);
            RefreshTradeTerminal();
        }

        private void SwitchTradeTab(bool sailingTab)
        {
            _tradeSailingTabActive = sailingTab;
            if (_exchangeTabBody != null) _exchangeTabBody.SetActive(!sailingTab);
            if (_sailingTabBody != null) _sailingTabBody.SetActive(sailingTab);
            if (_exchangeTabBtnImg != null)
                _exchangeTabBtnImg.color = !sailingTab ? new Color(0.20f, 0.58f, 0.74f, 1f) : new Color(0.11f, 0.22f, 0.32f, 0.9f);
            if (_sailingTabBtnImg != null)
                _sailingTabBtnImg.color = sailingTab ? new Color(0.20f, 0.58f, 0.74f, 1f) : new Color(0.11f, 0.22f, 0.32f, 0.9f);
            RefreshTradeTerminal();
        }

        private void SelectTradeQuantityPreset(int index, int val)
        {
            if (_boundTradeState != null)
            {
                if (val <= 0)
                {
                    _boundTradeState.SelectedSellAll = true;
                }
                else
                {
                    _boundTradeState.SelectedSellAll = false;
                    _boundTradeState.SelectedTradeQuantity = val;
                }
            }
            for (int i = 0; i < _qtyPresetImages.Count; i++)
            {
                _qtyPresetImages[i].color = (i == index)
                    ? new Color(0.22f, 0.66f, 0.88f, 1f)
                    : new Color(0.13f, 0.24f, 0.35f, 0.92f);
            }
            if (_tradePanel != null && _tradePanel.activeSelf)
                RebuildListingCards();
        }

        private void SelectDestinationPort(int index, string portCode)
        {
            if (_boundTradeState != null)
                _boundTradeState.SelectedDestinationPort = portCode;
            for (int i = 0; i < _destPortImages.Count; i++)
            {
                _destPortImages[i].color = (i == index)
                    ? new Color(0.22f, 0.66f, 0.88f, 1f)
                    : new Color(0.13f, 0.24f, 0.35f, 0.92f);
            }
        }

        private int GetOrderTradeQty()
        {
            if (_boundTradeState == null) return 25;
            return _boundTradeState.SelectedSellAll ? 50 : Mathf.Max(5, _boundTradeState.SelectedTradeQuantity);
        }

        public void RefreshTradeTerminal()
        {
            if (_boundTradeState == null || _tradePanel == null || !_tradePanel.activeSelf)
                return;

            if (_tradeCreditsText != null)
                _tradeCreditsText.text = $"CREDITS: {_boundTradeState.CreditsText}";

            UpdateGaugeBar(_massBarFill, _massBarLabel,
                _boundTradeState.HoldMassTonnes / (float)Mathf.Max(1, _boundTradeState.HoldMassLimitTonnes),
                $"{_boundTradeState.HoldMassTonnes} / {_boundTradeState.HoldMassLimitTonnes} t",
                new Color(0.22f, 0.72f, 0.95f));

            UpdateGaugeBar(_volBarFill, _volBarLabel,
                _boundTradeState.HoldVolumeM3 / Mathf.Max(1f, _boundTradeState.HoldVolumeLimitM3),
                $"{_boundTradeState.HoldVolumeM3:0.0} / {_boundTradeState.HoldVolumeLimitM3:0} m³",
                new Color(0.32f, 0.85f, 0.65f));

            UpdateGaugeBar(_tankBarFill, _tankBarLabel,
                _boundTradeState.TankTonnes / (float)Mathf.Max(1, _boundTradeState.TankLimitTonnes),
                $"{_boundTradeState.TankTonnes} / {_boundTradeState.TankLimitTonnes} t",
                new Color(0.92f, 0.58f, 0.18f));

            UpdateGaugeBar(_storesBarFill, _storesBarLabel,
                _boundTradeState.StoresCrewDays / (float)Mathf.Max(1, _boundTradeState.StoresLimitCrewDays),
                $"{_boundTradeState.StoresCrewDays} / {_boundTradeState.StoresLimitCrewDays} d",
                new Color(0.68f, 0.55f, 0.95f));

            if (!_tradeSailingTabActive)
                RebuildListingCards();
            else
                RefreshSailingOutputBox();
        }

        private static void UpdateGaugeBar(Image fill, Text label, float ratio, string text, Color normalColor)
        {
            float clamped = Mathf.Clamp01(ratio);
            if (fill != null)
            {
                fill.rectTransform.anchorMax = new Vector2(clamped, 1f);
                fill.color = clamped >= 0.99f ? new Color(0.95f, 0.28f, 0.24f)
                           : clamped >= 0.80f ? new Color(0.95f, 0.72f, 0.20f)
                           : normalColor;
            }
            if (label != null) label.text = text;
        }

        private void RebuildListingCards()
        {
            if (_listingsContentRoot == null || _boundTradeState == null) return;
            for (int i = _listingsContentRoot.childCount - 1; i >= 0; i--)
                DestroySafe(_listingsContentRoot.GetChild(i).gameObject);

            if (_boundTradeState.Listings.Count == 0)
            {
                var emptyRow = CreatePanel(_listingsContentRoot, "EmptyNotice", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.06f, 0.11f, 0.17f, 0.9f));
                emptyRow.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 58f);
                string safePortHint = MushProtocol.EscapeUnityRichText(_boundTradeState.AboardDockedPortHint);
                string emptyMsg = !string.IsNullOrEmpty(safePortHint)
                    ? $"You are aboard your hull at <b>{safePortHint}</b> — step ashore (<b>+ship/leave</b>) to trade on the port exchange."
                    : "No cached listings yet — visit a port exchange/chandlery or click <b>REFRESH BOARD</b>.";
                var msg = CreateText(emptyRow.transform, emptyMsg, 14, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.75f, 0.88f, 0.98f));
                msg.supportRichText = true;
                SetRect(msg.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 0f), new Vector2(-12f, 0f));
                return;
            }

            for (int i = 0; i < _boundTradeState.Listings.Count; i++)
            {
                var row = _boundTradeState.Listings[i];
                var good = _boundTradeState.FindGoodByNameOrCode(row.GoodCode);

                var card = CreatePanel(_listingsContentRoot, $"Listing_{row.ListingId}", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero,
                    row.IsWantedBid ? new Color(0.13f, 0.10f, 0.06f, 0.94f) : new Color(0.06f, 0.12f, 0.18f, 0.94f));
                card.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 48f);

                string badge = row.IsWantedBid ? $"<color=#FFBB55>[BID #{row.ListingId}]</color>" : $"<color=#55DDFF>[ASK #{row.ListingId}]</color>";
                string measureInfo = string.Empty;
                if (good != null && good.Stowage == "stow.bulk")
                    measureInfo = $" <color=#7799B5>({good.VolumeLPerUnit / 1000f:0.1} m³/t)</color>";
                else if (good != null && good.Stowage == "stow.propellant")
                    measureInfo = " <color=#DDA055>(tank)</color>";
                else if (good != null && good.Stowage == "stow.provisions")
                    measureInfo = " <color=#B599EE>(stores)</color>";

                string rawVendor = string.IsNullOrEmpty(row.VendorName) ? row.Venue : row.VendorName;
                string vendorClause = MushProtocol.EscapeUnityRichText(rawVendor);
                string safeItem = MushProtocol.EscapeUnityRichText((row.ItemName ?? string.Empty).ToUpperInvariant());
                string safePrice = MushProtocol.EscapeUnityRichText(row.UnitPriceText);
                string desc = $"{badge}  <b>{safeItem}</b>{measureInfo}   —   <b>{row.Quantity}</b> × <color=#77FFBB>{safePrice}</color>   <color=#99B8CC>[{vendorClause}]</color>";
                var txt = CreateText(card.transform, desc, 13, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
                txt.supportRichText = true;
                SetRect(txt.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(12f, 2f), new Vector2(-190f, -2f));

                if (!row.IsWantedBid)
                {
                    int buyQty = _boundTradeState.SelectedSellAll
                        ? Mathf.Min(row.Quantity, _boundTradeState.ComputeMaxAdditionalUnitsForHold(row.GoodCode))
                        : Mathf.Min(Mathf.Max(1, row.Quantity), _boundTradeState.SelectedTradeQuantity);
                    buyQty = Mathf.Max(1, buyQty);

                    var capturedRow = row;
                    int capturedQty = buyQty;
                    var buyBtn = CreateButton(card.transform, $"BUY {buyQty}", new Color(0.14f, 0.52f, 0.36f), () =>
                        OnMarketBuyRequested?.Invoke(capturedRow, capturedQty, _boundTradeState.SelectedSellAll));
                    SetRect(buyBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-178f, 6f), new Vector2(-10f, -6f));
                }
                else
                {
                    bool sellAll = _boundTradeState.SelectedSellAll;
                    int sellQty = Mathf.Max(1, _boundTradeState.SelectedTradeQuantity);
                    string btnLbl = !row.IsFunded ? "UNFUNDED" : (sellAll ? "SELL ALL" : $"SELL {sellQty}");
                    var capturedRow = row;
                    var sellBtn = CreateButton(card.transform, btnLbl, row.IsFunded ? new Color(0.56f, 0.36f, 0.12f) : new Color(0.25f, 0.25f, 0.28f), () =>
                    {
                        if (capturedRow.IsFunded)
                            OnMarketSellRequested?.Invoke(capturedRow, sellQty, sellAll);
                    });
                    sellBtn.interactable = row.IsFunded;
                    SetRect(sellBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-178f, 6f), new Vector2(-10f, -6f));
                }
            }

            if (_boundTradeState.NextAfterListingId.HasValue)
            {
                ulong afterId = _boundTradeState.NextAfterListingId.Value;
                var moreRow = CreatePanel(_listingsContentRoot, "LoadMoreRow", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.08f, 0.16f, 0.24f, 0.94f));
                moreRow.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 42f);
                var moreBtn = CreateButton(moreRow.transform, "LOAD MORE LISTINGS", new Color(0.16f, 0.42f, 0.58f), () =>
                    OnLoadMoreTradeRequested?.Invoke(afterId));
                SetRect(moreBtn.GetComponent<RectTransform>(), new Vector2(0.25f, 0f), new Vector2(0.75f, 1f), new Vector2(0f, 4f), new Vector2(0f, -4f));
            }

            LayoutRebuilder.ForceRebuildLayoutImmediate(_listingsContentRoot.GetComponent<RectTransform>());
        }

        private void RefreshSailingOutputBox()
        {
            if (_sailingPlanOutputText == null || _boundTradeState == null) return;
            var sb = new StringBuilder(512);
            sb.AppendFormat("<color=#66DDFF><b>SELECTED CORRIDOR DESTINATION:</b></color> {0}\n", _boundTradeState.SelectedDestinationPort.ToUpperInvariant());
            if (_boundTradeState.SailingOrderLines.Count > 0)
            {
                sb.AppendLine("<color=#88EEAA><b>QUEUED STANDING ORDERS:</b></color>");
                for (int i = 0; i < _boundTradeState.SailingOrderLines.Count; i++)
                    sb.AppendLine("  " + MushProtocol.EscapeUnityRichText(_boundTradeState.SailingOrderLines[i]));
            }
            if (_sailingEchoLines.Count > 0)
            {
                sb.AppendLine("\n<color=#AADDFF><b>PORT / HELM TELEMETRY:</b></color>");
                for (int i = 0; i < _sailingEchoLines.Count; i++)
                    sb.AppendLine(_sailingEchoLines[i]);
            }
            _sailingPlanOutputText.text = sb.ToString();
            LayoutRebuilder.ForceRebuildLayoutImmediate(_sailingPlanOutputText.rectTransform);
        }

        private void AddQuickBtn(string label, Action onClick)
        {
            CreateButton(_quickActionsContainer, label, new Color(0.11f, 0.24f, 0.34f, 0.95f), onClick);
        }

        public void ShowDetailModal(
            string header,
            string body,
            List<(string label, Action action)> actions = null,
            string npcName = null,
            string commandPrefix = null,
            string defaultInputValue = null)
        {
            if (_modalPanel == null) return;
            if (_tradePanel != null) _tradePanel.SetActive(false);
            _modalPanel.SetActive(true);
            _modalHeaderText.text = header;
            _modalBaseIntro = body ?? string.Empty;
            _modalResponseLines.Clear();
            _modalBodyText.text = _modalBaseIntro;
            LayoutRebuilder.ForceRebuildLayoutImmediate(_modalBodyText.rectTransform);
            if (_modalScrollRect != null)
                _modalScrollRect.verticalNormalizedPosition = 1f;
            _activeNpcTargetName = npcName;
            _modalCommandPrefix = commandPrefix;
            bool showInput = !string.IsNullOrEmpty(npcName) || !string.IsNullOrEmpty(commandPrefix);
            _npcSayInput.gameObject.SetActive(showInput);
            if (showInput)
                _npcSayInput.text = defaultInputValue ?? string.Empty;

            for (int i = _modalButtonsRoot.childCount - 1; i >= 0; i--)
                DestroySafe(_modalButtonsRoot.GetChild(i).gameObject);

            if (actions != null)
            {
                foreach (var item in actions)
                {
                    var act = item.action;
                    CreateButton(_modalButtonsRoot, item.label, new Color(0.14f, 0.38f, 0.48f), () => act?.Invoke());
                }
            }
        }

        public void AppendModalResponseLine(string richLine, bool resetPreviousResponses = false)
        {
            if (_tradePanel != null && _tradePanel.activeSelf)
            {
                if (resetPreviousResponses) _sailingEchoLines.Clear();
                if (!string.IsNullOrWhiteSpace(richLine))
                {
                    foreach (string sub in richLine.Split('\n'))
                    {
                        if (string.IsNullOrWhiteSpace(sub)) continue;
                        _sailingEchoLines.Add(sub.TrimEnd('\r'));
                    }
                    while (_sailingEchoLines.Count > 24)
                        _sailingEchoLines.RemoveAt(0);
                }
                if (_tradeSailingTabActive)
                    RefreshSailingOutputBox();
            }

            if (_modalPanel == null || !_modalPanel.activeSelf || _modalBodyText == null)
                return;
            if (resetPreviousResponses)
                _modalResponseLines.Clear();

            if (!string.IsNullOrWhiteSpace(richLine))
            {
                foreach (string sub in richLine.Split('\n'))
                {
                    if (string.IsNullOrWhiteSpace(sub)) continue;
                    _modalResponseLines.Add(sub.TrimEnd('\r'));
                }
                while (_modalResponseLines.Count > 30)
                    _modalResponseLines.RemoveAt(0);
            }

            if (_modalResponseLines.Count == 0)
            {
                _modalBodyText.text = _modalBaseIntro;
            }
            else
            {
                _modalBodyText.text = $"{_modalBaseIntro}\n\n{string.Join("\n", _modalResponseLines)}";
            }

            LayoutRebuilder.ForceRebuildLayoutImmediate(_modalBodyText.rectTransform);
            if (_modalScrollRect != null)
                _modalScrollRect.verticalNormalizedPosition = 0f;
        }

        public void ShowServerRequestModal(MushServerRequest req)
        {
            var btns = new List<(string, Action)>();
            foreach (var opt in req.Options)
            {
                string key = opt.Key;
                btns.Add((opt.Label, () =>
                {
                    _modalPanel.SetActive(false);
                    OnRequestChoiceSubmitted?.Invoke(req, key);
                }));
            }
            ShowDetailModal("PORT / SYSTEM REQUEST", req.Prompt, btns);
        }

        public void UpdateDockingHud(DockingClientSession session, bool assistOn)
        {
            if (_dockingPanel == null) return;
            if (session == null)
            {
                _dockingPanel.SetActive(false);
                return;
            }
            _dockingPanel.SetActive(true);
            var gates = session.EvaluateGates();
            bool[] ok = { gates.LateralOk, gates.AttitudeOk, gates.RollOk, gates.ClosureOk, gates.RangeOk };
            for (int i = 0; i < 5; i++)
            {
                _gatePills[i].color = ok[i]
                    ? new Color(0.12f, 0.62f, 0.34f, 0.95f)
                    : new Color(0.55f, 0.32f, 0.08f, 0.95f);
            }

            var sb = new StringBuilder(256);
            sb.AppendFormat("Target: <b>{0}</b>   [{1}/5 GATES GREEN]\n", session.TargetLabel, gates.GreenCount);
            sb.AppendFormat("Range Z: <b>{0} mm</b> (<= 500)   Closure: <b>{1} mm/s</b> (100..800)\n", session.DzMm, session.VzMmps);
            sb.AppendFormat("Lateral: X {0:+#;-#;0} mm ({1:+#;-#;0} mm/s) | Y {2:+#;-#;0} mm ({3:+#;-#;0} mm/s)\n",
                session.DxMm, session.VxMmps, session.DyMm, session.VyMmps);
            sb.AppendFormat("Attitude: P {0:0.0} deg | Y {1:0.0} deg | R {2:0.0} deg\n",
                session.PitchMdeg / 1000f, session.YawMdeg / 1000f, session.RollMdeg / 1000f);
            sb.AppendFormat("Dampeners [X]: <b>{0}</b>  |  Guidance Assist [G]: <b>{1}</b>\n",
                session.DampenersOn ? "COUPLED" : "FREE", assistOn ? "ENGAGED" : "MANUAL");
            sb.Append("<color=#88CCFF>[WASD] Trans  [R/F] Closure  [Arrows] Pitch/Yaw  [Q/E] Roll  [Bksp] Abort</color>");
            _dockingTelemetryText.text = sb.ToString();
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
            t.font = _uiFont;
            t.text = content;
            t.fontSize = size;
            t.fontStyle = style;
            t.alignment = align;
            t.color = color;
            t.raycastTarget = false;
            return t;
        }

        private Button CreateButton(Transform parent, string label, Color bg, Action onClick)
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
                highlightedColor = new Color(Mathf.Clamp01(bg.r * 1.38f + 0.06f), Mathf.Clamp01(bg.g * 1.38f + 0.06f), Mathf.Clamp01(bg.b * 1.38f + 0.06f), 1f),
                pressedColor = new Color(Mathf.Clamp01(bg.r * 0.65f), Mathf.Clamp01(bg.g * 0.65f), Mathf.Clamp01(bg.b * 0.65f), 1f),
                selectedColor = new Color(Mathf.Clamp01(bg.r * 1.25f + 0.04f), Mathf.Clamp01(bg.g * 1.25f + 0.04f), Mathf.Clamp01(bg.b * 1.25f + 0.04f), 1f),
                disabledColor = new Color(0.2f, 0.22f, 0.25f, 0.6f),
                colorMultiplier = 1f,
                fadeDuration = 0.06f
            };
            btn.onClick.AddListener(() => onClick?.Invoke());

            var txt = CreateText(go.transform, label, 13, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 2f), new Vector2(-6f, -2f));
            return btn;
        }

        private InputField CreateInputField(Transform parent, string placeholder, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax)
        {
            var go = CreatePanel(parent, "InputField", anchorMin, anchorMax, offsetMin, offsetMax, new Color(0.08f, 0.13f, 0.20f, 0.95f));
            var input = go.AddComponent<InputField>();

            var ph = CreateText(go.transform, placeholder, 13, FontStyle.Italic, TextAnchor.MiddleLeft, new Color(0.5f, 0.62f, 0.72f));
            SetRect(ph.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 2f), new Vector2(-10f, -2f));

            var val = CreateText(go.transform, "", 14, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
            SetRect(val.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 2f), new Vector2(-10f, -2f));

            input.placeholder = ph;
            input.textComponent = val;
            return input;
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
