using System;
using System.Collections.Generic;
using System.Globalization;
using TacSim.Net;
using TacSim.Space;
using UnityEngine;
using UnityEngine.EventSystems;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// Interactive Solar System Holo-Orrery &amp; Astrogation Modal (`[M] STAR CHART`).
    /// Renders live `sky.chart` ephemeris keyframes with logarithmic orbital spacing, 3D tiltable/rotatable
    /// ecliptic projection, 3/6/12/24 light-minute rings, multi-leg trade corridor arcs, launch windows,
    /// and one-click authoritative navigation orders (`+ship/plan`, `+ship/order depart`, `+chart`).
    /// </summary>
    public sealed class TacSimSolarSystemMap : MonoBehaviour
    {
        public event Action OnRefreshChartRequested;
        public event Action<string> OnCommandRequested;
        public event Action<string> OnOpenSailingPlanRequested;
        public event Action OnOpenGroundChartRequested;

        public bool IsOpen => _overlay != null && _overlay.activeSelf;
        public TacSimSolarChartFrame CurrentFrame { get; private set; }
        public string SelectedBodyId { get; private set; }
        public string SelectedPortId { get; private set; }
        public bool IsTilted3D => _tiltDeg > 5f;
        public float Zoom => _zoom;
        public bool LastOrreryHasQuestDestinationRing { get; private set; }
        public string QuestDestinationPortId { get; private set; } = string.Empty;
        public string QuestDestinationPortLabel { get; private set; } = string.Empty;

        private const int TexSize = 512;

        private Font _font;
        private TacSimQuestJournal _questJournal;
        private GameObject _overlay;
        private Text _headingText;
        private Text _captionText;
        private Text _dossierText;
        private RawImage _orreryImage;
        private RectTransform _orreryRect;
        private Transform _labelsLayer;
        private Transform _routeListRoot;
        private Texture2D _orreryTex;
        private Color32[] _pixels;

        private Image _tiltBtnImg;
        private Image _lagBtnImg;
        private Image _routesBtnImg;
        private Button _questFocusBtn;
        private Text _questFocusBtnLbl;
        private Button _plotBtn;
        private Button _departBtn;
        private Button _windowBtn;
        private Button _sailingTabBtn;

        private readonly List<Text> _bodyLabelPool = new List<Text>();

        private float _tiltDeg = 38f;
        private float _yawDeg = -18f;
        private float _zoom = 1.0f;
        private Vector2 _panUv = Vector2.zero;
        private bool _showLagRings = true;
        private bool _showRoutes = true;
        private bool _dirty = true;
        private float _pulsePhase;
        private float _nextAnimRedrawAt;

        private sealed class OrreryPointerRelay : MonoBehaviour, IPointerClickHandler, IDragHandler, IScrollHandler
        {
            public TacSimSolarSystemMap Owner;
            public void OnPointerClick(PointerEventData eventData) => Owner?.HandleOrreryClick(eventData);
            public void OnDrag(PointerEventData eventData) => Owner?.HandleOrreryDrag(eventData);
            public void OnScroll(PointerEventData eventData) => Owner?.HandleOrreryScroll(eventData);
        }

        public void BuildModal(Transform canvasRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            if (_overlay != null)
            {
                if (Application.isPlaying) Destroy(_overlay);
                else DestroyImmediate(_overlay);
                _overlay = null;
            }
            _bodyLabelPool.Clear();

            _overlay = CreatePanel(
                canvasRoot,
                "SolarSystemMapOverlay",
                Vector2.zero,
                Vector2.one,
                Vector2.zero,
                Vector2.zero,
                new Color(0.01f, 0.02f, 0.05f, 0.86f));

            var card = CreatePanel(
                _overlay.transform,
                "SolarSystemMapCard",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-540f, -330f),
                new Vector2(540f, 330f),
                new Color(0.04f, 0.07f, 0.12f, 0.99f));

            _headingText = CreateText(card.transform, "SOLAR SYSTEM HOLO-ORRERY  //  WAITING FOR LIVE SKY CHART...", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.92f, 1f));
            SetRect(_headingText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -44f), new Vector2(-240f, -8f));

            var refreshBtn = CreateButton(card.transform, "REFRESH SKY", new Color(0.14f, 0.40f, 0.50f), () => OnRefreshChartRequested?.Invoke());
            SetRect(refreshBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-236f, -42f), new Vector2(-124f, -10f));

            var closeBtn = CreateButton(card.transform, "CLOSE [ESC]", new Color(0.45f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-116f, -42f), new Vector2(-14f, -10f));

            // Left pane: Interactive 512x512 Holo-Orrery viewport + toolbar
            var leftPane = CreatePanel(card.transform, "OrreryPane", new Vector2(0f, 0f), new Vector2(0.58f, 1f), new Vector2(16f, 14f), new Vector2(-8f, -52f), new Color(0.02f, 0.04f, 0.07f, 0.96f));

            var toolbar = CreatePanel(leftPane.transform, "OrreryToolbar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(8f, -38f), new Vector2(-8f, -6f), Color.clear);
            var tbHlg = toolbar.AddComponent<HorizontalLayoutGroup>();
            tbHlg.spacing = 6f;
            tbHlg.childControlWidth = true;
            tbHlg.childControlHeight = true;
            tbHlg.childForceExpandWidth = true;

            _tiltBtnImg = CreateButton(toolbar.transform, "3D TILT", new Color(0.20f, 0.52f, 0.72f), ToggleTiltMode).GetComponent<Image>();
            _lagBtnImg = CreateButton(toolbar.transform, "LAG RINGS", new Color(0.20f, 0.52f, 0.72f), ToggleLagRings).GetComponent<Image>();
            _routesBtnImg = CreateButton(toolbar.transform, "ROUTES", new Color(0.20f, 0.52f, 0.72f), ToggleRoutes).GetComponent<Image>();
            CreateButton(toolbar.transform, "ZOOM -", new Color(0.14f, 0.26f, 0.38f), () => AdjustZoom(-0.25f));
            CreateButton(toolbar.transform, "ZOOM +", new Color(0.14f, 0.26f, 0.38f), () => AdjustZoom(+0.25f));
            CreateButton(toolbar.transform, "RESET", new Color(0.22f, 0.26f, 0.34f), ResetView);
            CreateButton(toolbar.transform, "GROUND [G]", new Color(0.14f, 0.44f, 0.38f), () => OnOpenGroundChartRequested?.Invoke());
            _questFocusBtn = CreateButton(toolbar.transform, "QUEST", new Color(0.56f, 0.40f, 0.10f), () => FocusQuestDestinationPort());
            _questFocusBtnLbl = _questFocusBtn.GetComponentInChildren<Text>();
            _questFocusBtn.gameObject.SetActive(false);

            var viewportGo = new GameObject("OrreryViewport", typeof(RectTransform), typeof(RawImage));
            viewportGo.transform.SetParent(leftPane.transform, false);
            _orreryRect = viewportGo.GetComponent<RectTransform>();
            SetRect(_orreryRect, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(8f, 28f), new Vector2(-8f, -44f));

            _orreryTex = new Texture2D(TexSize, TexSize, TextureFormat.RGBA32, false)
            {
                name = "TacSimSolarOrreryTex",
                wrapMode = TextureWrapMode.Clamp,
                filterMode = FilterMode.Bilinear
            };
            _pixels = new Color32[TexSize * TexSize];
            _orreryImage = viewportGo.GetComponent<RawImage>();
            _orreryImage.texture = _orreryTex;
            _orreryImage.color = Color.white;
            _orreryImage.raycastTarget = true;

            var relay = viewportGo.AddComponent<OrreryPointerRelay>();
            relay.Owner = this;

            var labelsGo = new GameObject("BodyLabelsLayer", typeof(RectTransform));
            labelsGo.transform.SetParent(viewportGo.transform, false);
            SetRect(labelsGo.GetComponent<RectTransform>(), Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
            _labelsLayer = labelsGo.transform;

            _captionText = CreateText(leftPane.transform, "Drag to orbit 3D holo | Scroll to zoom | Click any body to plot corridor passage", 11, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.58f, 0.76f, 0.88f));
            SetRect(_captionText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(8f, 4f), new Vector2(-8f, 26f));

            // Right pane: Cleared Routes / Bodies List + Dossier + One-Click Navigation Actions
            var rightPane = CreatePanel(card.transform, "AstrogationSidebar", new Vector2(0.58f, 0f), new Vector2(1f, 1f), new Vector2(8f, 14f), new Vector2(-16f, -52f), new Color(0.03f, 0.06f, 0.10f, 0.96f));

            var listHdr = CreateText(rightPane.transform, "CORRIDOR DESTINATIONS & SYSTEM BODIES", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.74f, 0.88f, 0.98f));
            SetRect(listHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(12f, -30f), new Vector2(-12f, -6f));

            var listViewport = CreatePanel(rightPane.transform, "DestinationsViewport", new Vector2(0f, 0.46f), new Vector2(1f, 1f), new Vector2(10f, 4f), new Vector2(-10f, -34f), new Color(0.02f, 0.04f, 0.07f, 0.92f));
            listViewport.AddComponent<RectMask2D>();
            var listScroll = listViewport.AddComponent<ScrollRect>();
            var contentGo = new GameObject("DestinationsContent", typeof(RectTransform), typeof(VerticalLayoutGroup), typeof(ContentSizeFitter));
            contentGo.transform.SetParent(listViewport.transform, false);
            var contentRt = contentGo.GetComponent<RectTransform>();
            contentRt.anchorMin = new Vector2(0f, 1f);
            contentRt.anchorMax = new Vector2(1f, 1f);
            contentRt.pivot = new Vector2(0.5f, 1f);
            contentRt.offsetMin = Vector2.zero;
            contentRt.offsetMax = Vector2.zero;
            var vlg = contentGo.GetComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;
            var fitter = contentGo.GetComponent<ContentSizeFitter>();
            fitter.verticalFit = ContentSizeFitter.FitMode.PreferredSize;
            listScroll.viewport = listViewport.GetComponent<RectTransform>();
            listScroll.content = contentRt;
            listScroll.horizontal = false;
            listScroll.vertical = true;
            listScroll.movementType = ScrollRect.MovementType.Clamped;
            listScroll.scrollSensitivity = 26f;
            _routeListRoot = contentGo.transform;

            var dossierBox = CreatePanel(rightPane.transform, "TargetDossierBox", new Vector2(0f, 0.16f), new Vector2(1f, 0.45f), new Vector2(10f, 4f), new Vector2(-10f, -4f), new Color(0.04f, 0.08f, 0.13f, 0.95f));
            _dossierText = CreateText(dossierBox.transform, "Press <b>REFRESH SKY</b> or wait for the server's live <i>sky.chart</i> ephemeris feed to inspect orbital bodies, light lag, and corridor launch windows.", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 0.98f));
            _dossierText.supportRichText = true;
            _dossierText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _dossierText.verticalOverflow = VerticalWrapMode.Overflow;
            SetRect(_dossierText.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 6f), new Vector2(-10f, -6f));

            // One-click astrogation actions
            var actRow1 = CreatePanel(rightPane.transform, "NavActionsRow1", new Vector2(0f, 0.08f), new Vector2(1f, 0.15f), new Vector2(10f, 2f), new Vector2(-10f, -2f), Color.clear);
            var a1Hlg = actRow1.AddComponent<HorizontalLayoutGroup>();
            a1Hlg.spacing = 6f;
            a1Hlg.childControlWidth = true;
            a1Hlg.childControlHeight = true;
            a1Hlg.childForceExpandWidth = true;

            _plotBtn = CreateButton(actRow1.transform, "PLOT COURSE (+ship/plan)", new Color(0.14f, 0.46f, 0.58f), DispatchPlotCourse);
            _departBtn = CreateButton(actRow1.transform, "QUEUE DEPART", new Color(0.16f, 0.54f, 0.40f), DispatchQueueDepart);

            var actRow2 = CreatePanel(rightPane.transform, "NavActionsRow2", new Vector2(0f, 0.01f), new Vector2(1f, 0.08f), new Vector2(10f, 2f), new Vector2(-10f, -2f), Color.clear);
            var a2Hlg = actRow2.AddComponent<HorizontalLayoutGroup>();
            a2Hlg.spacing = 6f;
            a2Hlg.childControlWidth = true;
            a2Hlg.childControlHeight = true;
            a2Hlg.childForceExpandWidth = true;

            _windowBtn = CreateButton(actRow2.transform, "WINDOW SCAN (+chart)", new Color(0.24f, 0.34f, 0.52f), DispatchWindowScan);
            _sailingTabBtn = CreateButton(actRow2.transform, "OPEN SAILING PLAN", new Color(0.46f, 0.34f, 0.14f), DispatchOpenSailingPlan);

            RefreshToggleHighlights();
            RedrawOrreryTexture();
            _overlay.SetActive(false);
        }

        public void BindQuestJournal(TacSimQuestJournal journal)
        {
            if (_questJournal == journal) return;
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated -= RefreshQuestGuidance;
            _questJournal = journal;
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated += RefreshQuestGuidance;
            RefreshQuestGuidance();
        }

        public void RefreshQuestGuidance()
        {
            if (_questJournal != null && _questJournal.TryGetActiveDestinationPort(out var hint))
            {
                QuestDestinationPortId = hint.PortId;
                QuestDestinationPortLabel = hint.PortLabel;
                if (_questFocusBtn != null) _questFocusBtn.gameObject.SetActive(true);
                if (_questFocusBtnLbl != null) _questFocusBtnLbl.text = $"QUEST: {hint.PortId.ToUpperInvariant()}";
            }
            else
            {
                QuestDestinationPortId = string.Empty;
                QuestDestinationPortLabel = string.Empty;
                if (_questFocusBtn != null) _questFocusBtn.gameObject.SetActive(false);
            }
            RebuildSidebarList();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public bool FocusQuestDestinationPort()
        {
            if (_questJournal == null || !_questJournal.TryGetActiveDestinationPort(out var hint))
                return false;
            FocusPort(hint.PortId);
            return true;
        }

        public void ResetSessionState()
        {
            CurrentFrame = null;
            SelectedBodyId = null;
            SelectedPortId = null;
            LastOrreryHasQuestDestinationRing = false;
            QuestDestinationPortId = string.Empty;
            QuestDestinationPortLabel = string.Empty;
            if (_questFocusBtn != null) _questFocusBtn.gameObject.SetActive(false);
            if (_overlay != null) _overlay.SetActive(false);
            if (_headingText != null) _headingText.text = "SOLAR SYSTEM HOLO-ORRERY  //  WAITING FOR LIVE SKY CHART...";
            if (_dossierText != null) _dossierText.text = "Press <b>REFRESH SKY</b> to query the server's live <i>sky.chart</i> ephemeris.";
            RebuildSidebarList();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void SetOpen(bool open)
        {
            if (_overlay == null) return;
            if (open)
            {
                _overlay.transform.SetAsLastSibling();
                _overlay.SetActive(true);
                _dirty = true;
                RedrawOrreryTexture();
                OnRefreshChartRequested?.Invoke();
            }
            else
            {
                _overlay.SetActive(false);
            }
        }

        public void ApplyChartFrame(TacSimSolarChartFrame frame)
        {
            if (frame == null) return;
            CurrentFrame = frame;

            // Default selection: first cleared route's body/port, else the vantage body
            if (string.IsNullOrEmpty(SelectedBodyId) || frame.FindBody(SelectedBodyId) == null)
            {
                if (frame.Routes.Count > 0)
                {
                    SelectedPortId = frame.Routes[0].To;
                    SelectedBodyId = !string.IsNullOrEmpty(frame.Routes[0].Body) ? frame.Routes[0].Body : frame.Routes[0].To;
                }
                else
                {
                    var here = frame.FindHereBody();
                    SelectedBodyId = here != null ? here.Id : (frame.Bodies.Count > 0 ? frame.Bodies[0].Id : null);
                    SelectedPortId = null;
                }
            }

            if (_headingText != null)
                _headingText.text = frame.BuildHeadingText();
            if (_captionText != null)
                _captionText.text = string.Format(
                    CultureInfo.InvariantCulture,
                    "Log scale past {0:0.0} AU (edge {1:0.1} AU) | Rings: 3/6/12/24 light-min | Drag to orbit, scroll to zoom, click body",
                    frame.ScaleR0Au,
                    frame.ScaleAuMax);

            RebuildSidebarList();
            RefreshSelectedDossier();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void SelectBody(string bodyId, string explicitPortId = null)
        {
            if (CurrentFrame == null) return;
            SelectedBodyId = bodyId;
            if (!string.IsNullOrEmpty(explicitPortId))
            {
                SelectedPortId = explicitPortId;
            }
            else
            {
                var route = CurrentFrame.FindRouteForBodyOrPort(bodyId);
                SelectedPortId = route != null ? route.To : null;
            }
            RefreshSelectedDossier();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void FocusPort(string portSlug)
        {
            if (string.IsNullOrWhiteSpace(portSlug))
            {
                SetOpen(true);
                return;
            }
            string cleanSlug = portSlug.Trim().ToLowerInvariant();
            SelectedPortId = cleanSlug;
            if (CurrentFrame != null)
            {
                var route = CurrentFrame.FindRouteForBodyOrPort(cleanSlug);
                if (route != null)
                {
                    SelectedBodyId = !string.IsNullOrEmpty(route.Body) ? route.Body : route.To;
                    SelectedPortId = route.To;
                }
                else
                {
                    var body = CurrentFrame.FindBody(cleanSlug);
                    if (body != null)
                        SelectedBodyId = body.Id;
                }
            }
            SetOpen(true);
            RefreshSelectedDossier();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void ToggleTiltMode()
        {
            _tiltDeg = _tiltDeg > 5f ? 0f : 38f;
            RefreshToggleHighlights();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void ToggleLagRings()
        {
            _showLagRings = !_showLagRings;
            RefreshToggleHighlights();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void ToggleRoutes()
        {
            _showRoutes = !_showRoutes;
            RefreshToggleHighlights();
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void AdjustZoom(float delta)
        {
            _zoom = Mathf.Clamp(_zoom + delta, 0.70f, 3.0f);
            _dirty = true;
            RedrawOrreryTexture();
        }

        public void ResetView()
        {
            _tiltDeg = 38f;
            _yawDeg = -18f;
            _zoom = 1.0f;
            _panUv = Vector2.zero;
            RefreshToggleHighlights();
            _dirty = true;
            RedrawOrreryTexture();
        }

        private void Update()
        {
            if (!IsOpen || CurrentFrame == null) return;
            if (Time.unscaledTime >= _nextAnimRedrawAt)
            {
                _nextAnimRedrawAt = Time.unscaledTime + 0.08f; // crisp ~12 Hz pulse along plotted route
                _pulsePhase = (_pulsePhase + 0.03f) % 1.0f;
                _dirty = true;
            }
            if (_dirty)
            {
                RedrawOrreryTexture();
            }
        }

        private void OnDestroy()
        {
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated -= RefreshQuestGuidance;
            if (_orreryTex != null)
            {
                if (Application.isPlaying) Destroy(_orreryTex);
                else DestroyImmediate(_orreryTex);
                _orreryTex = null;
            }
        }

        private void HandleOrreryClick(PointerEventData eventData)
        {
            if (CurrentFrame == null || _orreryRect == null || eventData == null) return;
            if (!RectTransformUtility.ScreenPointToLocalPointInRectangle(_orreryRect, eventData.position, eventData.pressEventCamera, out Vector2 local))
                return;

            Rect r = _orreryRect.rect;
            if (r.width <= 1f || r.height <= 1f) return;
            Vector2 uv = new Vector2((local.x - r.x) / r.width, (local.y - r.y) / r.height);
            var hit = CurrentFrame.FindNearestPrimaryBody(uv, _zoom, _panUv, _tiltDeg, _yawDeg, maxDistUv: 0.075f);
            if (hit != null)
                SelectBody(hit.Id);
        }

        private void HandleOrreryDrag(PointerEventData eventData)
        {
            if (eventData == null) return;
            _yawDeg = (_yawDeg + eventData.delta.x * 0.45f) % 360f;
            _tiltDeg = Mathf.Clamp(_tiltDeg - eventData.delta.y * 0.35f, 0f, 65f);
            RefreshToggleHighlights();
            _dirty = true;
            RedrawOrreryTexture();
        }

        private void HandleOrreryScroll(PointerEventData eventData)
        {
            if (eventData == null) return;
            AdjustZoom(eventData.scrollDelta.y * 0.12f);
        }

        private bool TryResolveSelectedSafePort(out string safePort)
        {
            safePort = null;
            string raw = !string.IsNullOrEmpty(SelectedPortId)
                ? SelectedPortId
                : CurrentFrame?.FindRouteForBodyOrPort(SelectedBodyId)?.To;
            if (string.IsNullOrEmpty(raw)) return false;
            return MushProtocol.TryValidateSafeCommandToken(raw, out safePort, maxLen: 48);
        }

        public void DispatchPlotCourse()
        {
            if (TryResolveSelectedSafePort(out string port))
                OnCommandRequested?.Invoke($"+ship/plan {port}");
            else
                OnCommandRequested?.Invoke("+ship/plan");
        }

        public void DispatchQueueDepart()
        {
            if (TryResolveSelectedSafePort(out string port))
                OnCommandRequested?.Invoke($"+ship/order depart {port}");
        }

        public void DispatchWindowScan()
        {
            if (TryResolveSelectedSafePort(out string port))
                OnCommandRequested?.Invoke($"+chart {port}");
            else
                OnCommandRequested?.Invoke("+chart");
        }

        public void DispatchOpenSailingPlan()
        {
            string port = TryResolveSelectedSafePort(out string safePort) ? safePort : string.Empty;
            SetOpen(false);
            OnOpenSailingPlanRequested?.Invoke(port);
        }

        private bool IsQuestDestinationRoute(SolarChartRoute route)
        {
            return route != null &&
                   !string.IsNullOrEmpty(QuestDestinationPortId) &&
                   string.Equals(route.To, QuestDestinationPortId, StringComparison.OrdinalIgnoreCase);
        }

        private bool IsQuestDestinationBody(SolarChartBody body)
        {
            if (body == null || string.IsNullOrEmpty(QuestDestinationPortId)) return false;
            if (string.Equals(body.Id, QuestDestinationPortId, StringComparison.OrdinalIgnoreCase))
                return true;
            if (CurrentFrame != null)
            {
                for (int i = 0; i < CurrentFrame.Routes.Count; i++)
                {
                    var r = CurrentFrame.Routes[i];
                    if (string.Equals(r.To, QuestDestinationPortId, StringComparison.OrdinalIgnoreCase) &&
                        string.Equals(r.Body, body.Id, StringComparison.OrdinalIgnoreCase))
                    {
                        return true;
                    }
                }
            }
            return false;
        }

        private void RebuildSidebarList()
        {
            if (_routeListRoot == null) return;
            for (int i = _routeListRoot.childCount - 1; i >= 0; i--)
            {
                var ch = _routeListRoot.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }

            if (CurrentFrame == null) return;

            // 1. Cleared Passage Routes first (most actionable for captains)
            for (int i = 0; i < CurrentFrame.Routes.Count; i++)
            {
                var r = CurrentFrame.Routes[i];
                string bodyId = !string.IsNullOrEmpty(r.Body) ? r.Body : r.To;
                string portId = r.To;
                bool isQuest = IsQuestDestinationRoute(r);
                string summary = r.Loaded != null
                    ? $"{r.To.ToUpperInvariant()}  ({TacSimSolarChartFrame.FormatHours(r.Loaded.TotalSeconds)}, {r.Loaded.BurnTonnes:0.#}t)"
                    : $"{r.To.ToUpperInvariant()}  (overweight)";
                bool isSel = string.Equals(SelectedPortId, portId, StringComparison.OrdinalIgnoreCase);
                Color col = isQuest
                    ? (isSel ? new Color(0.68f, 0.48f, 0.14f) : new Color(0.54f, 0.38f, 0.12f))
                    : (isSel ? new Color(0.22f, 0.56f, 0.72f) : new Color(0.14f, 0.28f, 0.40f));
                string prefix = isQuest ? "[QUEST] ROUTE: " : "ROUTE: ";
                var btn = CreateButton(_routeListRoot, prefix + summary, col, () => SelectBody(bodyId, portId));
                var le = btn.gameObject.AddComponent<LayoutElement>();
                le.minHeight = 30f;
                le.preferredHeight = 30f;
            }

            // 2. All primary solar bodies sorted nearest-Sun first
            var sorted = new List<SolarChartBody>();
            for (int i = 0; i < CurrentFrame.Bodies.Count; i++)
            {
                if (string.IsNullOrEmpty(CurrentFrame.Bodies[i].ParentId))
                    sorted.Add(CurrentFrame.Bodies[i]);
            }
            sorted.Sort((a, b) => a.RadiusAu.CompareTo(b.RadiusAu));

            for (int i = 0; i < sorted.Count; i++)
            {
                var b = sorted[i];
                string bId = b.Id;
                bool isQuest = IsQuestDestinationBody(b);
                string tag = b.Here
                    ? "HERE"
                    : (b.LagSeconds.HasValue ? TacSimSolarChartFrame.FormatLightLag(b.LagSeconds.Value) : $"{b.RadiusAu:0.00} AU");
                bool isSel = string.Equals(SelectedBodyId, bId, StringComparison.OrdinalIgnoreCase);
                Color col = isQuest && !b.Here
                    ? new Color(0.48f, 0.36f, 0.12f)
                    : (b.Here
                        ? new Color(0.42f, 0.34f, 0.12f)
                        : (isSel ? new Color(0.18f, 0.44f, 0.58f) : new Color(0.10f, 0.17f, 0.25f)));
                string prefix = isQuest ? "[QUEST] " : string.Empty;
                var btn = CreateButton(_routeListRoot, $"{prefix}{b.Label}  —  {b.RadiusAu:0.00} AU  [{tag}]", col, () => SelectBody(bId));
                var le = btn.gameObject.AddComponent<LayoutElement>();
                le.minHeight = 26f;
                le.preferredHeight = 26f;
            }
        }

        private void RefreshSelectedDossier()
        {
            if (_dossierText == null) return;
            if (CurrentFrame == null)
            {
                _dossierText.text = "No live <i>sky.chart</i> frame yet — press <b>REFRESH SKY</b>.";
                return;
            }
            var body = CurrentFrame.FindBody(SelectedBodyId) ?? CurrentFrame.FindHereBody();
            var route = !string.IsNullOrEmpty(SelectedPortId)
                ? CurrentFrame.FindRouteForBodyOrPort(SelectedPortId)
                : CurrentFrame.FindRouteForBodyOrPort(body?.Id);
            _dossierText.text = CurrentFrame.BuildBodyAndRouteDossier(body, route);

            bool hasPort = route != null && MushProtocol.TryValidateSafeCommandToken(route.To, out _, maxLen: 48);
            if (_departBtn != null) _departBtn.interactable = hasPort;
            if (_plotBtn != null) _plotBtn.interactable = true;
            if (_windowBtn != null) _windowBtn.interactable = true;
            if (_sailingTabBtn != null) _sailingTabBtn.interactable = true;
        }

        private void RefreshToggleHighlights()
        {
            Color on = new Color(0.20f, 0.56f, 0.76f, 1f);
            Color off = new Color(0.12f, 0.20f, 0.28f, 0.92f);
            if (_tiltBtnImg != null) _tiltBtnImg.color = IsTilted3D ? on : off;
            if (_lagBtnImg != null) _lagBtnImg.color = _showLagRings ? on : off;
            if (_routesBtnImg != null) _routesBtnImg.color = _showRoutes ? on : off;
        }

        private void RedrawOrreryTexture()
        {
            _dirty = false;
            LastOrreryHasQuestDestinationRing = false;
            if (_orreryTex == null || _pixels == null) return;

            // 1. Deep space radial gradient background
            Color32 bgOuter = new Color32(7, 11, 18, 255);
            Color32 bgInner = new Color32(15, 24, 38, 255);
            float half = TexSize * 0.5f;
            for (int y = 0; y < TexSize; y++)
            {
                float dy = (y - half) / half;
                int row = y * TexSize;
                for (int x = 0; x < TexSize; x++)
                {
                    float dx = (x - half) / half;
                    float t = Mathf.Clamp01(1f - Mathf.Sqrt(dx * dx + dy * dy));
                    t *= t;
                    _pixels[row + x] = new Color32(
                        (byte)(bgOuter.r + (bgInner.r - bgOuter.r) * t),
                        (byte)(bgOuter.g + (bgInner.g - bgOuter.g) * t),
                        (byte)(bgOuter.b + (bgInner.b - bgOuter.b) * t),
                        255);
                }
            }

            // Subtle azimuth crosshairs through Sun
            Color32 gridCol = new Color32(28, 44, 64, 255);
            if (CurrentFrame != null)
            {
                Vector2 sunUv = CurrentFrame.ProjectEclipticToUv(0.0, 0.0, 0.0, _zoom, _panUv, _tiltDeg, _yawDeg, out _);
                Vector2 pxSun = UvToPx(sunUv);

                for (int deg = 0; deg < 360; deg += 45)
                {
                    Vector2 outerUv = CurrentFrame.ProjectEclipticToUv(CurrentFrame.ScaleAuMax, deg, 0.0, _zoom, _panUv, _tiltDeg, _yawDeg, out _);
                    DrawLine(pxSun, UvToPx(outerUv), gridCol, dashed: true);
                }

                // 2. Planetary orbital rings
                Color32 orbitCol = new Color32(52, 72, 96, 255);
                for (int i = 0; i < CurrentFrame.Bodies.Count; i++)
                {
                    var b = CurrentFrame.Bodies[i];
                    if (!string.IsNullOrEmpty(b.ParentId) || b.RadiusAu <= 0.01) continue;
                    Color32 ringCol = string.Equals(b.Id, SelectedBodyId, StringComparison.OrdinalIgnoreCase)
                        ? new Color32(90, 175, 225, 255)
                        : (b.Here ? new Color32(140, 120, 58, 255) : orbitCol);
                    Vector2 prev = Vector2.zero;
                    const int Segs = 72;
                    for (int s = 0; s <= Segs; s++)
                    {
                        double lon = (360.0 * s) / Segs;
                        Vector2 uv = CurrentFrame.ProjectEclipticToUv(b.RadiusAu, lon, 0.0, _zoom, _panUv, _tiltDeg, _yawDeg, out _);
                        Vector2 p = UvToPx(uv);
                        if (s > 0) DrawLine(prev, p, ringCol, dashed: false);
                        prev = p;
                    }
                }

                // 3. Light-lag rings (3, 6, 12, 24 light-minutes from current body)
                if (_showLagRings)
                {
                    Color32 lagCol = new Color32(58, 122, 152, 255);
                    for (int r = 0; r < TacSimSolarChartFrame.LagRingMinutes.Length; r++)
                    {
                        var ring = CurrentFrame.ComputeLagRingUv(TacSimSolarChartFrame.LagRingMinutes[r], _zoom, _panUv, _tiltDeg, _yawDeg, steps: 80);
                        for (int k = 1; k < ring.Count; k++)
                            DrawLine(UvToPx(ring[k - 1]), UvToPx(ring[k]), lagCol, dashed: true);
                    }
                }

                // 4. Multi-leg corridor trade routes
                if (_showRoutes)
                {
                    for (int i = 0; i < CurrentFrame.Routes.Count; i++)
                    {
                        var route = CurrentFrame.Routes[i];
                        bool isQuestRoute = IsQuestDestinationRoute(route);
                        bool isSel = string.Equals(route.To, SelectedPortId, StringComparison.OrdinalIgnoreCase) ||
                                     string.Equals(route.Body, SelectedBodyId, StringComparison.OrdinalIgnoreCase);
                        bool highlight = isSel || isQuestRoute;
                        Color32 routeCol = isQuestRoute
                            ? new Color32(255, 224, 96, 255)
                            : (isSel ? new Color32(255, 212, 92, 255) : new Color32(165, 136, 76, 255));

                        var poly = CurrentFrame.ComputeRoutePolylineUv(route, _zoom, _panUv, _tiltDeg, _yawDeg);
                        for (int k = 1; k < poly.Count; k++)
                        {
                            Vector2 p0 = UvToPx(poly[k - 1]);
                            Vector2 p1 = UvToPx(poly[k]);
                            DrawLine(p0, p1, routeCol, dashed: !highlight);
                            if (highlight)
                            {
                                DrawLine(p0 + Vector2.up, p1 + Vector2.up, routeCol, dashed: false);
                                Vector2 pulse = Vector2.Lerp(p0, p1, _pulsePhase);
                                DrawFilledCircle(pulse, 3.5f, new Color32(255, 248, 190, 255));
                            }
                        }
                    }
                }

                // 5. Glowing Sun at center
                DrawFilledCircle(pxSun, 12f, new Color32(95, 72, 24, 255));
                DrawFilledCircle(pxSun, 8f, new Color32(215, 168, 62, 255));
                DrawFilledCircle(pxSun, 5f, new Color32(255, 240, 170, 255));

                // 6. Planetary bodies + 3D ecliptic inclination stems + selection reticles + labels
                int labelIdx = 0;
                for (int i = 0; i < CurrentFrame.Bodies.Count; i++)
                {
                    var b = CurrentFrame.Bodies[i];
                    if (!string.IsNullOrEmpty(b.ParentId)) continue;

                    Vector2 uv = CurrentFrame.ProjectBodyToUv(b, _zoom, _panUv, _tiltDeg, _yawDeg, out Vector2 footUv);
                    Vector2 p = UvToPx(uv);
                    Vector2 pf = UvToPx(footUv);

                    if (Vector2.Distance(p, pf) > 1.5f)
                    {
                        DrawLine(pf, p, new Color32(90, 130, 160, 255), dashed: false);
                        DrawFilledCircle(pf, 2f, new Color32(90, 130, 160, 255));
                    }

                    Color32 bodyCol = GetPlanetColor(b.Id, b.Here);
                    float radius = b.Here ? 6.5f : 5.0f;
                    DrawFilledCircle(p, radius, bodyCol);

                    if (b.Here)
                        DrawCircleOutline(p, 11.5f, new Color32(242, 201, 76, 255));

                    if (string.Equals(b.Id, SelectedBodyId, StringComparison.OrdinalIgnoreCase))
                        DrawCircleOutline(p, 14.5f, new Color32(92, 224, 255, 255));

                    bool isQuestBody = IsQuestDestinationBody(b);
                    if (isQuestBody)
                    {
                        DrawCircleOutline(p, 16.5f, new Color32(255, 220, 72, 255));
                        DrawCircleOutline(p, 18.0f, new Color32(255, 236, 130, 255));
                        LastOrreryHasQuestDestinationRing = true;
                    }

                    PositionBodyLabel(labelIdx++, b, uv, isQuestBody);
                }
                HideUnusedLabels(labelIdx);
            }
            else
            {
                // Clean standby Sun & concentric rings when awaiting first server frame
                Vector2 center = new Vector2(half, half);
                DrawCircleOutline(center, 60f, gridCol);
                DrawCircleOutline(center, 120f, gridCol);
                DrawCircleOutline(center, 185f, gridCol);
                DrawFilledCircle(center, 7f, new Color32(245, 211, 107, 255));
                HideUnusedLabels(0);
            }

            _orreryTex.SetPixels32(_pixels);
            _orreryTex.Apply(false, false);
        }

        private void PositionBodyLabel(int poolIndex, SolarChartBody body, Vector2 uv, bool isQuestTarget)
        {
            if (_labelsLayer == null) return;
            while (_bodyLabelPool.Count <= poolIndex)
            {
                var lbl = CreateText(_labelsLayer, "", 11, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
                lbl.rectTransform.sizeDelta = new Vector2(130f, 20f);
                _bodyLabelPool.Add(lbl);
            }
            var t = _bodyLabelPool[poolIndex];
            t.gameObject.SetActive(true);
            string baseLabel = body.Here ? $"★ {body.Label}" : body.Label;
            t.text = isQuestTarget ? $"[!] {baseLabel}" : baseLabel;
            t.color = isQuestTarget
                ? new Color(1f, 0.88f, 0.34f)
                : (body.Here
                    ? new Color(0.96f, 0.82f, 0.34f)
                    : (string.Equals(body.Id, SelectedBodyId, StringComparison.OrdinalIgnoreCase)
                        ? new Color(0.45f, 0.94f, 1f)
                        : new Color(0.76f, 0.88f, 0.96f)));
            var rt = t.rectTransform;
            rt.anchorMin = uv;
            rt.anchorMax = uv;
            rt.pivot = new Vector2(0f, 0.5f);
            rt.anchoredPosition = new Vector2(10f, 2f);
        }

        private void HideUnusedLabels(int startIdx)
        {
            for (int i = startIdx; i < _bodyLabelPool.Count; i++)
            {
                if (_bodyLabelPool[i] != null)
                    _bodyLabelPool[i].gameObject.SetActive(false);
            }
        }

        private static Color32 GetPlanetColor(string id, bool here)
        {
            if (here) return new Color32(242, 201, 76, 255);
            string k = (id ?? string.Empty).ToLowerInvariant();
            if (k.Contains("mercury")) return new Color32(176, 168, 160, 255);
            if (k.Contains("venus")) return new Color32(232, 206, 146, 255);
            if (k.Contains("earth")) return new Color32(92, 174, 242, 255);
            if (k.Contains("mars")) return new Color32(218, 108, 72, 255);
            if (k.Contains("ceres")) return new Color32(116, 214, 232, 255);
            if (k.Contains("vesta")) return new Color32(198, 204, 214, 255);
            if (k.Contains("jupiter")) return new Color32(216, 166, 118, 255);
            if (k.Contains("saturn")) return new Color32(226, 202, 138, 255);
            return new Color32(160, 196, 216, 255);
        }

        private static Vector2 UvToPx(Vector2 uv)
        {
            return new Vector2(uv.x * (TexSize - 1), uv.y * (TexSize - 1));
        }

        private void Plot(int x, int y, Color32 col)
        {
            if ((uint)x >= TexSize || (uint)y >= TexSize) return;
            _pixels[y * TexSize + x] = col;
        }

        private void DrawFilledCircle(Vector2 center, float radius, Color32 col)
        {
            int cx = Mathf.RoundToInt(center.x);
            int cy = Mathf.RoundToInt(center.y);
            int r = Mathf.CeilToInt(radius);
            float rSq = radius * radius;
            for (int dy = -r; dy <= r; dy++)
            {
                for (int dx = -r; dx <= r; dx++)
                {
                    if (dx * dx + dy * dy <= rSq)
                        Plot(cx + dx, cy + dy, col);
                }
            }
        }

        private void DrawCircleOutline(Vector2 center, float radius, Color32 col)
        {
            int segs = Mathf.Clamp(Mathf.RoundToInt(radius * 4f), 24, 144);
            Vector2 prev = Vector2.zero;
            for (int i = 0; i <= segs; i++)
            {
                float a = (Mathf.PI * 2f * i) / segs;
                Vector2 p = new Vector2(center.x + Mathf.Cos(a) * radius, center.y + Mathf.Sin(a) * radius);
                if (i > 0) DrawLine(prev, p, col, dashed: false);
                prev = p;
            }
        }

        private void DrawLine(Vector2 a, Vector2 b, Color32 col, bool dashed)
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
            int step = 0;
            int guard = TexSize * 3;
            while (guard-- > 0)
            {
                if (!dashed || ((step / 4) % 2 == 0))
                    Plot(x0, y0, col);
                if (x0 == x1 && y0 == y1) break;
                int e2 = err * 2;
                if (e2 > -dy) { err -= dy; x0 += sx; }
                if (e2 < dx) { err += dx; y0 += sy; }
                step++;
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

            var txt = CreateText(go.transform, label, 12, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 2f), new Vector2(-6f, -2f));
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
