using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum SocialModalTab
    {
        OnlineWho = 0,
        Mail = 1,
        Channels = 2
    }

    public sealed class MailFolderSummary
    {
        public string Name = "inbox";
        public int Count;
        public int Unread;
    }

    public sealed class MailInboxEntry
    {
        public int Index;
        public long Id;
        public string FromName = string.Empty;
        public string Subject = string.Empty;
        public string Status = "read"; // "urgent" | "unread" | "read"
        public string Delivery = string.Empty;
    }

    public sealed class ComsysChannelInfo
    {
        public string Name = string.Empty;
        public string Alias = string.Empty;
        public string Title = string.Empty;
        public bool Gagged;
        public bool IsIc;
        public bool Logged;
        public int UnreadCount;
        public ulong LastSeq;
        public bool BacklogLoaded;
        public string ColorHex = string.Empty;
        public readonly List<string> RecentLines = new List<string>();
    }

    /// <summary>
    /// Live MMORPG `[O]` Social, Online Pilots (`WHO`), IC Mail (`@mail` / `mail.list`),
    /// and Comsys Channels (`comsys` / `channel_backlog` / `clist` / `addcom`) window.
    /// Zero offline fake entries: every list is driven directly by authoritative `mushd` frames.
    /// </summary>
    public sealed class TacSimSocialAndMailModal : MonoBehaviour
    {
        public event Action<string> OnCommandRequested;
        public event Action<string> OnPrefillChatRequested;
        public event Action OnRefreshWhoRequested;
        public event Action<string> OnRefreshMailRequested;
        public event Action OnRefreshComsysRequested;
        public event Action<string> OnChannelBacklogRequested;
        public event Action<int> OnUnreadMailCountChanged;

        private const int MaxWhoRows = 14;
        private const int MaxMailRows = 10;
        private const int MaxChannelRows = 10;

        private sealed class PooledWhoRow
        {
            public GameObject Root;
            public Text Label;
            public Button PageBtn;
            public Button MailBtn;
            public string BoundName;
        }

        private sealed class PooledMailRow
        {
            public GameObject Root;
            public Image Bg;
            public Text Label;
            public Button ReadBtn;
            public Button DeleteBtn;
            public int BoundIndex;
        }

        private sealed class PooledChannelRow
        {
            public GameObject Root;
            public Image Bg;
            public Text Label;
            public Button SelectBtn;
            public Button GagBtn;
            public Text GagBtnLabel;
            public string BoundName;
            public string BoundAlias;
            public bool BoundGagged;
        }

        private Font _font;
        private GameObject _modalRoot;
        private Text _headerText;
        private readonly List<Image> _tabImages = new List<Image>();
        private readonly GameObject[] _tabBodies = new GameObject[3];

        // Tab 0: WHO
        private Text _whoSummaryText;
        private readonly List<PooledWhoRow> _whoPool = new List<PooledWhoRow>(MaxWhoRows);
        private readonly List<MushWhoEntry> _whoEntries = new List<MushWhoEntry>();

        // Tab 1: Mail
        private Text _mailFolderStatusText;
        private Text _mailReaderText;
        private InputField _mailToInput;
        private InputField _mailSubjInput;
        private InputField _mailBodyInput;
        private readonly List<PooledMailRow> _mailPool = new List<PooledMailRow>(MaxMailRows);
        private readonly List<MailFolderSummary> _mailFolders = new List<MailFolderSummary>();
        private readonly List<MailInboxEntry> _mailEntries = new List<MailInboxEntry>();
        private readonly List<string> _mailReaderLines = new List<string>();
        private string _activeMailFolder = "inbox";
        private string _mailQuotaLabel = string.Empty;
        private float _mailCaptureUntilUnscaled;

        // Tab 2: Channels
        private Text _channelTranscriptText;
        private Text _activeChanHeaderText;
        private InputField _chanSpeakInput;
        private InputField _chanJoinAliasInput;
        private InputField _chanJoinNameInput;
        private readonly List<PooledChannelRow> _chanPool = new List<PooledChannelRow>(MaxChannelRows);
        private readonly List<ComsysChannelInfo> _channels = new List<ComsysChannelInfo>();
        private string _selectedChannelName = string.Empty;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public SocialModalTab ActiveTab { get; private set; } = SocialModalTab.OnlineWho;
        public int UnreadMailCount { get; private set; }
        public int OnlinePilotCount => _whoEntries.Count;
        public int MailEntryCount => _mailEntries.Count;
        public int ChannelCount => _channels.Count;
        public string ActiveMailFolder => _activeMailFolder;
        public string SelectedChannelName => _selectedChannelName;

        public bool IsAnyInputFocused
        {
            get
            {
                if (!IsOpen) return false;
                return (_mailToInput != null && _mailToInput.isFocused) ||
                       (_mailSubjInput != null && _mailSubjInput.isFocused) ||
                       (_mailBodyInput != null && _mailBodyInput.isFocused) ||
                       (_chanSpeakInput != null && _chanSpeakInput.isFocused) ||
                       (_chanJoinAliasInput != null && _chanJoinAliasInput.isFocused) ||
                       (_chanJoinNameInput != null && _chanJoinNameInput.isFocused);
            }
        }

        public static string FormatDurationShort(int seconds)
        {
            if (seconds < 60) return $"{Mathf.Max(0, seconds)}s";
            if (seconds < 3600) return $"{seconds / 60}m";
            int hrs = seconds / 3600;
            int mins = (seconds % 3600) / 60;
            return $"{hrs}h{mins:00}m";
        }

        public static bool TryBuildQuickMailCommand(string rawTo, string rawSubject, string rawBody, out string command, out string error)
        {
            command = null;
            error = null;
            string to = MushProtocol.StripControlChars(rawTo ?? string.Empty).Trim();
            string subj = MushProtocol.StripControlChars(rawSubject ?? string.Empty).Trim();
            string body = MushProtocol.StripControlChars(rawBody ?? string.Empty).Trim();

            if (string.IsNullOrEmpty(to) || to.IndexOf('=') >= 0)
            {
                error = "Enter a valid recipient name without '='.";
                return false;
            }
            if (string.IsNullOrEmpty(subj) || subj.IndexOf('/') >= 0)
            {
                error = "Enter a subject without '/'.";
                return false;
            }
            if (string.IsNullOrEmpty(body))
            {
                error = "Enter a message body.";
                return false;
            }
            command = $"@mail/quick {to}={subj}/{body}";
            return true;
        }

        public void BuildModal(Transform parent, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _tabImages.Clear();
            _whoPool.Clear();
            _mailPool.Clear();
            _chanPool.Clear();

            _modalRoot = CreatePanel(
                parent,
                "SocialMailCommsModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-470f, -305f),
                new Vector2(470f, 305f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            CreatePanel(_modalRoot.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.32f, 0.92f, 0.78f, 0.95f));

            _headerText = CreateText(_modalRoot.transform, "SOCIAL, IC MAIL & COMSYS CHANNELS [O]", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.94f, 0.88f));
            SetRect(_headerText.rectTransform, new Vector2(0f, 1f), new Vector2(0.75f, 1f), new Vector2(18f, -42f), new Vector2(0f, -8f));

            var closeBtn = CreateButton(_modalRoot.transform, "CLOSE [ESC]", new Color(0.46f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -40f), new Vector2(-14f, -10f));

            // Tabs
            var tabStrip = CreatePanel(_modalRoot.transform, "SocialTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -80f), new Vector2(-16f, -46f), Color.clear);
            var hlg = tabStrip.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 6f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            string[] labels = { "ONLINE PILOTS (WHO)", "IC MAIL (@mail)", "CHANNELS (comsys)" };
            for (int i = 0; i < labels.Length; i++)
            {
                var tab = (SocialModalTab)i;
                var tb = CreateButton(tabStrip.transform, labels[i], new Color(0.11f, 0.22f, 0.34f), () => SelectTab(tab, autoRefresh: true));
                _tabImages.Add(tb.GetComponent<Image>());
            }

            BuildWhoTab(_modalRoot.transform);
            BuildMailTab(_modalRoot.transform);
            BuildChannelsTab(_modalRoot.transform);

            SelectTab(SocialModalTab.OnlineWho, autoRefresh: false);
            _modalRoot.SetActive(false);
        }

        private void BuildWhoTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabOnlineWho", Vector2.zero, Vector2.one, new Vector2(16f, 14f), new Vector2(-16f, -88f), new Color(0.04f, 0.08f, 0.13f, 0.88f));
            _tabBodies[0] = body;

            _whoSummaryText = CreateText(body.transform, "0 pilots online — click REFRESH WHO to poll live roster.", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.75f, 0.90f, 0.98f));
            SetRect(_whoSummaryText.rectTransform, new Vector2(0f, 1f), new Vector2(0.76f, 1f), new Vector2(12f, -34f), new Vector2(0f, -6f));

            var refBtn = CreateButton(body.transform, "REFRESH WHO", new Color(0.14f, 0.42f, 0.52f), () => OnRefreshWhoRequested?.Invoke());
            SetRect(refBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-132f, -34f), new Vector2(-12f, -6f));

            var listArea = CreatePanel(body.transform, "WhoListArea", Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -40f), new Color(0.02f, 0.05f, 0.09f, 0.90f));
            var vlg = listArea.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxWhoRows; i++)
            {
                var rowGo = CreatePanel(listArea.transform, $"WhoRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 29f), new Color(0.06f, 0.12f, 0.19f, 0.92f));
                rowGo.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 29f);
                var lbl = CreateText(rowGo.transform, string.Empty, 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.90f, 0.96f, 1f));
                lbl.supportRichText = true;
                SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 0f), new Vector2(-152f, 0f));

                var row = new PooledWhoRow { Root = rowGo, Label = lbl };
                row.PageBtn = CreateButton(rowGo.transform, "PAGE", new Color(0.16f, 0.42f, 0.50f), () =>
                {
                    if (string.IsNullOrEmpty(row.BoundName)) return;
                    SetOpen(false);
                    OnPrefillChatRequested?.Invoke($"page {row.BoundName}=");
                });
                SetRect(row.PageBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-144f, 3f), new Vector2(-78f, -3f));

                row.MailBtn = CreateButton(rowGo.transform, "MAIL", new Color(0.20f, 0.46f, 0.36f), () =>
                {
                    if (string.IsNullOrEmpty(row.BoundName)) return;
                    SelectTab(SocialModalTab.Mail, autoRefresh: true);
                    if (_mailToInput != null)
                        _mailToInput.text = row.BoundName;
                });
                SetRect(row.MailBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-72f, 3f), new Vector2(-6f, -3f));

                rowGo.SetActive(false);
                _whoPool.Add(row);
            }
        }

        private void BuildMailTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabMail", Vector2.zero, Vector2.one, new Vector2(16f, 14f), new Vector2(-16f, -88f), new Color(0.04f, 0.08f, 0.13f, 0.88f));
            _tabBodies[1] = body;

            // Folder bar
            var fBar = CreatePanel(body.transform, "MailFolderBar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(12f, -36f), new Vector2(-12f, -6f), Color.clear);
            var fHlg = fBar.AddComponent<HorizontalLayoutGroup>();
            fHlg.spacing = 6f;
            fHlg.childControlWidth = true;
            fHlg.childControlHeight = true;
            fHlg.childForceExpandWidth = true;

            CreateButton(fBar.transform, "INBOX", new Color(0.14f, 0.38f, 0.50f), () => SwitchMailFolder("inbox"));
            CreateButton(fBar.transform, "SENT", new Color(0.14f, 0.28f, 0.40f), () => SwitchMailFolder("sent"));
            CreateButton(fBar.transform, "ARCHIVE", new Color(0.14f, 0.28f, 0.40f), () => SwitchMailFolder("archive"));
            CreateButton(fBar.transform, "REFRESH", new Color(0.16f, 0.46f, 0.38f), () => OnRefreshMailRequested?.Invoke(_activeMailFolder));
            CreateButton(fBar.transform, "PURGE TRASH", new Color(0.46f, 0.20f, 0.22f), () =>
            {
                OnCommandRequested?.Invoke("@mail/purge");
                OnRefreshMailRequested?.Invoke(_activeMailFolder);
            });

            _mailFolderStatusText = CreateText(body.transform, "Folder: INBOX — All mail travels in character.", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.74f, 0.88f, 0.96f));
            SetRect(_mailFolderStatusText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(14f, -60f), new Vector2(-14f, -38f));

            // Left: Inbox list (top-left)
            var listBox = CreatePanel(body.transform, "MailListBox", new Vector2(0f, 0.32f), new Vector2(0.55f, 1f), new Vector2(12f, 4f), new Vector2(-6f, -64f), new Color(0.02f, 0.05f, 0.09f, 0.92f));
            var vlg = listBox.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 3f;
            vlg.padding = new RectOffset(5, 5, 5, 5);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxMailRows; i++)
            {
                var rowGo = CreatePanel(listBox.transform, $"MailRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 26f), new Color(0.06f, 0.12f, 0.18f, 0.92f));
                rowGo.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 26f);
                var lbl = CreateText(rowGo.transform, string.Empty, 11, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
                lbl.supportRichText = true;
                SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-104f, 0f));

                var row = new PooledMailRow { Root = rowGo, Bg = rowGo.GetComponent<Image>(), Label = lbl };
                row.ReadBtn = CreateButton(rowGo.transform, "READ", new Color(0.16f, 0.44f, 0.56f), () =>
                {
                    if (row.BoundIndex <= 0) return;
                    _mailReaderLines.Clear();
                    _mailCaptureUntilUnscaled = Time.unscaledTime + 4.0f;
                    OnCommandRequested?.Invoke($"@mail {row.BoundIndex}");
                    OnRefreshMailRequested?.Invoke(_activeMailFolder);
                });
                SetRect(row.ReadBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-98f, 2f), new Vector2(-50f, -2f));

                row.DeleteBtn = CreateButton(rowGo.transform, "DEL", new Color(0.44f, 0.18f, 0.20f), () =>
                {
                    if (row.BoundIndex <= 0) return;
                    OnCommandRequested?.Invoke($"@mail/delete {row.BoundIndex}");
                    OnRefreshMailRequested?.Invoke(_activeMailFolder);
                });
                SetRect(row.DeleteBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-46f, 2f), new Vector2(-4f, -2f));

                rowGo.SetActive(false);
                _mailPool.Add(row);
            }

            // Right: Message Reader pane
            var readBox = CreatePanel(body.transform, "MailReaderBox", new Vector2(0.55f, 0.32f), new Vector2(1f, 1f), new Vector2(6f, 4f), new Vector2(-12f, -64f), new Color(0.02f, 0.05f, 0.09f, 0.92f));
            _mailReaderText = CreateText(readBox.transform, "<color=#88AAC8>Select a message and click READ to inspect IC mail contents.</color>", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.94f, 0.98f));
            _mailReaderText.supportRichText = true;
            _mailReaderText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _mailReaderText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_mailReaderText.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 8f), new Vector2(-10f, -8f));

            // Bottom: One-Shot Quick Mail Composer (@mail/quick)
            var compBox = CreatePanel(body.transform, "MailComposeBox", new Vector2(0f, 0f), new Vector2(1f, 0.31f), new Vector2(12f, 10f), new Vector2(-12f, -4f), new Color(0.05f, 0.10f, 0.16f, 0.95f));
            var cHdr = CreateText(compBox.transform, "QUICK IC MAIL DISPATCH (@mail/quick)", 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.85f, 0.45f));
            SetRect(cHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -22f), new Vector2(-10f, -2f));

            _mailToInput = CreateInputField(compBox.transform, "Recipient pilot...", new Vector2(0f, 1f), new Vector2(0.32f, 1f), new Vector2(10f, -54f), new Vector2(-4f, -24f));
            _mailSubjInput = CreateInputField(compBox.transform, "Subject (no '/')...", new Vector2(0.32f, 1f), new Vector2(1f, 1f), new Vector2(4f, -54f), new Vector2(-10f, -24f));
            _mailBodyInput = CreateInputField(compBox.transform, "Message body...", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 10f), new Vector2(-132f, -60f));

            var sendBtn = CreateButton(compBox.transform, "SEND IC MAIL", new Color(0.14f, 0.50f, 0.38f), SubmitQuickMail);
            SetRect(sendBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-124f, 10f), new Vector2(-10f, -60f));
        }

        private void BuildChannelsTab(Transform parent)
        {
            var body = CreatePanel(parent, "TabChannels", Vector2.zero, Vector2.one, new Vector2(16f, 14f), new Vector2(-16f, -88f), new Color(0.04f, 0.08f, 0.13f, 0.88f));
            _tabBodies[2] = body;

            // Left: Joined Channels list + Join bar
            var leftPane = CreatePanel(body.transform, "ChannelListPane", new Vector2(0f, 0f), new Vector2(0.38f, 1f), new Vector2(12f, 10f), new Vector2(-6f, -10f), new Color(0.02f, 0.05f, 0.09f, 0.92f));
            var cTop = CreatePanel(leftPane.transform, "ChanTopActs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(6f, -34f), new Vector2(-6f, -6f), Color.clear);
            var ctHlg = cTop.AddComponent<HorizontalLayoutGroup>();
            ctHlg.spacing = 5f;
            ctHlg.childControlWidth = true;
            ctHlg.childControlHeight = true;
            ctHlg.childForceExpandWidth = true;
            CreateButton(cTop.transform, "REFRESH", new Color(0.14f, 0.40f, 0.52f), () => OnRefreshComsysRequested?.Invoke());
            CreateButton(cTop.transform, "LIST ALL (clist)", new Color(0.18f, 0.34f, 0.48f), () => OnCommandRequested?.Invoke("clist"));

            var listArea = CreatePanel(leftPane.transform, "JoinedChanRows", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(4f, 68f), new Vector2(-4f, -38f), Color.clear);
            var vlg = listArea.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(4, 4, 4, 4);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxChannelRows; i++)
            {
                var rowGo = CreatePanel(listArea.transform, $"ChanRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 28f), new Color(0.07f, 0.13f, 0.20f, 0.92f));
                rowGo.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 28f);
                var img = rowGo.GetComponent<Image>();
                img.raycastTarget = true;
                var selBtn = rowGo.AddComponent<Button>();
                selBtn.targetGraphic = img;

                var lbl = CreateText(rowGo.transform, string.Empty, 11, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
                lbl.supportRichText = true;
                SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-62f, 0f));

                var row = new PooledChannelRow { Root = rowGo, Bg = img, Label = lbl, SelectBtn = selBtn };
                selBtn.onClick.AddListener(() => SelectChannel(row.BoundName));

                row.GagBtn = CreateButton(rowGo.transform, "GAG", new Color(0.32f, 0.24f, 0.14f), () =>
                {
                    if (string.IsNullOrEmpty(row.BoundAlias)) return;
                    OnCommandRequested?.Invoke(row.BoundGagged ? $"@channel/ungag {row.BoundAlias}" : $"@channel/gag {row.BoundAlias}");
                    OnRefreshComsysRequested?.Invoke();
                });
                SetRect(row.GagBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-58f, 3f), new Vector2(-4f, -3f));
                row.GagBtnLabel = row.GagBtn.GetComponentInChildren<Text>();

                rowGo.SetActive(false);
                _chanPool.Add(row);
            }

            // Bottom of left pane: Join channel (`addcom <alias>=<channel>`)
            _chanJoinAliasInput = CreateInputField(leftPane.transform, "alias", new Vector2(0f, 0f), new Vector2(0.34f, 0f), new Vector2(6f, 36f), new Vector2(-2f, 62f));
            _chanJoinNameInput = CreateInputField(leftPane.transform, "channel name...", new Vector2(0.34f, 0f), new Vector2(1f, 0f), new Vector2(2f, 36f), new Vector2(-6f, 62f));
            var joinBtn = CreateButton(leftPane.transform, "JOIN CHANNEL (addcom)", new Color(0.16f, 0.44f, 0.38f), SubmitJoinChannel);
            SetRect(joinBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(6f, 6f), new Vector2(-6f, 32f));

            // Right: Selected Channel Transcript & Speak Bar
            var rightPane = CreatePanel(body.transform, "ChannelTranscriptPane", new Vector2(0.38f, 0f), new Vector2(1f, 1f), new Vector2(6f, 10f), new Vector2(-12f, -10f), new Color(0.02f, 0.05f, 0.09f, 0.92f));
            _activeChanHeaderText = CreateText(rightPane.transform, "Select a channel on the left to read & speak.", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.50f, 0.92f, 1f));
            SetRect(_activeChanHeaderText.rectTransform, new Vector2(0f, 1f), new Vector2(0.62f, 1f), new Vector2(10f, -32f), new Vector2(0f, -4f));

            var recallChanBtn = CreateButton(rightPane.transform, "RECALL", new Color(0.14f, 0.42f, 0.46f), () =>
            {
                if (!string.IsNullOrEmpty(_selectedChannelName))
                    OnChannelBacklogRequested?.Invoke(_selectedChannelName);
            });
            SetRect(recallChanBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-174f, -30f), new Vector2(-96f, -6f));

            var whoChanBtn = CreateButton(rightPane.transform, "MEMBERS", new Color(0.16f, 0.36f, 0.50f), () =>
            {
                if (!string.IsNullOrEmpty(_selectedChannelName))
                    OnCommandRequested?.Invoke($"@channel/who {_selectedChannelName}");
            });
            SetRect(whoChanBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-92f, -30f), new Vector2(-8f, -6f));

            _channelTranscriptText = CreateText(rightPane.transform, "<color=#88AAC8>Live channel traffic appears here as messages arrive.</color>", 12, FontStyle.Normal, TextAnchor.LowerLeft, new Color(0.88f, 0.94f, 0.98f));
            _channelTranscriptText.supportRichText = true;
            _channelTranscriptText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _channelTranscriptText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_channelTranscriptText.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 44f), new Vector2(-10f, -36f));

            _chanSpeakInput = CreateInputField(rightPane.transform, "Transmit on selected channel...", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(8f, 8f), new Vector2(-88f, 38f));
            _chanSpeakInput.onSubmit.AddListener(_ => SubmitChannelMessage());
            var sendChanBtn = CreateButton(rightPane.transform, "SEND", new Color(0.14f, 0.46f, 0.54f), SubmitChannelMessage);
            SetRect(sendChanBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-82f, 8f), new Vector2(-8f, 38f));
        }

        public void SetOpen(bool open, SocialModalTab? tabOverride = null)
        {
            if (_modalRoot == null) return;
            _modalRoot.SetActive(open);
            if (open)
            {
                if (tabOverride.HasValue)
                    SelectTab(tabOverride.Value, autoRefresh: true);
                else
                    RefreshCurrentTabFromServer();
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void ResetSessionState()
        {
            _whoEntries.Clear();
            _mailFolders.Clear();
            _mailEntries.Clear();
            _mailReaderLines.Clear();
            _channels.Clear();
            _activeMailFolder = "inbox";
            _selectedChannelName = string.Empty;
            _mailQuotaLabel = string.Empty;
            SetUnreadCount(0);
            SetOpen(false);
        }

        public void SelectTab(SocialModalTab tab, bool autoRefresh = false)
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
                _tabImages[i].color = i == idx
                    ? new Color(0.20f, 0.58f, 0.74f, 1f)
                    : new Color(0.10f, 0.20f, 0.30f, 0.94f);
            }
            if (autoRefresh)
                RefreshCurrentTabFromServer();
            RefreshUi();
        }

        private void RefreshCurrentTabFromServer()
        {
            switch (ActiveTab)
            {
                case SocialModalTab.OnlineWho: OnRefreshWhoRequested?.Invoke(); break;
                case SocialModalTab.Mail: OnRefreshMailRequested?.Invoke(_activeMailFolder); break;
                case SocialModalTab.Channels: OnRefreshComsysRequested?.Invoke(); break;
            }
        }

        private void SwitchMailFolder(string folder)
        {
            if (string.IsNullOrWhiteSpace(folder)) return;
            _activeMailFolder = folder.Trim().ToLowerInvariant();
            OnCommandRequested?.Invoke($"@mail/folder {_activeMailFolder}");
            OnRefreshMailRequested?.Invoke(_activeMailFolder);
            RefreshMailUi();
        }

        public void ApplyWhoEntries(List<MushWhoEntry> entries)
        {
            _whoEntries.Clear();
            if (entries != null)
                _whoEntries.AddRange(entries);
            RefreshWhoUi();
        }

        public void ApplyMailNotice(int unread)
        {
            SetUnreadCount(Mathf.Max(0, unread));
        }

        public void ApplyMailPanel(JsonNode data)
        {
            if (data == null || !data.IsObject) return;
            string folder = data.Str("folder", string.Empty);
            if (!string.IsNullOrEmpty(folder))
                _activeMailFolder = folder.Trim().ToLowerInvariant();

            _mailFolders.Clear();
            int inboxUnread = UnreadMailCount;
            var fArr = data["folders"].AsArray();
            if (fArr != null)
            {
                for (int i = 0; i < fArr.Count; i++)
                {
                    var f = fArr[i];
                    if (!f.IsObject) continue;
                    var item = new MailFolderSummary
                    {
                        Name = f.Str("name", "inbox"),
                        Count = f.Int("count", 0),
                        Unread = f.Int("unread", 0)
                    };
                    _mailFolders.Add(item);
                    if (string.Equals(item.Name, "inbox", StringComparison.OrdinalIgnoreCase))
                        inboxUnread = item.Unread;
                }
            }
            SetUnreadCount(inboxUnread);

            _mailEntries.Clear();
            var eArr = data["entries"].AsArray();
            if (eArr != null)
            {
                for (int i = 0; i < eArr.Count; i++)
                {
                    var e = eArr[i];
                    if (!e.IsObject) continue;
                    var fromObj = e["from"];
                    string fromName = fromObj.IsObject ? fromObj.Str("name", "?") : e.Str("from", "?");
                    _mailEntries.Add(new MailInboxEntry
                    {
                        Index = e.Int("index", i + 1),
                        Id = e.Long("id", 0),
                        FromName = fromName,
                        Subject = e.Str("subject", "(no subject)"),
                        Status = e.Str("status", "read"),
                        Delivery = e.Str("delivery", string.Empty)
                    });
                }
            }

            var q = data["quota"];
            _mailQuotaLabel = q.IsObject ? $"Quota: {q.Int("used", 0)}/{q.Int("max", 0)}" : "Quota: Staff/Exempt";
            RefreshMailUi();
        }

        public void IngestMailReadOutput(string plain, string rich)
        {
            if (string.IsNullOrWhiteSpace(plain)) return;
            if (!IsOpen || ActiveTab != SocialModalTab.Mail) return;
            if (Time.unscaledTime > _mailCaptureUntilUnscaled) return;

            string line = !string.IsNullOrWhiteSpace(rich) ? rich : MushProtocol.EscapeUnityRichText(plain);
            _mailReaderLines.Add(line);
            while (_mailReaderLines.Count > 24)
                _mailReaderLines.RemoveAt(0);
            if (_mailReaderText != null)
                _mailReaderText.text = string.Join("\n", _mailReaderLines);
        }

        public void ApplyComsysPanel(JsonNode data)
        {
            if (data == null || !data.IsObject) return;

            // Delta frame carrying channel backlog slice (`comsys.backlog` -> `data.backlog`)
            var backlog = data["backlog"];
            if (backlog != null && backlog.IsObject)
            {
                ApplyChannelBacklog(backlog);
                return;
            }

            var arr = data["channels"].AsArray();
            if (arr == null) return;

            var oldLogs = new Dictionary<string, (List<string> lines, bool loaded)>(StringComparer.OrdinalIgnoreCase);
            for (int i = 0; i < _channels.Count; i++)
                oldLogs[_channels[i].Name] = (new List<string>(_channels[i].RecentLines), _channels[i].BacklogLoaded);

            _channels.Clear();
            for (int i = 0; i < arr.Count; i++)
            {
                var c = arr[i];
                if (!c.IsObject) continue;
                string name = c.Str("name", string.Empty).Trim();
                if (string.IsNullOrEmpty(name)) continue;
                var info = new ComsysChannelInfo
                {
                    Name = name,
                    Alias = c.Str("alias", name).Trim(),
                    Title = c.Str("title", string.Empty).Trim(),
                    Gagged = c.Bool("gagged", false),
                    IsIc = c.Bool("ic", false),
                    Logged = c.Bool("logged", false),
                    UnreadCount = Mathf.Max(0, c.Int("unread", 0)),
                    LastSeq = c.ULong("seq", 0),
                    ColorHex = c["color"].IsObject ? c["color"].Str("web_hex", string.Empty) : string.Empty
                };
                if (oldLogs.TryGetValue(name, out var preserved))
                {
                    info.RecentLines.AddRange(preserved.lines);
                    info.BacklogLoaded = preserved.loaded;
                }
                _channels.Add(info);
            }
            if (string.IsNullOrEmpty(_selectedChannelName) && _channels.Count > 0)
                _selectedChannelName = _channels[0].Name;

            var active = FindChannel(_selectedChannelName);
            if (active != null && active.Logged && !active.BacklogLoaded)
                OnChannelBacklogRequested?.Invoke(active.Name);

            RefreshChannelsUi();
        }

        private void ApplyChannelBacklog(JsonNode backlog)
        {
            string chanName = backlog.Str("channel", string.Empty).Trim();
            if (string.IsNullOrEmpty(chanName)) return;

            var target = FindChannel(chanName);
            if (target == null)
            {
                target = new ComsysChannelInfo { Name = chanName, Alias = chanName, Logged = true };
                _channels.Add(target);
                if (string.IsNullOrEmpty(_selectedChannelName))
                    _selectedChannelName = target.Name;
            }

            var msgs = backlog["messages"].AsArray();
            var formatted = new List<string>(msgs != null ? msgs.Count : 0);
            if (msgs != null)
            {
                // `comsys.backlog` delivers messages newest-first; reverse into chronological display order.
                for (int i = msgs.Count - 1; i >= 0; i--)
                {
                    var m = msgs[i];
                    if (!m.IsObject) continue;
                    string actor = m["actor"].IsObject ? m["actor"].Str("name", "Pilot") : "Pilot";
                    string title = m.Str("title", string.Empty).Trim();
                    string kind = m.Str("kind", "say");
                    string rich = MushProtocol.RenderRichTextToUnityMarkup(m["text"]);
                    if (string.IsNullOrEmpty(rich))
                        rich = MushProtocol.EscapeUnityRichText(MushProtocol.FlattenRichText(m["text"]));

                    string prefix = !string.IsNullOrEmpty(title)
                        ? $"<color=#88CCEE>[{MushProtocol.EscapeUnityRichText(title)}]</color> "
                        : string.Empty;
                    string safeActor = MushProtocol.EscapeUnityRichText(actor);
                    string line = string.Equals(kind, "pose", StringComparison.OrdinalIgnoreCase)
                        ? $"{prefix}<b>{safeActor}</b> {rich}"
                        : (string.Equals(kind, "emit", StringComparison.OrdinalIgnoreCase)
                            ? $"{prefix}{rich}"
                            : $"{prefix}<b>{safeActor}:</b> {rich}");
                    formatted.Add(line);
                }
            }

            target.RecentLines.Clear();
            target.RecentLines.AddRange(formatted);
            while (target.RecentLines.Count > 40)
                target.RecentLines.RemoveAt(0);
            target.BacklogLoaded = true;
            if (string.Equals(target.Name, _selectedChannelName, StringComparison.OrdinalIgnoreCase))
                target.UnreadCount = 0;

            RefreshChannelsUi();
        }

        public void IngestChannelMessage(MushChannelEvent ch)
        {
            if (ch == null || string.IsNullOrWhiteSpace(ch.Channel)) return;
            ComsysChannelInfo target = FindChannel(ch.Channel);
            if (target == null)
            {
                target = new ComsysChannelInfo { Name = ch.Channel.Trim(), Alias = ch.Channel.Trim() };
                _channels.Add(target);
                if (string.IsNullOrEmpty(_selectedChannelName))
                    _selectedChannelName = target.Name;
            }
            string actor = ch.Actor != null && !string.IsNullOrEmpty(ch.Actor.Name) ? ch.Actor.Name : "Pilot";
            string body = !string.IsNullOrEmpty(ch.RichMarkup) ? ch.RichMarkup : MushProtocol.EscapeUnityRichText(ch.Text);
            target.RecentLines.Add($"<b>{MushProtocol.EscapeUnityRichText(actor)}:</b> {body}");
            while (target.RecentLines.Count > 40)
                target.RecentLines.RemoveAt(0);
            bool viewingNow = IsOpen && ActiveTab == SocialModalTab.Channels &&
                              string.Equals(target.Name, _selectedChannelName, StringComparison.OrdinalIgnoreCase);
            if (!viewingNow)
                target.UnreadCount++;
            if (IsOpen && ActiveTab == SocialModalTab.Channels)
                RefreshChannelsUi();
        }

        public void SelectChannel(string channelName)
        {
            if (string.IsNullOrWhiteSpace(channelName)) return;
            _selectedChannelName = channelName.Trim();
            var active = FindChannel(_selectedChannelName);
            if (active != null)
            {
                active.UnreadCount = 0;
                if (active.Logged && !active.BacklogLoaded)
                    OnChannelBacklogRequested?.Invoke(active.Name);
            }
            RefreshChannelsUi();
        }

        public int GetChannelLineCount(string channelName)
        {
            var ch = FindChannel(channelName);
            return ch != null ? ch.RecentLines.Count : 0;
        }

        public void SubmitQuickMail()
        {
            string to = _mailToInput != null ? _mailToInput.text : string.Empty;
            string subj = _mailSubjInput != null ? _mailSubjInput.text : string.Empty;
            string body = _mailBodyInput != null ? _mailBodyInput.text : string.Empty;
            if (!TryBuildQuickMailCommand(to, subj, body, out string cmd, out string err))
            {
                if (_mailReaderText != null)
                    _mailReaderText.text = $"<color=#FF7766>{MushProtocol.EscapeUnityRichText(err)}</color>";
                return;
            }
            if (_mailSubjInput != null) _mailSubjInput.text = string.Empty;
            if (_mailBodyInput != null) _mailBodyInput.text = string.Empty;
            OnCommandRequested?.Invoke(cmd);
            OnRefreshMailRequested?.Invoke(_activeMailFolder);
            if (_mailReaderText != null)
                _mailReaderText.text = $"<color=#66FFAA>Dispatched IC mail to <b>{MushProtocol.EscapeUnityRichText(to.Trim())}</b>.</color>";
        }

        private void SubmitChannelMessage()
        {
            if (_chanSpeakInput == null || string.IsNullOrWhiteSpace(_chanSpeakInput.text)) return;
            var ch = FindChannel(_selectedChannelName);
            if (ch == null || string.IsNullOrWhiteSpace(ch.Alias)) return;
            string msg = MushProtocol.StripControlChars(_chanSpeakInput.text).Trim();
            _chanSpeakInput.text = string.Empty;
            if (string.IsNullOrEmpty(msg)) return;
            OnCommandRequested?.Invoke($"{ch.Alias} {msg}");
        }

        private void SubmitJoinChannel()
        {
            string alias = MushProtocol.StripControlChars(_chanJoinAliasInput != null ? _chanJoinAliasInput.text : string.Empty).Trim();
            string chan = MushProtocol.StripControlChars(_chanJoinNameInput != null ? _chanJoinNameInput.text : string.Empty).Trim();
            if (!MushProtocol.TryValidateSafeCommandToken(alias, out string safeAlias) ||
                !MushProtocol.TryValidateSafeKeywordOrName(chan, out string safeChan))
            {
                return;
            }
            if (_chanJoinAliasInput != null) _chanJoinAliasInput.text = string.Empty;
            if (_chanJoinNameInput != null) _chanJoinNameInput.text = string.Empty;
            OnCommandRequested?.Invoke($"addcom {safeAlias}={safeChan}");
            OnRefreshComsysRequested?.Invoke();
        }

        private ComsysChannelInfo FindChannel(string name)
        {
            for (int i = 0; i < _channels.Count; i++)
            {
                if (string.Equals(_channels[i].Name, name, StringComparison.OrdinalIgnoreCase))
                    return _channels[i];
            }
            return null;
        }

        private void SetUnreadCount(int count)
        {
            if (UnreadMailCount == count) return;
            UnreadMailCount = count;
            OnUnreadMailCountChanged?.Invoke(count);
        }

        private void RefreshUi()
        {
            RefreshWhoUi();
            RefreshMailUi();
            RefreshChannelsUi();
        }

        private void RefreshWhoUi()
        {
            if (_whoSummaryText != null)
                _whoSummaryText.text = $"{_whoEntries.Count} pilot{(_whoEntries.Count == 1 ? "" : "s")} connected on grid";

            for (int i = 0; i < _whoPool.Count; i++)
            {
                var row = _whoPool[i];
                if (i < _whoEntries.Count)
                {
                    var e = _whoEntries[i];
                    row.BoundName = e.Name;
                    string alias = !string.IsNullOrEmpty(e.Alias) ? $" ({MushProtocol.EscapeUnityRichText(e.Alias)})" : string.Empty;
                    string doing = !string.IsNullOrEmpty(e.Doing) ? $"  <color=#88B8D8>— {MushProtocol.EscapeUnityRichText(e.Doing)}</color>" : string.Empty;
                    row.Label.text = $"<b>{MushProtocol.EscapeUnityRichText(e.Name)}</b>{alias}  <color=#66EEAA>[On {FormatDurationShort(e.ConnSeconds)} · Idle {FormatDurationShort(e.IdleSeconds)}]</color>{doing}";
                    row.Root.SetActive(true);
                }
                else
                {
                    row.BoundName = null;
                    row.Root.SetActive(false);
                }
            }
        }

        private void RefreshMailUi()
        {
            if (_mailFolderStatusText != null)
            {
                string unreadBadge = UnreadMailCount > 0 ? $"  |  <color=#FFCC55>{UnreadMailCount} UNREAD</color>" : string.Empty;
                _mailFolderStatusText.supportRichText = true;
                _mailFolderStatusText.text = $"FOLDER: <b>{_activeMailFolder.ToUpperInvariant()}</b> ({_mailEntries.Count}){unreadBadge}  |  {_mailQuotaLabel}";
            }

            for (int i = 0; i < _mailPool.Count; i++)
            {
                var row = _mailPool[i];
                if (i < _mailEntries.Count)
                {
                    var e = _mailEntries[i];
                    row.BoundIndex = e.Index;
                    bool unread = !string.Equals(e.Status, "read", StringComparison.OrdinalIgnoreCase);
                    bool urgent = string.Equals(e.Status, "urgent", StringComparison.OrdinalIgnoreCase);
                    row.Bg.color = unread
                        ? new Color(0.11f, 0.22f, 0.34f, 0.96f)
                        : new Color(0.05f, 0.10f, 0.16f, 0.90f);
                    string prefix = urgent ? "<color=#FF6655>[!]</color> " : (unread ? "<color=#66FFAA>[NEW]</color> " : "");
                    row.Label.text = $"{prefix}<b>#{e.Index}</b> {MushProtocol.EscapeUnityRichText(e.FromName)}: {MushProtocol.EscapeUnityRichText(e.Subject)}";
                    row.Root.SetActive(true);
                }
                else
                {
                    row.BoundIndex = 0;
                    row.Root.SetActive(false);
                }
            }
        }

        private void RefreshChannelsUi()
        {
            for (int i = 0; i < _chanPool.Count; i++)
            {
                var row = _chanPool[i];
                if (i < _channels.Count)
                {
                    var c = _channels[i];
                    row.BoundName = c.Name;
                    row.BoundAlias = c.Alias;
                    row.BoundGagged = c.Gagged;
                    bool selected = string.Equals(c.Name, _selectedChannelName, StringComparison.OrdinalIgnoreCase);
                    row.Bg.color = selected
                        ? new Color(0.16f, 0.44f, 0.60f, 0.98f)
                        : new Color(0.06f, 0.12f, 0.19f, 0.90f);
                    string scopeTag = c.IsIc ? "<color=#66FFAA>[IC]</color>" : "<color=#88CCFF>[OOC]</color>";
                    string unreadTag = c.UnreadCount > 0 ? $" <color=#FFCC55>({c.UnreadCount})</color>" : string.Empty;
                    string gagTag = c.Gagged ? " <color=#FF7766>(GAGGED)</color>" : string.Empty;
                    row.Label.text = $"{scopeTag} {MushProtocol.EscapeUnityRichText(c.Name)} ({MushProtocol.EscapeUnityRichText(c.Alias)}){unreadTag}{gagTag}";
                    if (row.GagBtnLabel != null)
                        row.GagBtnLabel.text = c.Gagged ? "UNGAG" : "GAG";
                    row.Root.SetActive(true);
                }
                else
                {
                    row.BoundName = null;
                    row.BoundAlias = null;
                    row.Root.SetActive(false);
                }
            }

            var active = FindChannel(_selectedChannelName);
            if (_activeChanHeaderText != null)
            {
                string logBadge = active != null && active.Logged ? " · LOGGED" : string.Empty;
                _activeChanHeaderText.text = active != null
                    ? $"{(active.IsIc ? "IC" : "OOC")} CHANNEL: {active.Name.ToUpperInvariant()}  (verb: {active.Alias}{logBadge})"
                    : "No channels joined — click LIST ALL (clist) or join below.";
            }
            if (_channelTranscriptText != null)
            {
                if (active != null && active.RecentLines.Count > 0)
                    _channelTranscriptText.text = string.Join("\n", active.RecentLines);
                else
                    _channelTranscriptText.text = active != null
                        ? $"<color=#88AAC8>Connected to {MushProtocol.EscapeUnityRichText(active.Name)}. Type below to transmit via '{MushProtocol.EscapeUnityRichText(active.Alias)}'.</color>"
                        : "<color=#88AAC8>Join a channel via addcom to start talking.</color>";
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

        private InputField CreateInputField(Transform parent, string placeholder, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax)
        {
            var go = CreatePanel(parent, "SocialInput", aMin, aMax, oMin, oMax, new Color(0.08f, 0.14f, 0.22f, 0.96f));
            var input = go.AddComponent<InputField>();
            var ph = CreateText(go.transform, placeholder, 11, FontStyle.Italic, TextAnchor.MiddleLeft, new Color(0.50f, 0.64f, 0.76f));
            SetRect(ph.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 2f), new Vector2(-8f, -2f));
            var val = CreateText(go.transform, string.Empty, 12, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
            SetRect(val.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 2f), new Vector2(-8f, -2f));
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
