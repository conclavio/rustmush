using System.Collections.Generic;
using NUnit.Framework;
using TacSim.Core;
using TacSim.Net;
using TacSim.UI;
using UnityEngine;

namespace TacSim.Tests
{
    public sealed class TacSimMindTerminalTests
    {
        private static List<JsonNode> DrainOutboundFrames(MushWsClient ws)
        {
            var list = new List<JsonNode>();
            while (ws.TryDequeueOutbound(out string raw))
                list.Add(JsonNode.Parse(raw));
            return list;
        }

        [Test]
        public void Protocol_BuildsCanonicalMindMarketQuery_AndMindInvokeEnvelopes()
        {
            var listings = JsonNode.Parse(MushProtocol.BuildMindMarketQueryCommand(7UL, "listings", afterListingCode: "ABCD1234"));
            Assert.AreEqual("req", listings.Str("t"));
            Assert.AreEqual(7L, listings["d"].Long("id"));
            Assert.AreEqual("mind_market", listings["d"]["q"].Str("t"));
            Assert.AreEqual("listings", listings["d"]["q"]["d"].Str("kind"));
            Assert.AreEqual("ABCD1234", listings["d"]["q"]["d"].Str("after"));

            var holdings = JsonNode.Parse(MushProtocol.BuildMindMarketQueryCommand(8UL, "holdings", section: "owner", afterHoldingNumber: 12L));
            Assert.AreEqual("mind_market", holdings["d"]["q"].Str("t"));
            Assert.AreEqual("holdings", holdings["d"]["q"]["d"].Str("kind"));
            Assert.AreEqual("owner", holdings["d"]["q"]["d"].Str("section"));
            Assert.AreEqual(12L, holdings["d"]["q"]["d"].Long("after"));

            var reqBody = JsonNode.NewObject();
            reqBody["op"] = JsonNode.FromString("list");
            var inv = JsonNode.Parse(MushProtocol.BuildMindInvokeCommand(9UL, reqBody));
            Assert.AreEqual("invoke", inv.Str("t"));
            Assert.AreEqual(9L, inv["d"].Long("id"));
            Assert.AreEqual("mind", inv["d"]["request"].Str("service"));
            Assert.AreEqual("list", inv["d"]["request"]["request"].Str("op"));
        }

