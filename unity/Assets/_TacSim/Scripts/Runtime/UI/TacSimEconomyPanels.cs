using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum TradeTerminalTab
    {
        Exchange = 0,
        History = 1,
        Jobs = 2,
        Industry = 3,
        Sailing = 4
    }

    /// <summary>
    /// One `+market/history` reply, kept apart from the shared 24-line echo log: a heading plus up to
    /// 24 week rows is 25 lines, so the shared log drops the heading. A heading starts (and resets) the
    /// capture, the week rows after it are appended, and any other line ends it.
    /// </summary>
    public sealed class MarketHistoryReply
    {
        /// <summary>A guard far above the server's 24-week window.</summary>
        public const int MaxLines = 64;

        private readonly List<string> _lines = new List<string>();
        private bool _capturing;

        /// <summary>The last reply: its heading, then its rows, newest first; empty before any reply.</summary>
        public IReadOnlyList<string> Lines => _lines;

        public bool IsCapturing => _capturing;

        /// <summary>Feeds one server line; true when the line belongs to a history reply.</summary>
        public bool Ingest(string rawLine)
        {
            if (string.IsNullOrWhiteSpace(rawLine)) return false;
            string plain = TacSimEconomyPanels.StripRichTags(rawLine);
            if (TacSimEconomyPanels.RxHistoryHeader.IsMatch(plain))
            {
                _lines.Clear();
                _lines.Add(rawLine);
                _capturing = true;
                return true;
            }
            if (!_capturing) return false;
            if (TacSimEconomyPanels.RxHistoryRow.IsMatch(plain))
            {
                if (_lines.Count < MaxLines)
                    _lines.Add(rawLine);
                return true;
            }
            _capturing = false;
            return false;
        }

        public void Clear()
        {
            _lines.Clear();
            _capturing = false;
        }
    }

    public sealed class ParsedPriceHistoryRow
    {
        public string When = string.Empty;
        public string AskText = string.Empty;
        public string BidText = string.Empty;
        public double? AskMajor;
        public double? BidMajor;
    }

    public sealed class ParsedPriceHistorySummary
    {
        public string GoodLabel;
        public readonly List<ParsedPriceHistoryRow> RowsNewestFirst = new List<ParsedPriceHistoryRow>();
        public string AskSparkline = "—";
        public string BidSparkline = "—";
        public string LatestAsk = "—";
        public string LatestBid = "—";
        public string AskRange = "—";
        public string BidRange = "—";
    }

    /// <summary>
    /// Doc 48 (`EX.1`–`EX.8`, `PL.1`–`PL.9`) diegetic uGUI sub-panels inside the Port Terminal:
    /// 1. 24-Week Price History Sparklines, Public Depth, Lockers &amp; Debts (`+market/history`, `+locker`, `+debts`)
    /// 2. Open Jobs, Haulage, Escort, Survey, Retrieval &amp; Crew Terms (`+market/jobs`, `+haul`, `+escort`, `+sortie/crew`)
    /// 3. Port Works, Shipyard Commissioning, Bay Rearm/Repair &amp; Core Shop (`+works`, `+yard`, `+bay`, `+core`, `@facility/list`)
    /// </summary>
    public sealed class TacSimEconomyPanels
    {
        private static readonly string[] SparkBlocks = { "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█" };

        private static readonly string[] CanonicalSurveyIds =
        {
            "ahuna-a",
            "kerwan-f",
            "juling-k",
            "occator-p"
        };

        private static readonly (string code, string label)[] HistoryAndHaulGoods =
        {
            ("volatiles", "Volatiles"),
            ("metals", "Metals"),
            ("structural_alloys", "Alloys"),
            ("industrial_parts", "Parts"),
            ("carbon_feedstock", "Carbon"),
            ("instrument_boards", "Boards"),
            ("propellant", "Fuel"),
            ("provisions", "Stores")
        };

        internal static readonly Regex RxHistoryHeader = new Regex(
            @"House prices of (.+?) here, by IC week, newest first \(\d+\):",
            RegexOptions.IgnoreCase | RegexOptions.Compiled);

        internal static readonly Regex RxHistoryRow = new Regex(
            @"^\s*(this IC week|last IC week|\d+\s+IC weeks ago):\s*ask\s+([^,]+),\s*bid\s+(.+)$",
            RegexOptions.IgnoreCase | RegexOptions.Compiled);

        private static readonly Regex RxTagStrip = new Regex(@"<[^>]+>", RegexOptions.Compiled);
        private static readonly Regex RxDecimal = new Regex(@"(-?\d+(?:\.\d+)?)", RegexOptions.Compiled);
        private static readonly Regex RxHullDbref = new Regex(@"^#[1-9]\d*$", RegexOptions.Compiled);

        private readonly Font _font;
        private readonly Action<string> _emitCommand;
        private readonly Func<string> _resolveSelectedPort;
        private readonly Func<string> _resolveActiveHullRef;
        private readonly Func<string> _resolveEscortHullRef;

        public GameObject HistoryTabBody { get; private set; }
        public GameObject JobsTabBody { get; private set; }
        public GameObject IndustryTabBody { get; private set; }

        public string SelectedHistoryGood { get; private set; } = "volatiles";
        public string SelectedHaulGood { get; private set; } = "volatiles";
        public string SelectedSurveyId { get; private set; } = "ahuna-a";
        public ParsedPriceHistorySummary LatestHistory { get; private set; } = new ParsedPriceHistorySummary();

        private int _selectedHaulGoodIndex;
        private int _selectedSurveyIndex;

        /// <summary>The last `+market/history` reply; the HUD feeds it every server line.</summary>
        public MarketHistoryReply HistoryReply { get; } = new MarketHistoryReply();

        private Text _historySparkText;
        private Text _historyOutputText;
        private RectTransform _historyViewportRect;
        private GameObject _historyQuestBanner;
        private Text _historyQuestLabel;
        private string _questHistorySlug = string.Empty;
        private string _questHistoryTitle = string.Empty;

        private Text _jobsOutputText;
        private RectTransform _jobsViewportRect;
        private GameObject _jobsQuestBanner;
        private Text _jobsQuestLabel;
        private Text _jobsQuestBtnLabel;
        private string _jobsQuestFilterKind = string.Empty;
        private Text _industryOutputText;
        private RectTransform _industryViewportRect;
        private GameObject _industryQuestBanner;
        private Image _industryQuestBannerBg;
        private Text _industryQuestLabel;
        private Text _industryQuestBtnLabel;
        private string _industryQuestCommand = string.Empty;
        private bool _industryQuestIsWayfinder;
        private bool _industryQuestClosesTradeModal;

        public event Action OnIndustryQuestWayfinderRequested;
        public event Action OnIndustryQuestTurnInDispatched;

        private Text _haulGoodBtnLabel;
        private InputField _jobIdInput;
        private InputField _surveyIdInput;
        private InputField _haulQtyInput;
        private InputField _haulFeeInput;
        private InputField _workOrderIdInput;
        private InputField _bayHullInput;
        private InputField _debtIdInput;
        private readonly List<Image> _goodBtnImages = new List<Image>();

        public string QuestHistoryCommodityForTests => _questHistorySlug;
        public bool IsHistoryQuestBannerVisible => _historyQuestBanner != null && _historyQuestBanner.activeSelf;
        public string HistoryQuestBannerTextForTests => _historyQuestLabel != null ? _historyQuestLabel.text : string.Empty;
        public bool IsIndustryQuestBannerVisible => _industryQuestBanner != null && _industryQuestBanner.activeSelf;
        public string IndustryQuestBannerTextForTests => _industryQuestLabel != null ? _industryQuestLabel.text : string.Empty;
        public string IndustryQuestButtonLabelForTests => _industryQuestBtnLabel != null ? _industryQuestBtnLabel.text : string.Empty;
        public bool IsJobsQuestBannerVisible => _jobsQuestBanner != null && _jobsQuestBanner.activeSelf;
        public string JobsQuestBannerTextForTests => _jobsQuestLabel != null ? _jobsQuestLabel.text : string.Empty;

        public bool TriggerJobsQuestFilterForTests()
        {
            if (!IsJobsQuestBannerVisible || string.IsNullOrEmpty(_jobsQuestFilterKind)) return false;
            _emitCommand?.Invoke(_jobsQuestFilterKind == "survey" ? "+market/jobs survey" : "+market/jobs haul");
            return true;
        }

        public bool TriggerQuestHistoryTrendForTests()
        {
            if (string.IsNullOrEmpty(_questHistorySlug)) return false;
            SelectQuestHistoryCommodityIfAvailable();
            ClearHistorySeries($"Fetching 24 IC-week house series for <b>{MushProtocol.EscapeUnityRichText(_questHistorySlug)}</b>...");
            _emitCommand?.Invoke($"+market/history tacsim:{_questHistorySlug}");
            return true;
        }

        public bool TriggerIndustryQuestActionForTests()
        {
            if (!IsIndustryQuestBannerVisible) return false;
            if (_industryQuestIsWayfinder)
            {
                OnIndustryQuestWayfinderRequested?.Invoke();
                return true;
            }
            if (string.IsNullOrEmpty(_industryQuestCommand)) return false;
            _emitCommand?.Invoke(_industryQuestCommand);
            if (_industryQuestClosesTradeModal)
                OnIndustryQuestTurnInDispatched?.Invoke();
            return true;
        }

        public bool SelectQuestHistoryCommodityIfAvailable()
        {
            if (string.IsNullOrEmpty(_questHistorySlug)) return false;
            SelectedHistoryGood = _questHistorySlug;
            int idx = FindHistoryGoodIndex(_questHistorySlug);
            HighlightGoodButtons(idx);
            return true;
        }

        private static int FindHistoryGoodIndex(string slug)
        {
            if (string.IsNullOrWhiteSpace(slug)) return -1;
            string s = slug.Trim();
            for (int i = 0; i < HistoryAndHaulGoods.Length; i++)
            {
                if (string.Equals(HistoryAndHaulGoods[i].code, s, StringComparison.OrdinalIgnoreCase))
                    return i;
            }
            return -1;
        }

        public void ApplyQuestGuidance(
            string tradeCommoditySlug,
            string tradeQuestTitle,
            string coopActionLabel,
            string coopActionCommand,
            string refitReadinessSummary,
            bool refitReady,
            string questDestPortLabel = null,
            bool hasSurfaceMiningQuest = false,
            string questStepTag = null,
            string readyTurnInSummary = null,
            string readyTurnInCommand = null,
            bool readyTurnInWayfinder = false)
        {
            _questHistorySlug = (tradeCommoditySlug ?? string.Empty).Trim().ToLowerInvariant();
            _questHistoryTitle = (tradeQuestTitle ?? string.Empty).Trim();
            string stepSuffix = string.IsNullOrWhiteSpace(questStepTag) ? string.Empty : $" {questStepTag.Trim()}";

            bool showHist = !string.IsNullOrEmpty(_questHistorySlug);
            if (_historyQuestBanner != null)
            {
                _historyQuestBanner.SetActive(showHist);
                if (showHist && _historyQuestLabel != null)
                {
                    string lbl = _questHistorySlug.Replace('_', ' ').ToUpperInvariant();
                    string titleSuffix = string.IsNullOrEmpty(_questHistoryTitle) ? string.Empty : $" — {_questHistoryTitle}";
                    _historyQuestLabel.text = $"[QUEST] 24-WEEK TREND: {lbl}{titleSuffix}{stepSuffix}";
                }
            }
            if (_historyViewportRect != null)
                _historyViewportRect.offsetMax = new Vector2(0f, showHist ? -206f : -164f);
            HighlightGoodButtons(FindHistoryGoodIndex(SelectedHistoryGood));

            bool showRefit = !string.IsNullOrEmpty(coopActionCommand) &&
                             coopActionCommand.StartsWith("+coop/refit", StringComparison.OrdinalIgnoreCase);
            bool showReadyTurnIn = !showRefit && !string.IsNullOrWhiteSpace(readyTurnInSummary);
            bool showInd = showRefit || showReadyTurnIn;
            _industryQuestCommand = string.Empty;
            _industryQuestIsWayfinder = false;
            _industryQuestClosesTradeModal = false;
            if (_industryQuestBanner != null)
            {
                _industryQuestBanner.SetActive(showInd);
                if (showRefit)
                {
                    _industryQuestCommand = coopActionCommand.Trim();
                    if (_industryQuestLabel != null)
                    {
                        string hdr = string.IsNullOrEmpty(coopActionLabel) ? "REFIT DRAY-P" : coopActionLabel.ToUpperInvariant();
                        string detail = string.IsNullOrEmpty(refitReadinessSummary) ? _industryQuestCommand : refitReadinessSummary;
                        _industryQuestLabel.text = $"[QUEST] {hdr} — {detail}{stepSuffix}";
                    }
                    if (_industryQuestBtnLabel != null)
                        _industryQuestBtnLabel.text = $"Run {_industryQuestCommand}";
                    if (_industryQuestBannerBg != null)
                        _industryQuestBannerBg.color = refitReady
                            ? new Color(0.08f, 0.34f, 0.22f, 0.94f)
                            : new Color(0.32f, 0.24f, 0.08f, 0.94f);
                }
                else if (showReadyTurnIn)
                {
                    bool canFinishHere = !string.IsNullOrWhiteSpace(readyTurnInCommand);
                    _industryQuestCommand = canFinishHere ? readyTurnInCommand.Trim() : string.Empty;
                    _industryQuestIsWayfinder = !canFinishHere && readyTurnInWayfinder;
                    _industryQuestClosesTradeModal = true;
                    if (_industryQuestLabel != null)
                        _industryQuestLabel.text = $"{readyTurnInSummary.Trim()}{stepSuffix}";
                    if (_industryQuestBtnLabel != null)
                        _industryQuestBtnLabel.text = canFinishHere ? "TURN IN (?)" : "WAYFINDER";
                    if (_industryQuestBannerBg != null)
                        _industryQuestBannerBg.color = canFinishHere
                            ? new Color(0.08f, 0.34f, 0.22f, 0.94f)
                            : new Color(0.32f, 0.24f, 0.08f, 0.94f);
                }
            }
            if (_industryViewportRect != null)
                _industryViewportRect.offsetMax = new Vector2(0f, showInd ? -176f : -134f);

            bool showJobs = hasSurfaceMiningQuest || !string.IsNullOrWhiteSpace(questDestPortLabel);
            _jobsQuestFilterKind = hasSurfaceMiningQuest
                ? "survey"
                : (!string.IsNullOrWhiteSpace(questDestPortLabel) ? "haul" : string.Empty);
            if (_jobsQuestBanner != null)
            {
                _jobsQuestBanner.SetActive(showJobs);
                if (showJobs)
                {
                    if (_jobsQuestLabel != null)
                    {
                        _jobsQuestLabel.text = hasSurfaceMiningQuest
                            ? $"[QUEST] Surface Survey Commissions — check open Ceres survey contracts (+market/jobs survey){stepSuffix}"
                            : $"[QUEST] Corridor Haulage -> {questDestPortLabel.Trim()} — stack bonus freight (+market/jobs haul){stepSuffix}";
                    }
                    if (_jobsQuestBtnLabel != null)
                        _jobsQuestBtnLabel.text = hasSurfaceMiningQuest ? "Filter Surveys" : "Filter Haulage";
                }
            }
            if (_jobsViewportRect != null)
                _jobsViewportRect.offsetMax = new Vector2(0f, showJobs ? -176f : -134f);
        }

        // Two-click confirmation state for financial / contract commitments (`U-M15`)
        private string _armedConfirmKey;
        private Text _armedConfirmLabel;
        private string _armedOriginalText;
        private float _armedAtTime;
        private const float ConfirmWindowSeconds = 8f;

        public bool IsAnyInputFocused =>
            (_jobIdInput != null && _jobIdInput.isFocused) ||
            (_surveyIdInput != null && _surveyIdInput.isFocused) ||
            (_haulQtyInput != null && _haulQtyInput.isFocused) ||
            (_haulFeeInput != null && _haulFeeInput.isFocused) ||
            (_workOrderIdInput != null && _workOrderIdInput.isFocused) ||
            (_bayHullInput != null && _bayHullInput.isFocused) ||
            (_debtIdInput != null && _debtIdInput.isFocused);

        public static bool IsValidHullDbref(string candidate)
        {
            return !string.IsNullOrWhiteSpace(candidate) && RxHullDbref.IsMatch(candidate.Trim());
        }

        public void SetSelectedHaulGood(string goodCode)
        {
            if (string.IsNullOrWhiteSpace(goodCode)) return;
            string trimmed = goodCode.Trim();
            SelectedHaulGood = trimmed;
            for (int i = 0; i < HistoryAndHaulGoods.Length; i++)
            {
                if (string.Equals(HistoryAndHaulGoods[i].code, trimmed, StringComparison.OrdinalIgnoreCase))
                {
                    _selectedHaulGoodIndex = i;
                    break;
                }
            }
            if (_haulGoodBtnLabel != null)
                _haulGoodBtnLabel.text = $"Good: {SelectedHaulGood}";
        }

        public void SetSelectedSurveyId(string surveyId)
        {
            if (string.IsNullOrWhiteSpace(surveyId)) return;
            string trimmed = surveyId.Trim();
            SelectedSurveyId = trimmed;
            for (int i = 0; i < CanonicalSurveyIds.Length; i++)
            {
                if (string.Equals(CanonicalSurveyIds[i], trimmed, StringComparison.OrdinalIgnoreCase))
                {
                    _selectedSurveyIndex = i;
                    break;
                }
            }
            if (_surveyIdInput != null)
                _surveyIdInput.text = trimmed;
        }

        public void DisarmConfirm()
        {
            if (_armedConfirmLabel != null && !string.IsNullOrEmpty(_armedOriginalText))
                _armedConfirmLabel.text = _armedOriginalText;
            _armedConfirmKey = null;
            _armedConfirmLabel = null;
            _armedOriginalText = null;
        }

        public void ClearHistorySeries(string statusPrompt = null)
        {
            HistoryReply.Clear();
            LatestHistory = new ParsedPriceHistorySummary();
            if (_historySparkText != null)
            {
                _historySparkText.text = !string.IsNullOrEmpty(statusPrompt)
                    ? statusPrompt
                    : "Select a commodity above to fetch 24 IC-week house ask & bid trends (`+market/history`).";
            }
        }

        public void ResetSessionState()
        {
            DisarmConfirm();
            SelectedHistoryGood = "volatiles";
            SetSelectedHaulGood("volatiles");
            SetSelectedSurveyId("ahuna-a");
            ApplyQuestGuidance(string.Empty, string.Empty, string.Empty, string.Empty, string.Empty, false);
            ClearHistorySeries();
            if (_historyOutputText != null) _historyOutputText.text = "Ready.";
            if (_jobsOutputText != null) _jobsOutputText.text = "Ready.";
            if (_industryOutputText != null) _industryOutputText.text = "Ready.";
            if (_jobIdInput != null) _jobIdInput.text = string.Empty;
            if (_haulQtyInput != null) _haulQtyInput.text = "10";
            if (_haulFeeInput != null) _haulFeeInput.text = "250";
            if (_workOrderIdInput != null) _workOrderIdInput.text = "1";
            if (_bayHullInput != null) _bayHullInput.text = string.Empty;
            if (_debtIdInput != null) _debtIdInput.text = "1";
            HighlightGoodButtons(0);
        }

        public TacSimEconomyPanels(
            Font font,
            Action<string> emitCommand,
            Func<string> resolveSelectedPort,
            Func<string> resolveActiveHullRef,
            Func<string> resolveEscortHullRef = null)
        {
            _font = font;
            _emitCommand = emitCommand;
            _resolveSelectedPort = resolveSelectedPort;
            _resolveActiveHullRef = resolveActiveHullRef;
            _resolveEscortHullRef = resolveEscortHullRef;
        }

        public void BuildSubPanels(Transform tradePanelTransform)
        {
            BuildHistoryTab(tradePanelTransform);
            BuildJobsTab(tradePanelTransform);
            BuildIndustryTab(tradePanelTransform);
        }

        public static string StripRichTags(string line)
        {
            if (string.IsNullOrEmpty(line)) return string.Empty;
            return RxTagStrip.Replace(line, string.Empty);
        }

        private static double? ParseMajorPrice(string text)
        {
            if (string.IsNullOrWhiteSpace(text)) return null;
            string trimmed = text.Trim();
            if (string.Equals(trimmed, "none", StringComparison.OrdinalIgnoreCase))
                return null;
            var m = RxDecimal.Match(trimmed.Replace(",", string.Empty));
            if (!m.Success) return null;
            if (double.TryParse(m.Groups[1].Value, NumberStyles.Float, CultureInfo.InvariantCulture, out double v))
                return v;
            return null;
        }

        public static string BuildSparkline(IReadOnlyList<double?> valuesOldestFirst)
        {
            if (valuesOldestFirst == null || valuesOldestFirst.Count == 0)
                return "—";

            double min = double.MaxValue;
            double max = double.MinValue;
            int validCount = 0;
            for (int i = 0; i < valuesOldestFirst.Count; i++)
            {
                if (valuesOldestFirst[i].HasValue)
                {
                    double v = valuesOldestFirst[i].Value;
                    if (v < min) min = v;
                    if (v > max) max = v;
                    validCount++;
                }
            }
            if (validCount == 0) return "—";

            double span = max - min;
            var sb = new StringBuilder(valuesOldestFirst.Count);
            for (int i = 0; i < valuesOldestFirst.Count; i++)
            {
                if (!valuesOldestFirst[i].HasValue)
                {
                    sb.Append('·');
                    continue;
                }
                if (span <= 0.0001)
                {
                    sb.Append(SparkBlocks[4]);
                    continue;
                }
                int idx = Mathf.Clamp(Mathf.RoundToInt((float)((valuesOldestFirst[i].Value - min) / span * 7.0)), 0, 7);
                sb.Append(SparkBlocks[idx]);
            }
            return sb.ToString();
        }

        public static ParsedPriceHistorySummary ParseMarketHistoryLines(IEnumerable<string> rawLines)
        {
            var res = new ParsedPriceHistorySummary();
            if (rawLines == null) return res;

            foreach (string raw in rawLines)
            {
                string plain = StripRichTags(raw);
                var hm = RxHistoryHeader.Match(plain);
                if (hm.Success)
                {
                    res.GoodLabel = hm.Groups[1].Value.Trim();
                    res.RowsNewestFirst.Clear();
                    continue;
                }

                var rm = RxHistoryRow.Match(plain);
                if (rm.Success)
                {
                    string when = rm.Groups[1].Value.Trim();
                    string askTxt = rm.Groups[2].Value.Trim();
                    string bidTxt = rm.Groups[3].Value.Trim();
                    res.RowsNewestFirst.Add(new ParsedPriceHistoryRow
                    {
                        When = when,
                        AskText = askTxt,
                        BidText = bidTxt,
                        AskMajor = ParseMajorPrice(askTxt),
                        BidMajor = ParseMajorPrice(bidTxt),
                    });
                }
            }

            if (res.RowsNewestFirst.Count == 0)
                return res;

            var asksOldest = new List<double?>(res.RowsNewestFirst.Count);
            var bidsOldest = new List<double?>(res.RowsNewestFirst.Count);
            double askMin = double.MaxValue, askMax = double.MinValue;
            double bidMin = double.MaxValue, bidMax = double.MinValue;
            int askValid = 0, bidValid = 0;

            for (int i = res.RowsNewestFirst.Count - 1; i >= 0; i--)
            {
                var r = res.RowsNewestFirst[i];
                asksOldest.Add(r.AskMajor);
                bidsOldest.Add(r.BidMajor);
                if (r.AskMajor.HasValue)
                {
                    askMin = Math.Min(askMin, r.AskMajor.Value);
                    askMax = Math.Max(askMax, r.AskMajor.Value);
                    askValid++;
                }
                if (r.BidMajor.HasValue)
                {
                    bidMin = Math.Min(bidMin, r.BidMajor.Value);
                    bidMax = Math.Max(bidMax, r.BidMajor.Value);
                    bidValid++;
                }
            }

            res.AskSparkline = BuildSparkline(asksOldest);
            res.BidSparkline = BuildSparkline(bidsOldest);
            res.LatestAsk = res.RowsNewestFirst[0].AskText;
            res.LatestBid = res.RowsNewestFirst[0].BidText;
            res.AskRange = askValid > 0
                ? string.Format(CultureInfo.InvariantCulture, "{0:F2} - {1:F2}", askMin, askMax)
                : "—";
            res.BidRange = bidValid > 0
                ? string.Format(CultureInfo.InvariantCulture, "{0:F2} - {1:F2}", bidMin, bidMax)
                : "—";
            return res;
        }

        public void IngestEchoLines(IReadOnlyList<string> echoLines)
        {
            IngestTabEchoLines(TradeTerminalTab.History, echoLines);
        }

        public void IngestTabEchoLines(TradeTerminalTab tab, IReadOnlyList<string> echoLines)
        {
            if (echoLines == null) return;
            string joined = echoLines.Count > 0 ? string.Join("\n", echoLines) : "Ready.";

            if (tab == TradeTerminalTab.History)
            {
                bool noHistoryReported = false;
                for (int i = 0; i < echoLines.Count; i++)
                {
                    string plain = StripRichTags(echoLines[i]);
                    if (plain.IndexOf("No price history", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        noHistoryReported = true;
                        break;
                    }
                }

                if (noHistoryReported)
                {
                    ClearHistorySeries($"No price history is kept here for <b>{MushProtocol.EscapeUnityRichText(SelectedHistoryGood)}</b>.");
                }
                else
                {
                    // Prefer the dedicated HistoryReply accumulator (holds heading + all 24 weeks
                    // without eviction), falling back to echoLines for direct callers/tests.
                    IReadOnlyList<string> sourceLines = HistoryReply.Lines.Count > 0 ? HistoryReply.Lines : echoLines;
                    var parsed = ParseMarketHistoryLines(sourceLines);
                    if (parsed.RowsNewestFirst.Count > 0)
                    {
                        LatestHistory = parsed;
                        if (_historySparkText != null)
                        {
                            string lbl = string.IsNullOrEmpty(LatestHistory.GoodLabel) ? SelectedHistoryGood : LatestHistory.GoodLabel;
                            _historySparkText.text =
                                $"<color=#66DDFF><b>{MushProtocol.EscapeUnityRichText(lbl)} — 24-WEEK HOUSE SERIES</b></color>\n" +
                                $"ASK TREND: <color=#66EEFF><b>{LatestHistory.AskSparkline}</b></color>   Latest: <b>{MushProtocol.EscapeUnityRichText(LatestHistory.LatestAsk)}</b>  (Band {LatestHistory.AskRange})\n" +
                                $"BID TREND: <color=#FFCC66><b>{LatestHistory.BidSparkline}</b></color>   Latest: <b>{MushProtocol.EscapeUnityRichText(LatestHistory.LatestBid)}</b>  (Band {LatestHistory.BidRange})";
                        }
                    }
                }

                if (_historyOutputText != null) _historyOutputText.text = joined;
            }
            else if (tab == TradeTerminalTab.Jobs)
            {
                // Auto-detect an open canonical survey ID surfaced by `+market/jobs survey` or `+survey`.
                for (int i = 0; i < echoLines.Count; i++)
                {
                    string plain = StripRichTags(echoLines[i]);
                    for (int s = 0; s < CanonicalSurveyIds.Length; s++)
                    {
                        if (plain.IndexOf(CanonicalSurveyIds[s], StringComparison.OrdinalIgnoreCase) >= 0)
                        {
                            SelectedSurveyId = CanonicalSurveyIds[s];
                            _selectedSurveyIndex = s;
                            if (_surveyIdInput != null && !_surveyIdInput.isFocused &&
                                (string.IsNullOrWhiteSpace(_surveyIdInput.text) || Array.IndexOf(CanonicalSurveyIds, _surveyIdInput.text.Trim()) >= 0))
                            {
                                _surveyIdInput.text = SelectedSurveyId;
                            }
                            break;
                        }
                    }
                }
                if (_jobsOutputText != null) _jobsOutputText.text = joined;
            }
            else if (tab == TradeTerminalTab.Industry)
            {
                if (_industryOutputText != null) _industryOutputText.text = joined;
            }
        }

        private string ResolveEscortHullDbref()
        {
            string escortCandidate = _resolveEscortHullRef != null ? _resolveEscortHullRef() : null;
            if (IsValidHullDbref(escortCandidate))
                return escortCandidate.Trim();
            string candidate = _resolveActiveHullRef != null ? _resolveActiveHullRef() : null;
            return IsValidHullDbref(candidate) ? candidate.Trim() : null;
        }

        private string ResolveBayTarget()
        {
            if (_bayHullInput != null && !string.IsNullOrWhiteSpace(_bayHullInput.text))
                return _bayHullInput.text.Trim();
            string active = ResolveEscortHullDbref();
            return !string.IsNullOrEmpty(active) ? active : "ship";
        }

        private string ResolveSurveyId()
        {
            if (_surveyIdInput != null && !string.IsNullOrWhiteSpace(_surveyIdInput.text))
                return _surveyIdInput.text.Trim();
            return !string.IsNullOrEmpty(SelectedSurveyId) ? SelectedSurveyId : CanonicalSurveyIds[0];
        }

        private int ResolveHaulQuantity()
        {
            if (_haulQtyInput != null && int.TryParse(_haulQtyInput.text.Trim(), out int q) && q > 0)
                return q;
            return 10;
        }

        private int ResolveHaulOrEscortFee(int defaultFee)
        {
            if (_haulFeeInput != null && int.TryParse(_haulFeeInput.text.Trim(), out int f) && f > 0)
                return f;
            return defaultFee;
        }

        private Button CreateConfirmButton(
            Transform parent,
            string confirmKey,
            string label,
            string confirmLabel,
            Color bg,
            Action onFirstClickPreview,
            Action onConfirmed)
        {
            Text lblRef = null;
            var btn = CreateButton(parent, label, bg, () =>
            {
                if (_armedConfirmKey == confirmKey && (Time.unscaledTime - _armedAtTime) <= ConfirmWindowSeconds)
                {
                    DisarmConfirm();
                    onConfirmed?.Invoke();
                    return;
                }
                DisarmConfirm();
                _armedConfirmKey = confirmKey;
                _armedConfirmLabel = lblRef;
                _armedOriginalText = label;
                _armedAtTime = Time.unscaledTime;
                if (lblRef != null) lblRef.text = confirmLabel;
                onFirstClickPreview?.Invoke();
            });
            lblRef = btn.GetComponentInChildren<Text>();
            return btn;
        }

        private void BuildHistoryTab(Transform parent)
        {
            HistoryTabBody = CreatePanel(parent, "HistoryTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -154f), Color.clear);

            var goodRow = CreateRow(HistoryTabBody.transform, "HistoryGoodsRow", -38f, 0f);
            _goodBtnImages.Clear();
            for (int i = 0; i < HistoryAndHaulGoods.Length; i++)
            {
                int idx = i;
                string code = HistoryAndHaulGoods[i].code;
                var b = CreateButton(goodRow, HistoryAndHaulGoods[i].label, new Color(0.14f, 0.28f, 0.40f), () =>
                {
                    DisarmConfirm();
                    SelectedHistoryGood = code;
                    HighlightGoodButtons(idx);
                    ClearHistorySeries($"Fetching 24 IC-week house series for <b>{MushProtocol.EscapeUnityRichText(code)}</b>...");
                    _emitCommand?.Invoke($"+market/history tacsim:{code}");
                });
                _goodBtnImages.Add(b.GetComponent<Image>());
            }
            HighlightGoodButtons(0);

            var actRow = CreateRow(HistoryTabBody.transform, "HistoryActionsRow", -82f, -44f);
            CreateButton(actRow, "24w Trend", new Color(0.16f, 0.46f, 0.56f), () =>
            {
                DisarmConfirm();
                ClearHistorySeries($"Fetching 24 IC-week house series for <b>{MushProtocol.EscapeUnityRichText(SelectedHistoryGood)}</b>...");
                _emitCommand?.Invoke($"+market/history tacsim:{SelectedHistoryGood}");
            });
            CreateButton(actRow, "Public Depth", new Color(0.16f, 0.36f, 0.48f), () => _emitCommand?.Invoke($"+market/depth tacsim:{SelectedHistoryGood}"));
            CreateButton(actRow, "My Listings", new Color(0.20f, 0.34f, 0.44f), () => _emitCommand?.Invoke("+market/mine"));
            CreateButton(actRow, "Check Locker", new Color(0.18f, 0.42f, 0.36f), () => _emitCommand?.Invoke("+locker"));
            CreateButton(actRow, "Open Locker", new Color(0.16f, 0.50f, 0.38f), () => _emitCommand?.Invoke("+locker/open"));
            CreateButton(actRow, "Debts", new Color(0.46f, 0.28f, 0.16f), () => _emitCommand?.Invoke("+debts"));
            _debtIdInput = CreateMiniInput(actRow, "Debt #", "1");
            CreateConfirmButton(
                actRow,
                "pay_debt",
                "Pay Debt",
                "CONFIRM PAY",
                new Color(0.52f, 0.24f, 0.18f),
                onFirstClickPreview: () =>
                {
                    string id = _debtIdInput != null && !string.IsNullOrWhiteSpace(_debtIdInput.text) ? _debtIdInput.text.Trim() : "1";
                    if (_historyOutputText != null)
                        _historyOutputText.text = $"<color=#FFCC66>Confirm payment of Debt #{MushProtocol.EscapeUnityRichText(id)}: click CONFIRM PAY to run +debts/pay {MushProtocol.EscapeUnityRichText(id)}.</color>";
                },
                onConfirmed: () =>
                {
                    string id = _debtIdInput != null && !string.IsNullOrWhiteSpace(_debtIdInput.text) ? _debtIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+debts/pay {id}");
                });

            var sparkCard = CreatePanel(HistoryTabBody.transform, "SparklineSummaryCard", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -156f), new Vector2(0f, -88f), new Color(0.04f, 0.10f, 0.16f, 0.94f));
            _historySparkText = CreateText(sparkCard.transform, "Select a commodity above to fetch 24 IC-week house ask & bid trends (`+market/history`).", 13, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
            _historySparkText.supportRichText = true;
            SetRect(_historySparkText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 4f), new Vector2(-12f, -4f));

            _historyQuestBanner = CreatePanel(HistoryTabBody.transform, "HistoryQuestBanner", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -200f), new Vector2(0f, -162f), new Color(0.32f, 0.24f, 0.08f, 0.94f));
            _historyQuestLabel = CreateText(_historyQuestBanner.transform, "[QUEST] 24-WEEK TREND", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.93f, 0.68f));
            SetRect(_historyQuestLabel.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 2f), new Vector2(-180f, -2f));
            var hqBtn = CreateButton(_historyQuestBanner.transform, "Load Quest Trend", new Color(0.56f, 0.39f, 0.10f), () => TriggerQuestHistoryTrendForTests());
            SetRect(hqBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-172f, 4f), new Vector2(-8f, -4f));
            _historyQuestBanner.SetActive(false);

            _historyOutputText = BuildScrollOutput(HistoryTabBody.transform, "HistoryViewport", -164f, out _historyViewportRect);
            HistoryTabBody.SetActive(false);
        }

        private void BuildJobsTab(Transform parent)
        {
            JobsTabBody = CreatePanel(parent, "JobsTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -154f), Color.clear);

            var filterRow = CreateRow(JobsTabBody.transform, "JobsFilterRow", -38f, 0f);
            CreateButton(filterRow, "All Jobs", new Color(0.16f, 0.44f, 0.56f), () => _emitCommand?.Invoke("+market/jobs"));
            CreateButton(filterRow, "Haulage", new Color(0.16f, 0.38f, 0.48f), () => _emitCommand?.Invoke("+market/jobs haul"));
            CreateButton(filterRow, "Escort", new Color(0.46f, 0.26f, 0.18f), () => _emitCommand?.Invoke("+market/jobs escort"));
            CreateButton(filterRow, "Survey", new Color(0.18f, 0.42f, 0.34f), () => _emitCommand?.Invoke("+market/jobs survey"));
            CreateButton(filterRow, "Retrieval", new Color(0.38f, 0.32f, 0.16f), () => _emitCommand?.Invoke("+market/jobs retrieval"));
            CreateButton(filterRow, "Passage", new Color(0.28f, 0.28f, 0.48f), () => _emitCommand?.Invoke("+market/jobs passage"));
            CreateButton(filterRow, "Crew (+sortie/crew)", new Color(0.16f, 0.48f, 0.40f), () => _emitCommand?.Invoke("+sortie/crew"));

            var paramsRow = CreateRow(JobsTabBody.transform, "JobsParamsRow", -82f, -44f);
            // The job box starts empty: a placeholder id such as "1" matches no real job.
            _jobIdInput = CreateMiniInput(paramsRow, "Job #", string.Empty);
            _surveyIdInput = CreateMiniInput(paramsRow, "Survey ID", CanonicalSurveyIds[0]);
            CreateButton(paramsRow, "Next Survey", new Color(0.16f, 0.34f, 0.44f), () =>
            {
                _selectedSurveyIndex = (_selectedSurveyIndex + 1) % CanonicalSurveyIds.Length;
                SetSelectedSurveyId(CanonicalSurveyIds[_selectedSurveyIndex]);
            });
            var haulGoodBtn = CreateButton(paramsRow, $"Good: {SelectedHaulGood}", new Color(0.16f, 0.34f, 0.42f), () =>
            {
                _selectedHaulGoodIndex = (_selectedHaulGoodIndex + 1) % HistoryAndHaulGoods.Length;
                SetSelectedHaulGood(HistoryAndHaulGoods[_selectedHaulGoodIndex].code);
            });
            _haulGoodBtnLabel = haulGoodBtn.GetComponentInChildren<Text>();
            _haulQtyInput = CreateMiniInput(paramsRow, "Qty t", "10");
            _haulFeeInput = CreateMiniInput(paramsRow, "Fee cr", "250");

            var actRow = CreateRow(JobsTabBody.transform, "JobsActRow", -126f, -88f);
            CreateConfirmButton(
                actRow,
                "accept_haul",
                "Accept Haul",
                "CONFIRM HAUL",
                new Color(0.16f, 0.48f, 0.36f),
                onFirstClickPreview: () => EmitJobCommand("+haul/accept", go: false),
                onConfirmed: () => EmitJobCommand("+haul/accept", go: true));
            CreateConfirmButton(
                actRow,
                "deliver_haul",
                "Deliver Haul",
                "CONFIRM DELIV",
                new Color(0.48f, 0.34f, 0.14f),
                onFirstClickPreview: () => EmitJobCommand("+haul/deliver", go: false),
                onConfirmed: () => EmitJobCommand("+haul/deliver", go: true));
            CreateConfirmButton(
                actRow,
                "accept_survey",
                "Accept Survey",
                "CONFIRM SURV",
                new Color(0.18f, 0.38f, 0.50f),
                onFirstClickPreview: () => EmitSurveyAccept(confirm: false),
                onConfirmed: () => EmitSurveyAccept(confirm: true));
            CreateConfirmButton(
                actRow,
                "post_haul",
                "Post Haul",
                "CONFIRM POST",
                new Color(0.22f, 0.42f, 0.48f),
                onFirstClickPreview: () =>
                {
                    string dest = _resolveSelectedPort != null ? _resolveSelectedPort() : "stickney";
                    int qty = ResolveHaulQuantity();
                    int fee = ResolveHaulOrEscortFee(250);
                    _emitCommand?.Invoke($"+haul/post {qty} {SelectedHaulGood} to={dest} fee={fee}");
                },
                onConfirmed: () =>
                {
                    string dest = _resolveSelectedPort != null ? _resolveSelectedPort() : "stickney";
                    int qty = ResolveHaulQuantity();
                    int fee = ResolveHaulOrEscortFee(250);
                    _emitCommand?.Invoke($"+haul/post {qty} {SelectedHaulGood} to={dest} fee={fee} go");
                });
            CreateConfirmButton(
                actRow,
                "post_escort",
                "Post Escort",
                "CONFIRM ESCRT",
                new Color(0.46f, 0.26f, 0.20f),
                onFirstClickPreview: () =>
                {
                    string hull = ResolveEscortHullDbref();
                    if (string.IsNullOrEmpty(hull))
                    {
                        DisarmConfirm();
                        if (_jobsOutputText != null)
                            _jobsOutputText.text = "<color=#FFAA55>Cannot post escort: no active hull #‹dbref› detected. Refresh Exchange/Ship status while docked first.</color>";
                        return;
                    }
                    string dest = _resolveSelectedPort != null ? _resolveSelectedPort() : "stickney";
                    int fee = ResolveHaulOrEscortFee(300);
                    _emitCommand?.Invoke($"+escort/post {hull} to={dest} fee={fee}");
                },
                onConfirmed: () =>
                {
                    string hull = ResolveEscortHullDbref();
                    if (string.IsNullOrEmpty(hull))
                    {
                        if (_jobsOutputText != null)
                            _jobsOutputText.text = "<color=#FFAA55>Cannot post escort: no active hull #‹dbref› detected.</color>";
                        return;
                    }
                    string dest = _resolveSelectedPort != null ? _resolveSelectedPort() : "stickney";
                    int fee = ResolveHaulOrEscortFee(300);
                    _emitCommand?.Invoke($"+escort/post {hull} to={dest} fee={fee} go");
                });
            CreateButton(actRow, "My Crew", new Color(0.22f, 0.32f, 0.44f), () => _emitCommand?.Invoke("+sortie/contract"));

            _jobsQuestBanner = CreatePanel(JobsTabBody.transform, "JobsQuestBanner", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -170f), new Vector2(0f, -132f), new Color(0.32f, 0.24f, 0.08f, 0.94f));
            _jobsQuestLabel = CreateText(_jobsQuestBanner.transform, "[QUEST] Jobs", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.93f, 0.68f));
            SetRect(_jobsQuestLabel.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 2f), new Vector2(-176f, -2f));
            var jqBtn = CreateButton(_jobsQuestBanner.transform, "Filter Jobs", new Color(0.56f, 0.39f, 0.10f), () => TriggerJobsQuestFilterForTests());
            _jobsQuestBtnLabel = jqBtn.GetComponentInChildren<Text>();
            SetRect(jqBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-168f, 4f), new Vector2(-8f, -4f));
            _jobsQuestBanner.SetActive(false);

            _jobsOutputText = BuildScrollOutput(JobsTabBody.transform, "JobsViewport", -134f, out _jobsViewportRect);
            JobsTabBody.SetActive(false);
        }

        private void BuildIndustryTab(Transform parent)
        {
            IndustryTabBody = CreatePanel(parent, "IndustryTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -154f), Color.clear);

            var topRow = CreateRow(IndustryTabBody.transform, "WorksStatusRow", -38f, 0f);
            CreateButton(topRow, "Works (+works)", new Color(0.16f, 0.44f, 0.54f), () => _emitCommand?.Invoke("+works"));
            CreateButton(topRow, "Shipyard (+yard)", new Color(0.18f, 0.38f, 0.50f), () => _emitCommand?.Invoke("+yard"));
            _bayHullInput = CreateMiniInput(topRow, "Hull/#", string.Empty);
            CreateButton(topRow, "Bay Status", new Color(0.18f, 0.42f, 0.36f), () =>
            {
                _emitCommand?.Invoke($"+bay {ResolveBayTarget()}");
            });
            CreateConfirmButton(
                topRow,
                "bay_rearm",
                "Rearm Magazines",
                "CONFIRM REARM",
                new Color(0.50f, 0.30f, 0.14f),
                onFirstClickPreview: () =>
                {
                    if (_industryOutputText != null)
                        _industryOutputText.text = $"<color=#FFCC66>Confirm magazine rearm for {MushProtocol.EscapeUnityRichText(ResolveBayTarget())}: click CONFIRM REARM to commit.</color>";
                },
                onConfirmed: () => _emitCommand?.Invoke($"+bay/order {ResolveBayTarget()}=rearm"));
            CreateConfirmButton(
                topRow,
                "bay_repair",
                "Full Repair",
                "CONFIRM REPAIR",
                new Color(0.16f, 0.48f, 0.36f),
                onFirstClickPreview: () =>
                {
                    if (_industryOutputText != null)
                        _industryOutputText.text = $"<color=#FFCC66>Confirm full repair order for {MushProtocol.EscapeUnityRichText(ResolveBayTarget())}: click CONFIRM REPAIR to commit.</color>";
                },
                onConfirmed: () => _emitCommand?.Invoke($"+bay/order {ResolveBayTarget()}=all"));
            CreateButton(topRow, "Core Shop (+core)", new Color(0.32f, 0.26f, 0.48f), () => _emitCommand?.Invoke("+core"));
            CreateButton(topRow, "Facilities", new Color(0.22f, 0.32f, 0.42f), () => _emitCommand?.Invoke("@facility/list"));

            var recipeRow = CreateRow(IndustryTabBody.transform, "WorksRecipesRow", -82f, -44f);
            CreateConfirmButton(recipeRow, "works_metals", "Mill Metals", "CONFIRM MILL", new Color(0.16f, 0.40f, 0.46f),
                () => NotifyIndustryConfirm("+works/order metals"), () => _emitCommand?.Invoke("+works/order metals"));
            CreateConfirmButton(recipeRow, "works_fittings", "Fittings Batch", "CONFIRM FIT", new Color(0.16f, 0.40f, 0.46f),
                () => NotifyIndustryConfirm("+works/order fittings"), () => _emitCommand?.Invoke("+works/order fittings"));
            CreateConfirmButton(recipeRow, "works_ap30", "30mm AP Box", "CONFIRM 30MM", new Color(0.42f, 0.28f, 0.16f),
                () => NotifyIndustryConfirm("+works/order ap_30"), () => _emitCommand?.Invoke("+works/order ap_30"));
            CreateConfirmButton(recipeRow, "works_he105", "105mm HE", "CONFIRM 105MM", new Color(0.42f, 0.28f, 0.16f),
                () => NotifyIndustryConfirm("+works/order he_105"), () => _emitCommand?.Invoke("+works/order he_105"));
            CreateConfirmButton(recipeRow, "yard_dray", "Order Dray", "CONFIRM DRAY", new Color(0.20f, 0.42f, 0.56f),
                () => NotifyIndustryConfirm("+yard/order dray"), () => _emitCommand?.Invoke("+yard/order dray"));
            CreateConfirmButton(recipeRow, "yard_dray_p", "Order Dray-P", "CONFIRM DRAY-P", new Color(0.20f, 0.42f, 0.56f),
                () => NotifyIndustryConfirm("+yard/order dray_p"), () => _emitCommand?.Invoke("+yard/order dray_p"));
            CreateConfirmButton(recipeRow, "yard_tank", "Order Line Tank", "CONFIRM TANK", new Color(0.46f, 0.24f, 0.22f),
                () => NotifyIndustryConfirm("+yard/order line_tank"), () => _emitCommand?.Invoke("+yard/order line_tank"));

            var orderRow = CreateRow(IndustryTabBody.transform, "OrderEscrowRow", -126f, -88f);
            _workOrderIdInput = CreateMiniInput(orderRow, "Order #", "1");
            CreateConfirmButton(orderRow, "works_buy", "Buy Works Bill", "CONFIRM BUY", new Color(0.16f, 0.46f, 0.38f),
                () =>
                {
                    string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                    NotifyIndustryConfirm($"+works/buy {ord}");
                },
                () =>
                {
                    string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+works/buy {ord}");
                });
            CreateButton(orderRow, "Collect Works", new Color(0.48f, 0.34f, 0.14f), () =>
            {
                string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+works/collect {ord}");
            });
            CreateConfirmButton(orderRow, "yard_buy", "Buy Yard Bill", "CONFIRM BUY", new Color(0.16f, 0.46f, 0.54f),
                () =>
                {
                    string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                    NotifyIndustryConfirm($"+yard/buy {ord}");
                },
                () =>
                {
                    string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+yard/buy {ord}");
                });
            CreateConfirmButton(orderRow, "yard_collect", "Commission Hull", "CONFIRM COMM", new Color(0.50f, 0.32f, 0.16f),
                () =>
                {
                    string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                    NotifyIndustryConfirm($"+yard/collect {ord}");
                },
                () =>
                {
                    string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+yard/collect {ord}");
                });

            _industryQuestBanner = CreatePanel(IndustryTabBody.transform, "IndustryQuestBanner", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -170f), new Vector2(0f, -132f), new Color(0.32f, 0.24f, 0.08f, 0.94f));
            _industryQuestBannerBg = _industryQuestBanner.GetComponent<Image>();
            _industryQuestLabel = CreateText(_industryQuestBanner.transform, "[QUEST] REFIT DRAY-P", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.94f, 0.72f));
            SetRect(_industryQuestLabel.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 2f), new Vector2(-216f, -2f));
            var iqBtn = CreateButton(_industryQuestBanner.transform, "Run +coop/refit prospector", new Color(0.56f, 0.39f, 0.10f), () => TriggerIndustryQuestActionForTests());
            SetRect(iqBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-208f, 4f), new Vector2(-8f, -4f));
            _industryQuestBtnLabel = iqBtn.GetComponentInChildren<Text>();
            _industryQuestBanner.SetActive(false);

            _industryOutputText = BuildScrollOutput(IndustryTabBody.transform, "IndustryViewport", -134f, out _industryViewportRect);
            IndustryTabBody.SetActive(false);
        }

        private void NotifyIndustryConfirm(string command)
        {
            if (_industryOutputText != null)
                _industryOutputText.text = $"<color=#FFCC66>Click again to confirm order: <b>{MushProtocol.EscapeUnityRichText(command)}</b></color>";
        }

        private void HighlightGoodButtons(int selectedIdx)
        {
            int questIdx = FindHistoryGoodIndex(_questHistorySlug);
            for (int i = 0; i < _goodBtnImages.Count; i++)
            {
                if (_goodBtnImages[i] == null) continue;
                if (i == selectedIdx && i == questIdx)
                    _goodBtnImages[i].color = new Color(0.74f, 0.54f, 0.12f, 1f);
                else if (i == selectedIdx)
                    _goodBtnImages[i].color = new Color(0.22f, 0.66f, 0.88f, 1f);
                else if (i == questIdx)
                    _goodBtnImages[i].color = new Color(0.46f, 0.34f, 0.10f, 0.96f);
                else
                    _goodBtnImages[i].color = new Color(0.14f, 0.26f, 0.38f, 0.92f);
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

        private Text BuildScrollOutput(Transform parent, string name, float topInset)
        {
            return BuildScrollOutput(parent, name, topInset, out _);
        }

        private Text BuildScrollOutput(Transform parent, string name, float topInset, out RectTransform viewportRect)
        {
            var vp = CreatePanel(parent, name, new Vector2(0f, 0f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, topInset), new Color(0.02f, 0.05f, 0.09f, 0.90f));
            viewportRect = vp.GetComponent<RectTransform>();
            vp.AddComponent<RectMask2D>();
            var sr = vp.AddComponent<ScrollRect>();

            var txt = CreateText(vp.transform, "Ready.", 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 1f));
            txt.supportRichText = true;
            txt.horizontalOverflow = HorizontalWrapMode.Wrap;
            txt.verticalOverflow = VerticalWrapMode.Overflow;
            var csf = txt.gameObject.AddComponent<ContentSizeFitter>();
            csf.horizontalFit = ContentSizeFitter.FitMode.Unconstrained;
            csf.verticalFit = ContentSizeFitter.FitMode.PreferredSize;
            txt.rectTransform.anchorMin = new Vector2(0f, 1f);
            txt.rectTransform.anchorMax = new Vector2(1f, 1f);
            txt.rectTransform.pivot = new Vector2(0.5f, 1f);
            txt.rectTransform.offsetMin = new Vector2(12f, 0f);
            txt.rectTransform.offsetMax = new Vector2(-12f, 0f);

            sr.viewport = vp.GetComponent<RectTransform>();
            sr.content = txt.rectTransform;
            sr.horizontal = false;
            sr.vertical = true;
            sr.movementType = ScrollRect.MovementType.Clamped;
            return txt;
        }

        /// <summary>
        /// Builds `+survey/accept &lt;id&gt;` (the preview) or `+survey/accept &lt;id&gt; go` (the
        /// confirm). Returns null when the id is empty, so the caller sends nothing.
        /// </summary>
        public static string BuildSurveyAcceptCommand(string rawId, bool confirm)
        {
            return BuildJobCommand("+survey/accept", rawId, confirm);
        }

        /// <summary>
        /// Builds `verb &lt;id&gt;` or `verb &lt;id&gt; go` on the typed job id; null when the id is empty,
        /// so a button never sends a placeholder id the server does not know.
        /// </summary>
        public static string BuildJobCommand(string verb, string rawId, bool go)
        {
            if (string.IsNullOrWhiteSpace(rawId)) return null;
            string id = rawId.Trim();
            return go ? $"{verb} {id} go" : $"{verb} {id}";
        }

        private void EmitJobCommand(string verb, bool go)
        {
            string cmd = BuildJobCommand(verb, _jobIdInput != null ? _jobIdInput.text : null, go);
            if (cmd == null)
            {
                DisarmConfirm();
                if (_jobsOutputText != null)
                    _jobsOutputText.text = "Type a job number in the Job # box first (list them with the job filters).";
                return;
            }
            _emitCommand?.Invoke(cmd);
        }

        private void EmitSurveyAccept(bool confirm)
        {
            string cmd = BuildSurveyAcceptCommand(ResolveSurveyId(), confirm);
            if (cmd == null)
            {
                DisarmConfirm();
                if (_jobsOutputText != null)
                    _jobsOutputText.text = "Type a survey ID in the Survey ID box first (list them with the Survey filter).";
                return;
            }
            _emitCommand?.Invoke(cmd);
        }

        private InputField CreateMiniInput(Transform parent, string placeholder, string defaultVal)
        {
            var go = CreatePanel(parent, "Input_" + placeholder, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.08f, 0.13f, 0.20f, 0.95f));
            var le = go.AddComponent<LayoutElement>();
            le.preferredWidth = 76f;
            le.flexibleWidth = 0.35f;

            var input = go.AddComponent<InputField>();
            var ph = CreateText(go.transform, placeholder, 12, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.5f, 0.62f, 0.72f));
            SetRect(ph.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            var val = CreateText(go.transform, defaultVal, 12, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
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
