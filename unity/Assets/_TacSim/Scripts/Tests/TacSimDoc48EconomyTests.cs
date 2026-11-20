using System.Collections.Generic;
using NUnit.Framework;
using TacSim.Core;
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

        [Test]
        public void MarketHistoryReply_KeepsHeadingAndAll24Weeks_AndResetsOnNextHeading()
        {
            var reply = new MarketHistoryReply();
            Assert.IsFalse(reply.Ingest("Ceres Freeport exchange is open."), "a line before any heading is not history");
            Assert.AreEqual(0, reply.Lines.Count);

            // Heading plus 24 week rows: 25 lines, one more than the shared echo log holds.
            Assert.IsTrue(reply.Ingest("<color=#BBD8EE>House prices of Volatiles here, by IC week, newest first (24):</color>"));
            Assert.IsTrue(reply.Ingest("  this IC week: ask 102.00 cr, bid 90.00 cr"));
            Assert.IsTrue(reply.Ingest("  last IC week: ask 101.00 cr, bid 89.00 cr"));
            for (int w = 2; w < 24; w++)
                Assert.IsTrue(reply.Ingest($"  {w} IC weeks ago: ask {100 - w}.00 cr, bid none"));
            Assert.AreEqual(25, reply.Lines.Count);

            var parsed = TacSimEconomyPanels.ParseMarketHistoryLines(reply.Lines);
            Assert.AreEqual("Volatiles", parsed.GoodLabel);
            Assert.AreEqual(24, parsed.RowsNewestFirst.Count);
            Assert.AreEqual("102.00 cr", parsed.LatestAsk);

            // Any other line ends the capture; a stray row after it is not appended.
            Assert.IsFalse(reply.Ingest("You have 41250.00 cr."));
            Assert.IsFalse(reply.Ingest("  this IC week: ask 1.00 cr, bid 1.00 cr"));
            Assert.AreEqual(25, reply.Lines.Count);

            // A second good's heading drops the first good's rows.
            Assert.IsTrue(reply.Ingest("House prices of Structural Alloys here, by IC week, newest first (1):"));
            Assert.IsTrue(reply.Ingest("  this IC week: ask 420.00 cr, bid 360.00 cr"));
            parsed = TacSimEconomyPanels.ParseMarketHistoryLines(reply.Lines);
            Assert.AreEqual("Structural Alloys", parsed.GoodLabel);
            Assert.AreEqual(1, parsed.RowsNewestFirst.Count);
            Assert.AreEqual("420.00 cr", parsed.LatestAsk);
        }

        [Test]
        public void PortTradeFeed_DecodesHelmHullId_ForEscortPost()
        {
            Assert.AreEqual("#123", TacSimTradeAndShipState.NormalizeHullRef("#123"));
            Assert.AreEqual("#123", TacSimTradeAndShipState.NormalizeHullRef(" 123 "));
            Assert.AreEqual(string.Empty, TacSimTradeAndShipState.NormalizeHullRef("ship"));
            Assert.AreEqual(string.Empty, TacSimTradeAndShipState.NormalizeHullRef("#-4"));
            Assert.AreEqual(string.Empty, TacSimTradeAndShipState.NormalizeHullRef(null));

            var trade = new TacSimTradeAndShipState();
            Assert.AreEqual(string.Empty, trade.ShipHullId);

            trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                ""enabled"": true,
                ""venue"": ""ceres"",
                ""ship"": { ""hull_id"": ""#123"", ""hull_name"": ""Salt Moth"" }
            }"));
            Assert.AreEqual("#123", trade.ShipHullId);

            // No helm hull: the server sends `ship: null`, and the id is cleared.
            trade.ApplyPortTradeFeed(JsonNode.Parse(@"{ ""enabled"": true, ""venue"": ""ceres"", ""ship"": null }"));
            Assert.AreEqual(string.Empty, trade.ShipHullId);
        }

        [Test]
        public void G7_CorridorDestinations_AreExactlyTheLanesTwelveVenueWords()
        {
            var expected = new[]
            {
                "ceres", "stickney", "luna", "deimos", "titan", "commons",
                "kerwan", "ahuna", "juling", "occator", "rheasilvia", "urvara",
            };
            var ports = TacSimHudController.CorridorDestinations;
            Assert.AreEqual(expected.Length, ports.Length);
            for (int i = 0; i < expected.Length; i++)
            {
                Assert.AreEqual(expected[i], ports[i].code);
                Assert.IsFalse(string.IsNullOrWhiteSpace(ports[i].label));
            }
            Assert.AreEqual("Aitken Highport (Luna)", ports[TacSimHudController.CorridorDestinationIndex("luna")].label);
            Assert.AreEqual(-1, TacSimHudController.CorridorDestinationIndex("aitken"));
        }

        [Test]
        public void G6_SurveyAcceptCommand_RefusesEmptyId_AndAppendsGoOnConfirm()
        {
            Assert.IsNull(TacSimEconomyPanels.BuildSurveyAcceptCommand(null, confirm: false));
            Assert.IsNull(TacSimEconomyPanels.BuildSurveyAcceptCommand("   ", confirm: true));
            Assert.AreEqual("+survey/accept 7", TacSimEconomyPanels.BuildSurveyAcceptCommand(" 7 ", confirm: false));
            Assert.AreEqual("+survey/accept 7 go", TacSimEconomyPanels.BuildSurveyAcceptCommand("7", confirm: true));
        }
    }
}
