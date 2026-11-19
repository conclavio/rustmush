using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;
using UnityEngine.EventSystems;
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
        public int Number;           // filed # (owner) or licence # (paid)
        public string Name = string.Empty;
        public string Form = string.Empty;
        public string PhaseState = "active";
        public int? DaysLeft;
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
        /// <summary>Rows pooled up front: one full server page (`LISTINGS_PER_PAGE` = 20). A longer page grows the pool.</summary>
        private const int MaxPooledRows = 20;

        /// <summary>Every live terminal, so the HUD can ask whether any of them holds keyboard focus.</summary>
        private static readonly List<TacSimMindTerminal> LiveTerminals = new List<TacSimMindTerminal>();

        /// <summary>True when an input field of any live mind terminal is focused (gates the avatar's walk keys).</summary>
        public static bool IsAnyInputFocused
        {
            get
            {
                for (int i = 0; i < LiveTerminals.Count; i++)
                {
                    var t = LiveTerminals[i];
                    if (t != null && t.HasFocusedInput) return true;
                }
                return false;
            }
        }

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

        /// <summary>True when one of this terminal's own input fields is focused.</summary>
        public bool HasFocusedInput =>
            (_sealLicenseInput != null && _sealLicenseInput.isFocused) ||
            (_newMindNameInput != null && _newMindNameInput.isFocused) ||
            (_rhaiSourceInput != null && _rhaiSourceInput.isFocused) ||
            (_saveNotesInput != null && _saveNotesInput.isFocused) ||
            (_traceUnitInput != null && _traceUnitInput.isFocused);

        public MindTerminalTab ActiveTab { get; private set; } = MindTerminalTab.Market;

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

        private MushWsClient _ws;
        private Font _font;
        private Canvas _canvas;
        private GameObject _launcherPill;
        private GameObject _windowRoot;

        private readonly List<Image> _tabBtnImages = new List<Image>();
        private GameObject _marketPane;
        private GameObject _holdingsPane;
        private GameObject _workshopPane;
        private GameObject _tracePane;

        private Text _statusBannerText;
        private Text _marketSummaryText;
        private Text _holdingsSummaryText;
        private Text _workshopMetaText;
        private Text _traceOutputText;

        private InputField _sealLicenseInput;
        private InputField _newMindNameInput;
        private InputField _rhaiSourceInput;
        private InputField _saveNotesInput;
        private InputField _traceUnitInput;

        private bool _suppressSourceDirtyCallback;
        private bool _pendingCheckAfterReplace;
        private bool _pendingSaveAfterReplace;
        private string _pendingSaveNotes = string.Empty;
        private readonly List<string> _libraryNames = new List<string>();
        private readonly List<Button> _libraryButtons = new List<Button>();

        private sealed class PooledActionRow
        {
            public GameObject Root;
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

        private readonly List<PooledActionRow> _marketRows = new List<PooledActionRow>(MaxPooledRows);
        private readonly List<PooledActionRow> _holdingRows = new List<PooledActionRow>(MaxPooledRows);
        private Transform _marketRowsContent;
        private Transform _holdingRowsContent;

        private void Awake()
        {
            MushWsClient.OnInstanceCreated += HandleClientInstanceCreated;
            if (!LiveTerminals.Contains(this))
                LiveTerminals.Add(this);
        }

        private void OnDestroy()
        {
            LiveTerminals.Remove(this);
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
            }
        }

        private void UnbindClient()
        {
            if (_ws == null) return;
            _ws.OnPanelUpdated -= HandlePanelUpdated;
            _ws.OnInvokeResultReceived -= HandleInvokeResult;
            _ws.OnSystemLineReceived -= HandleSystemLine;
            _ws = null;
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
                        string wCode = wArr[i].AsString();
                        if (!string.IsNullOrEmpty(wCode))
                            page.WaitingCodes.Add(wCode);
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
                    int num = kind == "owner" ? r.Int("filed", 0) : r.Int("licence", 0);
                    if (num <= 0) continue;

                    var phaseObj = r["phase"];
                    string phaseState = phaseObj.IsObject ? phaseObj.Str("state", "active") : "active";
                    int? daysLeft = (phaseObj.IsObject && phaseObj.ContainsKey("days_left") && !phaseObj["days_left"].IsNull)
                        ? (int?)phaseObj.Int("days_left", 0)
                        : null;

                    var row = new MindHoldingRowView
                    {
                        Kind = kind,
                        Number = num,
                        Name = r.Str("name", $"#{num}"),
                        Form = r.Str("form", kind == "owner" ? "original" : "sealed"),
                        PhaseState = phaseState,
                        DaysLeft = daysLeft,
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
                for (int i = 0; i < Math.Min(traces.Count, 64) && view.Lines.Count < 256; i++)
                {
                    var tr = traces[i];
                    if (!tr.IsObject) continue;
                    int seq = tr.Int("think_seq", i + 1);
                    int tMs = tr.Int("t_ms", 0);
                    string src = tr.Bool("rom", false) ? "ROM" : "mind";
                    string outWord = FormatTraceOutcome(tr["outcome"]);
                    view.Lines.Add($"Think {seq} · {tMs} ms · {src} · {outWord}");

                    var diag = tr["diagnostics"];
                    if (diag.IsObject)
                    {
                        string ops = diag.ContainsKey("ops") && !diag["ops"].IsNull ? diag.Int("ops", 0).ToString() : "unmeasured";
                        string fees = diag.ContainsKey("fees") && !diag["fees"].IsNull ? diag.Int("fees", 0).ToString() : "unmeasured";
                        view.Lines.Add($"  Operations {ops}; host fees {fees}");
                    }

                    var authors = tr["author"].AsArray();
                    if (authors != null)
                    {
                        for (int a = 0; a < Math.Min(authors.Count, 8); a++)
                        {
                            string al = authors[a].AsString();
                            if (!string.IsNullOrEmpty(al))
                                view.Lines.Add($"  Author: {al}");
                        }
                    }

                    var intents = tr["intents"].AsArray();
                    if (intents != null)
                    {
                        for (int k = 0; k < Math.Min(intents.Count, 16); k++)
                        {
                            var it = intents[k];
                            if (!it.IsObject) continue;
                            string act = it.Str("action", string.Empty);
                            string det = it.Str("detail", string.Empty);
                            string itOut = FormatTraceOutcome(it["outcome"]);
                            view.Lines.Add($"  {act}{(string.IsNullOrEmpty(det) ? "" : " " + det)}: {itOut}");
                        }
                    }
                }
            }

            return view;
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
            switch (status)
            {
                case "queued":
                case "pending":
                    SetStatus($"Job #{res.Int("job", 0)} {status}...");
                    break;

                case "error":
                    _pendingCheckAfterReplace = false;
                    _pendingSaveAfterReplace = false;
                    SetStatus($"Refused ({res.Str("code", "error")}): {res.Str("message", "Request failed.")}");
                    break;

                case "library":
                {
                    _libraryNames.Clear();
                    var arr = res["names"].AsArray();
                    if (arr != null)
                    {
                        for (int i = 0; i < Math.Min(arr.Count, 20); i++)
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
                    LoadedMindName = res.Str("name", string.Empty);
                    LoadedDraftBase = res["base"] != null ? res["base"].Clone() : null;
                    LoadedDraft = res["draft"] != null ? res["draft"].Clone() : null;
                    IsDraftDirty = false;
                    if (_rhaiSourceInput != null && LoadedDraft != null && LoadedDraft.IsObject)
                    {
                        _suppressSourceDirtyCallback = true;
                        _rhaiSourceInput.text = LoadedDraft.Str("source", string.Empty);
                        _suppressSourceDirtyCallback = false;
                    }
                    SetStatus($"Opened working copy '{LoadedMindName}'.");
                    RefreshWorkshopPane();
                    break;
                }

                case "edited":
                {
                    string editedName = res.Str("name", LoadedMindName);
                    LoadedMindName = editedName;
                    if (res["base"] != null && !res["base"].IsNull)
                        LoadedDraftBase = res["base"].Clone();
                    IsDraftDirty = false;

                    if (_pendingCheckAfterReplace)
                    {
                        _pendingCheckAfterReplace = false;
                        SendWorkshopCheckInternal();
                    }
                    else if (_pendingSaveAfterReplace)
                    {
                        _pendingSaveAfterReplace = false;
                        SendWorkshopSaveInternal(_pendingSaveNotes);
                    }
                    else
                    {
                        SetStatus($"Updated '{editedName}'.");
                        RequestReadMind(editedName);
                        RequestLibraryList();
                    }
                    break;
                }

                case "checked":
                {
                    int diagCount = res["diagnostics"].AsArray()?.Count ?? 0;
                    int repCount = res["reports"].AsArray()?.Count ?? 0;
                    SetStatus($"Check complete: {repCount} isolated picture report(s), {diagCount} diagnostic(s).");
                    RefreshWorkshopPane(res["diagnostics"].AsArray(), res["reports"].AsArray());
                    break;
                }

                case "saved":
                {
                    int rev = res.Int("revision", 1);
                    string dig = res.Str("digest", string.Empty);
                    if (dig.Length > 12) dig = dig.Substring(0, 12);
                    if (res["base"] != null && !res["base"].IsNull)
                        LoadedDraftBase = res["base"].Clone();
                    SetStatus($"Saved revision r{rev} ({dig}). Existing hull seals keep their earlier revision.");
                    RefreshWorkshopPane();
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

        public void RequestMarketListings(string afterCode = null)
        {
            _ws?.SendMindMarketListings(afterCode);
        }

        public void RequestHoldings(string section = "paid", long? afterNum = null)
        {
            HoldingsSection = string.Equals(section, "owner", StringComparison.OrdinalIgnoreCase) ? "owner" : "paid";
            _ws?.SendMindMarketHoldings(HoldingsSection, afterNum);
        }

        public void SendMarketAction(string act, string code = null, int? licence = null, int? filed = null, string form = null)
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
            _ws.SendMindInvoke(req);
            SetStatus($"Sent market action '{act}'...");
        }

        public void SealMindToShipBerth(int licenseNumber)
        {
            if (licenseNumber <= 0) return;
            string cmd = $"+ship/mind filed:{licenseNumber} go";
            _ws?.SendLine(cmd);
            SetStatus($"> {cmd}");
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
            _ws.SendMindInvoke(req);
        }

        public void RequestReadMind(string name)
        {
            if (_ws == null || string.IsNullOrWhiteSpace(name)) return;
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("read");
            req["name"] = JsonNode.FromString(name.Trim());
            req["revision"] = JsonNode.Null;
            _ws.SendMindInvoke(req);
        }

        public void RequestCreateMindFromStandard(string newName)
        {
            if (_ws == null || string.IsNullOrWhiteSpace(newName)) return;
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("create");
            req["name"] = JsonNode.FromString(newName.Trim());
            req["from"] = JsonNode.FromString("Standard");
            _ws.SendMindInvoke(req);
        }

        public void UpdateWorkingSource(string newSource)
        {
            if (LoadedDraft == null || !LoadedDraft.IsObject) return;
            if (newSource != null && (Encoding.UTF8.GetByteCount(newSource) > 32 * 1024 || newSource.IndexOf('\0') >= 0))
            {
                SetStatus("Source must fit 32 KiB and contain no NUL characters.");
                return;
            }
            LoadedDraft["source"] = JsonNode.FromString(newSource ?? string.Empty);
            IsDraftDirty = true;
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
                _pendingCheckAfterReplace = true;
                _pendingSaveAfterReplace = false;
                SendReplaceWorkingCopy();
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
            if (Encoding.UTF8.GetByteCount(cleanNotes) > 2048)
            {
                SetStatus("Revision notes must fit 2 KiB.");
                return;
            }
            if (IsDraftDirty)
            {
                _pendingSaveAfterReplace = true;
                _pendingCheckAfterReplace = false;
                _pendingSaveNotes = cleanNotes;
                SendReplaceWorkingCopy();
                return;
            }
            SendWorkshopSaveInternal(cleanNotes);
        }

        private void SendReplaceWorkingCopy()
        {
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("replace");
            req["name"] = JsonNode.FromString(LoadedMindName);
            req["base"] = LoadedDraftBase.Clone();
            req["draft"] = LoadedDraft.Clone();
            _ws.SendMindInvoke(req);
        }

        private void SendWorkshopCheckInternal()
        {
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("check");
            req["name"] = JsonNode.FromString(LoadedMindName);
            req["base"] = LoadedDraftBase.Clone();
            _ws.SendMindInvoke(req);
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
            _ws.SendMindInvoke(req);
            SetStatus($"Saving immutable revision of '{LoadedMindName}'...");
        }

        public void RequestUnitMindTrace(int unitId, bool follow)
        {
            if (_ws == null || unitId <= 0) return;
            var req = JsonNode.NewObject();
            req["op"] = JsonNode.FromString("trace");
            req["unit"] = JsonNode.FromLong(unitId);
            req["count"] = JsonNode.FromLong(8);
            req["follow"] = JsonNode.FromBool(follow);
            _ws.SendMindInvoke(req);
            SetStatus($"Requesting mind trace for unit #{unitId}...");
        }

        // ---- uGUI Construction & Tab Switching ----

        public void BuildTerminalUi()
        {
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
            _marketRowsContent = null;
            _holdingRowsContent = null;
            _libraryButtons.Clear();

            var canvasGo = new GameObject("MindTerminalCanvas");
            canvasGo.transform.SetParent(transform, false);
            _canvas = canvasGo.AddComponent<Canvas>();
            _canvas.renderMode = RenderMode.ScreenSpaceOverlay;
            _canvas.sortingOrder = 26;

            var scaler = canvasGo.AddComponent<CanvasScaler>();
            scaler.uiScaleMode = CanvasScaler.ScaleMode.ScaleWithScreenSize;
            scaler.referenceResolution = new Vector2(1920f, 1080f);
            scaler.matchWidthOrHeight = 0.5f;
            canvasGo.AddComponent<GraphicRaycaster>();

            // Top-right `[F10] MINDS & SEALING` launcher pill
            _launcherPill = CreatePanel(canvasGo.transform, "MindLauncherPill", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-216f, -102f), new Vector2(-16f, -68f), new Color(0.08f, 0.20f, 0.32f, 0.92f));
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
            _windowRoot.SetActive(open);
            if (open)
                SwitchTab(ActiveTab, autoRefresh: true);
        }

        public void SwitchTab(MindTerminalTab tab, bool autoRefresh = true)
        {
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
            if (_launcherPill != null)
            {
                bool connected = _ws != null && _ws.State == MushWsConnectionState.Online;
                if (_launcherPill.activeSelf != connected)
                    _launcherPill.SetActive(connected);
            }

            if (Input.GetKeyDown(KeyCode.F10))
                SetTerminalOpen(!IsTerminalOpen);
            else if (IsTerminalOpen && Input.GetKeyDown(KeyCode.Escape))
                SetTerminalOpen(false);
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
            CreateBtn(topBar, "Class III Reg (+enduser/apply go)", new Color(0.26f, 0.34f, 0.48f), () =>
            {
                _ws?.SendLine("+enduser/apply go");
                SetStatus("> +enduser/apply go");
            });

            _marketSummaryText = CreateTxt(_marketPane.transform, "Querying mind.market listings...", 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.82f, 0.92f, 1f));
            SetRect(_marketSummaryText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(4f, -64f), new Vector2(-4f, -40f));

            _marketRowsContent = CreateScrollingRowList(_marketPane.transform, "MarketRowsArea", -68f);
            for (int i = 0; i < MaxPooledRows; i++)
                _marketRows.Add(CreatePooledActionRow(_marketRowsContent, $"MktRow_{i}"));
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
            CreateBtn(sealBar, "Seal To Berth", new Color(0.16f, 0.50f, 0.36f), () =>
            {
                if (int.TryParse(_sealLicenseInput != null ? _sealLicenseInput.text.Trim() : "1", out int n) && n > 0)
                    SealMindToShipBerth(n);
            });
            CreateBtn(sealBar, "Unseal (+ship/mind off)", new Color(0.48f, 0.22f, 0.20f), UnsealShipMind);
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

            _holdingRowsContent = CreateScrollingRowList(_holdingsPane.transform, "HoldingsRowsArea", -108f);
            for (int i = 0; i < MaxPooledRows; i++)
                _holdingRows.Add(CreatePooledActionRow(_holdingRowsContent, $"HoldRow_{i}"));
        }

        private void BuildWorkshopPane(Transform parent)
        {
            _workshopPane = CreatePanel(parent, "WorkshopPane", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 40f), new Vector2(-16f, -92f), Color.clear);

            var libRow = CreateRow(_workshopPane.transform, "LibraryRow", -36f, 0f);
            CreateBtn(libRow, "Refresh Library", new Color(0.16f, 0.42f, 0.54f), RequestLibraryList);
            _newMindNameInput = CreateMiniInput(libRow, "New name...", "escort_v1");
            CreateBtn(libRow, "New From Standard", new Color(0.16f, 0.48f, 0.36f), () =>
            {
                string nm = _newMindNameInput != null ? _newMindNameInput.text : "escort_v1";
                RequestCreateMindFromStandard(nm);
            });
            _saveNotesInput = CreateMiniInput(libRow, "Revision notes...", "Tuned doctrine");
            CreateBtn(libRow, "Check 6 Pictures", new Color(0.20f, 0.38f, 0.54f), CheckWorkingMind);
            CreateBtn(libRow, "Save Revision", new Color(0.48f, 0.32f, 0.14f), () =>
                SaveWorkingMindRevision(_saveNotesInput != null ? _saveNotesInput.text : string.Empty));

            var namesRow = CreateRow(_workshopPane.transform, "OwnedNamesRow", -72f, -40f);
            for (int i = 0; i < 6; i++)
            {
                int slot = i;
                var b = CreateBtn(namesRow, "-", new Color(0.12f, 0.24f, 0.36f), () =>
                {
                    if (slot < _libraryNames.Count)
                        RequestReadMind(_libraryNames[slot]);
                });
                b.gameObject.SetActive(false);
                _libraryButtons.Add(b);
            }

            _workshopMetaText = CreateTxt(_workshopPane.transform, "Working Copy: (none loaded)", 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.84f, 0.94f, 1f));
            SetRect(_workshopMetaText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(4f, -98f), new Vector2(-4f, -76f));

            var editorBox = CreatePanel(_workshopPane.transform, "RhaiEditorBox", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(0f, 0f), new Vector2(0f, -104f), new Color(0.04f, 0.08f, 0.14f, 0.96f));
            _rhaiSourceInput = editorBox.AddComponent<InputField>();
            _rhaiSourceInput.lineType = InputField.LineType.MultiLineNewline;
            var srcPh = CreateTxt(editorBox.transform, "// Open or create a working mind above to inspect & edit its Rhai source...", 13, FontStyle.Italic, TextAnchor.UpperLeft, new Color(0.45f, 0.58f, 0.70f));
            SetRect(srcPh.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));
            var srcTxt = CreateTxt(editorBox.transform, string.Empty, 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.90f, 0.97f, 1f));
            srcTxt.horizontalOverflow = HorizontalWrapMode.Wrap;
            srcTxt.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(srcTxt.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 10f), new Vector2(-12f, -10f));
            _rhaiSourceInput.placeholder = srcPh;
            _rhaiSourceInput.textComponent = srcTxt;
            _rhaiSourceInput.onValueChanged.AddListener(val =>
            {
                if (!_suppressSourceDirtyCallback)
                    UpdateWorkingSource(val);
            });
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

            var vp = CreatePanel(_tracePane.transform, "TraceViewport", new Vector2(0f, 0f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, -44f), new Color(0.02f, 0.05f, 0.09f, 0.90f));
            vp.AddComponent<RectMask2D>();
            var sr = vp.AddComponent<ScrollRect>();
            _traceOutputText = CreateTxt(vp.transform, "Enter a unit or ship # above to inspect source-free Think diagnostics (`arena.mind`).", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 1f));
            _traceOutputText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _traceOutputText.verticalOverflow = VerticalWrapMode.Overflow;
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

            EnsurePooledRows(_marketRows, _marketRowsContent, MarketPage.Rows.Count, "MktRow_");
            int shown = Math.Min(MarketPage.Rows.Count, _marketRows.Count);
            for (int i = 0; i < shown; i++)
            {
                var r = MarketPage.Rows[i];
                var slot = _marketRows[i];
                string code = r.Code;
                slot.Label.text = $"<b>[{code}] {MushProtocol.EscapeUnityRichText(r.Name)}</b> · Class {r.ClassWord} ({r.Form}) · <b>{MushProtocol.EscapeUnityRichText(r.PriceText)}</b>";
                BindRowButton(slot.BtnA, slot.BtnALabel, "Quote", () => SendMarketAction("quote", code: code), out slot.ActA);
                BindRowButton(slot.BtnB, slot.BtnBLabel, "Pay", () => SendMarketAction("pay", code: code), out slot.ActB);
                BindRowButton(slot.BtnC, slot.BtnCLabel, "Complete", () => SendMarketAction("complete", code: code), out slot.ActC);
                slot.Root.SetActive(true);
            }
            for (int i = shown; i < _marketRows.Count; i++)
                _marketRows[i].Root.SetActive(false);
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

            EnsurePooledRows(_holdingRows, _holdingRowsContent, HoldingsPage.Rows.Count, "HoldRow_");
            int shown = Math.Min(HoldingsPage.Rows.Count, _holdingRows.Count);
            for (int i = 0; i < shown; i++)
            {
                var r = HoldingsPage.Rows[i];
                var slot = _holdingRows[i];
                int num = r.Number;
                string days = r.DaysLeft.HasValue ? $" ({r.DaysLeft.Value}d)" : string.Empty;
                slot.Label.text = $"<b>#{num} {MushProtocol.EscapeUnityRichText(r.Name)}</b> [{r.Form} · {r.PhaseState}{days}]";

                BindRowButton(slot.BtnA, slot.BtnALabel, "Seal Ship", () =>
                {
                    if (_sealLicenseInput != null) _sealLicenseInput.text = num.ToString();
                    SealMindToShipBerth(num);
                }, out slot.ActA);

                // Server actions: a button shows only when its word is in the row's `actions`.
                if (r.Kind == "paid")
                {
                    BindRowButton(slot.BtnB, slot.BtnBLabel, "Renew", IfRowOffers(r, "renew", () => SendMarketAction("renew", licence: num)), out slot.ActB);
                    BindRowButton(slot.BtnC, slot.BtnCLabel, "Pay Renew", IfRowOffers(r, "renew_pay", () => SendMarketAction("renew_pay", licence: num)), out slot.ActC);
                }
                else
                {
                    BindRowButton(slot.BtnB, slot.BtnBLabel, "Publish", IfRowOffers(r, "publish", () => SendMarketAction("publish", filed: num, form: "sealed")), out slot.ActB);
                    BindRowButton(slot.BtnC, slot.BtnCLabel, "Withdraw", IfRowOffers(r, "withdraw", () => SendMarketAction("withdraw", filed: num)), out slot.ActC);
                }

                slot.Root.SetActive(true);
            }
            for (int i = shown; i < _holdingRows.Count; i++)
                _holdingRows[i].Root.SetActive(false);
        }

        private void RefreshWorkshopPane(List<JsonNode> diags = null, List<JsonNode> reports = null)
        {
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

            if (_workshopMetaText == null) return;
            var sb = new StringBuilder(160);
            sb.Append(string.IsNullOrEmpty(LoadedMindName)
                ? "Working Copy: (none)"
                : $"Working Copy: {LoadedMindName}{(IsDraftDirty ? " [UNSAVED]" : "")}");
            if (diags != null && diags.Count > 0)
                sb.Append($"  |  Diagnostics: {diags.Count}");
            if (reports != null && reports.Count > 0)
                sb.Append($"  |  Pictures Checked: {reports.Count}");
            _workshopMetaText.text = sb.ToString();
        }

        private void RefreshTracePane()
        {
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

        private PooledActionRow CreatePooledActionRow(Transform parent, string name)
        {
            var root = CreatePanel(parent, name, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.07f, 0.13f, 0.20f, 0.94f));
            root.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 36f);

            var row = new PooledActionRow { Root = root };
            row.Label = CreateTxt(root.transform, string.Empty, 12, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
            row.Label.supportRichText = true;
            SetRect(row.Label.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 2f), new Vector2(-290f, -2f));

            row.BtnA = CreateBtn(root.transform, "A", new Color(0.16f, 0.44f, 0.36f), () => row.ActA?.Invoke());
            SetRect(row.BtnA.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-282f, 4f), new Vector2(-194f, -4f));
            row.BtnALabel = row.BtnA.GetComponentInChildren<Text>();

            row.BtnB = CreateBtn(root.transform, "B", new Color(0.18f, 0.36f, 0.52f), () => row.ActB?.Invoke());
            SetRect(row.BtnB.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-190f, 4f), new Vector2(-102f, -4f));
            row.BtnBLabel = row.BtnB.GetComponentInChildren<Text>();

            row.BtnC = CreateBtn(root.transform, "C", new Color(0.46f, 0.30f, 0.14f), () => row.ActC?.Invoke());
            SetRect(row.BtnC.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-98f, 4f), new Vector2(-8f, -4f));
            row.BtnCLabel = row.BtnC.GetComponentInChildren<Text>();

            root.SetActive(false);
            return row;
        }

        /// <summary>The callback when the row's server `actions` list carries `word`, else null (the button hides).</summary>
        public static Action IfRowOffers(MindHoldingRowView row, string word, Action callback)
        {
            if (row == null || callback == null) return null;
            return row.Actions.Contains(word) ? callback : null;
        }

        /// <summary>A masked, vertically scrolling list below the pane's header; returns the content the rows go in.</summary>
        private Transform CreateScrollingRowList(Transform parent, string name, float topOffset)
        {
            var viewport = CreatePanel(parent, name, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(0f, 0f), new Vector2(0f, topOffset), new Color(0.02f, 0.05f, 0.09f, 0.88f));
            viewport.AddComponent<RectMask2D>();
            var sr = viewport.AddComponent<ScrollRect>();

            var content = new GameObject(name + "Content", typeof(RectTransform));
            content.transform.SetParent(viewport.transform, false);
            var contentRt = content.GetComponent<RectTransform>();
            contentRt.anchorMin = new Vector2(0f, 1f);
            contentRt.anchorMax = new Vector2(1f, 1f);
            contentRt.pivot = new Vector2(0.5f, 1f);
            contentRt.offsetMin = Vector2.zero;
            contentRt.offsetMax = Vector2.zero;

            var vlg = content.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f; vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true; vlg.childControlHeight = false; vlg.childForceExpandWidth = true; vlg.childForceExpandHeight = false;
            var csf = content.AddComponent<ContentSizeFitter>();
            csf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            csf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;

            sr.viewport = viewport.GetComponent<RectTransform>();
            sr.content = contentRt;
            sr.horizontal = false;
            sr.vertical = true;
            sr.movementType = ScrollRect.MovementType.Clamped;
            return content.transform;
        }

        /// <summary>Grows a row pool so every row of a server page has a slot.</summary>
        private void EnsurePooledRows(List<PooledActionRow> pool, Transform content, int needed, string namePrefix)
        {
            if (content == null) return;
            while (pool.Count < needed)
                pool.Add(CreatePooledActionRow(content, $"{namePrefix}{pool.Count}"));
        }

        private static void BindRowButton(Button btn, Text lbl, string text, Action callback, out Action slotAction)
        {
            slotAction = callback;
            if (lbl != null) lbl.text = text;
            if (btn != null) btn.gameObject.SetActive(callback != null);
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
