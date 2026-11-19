using System.Collections.Generic;
using NUnit.Framework;
using TacSim.UI;
using TacSim.World;
using UnityEngine;

namespace TacSim.Tests
{
    public sealed class TacSimDoc48EconomyTests
    {
        [Test]
        public void Track6_Doc48_MarketHistorySparklines_AndFiveTabPortTerminal()
        {
            // 1. Parse canonical 24-week `+market/history` lines (including rich-text color tags)
            var lines = new[]
            {
                "<color=#BBD8EE>House prices of Structural Alloys here, by IC week, newest first (4):</color>",
                "<color=#BBD8EE>  this IC week: ask 420.00 cr, bid 360.00 cr</color>",
                "<color=#BBD8EE>  last IC week: ask 400.00 cr, bid 345.00 cr</color>",
                "<color=#BBD8EE>  2 IC weeks ago: ask 380.00 cr, bid 330.00 cr</color>",
                "<color=#BBD8EE>  3 IC weeks ago: ask 370.00 cr, bid none</color>",
            };

            var parsed = TacSimEconomyPanels.ParseMarketHistoryLines(lines);
            Assert.AreEqual("Structural Alloys", parsed.GoodLabel);
            Assert.AreEqual(4, parsed.RowsNewestFirst.Count);
            Assert.AreEqual("420.00 cr", parsed.LatestAsk);
            Assert.AreEqual("360.00 cr", parsed.LatestBid);
            Assert.AreEqual("370.00 - 420.00", parsed.AskRange);
            Assert.AreEqual("330.00 - 360.00", parsed.BidRange);
            Assert.AreEqual(4, parsed.AskSparkline.Length);
            StringAssert.Contains("·", parsed.BidSparkline);

            // 2. Verify 5-tab Port Terminal (`Exchange`, `History`, `Jobs`, `Industry`, `Sailing`) & command dispatch
            var rootGo = new GameObject("TestDoc48Hud");
            try
            {
                var hud = rootGo.AddComponent<TacSimHudController>();
                hud.BuildHud();
                hud.BindTradeState(new TacSimTradeAndShipState());

                var emitted = new List<string>();
                hud.OnCommandSubmitted += emitted.Add;

                hud.OpenTradeAndSailingModal("FREEPORT COMMODITY EXCHANGE", openSailingTab: false);
                Assert.AreEqual(TradeTerminalTab.Exchange, hud.ActiveTradeTab);

                hud.SwitchTradeTerminalTab(TradeTerminalTab.History, autoQuery: true);
                Assert.AreEqual(TradeTerminalTab.History, hud.ActiveTradeTab);
                Assert.IsTrue(hud.EconomyPanels.HistoryTabBody.activeSelf);
                CollectionAssert.Contains(emitted, "+market/history tacsim:volatiles");

                foreach (string ln in lines)
                    hud.AppendModalResponseLine(ln);
                Assert.AreEqual("Structural Alloys", hud.EconomyPanels.LatestHistory.GoodLabel);

                hud.SwitchTradeTerminalTab(TradeTerminalTab.Jobs, autoQuery: true);
                Assert.AreEqual(TradeTerminalTab.Jobs, hud.ActiveTradeTab);
                Assert.IsTrue(hud.EconomyPanels.JobsTabBody.activeSelf);
                CollectionAssert.Contains(emitted, "+market/jobs");

                hud.SwitchTradeTerminalTab(TradeTerminalTab.Industry, autoQuery: true);
                Assert.AreEqual(TradeTerminalTab.Industry, hud.ActiveTradeTab);
                Assert.IsTrue(hud.EconomyPanels.IndustryTabBody.activeSelf);
                CollectionAssert.Contains(emitted, "+works");
            }
            finally
            {
                Object.DestroyImmediate(rootGo);
            }
        }
    }
}
