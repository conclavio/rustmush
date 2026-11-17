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
        public string UnitPriceText;
        public decimal UnitPriceMajor;
        public string Venue;
        public string VendorName;
        public bool IsWantedBid;
        public bool IsFunded = true;
    }

    /// <summary>
    /// Authoritative client trade, credit, hold-gauge, and sailing-plan state for the
    /// Visual Commodity Exchange, Dual Mass/Volume Hold Bars, and Sailing Plan Builder.
    /// Consumes structured `port.trade` panel keyframes and `ambient.tacsim.stowage` first,
    /// with seamless fallback to `+market/list` / `+ship` / `+credit` line parsing for older servers.
    /// </summary>
    public sealed class TacSimTradeAndShipState
    {
        // Fallback line parsers for older mushd builds without `port.trade`
        private static readonly Regex ListingRegex = new Regex(
            @"^#(\d+)\s+(.+?)\s+[—\-]+\s+(wanted\s+)?(\d+)\s+[×x]\s+([^\s\[]+)\s+\[([^\]]+)\](?:\s+(?:at|by)\s+(.+?))?(\s+[—\-]+\s+not buying)?(\s+\(hidden\))?\s*$",
            RegexOptions.Compiled | RegexOptions.CultureInvariant);

        private static readonly Regex QuoteRegex = new Regex(
            @"^Quote\s+#(\d+)\b",
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

        private static readonly Regex CreditAmountRegex = new Regex(
            @"(\d[\d,]*\.\d{2}\s*cr|\d[\d,]*\s*cr)\b",
            RegexOptions.Compiled | RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        public List<GoodItemDef> GoodsCatalog { get; private set; }
        public List<MarketListingRow> Listings { get; } = new List<MarketListingRow>();
        public List<string> SailingOrderLines { get; } = new List<string>();

        public int SelectedTradeQuantity { get; set; } = 10;
        public bool SelectedSellAll { get; set; }
        public string SelectedDestinationPort { get; set; } = "stickney";

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

        // Pending 1-click buy quote -> accept state
        private ulong? _pendingAutoAcceptListingId;
        private float _pendingAutoAcceptExpiresAt;
        private bool _clearListingsOnNextRow;

        public event Action OnStateChanged;

        public TacSimTradeAndShipState(List<GoodItemDef> goodsCatalog = null)
        {
            GoodsCatalog = goodsCatalog ?? TacSimContentLoader.LoadGoodsTable();
        }

        public void ReloadGoodsCatalog()
        {
            GoodsCatalog = TacSimContentLoader.LoadGoodsTable();
        }

        public void NotifyMarketRefreshRequested()
        {
            _clearListingsOnNextRow = true;
        }

        public string BeginOneClickBuy(ulong listingId, int quantity, float currentTimeS)
        {
            int safeQty = Mathf.Max(1, quantity);
            _pendingAutoAcceptListingId = listingId;
            _pendingAutoAcceptExpiresAt = currentTimeS + 8.0f;
            return $"+market/quote #{listingId}={safeQty}";
        }

        public static string FormatOneClickSellCommand(ulong listingId, int quantity, bool sellAll)
        {
            if (sellAll)
                return $"+market/sell {listingId}=all";
            return $"+market/sell {listingId}={Mathf.Max(1, quantity)}";
        }

        /// <summary>
        /// Hydrates credits, market asks/bids, and own-ship stowage directly from structured `port.trade` JSON!
        /// </summary>
        public void ApplyPortTradeFeed(JsonNode data)
        {
            if (data == null || !data.IsObject || !data.Bool("enabled", false))
                return;

            string balTxt = data.Str("balance_text", string.Empty);
            if (!string.IsNullOrEmpty(balTxt))
                CreditsText = balTxt;

            string venue = data.Str("venue", string.Empty);
            var listArr = data["listings"].AsArray();
            if (listArr != null)
            {
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
                    string priceTxt = l.Str("price_text", "0.00cr");
                    string vendor = l.Str("vendor_name", string.Empty);
                    bool isBid = string.Equals(side, "bid", StringComparison.OrdinalIgnoreCase);

                    Listings.Add(new MarketListingRow
                    {
                        ListingId = id,
                        ItemName = goodLabel,
                        GoodCode = goodCode,
                        Quantity = qty,
                        UnitPriceText = priceTxt,
                        UnitPriceMajor = ParseDecimalMoney(priceTxt),
                        Venue = venue,
                        VendorName = vendor,
                        IsWantedBid = isBid,
                        IsFunded = qty > 0
                    });
                }
            }

            var shipObj = data["ship"];
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
                return Mathf.Max(1, TankLimitTonnes - TankTonnes);
            if (g.Stowage == "stow.provisions")
                return Mathf.Max(1, StoresLimitCrewDays - StoresCrewDays);

            int freeKg = Mathf.Max(0, (HoldMassLimitTonnes - HoldMassTonnes) * 1000);
            int freeL = Mathf.Max(0, Mathf.RoundToInt((HoldVolumeLimitM3 - HoldVolumeM3) * 1000f));

            int byMass = g.MassKgPerUnit > 0 ? freeKg / g.MassKgPerUnit : 999;
            int byVol = g.VolumeLPerUnit > 0 ? freeL / g.VolumeLPerUnit : 999;
            return Mathf.Max(1, Mathf.Min(byMass, byVol));
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
                var row = new MarketListingRow
                {
                    ListingId = lid,
                    ItemName = itemName,
                    GoodCode = good != null ? good.Id : itemName,
                    Quantity = qty,
                    UnitPriceText = priceTxt,
                    UnitPriceMajor = ParseDecimalMoney(priceTxt),
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

            Match qm = QuoteRegex.Match(line);
            if (qm.Success && ulong.TryParse(qm.Groups[1].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out ulong quoteId))
            {
                if (_pendingAutoAcceptListingId.HasValue && currentTimeS <= _pendingAutoAcceptExpiresAt)
                {
                    _pendingAutoAcceptListingId = null;
                    _clearListingsOnNextRow = true;
                    followUpCommands = new List<string>
                    {
                        $"+market/accept #{quoteId}",
                        "+ship",
                        "+credit",
                        "+market/list"
                    };
                    return true;
                }
            }

            if (line.StartsWith("Agreed. Contract #", StringComparison.OrdinalIgnoreCase) ||
                line.IndexOf("your payment is escrowed", StringComparison.OrdinalIgnoreCase) >= 0 ||
                line.IndexOf("reaches you once the goods have moved", StringComparison.OrdinalIgnoreCase) >= 0)
            {
                _clearListingsOnNextRow = true;
                followUpCommands = new List<string> { "+ship", "+credit", "+market/list" };
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

            if (!lm.Success && !qm.Success &&
                (line.IndexOf("available", StringComparison.OrdinalIgnoreCase) >= 0 ||
                 line.IndexOf("balance", StringComparison.OrdinalIgnoreCase) >= 0 ||
                 line.StartsWith("Cash:", StringComparison.OrdinalIgnoreCase) ||
                 line.StartsWith("Account", StringComparison.OrdinalIgnoreCase) ||
                 line.IndexOf("credit", StringComparison.OrdinalIgnoreCase) >= 0))
            {
                Match cm = CreditAmountRegex.Match(line);
                if (cm.Success)
                {
                    CreditsText = cm.Groups[1].Value.Replace(" ", "");
                    mutated = true;
                }
            }

            if (Regex.IsMatch(line, @"^\d+\.\s+(?:buy|sell|depart|stop|await)\b", RegexOptions.IgnoreCase))
            {
                if (SailingOrderLines.Count >= 16) SailingOrderLines.RemoveAt(0);
                SailingOrderLines.Add(line);
                mutated = true;
            }
            else if (line.IndexOf("Sailing plan cleared", StringComparison.OrdinalIgnoreCase) >= 0 ||
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
