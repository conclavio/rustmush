using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum MindTerminalTab
    {
        Market = 0,
        HoldingsAndSealing = 1,
        Workshop = 2,
        LiveTrace = 3
    }

    public sealed class MindMarketRowView
    {
        public string Code = string.Empty;
        public string Name = string.Empty;
        public string ClassWord = string.Empty;
        public string Form = string.Empty;
        public bool Transferable;
        public string PriceMinor = "0";
        public string PriceText = string.Empty;
        public string TermsLine = string.Empty;
        public string SplitLine = string.Empty;
    }

    public sealed class MindOwnQuoteView
    {
        public string Code = string.Empty;
        public string PriceText = string.Empty;
        public int ExpiresInSeconds;
        public bool Expired;
    }

    public sealed class MindMarketPageView
    {
        public bool Open;
        public string Notice;
        public string NextAfterCode;
        public readonly List<MindMarketRowView> Rows = new List<MindMarketRowView>();
        public readonly List<MindOwnQuoteView> OwnQuotes = new List<MindOwnQuoteView>();
        public readonly List<string> WaitingCodes = new List<string>();
    }

    public sealed class MindHoldingRowView
    {
        public string Kind = "paid"; // "owner" | "paid"
        public int Number;           // legacy display #: filed # (owner) or licence # (paid)
        public long FiledId;         // > 0 on "owner" rows
        public long? LicenceId;      // licence # on "paid" rows, or owner_licence on "owner" rows (null if absent)
        public string Name = string.Empty;
        public string Form = string.Empty;
        public string PhaseState = "active";
        public int? DaysLeft;
        public bool House;
        public bool Transferable;
        public bool Dependent;
        public bool Revoked;
        public bool Selling;
        public string Renewal;       // "offered" | "paid" | null
        public readonly List<string> Actions = new List<string>();
    }

    public sealed class MindHoldingsPageView
    {
        public bool Open;
        public string Section = "paid";
        public string Notice;
        public long? NextAfterNumber;
        public readonly List<MindHoldingRowView> Rows = new List<MindHoldingRowView>();
    }

    public sealed class MindDiagnosticView
    {
        public string Field = string.Empty;
        public string Message = string.Empty;
        public int? Line;
        public int? Column;
    }

    public sealed class MindPictureReportView
    {
        public string Picture = string.Empty;
        public bool Armed;
        public long Operations;
        public long Fees;
        public string Fault;
        public readonly List<string> Intents = new List<string>();
        public readonly List<string> StationRefusals = new List<string>();
        public readonly List<string> ClassViolations = new List<string>();
    }

    public sealed class MindTraceView
    {
        public bool Available;
        public int UnitId;
        public readonly List<string> Lines = new List<string>();
    }

    /// <summary>
    /// Track 7: Unity 3D Mind Market, Holdings, Ship-Mind Berth Sealing (`+ship/mind`, `+ship/posture`),
    /// Rhai Doctrine Workshop (`InvokeRequest::Mind`), and Live `arena.mind` Trace Inspector (`[F10]`).
    /// </summary>
    public sealed class TacSimMindTerminal : MonoBehaviour
    {
        private const int InitialPooledRows = 20;
        private const int MaxSourceBytes = 32 * 1024;
        private const int MaxNotesBytes = 2048;
        private const float ConfirmWindowSeconds = 8f;
        private const int SortingOrderHiddenBehindLogin = 15;
        private const int SortingOrderActiveOnline = 22;

        public static TacSimMindTerminal ActiveInstance { get; private set; }

        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.AfterSceneLoad)]
        private static void EnsureRuntimeInstance()
        {
            if (FindAnyObjectByType<TacSimMindTerminal>() != null)
                return;
            var go = new GameObject("TacSimMindTerminal");
            DontDestroyOnLoad(go);
            var term = go.AddComponent<TacSimMindTerminal>();
            term.BuildTerminalUi();
            if (MushWsClient.LatestInstance != null)
                term.BindClient(MushWsClient.LatestInstance);
        }

        public bool IsTerminalOpen => _windowRoot != null && _windowRoot.activeSelf;
        public MindTerminalTab ActiveTab { get; private set; } = MindTerminalTab.Market;

        public bool IsAnyInputFocused =>
            (_sealLicenseInput != null && _sealLicenseInput.isFocused) ||
            (_newMindNameInput != null && _newMindNameInput.isFocused) ||
            (_rhaiSourceInput != null && _rhaiSourceInput.isFocused) ||
            (_saveNotesInput != null && _saveNotesInput.isFocused) ||
            (_traceUnitInput != null && _traceUnitInput.isFocused);

        public bool CanOpenViaHotkey =>
            _ws != null &&
            _ws.State == MushWsConnectionState.Online &&
            !IsLoginOverlayBlocking();

        public MindMarketPageView MarketPage { get; private set; }
        public MindHoldingsPageView HoldingsPage { get; private set; }
        public MindTraceView TracePage { get; private set; }
        public string HoldingsSection { get; private set; } = "paid";

        public IReadOnlyList<string> LibraryNames => _libraryNames;
        public string LoadedMindName { get; private set; } = string.Empty;
        public JsonNode LoadedDraftBase { get; private set; }
        public JsonNode LoadedDraft { get; private set; }
        public bool IsDraftDirty { get; private set; }
        public string StatusBanner { get; private set; } = "Ready.";
        public IReadOnlyList<MindDiagnosticView> LastDiagnostics => _lastDiagnostics;
        public IReadOnlyList<MindPictureReportView> LastReports => _lastReports;
        public string WorkshopOutputText => _workshopOutputText != null ? _workshopOutputText.text : string.Empty;
        public bool IsFollowingTrace => _followingTraceUnitId > 0;

        public int VisibleMarketRowCount => CountActiveRows(_marketRows);
        public int VisibleHoldingRowCount => CountActiveRows(_holdingRows);
        public int VisibleLibraryButtonCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _libraryButtons.Count; i++)
                {
                    if (_libraryButtons[i] != null && _libraryButtons[i].gameObject.activeSelf)
                        n++;
                }
                return n;
            }
        }

        public MushWsClient BoundClient => _ws;
        public TacSimHudController BoundHud => _explicitHud;
        public Canvas TerminalCanvas => _canvas;
        public RectTransform LauncherPillRect => _launcherPill != null ? _launcherPill.GetComponent<RectTransform>() : null;

        private MushWsClient _ws;
        private TacSimHudController _explicitHud;
        private Font _font;
        private Canvas _canvas;
        private GameObject _launcherPill;
        private GameObject _windowRoot;

        private readonly List<Image> _tabBtnImages = new List<Image>();
        private GameObject _marketPane;
        private GameObject _holdingsPane;
        private GameObject _workshopPane;
        private GameObject _tracePane;

        private Transform _marketRowsContainer;
        private Transform _holdingRowsContainer;
        private Transform _libraryButtonsContainer;

        private Text _statusBannerText;
        private Text _marketSummaryText;
        private Text _holdingsSummaryText;
        private Text _workshopMetaText;
        private Text _workshopOutputText;
        private Text _traceOutputText;
        private Button _stopFollowBtn;

        private InputField _sealLicenseInput;
        private InputField _newMindNameInput;
        private InputField _rhaiSourceInput;
        private InputField _saveNotesInput;
        private InputField _traceUnitInput;

        private bool _suppressSourceDirtyCallback;
        private int _draftSessionGeneration;
        private int _draftBaseGeneration;
        private int _localDraftEditVersion;
        private ulong _latestReadRequestId;
        private int _followingTraceUnitId;

        private readonly List<string> _libraryNames = new List<string>();
        private readonly List<Button> _libraryButtons = new List<Button>();
        private readonly List<MindDiagnosticView> _lastDiagnostics = new List<MindDiagnosticView>();
        private readonly List<MindPictureReportView> _lastReports = new List<MindPictureReportView>();

        private enum PendingMindOp
        {
            List,
            Read,
            Create,
            Replace,
            Check,
            Save,
            Market,
            Trace,
            Other
        }

        private sealed class PendingMindRequest
        {
            public PendingMindOp Op;
            public string TargetName;
            public int DraftSessionGenerationAtSend;
            public int DraftBaseGenerationAtSend;
            public int DraftEditVersionAtSend;
            public bool AllowDiscardDirty;
            public bool ChainCheckAfterReplace;
            public bool ChainSaveAfterReplace;
            public string PendingSaveNotes;
        }

        private readonly Dictionary<ulong, PendingMindRequest> _pendingRequests = new Dictionary<ulong, PendingMindRequest>();

        // Two-click confirmation state (`U-M15` + dirty-draft guard)
        private string _armedConfirmKey;
        private Text _armedConfirmLabel;
        private string _armedOriginalText;
        private float _armedAtTime;

        private sealed class PooledActionRow
        {
            public GameObject Root;
            public Image Background;
            public Text Label;
            public Button BtnA;
            public Text BtnALabel;
            public Action ActA;
            public Button BtnB;
            public Text BtnBLabel;
            public Action ActB;
            public Button BtnC;
            public Text BtnCLabel;
            public Action ActC;
        }

        private readonly List<PooledActionRow> _marketRows = new List<PooledActionRow>(InitialPooledRows);
        private readonly List<PooledActionRow> _holdingRows = new List<PooledActionRow>(InitialPooledRows);

        private void Awake()
        {
            ActiveInstance = this;
            MushWsClient.OnInstanceCreated += HandleClientInstanceCreated;
        }

        private void OnDestroy()
        {
            if (ActiveInstance == this)
                ActiveInstance = null;
            MushWsClient.OnInstanceCreated -= HandleClientInstanceCreated;
            UnbindClient();
        }

        private void HandleClientInstanceCreated(MushWsClient client)
        {
            BindClient(client);
        }

        public void BindClient(MushWsClient client)
        {
            if (_ws == client) return;
            UnbindClient();
            _ws = client;
            if (_ws != null)
            {
                _ws.OnPanelUpdated += HandlePanelUpdated;
                _ws.OnInvokeResultReceived += HandleInvokeResult;
                _ws.OnSystemLineReceived += HandleSystemLine;
                _ws.OnFatalSessionEnded += HandleFatalSessionEnded;
            }
        }

        private void UnbindClient()
        {
            if (_ws == null) return;
            _ws.OnPanelUpdated -= HandlePanelUpdated;
            _ws.OnInvokeResultReceived -= HandleInvokeResult;
            _ws.OnSystemLineReceived -= HandleSystemLine;
            _ws.OnFatalSessionEnded -= HandleFatalSessionEnded;
            _ws = null;
        }

        private void HandleFatalSessionEnded(string _)
        {
            ResetSessionState();
        }

        public void ResetSessionState()
        {
            DisarmConfirm();
            _followingTraceUnitId = 0;
            _pendingRequests.Clear();
            _latestReadRequestId = 0;
            _draftSessionGeneration = 0;
            _draftBaseGeneration = 0;
            _localDraftEditVersion = 0;

            MarketPage = null;
            HoldingsPage = null;
            TracePage = null;
            HoldingsSection = "paid";
            _libraryNames.Clear();
            LoadedMindName = string.Empty;
            LoadedDraftBase = null;
            LoadedDraft = null;
            IsDraftDirty = false;
            _lastDiagnostics.Clear();
            _lastReports.Clear();

            if (_windowRoot != null) _windowRoot.SetActive(false);
            if (_launcherPill != null) _launcherPill.SetActive(false);
            if (_canvas != null) _canvas.sortingOrder = SortingOrderHiddenBehindLogin;

            if (_rhaiSourceInput != null)
            {
                _suppressSourceDirtyCallback = true;
                _rhaiSourceInput.text = string.Empty;
                _suppressSourceDirtyCallback = false;
                _rhaiSourceInput.interactable = false;
            }
            if (_sealLicenseInput != null) _sealLicenseInput.text = "1";
            if (_newMindNameInput != null) _newMindNameInput.text = "escort_v1";
            if (_saveNotesInput != null) _saveNotesInput.text = "Tuned doctrine";
            if (_traceUnitInput != null) _traceUnitInput.text = "1";
            if (_workshopOutputText != null) _workshopOutputText.text = "Diagnostics and isolated picture reports will appear here after Check or Save.";
            if (_stopFollowBtn != null) _stopFollowBtn.interactable = false;

            SetStatus("Ready.");
            RefreshMarketPane();
            RefreshHoldingsPane();
            RefreshWorkshopPane();
            RefreshTracePane();
        }

        public void BindHud(TacSimHudController hud)
        {
            _explicitHud = hud;
        }

        private TacSimHudController ResolveActiveHud()
        {
            if (_explicitHud != null) return _explicitHud;
            var active = TacSimHudController.ActiveInstance;
            if (active == null) return null;
            if (Application.isPlaying) return active;
            if (_ws != null && ReferenceEquals(active.BoundWsClient, _ws)) return active;
            return null;
        }

        private bool IsLoginOverlayBlocking()
        {
            var hud = ResolveActiveHud();
            return hud != null && hud.IsLoginOverlayVisible;
        }

        private static int CountActiveRows(List<PooledActionRow> pool)
        {
            int n = 0;
            for (int i = 0; i < pool.Count; i++)
            {
                if (pool[i].Root != null && pool[i].Root.activeSelf)
                    n++;
            }
            return n;
        }

        // ---- Closed Wire Decoders (`mind.market`, `mind.holdings`, `arena.mind`) ----

        private static bool CheckSchema(JsonNode root, string expectedId, int expectedVersion = 1)
        {
            if (root == null || !root.IsObject) return false;
            var sc = root["schema"];
            if (!sc.IsObject) return false;
            return sc.Str("id", string.Empty) == expectedId && sc.Int("version", 0) == expectedVersion;
        }

        public static MindMarketPageView DecodeMindMarketView(JsonNode root)
        {
            if (!CheckSchema(root, "mind.market", 1))
                return null;

            var page = new MindMarketPageView
            {
                Open = root.Bool("open", true),
                Notice = root.ContainsKey("notice") && !root["notice"].IsNull ? root.Str("notice", null) : null,
                NextAfterCode = root.ContainsKey("after") && !root["after"].IsNull ? root.Str("after", null) : null,
            };

            var rowsArr = root["rows"].AsArray();
            if (rowsArr != null)
            {
                for (int i = 0; i < Math.Min(rowsArr.Count, 128); i++)
                {
                    var r = rowsArr[i];
                    if (!r.IsObject) continue;
                    string code = r.Str("code", string.Empty).Trim();
                    if (code.Length != 8) continue;
                    page.Rows.Add(new MindMarketRowView
                    {
                        Code = code,
                        Name = r.Str("name", code),
                        ClassWord = r.Str("class", "I"),
                        Form = r.Str("form", "sealed"),
                        Transferable = r.Bool("transferable", true),
                        PriceMinor = r.Str("price_minor", "0"),
                        PriceText = r.Str("price_text", string.Empty),
                        TermsLine = r.Str("terms_line", string.Empty),
                        SplitLine = r.Str("split_line", string.Empty),
                    });
                }
            }

            var ownObj = root["own"];
            if (ownObj.IsObject)
            {
                var qArr = ownObj["quotes"].AsArray();
                if (qArr != null)
                {
                    for (int i = 0; i < qArr.Count; i++)
                    {
                        var q = qArr[i];
                        if (!q.IsObject) continue;
                        page.OwnQuotes.Add(new MindOwnQuoteView
                        {
                            Code = q.Str("code", string.Empty),
                            PriceText = q.Str("price_text", string.Empty),
                            ExpiresInSeconds = q.Int("expires_in_seconds", 0),
                            Expired = q.Bool("expired", false),
                        });
                    }
                }
                var wArr = ownObj["waiting"].AsArray();
                if (wArr != null)
                {
                    for (int i = 0; i < wArr.Count; i++)
                    {
                        var item = wArr[i];
                        // Live `query.rs:302` emits `[{"code": "<8-hex>"}]`; keep string fallback defensively (`U-H25`).
                        string wCode = item.IsObject
                            ? item.Str("code", string.Empty)
                            : item.AsString();
                        if (!string.IsNullOrWhiteSpace(wCode))
                            page.WaitingCodes.Add(wCode.Trim());
                    }
                }
            }

            return page;
        }

        public static MindHoldingsPageView DecodeMindHoldingsView(JsonNode root)
        {
            if (!CheckSchema(root, "mind.holdings", 1))
                return null;

            var page = new MindHoldingsPageView
            {
                Open = root.Bool("open", true),
                Section = root.Str("section", "paid"),
                Notice = root.ContainsKey("notice") && !root["notice"].IsNull ? root.Str("notice", null) : null,
                NextAfterNumber = root.ContainsKey("after") && !root["after"].IsNull ? (long?)root.Long("after", 0) : null,
            };

            var rowsArr = root["rows"].AsArray();
            if (rowsArr != null)
            {
                for (int i = 0; i < Math.Min(rowsArr.Count, 128); i++)
                {
                    var r = rowsArr[i];
                    if (!r.IsObject) continue;
                    string kind = r.Str("kind", "paid");
                    bool isOwner = string.Equals(kind, "owner", StringComparison.OrdinalIgnoreCase);

                    long filedId = 0;
                    long? licenceId = null;
                    long displayNum;
                    if (isOwner)
                    {
                        filedId = r.Long("filed", 0);
                        if (filedId <= 0) continue;
                        displayNum = filedId;
                        // Server `query.rs` emits `"owner_licence": <u64|null>` alongside `"filed"` (`U-C06`).
                        if (r.ContainsKey("owner_licence") && !r["owner_licence"].IsNull)
                        {
                            long ol = r.Long("owner_licence", 0);
                            if (ol > 0) licenceId = ol;
                        }
                    }
                    else
                    {
                        long lic = r.Long("licence", 0);
                        if (lic <= 0) continue;
                        licenceId = lic;
                        displayNum = lic;
                    }

                    var phaseObj = r["phase"];
                    string phaseState = phaseObj.IsObject ? phaseObj.Str("state", "active") : "active";
                    int? daysLeft = (phaseObj.IsObject && phaseObj.ContainsKey("days_left") && !phaseObj["days_left"].IsNull)
                        ? (int?)phaseObj.Int("days_left", 0)
                        : null;

                    var row = new MindHoldingRowView
                    {
                        Kind = isOwner ? "owner" : "paid",
                        Number = (int)Math.Min(int.MaxValue, displayNum),
                        FiledId = filedId,
                        LicenceId = licenceId,
                        Name = r.Str("name", $"#{displayNum}"),
                        Form = r.Str("form", isOwner ? "original" : "sealed"),
                        PhaseState = phaseState,
                        DaysLeft = daysLeft,
                        House = r.Bool("house", false),
                        Transferable = r.Bool("transferable", false),
                        Dependent = r.Bool("dependent", false),
                        Revoked = r.Bool("revoked", false),
                        Selling = r.Bool("selling", false),
                        Renewal = r.ContainsKey("renewal") && !r["renewal"].IsNull ? r.Str("renewal", null) : null,
                    };

                    var actArr = r["actions"].AsArray();
                    if (actArr != null)
                    {
                        for (int a = 0; a < actArr.Count; a++)
                        {
                            string act = actArr[a].AsString();
                            if (!string.IsNullOrEmpty(act))
                                row.Actions.Add(act);
                        }
                    }
                    page.Rows.Add(row);
                }
            }

            return page;
        }

        public static MindTraceView DecodeMindTraceView(JsonNode root)
        {
            if (!CheckSchema(root, "arena.mind", 1))
                return null;

            var view = new MindTraceView
            {
                Available = root.Bool("available", false),
                UnitId = root.Int("unit", 0),
            };
            if (!view.Available)
                return view;

            int omitted = root.Int("omitted_thinks", 0);
            if (omitted > 0)
                view.Lines.Add($"{omitted} earlier thinks omitted by server payload bound.");

            var traces = root["traces"].AsArray();
            if (traces != null)
            {
                for (int i = 0; i < Math.Min(traces.Count, 64) && view.Lines.Count < 512; i++)
                {
                    var tr = traces[i];
                    if (!tr.IsObject) continue;
                    int seq = tr.Int("think_seq", i + 1);
                    long tMs = tr.Long("t_ms", 0);
                    string src = tr.Bool("rom", false) ? "ROM" : "mind";
                    string outWord = FormatTraceOutcome(tr["outcome"]);
                    string settledTag = tr.ContainsKey("settled")
                        ? (tr.Bool("settled", false) ? " · settled" : " · unsettled")
                        : string.Empty;
                    view.Lines.Add($"Think {seq} · {tMs} ms · {src} · {outWord}{settledTag}");

                    if (tr.ContainsKey("landed_ms") && !tr["landed_ms"].IsNull)
                        view.Lines.Add($"  Landing: {tr.Long("landed_ms", 0)} ms");

                    if (tr.ContainsKey("fault") && !tr["fault"].IsNull)
                    {
                        string f = tr.Str("fault", string.Empty);
                        if (!string.IsNullOrEmpty(f))
                            view.Lines.Add($"  Fault: {f}");
                    }

                    if (tr.ContainsKey("rom_drop") && !tr["rom_drop"].IsNull)
                    {
                        string rd = tr.Str("rom_drop", string.Empty);
                        if (!string.IsNullOrEmpty(rd))
                            view.Lines.Add($"  ROM drop: {rd}");
                    }

                    var diag = tr["diagnostics"];
                    if (diag.IsObject)
                    {
                        string ops = diag.ContainsKey("ops") && !diag["ops"].IsNull ? diag.Long("ops", 0).ToString() : "unmeasured";
                        string fees = diag.ContainsKey("fees") && !diag["fees"].IsNull ? diag.Long("fees", 0).ToString() : "unmeasured";
                        view.Lines.Add($"  Operations {ops}; host fees {fees}");

                        var kernels = diag["kernels"].AsArray();
                        if (kernels != null)
                        {
                            for (int k = 0; k < Math.Min(kernels.Count, 32) && view.Lines.Count < 512; k++)
                            {
                                var kn = kernels[k];
                                if (!kn.IsObject) continue;
                                view.Lines.Add($"  Kernel {kn.Str("kernel", "?")}: {kn.Long("fee", 0)} fees");
                            }
                        }

                        var rules = diag["rules"].AsArray();
                        if (rules != null)
                        {
                            for (int r = 0; r < Math.Min(rules.Count, 8) && view.Lines.Count < 512; r++)
                            {
                                var rl = rules[r];
                                if (!rl.IsObject) continue;
                                string mVal = rl["matches"].IsNull ? "n/a" : (rl.Bool("matches", false) ? "true" : "false");
                                view.Lines.Add($"  Rule {rl.Int("slot", r + 1)}: {rl.Str("description", "?")} = {mVal}");
                                var terms = rl["terms"].AsArray();
                                if (terms != null)
                                {
                                    for (int t = 0; t < Math.Min(terms.Count, 6) && view.Lines.Count < 512; t++)
                                    {
                                        var tm = terms[t];
                                        if (!tm.IsObject) continue;
                                        string tv = tm["value"].IsNull ? "n/a" : (tm.Bool("value", false) ? "true" : "false");
                                        view.Lines.Add($"    {tm.Str("description", "?")} = {tv}");
                                    }
                                }
                            }
                        }

                        var om = diag["omitted"];
                        if (om.IsObject)
                        {
                            var omParts = new List<string>();
                            AppendOmitted(omParts, "author_lines", om.Int("author_lines", 0));
                            AppendOmitted(omParts, "author_chars", om.Int("author_chars", 0));
                            AppendOmitted(omParts, "kernel_calls", om.Int("kernel_calls", 0));
                            AppendOmitted(omParts, "rules", om.Int("rules", 0));
                            AppendOmitted(omParts, "rule_terms", om.Int("rule_terms", 0));
                            AppendOmitted(omParts, "detail_chars", om.Int("detail_chars", 0));
                            AppendOmitted(omParts, "intents", om.Int("intents", 0));
                            if (omParts.Count > 0)
                                view.Lines.Add($"  Omitted: {string.Join(", ", omParts)}");
                        }
                    }

                    var authors = tr["author"].AsArray();
                    if (authors != null)
                    {
                        for (int a = 0; a < Math.Min(authors.Count, 8) && view.Lines.Count < 512; a++)
                        {
                            string al = authors[a].AsString();
                            if (!string.IsNullOrEmpty(al))
                                view.Lines.Add($"  Author: {al}");
                        }
                    }

                    var intents = tr["intents"].AsArray();
                    if (intents != null)
                    {
                        for (int k = 0; k < Math.Min(intents.Count, 16) && view.Lines.Count < 512; k++)
                        {
                            var it = intents[k];
                            if (!it.IsObject) continue;
                            string act = it.Str("action", string.Empty);
                            string det = it["detail"].IsNull ? string.Empty : it.Str("detail", string.Empty);
                            string itOut = FormatTraceOutcome(it["outcome"]);
                            view.Lines.Add($"  {act}{(string.IsNullOrEmpty(det) ? "" : " " + det)}: {itOut}");
                        }
                    }
                }
            }

            return view;
        }

        private static void AppendOmitted(List<string> parts, string label, int count)
        {
            if (count > 0) parts.Add($"{label}={count}");
        }

        private static string FormatTraceOutcome(JsonNode o)
        {
            if (o == null || !o.IsObject) return "unknown";
            string st = o.Str("state", "unknown");
            if (st == "refused" || st == "cancelled")
                return $"{st}: {o.Str("reason", string.Empty)}";
            if (st == "faulted")
                return $"faulted: {o.Str("fault", string.Empty)}";
            return st;
        }

        private static List<MindDiagnosticView> DecodeDiagnostics(JsonNode arrNode)
        {
            var list = new List<MindDiagnosticView>();
            var arr = arrNode.AsArray();
            if (arr == null) return list;
            for (int i = 0; i < arr.Count; i++)
            {
                var d = arr[i];
                if (!d.IsObject) continue;
                list.Add(new MindDiagnosticView
                {
                    Field = d.Str("field", "mind"),
                    Message = d.Str("message", string.Empty),
                    Line = d.ContainsKey("line") && !d["line"].IsNull ? (int?)d.Int("line", 0) : null,
                    Column = d.ContainsKey("column") && !d["column"].IsNull ? (int?)d.Int("column", 0) : null,
                });
            }
            return list;
        }

        private static List<MindPictureReportView> DecodePictureReports(JsonNode arrNode)
        {
            var list = new List<MindPictureReportView>();
            var arr = arrNode.AsArray();
            if (arr == null) return list;
            for (int i = 0; i < arr.Count; i++)
            {
                var r = arr[i];
                if (!r.IsObject) continue;
                var rep = new MindPictureReportView
                {
                    Picture = r.Str("picture", $"picture_{i + 1}"),
                    Armed = r.Bool("armed", false),
                    Operations = r.Long("operations", 0),
                    Fees = r.Long("fees", 0),
                    Fault = r.ContainsKey("fault") && !r["fault"].IsNull ? r.Str("fault", null) : null,
                };
                ReadStringList(r["intents"], rep.Intents);
                ReadStringList(r["station_refusals"], rep.StationRefusals);
                ReadStringList(r["class_violations"], rep.ClassViolations);
                list.Add(rep);
            }
            return list;
        }

        private static void ReadStringList(JsonNode arrNode, List<string> dest)
        {
            var arr = arrNode.AsArray();
            if (arr == null) return;
            for (int i = 0; i < arr.Count; i++)
            {
                string s = arr[i].AsString();
                if (!string.IsNullOrEmpty(s))
                    dest.Add(s);
            }
        }

        // ---- Live Event Handlers & Outbound Actions ----

        private void HandlePanelUpdated(string feed, JsonNode data)
        {
            if (feed == "mind.market")
            {
                MarketPage = DecodeMindMarketView(data);
                RefreshMarketPane();
            }
            else if (feed == "mind.holdings")
            {
                HoldingsPage = DecodeMindHoldingsView(data);
                RefreshHoldingsPane();
            }
            else if (feed == "arena.mind")
            {
                TracePage = DecodeMindTraceView(data);
                RefreshTracePane();
            }
        }

        private void HandleSystemLine(string plainLine)
        {
            if (!IsTerminalOpen || string.IsNullOrWhiteSpace(plainLine))
                return;
            // Ignore room chat / channel lines that incidentally mention "mind" (`L-3`).
            if (plainLine.IndexOf(" says, \"", StringComparison.Ordinal) >= 0)
                return;
            if (plainLine.IndexOf("mind", StringComparison.OrdinalIgnoreCase) >= 0 ||
                plainLine.IndexOf("posture", StringComparison.OrdinalIgnoreCase) >= 0 ||
                plainLine.IndexOf("licen", StringComparison.OrdinalIgnoreCase) >= 0 ||
                plainLine.IndexOf("end-user", StringComparison.OrdinalIgnoreCase) >= 0)
            {
                SetStatus(plainLine.Trim());
            }
        }

        public void HandleInvokeResult(ulong requestId, string service, JsonNode res)
        {
            if (service != "mind" || res == null || !res.IsObject)
                return;

            string status = res.Str("status", string.Empty);
            bool isProgressOnly = status == "queued" || status == "pending";
            _pendingRequests.TryGetValue(requestId, out var pending);
            if (!isProgressOnly && pending != null)
                _pendingRequests.Remove(requestId);

            switch (status)
            {
                case "queued":
                case "pending":
                    SetStatus($"Job #{res.Long("job", 0)} {status}...");
                    break;

                case "error":
                {
                    string code = res.Str("code", "error");
                    string message = res.Str("message", "Request failed.");
                    if (pending != null && pending.Op == PendingMindOp.Save)
                        SetStatus($"Save failed ({code}): {message}");
                    else
                        SetStatus($"Refused ({code}): {message}");
                    break;
                }

                case "cancelled":
                {
                    SetStatus($"Job #{res.Long("job", 0)} cancelled.");
                    break;
                }

                case "library":
                {
                    _libraryNames.Clear();
                    var arr = res["names"].AsArray();
                    if (arr != null)
                    {
                        for (int i = 0; i < Math.Min(arr.Count, 64); i++)
                        {
                            string nm = arr[i].AsString();
                            if (!string.IsNullOrEmpty(nm))
                                _libraryNames.Add(nm);
                        }
                    }
                    SetStatus($"Loaded {_libraryNames.Count} working mind(s) in library.");
                    RefreshWorkshopPane();
                    break;
                }

                case "read":
                {
                    // Guard against out-of-order stale `read` responses and dirty local edits (`U-H28`).
                    if (pending != null && pending.Op == PendingMindOp.Read && _latestReadRequestId != 0 && requestId != _latestReadRequestId)
                    {
                        break;
                    }
                    if (IsDraftDirty)
                    {
                        bool userEditedSinceReadSent = pending != null && _localDraftEditVersion != pending.DraftEditVersionAtSend;
                        bool allowOverwriteDirty = pending != null &&
                            pending.Op == PendingMindOp.Read &&
                            pending.AllowDiscardDirty &&
                            !userEditedSinceReadSent;
                        if (!allowOverwriteDirty)
                        {
                            SetStatus("Ignored out-of-order read to preserve unsaved local Rhai edits.");
                            break;
                        }
                    }

                    LoadedMindName = res.Str("name", string.Empty);
                    LoadedDraftBase = res["base"] != null && !res["base"].IsNull ? res["base"].Clone() : null;
                    LoadedDraft = res["draft"] != null && !res["draft"].IsNull ? res["draft"].Clone() : null;
                    IsDraftDirty = false;
                    _draftSessionGeneration++;
                    _draftBaseGeneration++;
                    _localDraftEditVersion++;
                    _lastDiagnostics.Clear();
                    _lastDiagnostics.AddRange(DecodeDiagnostics(res["diagnostics"]));
                    _lastReports.Clear();

                    if (_rhaiSourceInput != null)
                    {
                        bool hasDraft = LoadedDraft != null && LoadedDraft.IsObject;
                        _suppressSourceDirtyCallback = true;
                        _rhaiSourceInput.text = hasDraft ? LoadedDraft.Str("source", string.Empty) : string.Empty;
                        _suppressSourceDirtyCallback = false;
                        _rhaiSourceInput.interactable = hasDraft;
                    }
                    SetStatus(_lastDiagnostics.Count > 0
                        ? $"Opened working copy '{LoadedMindName}' ({_lastDiagnostics.Count} diagnostic(s))."
                        : $"Opened working copy '{LoadedMindName}'.");
                    RefreshWorkshopPane();
                    break;
                }

                case "edited":
                {
                    if (pending != null && pending.Op == PendingMindOp.Create)
                    {
                        string createdName = res.Str("name", pending.TargetName);
                        bool switchedOrEditedDuringCreate =
                            pending.DraftSessionGenerationAtSend != _draftSessionGeneration ||
                            (IsDraftDirty && pending.DraftEditVersionAtSend != _localDraftEditVersion);
                        if (switchedOrEditedDuringCreate)
                        {
                            SetStatus($"Created '{createdName}' in background; kept active draft '{LoadedMindName}'.");
                            RequestLibraryList();
                            RefreshWorkshopPane();
                            break;
                        }

                        // Reset stale previous draft immediately so new `base` is never paired with old `LoadedDraft` (`U-H28`).
                        LoadedMindName = createdName;
                        LoadedDraftBase = res["base"] != null && !res["base"].IsNull ? res["base"].Clone() : null;
                        LoadedDraft = null;
                        IsDraftDirty = false;
                        _draftSessionGeneration++;
                        _draftBaseGeneration++;
                        _localDraftEditVersion++;
                        _lastDiagnostics.Clear();
                        _lastReports.Clear();
                        if (_rhaiSourceInput != null)
                        {
                            _suppressSourceDirtyCallback = true;
                            _rhaiSourceInput.text = string.Empty;
                            _suppressSourceDirtyCallback = false;
                            _rhaiSourceInput.interactable = false;
                        }
                        SetStatus($"Created '{createdName}'; loading working copy...");
                        RequestReadMind(createdName);
                        RequestLibraryList();
                        RefreshWorkshopPane();
                        break;
                    }

                    string editedName = res.Str("name", pending != null ? pending.TargetName : string.Empty);
                    bool matchesActiveDraft =
                        pending != null &&
                        pending.Op == PendingMindOp.Replace &&
                        !string.IsNullOrEmpty(LoadedMindName) &&
                        string.Equals(pending.TargetName, LoadedMindName, StringComparison.OrdinalIgnoreCase) &&
                        (string.IsNullOrEmpty(editedName) || string.Equals(editedName, LoadedMindName, StringComparison.OrdinalIgnoreCase)) &&
                        pending.DraftSessionGenerationAtSend == _draftSessionGeneration &&
                        pending.DraftBaseGenerationAtSend == _draftBaseGeneration;

                    if (!matchesActiveDraft)
                    {
                        string targetLabel = !string.IsNullOrEmpty(editedName)
                            ? editedName
                            : (pending != null && !string.IsNullOrEmpty(pending.TargetName) ? pending.TargetName : "unknown");
                        SetStatus($"Ignored late edit reply for '{targetLabel}' (active draft is '{LoadedMindName}').");
                        RequestLibraryList();
                        break;
                    }

                    if (res["base"] != null && !res["base"].IsNull)
                    {
                        LoadedDraftBase = res["base"].Clone();
                        _draftBaseGeneration++;
                    }

                    // Preserve dirty flag if the player typed additional edits while `replace` was in flight (`U-H28`).
                    bool editedWhileInFlight = _localDraftEditVersion != pending.DraftEditVersionAtSend;
                    IsDraftDirty = editedWhileInFlight;

                    bool chainCheck = pending.ChainCheckAfterReplace;
                    bool chainSave = pending.ChainSaveAfterReplace;
                    string saveNotes = pending.PendingSaveNotes;

                    if (chainCheck)
                    {
                        if (IsDraftDirty)
                            SendReplaceWorkingCopy(chainCheck: true, chainSave: false, saveNotes: string.Empty);
                        else
                            SendWorkshopCheckInternal();
                    }
                    else if (chainSave)
                    {
                        if (IsDraftDirty)
                            SendReplaceWorkingCopy(chainCheck: false, chainSave: true, saveNotes: saveNotes);
                        else
                            SendWorkshopSaveInternal(saveNotes);
                    }
                    else
                    {
                        SetStatus($"Updated '{LoadedMindName}'.");
                        if (!IsDraftDirty)
                            RequestReadMind(LoadedMindName);
                        RequestLibraryList();
                    }
                    RefreshWorkshopPane();
                    break;
                }

                case "checked":
                {
                    bool matchesActiveDraft =
                        pending != null &&
                        (pending.Op == PendingMindOp.Check || pending.Op == PendingMindOp.Save) &&
                        !string.IsNullOrEmpty(LoadedMindName) &&
                        string.Equals(pending.TargetName, LoadedMindName, StringComparison.OrdinalIgnoreCase) &&
                        pending.DraftSessionGenerationAtSend == _draftSessionGeneration &&
                        pending.DraftBaseGenerationAtSend == _draftBaseGeneration &&
                        pending.DraftEditVersionAtSend == _localDraftEditVersion;

                    if (!matchesActiveDraft)
                    {
                        string targetLabel = pending != null && !string.IsNullOrEmpty(pending.TargetName)
                            ? $" for '{pending.TargetName}'"
                            : string.Empty;
                        SetStatus($"Ignored stale check result{targetLabel} (active draft changed).");
                        break;
                    }

                    _lastDiagnostics.Clear();
                    _lastDiagnostics.AddRange(DecodeDiagnostics(res["diagnostics"]));
                    _lastReports.Clear();
                    _lastReports.AddRange(DecodePictureReports(res["reports"]));

                    bool wasSave = pending.Op == PendingMindOp.Save;
                    int faultOrViolationCount = 0;
                    for (int i = 0; i < _lastReports.Count; i++)
                    {
                        if (!string.IsNullOrEmpty(_lastReports[i].Fault) || _lastReports[i].ClassViolations.Count > 0)
                            faultOrViolationCount++;
                    }

                    if (wasSave)
                    {
                        string firstErr = _lastDiagnostics.Count > 0
                            ? FormatDiagnosticSummary(_lastDiagnostics[0])
                            : "fix diagnostics below.";
                        SetStatus($"Save rejected — {firstErr}");
                    }
                    else if (_lastDiagnostics.Count > 0 || faultOrViolationCount > 0)
                    {
                        SetStatus($"Check failed: {_lastDiagnostics.Count} diagnostic(s), {faultOrViolationCount} picture fault(s)/violation(s).");
                    }
                    else
                    {
                        SetStatus($"Check passed: {_lastReports.Count} isolated picture report(s), 0 diagnostics.");
                    }
                    RefreshWorkshopPane();
                    break;
                }

                case "saved":
                {
                    long rev = res.Long("revision", 1);
                    string dig = res.Str("digest", string.Empty);
                    if (dig.Length > 12) dig = dig.Substring(0, 12);

                    bool matchesActiveDraft =
                        pending != null &&
                        pending.Op == PendingMindOp.Save &&
                        !string.IsNullOrEmpty(LoadedMindName) &&
                        string.Equals(pending.TargetName, LoadedMindName, StringComparison.OrdinalIgnoreCase) &&
                        pending.DraftSessionGenerationAtSend == _draftSessionGeneration &&
                        pending.DraftBaseGenerationAtSend == _draftBaseGeneration &&
                        pending.DraftEditVersionAtSend == _localDraftEditVersion;

                    if (!matchesActiveDraft)
                    {
                        string targetLabel = pending != null && !string.IsNullOrEmpty(pending.TargetName)
                            ? $" for '{pending.TargetName}'"
                            : string.Empty;
                        SetStatus($"Saved revision r{rev} ({dig}){targetLabel} in background; active draft unchanged.");
                        break;
                    }

                    if (res["base"] != null && !res["base"].IsNull)
                    {
                        LoadedDraftBase = res["base"].Clone();
                        _draftBaseGeneration++;
                    }
                    _lastDiagnostics.Clear();
                    SetStatus($"Saved revision r{rev} ({dig}). Existing hull seals keep their earlier revision.");
                    RefreshWorkshopPane();
                    break;
                }

                case "filed_source":
                {
                    string lic = res.Str("licence", string.Empty);
                    string sha = res.Str("source_sha256", string.Empty);
                    if (sha.Length > 12) sha = sha.Substring(0, 12);
                    var srcLines = new List<string>();
                    ReadStringList(res["lines"], srcLines);
                    if (_workshopOutputText != null)
                    {
                        var sb = new StringBuilder(256);
                        sb.AppendLine($"PAID SOURCE — License #{lic} (sha {sha}, from line {res.Int("first", 1)}):");
                        for (int i = 0; i < srcLines.Count; i++)
                            sb.AppendLine(srcLines[i]);
                        _workshopOutputText.text = sb.ToString();
                    }
                    SetStatus($"Loaded {srcLines.Count} line(s) of paid source for license #{lic} ({sha}).");
                    break;
                }

                case "removal_requested":
                {
                    string nm = res.Str("name", string.Empty);
                    SetStatus($"Removal requested for '{nm}' — awaiting server confirmation.");
                    break;
                }

                case "removed":
                {
                    string nm = res.Str("name", string.Empty);
                    _libraryNames.RemoveAll(x => string.Equals(x, nm, StringComparison.OrdinalIgnoreCase));
                    if (string.Equals(LoadedMindName, nm, StringComparison.OrdinalIgnoreCase))
                    {
                        LoadedMindName = string.Empty;
                        LoadedDraftBase = null;
                        LoadedDraft = null;
                        IsDraftDirty = false;
                        _draftSessionGeneration++;
                        _draftBaseGeneration++;
                        _localDraftEditVersion++;
                        if (_rhaiSourceInput != null)
                        {
                            _suppressSourceDirtyCallback = true;
                            _rhaiSourceInput.text = string.Empty;
                            _suppressSourceDirtyCallback = false;
                            _rhaiSourceInput.interactable = false;
                        }
                    }
                    SetStatus($"Removed working mind '{nm}' (retained immutable revisions survive).");
                    RefreshWorkshopPane();
                    break;
                }

                case "pruned":
                {
                    int revs = res.Int("revisions", 0);
                    int retained = res.Int("source_candidates_retained", 0);
                    SetStatus($"Pruned {revs} unreferenced revision(s); {retained} source candidate(s) retained.");
                    break;
                }

                case "staged":
                {
                    SetStatus(res.Str("line", "Market action staged."));
                    RequestMarketListings();
                    RequestHoldings(HoldingsSection);
                    break;
                }

                case "shown":
                {
                    var linesArr = res["lines"].AsArray();
                    if (linesArr != null && linesArr.Count > 0)
                        SetStatus(linesArr[0].AsString(string.Empty));
                    break;
                }

                case "trace":
                {
                    TracePage = DecodeMindTraceView(res["data"]);
                    SetStatus(TracePage != null && TracePage.Available
                        ? $"Loaded {TracePage.Lines.Count} trace line(s) for unit #{TracePage.UnitId}."
                        : "No mind trace available for that unit.");
                    RefreshTracePane();
                    break;
                }
            }
        }

        private ulong SendTrackedMindInvoke(JsonNode req, PendingMindOp op, string targetName = null,
            bool chainCheck = false, bool chainSave = false, string saveNotes = null, bool allowDiscardDirty = false)
        {
            if (_ws == null) return 0;
            ulong reqId = _ws.SendMindInvoke(req);
            if (reqId > 0)
            {
                _pendingRequests[reqId] = new PendingMindRequest
                {
                    Op = op,
                    TargetName = targetName ?? LoadedMindName,
                    DraftSessionGenerationAtSend = _draftSessionGeneration,
                    DraftBaseGenerationAtSend = _draftBaseGeneration,
                    DraftEditVersionAtSend = _localDraftEditVersion,
                    AllowDiscardDirty = allowDiscardDirty,
                    ChainCheckAfterReplace = chainCheck,
                    ChainSaveAfterReplace = chainSave,
                    PendingSaveNotes = saveNotes ?? string.Empty
                };
            }
            return reqId;
        }

        public void RequestMarketListings(string afterCode = null)
        {
            _ws?.SendMindMarketListings(afterCode);
        }

        public void RequestHoldings(string section = "paid", long? afterNum = null)
        {
            HoldingsSection = string.Equals(section, "owner", StringComparison.OrdinalIgnoreCase) ? "owner" : "paid";
            _ws?.SendMindMarketHoldings(HoldingsSection, afterNum);
        }

        public void SendMarketAction(string act, string code = null, long? licence = null, long? filed = null, string form = null)
        {
            if (_ws == null) return;
            var action = JsonNode.NewObject();
            action["act"] = JsonNode.FromString(act);
            if (!string.IsNullOrEmpty(code))
                action["code"] = JsonNode.FromString(code);
            if (licence.HasValue)
                action["licence"] = JsonNode.FromLong(licence.Value);
            if (filed.HasValue)
                action["filed"] = JsonNode.FromLong(filed.Value);
            if (!string.IsNullOrEmpty(form))
            {
                action["form"] = JsonNode.FromString(form);
                action["royalty"] = JsonNode.FromLong(0);
                action["transfer"] = JsonNode.FromBool(true);
            }

            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("market");
            req["action"] = action;
            SendTrackedMindInvoke(req, PendingMindOp.Market);
            SetStatus($"Sent market action '{act}'...");
        }

        public void RequestMarketRefund(string listingCode)
        {
            if (string.IsNullOrWhiteSpace(listingCode)) return;
            SendMarketAction("refund", code: listingCode.Trim());
        }

        public void SealMindToShipBerth(long licenseNumber)
        {
            if (licenseNumber <= 0) return;
            string cmd = $"+ship/mind filed:{licenseNumber} go";
            _ws?.SendLine(cmd);
            SetStatus($"> {cmd}");
        }

        /// <summary>
        /// Safely seals a holding row strictly using its verified `LicenceId` (`owner_licence` on owner rows,
        /// `licence` on paid rows), never passing `FiledId` as a `LicenceId` (`U-C06`).
        /// </summary>
        public bool TrySealHoldingRow(MindHoldingRowView row)
        {
            if (row == null)
            {
                SetStatus("Cannot seal: no holding row selected.");
                return false;
            }
            if (!row.LicenceId.HasValue || row.LicenceId.Value <= 0)
            {
                SetStatus("Cannot seal: owner_licence is absent; ship sealing requires a valid license ID, not a filed ID.");
                return false;
            }
            if (row.Revoked || string.Equals(row.PhaseState, "lapsed", StringComparison.OrdinalIgnoreCase))
            {
                SetStatus($"Cannot seal license #{row.LicenceId.Value}: status is {(row.Revoked ? "revoked" : row.PhaseState)}.");
                return false;
            }
            if (_sealLicenseInput != null)
                _sealLicenseInput.text = row.LicenceId.Value.ToString();
            SealMindToShipBerth(row.LicenceId.Value);
            return true;
        }

        public void UnsealShipMind()
        {
            const string cmd = "+ship/mind off go";
            _ws?.SendLine(cmd);
            SetStatus($"> {cmd}");
        }

        public void SetShipLinkdeadPosture(string posture)
        {
            string p = (posture ?? "return").Trim().ToLowerInvariant();
            if (p != "guard" && p != "hold") p = "return";
            string cmd = $"+ship/posture {p}";
            _ws?.SendLine(cmd);
            SetStatus($"> {cmd}");
        }

        public void RequestLibraryList()
        {
            if (_ws == null) return;
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("list");
            SendTrackedMindInvoke(req, PendingMindOp.List);
        }

        public void RequestReadMind(string name, bool allowDiscardDirty = false)
        {
            if (_ws == null || string.IsNullOrWhiteSpace(name)) return;
            string trimmed = name.Trim();
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("read");
            req["name"] = JsonNode.FromString(trimmed);
            req["revision"] = JsonNode.Null;
            _latestReadRequestId = SendTrackedMindInvoke(req, PendingMindOp.Read, targetName: trimmed, allowDiscardDirty: allowDiscardDirty);
        }

        public void RequestCreateMindFromStandard(string newName)
        {
            if (_ws == null || string.IsNullOrWhiteSpace(newName)) return;
            string trimmed = newName.Trim();
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("create");
            req["name"] = JsonNode.FromString(trimmed);
            req["from"] = JsonNode.FromString("Standard");
            SendTrackedMindInvoke(req, PendingMindOp.Create, targetName: trimmed);
        }

        public void UpdateWorkingSource(string newSource)
        {
            if (LoadedDraft == null || !LoadedDraft.IsObject)
            {
                if (_rhaiSourceInput != null)
                {
                    _suppressSourceDirtyCallback = true;
                    _rhaiSourceInput.text = string.Empty;
                    _suppressSourceDirtyCallback = false;
                    _rhaiSourceInput.interactable = false;
                }
                SetStatus("Open or create a working mind copy before editing.");
                return;
            }
            if (newSource != null && (Encoding.UTF8.GetByteCount(newSource) > MaxSourceBytes || newSource.IndexOf('\0') >= 0))
            {
                SetStatus("Source must fit 32 KiB and contain no NUL characters — edit reverted.");
                if (_rhaiSourceInput != null)
                {
                    _suppressSourceDirtyCallback = true;
                    _rhaiSourceInput.text = LoadedDraft.Str("source", string.Empty);
                    _suppressSourceDirtyCallback = false;
                }
                return;
            }
            LoadedDraft["source"] = JsonNode.FromString(newSource ?? string.Empty);
            _localDraftEditVersion++;
            IsDraftDirty = true;
            RefreshWorkshopPane();
        }

        public void CheckWorkingMind()
        {
            if (_ws == null || string.IsNullOrEmpty(LoadedMindName) || LoadedDraftBase == null)
            {
                SetStatus("Open or create a working mind copy first.");
                return;
            }
            if (IsDraftDirty)
            {
                SendReplaceWorkingCopy(chainCheck: true, chainSave: false, saveNotes: string.Empty);
                return;
            }
            SendWorkshopCheckInternal();
        }

        public void SaveWorkingMindRevision(string notes)
        {
            if (_ws == null || string.IsNullOrEmpty(LoadedMindName) || LoadedDraftBase == null)
            {
                SetStatus("Open or create a working mind copy first.");
                return;
            }
            string cleanNotes = notes ?? string.Empty;
            if (Encoding.UTF8.GetByteCount(cleanNotes) > MaxNotesBytes)
            {
                SetStatus("Revision notes must fit 2 KiB.");
                return;
            }
            if (IsDraftDirty)
            {
                SendReplaceWorkingCopy(chainCheck: false, chainSave: true, saveNotes: cleanNotes);
                return;
            }
            SendWorkshopSaveInternal(cleanNotes);
        }

        private void SendReplaceWorkingCopy(bool chainCheck, bool chainSave, string saveNotes)
        {
            if (LoadedDraft == null || !LoadedDraft.IsObject)
            {
                SetStatus("No editable draft loaded.");
                return;
            }
            string src = LoadedDraft.Str("source", string.Empty);
            if (Encoding.UTF8.GetByteCount(src) > MaxSourceBytes || src.IndexOf('\0') >= 0)
            {
                SetStatus("Source exceeds 32 KiB or contains NUL — replace aborted.");
                return;
            }
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("replace");
            req["name"] = JsonNode.FromString(LoadedMindName);
            req["base"] = LoadedDraftBase.Clone();
            req["draft"] = LoadedDraft.Clone();
            SendTrackedMindInvoke(req, PendingMindOp.Replace, LoadedMindName, chainCheck, chainSave, saveNotes);
        }

        private void SendWorkshopCheckInternal()
        {
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("check");
            req["name"] = JsonNode.FromString(LoadedMindName);
            req["base"] = LoadedDraftBase.Clone();
            SendTrackedMindInvoke(req, PendingMindOp.Check, LoadedMindName);
            SetStatus($"Checking '{LoadedMindName}' against 6 isolated pictures...");
        }

        private void SendWorkshopSaveInternal(string notes)
        {
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("save");
            req["name"] = JsonNode.FromString(LoadedMindName);
            req["base"] = LoadedDraftBase.Clone();
            req["notes"] = JsonNode.FromString(notes ?? string.Empty);
            req["version"] = JsonNode.Null;
            SendTrackedMindInvoke(req, PendingMindOp.Save, LoadedMindName);
            SetStatus($"Saving immutable revision of '{LoadedMindName}'...");
        }

        public void RequestUnitMindTrace(int unitId, bool follow)
        {
            if (_ws == null || unitId <= 0) return;
            _followingTraceUnitId = follow ? unitId : 0;
            if (_stopFollowBtn != null) _stopFollowBtn.interactable = _followingTraceUnitId > 0;
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("trace");
            req["unit"] = JsonNode.FromLong(unitId);
            req["count"] = JsonNode.FromLong(8);
            req["follow"] = JsonNode.FromBool(follow);
            SendTrackedMindInvoke(req, PendingMindOp.Trace);
            SetStatus(follow
                ? $"Following live mind trace for unit #{unitId}..."
                : $"Requesting mind trace for unit #{unitId}...");
        }

        public void StopFollowingMindTrace()
        {
            if (_followingTraceUnitId <= 0) return;
            int unitId = _followingTraceUnitId;
            _followingTraceUnitId = 0;
            if (_stopFollowBtn != null) _stopFollowBtn.interactable = false;
            if (_ws != null)
            {
                var req = JsonNode.NewObject();
                req["op"] = JsonNode.FromString("trace");
                req["unit"] = JsonNode.FromLong(unitId);
                req["count"] = JsonNode.FromLong(1);
                req["follow"] = JsonNode.FromBool(false);
                SendTrackedMindInvoke(req, PendingMindOp.Trace);
            }
            SetStatus($"Stopped following live mind trace for unit #{unitId}.");
        }

        // ---- Two-Click Confirmation Helper (`U-M15`) ----

        public void DisarmConfirm()
        {
            if (_armedConfirmLabel != null && !string.IsNullOrEmpty(_armedOriginalText))
                _armedConfirmLabel.text = _armedOriginalText;
            _armedConfirmKey = null;
            _armedConfirmLabel = null;
            _armedOriginalText = null;
        }

        private void InvokeWithTwoClickConfirm(string confirmKey, Text btnLabel, string originalLabel, string confirmLabel, string previewPrompt, Action onConfirmed)
        {
            if (_armedConfirmKey == confirmKey && (Time.unscaledTime - _armedAtTime) <= ConfirmWindowSeconds)
            {
                DisarmConfirm();
                onConfirmed?.Invoke();
                return;
            }
            DisarmConfirm();
            _armedConfirmKey = confirmKey;
            _armedConfirmLabel = btnLabel;
            _armedOriginalText = originalLabel;
            _armedAtTime = Time.unscaledTime;
            if (btnLabel != null) btnLabel.text = confirmLabel;
            if (!string.IsNullOrEmpty(previewPrompt))
                SetStatus(previewPrompt);
        }

        // ---- uGUI Construction & Tab Switching ----

        public void BuildTerminalUi()
        {
            ActiveInstance = this;
            _font = Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            if (_font == null)
                _font = Font.CreateDynamicFontFromOSFont("Consolas", 14);

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _tabBtnImages.Clear();
            _marketRows.Clear();
            _holdingRows.Clear();
            _libraryButtons.Clear();

            var canvasGo = new GameObject("MindTerminalCanvas");
            canvasGo.transform.SetParent(transform, false);
            _canvas = canvasGo.AddComponent<Canvas>();
            _canvas.renderMode = RenderMode.ScreenSpaceOverlay;
            _canvas.sortingOrder = IsLoginOverlayBlocking() ? SortingOrderHiddenBehindLogin : SortingOrderActiveOnline;

            var scaler = canvasGo.AddComponent<CanvasScaler>();
            scaler.uiScaleMode = CanvasScaler.ScaleMode.ScaleWithScreenSize;
            scaler.referenceResolution = new Vector2(1920f, 1080f);
            scaler.matchWidthOrHeight = 0.5f;
            canvasGo.AddComponent<GraphicRaycaster>();

            // Docked inside right edge of TopStrip (`y = -58..-16`) so it NEVER overlaps DockingHud,
            // SpaceFireControlCard, QuarryCard, or ArenaContactsPanel (`y <= -74`) (`U-H29`).
            _launcherPill = CreatePanel(canvasGo.transform, "MindLauncherPill", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-220f, -58f), new Vector2(-22f, -16f), new Color(0.08f, 0.20f, 0.32f, 0.92f));
            var launchBtn = CreateBtn(_launcherPill.transform, "[F10] MINDS & SEALING", new Color(0.14f, 0.36f, 0.52f), () => SetTerminalOpen(!IsTerminalOpen));
            SetRect(launchBtn.GetComponent<RectTransform>(), Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
            _launcherPill.SetActive(false);

            // Centered modal window
            _windowRoot = CreatePanel(canvasGo.transform, "MindTerminalWindow", new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-490f, -315f), new Vector2(490f, 315f), new Color(0.03f, 0.06f, 0.11f, 0.99f));

            var hdr = CreateTxt(_windowRoot.transform, "MIND MARKET, BERTH SEALING & RHAI DOCTRINE WORKSHOP [F10]", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.52f, 0.90f, 1.0f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -44f), new Vector2(-140f, -8f));

            var closeBtn = CreateBtn(_windowRoot.transform, "CLOSE [ESC]", new Color(0.45f, 0.16f, 0.18f), () => SetTerminalOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-124f, -42f), new Vector2(-16f, -10f));

            var tabRow = CreateRow(_windowRoot.transform, "MindTabsRow", -84f, -48f);
            (MindTerminalTab tab, string label)[] tabs =
            {
                (MindTerminalTab.Market, "MIND MARKET"),
                (MindTerminalTab.HoldingsAndSealing, "HOLDINGS & SHIP SEAL"),
                (MindTerminalTab.Workshop, "RHAI WORKSHOP"),
                (MindTerminalTab.LiveTrace, "LIVE MIND TRACE")
            };
            for (int i = 0; i < tabs.Length; i++)
            {
                var captured = tabs[i].tab;
                var b = CreateBtn(tabRow, tabs[i].label, new Color(0.14f, 0.30f, 0.44f), () => SwitchTab(captured, autoRefresh: true));
                _tabBtnImages.Add(b.GetComponent<Image>());
            }

            _statusBannerText = CreateTxt(_windowRoot.transform, "Ready.", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.66f, 0.95f, 0.80f));
            SetRect(_statusBannerText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(20f, 8f), new Vector2(-20f, 34f));

            BuildMarketPane(_windowRoot.transform);
            BuildHoldingsAndSealingPane(_windowRoot.transform);
            BuildWorkshopPane(_windowRoot.transform);
            BuildTracePane(_windowRoot.transform);

            SwitchTab(MindTerminalTab.Market, autoRefresh: false);
            _windowRoot.SetActive(false);
        }

        public void SetTerminalOpen(bool open)
        {
            if (_windowRoot == null) return;
            if (open && IsLoginOverlayBlocking())
            {
                _windowRoot.SetActive(false);
                if (_canvas != null) _canvas.sortingOrder = SortingOrderHiddenBehindLogin;
                return;
            }
            DisarmConfirm();
            if (!open && _followingTraceUnitId > 0)
                StopFollowingMindTrace();
            _windowRoot.SetActive(open);
            if (_canvas != null)
                _canvas.sortingOrder = open ? SortingOrderActiveOnline : (IsLoginOverlayBlocking() ? SortingOrderHiddenBehindLogin : SortingOrderActiveOnline);
            if (open)
                SwitchTab(ActiveTab, autoRefresh: true);
        }

        public bool TryHandleHotkeyToggle()
        {
            if (!CanOpenViaHotkey)
            {
                if (IsTerminalOpen) SetTerminalOpen(false);
                return false;
            }
            SetTerminalOpen(!IsTerminalOpen);
            return true;
        }

        public void SwitchTab(MindTerminalTab tab, bool autoRefresh = true)
        {
            DisarmConfirm();
            ActiveTab = tab;
            if (_marketPane != null) _marketPane.SetActive(tab == MindTerminalTab.Market);
            if (_holdingsPane != null) _holdingsPane.SetActive(tab == MindTerminalTab.HoldingsAndSealing);
            if (_workshopPane != null) _workshopPane.SetActive(tab == MindTerminalTab.Workshop);
            if (_tracePane != null) _tracePane.SetActive(tab == MindTerminalTab.LiveTrace);

            for (int i = 0; i < _tabBtnImages.Count; i++)
            {
                if (_tabBtnImages[i] == null) continue;
                _tabBtnImages[i].color = ((int)tab == i)
                    ? new Color(0.20f, 0.58f, 0.78f, 1f)
                    : new Color(0.11f, 0.22f, 0.33f, 0.92f);
            }

            if (autoRefresh && _ws != null)
            {
                if (tab == MindTerminalTab.Market) RequestMarketListings();
                else if (tab == MindTerminalTab.HoldingsAndSealing) RequestHoldings(HoldingsSection);
                else if (tab == MindTerminalTab.Workshop) RequestLibraryList();
            }
        }

        private void Update()
        {
            bool loginBlocking = IsLoginOverlayBlocking();
            bool online = _ws != null && _ws.State == MushWsConnectionState.Online && !loginBlocking;

            if (_canvas != null)
            {
                int desiredOrder = loginBlocking ? SortingOrderHiddenBehindLogin : SortingOrderActiveOnline;
                if (_canvas.sortingOrder != desiredOrder)
                    _canvas.sortingOrder = desiredOrder;
            }

            if (loginBlocking && IsTerminalOpen)
                SetTerminalOpen(false);

            if (_launcherPill != null && _launcherPill.activeSelf != online)
                _launcherPill.SetActive(online);

            if (Input.GetKeyDown(KeyCode.F10))
            {
                TryHandleHotkeyToggle();
            }
            else if (IsTerminalOpen && Input.GetKeyDown(KeyCode.Escape) && ResolveActiveHud() == null)
            {
                // When HUD is present, `TacSimHudController.Update()` runs unified single-layer Escape priority (`U-H30`).
                SetTerminalOpen(false);
            }
        }

        private void BuildMarketPane(Transform parent)
        {
            _marketPane = CreatePanel(parent, "MarketPane", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 40f), new Vector2(-16f, -92f), Color.clear);

            var topBar = CreateRow(_marketPane.transform, "MarketTopBar", -36f, 0f);
            CreateBtn(topBar, "Refresh Listings", new Color(0.16f, 0.44f, 0.54f), () => RequestMarketListings());
            CreateBtn(topBar, "Next Page", new Color(0.16f, 0.32f, 0.46f), () =>
            {
                if (MarketPage != null && !string.IsNullOrEmpty(MarketPage.NextAfterCode))
                    RequestMarketListings(MarketPage.NextAfterCode);
            });
            Text enduserLbl = null;
            var enduserBtn = CreateBtn(topBar, "Class III Reg (+enduser)", new Color(0.26f, 0.34f, 0.48f), () =>
            {
                InvokeWithTwoClickConfirm(
                    "enduser_apply",
                    enduserLbl,
                    "Class III Reg (+enduser)",
                    "CONFIRM REG GO",
                    "Previewing +enduser/apply — click CONFIRM REG GO to commit Class III registration.",
                    () =>
                    {
                        _ws?.SendLine("+enduser/apply go");
                        SetStatus("> +enduser/apply go");
                    });
            });
            enduserLbl = enduserBtn.GetComponentInChildren<Text>();

            _marketSummaryText = CreateTxt(_marketPane.transform, "Querying mind.market listings...", 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.82f, 0.92f, 1f));
            SetRect(_marketSummaryText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(4f, -64f), new Vector2(-4f, -40f));

            _marketRowsContainer = CreateScrollListArea(_marketPane.transform, "MarketRowsViewport", -68f);
            EnsureMarketRowPool(InitialPooledRows);
        }

        private void BuildHoldingsAndSealingPane(Transform parent)
        {
            _holdingsPane = CreatePanel(parent, "HoldingsAndSealingPane", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 40f), new Vector2(-16f, -92f), Color.clear);

            var sealBar = CreateRow(_holdingsPane.transform, "ShipMindSealBar", -38f, 0f);
            CreateBtn(sealBar, "Inspect (+ship/mind)", new Color(0.16f, 0.42f, 0.52f), () =>
            {
                _ws?.SendLine("+ship/mind");
                SetStatus("> +ship/mind");
            });
            _sealLicenseInput = CreateMiniInput(sealBar, "Lic #", "1");
            Text sealBarLbl = null;
            var sealBarBtn = CreateBtn(sealBar, "Seal To Berth", new Color(0.16f, 0.50f, 0.36f), () =>
            {
                if (!long.TryParse(_sealLicenseInput != null ? _sealLicenseInput.text.Trim() : "1", out long n) || n <= 0)
                {
                    SetStatus("Enter a valid positive License # to seal onto your berthed ship.");
                    return;
                }
                InvokeWithTwoClickConfirm(
                    $"seal_manual_{n}",
                    sealBarLbl,
                    "Seal To Berth",
                    "CONFIRM SEAL",
                    $"Confirm sealing License #{n} onto berthed hull (+ship/mind filed:{n} go): click CONFIRM SEAL.",
                    () => SealMindToShipBerth(n));
            });
            sealBarLbl = sealBarBtn.GetComponentInChildren<Text>();

            Text unsealLbl = null;
            var unsealBtn = CreateBtn(sealBar, "Unseal (+ship/mind off)", new Color(0.48f, 0.22f, 0.20f), () =>
            {
                InvokeWithTwoClickConfirm(
                    "unseal_berth",
                    unsealLbl,
                    "Unseal (+ship/mind off)",
                    "CONFIRM UNSEAL",
                    "Confirm unsealing ship mind (+ship/mind off go): click CONFIRM UNSEAL.",
                    UnsealShipMind);
            });
            unsealLbl = unsealBtn.GetComponentInChildren<Text>();

            CreateBtn(sealBar, "Posture: Return", new Color(0.18f, 0.36f, 0.48f), () => SetShipLinkdeadPosture("return"));
            CreateBtn(sealBar, "Posture: Guard", new Color(0.38f, 0.30f, 0.16f), () => SetShipLinkdeadPosture("guard"));
            CreateBtn(sealBar, "Posture: Hold", new Color(0.24f, 0.30f, 0.42f), () => SetShipLinkdeadPosture("hold"));

            var secBar = CreateRow(_holdingsPane.transform, "HoldingsSectionBar", -78f, -44f);
            CreateBtn(secBar, "Paid Licenses", new Color(0.16f, 0.42f, 0.54f), () => RequestHoldings("paid"));
            CreateBtn(secBar, "Filed Originals", new Color(0.24f, 0.36f, 0.50f), () => RequestHoldings("owner"));
            CreateBtn(secBar, "Next Page", new Color(0.16f, 0.30f, 0.42f), () =>
            {
                if (HoldingsPage != null && HoldingsPage.NextAfterNumber.HasValue)
                    RequestHoldings(HoldingsSection, HoldingsPage.NextAfterNumber.Value);
            });

            _holdingsSummaryText = CreateTxt(_holdingsPane.transform, "Select Paid Licenses or Filed Originals above, or seal a license directly onto your berthed ship's core.", 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.82f, 0.92f, 1f));
            SetRect(_holdingsSummaryText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(4f, -104f), new Vector2(-4f, -82f));

            _holdingRowsContainer = CreateScrollListArea(_holdingsPane.transform, "HoldingsRowsViewport", -108f);
            EnsureHoldingRowPool(InitialPooledRows);
        }

        private void BuildWorkshopPane(Transform parent)
        {
            _workshopPane = CreatePanel(parent, "WorkshopPane", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 40f), new Vector2(-16f, -92f), Color.clear);

            var libRow = CreateRow(_workshopPane.transform, "LibraryRow", -36f, 0f);
            CreateBtn(libRow, "Refresh Library", new Color(0.16f, 0.42f, 0.54f), RequestLibraryList);
            _newMindNameInput = CreateMiniInput(libRow, "New name...", "escort_v1");
            Text newStdLbl = null;
            var newStdBtn = CreateBtn(libRow, "New From Standard", new Color(0.16f, 0.48f, 0.36f), () =>
            {
                string nm = _newMindNameInput != null ? _newMindNameInput.text : "escort_v1";
                if (IsDraftDirty)
                {
                    InvokeWithTwoClickConfirm(
                        "create_discard_dirty",
                        newStdLbl,
                        "New From Standard",
                        "DISCARD & CREATE",
                        $"Unsaved edits in '{LoadedMindName}' — click DISCARD & CREATE to create '{nm}'.",
                        () => RequestCreateMindFromStandard(nm));
                }
                else
                {
                    RequestCreateMindFromStandard(nm);
                }
            });
            newStdLbl = newStdBtn.GetComponentInChildren<Text>();

            _saveNotesInput = CreateMiniInput(libRow, "Revision notes...", "Tuned doctrine");
            CreateBtn(libRow, "Check 6 Pictures", new Color(0.20f, 0.38f, 0.54f), CheckWorkingMind);
            CreateBtn(libRow, "Save Revision", new Color(0.48f, 0.32f, 0.14f), () =>
                SaveWorkingMindRevision(_saveNotesInput != null ? _saveNotesInput.text : string.Empty));

            // Horizontal ScrollRect supporting all 20 library minds (`48d H-43`).
            var namesVp = CreatePanel(_workshopPane.transform, "OwnedNamesViewport", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -74f), new Vector2(0f, -40f), new Color(0.02f, 0.05f, 0.09f, 0.72f));
            namesVp.AddComponent<RectMask2D>();
            var namesSr = namesVp.AddComponent<ScrollRect>();
            var namesContent = new GameObject("OwnedNamesContent", typeof(RectTransform), typeof(HorizontalLayoutGroup), typeof(ContentSizeFitter));
            namesContent.transform.SetParent(namesVp.transform, false);
            var nRt = namesContent.GetComponent<RectTransform>();
            nRt.anchorMin = new Vector2(0f, 0f);
            nRt.anchorMax = new Vector2(0f, 1f);
            nRt.pivot = new Vector2(0f, 0.5f);
            nRt.offsetMin = Vector2.zero;
            nRt.offsetMax = Vector2.zero;
            var nHlg = namesContent.GetComponent<HorizontalLayoutGroup>();
            nHlg.spacing = 6f;
            nHlg.padding = new RectOffset(4, 4, 2, 2);
            nHlg.childControlWidth = false;
            nHlg.childControlHeight = true;
            nHlg.childForceExpandWidth = false;
            nHlg.childForceExpandHeight = true;
            var nCsf = namesContent.GetComponent<ContentSizeFitter>();
            nCsf.horizontalFit = ContentSizeFitter.FitMode.PreferredSize;
            nCsf.verticalFit = ContentSizeFitter.FitMode.Unconstrained;
            namesSr.viewport = namesVp.GetComponent<RectTransform>();
            namesSr.content = nRt;
            namesSr.horizontal = true;
            namesSr.vertical = false;
            namesSr.movementType = ScrollRect.MovementType.Clamped;
            _libraryButtonsContainer = namesContent.transform;
            EnsureLibraryButtonPool(InitialPooledRows);

            _workshopMetaText = CreateTxt(_workshopPane.transform, "Working Copy: (none loaded)", 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.84f, 0.94f, 1f));
            SetRect(_workshopMetaText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(4f, -100f), new Vector2(-4f, -78f));

            // Scrollable Rhai Source Editor (removes 20-line truncation; `U-H27`).
            var editorVp = CreatePanel(_workshopPane.transform, "RhaiEditorViewport", new Vector2(0f, 0.38f), new Vector2(1f, 1f), new Vector2(0f, 4f), new Vector2(0f, -104f), new Color(0.04f, 0.08f, 0.14f, 0.96f));
            editorVp.AddComponent<RectMask2D>();
            var editorSr = editorVp.AddComponent<ScrollRect>();

            var editorBox = CreatePanel(editorVp.transform, "RhaiEditorBox", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, Vector2.zero, Color.clear);
            var editorRt = editorBox.GetComponent<RectTransform>();
            editorRt.pivot = new Vector2(0.5f, 1f);
            editorRt.sizeDelta = new Vector2(0f, 220f);
            var editorLe = editorBox.AddComponent<LayoutElement>();
            editorLe.minHeight = 200f;
            var editorCsf = editorBox.AddComponent<ContentSizeFitter>();
            editorCsf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            editorCsf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;

            _rhaiSourceInput = editorBox.AddComponent<InputField>();
            _rhaiSourceInput.lineType = InputField.LineType.MultiLineNewline;
            _rhaiSourceInput.interactable = false;

            var srcPh = CreateTxt(editorBox.transform, "// Open or create a working mind above to inspect & edit its Rhai source...", 13, FontStyle.Italic, TextAnchor.UpperLeft, new Color(0.45f, 0.58f, 0.70f));
            SetRect(srcPh.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));

            var srcTxt = CreateTxt(editorBox.transform, string.Empty, 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.90f, 0.97f, 1f));
            srcTxt.horizontalOverflow = HorizontalWrapMode.Wrap;
            srcTxt.verticalOverflow = VerticalWrapMode.Overflow;
            srcTxt.supportRichText = false;
            SetRect(srcTxt.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));

            _rhaiSourceInput.placeholder = srcPh;
            _rhaiSourceInput.textComponent = srcTxt;
            _rhaiSourceInput.onValueChanged.AddListener(val =>
            {
                if (!_suppressSourceDirtyCallback)
                    UpdateWorkingSource(val);
            });

            editorSr.viewport = editorVp.GetComponent<RectTransform>();
            editorSr.content = editorRt;
            editorSr.horizontal = false;
            editorSr.vertical = true;
            editorSr.movementType = ScrollRect.MovementType.Clamped;
            editorSr.scrollSensitivity = 28f;

            // Scrollable Diagnostics & 6-Picture Reports Output Pane (`U-H26`).
            var diagVp = CreatePanel(_workshopPane.transform, "WorkshopOutputViewport", new Vector2(0f, 0f), new Vector2(1f, 0.38f), Vector2.zero, new Vector2(0f, -4f), new Color(0.02f, 0.05f, 0.09f, 0.94f));
            diagVp.AddComponent<RectMask2D>();
            var diagSr = diagVp.AddComponent<ScrollRect>();
            _workshopOutputText = CreateTxt(diagVp.transform, "Diagnostics and isolated picture reports will appear here after Check or Save.", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.94f, 1f));
            _workshopOutputText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _workshopOutputText.verticalOverflow = VerticalWrapMode.Overflow;
            _workshopOutputText.supportRichText = false;
            var diagCsf = _workshopOutputText.gameObject.AddComponent<ContentSizeFitter>();
            diagCsf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            diagCsf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;
            _workshopOutputText.rectTransform.anchorMin = new Vector2(0f, 1f);
            _workshopOutputText.rectTransform.anchorMax = new Vector2(1f, 1f);
            _workshopOutputText.rectTransform.pivot = new Vector2(0.5f, 1f);
            _workshopOutputText.rectTransform.offsetMin = new Vector2(12f, 0f);
            _workshopOutputText.rectTransform.offsetMax = new Vector2(-12f, 0f);
            diagSr.viewport = diagVp.GetComponent<RectTransform>();
            diagSr.content = _workshopOutputText.rectTransform;
            diagSr.horizontal = false;
            diagSr.vertical = true;
            diagSr.movementType = ScrollRect.MovementType.Clamped;
            diagSr.scrollSensitivity = 28f;
        }

        private void BuildTracePane(Transform parent)
        {
            _tracePane = CreatePanel(parent, "TracePane", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 40f), new Vector2(-16f, -92f), Color.clear);

            var topRow = CreateRow(_tracePane.transform, "TraceControlsRow", -38f, 0f);
            _traceUnitInput = CreateMiniInput(topRow, "Unit #", "1");
            CreateBtn(topRow, "Pull 8 Thinks", new Color(0.16f, 0.44f, 0.54f), () =>
            {
                if (int.TryParse(_traceUnitInput != null ? _traceUnitInput.text.Trim().TrimStart('#') : "1", out int u) && u > 0)
                    RequestUnitMindTrace(u, follow: false);
            });
            CreateBtn(topRow, "Follow Live Trace", new Color(0.16f, 0.50f, 0.38f), () =>
            {
                if (int.TryParse(_traceUnitInput != null ? _traceUnitInput.text.Trim().TrimStart('#') : "1", out int u) && u > 0)
                    RequestUnitMindTrace(u, follow: true);
            });
            _stopFollowBtn = CreateBtn(topRow, "Stop Following", new Color(0.46f, 0.22f, 0.20f), StopFollowingMindTrace);
            _stopFollowBtn.interactable = false;

            var vp = CreatePanel(_tracePane.transform, "TraceViewport", new Vector2(0f, 0f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, -44f), new Color(0.02f, 0.05f, 0.09f, 0.90f));
            vp.AddComponent<RectMask2D>();
            var sr = vp.AddComponent<ScrollRect>();
            _traceOutputText = CreateTxt(vp.transform, "Enter a unit or ship # above to inspect source-free Think diagnostics (`arena.mind`).", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 1f));
            _traceOutputText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _traceOutputText.verticalOverflow = VerticalWrapMode.Overflow;
            _traceOutputText.supportRichText = false;
            var csf = _traceOutputText.gameObject.AddComponent<ContentSizeFitter>();
            csf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            csf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;
            _traceOutputText.rectTransform.anchorMin = new Vector2(0f, 1f);
            _traceOutputText.rectTransform.anchorMax = new Vector2(1f, 1f);
            _traceOutputText.rectTransform.pivot = new Vector2(0.5f, 1f);
            _traceOutputText.rectTransform.offsetMin = new Vector2(12f, 0f);
            _traceOutputText.rectTransform.offsetMax = new Vector2(-12f, 0f);
            sr.viewport = vp.GetComponent<RectTransform>();
            sr.content = _traceOutputText.rectTransform;
            sr.horizontal = false;
            sr.vertical = true;
            sr.movementType = ScrollRect.MovementType.Clamped;
            sr.scrollSensitivity = 28f;
        }

        private Transform CreateScrollListArea(Transform parent, string name, float topInset)
        {
            var vp = CreatePanel(parent, name, new Vector2(0f, 0f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, topInset), new Color(0.02f, 0.05f, 0.09f, 0.88f));
            vp.AddComponent<RectMask2D>();
            var sr = vp.AddComponent<ScrollRect>();

            var contentGo = new GameObject(name + "_Content", typeof(RectTransform), typeof(VerticalLayoutGroup), typeof(ContentSizeFitter));
            contentGo.transform.SetParent(vp.transform, false);
            var cRt = contentGo.GetComponent<RectTransform>();
            cRt.anchorMin = new Vector2(0f, 1f);
            cRt.anchorMax = new Vector2(1f, 1f);
            cRt.pivot = new Vector2(0.5f, 1f);
            cRt.offsetMin = Vector2.zero;
            cRt.offsetMax = Vector2.zero;

            var vlg = contentGo.GetComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            var csf = contentGo.GetComponent<ContentSizeFitter>();
            csf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            csf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;

            sr.viewport = vp.GetComponent<RectTransform>();
            sr.content = cRt;
            sr.horizontal = false;
            sr.vertical = true;
            sr.movementType = ScrollRect.MovementType.Clamped;
            sr.scrollSensitivity = 28f;
            return contentGo.transform;
        }

        private void EnsureMarketRowPool(int needed)
        {
            if (_marketRowsContainer == null) return;
            while (_marketRows.Count < needed)
                _marketRows.Add(CreatePooledActionRow(_marketRowsContainer, $"MktRow_{_marketRows.Count}", rowHeight: 48f));
        }

        private void EnsureHoldingRowPool(int needed)
        {
            if (_holdingRowsContainer == null) return;
            while (_holdingRows.Count < needed)
                _holdingRows.Add(CreatePooledActionRow(_holdingRowsContainer, $"HoldRow_{_holdingRows.Count}", rowHeight: 42f));
        }

        private void EnsureLibraryButtonPool(int needed)
        {
            if (_libraryButtonsContainer == null) return;
            while (_libraryButtons.Count < needed)
            {
                int slot = _libraryButtons.Count;
                Text btnLbl = null;
                var b = CreateBtn(_libraryButtonsContainer, "-", new Color(0.12f, 0.24f, 0.36f), () =>
                {
                    if (slot >= _libraryNames.Count) return;
                    string target = _libraryNames[slot];
                    bool isCurrentMind = string.Equals(target, LoadedMindName, StringComparison.OrdinalIgnoreCase);
                    if (IsDraftDirty && isCurrentMind)
                    {
                        DisarmConfirm();
                        SetStatus($"'{LoadedMindName}' is already open with unsaved edits.");
                        return;
                    }
                    if (IsDraftDirty)
                    {
                        InvokeWithTwoClickConfirm(
                            $"switch_mind_{target}",
                            btnLbl,
                            target,
                            $"DISCARD->{target}",
                            $"Unsaved edits in '{LoadedMindName}' — click again to discard & open '{target}'.",
                            () => RequestReadMind(target, allowDiscardDirty: true));
                    }
                    else
                    {
                        RequestReadMind(target);
                    }
                });
                btnLbl = b.GetComponentInChildren<Text>();
                var le = b.gameObject.AddComponent<LayoutElement>();
                le.minWidth = 104f;
                le.preferredWidth = 128f;
                b.gameObject.SetActive(false);
                _libraryButtons.Add(b);
            }
        }

        private void RefreshMarketPane()
        {
            if (_marketSummaryText == null) return;
            if (MarketPage == null)
            {
                _marketSummaryText.text = "No such market page.";
                for (int i = 0; i < _marketRows.Count; i++) _marketRows[i].Root.SetActive(false);
                return;
            }

            _marketSummaryText.text = !string.IsNullOrEmpty(MarketPage.Notice)
                ? MarketPage.Notice
                : $"Showing {MarketPage.Rows.Count} listing(s) · Active Quotes: {MarketPage.OwnQuotes.Count} · Waiting Escrow: {MarketPage.WaitingCodes.Count}";

            int totalRows = MarketPage.Rows.Count + MarketPage.OwnQuotes.Count + MarketPage.WaitingCodes.Count;
            EnsureMarketRowPool(totalRows);

            int rowIndex = 0;

            // 1. Live market listings (including decoded TermsLine / SplitLine / Transferable; `U-M15`, `48d H-43`).
            for (int i = 0; i < MarketPage.Rows.Count; i++)
            {
                var r = MarketPage.Rows[i];
                var slot = _marketRows[rowIndex++];
                string code = r.Code;
                string safeCode = MushProtocol.EscapeUnityRichText(code);
                string safeName = MushProtocol.EscapeUnityRichText(r.Name);
                string safeClass = MushProtocol.EscapeUnityRichText(r.ClassWord);
                string safeForm = MushProtocol.EscapeUnityRichText(r.Form);
                string safePrice = MushProtocol.EscapeUnityRichText(r.PriceText);
                string xfer = r.Transferable ? "xfer" : "non-xfer";
                var desc = new StringBuilder(160);
                desc.Append($"<b>[{safeCode}] {safeName}</b> · Class {safeClass} ({safeForm}, {xfer}) · <b>{safePrice}</b>");
                if (!string.IsNullOrEmpty(r.TermsLine) || !string.IsNullOrEmpty(r.SplitLine))
                {
                    desc.Append("\n<color=#8FB5CC>");
                    if (!string.IsNullOrEmpty(r.TermsLine))
                        desc.Append(MushProtocol.EscapeUnityRichText(r.TermsLine));
                    if (!string.IsNullOrEmpty(r.TermsLine) && !string.IsNullOrEmpty(r.SplitLine))
                        desc.Append("  |  ");
                    if (!string.IsNullOrEmpty(r.SplitLine))
                        desc.Append(MushProtocol.EscapeUnityRichText(r.SplitLine));
                    desc.Append("</color>");
                }
                slot.Background.color = new Color(0.07f, 0.13f, 0.20f, 0.94f);
                slot.Label.text = desc.ToString();

                BindRowButton(slot.BtnA, slot.BtnALabel, "Quote", () => SendMarketAction("quote", code: code), out slot.ActA);
                BindRowButton(slot.BtnB, slot.BtnBLabel, "Pay", () =>
                {
                    InvokeWithTwoClickConfirm(
                        $"mkt_pay_{code}",
                        slot.BtnBLabel,
                        "Pay",
                        "CONFIRM PAY",
                        $"Confirm paying quote [{code}] ({r.PriceText}) into escrow: click CONFIRM PAY.",
                        () => SendMarketAction("pay", code: code));
                }, out slot.ActB);
                BindRowButton(slot.BtnC, slot.BtnCLabel, "Complete", () => SendMarketAction("complete", code: code), out slot.ActC);
                slot.Root.SetActive(true);
            }

            // 2. Own open quotes (`U-H25`).
            for (int i = 0; i < MarketPage.OwnQuotes.Count; i++)
            {
                var q = MarketPage.OwnQuotes[i];
                var slot = _marketRows[rowIndex++];
                string code = q.Code;
                string safeCode = MushProtocol.EscapeUnityRichText(code);
                string safePrice = MushProtocol.EscapeUnityRichText(q.PriceText);
                string expText = q.Expired ? "<color=#FF7766>EXPIRED</color>" : $"{q.ExpiresInSeconds}s left";
                slot.Background.color = new Color(0.09f, 0.14f, 0.18f, 0.94f);
                slot.Label.text = $"<color=#66DDFF>[QUOTE {safeCode}]</color>  <b>{safePrice}</b>  ({expText})";

                BindRowButton(slot.BtnA, slot.BtnALabel, "Re-Quote", () => SendMarketAction("quote", code: code), out slot.ActA);
                if (!q.Expired)
                {
                    BindRowButton(slot.BtnB, slot.BtnBLabel, "Pay Quote", () =>
                    {
                        InvokeWithTwoClickConfirm(
                            $"quote_pay_{code}",
                            slot.BtnBLabel,
                            "Pay Quote",
                            "CONFIRM PAY",
                            $"Confirm paying active quote [{code}] ({q.PriceText}): click CONFIRM PAY.",
                            () => SendMarketAction("pay", code: code));
                    }, out slot.ActB);
                }
                else
                {
                    BindRowButton(slot.BtnB, slot.BtnBLabel, "Expired", null, out slot.ActB, showDisabled: true);
                }
                BindRowButton(slot.BtnC, slot.BtnCLabel, string.Empty, null, out slot.ActC);
                slot.Root.SetActive(true);
            }

            // 3. Funded escrow purchases waiting for Complete or Refund (`U-H25`).
            for (int i = 0; i < MarketPage.WaitingCodes.Count; i++)
            {
                string wCode = MarketPage.WaitingCodes[i];
                var slot = _marketRows[rowIndex++];
                string safeCode = MushProtocol.EscapeUnityRichText(wCode);
                slot.Background.color = new Color(0.14f, 0.12f, 0.06f, 0.94f);
                slot.Label.text = $"<color=#FFCC66>[ESCROW WAITING {safeCode}]</color>  Funded purchase awaiting completion or refund";

                BindRowButton(slot.BtnA, slot.BtnALabel, "Complete", () => SendMarketAction("complete", code: wCode), out slot.ActA);
                BindRowButton(slot.BtnB, slot.BtnBLabel, "Refund", () =>
                {
                    InvokeWithTwoClickConfirm(
                        $"escrow_refund_{wCode}",
                        slot.BtnBLabel,
                        "Refund",
                        "CONFIRM REFUND",
                        $"Confirm refund of waiting escrow [{wCode}]: click CONFIRM REFUND.",
                        () => RequestMarketRefund(wCode));
                }, out slot.ActB);
                BindRowButton(slot.BtnC, slot.BtnCLabel, string.Empty, null, out slot.ActC);
                slot.Root.SetActive(true);
            }

            for (int i = rowIndex; i < _marketRows.Count; i++)
                _marketRows[i].Root.SetActive(false);

            if (_marketRowsContainer != null)
                LayoutRebuilder.ForceRebuildLayoutImmediate(_marketRowsContainer.GetComponent<RectTransform>());
        }

        private void RefreshHoldingsPane()
        {
            if (_holdingsSummaryText == null) return;
            if (HoldingsPage == null)
            {
                _holdingsSummaryText.text = "No such market page.";
                for (int i = 0; i < _holdingRows.Count; i++) _holdingRows[i].Root.SetActive(false);
                return;
            }

            _holdingsSummaryText.text = !string.IsNullOrEmpty(HoldingsPage.Notice)
                ? HoldingsPage.Notice
                : $"Section: {HoldingsPage.Section.ToUpperInvariant()} · {HoldingsPage.Rows.Count} holding(s)";

            EnsureHoldingRowPool(HoldingsPage.Rows.Count);
            for (int i = 0; i < HoldingsPage.Rows.Count; i++)
            {
                var r = HoldingsPage.Rows[i];
                var slot = _holdingRows[i];
                var capturedRow = r;
                string days = r.DaysLeft.HasValue ? $" ({r.DaysLeft.Value}d)" : string.Empty;
                string safeName = MushProtocol.EscapeUnityRichText(r.Name);
                string safeForm = MushProtocol.EscapeUnityRichText(r.Form);
                string safePhase = MushProtocol.EscapeUnityRichText(r.PhaseState);
                string idBadge = r.Kind == "owner"
                    ? (r.LicenceId.HasValue ? $"Filed #{r.FiledId} / Lic #{r.LicenceId.Value}" : $"Filed #{r.FiledId} (no owner lic)")
                    : $"Lic #{r.LicenceId ?? r.Number}";
                slot.Background.color = new Color(0.07f, 0.13f, 0.20f, 0.94f);
                slot.Label.text = $"<b>{idBadge} {safeName}</b> [{safeForm} · {safePhase}{days}]";

                // `U-C06`: ONLY bind Seal Ship when a real `LicenceId` is available and active; NEVER seal with `FiledId`!
                bool canSeal = r.LicenceId.HasValue && r.LicenceId.Value > 0 && !r.Revoked &&
                               !string.Equals(r.PhaseState, "lapsed", StringComparison.OrdinalIgnoreCase);
                if (canSeal)
                {
                    long licNum = r.LicenceId.Value;
                    BindRowButton(slot.BtnA, slot.BtnALabel, "Seal Ship", () =>
                    {
                        InvokeWithTwoClickConfirm(
                            $"seal_row_{licNum}",
                            slot.BtnALabel,
                            "Seal Ship",
                            "CONFIRM SEAL",
                            $"Confirm sealing License #{licNum} ({r.Name}) to ship berth (+ship/mind filed:{licNum} go): click CONFIRM SEAL.",
                            () => TrySealHoldingRow(capturedRow));
                    }, out slot.ActA);
                }
                else
                {
                    string reasonLabel = !r.LicenceId.HasValue ? "No Lic" : (r.Revoked ? "Revoked" : "Lapsed");
                    BindRowButton(slot.BtnA, slot.BtnALabel, reasonLabel, null, out slot.ActA, showDisabled: true);
                }

                // `48d H-43`: Strictly gate row actions on server-advertised `r.Actions`!
                if (r.Kind == "paid")
                {
                    long lic = r.LicenceId ?? r.Number;
                    bool canRenew = HasAction(r.Actions, "renew");
                    bool canRenewPay = HasAction(r.Actions, "renew_pay");
                    bool canRenewDecline = HasAction(r.Actions, "renew_decline");

                    if (canRenew)
                    {
                        BindRowButton(slot.BtnB, slot.BtnBLabel, "Renew", () => SendMarketAction("renew", licence: lic), out slot.ActB);
                    }
                    else if (canRenewPay)
                    {
                        BindRowButton(slot.BtnB, slot.BtnBLabel, "Pay Renew", () =>
                        {
                            InvokeWithTwoClickConfirm(
                                $"renew_pay_{lic}",
                                slot.BtnBLabel,
                                "Pay Renew",
                                "CONFIRM PAY",
                                $"Confirm paying renewal for License #{lic}: click CONFIRM PAY.",
                                () => SendMarketAction("renew_pay", licence: lic));
                        }, out slot.ActB);
                    }
                    else
                    {
                        BindRowButton(slot.BtnB, slot.BtnBLabel, string.Empty, null, out slot.ActB);
                    }

                    if (canRenewDecline)
                    {
                        BindRowButton(slot.BtnC, slot.BtnCLabel, "Decline", () =>
                        {
                            InvokeWithTwoClickConfirm(
                                $"renew_decline_{lic}",
                                slot.BtnCLabel,
                                "Decline",
                                "CONFIRM DECL",
                                $"Confirm declining renewal for License #{lic}: click CONFIRM DECL.",
                                () => SendMarketAction("renew_decline", licence: lic));
                        }, out slot.ActC);
                    }
                    else if (canRenewPay && canRenew)
                    {
                        BindRowButton(slot.BtnC, slot.BtnCLabel, "Pay Renew", () =>
                        {
                            InvokeWithTwoClickConfirm(
                                $"renew_pay_c_{lic}",
                                slot.BtnCLabel,
                                "Pay Renew",
                                "CONFIRM PAY",
                                $"Confirm paying renewal for License #{lic}: click CONFIRM PAY.",
                                () => SendMarketAction("renew_pay", licence: lic));
                        }, out slot.ActC);
                    }
                    else
                    {
                        BindRowButton(slot.BtnC, slot.BtnCLabel, string.Empty, null, out slot.ActC);
                    }
                }
                else
                {
                    long filedId = r.FiledId;
                    bool canPublish = HasAction(r.Actions, "publish");
                    bool canWithdraw = HasAction(r.Actions, "withdraw");

                    if (canPublish)
                    {
                        BindRowButton(slot.BtnB, slot.BtnBLabel, "Publish", () =>
                        {
                            InvokeWithTwoClickConfirm(
                                $"publish_{filedId}",
                                slot.BtnBLabel,
                                "Publish",
                                "CONFIRM PUB",
                                $"Confirm publishing Filed #{filedId} ({r.Name}) as sealed transferable listing: click CONFIRM PUB.",
                                () => SendMarketAction("publish", filed: filedId, form: "sealed"));
                        }, out slot.ActB);
                    }
                    else
                    {
                        BindRowButton(slot.BtnB, slot.BtnBLabel, string.Empty, null, out slot.ActB);
                    }

                    if (canWithdraw)
                    {
                        BindRowButton(slot.BtnC, slot.BtnCLabel, "Withdraw", () =>
                        {
                            InvokeWithTwoClickConfirm(
                                $"withdraw_{filedId}",
                                slot.BtnCLabel,
                                "Withdraw",
                                "CONFIRM WDRW",
                                $"Confirm withdrawing Filed #{filedId} ({r.Name}) from sale: click CONFIRM WDRW.",
                                () => SendMarketAction("withdraw", filed: filedId));
                        }, out slot.ActC);
                    }
                    else
                    {
                        BindRowButton(slot.BtnC, slot.BtnCLabel, string.Empty, null, out slot.ActC);
                    }
                }

                slot.Root.SetActive(true);
            }

            for (int i = HoldingsPage.Rows.Count; i < _holdingRows.Count; i++)
                _holdingRows[i].Root.SetActive(false);

            if (_holdingRowsContainer != null)
                LayoutRebuilder.ForceRebuildLayoutImmediate(_holdingRowsContainer.GetComponent<RectTransform>());
        }

        private static bool HasAction(List<string> actions, string name)
        {
            if (actions == null) return false;
            for (int i = 0; i < actions.Count; i++)
            {
                if (string.Equals(actions[i], name, StringComparison.OrdinalIgnoreCase))
                    return true;
            }
            return false;
        }

        private static string FormatDiagnosticSummary(MindDiagnosticView d)
        {
            if (d == null) return string.Empty;
            string loc = d.Line.HasValue
                ? (d.Column.HasValue ? $" @ {d.Line.Value}:{d.Column.Value}" : $" @ line {d.Line.Value}")
                : string.Empty;
            return $"[{d.Field}{loc}] {d.Message}";
        }

        private void RefreshWorkshopPane()
        {
            EnsureLibraryButtonPool(_libraryNames.Count);
            for (int i = 0; i < _libraryButtons.Count; i++)
            {
                bool has = i < _libraryNames.Count;
                _libraryButtons[i].gameObject.SetActive(has);
                if (has)
                {
                    var txt = _libraryButtons[i].GetComponentInChildren<Text>();
                    if (txt != null) txt.text = _libraryNames[i];
                }
            }

            if (_rhaiSourceInput != null)
                _rhaiSourceInput.interactable = LoadedDraft != null && LoadedDraft.IsObject;

            if (_workshopMetaText != null)
            {
                var sb = new StringBuilder(160);
                sb.Append(string.IsNullOrEmpty(LoadedMindName)
                    ? "Working Copy: (none)"
                    : $"Working Copy: {LoadedMindName}{(IsDraftDirty ? " [UNSAVED]" : "")}");
                if (_lastDiagnostics.Count > 0)
                    sb.Append($"  |  Diagnostics: {_lastDiagnostics.Count}");
                if (_lastReports.Count > 0)
                    sb.Append($"  |  Pictures Checked: {_lastReports.Count}");
                _workshopMetaText.text = sb.ToString();
            }

            if (_workshopOutputText != null && (_lastDiagnostics.Count > 0 || _lastReports.Count > 0))
            {
                var sb = new StringBuilder(512);
                if (_lastDiagnostics.Count > 0)
                {
                    sb.AppendLine($"DIAGNOSTICS ({_lastDiagnostics.Count}):");
                    for (int i = 0; i < _lastDiagnostics.Count; i++)
                        sb.AppendLine("  " + FormatDiagnosticSummary(_lastDiagnostics[i]));
                }
                if (_lastReports.Count > 0)
                {
                    if (sb.Length > 0) sb.AppendLine();
                    sb.AppendLine($"ISOLATED PICTURE REPORTS ({_lastReports.Count}):");
                    for (int i = 0; i < _lastReports.Count; i++)
                    {
                        var rep = _lastReports[i];
                        string arm = rep.Armed ? "armed" : "unarmed";
                        string faultStr = !string.IsNullOrEmpty(rep.Fault) ? $" · FAULT: {rep.Fault}" : string.Empty;
                        sb.AppendLine($"  • {rep.Picture} ({arm}) — {rep.Operations} ops, {rep.Fees} fees{faultStr}");
                        if (rep.Intents.Count > 0)
                            sb.AppendLine($"      Intents: {string.Join("; ", rep.Intents)}");
                        if (rep.StationRefusals.Count > 0)
                            sb.AppendLine($"      Refusals: {string.Join("; ", rep.StationRefusals)}");
                        if (rep.ClassViolations.Count > 0)
                            sb.AppendLine($"      Class violations: {string.Join("; ", rep.ClassViolations)}");
                    }
                }
                _workshopOutputText.text = sb.ToString().TrimEnd();
            }
        }

        private void RefreshTracePane()
        {
            if (_stopFollowBtn != null)
                _stopFollowBtn.interactable = _followingTraceUnitId > 0;
            if (_traceOutputText == null) return;
            if (TracePage == null || !TracePage.Available || TracePage.Lines.Count == 0)
            {
                _traceOutputText.text = "No live mind trace available for that unit.";
                return;
            }
            _traceOutputText.text = string.Join("\n", TracePage.Lines);
        }

        private void SetStatus(string msg)
        {
            StatusBanner = msg ?? string.Empty;
            if (_statusBannerText != null)
                _statusBannerText.text = StatusBanner;
        }

        private PooledActionRow CreatePooledActionRow(Transform parent, string name, float rowHeight)
        {
            var root = CreatePanel(parent, name, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.07f, 0.13f, 0.20f, 0.94f));
            root.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, rowHeight);

            var row = new PooledActionRow
            {
                Root = root,
                Background = root.GetComponent<Image>()
            };
            row.Label = CreateTxt(root.transform, string.Empty, 12, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
            row.Label.supportRichText = true;
            SetRect(row.Label.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 2f), new Vector2(-296f, -2f));

            row.BtnA = CreateBtn(root.transform, "A", new Color(0.16f, 0.44f, 0.36f), () => row.ActA?.Invoke());
            SetRect(row.BtnA.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-288f, 4f), new Vector2(-198f, -4f));
            row.BtnALabel = row.BtnA.GetComponentInChildren<Text>();

            row.BtnB = CreateBtn(root.transform, "B", new Color(0.18f, 0.36f, 0.52f), () => row.ActB?.Invoke());
            SetRect(row.BtnB.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-194f, 4f), new Vector2(-104f, -4f));
            row.BtnBLabel = row.BtnB.GetComponentInChildren<Text>();

            row.BtnC = CreateBtn(root.transform, "C", new Color(0.46f, 0.30f, 0.14f), () => row.ActC?.Invoke());
            SetRect(row.BtnC.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-100f, 4f), new Vector2(-8f, -4f));
            row.BtnCLabel = row.BtnC.GetComponentInChildren<Text>();

            root.SetActive(false);
            return row;
        }

        private static void BindRowButton(Button btn, Text lbl, string text, Action callback, out Action slotAction, bool showDisabled = false)
        {
            slotAction = callback;
            if (lbl != null) lbl.text = text;
            if (btn != null)
            {
                bool visible = callback != null || showDisabled;
                btn.gameObject.SetActive(visible);
                btn.interactable = callback != null;
            }
        }

        private Transform CreateRow(Transform parent, string name, float yMin, float yMax)
        {
            var go = CreatePanel(parent, name, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, yMin), new Vector2(0f, yMax), Color.clear);
            var hlg = go.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 6f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;
            return go.transform;
        }

        private InputField CreateMiniInput(Transform parent, string placeholder, string defaultVal)
        {
            var go = CreatePanel(parent, "Input_" + placeholder, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.08f, 0.13f, 0.20f, 0.95f));
            var le = go.AddComponent<LayoutElement>();
            le.preferredWidth = 96f;
            le.flexibleWidth = 0.45f;

            var input = go.AddComponent<InputField>();
            var ph = CreateTxt(go.transform, placeholder, 12, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.5f, 0.62f, 0.72f));
            SetRect(ph.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            var val = CreateTxt(go.transform, defaultVal, 12, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(val.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            input.placeholder = ph;
            input.textComponent = val;
            input.text = defaultVal;
            return input;
        }

        private static GameObject CreatePanel(Transform parent, string name, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), anchorMin, anchorMax, offsetMin, offsetMax);
            return go;
        }

        private Text CreateTxt(Transform parent, string content, int size, FontStyle style, TextAnchor align, Color color)
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

        private Button CreateBtn(Transform parent, string label, Color bg, Action onClick)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;

            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.onClick.AddListener(() => onClick?.Invoke());

            var txt = CreateTxt(go.transform, label, 12, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
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
