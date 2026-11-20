using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.RegularExpressions;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;

namespace TacSim.World
{
    [Serializable]
    public sealed class MarketListingRow
    {
        public ulong ListingId;
        public string ItemName;
        public string GoodCode;
        public int Quantity;
        public long UnitPriceMinor;
        public string UnitPriceText;
        public decimal UnitPriceMajor;
        public string Venue;
        public string VendorName;
        public bool IsPersonal;
        public bool IsWantedBid;
        public bool IsFunded = true;
    }

    /// <summary>
    /// Authoritative client trade, credit, hold-gauge, and sailing-plan state for the
    /// Visual Commodity Exchange, Dual Mass/Volume Hold Bars, and Sailing Plan Builder.
    /// Consumes structured `port.trade` panel keyframes (with post-spread pricing, pagination,
    /// and correlated `open_quotes` verification before `+market/accept`), with safe fallback
    /// line parsing for older servers.
    /// </summary>
    public sealed class TacSimTradeAndShipState
    {
        private static readonly Regex ListingRegex = new Regex(
            @"^#(\d+)\s+(.+?)\s+[—\-]+\s+(wanted\s+)?(\d+)\s+[×x]\s+([^\s\[]+)\s+\[([^\]]+)\](?:\s+(?:at|by)\s+(.+?))?(\s+[—\-]+\s+not buying)?(\s+\(hidden\))?\s*$",
            RegexOptions.Compiled | RegexOptions.CultureInvariant);

