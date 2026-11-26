using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum QuestJournalTab
    {
        Quests = 0,
        Passages = 1,
        Surveys = 2,
        Claims = 3
    }

    /// <summary>
    /// WoW / EverQuest-style `[J]` Quest, Passage, Survey &amp; Mining Claims Journal +
    /// collapsible right-edge Pinned Objective Tracker Card docked cleanly below the Minimap.
    /// Powered 100% by live verified `mushd` verbs (`+quest`, `+passage`, `+survey`, `+claim/list`).
    /// </summary>
    public sealed class TacSimQuestJournal : MonoBehaviour
    {
        public event Action<string> OnCommandRequested;

        private const int MaxLinesPerTab = 48;

        private static readonly string[] TabCommands =
        {
            "+quest",
            "+passage",
            "+survey",
            "+claim/list"
        };

        private static readonly string[] TabTitles =
        {
            "QUESTS & FAVORS",
            "PASSENGER FARES",
            "SURVEY COMMISSIONS",
            "MINING CLAIMS"
        };

        private Font _font;
        private GameObject _modalRoot;
        private Text _bodyText;
        private Text _tabHintText;
        private readonly List<Image> _tabImages = new List<Image>();

        // Right-edge Collapsible Pinned Objective Tracker Card (docked below TacticalMinimap)
        private GameObject _trackerRoot;
        private RectTransform _trackerRect;
        private GameObject _trackerBody;
        private Text _trackerHeaderText;
        private Text _trackerDetailText;
        private Text _trackerCollapseLbl;
        private bool _trackerCollapsed;

        private readonly List<string>[] _tabLines =
        {
            new List<string>(),
            new List<string>(),
            new List<string>(),
            new List<string>()
        };
        private readonly List<string>[] _tabPlainLines =
        {
            new List<string>(),
            new List<string>(),
            new List<string>(),
            new List<string>()
        };

        private QuestJournalTab? _capturingTab;
        private float _captureExpiresAt;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public QuestJournalTab ActiveTab { get; private set; } = QuestJournalTab.Quests;
        public bool HasPinnedObjective => !string.IsNullOrEmpty(PinnedTitle);
        public bool IsTrackerVisible => _trackerRoot != null && _trackerRoot.activeSelf;
        public bool IsTrackerCollapsed => _trackerCollapsed;
        public string PinnedCategory { get; private set; } = string.Empty;
        public string PinnedTitle { get; private set; } = string.Empty;
        public string PinnedDetail { get; private set; } = string.Empty;

        public int GetTabLineCount(QuestJournalTab tab) => _tabLines[(int)tab].Count;
        public string GetTabSnapshot(QuestJournalTab tab) => string.Join("\n", _tabPlainLines[(int)tab]);

        public void BuildUi(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _tabImages.Clear();

            BuildPinnedTrackerCard(hudRoot);
            BuildJournalModal(hudRoot);
        }

        private void BuildPinnedTrackerCard(Transform hudRoot)
        {
            // Docked cleanly below TacticalMinimap (`-232..-16`, `-320..-72`) on right edge
            _trackerRoot = CreatePanel(
                hudRoot,
                "PinnedObjectiveTracker",
                new Vector2(1f, 1f),
                new Vector2(1f, 1f),
                new Vector2(-232f, -456f),
                new Vector2(-16f, -328f),
                new Color(0.03f, 0.06f, 0.11f, 0.90f));
            _trackerRect = _trackerRoot.GetComponent<RectTransform>();

            var hdrBar = CreatePanel(_trackerRoot.transform, "TrackerHeader", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -24f), Vector2.zero, new Color(0.10f, 0.22f, 0.30f, 0.96f));
            _trackerHeaderText = CreateText(hdrBar.transform, "OBJECTIVE [J]", 10, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.86f, 0.44f));
            SetRect(_trackerHeaderText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-56f, 0f));

            var openBtn = CreateSmallButton(hdrBar.transform, "J", new Color(0.16f, 0.38f, 0.50f), () => SetOpen(true), out _);
            SetRect(openBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-52f, 2f), new Vector2(-30f, -2f));

            var colBtn = CreateSmallButton(hdrBar.transform, "—", new Color(0.16f, 0.26f, 0.38f), ToggleTrackerCollapsed, out _trackerCollapseLbl);
            SetRect(colBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-26f, 2f), new Vector2(-4f, -2f));

            _trackerBody = CreatePanel(_trackerRoot.transform, "TrackerBody", Vector2.zero, Vector2.one, new Vector2(0f, 0f), new Vector2(0f, -24f), Color.clear);
            _trackerDetailText = CreateText(_trackerBody.transform, string.Empty, 11, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.95f, 1f));
            _trackerDetailText.supportRichText = true;
            _trackerDetailText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _trackerDetailText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_trackerDetailText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 24f), new Vector2(-8f, -6f));

            var unpinBtn = CreateSmallButton(_trackerBody.transform, "UNPIN", new Color(0.36f, 0.16f, 0.18f), ClearPinnedObjective, out _);
            SetRect(unpinBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-62f, 4f), new Vector2(-6f, 22f));

            _trackerRoot.SetActive(false);
        }

        private void BuildJournalModal(Transform hudRoot)
        {
            _modalRoot = CreatePanel(
                hudRoot,
                "QuestAndOpsJournalModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-455f, -290f),
                new Vector2(455f, 290f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            CreatePanel(_modalRoot.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(1f, 0.82f, 0.36f, 0.95f));

            var hdr = CreateText(_modalRoot.transform, "MISSION, PASSAGE, SURVEY & CLAIMS JOURNAL [J]", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.86f, 0.44f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(0.76f, 1f), new Vector2(18f, -42f), new Vector2(0f, -8f));

            var closeBtn = CreateButton(_modalRoot.transform, "CLOSE [ESC]", new Color(0.46f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -40f), new Vector2(-14f, -10f));

            // 4 Journal Tabs
            var tabStrip = CreatePanel(_modalRoot.transform, "JournalTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -80f), new Vector2(-16f, -46f), Color.clear);
            var hlg = tabStrip.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 6f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            for (int i = 0; i < TabTitles.Length; i++)
            {
                var captured = (QuestJournalTab)i;
                var tb = CreateButton(tabStrip.transform, TabTitles[i], new Color(0.12f, 0.22f, 0.34f), () => SelectTab(captured, queryServer: true));
                _tabImages.Add(tb.GetComponent<Image>());
            }

            _tabHintText = CreateText(_modalRoot.transform, "Live ledger synchronized via +quest", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.68f, 0.86f, 0.96f));
            SetRect(_tabHintText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(18f, -106f), new Vector2(-18f, -84f));

            var bodyPanel = CreatePanel(_modalRoot.transform, "JournalBody", Vector2.zero, Vector2.one, new Vector2(16f, 64f), new Vector2(-16f, -110f), new Color(0.02f, 0.05f, 0.09f, 0.94f));
            _bodyText = CreateText(bodyPanel.transform, string.Empty, 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.94f, 0.99f));
            _bodyText.supportRichText = true;
            _bodyText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _bodyText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_bodyText.rectTransform, Vector2.zero, Vector2.one, new Vector2(14f, 12f), new Vector2(-14f, -12f));

            // Bottom action strip
            var actRow = CreatePanel(_modalRoot.transform, "JournalActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(16f, 14f), new Vector2(-16f, 54f), Color.clear);
            var aHlg = actRow.AddComponent<HorizontalLayoutGroup>();
            aHlg.spacing = 8f;
            aHlg.childControlWidth = true;
            aHlg.childControlHeight = true;
            aHlg.childForceExpandWidth = true;

            CreateButton(actRow.transform, "REFRESH TAB", new Color(0.14f, 0.44f, 0.56f), () => QueryTabFromServer(ActiveTab));
            CreateButton(actRow.transform, "PIN ACTIVE TO HUD", new Color(0.46f, 0.36f, 0.14f), PinActiveTabSummary);
            CreateButton(actRow.transform, "CLEAR PIN", new Color(0.36f, 0.18f, 0.20f), ClearPinnedObjective);
            CreateButton(actRow.transform, "HAUL BOARD (+haul)", new Color(0.16f, 0.38f, 0.48f), () => OnCommandRequested?.Invoke("+haul"));
            CreateButton(actRow.transform, "ESCORT (+escort)", new Color(0.18f, 0.36f, 0.52f), () => OnCommandRequested?.Invoke("+escort"));

            SelectTab(QuestJournalTab.Quests, queryServer: false);
            _modalRoot.SetActive(false);
        }

        public void SetOpen(bool open, QuestJournalTab? tabOverride = null)
        {
            if (_modalRoot == null) return;
            _modalRoot.SetActive(open);
            if (open)
            {
                if (tabOverride.HasValue)
                    SelectTab(tabOverride.Value, queryServer: true);
                else
                    QueryTabFromServer(ActiveTab);
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void ResetSessionState()
        {
            for (int i = 0; i < _tabLines.Length; i++)
            {
                _tabLines[i].Clear();
                _tabPlainLines[i].Clear();
            }
            _capturingTab = null;
            _captureExpiresAt = 0f;
            ClearPinnedObjective();
            SetOpen(false);
        }

        public void SelectTab(QuestJournalTab tab, bool queryServer = true)
        {
            ActiveTab = tab;
            int idx = (int)tab;
            for (int i = 0; i < _tabImages.Count; i++)
            {
                _tabImages[i].color = i == idx
                    ? new Color(0.52f, 0.40f, 0.16f, 1f)
                    : new Color(0.10f, 0.20f, 0.30f, 0.94f);
            }
            if (queryServer)
                QueryTabFromServer(tab);
            RefreshJournalBody();
        }

        public void QueryTabFromServer(QuestJournalTab tab)
        {
            BeginCapture(tab, clearExisting: true);
            OnCommandRequested?.Invoke(TabCommands[(int)tab]);
        }

        public void BeginCapture(QuestJournalTab tab, bool clearExisting = true, float windowSec = 4.5f)
        {
            _capturingTab = tab;
            _captureExpiresAt = Time.unscaledTime + windowSec;
            if (clearExisting)
            {
                _tabLines[(int)tab].Clear();
                _tabPlainLines[(int)tab].Clear();
            }
        }

        public bool IngestServerOutput(string plain, string rich)
        {
            if (!_capturingTab.HasValue || string.IsNullOrWhiteSpace(plain))
                return false;
            if (Time.unscaledTime > _captureExpiresAt)
            {
                _capturingTab = null;
                return false;
            }
            string trimmed = plain.Trim();
            if (trimmed.StartsWith("> ", StringComparison.Ordinal) ||
                trimmed.StartsWith("[NET]", StringComparison.Ordinal) ||
                trimmed.StartsWith("[PACK SYNC]", StringComparison.Ordinal))
                return false;

            int idx = (int)_capturingTab.Value;
            string richClean = !string.IsNullOrWhiteSpace(rich) ? rich.TrimEnd('\r') : MushProtocol.EscapeUnityRichText(trimmed);
            _tabPlainLines[idx].Add(trimmed);
            _tabLines[idx].Add(richClean);
            while (_tabPlainLines[idx].Count > MaxLinesPerTab) _tabPlainLines[idx].RemoveAt(0);
            while (_tabLines[idx].Count > MaxLinesPerTab) _tabLines[idx].RemoveAt(0);

            if (IsOpen && ActiveTab == _capturingTab.Value)
                RefreshJournalBody();
            return true;
        }

        public void PinActiveTabSummary()
        {
            int idx = (int)ActiveTab;
            string cat = TabTitles[idx];
            string title;
            string detail;
            if (_tabPlainLines[idx].Count > 0)
            {
                title = _tabPlainLines[idx][0];
                int extra = Mathf.Min(3, _tabPlainLines[idx].Count - 1);
                if (extra > 0)
                {
                    var parts = new List<string>(extra);
                    for (int i = 1; i <= extra; i++)
                        parts.Add(_tabPlainLines[idx][i]);
                    detail = string.Join("\n", parts);
                }
                else
                {
                    detail = $"Run {TabCommands[idx]} to check live progress.";
                }
            }
            else
            {
                title = $"Check {TabTitles[idx]}";
                detail = $"Open [J] or run {TabCommands[idx]} for live objectives.";
            }
            PinObjective(cat, title, detail);
        }

        public void PinObjective(string category, string title, string detail)
        {
            PinnedCategory = (category ?? "OBJECTIVE").Trim().ToUpperInvariant();
            PinnedTitle = (title ?? string.Empty).Trim();
            PinnedDetail = (detail ?? string.Empty).Trim();
            if (_trackerHeaderText != null)
                _trackerHeaderText.text = string.IsNullOrEmpty(PinnedCategory) ? "OBJECTIVE [J]" : PinnedCategory;
            if (_trackerDetailText != null)
            {
                _trackerDetailText.text = $"<b><color=#66EEFF>{MushProtocol.EscapeUnityRichText(PinnedTitle)}</color></b>\n{MushProtocol.EscapeUnityRichText(PinnedDetail)}";
            }
        }

        public void ClearPinnedObjective()
        {
            PinnedCategory = string.Empty;
            PinnedTitle = string.Empty;
            PinnedDetail = string.Empty;
            if (_trackerRoot != null)
                _trackerRoot.SetActive(false);
        }

        public void ToggleTrackerCollapsed()
        {
            _trackerCollapsed = !_trackerCollapsed;
            if (_trackerBody != null) _trackerBody.SetActive(!_trackerCollapsed);
            if (_trackerCollapseLbl != null) _trackerCollapseLbl.text = _trackerCollapsed ? "+" : "—";
            if (_trackerRect != null)
                _trackerRect.offsetMin = new Vector2(-232f, _trackerCollapsed ? -352f : -456f);
        }

        public void SyncTrackerVisibility(TacSimActiveStage stage, bool rightCornerBusy)
        {
            if (_trackerRoot == null) return;
            bool show = HasPinnedObjective &&
                        !rightCornerBusy &&
                        stage != TacSimActiveStage.SimulatorArena;
            if (_trackerRoot.activeSelf != show)
                _trackerRoot.SetActive(show);
        }

        private void RefreshJournalBody()
        {
            int idx = (int)ActiveTab;
            if (_tabHintText != null)
                _tabHintText.text = $"{TabTitles[idx]}  ·  Live command: {TabCommands[idx]}  ({(_tabLines[idx].Count)} lines captured)";
            if (_bodyText != null)
            {
                if (_tabLines[idx].Count > 0)
                    _bodyText.text = string.Join("\n", _tabLines[idx]);
                else
                    _bodyText.text = $"<color=#88AAC8>Click <b>REFRESH TAB</b> to query live <b>{TabCommands[idx]}</b> telemetry from mushd.</color>";
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

        private Button CreateButton(Transform parent, string label, Color bg, Action onClick)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.onClick.AddListener(() => onClick?.Invoke());
            var txt = CreateText(go.transform, label, 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            return btn;
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
            labelTxt = CreateText(go.transform, label, 10, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(labelTxt.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
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
