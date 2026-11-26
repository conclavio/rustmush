using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum SceneIntelTab
    {
        RpScene = 0,
        SignalContacts = 1
    }

    public sealed class SignalContactLedgerRow
    {
        public string Id = string.Empty;
        public string Gen = string.Empty;
        public string Rev = string.Empty;
        public string State = string.Empty;
        public string Age = string.Empty;
        public string Band = string.Empty;
        public string Source = string.Empty;
    }

    public sealed class ActiveRpSceneState
    {
        public long Id;
        public string Title = string.Empty;
        public string Kind = string.Empty;
        public string Mode = string.Empty;
        public string Status = string.Empty;
        public int Round;
        public int Turn;
        public MushObjRef CurrentActor;
        public MushObjRef Facilitator;
        public MushObjRef Anchor;
        public readonly List<MushObjRef> Participants = new List<MushObjRef>();
        public readonly List<MushObjRef> Observers = new List<MushObjRef>();
        public bool HasFullKeyframe;
    }

    /// <summary>
    /// Live RP Scene Tracker Banner &amp; `[L]` 2-Tab Scene / Signal Propagation Intel Modal:
    /// - Tab 0 (`RP SCENE`): consumes live `scenes` panel &amp; `RoomView.ambient["scene"]`, tracks turn order,
    ///   highlights when it is the local pilot's turn, and dispatches verified `+timestop` / `+scene` / `+recap` verbs.
    /// - Tab 1 (`SIGNAL CONTACTS`): consumes live `tactics.contacts` propagation ledger (`id, gen, rev, state, age, band, source`)
    ///   with pooled clickable rows dispatching `+contact &lt;id&gt;` and `+contacts` / `+orders`.
    /// </summary>
    public sealed class TacSimSceneAndIntelModal : MonoBehaviour
    {
        public event Action<string> OnCommandRequested;
        public event Action OnRefreshScenesRequested;
        public event Action OnRefreshContactsRequested;
        public event Action<string> OnLocalPlayerTurnStarted; // scene title

        private const int MaxVisibleContactRows = 8;

        private sealed class PooledContactRow
        {
            public GameObject Root;
            public Image Background;
            public Button Button;
            public Text Label;
            public string ContactId;
        }

        private Font _font;
        private GameObject _sceneBannerRoot;
        private Image _sceneBannerBg;
        private Text _sceneBannerText;

        private GameObject _modalRoot;
        private readonly Image[] _tabImages = new Image[2];
        private readonly GameObject[] _tabBodies = new GameObject[2];

        // Tab 0: RP Scene
        private Text _sceneTitleText;
        private Text _sceneMetaText;
        private Text _sceneRosterText;
        private Button _nextTurnBtn;
        private Button _pauseResumeBtn;
        private Text _pauseResumeLabel;
        private Button _leaveBtn;
        private Button _endBtn;

        // Tab 1: Signal Contacts
        private Text _contactsHeaderText;
        private Text _contactDetailText;
        private Text _contactsEmptyText;
        private readonly List<PooledContactRow> _contactRowPool = new List<PooledContactRow>(MaxVisibleContactRows);
        private readonly List<SignalContactLedgerRow> _contacts = new List<SignalContactLedgerRow>(32);
        private bool _contactsTruncated;
        private string _selectedContactId = string.Empty;
        private readonly List<string> _contactInspectLines = new List<string>(12);
        private bool _capturingContactInspect;
        private float _contactInspectExpiresAt;

        private ActiveRpSceneState _scene;
        private string _localCharId = string.Empty;
        private string _localCharName = string.Empty;
        private long _lastTurnAlertKey = -1;
        private bool _bannerHiddenByStageOrPreset;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public bool IsSceneBannerVisible => _sceneBannerRoot != null && _sceneBannerRoot.activeSelf;
        public SceneIntelTab ActiveTab { get; private set; } = SceneIntelTab.RpScene;
        public ActiveRpSceneState ActiveScene => _scene;
        public int SignalContactCount => _contacts.Count;
        public int VisibleContactRowCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _contactRowPool.Count; i++)
                {
                    if (_contactRowPool[i].Root != null && _contactRowPool[i].Root.activeSelf) n++;
                }
                return n;
            }
        }

        public bool IsLocalPlayersTurn
        {
            get
            {
                if (_scene?.CurrentActor == null) return false;
                if (!string.IsNullOrEmpty(_localCharId) &&
                    string.Equals(_scene.CurrentActor.Id, _localCharId, StringComparison.OrdinalIgnoreCase))
                    return true;
                return !string.IsNullOrEmpty(_localCharName) &&
                       string.Equals(_scene.CurrentActor.Name, _localCharName, StringComparison.OrdinalIgnoreCase);
            }
        }

        public void SetLocalCharacter(string id, string name)
        {
            _localCharId = id ?? string.Empty;
            _localCharName = name ?? string.Empty;
            RefreshSceneUi();
        }

        public void BuildUi(Transform gameplayHudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _contactRowPool.Clear();
            if (gameplayHudRoot == null) return;

            // 1. Live Floating RP Scene Banner (top-center beneath TopStrip; hidden when no scene active or during Arena/Cinematic)
            _sceneBannerRoot = CreatePanel(
                gameplayHudRoot,
                "ActiveRpSceneBanner",
                new Vector2(0.5f, 1f),
                new Vector2(0.5f, 1f),
                new Vector2(-310f, -104f),
                new Vector2(310f, -70f),
                new Color(0.05f, 0.11f, 0.18f, 0.93f));
            _sceneBannerBg = _sceneBannerRoot.GetComponent<Image>();

            _sceneBannerText = CreateText(_sceneBannerRoot.transform, "RP SCENE ACTIVE", 12, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
            _sceneBannerText.supportRichText = true;
            SetRect(_sceneBannerText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(12f, 2f), new Vector2(-102f, -2f));

            var openBtn = CreateButton(_sceneBannerRoot.transform, "SCENE [L]", new Color(0.16f, 0.42f, 0.56f), () =>
            {
                SelectTab(SceneIntelTab.RpScene);
                SetOpen(true);
            });
            SetRect(openBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-96f, 4f), new Vector2(-6f, -4f));
            _sceneBannerRoot.SetActive(false);

            // 2. `[L]` Scene & Signal Contacts Modal
            _modalRoot = CreatePanel(
                gameplayHudRoot,
                "SceneAndSignalIntelModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-430f, -280f),
                new Vector2(430f, 280f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            CreatePanel(_modalRoot.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.36f, 0.92f, 0.78f, 0.95f));

            var hdr = CreateText(_modalRoot.transform, "RP SCENE TRACKER & SIGNAL PROPAGATION INTEL [L]", 17, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.48f, 0.94f, 0.88f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(18f, -42f), new Vector2(-128f, -8f));

            var closeBtn = CreateButton(_modalRoot.transform, "CLOSE [ESC]", new Color(0.45f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -40f), new Vector2(-14f, -10f));

            // Tabs
            var tabStrip = CreatePanel(_modalRoot.transform, "SceneIntelTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(18f, -78f), new Vector2(-18f, -46f), Color.clear);
            var tHlg = tabStrip.AddComponent<HorizontalLayoutGroup>();
            tHlg.spacing = 8f; tHlg.childControlWidth = true; tHlg.childControlHeight = true; tHlg.childForceExpandWidth = true;

            var t0 = CreateButton(tabStrip.transform, "RP SCENE & TURN ORDER", new Color(0.14f, 0.30f, 0.42f), () => SelectTab(SceneIntelTab.RpScene));
            var t1 = CreateButton(tabStrip.transform, "SIGNAL CONTACTS LEDGER", new Color(0.14f, 0.30f, 0.42f), () => SelectTab(SceneIntelTab.SignalContacts));
            _tabImages[0] = t0.GetComponent<Image>();
            _tabImages[1] = t1.GetComponent<Image>();

            _tabBodies[0] = CreatePanel(_modalRoot.transform, "RpSceneTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(18f, 14f), new Vector2(-18f, -86f), Color.clear);
            _tabBodies[1] = CreatePanel(_modalRoot.transform, "SignalContactsTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(18f, 14f), new Vector2(-18f, -86f), Color.clear);

            BuildRpSceneTab(_tabBodies[0].transform);
            BuildSignalContactsTab(_tabBodies[1].transform);

            SelectTab(SceneIntelTab.RpScene);
            _modalRoot.SetActive(false);
        }

        private void BuildRpSceneTab(Transform body)
        {
            var infoCard = CreatePanel(body, "SceneSummaryCard", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -74f), Vector2.zero, new Color(0.05f, 0.10f, 0.16f, 0.94f));
            _sceneTitleText = CreateText(infoCard.transform, "NO ACTIVE RP SCENE IN THIS ROOM", 15, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.90f, 0.96f, 1f));
            _sceneTitleText.supportRichText = true;
            SetRect(_sceneTitleText.rectTransform, new Vector2(0f, 0.48f), new Vector2(1f, 1f), new Vector2(12f, 2f), new Vector2(-12f, -6f));

            _sceneMetaText = CreateText(infoCard.transform, "Click START TIMESTOP (+timestop) to freeze the room into a structured turn-based scene.", 12, FontStyle.Normal, TextAnchor.LowerLeft, new Color(0.72f, 0.86f, 0.94f));
            _sceneMetaText.supportRichText = true;
            SetRect(_sceneMetaText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0.48f), new Vector2(12f, 6f), new Vector2(-12f, -2f));

            var rosterCard = CreatePanel(body, "SceneRosterCard", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(0f, 94f), new Vector2(0f, -82f), new Color(0.02f, 0.05f, 0.09f, 0.92f));
            _sceneRosterText = CreateText(rosterCard.transform, "No participants currently in initiative order.", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.92f, 0.98f));
            _sceneRosterText.supportRichText = true;
            _sceneRosterText.horizontalOverflow = HorizontalWrapMode.Wrap;
            SetRect(_sceneRosterText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 8f), new Vector2(-12f, -8f));

            // Row 1: Turn / Lifecycle Actions
            var row1 = CreatePanel(body, "SceneActionRow1", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(0f, 48f), new Vector2(0f, 86f), Color.clear);
            var h1 = row1.AddComponent<HorizontalLayoutGroup>();
            h1.spacing = 6f; h1.childControlWidth = true; h1.childControlHeight = true; h1.childForceExpandWidth = true;

            CreateButton(row1.transform, "START TIMESTOP (+timestop)", new Color(0.14f, 0.48f, 0.40f), () => OnCommandRequested?.Invoke("+timestop"));
            _nextTurnBtn = CreateButton(row1.transform, "NEXT TURN (+timestop/next)", new Color(0.16f, 0.42f, 0.56f), () => OnCommandRequested?.Invoke("+timestop/next"));
            _pauseResumeBtn = CreateButton(row1.transform, "PAUSE", new Color(0.46f, 0.34f, 0.14f), () =>
            {
                bool paused = _scene != null && string.Equals(_scene.Status, "paused", StringComparison.OrdinalIgnoreCase);
                OnCommandRequested?.Invoke(paused ? "+timestop/resume" : "+timestop/pause");
            });
            _pauseResumeLabel = _pauseResumeBtn.GetComponentInChildren<Text>();
            CreateButton(row1.transform, "REWIND", new Color(0.32f, 0.26f, 0.46f), () => OnCommandRequested?.Invoke("+timestop/rewind"));

            // Row 2: Logs, Recap & Exit
            var row2 = CreatePanel(body, "SceneActionRow2", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(0f, 4f), new Vector2(0f, 42f), Color.clear);
            var h2 = row2.AddComponent<HorizontalLayoutGroup>();
            h2.spacing = 6f; h2.childControlWidth = true; h2.childControlHeight = true; h2.childForceExpandWidth = true;

            CreateButton(row2.transform, "REFRESH (+scene)", new Color(0.14f, 0.30f, 0.44f), () =>
            {
                OnRefreshScenesRequested?.Invoke();
                OnCommandRequested?.Invoke("+scene");
            });
            CreateButton(row2.transform, "TIMELINE (+timestop/log)", new Color(0.16f, 0.34f, 0.46f), () => OnCommandRequested?.Invoke("+timestop/log"));
            CreateButton(row2.transform, "RECAP (+recap)", new Color(0.18f, 0.36f, 0.42f), () => OnCommandRequested?.Invoke("+recap"));
            _leaveBtn = CreateButton(row2.transform, "LEAVE SCENE", new Color(0.44f, 0.26f, 0.18f), () => OnCommandRequested?.Invoke("+timestop/leave"));
            _endBtn = CreateButton(row2.transform, "END SCENE", new Color(0.52f, 0.18f, 0.20f), () => OnCommandRequested?.Invoke("+timestop/end"));
        }

        private void BuildSignalContactsTab(Transform body)
        {
            var topBar = CreatePanel(body, "ContactsTopBar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -34f), Vector2.zero, new Color(0.05f, 0.10f, 0.16f, 0.94f));
            _contactsHeaderText = CreateText(topBar.transform, "SIGNAL PROPAGATION CONTACTS: 0  (CLICK ROW TO INSPECT +contact)", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.55f, 0.92f, 1f));
            SetRect(_contactsHeaderText.rectTransform, new Vector2(0f, 0f), new Vector2(0.68f, 1f), new Vector2(10f, 0f), Vector2.zero);

            var ordersBtn = CreateButton(topBar.transform, "+orders", new Color(0.16f, 0.34f, 0.46f), () => OnCommandRequested?.Invoke("+orders"));
            SetRect(ordersBtn.GetComponent<RectTransform>(), new Vector2(0.70f, 0f), new Vector2(0.84f, 1f), new Vector2(2f, 3f), new Vector2(-2f, -3f));

            var refBtn = CreateButton(topBar.transform, "REFRESH", new Color(0.14f, 0.44f, 0.40f), () =>
            {
                OnRefreshContactsRequested?.Invoke();
                OnCommandRequested?.Invoke("+contacts");
            });
            SetRect(refBtn.GetComponent<RectTransform>(), new Vector2(0.85f, 0f), new Vector2(1f, 1f), new Vector2(2f, 3f), new Vector2(-6f, -3f));

            var listBox = CreatePanel(body, "ContactsLedgerBox", new Vector2(0f, 0.30f), new Vector2(1f, 1f), new Vector2(0f, 6f), new Vector2(0f, -40f), new Color(0.02f, 0.05f, 0.09f, 0.90f));
            var vlg = listBox.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxVisibleContactRows; i++)
            {
                var row = new PooledContactRow();
                var btn = CreateButton(listBox.transform, string.Empty, new Color(0.08f, 0.15f, 0.22f, 0.94f), () =>
                {
                    InspectContactById(row.ContactId);
                });
                btn.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 28f);
                var lbl = btn.GetComponentInChildren<Text>();
                if (lbl != null)
                {
                    lbl.fontSize = 11;
                    lbl.alignment = TextAnchor.MiddleLeft;
                    lbl.supportRichText = true;
                    SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 0f), new Vector2(-10f, 0f));
                }
                row.Root = btn.gameObject;
                row.Button = btn;
                row.Background = btn.GetComponent<Image>();
                row.Label = lbl;
                row.Root.SetActive(false);
                _contactRowPool.Add(row);
            }

            _contactsEmptyText = CreateText(listBox.transform, "No propagation contacts currently held by your platform sensors.", 12, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.62f, 0.74f, 0.84f));
            _contactsEmptyText.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 28f);

            var detailBox = CreatePanel(body, "ContactDetailBox", new Vector2(0f, 0f), new Vector2(1f, 0.29f), new Vector2(0f, 4f), new Vector2(0f, -2f), new Color(0.04f, 0.08f, 0.13f, 0.94f));
            _contactDetailText = CreateText(detailBox.transform, "Select a contact row above to inspect its full light-delay provenance (+contact <id>).", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.84f, 0.92f, 0.98f));
            _contactDetailText.supportRichText = true;
            _contactDetailText.horizontalOverflow = HorizontalWrapMode.Wrap;
            SetRect(_contactDetailText.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 6f), new Vector2(-10f, -6f));
        }

        public void SelectTab(SceneIntelTab tab)
        {
            ActiveTab = tab;
            for (int i = 0; i < 2; i++)
            {
                bool active = i == (int)tab;
                if (_tabBodies[i] != null) _tabBodies[i].SetActive(active);
                if (_tabImages[i] != null)
                    _tabImages[i].color = active ? new Color(0.18f, 0.52f, 0.62f, 1f) : new Color(0.12f, 0.20f, 0.30f, 0.92f);
            }
            if (IsOpen)
            {
                if (tab == SceneIntelTab.RpScene) OnRefreshScenesRequested?.Invoke();
                else OnRefreshContactsRequested?.Invoke();
            }
        }

        public void SetOpen(bool open)
        {
            if (_modalRoot == null) return;
            _modalRoot.SetActive(open);
            if (open)
            {
                _modalRoot.transform.SetAsLastSibling();
                if (ActiveTab == SceneIntelTab.RpScene) OnRefreshScenesRequested?.Invoke();
                else OnRefreshContactsRequested?.Invoke();
                RefreshSceneUi();
                RefreshContactsUi();
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void SetBannerSuppressed(bool suppressed)
        {
            _bannerHiddenByStageOrPreset = suppressed;
            RefreshSceneBannerVisibility();
        }

        public void ResetSessionState()
        {
            _scene = null;
            _contacts.Clear();
            _contactsTruncated = false;
            _selectedContactId = string.Empty;
            _contactInspectLines.Clear();
            _capturingContactInspect = false;
            _lastTurnAlertKey = -1;
            if (_modalRoot != null) _modalRoot.SetActive(false);
            if (_sceneBannerRoot != null) _sceneBannerRoot.SetActive(false);
            RefreshSceneUi();
            RefreshContactsUi();
        }

        /// <summary>
        /// Applies the full `scenes` panel keyframe (or `null` when the player is no longer in any scene).
        /// </summary>
        public void ApplyScenesPanel(JsonNode data)
        {
            if (data == null || data.IsNull || !data.IsObject || !data.ContainsKey("id"))
            {
                _scene = null;
                _lastTurnAlertKey = -1;
                RefreshSceneUi();
                return;
            }

            var st = new ActiveRpSceneState
            {
                Id = data.Long("id", 0),
                Title = data.Str("title", string.Empty),
                Kind = data.Str("kind", string.Empty),
                Mode = data.Str("mode", string.Empty),
                Status = data.Str("status", "active"),
                Round = data.Int("round", 1),
                Turn = data.Int("turn", 0),
                CurrentActor = MushProtocol.ParseObjRef(data["current_actor"]),
                Facilitator = MushProtocol.ParseObjRef(data["facilitator"]),
                Anchor = MushProtocol.ParseObjRef(data["anchor"]),
                HasFullKeyframe = true
            };

            ParseObjRefArray(data["participants"].AsArray(), st.Participants);
            ParseObjRefArray(data["observers"].AsArray(), st.Observers);

            _scene = st;
            CheckLocalTurnTransition();
            RefreshSceneUi();
        }

        /// <summary>
        /// Applies the lightweight `RoomView.ambient["scene"]` glance when walking into a room with an active scene.
        /// Never overwrites a full `scenes` keyframe of the same scene ID.
        /// </summary>
        public void ApplyRoomSceneGlance(MushSceneAmbient glance)
        {
            if (glance == null)
            {
                if (_scene != null && !_scene.HasFullKeyframe)
                {
                    _scene = null;
                    RefreshSceneUi();
                }
                return;
            }

            if (_scene != null && _scene.HasFullKeyframe && _scene.Id == glance.Id)
            {
                _scene.Status = glance.Status;
                _scene.Round = glance.Round;
                _scene.Turn = glance.Turn;
                _scene.CurrentActor = glance.CurrentActor;
            }
            else
            {
                _scene = new ActiveRpSceneState
                {
                    Id = glance.Id,
                    Title = glance.Title,
                    Kind = glance.Kind,
                    Mode = glance.Mode,
                    Status = glance.Status,
                    Round = glance.Round,
                    Turn = glance.Turn,
                    CurrentActor = glance.CurrentActor,
                    HasFullKeyframe = false
                };
            }

            CheckLocalTurnTransition();
            RefreshSceneUi();
        }

        /// <summary>
        /// Applies the live `tactics.contacts` signal-propagation ledger panel frame.
        /// </summary>
        public void ApplySignalContactsPanel(JsonNode data)
        {
            _contacts.Clear();
            _contactsTruncated = false;
            if (data != null && data.IsObject)
            {
                _contactsTruncated = data.Bool("truncated", false);
                var rows = data["rows"].AsArray();
                if (rows != null)
                {
                    int cap = Math.Min(rows.Count, 256);
                    for (int i = 0; i < cap; i++)
                    {
                        var r = rows[i];
                        if (!r.IsObject) continue;
                        string id = r.Str("id", string.Empty);
                        if (string.IsNullOrEmpty(id)) continue;
                        _contacts.Add(new SignalContactLedgerRow
                        {
                            Id = id,
                            Gen = r["gen"].Kind == JsonKind.String ? r.Str("gen", "0") : r.Int("gen", 0).ToString(),
                            Rev = r["rev"].Kind == JsonKind.String ? r.Str("rev", "0") : r.Int("rev", 0).ToString(),
                            State = r.Str("state", string.Empty),
                            Age = r.Str("age", string.Empty),
                            Band = r.Str("band", string.Empty),
                            Source = r.Str("source", string.Empty)
                        });
                    }
                }
            }
            RefreshContactsUi();
        }

        public void InspectContactById(string rawContactId)
        {
            if (!MushProtocol.TryValidateSafeCommandToken(rawContactId, out string safeId, maxLen: 24))
                return;
            _selectedContactId = safeId;
            _contactInspectLines.Clear();
            _capturingContactInspect = true;
            _contactInspectExpiresAt = Time.unscaledTime + 2.5f;
            if (_contactDetailText != null)
                _contactDetailText.text = $"Querying light-delay contact dossier for <b>{MushProtocol.EscapeUnityRichText(safeId)}</b> (`+contact {MushProtocol.EscapeUnityRichText(safeId)}`)...";
            RefreshContactsUi();
            OnCommandRequested?.Invoke($"+contact {safeId}");
        }

        public void IngestServerOutput(string plainLine, string richMarkup)
        {
            if (!_capturingContactInspect || string.IsNullOrWhiteSpace(plainLine)) return;
            if (Application.isPlaying && Time.unscaledTime > _contactInspectExpiresAt)
            {
                _capturingContactInspect = false;
                return;
            }
            string formatted = !string.IsNullOrEmpty(richMarkup) ? richMarkup : MushProtocol.EscapeUnityRichText(plainLine);
            _contactInspectLines.Add(formatted);
            while (_contactInspectLines.Count > 6)
                _contactInspectLines.RemoveAt(0);
            if (_contactDetailText != null)
                _contactDetailText.text = string.Join("\n", _contactInspectLines);
        }

        private void CheckLocalTurnTransition()
        {
            if (_scene == null || !IsLocalPlayersTurn) return;
            long key = (_scene.Id * 100000L) + (_scene.Round * 1000L) + _scene.Turn;
            if (key != _lastTurnAlertKey)
            {
                _lastTurnAlertKey = key;
                OnLocalPlayerTurnStarted?.Invoke(string.IsNullOrEmpty(_scene.Title) ? $"Scene #{_scene.Id}" : _scene.Title);
            }
        }

        private void RefreshSceneBannerVisibility()
        {
            if (_sceneBannerRoot == null) return;
            bool show = _scene != null && !_bannerHiddenByStageOrPreset;
            if (_sceneBannerRoot.activeSelf != show)
                _sceneBannerRoot.SetActive(show);
        }

        private void RefreshSceneUi()
        {
            RefreshSceneBannerVisibility();

            if (_scene == null)
            {
                if (_sceneTitleText != null) _sceneTitleText.text = "NO ACTIVE RP SCENE IN THIS ROOM";
                if (_sceneMetaText != null) _sceneMetaText.text = "Click START TIMESTOP (+timestop) to freeze the room into a structured turn-based scene.";
                if (_sceneRosterText != null) _sceneRosterText.text = "No participants currently in initiative order.";
                if (_nextTurnBtn != null) _nextTurnBtn.interactable = false;
                if (_pauseResumeBtn != null) _pauseResumeBtn.interactable = false;
                if (_leaveBtn != null) _leaveBtn.interactable = false;
                if (_endBtn != null) _endBtn.interactable = false;
                return;
            }

            string title = string.IsNullOrEmpty(_scene.Title) ? $"Scene #{_scene.Id}" : _scene.Title;
            string safeTitle = MushProtocol.EscapeUnityRichText(title);
            string safeStatus = MushProtocol.EscapeUnityRichText((_scene.Status ?? "active").ToUpperInvariant());
            string actorName = _scene.CurrentActor != null && !string.IsNullOrEmpty(_scene.CurrentActor.Name)
                ? MushProtocol.EscapeUnityRichText(_scene.CurrentActor.Name)
                : "—";
            bool myTurn = IsLocalPlayersTurn;

            if (_sceneBannerText != null)
            {
                string turnBadge = myTurn
                    ? "<color=#55FF99><b>YOUR TURN!</b></color>"
                    : $"TURN: <color=#88DDFF><b>{actorName}</b></color>";
                _sceneBannerText.text = $"<color=#FFCC55>[SCENE #{_scene.Id}]</color> <b>{safeTitle}</b> · R{_scene.Round} [{safeStatus}] · {turnBadge}";
            }
            if (_sceneBannerBg != null)
            {
                _sceneBannerBg.color = myTurn
                    ? new Color(0.08f, 0.28f, 0.20f, 0.95f)
                    : new Color(0.05f, 0.11f, 0.18f, 0.93f);
            }

            if (_sceneTitleText != null)
                _sceneTitleText.text = $"SCENE #{_scene.Id}: <b>{safeTitle}</b>  <color=#FFCC55>[{safeStatus}]</color>";
            if (_sceneMetaText != null)
            {
                string fac = _scene.Facilitator != null ? MushProtocol.EscapeUnityRichText(_scene.Facilitator.Name) : "—";
                string anc = _scene.Anchor != null ? MushProtocol.EscapeUnityRichText(_scene.Anchor.Name) : "Current Room";
                _sceneMetaText.text =
                    $"Round <b>{_scene.Round}</b> · Turn <b>{_scene.Turn + 1}</b> · Current Actor: <b>{actorName}</b>{(myTurn ? " <color=#55FF99>[YOU]</color>" : "")} · Facilitator: <b>{fac}</b> · Anchor: <b>{anc}</b>";
            }

            if (_sceneRosterText != null)
            {
                var sb = new StringBuilder(256);
                sb.AppendLine("<color=#66EECC><b>TURN ORDER PARTICIPANTS</b></color>");
                if (_scene.Participants.Count == 0)
                {
                    sb.AppendLine($"  Current Turn: <b>{actorName}</b> (click REFRESH (+scene) for full turn roster)");
                }
                else
                {
                    for (int i = 0; i < _scene.Participants.Count; i++)
                    {
                        var p = _scene.Participants[i];
                        bool isCur = _scene.CurrentActor != null && string.Equals(p.Id, _scene.CurrentActor.Id, StringComparison.OrdinalIgnoreCase);
                        bool isMe = !string.IsNullOrEmpty(_localCharId) && string.Equals(p.Id, _localCharId, StringComparison.OrdinalIgnoreCase);
                        string marker = isCur ? "<color=#55FF99>>> </color>" : "   ";
                        string youTag = isMe ? " <color=#FFEE66>[YOU]</color>" : string.Empty;
                        sb.AppendLine($"{marker}{i + 1}. <b>{MushProtocol.EscapeUnityRichText(p.Name)}</b>{youTag}{(isCur ? "  <color=#55FF99>[ACTIVE TURN]</color>" : "")}");
                    }
                }
                if (_scene.Observers.Count > 0)
                {
                    sb.Append("\n<color=#99BBCC><b>OBSERVERS:</b> ");
                    for (int i = 0; i < _scene.Observers.Count; i++)
                    {
                        if (i > 0) sb.Append(", ");
                        sb.Append(MushProtocol.EscapeUnityRichText(_scene.Observers[i].Name));
                    }
                    sb.Append("</color>");
                }
                _sceneRosterText.text = sb.ToString();
            }

            bool paused = string.Equals(_scene.Status, "paused", StringComparison.OrdinalIgnoreCase);
            if (_nextTurnBtn != null) _nextTurnBtn.interactable = true;
            if (_pauseResumeBtn != null) _pauseResumeBtn.interactable = true;
            if (_pauseResumeLabel != null) _pauseResumeLabel.text = paused ? "RESUME (+timestop/resume)" : "PAUSE (+timestop/pause)";
            if (_leaveBtn != null) _leaveBtn.interactable = true;
            if (_endBtn != null) _endBtn.interactable = true;
        }

        private void RefreshContactsUi()
        {
            if (_contactsHeaderText != null)
            {
                string trunc = _contactsTruncated ? " (TRUNCATED)" : string.Empty;
                _contactsHeaderText.text = $"SIGNAL PROPAGATION CONTACTS: {_contacts.Count}{trunc}  (CLICK ROW TO INSPECT)";
            }

            int shown = Mathf.Min(_contacts.Count, _contactRowPool.Count);
            for (int i = 0; i < shown; i++)
            {
                var c = _contacts[i];
                var slot = _contactRowPool[i];
                bool selected = string.Equals(c.Id, _selectedContactId, StringComparison.OrdinalIgnoreCase);
                string stateCol = c.State.IndexOf("live", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                  c.State.IndexOf("active", StringComparison.OrdinalIgnoreCase) >= 0
                    ? "#55FF99"
                    : (c.State.IndexOf("stale", StringComparison.OrdinalIgnoreCase) >= 0 ? "#FFBB55" : "#99BBCC");
                slot.ContactId = c.Id;
                slot.Label.text =
                    $"<b>{MushProtocol.EscapeUnityRichText(c.Id)}</b>  [g{MushProtocol.EscapeUnityRichText(c.Gen)}.r{MushProtocol.EscapeUnityRichText(c.Rev)}]  " +
                    $"<color={stateCol}><b>{MushProtocol.EscapeUnityRichText(c.State.ToUpperInvariant())}</b></color>  " +
                    $"AGE: {MushProtocol.EscapeUnityRichText(c.Age)}  |  BAND: {MushProtocol.EscapeUnityRichText(c.Band)}  |  SRC: {MushProtocol.EscapeUnityRichText(c.Source)}";
                slot.Background.color = selected
                    ? new Color(0.18f, 0.36f, 0.48f, 0.96f)
                    : new Color(0.08f, 0.14f, 0.22f, 0.92f);
                if (!slot.Root.activeSelf) slot.Root.SetActive(true);
            }
            for (int i = shown; i < _contactRowPool.Count; i++)
            {
                if (_contactRowPool[i].Root.activeSelf) _contactRowPool[i].Root.SetActive(false);
            }
            if (_contactsEmptyText != null)
                _contactsEmptyText.gameObject.SetActive(shown == 0);
        }

        private static void ParseObjRefArray(List<JsonNode> arr, List<MushObjRef> dest)
        {
            if (arr == null) return;
            int cap = Math.Min(arr.Count, 48);
            for (int i = 0; i < cap; i++)
            {
                var o = MushProtocol.ParseObjRef(arr[i]);
                if (o != null) dest.Add(o);
            }
        }

        private static GameObject CreatePanel(Transform parent, string name, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax, Color bg)
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
            var go = new GameObject("Txt", typeof(RectTransform), typeof(Text));
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

        private static void SetRect(RectTransform rt, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax)
        {
            rt.anchorMin = aMin;
            rt.anchorMax = aMax;
            rt.offsetMin = oMin;
            rt.offsetMax = oMax;
        }
    }
}