        [Test]
        public void ClosedDecoders_ValidateSchemas_AndDecodeMarketHoldingsAndTrace()
        {
            Assert.IsNull(TacSimMindTerminal.DecodeMindMarketView(JsonNode.Parse(@"{""schema"":{""id"":""wrong"",""version"":1}}")));
            Assert.IsNull(TacSimMindTerminal.DecodeMindHoldingsView(JsonNode.Parse(@"{""schema"":{""id"":""mind.holdings"",""version"":2}}")));
            Assert.IsNull(TacSimMindTerminal.DecodeMindTraceView(JsonNode.Parse(@"{""schema"":{""id"":""arena.cockpit"",""version"":1}}")));

            var market = TacSimMindTerminal.DecodeMindMarketView(JsonNode.Parse(@"{
                ""schema"": { ""id"": ""mind.market"", ""version"": 1 },
                ""open"": true,
                ""notice"": null,
                ""after"": ""Z9Y8X7W6"",
                ""rows"": [
                    {
                        ""code"": ""K4M7P2Q9"",
                        ""name"": ""Convoy Escort IV"",
                        ""class"": ""II"",
                        ""form"": ""sealed"",
                        ""transferable"": true,
                        ""price_minor"": ""24000"",
                        ""price_text"": ""240.00 scrip"",
                        ""terms_line"": ""Perpetual hull seal"",
                        ""split_line"": ""85/15 guild split""
                    },
                    {
                        ""code"": ""SHORT"",
                        ""name"": ""Invalid Row""
                    }
                ],
                ""own"": {
                    ""quotes"": [{ ""code"": ""K4M7P2Q9"", ""price_text"": ""240.00 scrip"", ""expires_in_seconds"": 45, ""expired"": false }],
                    ""waiting"": [""M1N2P3Q4""]
                }
            }"));

            Assert.IsNotNull(market);
            Assert.AreEqual(1, market.Rows.Count, "Rows without an 8-char listing code must be ignored");
            Assert.AreEqual("K4M7P2Q9", market.Rows[0].Code);
            Assert.AreEqual("II", market.Rows[0].ClassWord);
            Assert.AreEqual("Z9Y8X7W6", market.NextAfterCode);
            Assert.AreEqual(1, market.OwnQuotes.Count);
            Assert.AreEqual(1, market.WaitingCodes.Count);

            var holdings = TacSimMindTerminal.DecodeMindHoldingsView(JsonNode.Parse(@"{
                ""schema"": { ""id"": ""mind.holdings"", ""version"": 1 },
                ""open"": true,
                ""section"": ""paid"",
                ""after"": 19,
                ""rows"": [
                    {
                        ""kind"": ""paid"",
                        ""licence"": 7,
                        ""name"": ""Convoy Escort IV"",
                        ""form"": ""sealed"",
                        ""phase"": { ""state"": ""active"", ""days_left"": 28 },
                        ""revoked"": false,
                        ""selling"": false,
                        ""renewal"": ""offered"",
                        ""actions"": [""renew""]
                    }
                ]
            }"));
            Assert.IsNotNull(holdings);
            Assert.AreEqual("paid", holdings.Section);
            Assert.AreEqual(19L, holdings.NextAfterNumber);
            Assert.AreEqual(1, holdings.Rows.Count);
            Assert.AreEqual(7, holdings.Rows[0].Number);
            Assert.AreEqual(28, holdings.Rows[0].DaysLeft);

            var trace = TacSimMindTerminal.DecodeMindTraceView(JsonNode.Parse(@"{
                ""schema"": { ""id"": ""arena.mind"", ""version"": 1 },
                ""available"": true,
                ""unit"": 42,
                ""omitted_thinks"": 2,
                ""traces"": [
                    {
                        ""think_seq"": 11,
                        ""t_ms"": 850,
                        ""rom"": false,
                        ""outcome"": { ""state"": ""completed"" },
                        ""diagnostics"": { ""ops"": 412, ""fees"": 18 },
                        ""author"": [""tracking target #3""],
                        ""intents"": [
                            { ""action"": ""fire"", ""detail"": ""mount 1"", ""outcome"": { ""state"": ""refused"", ""reason"": ""too_hot"" } }
                        ]
                    }
                ]
            }"));
            Assert.IsNotNull(trace);
            Assert.IsTrue(trace.Available);
            Assert.AreEqual(42, trace.UnitId);
            Assert.GreaterOrEqual(trace.Lines.Count, 4);
            Assert.IsTrue(trace.Lines[1].Contains("Think 11"));
            Assert.IsTrue(trace.Lines[trace.Lines.Count - 1].Contains("refused: too_hot"));
        }

        [Test]
        public void HoldingRowButtons_FollowServerActions()
        {
            var row = new MindHoldingRowView { Kind = "paid", Number = 7 };
            row.Actions.Add("renew_pay");
            row.Actions.Add("renew_decline");
            System.Action act = () => { };

            Assert.IsNull(TacSimMindTerminal.IfRowOffers(row, "renew", act), "renew is not offered once a renewal is pending");
            Assert.AreSame(act, TacSimMindTerminal.IfRowOffers(row, "renew_pay", act));
            Assert.IsNull(TacSimMindTerminal.IfRowOffers(null, "renew_pay", act));
        }

        [Test]
        public void MindTerminal_EndToEnd_MarketQuote_ShipSealing_AndDirtyWorkshopCheck()
        {
            using var ws = new MushWsClient();
            var go = new GameObject("TestMindTerminal");
            try
            {
                var term = go.AddComponent<TacSimMindTerminal>();
                term.BuildTerminalUi();
                term.BindClient(ws);

                // Opening terminal queries mind_market listings automatically
                term.SetTerminalOpen(true);
                Assert.IsTrue(term.IsTerminalOpen);
                var frames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, frames.Count);
                Assert.AreEqual("mind_market", frames[0]["d"]["q"].Str("t"));
                Assert.AreEqual("listings", frames[0]["d"]["q"]["d"].Str("kind"));

                // Quote action emits InvokeRequest::Mind { op: "market", action: { act: "quote", code: "K4M7P2Q9" } }
                term.SendMarketAction("quote", code: "K4M7P2Q9");
                frames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, frames.Count);
                Assert.AreEqual("invoke", frames[0].Str("t"));
                Assert.AreEqual("market", frames[0]["d"]["request"]["request"].Str("op"));
                Assert.AreEqual("quote", frames[0]["d"]["request"]["request"]["action"].Str("act"));
                Assert.AreEqual("K4M7P2Q9", frames[0]["d"]["request"]["request"]["action"].Str("code"));

                // Ship-mind berth sealing & link-dead posture emit canonical +ship/mind & +ship/posture verbs
                term.SealMindToShipBerth(7);
                term.SetShipLinkdeadPosture("guard");
                term.UnsealShipMind();
                frames = DrainOutboundFrames(ws);
                Assert.AreEqual(3, frames.Count);
                Assert.AreEqual("+ship/mind filed:7 go", frames[0].Str("d"));
                Assert.AreEqual("+ship/posture guard", frames[1].Str("d"));
                Assert.AreEqual("+ship/mind off go", frames[2].Str("d"));

                // Workshop: simulate read result -> edit source -> CheckWorkingMind flushes `replace` before `check`!
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 21,
                    ""ts"": 5000,
                    ""t"": ""invoke_result"",
                    ""d"": {
                        ""id"": 4,
                        ""result"": {
                            ""service"": ""mind"",
                            ""result"": {
                                ""status"": ""read"",
                                ""name"": ""escort_v1"",
                                ""base"": { ""rev"": 1, ""digest"": ""abc123def456"" },
                                ""draft"": { ""source"": ""fn think(ctx) { ctx.hold(); }"" }
                            }
                        }
                    }
                }"));
                ws.PumpMainThread();
                Assert.AreEqual("escort_v1", term.LoadedMindName);
                Assert.IsFalse(term.IsDraftDirty);

                term.UpdateWorkingSource("fn think(ctx) { ctx.guard(); }");
                Assert.IsTrue(term.IsDraftDirty);

                term.CheckWorkingMind();
                frames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, frames.Count, "Dirty draft must send `replace` first and wait for `edited` before `check`");
                Assert.AreEqual("replace", frames[0]["d"]["request"]["request"].Str("op"));
                Assert.AreEqual("fn think(ctx) { ctx.guard(); }", frames[0]["d"]["request"]["request"]["draft"].Str("source"));

                // Server replies `edited` with updated base -> terminal automatically chains `check`
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 22,
                    ""ts"": 5010,
                    ""t"": ""invoke_result"",
                    ""d"": {
                        ""id"": 5,
                        ""result"": {
                            ""service"": ""mind"",
                            ""result"": {
                                ""status"": ""edited"",
                                ""name"": ""escort_v1"",
                                ""base"": { ""rev"": 1, ""digest"": ""999888777666"" }
                            }
                        }
                    }
                }"));
                ws.PumpMainThread();

                Assert.IsFalse(term.IsDraftDirty);
                frames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, frames.Count);
                Assert.AreEqual("check", frames[0]["d"]["request"]["request"].Str("op"));
                Assert.AreEqual("999888777666", frames[0]["d"]["request"]["request"]["base"].Str("digest"));
            }
            finally
            {
                Object.DestroyImmediate(go);
            }
        }
    }
}
