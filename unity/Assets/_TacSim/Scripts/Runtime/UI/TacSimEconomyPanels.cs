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

        private static readonly Regex RxHistoryHeader = new Regex(
            @"House prices of (.+?) here, by IC week, newest first \(\d+\):",
            RegexOptions.IgnoreCase | RegexOptions.Compiled);

        private static readonly Regex RxHistoryRow = new Regex(
            @"^\s*(this IC week|last IC week|\d+\s+IC weeks ago):\s*ask\s+([^,]+),\s*bid\s+(.+)$",
            RegexOptions.IgnoreCase | RegexOptions.Compiled);

        private static readonly Regex RxTagStrip = new Regex(@"<[^>]+>", RegexOptions.Compiled);
        private static readonly Regex RxDecimal = new Regex(@"(-?\d+(?:\.\d+)?)", RegexOptions.Compiled);

        private readonly Font _font;
        private readonly Action<string> _emitCommand;
        private readonly Func<string> _resolveSelectedPort;
        private readonly Func<string> _resolveActiveHullRef;

        public GameObject HistoryTabBody { get; private set; }
        public GameObject JobsTabBody { get; private set; }
        public GameObject IndustryTabBody { get; private set; }

        public string SelectedHistoryGood { get; private set; } = "volatiles";
        public ParsedPriceHistorySummary LatestHistory { get; private set; } = new ParsedPriceHistorySummary();

        private Text _historySparkText;
        private Text _historyOutputText;
        private Text _jobsOutputText;
        private Text _industryOutputText;
        private InputField _jobIdInput;
        private InputField _workOrderIdInput;
        private InputField _debtIdInput;
        private readonly List<Image> _goodBtnImages = new List<Image>();

        public bool IsAnyInputFocused =>
            (_jobIdInput != null && _jobIdInput.isFocused) ||
            (_workOrderIdInput != null && _workOrderIdInput.isFocused) ||
            (_debtIdInput != null && _debtIdInput.isFocused);

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
            if (echoLines == null) return;
            var parsed = ParseMarketHistoryLines(echoLines);
            if (parsed.RowsNewestFirst.Count > 0)
                LatestHistory = parsed;

            string joined = echoLines.Count > 0 ? string.Join("\n", echoLines) : "Ready.";
            if (_historySparkText != null)
            {
                if (LatestHistory.RowsNewestFirst.Count > 0)
                {
                    string lbl = string.IsNullOrEmpty(LatestHistory.GoodLabel) ? SelectedHistoryGood : LatestHistory.GoodLabel;
                    _historySparkText.text =
                        $"<color=#66DDFF><b>{MushProtocol.EscapeUnityRichText(lbl)} — 24-WEEK HOUSE SERIES</b></color>\n" +
                        $"ASK TREND: <color=#66EEFF><b>{LatestHistory.AskSparkline}</b></color>   Latest: <b>{MushProtocol.EscapeUnityRichText(LatestHistory.LatestAsk)}</b>  (Band {LatestHistory.AskRange})\n" +
                        $"BID TREND: <color=#FFCC66><b>{LatestHistory.BidSparkline}</b></color>   Latest: <b>{MushProtocol.EscapeUnityRichText(LatestHistory.LatestBid)}</b>  (Band {LatestHistory.BidRange})";
                }
                else
                {
                    _historySparkText.text = "Select a commodity above to fetch 24 IC-week house ask & bid trends (`+market/history`).";
                }
            }

            if (_historyOutputText != null) _historyOutputText.text = joined;
            if (_jobsOutputText != null) _jobsOutputText.text = joined;
            if (_industryOutputText != null) _industryOutputText.text = joined;
        }

        private void BuildHistoryTab(Transform parent)
        {
            HistoryTabBody = CreatePanel(parent, "HistoryTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -154f), Color.clear);

            var goodRow = CreateRow(HistoryTabBody.transform, "HistoryGoodsRow", -38f, 0f);
            (string code, string label)[] goods =
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
            _goodBtnImages.Clear();
            for (int i = 0; i < goods.Length; i++)
            {
                int idx = i;
                string code = goods[i].code;
                var b = CreateButton(goodRow, goods[i].label, new Color(0.14f, 0.28f, 0.40f), () =>
                {
                    SelectedHistoryGood = code;
                    HighlightGoodButtons(idx);
                    _emitCommand?.Invoke($"+market/history tacsim:{code}");
                });
                _goodBtnImages.Add(b.GetComponent<Image>());
            }
            HighlightGoodButtons(0);

            var actRow = CreateRow(HistoryTabBody.transform, "HistoryActionsRow", -82f, -44f);
            CreateButton(actRow, "24w Trend", new Color(0.16f, 0.46f, 0.56f), () => _emitCommand?.Invoke($"+market/history tacsim:{SelectedHistoryGood}"));
            CreateButton(actRow, "Public Depth", new Color(0.16f, 0.36f, 0.48f), () => _emitCommand?.Invoke($"+market/depth tacsim:{SelectedHistoryGood}"));
            CreateButton(actRow, "My Listings", new Color(0.20f, 0.34f, 0.44f), () => _emitCommand?.Invoke("+market/mine"));
            CreateButton(actRow, "Check Locker", new Color(0.18f, 0.42f, 0.36f), () => _emitCommand?.Invoke("+locker"));
            CreateButton(actRow, "Open Locker", new Color(0.16f, 0.50f, 0.38f), () => _emitCommand?.Invoke("+locker/open"));
            CreateButton(actRow, "Debts", new Color(0.46f, 0.28f, 0.16f), () => _emitCommand?.Invoke("+debts"));
            _debtIdInput = CreateMiniInput(actRow, "Debt #", "1");
            CreateButton(actRow, "Pay Debt", new Color(0.52f, 0.24f, 0.18f), () =>
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

            var actRow = CreateRow(JobsTabBody.transform, "JobsActRow", -84f, -44f);
            _jobIdInput = CreateMiniInput(actRow, "Job #", "1");
            CreateButton(actRow, "Accept Haul", new Color(0.16f, 0.48f, 0.36f), () =>
            {
                string j = _jobIdInput != null && !string.IsNullOrWhiteSpace(_jobIdInput.text) ? _jobIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+haul/accept {j} go");
            });
            CreateButton(actRow, "Deliver Haul", new Color(0.48f, 0.34f, 0.14f), () =>
            {
                string j = _jobIdInput != null && !string.IsNullOrWhiteSpace(_jobIdInput.text) ? _jobIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+haul/deliver {j} go");
            });
            CreateButton(actRow, "Accept Survey", new Color(0.18f, 0.38f, 0.50f), () =>
            {
                string j = _jobIdInput != null && !string.IsNullOrWhiteSpace(_jobIdInput.text) ? _jobIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+survey/accept {j}");
            });
            CreateButton(actRow, "Post 10t Haul", new Color(0.22f, 0.42f, 0.48f), () =>
            {
                string dest = _resolveSelectedPort != null ? _resolveSelectedPort() : "stickney";
                _emitCommand?.Invoke($"+haul/post 10 {SelectedHistoryGood} to={dest} fee=250 go");
            });
            CreateButton(actRow, "Post Escort", new Color(0.46f, 0.26f, 0.20f), () =>
            {
                string dest = _resolveSelectedPort != null ? _resolveSelectedPort() : "stickney";
                string hull = _resolveActiveHullRef != null ? _resolveActiveHullRef() : "ship";
                _emitCommand?.Invoke($"+escort/post {hull} to={dest} fee=300 go");
            });
            CreateButton(actRow, "My Crew", new Color(0.22f, 0.32f, 0.44f), () => _emitCommand?.Invoke("+sortie/contract"));

            _jobsOutputText = BuildScrollOutput(JobsTabBody.transform, "JobsViewport", -92f);
            JobsTabBody.SetActive(false);
        }

        private void BuildIndustryTab(Transform parent)
        {
            IndustryTabBody = CreatePanel(parent, "IndustryTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(16f, 14f), new Vector2(-16f, -154f), Color.clear);

            var topRow = CreateRow(IndustryTabBody.transform, "WorksStatusRow", -38f, 0f);
            CreateButton(topRow, "Works (+works)", new Color(0.16f, 0.44f, 0.54f), () => _emitCommand?.Invoke("+works"));
            CreateButton(topRow, "Shipyard (+yard)", new Color(0.18f, 0.38f, 0.50f), () => _emitCommand?.Invoke("+yard"));
            CreateButton(topRow, "Bay Status", new Color(0.18f, 0.42f, 0.36f), () =>
            {
                string hull = _resolveActiveHullRef != null ? _resolveActiveHullRef() : "ship";
                _emitCommand?.Invoke($"+bay {hull}");
            });
            CreateButton(topRow, "Rearm Magazines", new Color(0.50f, 0.30f, 0.14f), () =>
            {
                string hull = _resolveActiveHullRef != null ? _resolveActiveHullRef() : "ship";
                _emitCommand?.Invoke($"+bay/order {hull}=rearm");
            });
            CreateButton(topRow, "Full Repair", new Color(0.16f, 0.48f, 0.36f), () =>
            {
                string hull = _resolveActiveHullRef != null ? _resolveActiveHullRef() : "ship";
                _emitCommand?.Invoke($"+bay/order {hull}=all");
            });
            CreateButton(topRow, "Core Shop (+core)", new Color(0.32f, 0.26f, 0.48f), () => _emitCommand?.Invoke("+core"));
            CreateButton(topRow, "Facilities", new Color(0.22f, 0.32f, 0.42f), () => _emitCommand?.Invoke("@facility/list"));

            var recipeRow = CreateRow(IndustryTabBody.transform, "WorksRecipesRow", -82f, -44f);
            CreateButton(recipeRow, "Mill Metals", new Color(0.16f, 0.40f, 0.46f), () => _emitCommand?.Invoke("+works/order metals"));
            CreateButton(recipeRow, "Fittings Batch", new Color(0.16f, 0.40f, 0.46f), () => _emitCommand?.Invoke("+works/order fittings"));
            CreateButton(recipeRow, "30mm AP Box", new Color(0.42f, 0.28f, 0.16f), () => _emitCommand?.Invoke("+works/order ap_30"));
            CreateButton(recipeRow, "105mm HE", new Color(0.42f, 0.28f, 0.16f), () => _emitCommand?.Invoke("+works/order he_105"));
            CreateButton(recipeRow, "Order Dray", new Color(0.20f, 0.42f, 0.56f), () => _emitCommand?.Invoke("+yard/order dray"));
            CreateButton(recipeRow, "Order Dray-P", new Color(0.20f, 0.42f, 0.56f), () => _emitCommand?.Invoke("+yard/order dray_p"));
            CreateButton(recipeRow, "Order Line Tank", new Color(0.46f, 0.24f, 0.22f), () => _emitCommand?.Invoke("+yard/order line_tank"));

            var orderRow = CreateRow(IndustryTabBody.transform, "OrderEscrowRow", -126f, -88f);
            _workOrderIdInput = CreateMiniInput(orderRow, "Order #", "1");
            CreateButton(orderRow, "Buy Works Bill", new Color(0.16f, 0.46f, 0.38f), () =>
            {
                string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+works/buy {ord}");
            });
            CreateButton(orderRow, "Collect Works", new Color(0.48f, 0.34f, 0.14f), () =>
            {
                string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+works/collect {ord}");
            });
            CreateButton(orderRow, "Buy Yard Bill", new Color(0.16f, 0.46f, 0.54f), () =>
            {
                string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+yard/buy {ord}");
            });
            CreateButton(orderRow, "Commission Hull", new Color(0.50f, 0.32f, 0.16f), () =>
            {
                string ord = _workOrderIdInput != null && !string.IsNullOrWhiteSpace(_workOrderIdInput.text) ? _workOrderIdInput.text.Trim() : "1";
                _emitCommand?.Invoke($"+yard/collect {ord}");
            });

            _industryOutputText = BuildScrollOutput(IndustryTabBody.transform, "IndustryViewport", -134f);
            IndustryTabBody.SetActive(false);
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