        private static readonly Regex ShipHoldSummaryRegex = new Regex(
            @"cargo\s+(\d+)\s*/\s*(\d+)\s*t\s*[·|]\s*([\d.]+)\s*/\s*(\d+)\s*m(?:³|3)",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        private static readonly Regex TankPairRegex = new Regex(
            @"(?:tank|TANKAGE)\s+(\d+)\s*/\s*(\d+)\s*t|reaction mass,\s*(\d+)\s+of\s+(\d+)\s*t",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        private static readonly Regex LockerPairRegex = new Regex(
            @"(?:locker|STORES)\s+(\d+)\s*/\s*(\d+)\s*crew-days|voyage provisions,\s*(\d+)\s+of\s+(\d+)\s*crew-days",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        private static readonly Regex LoadCellTotalRegex = new Regex(
            @"^\s*TOTAL:\s*(\d+)\s*t\s*[·|]\s*([\d.]+)\s*m(?:³|3)",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        private static readonly Regex LoadCellLimitRegex = new Regex(
            @"^\s*LIMIT:\s*(\d+)\s*t\s*[·|]\s*(\d+)\s*m(?:³|3)",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        private static readonly Regex AvailableCreditRegex = new Regex(
            @"(\d[\d,]*\.\d{2}\s*cr|\d[\d,]*\s*cr)\s+available\b",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        private static readonly Regex CreditAmountRegex = new Regex(
            @"(\d[\d,]*\.\d{2}\s*cr|\d[\d,]*\s*cr)\b",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        public List<GoodItemDef> GoodsCatalog { get; private set; }
        public List<MarketListingRow> Listings { get; } = new List<MarketListingRow>();
        public List<string> SailingOrderLines { get; } = new List<string>();
        public List<TacSimDockedHullRef> BerthDockedHulls { get; } = new List<TacSimDockedHullRef>();
        public bool HasAuthoritativeBerthFeed { get; private set; }

        public int SelectedTradeQuantity { get; set; } = 10;
        public bool SelectedSellAll { get; set; }
        public string SelectedDestinationPort { get; set; } = "stickney";

        public string ActiveVenue { get; private set; } = string.Empty;
        public string AboardDockedPortHint { get; private set; } = string.Empty;
        public ulong? NextAfterListingId { get; private set; }

        // Live hull capacity gauges
        public int HoldMassTonnes { get; private set; }
        public int HoldMassLimitTonnes { get; private set; } = 100;
        public float HoldVolumeM3 { get; private set; }
        public float HoldVolumeLimitM3 { get; private set; } = 120f;
        public int TankTonnes { get; private set; }
        public int TankLimitTonnes { get; private set; } = 80;
        public int StoresCrewDays { get; private set; }
        public int StoresLimitCrewDays { get; private set; } = 120;
        public string CreditsText { get; private set; } = "— cr";

        /// <summary>
        /// The helm hull as `#&lt;dbref&gt;` (the `port.trade` frame's `ship.hull_id`), or empty with no helm hull.
        /// `+escort/post` takes exactly this word.
        /// </summary>
        public string ShipHullId { get; private set; } = string.Empty;

        // Pending correlated 1-click buy intent (matched strictly against structured open_quotes)
        private ulong? _pendingBuyListingId;
        private int _pendingBuyQuantity;
        private long _pendingBuyMaxTotalMinor;
        private float _pendingBuyExpiresAt;
        private bool _clearListingsOnNextRow;
        private readonly HashSet<ulong> _knownOpenQuoteIds = new HashSet<ulong>();
        private readonly HashSet<ulong> _excludedQuoteIdsForPendingBuy = new HashSet<ulong>();

        public event Action OnStateChanged;

        public TacSimTradeAndShipState(List<GoodItemDef> goodsCatalog = null)
        {
            GoodsCatalog = goodsCatalog ?? TacSimContentLoader.LoadGoodsTable();
        }

        public void ResetSessionState()
        {
            Listings.Clear();
            SailingOrderLines.Clear();
            BerthDockedHulls.Clear();
            HasAuthoritativeBerthFeed = false;
            ActiveVenue = string.Empty;
            AboardDockedPortHint = string.Empty;
            NextAfterListingId = null;
            HoldMassTonnes = 0;
            HoldMassLimitTonnes = 100;
            HoldVolumeM3 = 0f;
            HoldVolumeLimitM3 = 120f;
            TankTonnes = 0;
            TankLimitTonnes = 80;
            StoresCrewDays = 0;
            StoresLimitCrewDays = 120;
            CreditsText = "— cr";
            ShipHullId = string.Empty;
            _clearListingsOnNextRow = false;
            _knownOpenQuoteIds.Clear();
            CancelPendingBuy();
            OnStateChanged?.Invoke();
        }

        public void ReloadGoodsCatalog()
        {
            GoodsCatalog = TacSimContentLoader.LoadGoodsTable();
            OnStateChanged?.Invoke();
        }

        public void NotifyMarketRefreshRequested()
        {
            _clearListingsOnNextRow = true;
        }

        public string BeginOneClickBuy(ulong listingId, int quantity, float currentTimeS, long displayedUnitPriceMinor = 0)
        {
            int safeQty = Mathf.Max(1, quantity);
            if (displayedUnitPriceMinor <= 0)
            {
                var found = Listings.Find(r => r.ListingId == listingId);
                if (found != null)
                {
                    displayedUnitPriceMinor = found.UnitPriceMinor > 0
                        ? found.UnitPriceMinor
                        : (long)Math.Round(found.UnitPriceMajor * 100m);
                }
            }

            // Never arm an unbounded auto-accept without a positive displayed price ceiling!
            if (displayedUnitPriceMinor <= 0)
            {
                CancelPendingBuy();
                return null;
            }

            _pendingBuyListingId = listingId;
            _pendingBuyQuantity = safeQty;
            try
            {
                _pendingBuyMaxTotalMinor = checked(displayedUnitPriceMinor * safeQty);
            }
            catch (OverflowException)
            {
                _pendingBuyMaxTotalMinor = long.MaxValue;
            }
            _pendingBuyExpiresAt = currentTimeS + 8.0f;
            // Snapshot already-open quote IDs so an unrelated pre-existing open quote is NEVER auto-accepted!
            _excludedQuoteIdsForPendingBuy.Clear();
            foreach (ulong qid in _knownOpenQuoteIds)
                _excludedQuoteIdsForPendingBuy.Add(qid);

            // `mush-market` reads quantity from the `=` right-hand side (`+market/quote #<id>=<qty>`)
            return $"+market/quote #{listingId}={safeQty}";
        }

        public void CancelPendingBuy()
        {
            _pendingBuyListingId = null;
            _pendingBuyQuantity = 0;
            _pendingBuyMaxTotalMinor = 0;
            _excludedQuoteIdsForPendingBuy.Clear();
        }

        /// <summary>
        /// Correlates a pending 1-click buy against structured `open_quotes` in `port.trade`.
        /// Only emits `+market/accept #&lt;quote_id&gt;` when `listing_id`, `quantity`, AND
        /// `total_minor &lt;= expectedMaxTotalMinor` all agree. If the seller's price moved up,
        /// cancels auto-accept and returns a warning so the player explicitly decides.
        /// </summary>
        public bool TryResolvePendingQuoteFromFeed(
            JsonNode data,
            float currentTimeS,
            out string acceptCommand,
            out string priceMovedWarning)
        {
            acceptCommand = null;
            priceMovedWarning = null;

            if (!_pendingBuyListingId.HasValue)
                return false;

            if (currentTimeS > _pendingBuyExpiresAt)
            {
                CancelPendingBuy();
                return false;
            }

            if (data == null || !data.IsObject)
                return false;

            var quotes = data["open_quotes"].AsArray();
            if (quotes == null || quotes.Count == 0)
                return false;

            ulong wantedListing = _pendingBuyListingId.Value;
            int wantedQty = _pendingBuyQuantity;
            long maxTotal = _pendingBuyMaxTotalMinor;

            // Scan newest quote first, skipping any quote_id that already existed before BeginOneClickBuy.
            for (int i = quotes.Count - 1; i >= 0; i--)
            {
                var q = quotes[i];
                if (!q.IsObject) continue;
                ulong qid = q.ULong("quote_id", 0);
                ulong lid = q.ULong("listing_id", 0);
                int qty = q.Int("quantity", 0);
                long totalMinor = q.Long("total_minor", -1);
                if (qid == 0 || lid != wantedListing || qty != wantedQty || totalMinor < 0)
                    continue;
                if (_excludedQuoteIdsForPendingBuy.Contains(qid))
                    continue;

                CancelPendingBuy();
                if (maxTotal <= 0 || totalMinor > maxTotal)
                {
                    string totalTxt = q.Str("total_text", $"{(totalMinor / 100.0):0.00}cr");
                    priceMovedWarning =
                        $"Quote #{qid} for listing #{lid} ({qty} units) priced at {totalTxt}, above displayed {(Math.Max(0L, maxTotal) / 100.0):0.00}cr — auto-accept held. Run `+market/accept #{qid}` to confirm.";
                    return true;
                }

                acceptCommand = $"+market/accept #{qid}";
                return true;
            }
            return false;
        }

        public static string FormatOneClickSellCommand(ulong listingId, int quantity, bool sellAll)
        {
            if (sellAll)
                return $"+market/sell {listingId}=all";
            return $"+market/sell {listingId}={Mathf.Max(1, quantity)}";
        }

        /// <summary>
        /// Hydrates credits, paginated market asks/bids, and own-ship stowage from structured `port.trade` JSON.
        /// </summary>
        public void ApplyPortTradeFeed(JsonNode data, bool appendPage = false)
        {
            if (data == null || !data.IsObject || !data.Bool("enabled", false))
                return;

            string balTxt = data.Str("balance_text", string.Empty);
            if (!string.IsNullOrEmpty(balTxt))
                CreditsText = balTxt;

            ActiveVenue = data.Str("venue", string.Empty);
            AboardDockedPortHint = data.Str("aboard_docked_port", string.Empty);

            var nextNode = data["next_after"];
            NextAfterListingId = (nextNode != null && !nextNode.IsNull && nextNode.AsLong(0) > 0)
                ? (ulong?)((ulong)nextNode.AsLong(0))
                : null;

            var listArr = data["listings"].AsArray();
            if (listArr != null)
            {
                if (!appendPage)
                    Listings.Clear();
                _clearListingsOnNextRow = false;

                for (int i = 0; i < listArr.Count; i++)
                {
                    var l = listArr[i];
                    if (!l.IsObject) continue;
                    ulong id = (ulong)Math.Max(0L, l.Long("id", 0));
                    if (id == 0) continue;
                    string side = l.Str("side", "ask");
                    string goodCode = l.Str("good", string.Empty);
                    string goodLabel = l.Str("good_label", goodCode);
                    int qty = l.Int("available_qty", 0);
                    long priceMinor = l.Long("unit_price_minor", 0);
                    string priceTxt = l.Str("price_text", "0.00cr");
                    string vendor = l.Str("vendor_name", string.Empty);
                    bool isBid = string.Equals(side, "bid", StringComparison.OrdinalIgnoreCase);
                    bool funded = l.ContainsKey("funded") ? l.Bool("funded", qty > 0) : qty > 0;

                    var row = new MarketListingRow
                    {
                        ListingId = id,
                        ItemName = goodLabel,
                        GoodCode = goodCode,
                        Quantity = qty,
                        UnitPriceMinor = priceMinor,
                        UnitPriceText = priceTxt,
                        UnitPriceMajor = priceMinor > 0 ? priceMinor / 100m : ParseDecimalMoney(priceTxt),
                        Venue = ActiveVenue,
                        VendorName = vendor,
                        IsPersonal = l.Bool("is_personal", false),
                        IsWantedBid = isBid,
                        IsFunded = funded
                    };

                    int existing = Listings.FindIndex(r => r.ListingId == id);
                    if (existing >= 0) Listings[existing] = row;
                    else Listings.Add(row);
                }
            }

            var openQuotesArr = data["open_quotes"].AsArray();
            if (openQuotesArr != null)
            {
                _knownOpenQuoteIds.Clear();
                for (int i = 0; i < openQuotesArr.Count; i++)
                {
                    var q = openQuotesArr[i];
                    if (!q.IsObject) continue;
                    ulong qid = q.ULong("quote_id", 0);
                    if (qid > 0) _knownOpenQuoteIds.Add(qid);
                }
            }

            var berthObj = data["berth"];
            if (berthObj.IsObject)
            {
                HasAuthoritativeBerthFeed = true;
                BerthDockedHulls.Clear();
                var bhArr = berthObj["docked_hulls"].AsArray();
                if (bhArr != null)
                {
                    for (int i = 0; i < bhArr.Count; i++)
                    {
                        var h = bhArr[i];
                        if (!h.IsObject) continue;
                        BerthDockedHulls.Add(new TacSimDockedHullRef
                        {
                            Id = h.Str("id", string.Empty),
                            Name = h.Str("name", string.Empty),
                            Mark = h.Str("mark", string.Empty),
                            Unit = h.Str("unit", string.Empty)
                        });
                    }
                }
            }

            var shipObj = data["ship"];
            ShipHullId = shipObj.IsObject ? NormalizeHullRef(shipObj.Str("hull_id", string.Empty)) : string.Empty;
            if (shipObj.IsObject && shipObj["stowage"].IsObject)
            {
                var st = shipObj["stowage"];
                ApplyRawStowageNumbers(
                    st.Long("cargo_kg", 0),
                    st.Long("cargo_limit_kg", 100000),
                    st.Long("cargo_l", 0),
                    st.Long("cargo_limit_l", 120000),
                    st.Long("tank_kg", 0),
                    st.Long("tank_limit_kg", 80000),
                    st.Long("stores_days", 0),
                    st.Long("stores_limit_days", 120),
                    notify: false);
            }

            OnStateChanged?.Invoke();
        }

        /// <summary>
        /// A hull reference as `#&lt;dbref&gt;` (what `ObjId` prints), or empty when it is not one.
        /// A bare number gains its `#`.
        /// </summary>
        public static string NormalizeHullRef(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw)) return string.Empty;
            string digits = raw.Trim();
            if (digits.StartsWith("#", StringComparison.Ordinal))
                digits = digits.Substring(1);
            if (!uint.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out uint id))
                return string.Empty;
            return "#" + id.ToString(CultureInfo.InvariantCulture);
        }

        public void ApplyStowageAmbient(TacSimStowageAmbient st)
        {
            if (st == null) return;
            ApplyRawStowageNumbers(
                st.CargoKg,
                st.CargoLimitKg,
                st.CargoL,
                st.CargoLimitL,
                st.TankKg,
                st.TankLimitKg,
                st.StoresDays,
                st.StoresLimitDays,
                notify: true);
        }

        private void ApplyRawStowageNumbers(
            long cargoKg, long cargoLimitKg,
            long cargoL, long cargoLimitL,
            long tankKg, long tankLimitKg,
            long storesDays, long storesLimitDays,
            bool notify)
        {
            HoldMassTonnes = (int)(Math.Max(0L, cargoKg) / 1000L);
            if (cargoLimitKg > 0) HoldMassLimitTonnes = Mathf.Max(1, (int)(cargoLimitKg / 1000L));
            HoldVolumeM3 = Mathf.Floor(Math.Max(0L, cargoL) / 100f) * 0.1f;
            if (cargoLimitL > 0) HoldVolumeLimitM3 = Mathf.Max(1f, cargoLimitL / 1000f);
            TankTonnes = (int)(Math.Max(0L, tankKg) / 1000L);
            if (tankLimitKg > 0) TankLimitTonnes = Mathf.Max(1, (int)(tankLimitKg / 1000L));
            StoresCrewDays = (int)Math.Max(0L, storesDays);
            if (storesLimitDays > 0) StoresLimitCrewDays = Mathf.Max(1, (int)storesLimitDays);

            if (notify)
                OnStateChanged?.Invoke();
        }

        public GoodItemDef FindGoodByNameOrCode(string nameOrCode)
        {
            if (string.IsNullOrWhiteSpace(nameOrCode) || GoodsCatalog == null)
                return null;
            string trimmed = nameOrCode.Trim();
            for (int i = 0; i < GoodsCatalog.Count; i++)
            {
                var g = GoodsCatalog[i];
                if (string.Equals(g.Id, trimmed, StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(g.Label, trimmed, StringComparison.OrdinalIgnoreCase))
                    return g;
            }
            for (int i = 0; i < GoodsCatalog.Count; i++)
            {
                var g = GoodsCatalog[i];
                if (!string.IsNullOrEmpty(g.Label) && g.Label.StartsWith(trimmed, StringComparison.OrdinalIgnoreCase))
                    return g;
            }
            return null;
        }

        public int ComputeMaxAdditionalUnitsForHold(string goodCode)
        {
            var g = FindGoodByNameOrCode(goodCode);
            if (g == null) return SelectedTradeQuantity;

            if (g.Stowage == "stow.propellant")
                return Mathf.Max(0, TankLimitTonnes - TankTonnes);
            if (g.Stowage == "stow.provisions")
                return Mathf.Max(0, StoresLimitCrewDays - StoresCrewDays);

            int freeKg = Mathf.Max(0, (HoldMassLimitTonnes - HoldMassTonnes) * 1000);
            int freeL = Mathf.Max(0, Mathf.RoundToInt((HoldVolumeLimitM3 - HoldVolumeM3) * 1000f));

            int byMass = g.MassKgPerUnit > 0 ? freeKg / g.MassKgPerUnit : 999;
            int byVol = g.VolumeLPerUnit > 0 ? freeL / g.VolumeLPerUnit : 999;
            return Mathf.Max(0, Mathf.Min(byMass, byVol));
        }

        public bool IngestServerLine(string rawLine, float currentTimeS, out List<string> followUpCommands)
        {
            followUpCommands = null;
            if (string.IsNullOrWhiteSpace(rawLine))
                return false;

            string line = rawLine.Trim();
            bool mutated = false;

            if (string.Equals(line, "Nothing is for sale.", StringComparison.OrdinalIgnoreCase))
            {
                Listings.Clear();
                _clearListingsOnNextRow = false;
                OnStateChanged?.Invoke();
                return true;
            }

            Match lm = ListingRegex.Match(line);
            if (lm.Success && ulong.TryParse(lm.Groups[1].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out ulong lid))
            {
                if (_clearListingsOnNextRow)
                {
                    Listings.Clear();
                    _clearListingsOnNextRow = false;
                }

                string itemName = lm.Groups[2].Value.Trim();
                bool isWanted = !string.IsNullOrWhiteSpace(lm.Groups[3].Value);
                int.TryParse(lm.Groups[4].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out int qty);
                string priceTxt = lm.Groups[5].Value.Trim();
                string venue = lm.Groups[6].Value.Trim();
                string vendor = lm.Groups[7].Success ? lm.Groups[7].Value.Trim() : string.Empty;
                bool notBuying = lm.Groups[8].Success && !string.IsNullOrWhiteSpace(lm.Groups[8].Value);

                var good = FindGoodByNameOrCode(itemName);
                decimal decPrice = ParseDecimalMoney(priceTxt);
                var row = new MarketListingRow
                {
                    ListingId = lid,
                    ItemName = itemName,
                    GoodCode = good != null ? good.Id : itemName,
                    Quantity = qty,
                    UnitPriceMinor = (long)Math.Round(decPrice * 100m),
                    UnitPriceText = priceTxt,
                    UnitPriceMajor = decPrice,
                    Venue = venue,
                    VendorName = vendor,
                    IsWantedBid = isWanted,
                    IsFunded = !notBuying && qty > 0
                };

                int existing = Listings.FindIndex(r => r.ListingId == lid);
                if (existing >= 0) Listings[existing] = row;
                else Listings.Add(row);

                OnStateChanged?.Invoke();
                return true;
            }

            // Note: Blind text-scraped auto-accept is intentionally omitted — purchase confirmation
            // correlates structured `open_quotes` via `TryResolvePendingQuoteFromFeed` so an
            // unrelated quote or moved price is never accepted blindly.

            if (line.StartsWith("Agreed. Contract #", StringComparison.OrdinalIgnoreCase) ||
                line.IndexOf("your payment is escrowed", StringComparison.OrdinalIgnoreCase) >= 0 ||
                line.IndexOf("reaches you once the goods have moved", StringComparison.OrdinalIgnoreCase) >= 0)
            {
                _clearListingsOnNextRow = true;
                followUpCommands = new List<string> { "ui:refresh_trade" };
            }

            Match hm = ShipHoldSummaryRegex.Match(line);
            if (hm.Success)
            {
                HoldMassTonnes = ParseIntSafe(hm.Groups[1].Value, HoldMassTonnes);
                HoldMassLimitTonnes = Mathf.Max(1, ParseIntSafe(hm.Groups[2].Value, HoldMassLimitTonnes));
                HoldVolumeM3 = ParseFloatSafe(hm.Groups[3].Value, HoldVolumeM3);
                HoldVolumeLimitM3 = Mathf.Max(1f, ParseFloatSafe(hm.Groups[4].Value, HoldVolumeLimitM3));
                mutated = true;
            }

            Match tm = TankPairRegex.Match(line);
            if (tm.Success)
            {
                string a = tm.Groups[1].Success && tm.Groups[1].Length > 0 ? tm.Groups[1].Value : tm.Groups[3].Value;
                string b = tm.Groups[2].Success && tm.Groups[2].Length > 0 ? tm.Groups[2].Value : tm.Groups[4].Value;
                TankTonnes = ParseIntSafe(a, TankTonnes);
                TankLimitTonnes = Mathf.Max(1, ParseIntSafe(b, TankLimitTonnes));
                mutated = true;
            }

            Match sm = LockerPairRegex.Match(line);
            if (sm.Success)
            {
                string a = sm.Groups[1].Success && sm.Groups[1].Length > 0 ? sm.Groups[1].Value : sm.Groups[3].Value;
                string b = sm.Groups[2].Success && sm.Groups[2].Length > 0 ? sm.Groups[2].Value : sm.Groups[4].Value;
                StoresCrewDays = ParseIntSafe(a, StoresCrewDays);
                StoresLimitCrewDays = Mathf.Max(1, ParseIntSafe(b, StoresLimitCrewDays));
                mutated = true;
            }

            Match totM = LoadCellTotalRegex.Match(line);
            if (totM.Success)
            {
                HoldMassTonnes = ParseIntSafe(totM.Groups[1].Value, HoldMassTonnes);
                HoldVolumeM3 = ParseFloatSafe(totM.Groups[2].Value, HoldVolumeM3);
                mutated = true;
            }

            Match limM = LoadCellLimitRegex.Match(line);
            if (limM.Success)
            {
                HoldMassLimitTonnes = Mathf.Max(1, ParseIntSafe(limM.Groups[1].Value, HoldMassLimitTonnes));
                HoldVolumeLimitM3 = Mathf.Max(1f, ParseFloatSafe(limM.Groups[2].Value, HoldVolumeLimitM3));
                mutated = true;
            }

            if (!lm.Success)
            {
                // Prefer `{available} available` in `+credit` ("Credit: 100.00cr posted, 30.00cr held, 70.00cr available.")
                Match availM = AvailableCreditRegex.Match(line);
                if (availM.Success)
                {
                    CreditsText = availM.Groups[1].Value.Replace(" ", "");
                    mutated = true;
                }
                else if (line.StartsWith("Cash:", StringComparison.OrdinalIgnoreCase) ||
                         line.IndexOf("balance", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    Match cm = CreditAmountRegex.Match(line);
                    if (cm.Success)
                    {
                        CreditsText = cm.Groups[1].Value.Replace(" ", "");
                        mutated = true;
                    }
                }
            }

            Match struckMatch = Regex.Match(line, @"^Order\s+(\d+)\s+is\s+struck\s+from\s+the\s+sailing\s+plan", RegexOptions.IgnoreCase);
            if (struckMatch.Success)
            {
                string idToken = struckMatch.Groups[1].Value;
                int removed = SailingOrderLines.RemoveAll(s =>
                    Regex.IsMatch(s, @"^\s*" + Regex.Escape(idToken) + @"[.:]\s") ||
                    s.StartsWith($"Order {idToken} ", StringComparison.OrdinalIgnoreCase));
                if (removed > 0)
                    mutated = true;
            }
            else if ((line.IndexOf("sailing plan (", StringComparison.OrdinalIgnoreCase) >= 0 &&
                      line.IndexOf("revision ", StringComparison.OrdinalIgnoreCase) >= 0) ||
                     line.StartsWith("The sailing plan from order ", StringComparison.OrdinalIgnoreCase))
            {
                // Fresh `+ship/orders` or `+ship/plan` header resets previous cached order rows
                SailingOrderLines.Clear();
                mutated = true;
            }
            else if (Regex.IsMatch(line, @"^\s*\d+[.:]\s*(?:buy|sell|depart|stop|await)\b", RegexOptions.IgnoreCase) ||
                     Regex.IsMatch(line, @"^\s*\d+\.\s+.+\s+—\s+", RegexOptions.IgnoreCase) ||
                     (line.StartsWith("Order ", StringComparison.OrdinalIgnoreCase) && line.IndexOf(" filed:", StringComparison.OrdinalIgnoreCase) > 0))
            {
                if (SailingOrderLines.Count >= 16) SailingOrderLines.RemoveAt(0);
                SailingOrderLines.Add(line);
                mutated = true;
            }
            else if (line.IndexOf("sailing plan is cleared", StringComparison.OrdinalIgnoreCase) >= 0 ||
                     line.IndexOf("Sailing plan cleared", StringComparison.OrdinalIgnoreCase) >= 0 ||
                     line.IndexOf("no sailing plan", StringComparison.OrdinalIgnoreCase) >= 0 ||
                     line.IndexOf("nothing left to do", StringComparison.OrdinalIgnoreCase) >= 0 ||
                     line.IndexOf("holds no orders", StringComparison.OrdinalIgnoreCase) >= 0 ||
                     line.IndexOf("has no orders", StringComparison.OrdinalIgnoreCase) >= 0 ||
                     line.IndexOf("No orders are filed", StringComparison.OrdinalIgnoreCase) >= 0)
            {
                SailingOrderLines.Clear();
                mutated = true;
            }

            if (mutated)
                OnStateChanged?.Invoke();

            return mutated;
        }

        private static decimal ParseDecimalMoney(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw)) return 0m;
            var cleaned = Regex.Replace(raw, @"[^\d.]", "");
            return decimal.TryParse(cleaned, NumberStyles.Float, CultureInfo.InvariantCulture, out decimal d) ? d : 0m;
        }

        private static int ParseIntSafe(string s, int fallback) =>
            int.TryParse(s, NumberStyles.Integer, CultureInfo.InvariantCulture, out int v) ? v : fallback;

        private static float ParseFloatSafe(string s, float fallback) =>
            float.TryParse(s, NumberStyles.Float, CultureInfo.InvariantCulture, out float v) ? v : fallback;
    }
}
