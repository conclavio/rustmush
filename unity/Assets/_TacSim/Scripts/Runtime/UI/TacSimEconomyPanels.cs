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

        private static readonly Regex RxHistoryHeader = new Regex(
            @"House prices of (.+?) here, by IC week, newest first \(\d+\):",
            RegexOptions.IgnoreCase | RegexOptions.Compiled);

        private static readonly Regex RxHistoryRow = new Regex(
            @"^\s*(this IC week|last IC week|\d+\s+IC weeks ago):\s*ask\s+([^,]+),\s*bid\s+(.+)$",
            RegexOptions.IgnoreCase | RegexOptions.Compiled);

        private static readonly Regex RxTagStrip = new Regex(@"<[^>]+>", RegexOptions.Compiled);
        private static readonly Regex RxDecimal = new Regex(@"(-?\d+(?:\.\d+)?)", RegexOptions.Compiled);
        private static readonly Regex RxHullDbref = new Regex(@"^#[1-9]\d*$", RegexOptions.Compiled);

        private readonly Font _font;
        private readonly Action<string> _emitCommand;
        private readonly Func<string> _resolveSelectedPort;
        private readonly Func<string> _resolveActiveHullRef;

        public GameObject HistoryTabBody { get; private set; }
        public GameObject JobsTabBody { get; private set; }
        public GameObject IndustryTabBody { get; private set; }

        public string SelectedHistoryGood { get; private set; } = "volatiles";
        public string SelectedHaulGood { get; private set; } = "volatiles";
        public string SelectedSurveyId { get; private set; } = "ahuna-a";
        public ParsedPriceHistorySummary LatestHistory { get; private set; } = new ParsedPriceHistorySummary();

        private int _selectedHaulGoodIndex;
        private int _selectedSurveyIndex;

        private Text _historySparkText;
        private Text _historyOutputText;
        private Text _jobsOutputText;
        private Text _industryOutputText;
        private Text _haulGoodBtnLabel;
        private InputField _jobIdInput;
        private InputField _surveyIdInput;
        private InputField _haulQtyInput;
        private InputField _haulFeeInput;
        private InputField _workOrderIdInput;
        private InputField _bayHullInput;
        private InputField _debtIdInput;
        private readonly List<Image> _goodBtnImages = new List<Image>();

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
            ClearHistorySeries();
            if (_historyOutputText != null) _historyOutputText.text = "Ready.";
            if (_jobsOutputText != null) _jobsOutputText.text = "Ready.";
            if (_industryOutputText != null) _industryOutputText.text = "Ready.";
            if (_jobIdInput != null) _jobIdInput.text = "1";
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
            Func<string> resolveActiveHullRef)
        {
            _font = font;
            _emitCommand = emitCommand;
            _resolveSelectedPort = resolveSelectedPort;
            _resolveActiveHullRef = resolveActiveHullRef;
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

                var parsed = ParseMarketHistoryLines(echoLines);
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
                else if (noHistoryReported)
                {
                    ClearHistorySeries($"No price history is kept here for <b>{MushProtocol.EscapeUnityRichText(SelectedHistoryGood)}</b>.");
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

            _historyOutputText = BuildScrollOutput(HistoryTabBody.transform, "HistoryViewport", -164f);
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
            _jobIdInput = CreateMiniInput(paramsRow, "Job #", "1");
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
                onFirstClickPreview: () =>
                {
                    string j = _jobIdInput != null && !string.IsNullOrWhiteSpace(_jobIdInput.text) ? _jobIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+haul/accept {j}");
                },
                onConfirmed: () =>
                {
                    string j = _jobIdInput != null && !string.IsNullOrWhiteSpace(_jobIdInput.text) ? _jobIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+haul/accept {j} go");
                });
            CreateConfirmButton(
                actRow,
                "deliver_haul",
                "Deliver Haul",
                "CONFIRM DELIV",
                new Color(0.48f, 0.34f, 0.14f),
                onFirstClickPreview: () =>
                {
                    string j = _jobIdInput != null && !string.IsNullOrWhiteSpace(_jobIdInput.text) ? _jobIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+haul/deliver {j}");
                },
                onConfirmed: () =>
                {
                    string j = _jobIdInput != null && !string.IsNullOrWhiteSpace(_jobIdInput.text) ? _jobIdInput.text.Trim() : "1";
                    _emitCommand?.Invoke($"+haul/deliver {j} go");
                });
            CreateConfirmButton(
                actRow,
                "accept_survey",
                "Accept Survey",
                "CONFIRM SURV",
                new Color(0.18f, 0.38f, 0.50f),
                onFirstClickPreview: () =>
                {
                    string sid = ResolveSurveyId();
                    _emitCommand?.Invoke($"+survey/accept {sid}");
                },
                onConfirmed: () =>
                {
                    string sid = ResolveSurveyId();
                    _emitCommand?.Invoke($"+survey/accept {sid} go");
                });
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

            _jobsOutputText = BuildScrollOutput(JobsTabBody.transform, "JobsViewport", -134f);
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

            _industryOutputText = BuildScrollOutput(IndustryTabBody.transform, "IndustryViewport", -134f);
            IndustryTabBody.SetActive(false);
        }

        private void NotifyIndustryConfirm(string command)
        {
            if (_industryOutputText != null)
                _industryOutputText.text = $"<color=#FFCC66>Click again to confirm order: <b>{MushProtocol.EscapeUnityRichText(command)}</b></color>";
        }

        private void HighlightGoodButtons(int selectedIdx)
        {
            for (int i = 0; i < _goodBtnImages.Count; i++)
            {
                if (_goodBtnImages[i] == null) continue;
                _goodBtnImages[i].color = i == selectedIdx
                    ? new Color(0.22f, 0.66f, 0.88f, 1f)
                    : new Color(0.14f, 0.26f, 0.38f, 0.92f);
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
            var vp = CreatePanel(parent, name, new Vector2(0f, 0f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, topInset), new Color(0.02f, 0.05f, 0.09f, 0.90f));
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
