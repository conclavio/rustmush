using System.Collections.Generic;
using System.Reflection;
using System.Text;
using NUnit.Framework;
using TacSim.Core;
using TacSim.Net;
using TacSim.UI;
using UnityEngine;
using UnityEngine.UI;

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

        private static string BuildCanonicalMindDraftJson(string source)
        {
            // Matches `mush_proto::invoke::MindDraft` (`#[serde(deny_unknown_fields)]`, all 7 required fields).
            var draft = JsonNode.NewObject();
            draft["source"] = JsonNode.FromString(source);
            draft["schema"] = JsonNode.NewObject();
            draft["presets"] = JsonNode.NewObject();
            draft["profiles"] = JsonNode.NewObject();
            draft["rules"] = JsonNode.NewObject();
            draft["declared_class"] = JsonNode.FromString("II");
            draft["mind_abi"] = JsonNode.FromString("v1");
            return draft.ToJson();
        }

        private static Transform FindPath(Transform root, string slashPath)
        {
            var cur = root;
            foreach (string seg in slashPath.Split('/'))
            {
                if (cur == null) return null;
                cur = cur.Find(seg);
            }
            return cur;
        }

        private static Button FindChildButton(Transform root, string textOrName)
        {
            var buttons = root.GetComponentsInChildren<Button>(true);
            for (int i = 0; i < buttons.Length; i++)
            {
                if (buttons[i].gameObject.name == textOrName)
                    return buttons[i];
                var txt = buttons[i].GetComponentInChildren<Text>();
                if (txt != null && txt.text == textOrName)
                    return buttons[i];
            }
            return null;
        }

        [Test]
        public void Protocol_BuildsCanonicalMindMarketQuery_AndMindInvokeEnvelopes()
        {
            // ListingCode is strictly 8 lowercase hex chars (`0-9`, `a-f`) per `invoke.rs:520-523`.
            var listings = JsonNode.Parse(MushProtocol.BuildMindMarketQueryCommand(7UL, "listings", afterListingCode: "a4b7c2d9"));
            Assert.AreEqual("req", listings.Str("t"));
            Assert.AreEqual(7L, listings["d"].Long("id"));
            Assert.AreEqual("mind_market", listings["d"]["q"].Str("t"));
            Assert.AreEqual("listings", listings["d"]["q"]["d"].Str("kind"));
            Assert.AreEqual("a4b7c2d9", listings["d"]["q"]["d"].Str("after"));

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
        public void ClosedDecoders_ValidateRealServerSchemas_WaitingObjects_OwnerLicence_AndFullArenaMindTrace()
        {
            Assert.IsNull(TacSimMindTerminal.DecodeMindMarketView(JsonNode.Parse(@"{""schema"":{""id"":""wrong"",""version"":1}}")));
            Assert.IsNull(TacSimMindTerminal.DecodeMindHoldingsView(JsonNode.Parse(@"{""schema"":{""id"":""mind.holdings"",""version"":2}}")));
            Assert.IsNull(TacSimMindTerminal.DecodeMindTraceView(JsonNode.Parse(@"{""schema"":{""id"":""arena.cockpit"",""version"":1}}")));

            // Real `query.rs:250-332` `mind.market` schema: lowercase 8-hex codes, `"240.00cr"`, `waiting: [{"code": "..."}]` + fallback string.
            var market = TacSimMindTerminal.DecodeMindMarketView(JsonNode.Parse(@"{
                ""schema"": { ""id"": ""mind.market"", ""version"": 1 },
                ""open"": true,
                ""notice"": null,
                ""after"": ""f0e1d2c3"",
                ""rows"": [
                    {
                        ""code"": ""a4b7c2d9"",
                        ""name"": ""Convoy Escort IV"",
                        ""class"": ""II"",
                        ""form"": ""sealed"",
                        ""transferable"": true,
                        ""term_seconds"": 2419200,
                        ""grace_seconds"": 604800,
                        ""price_minor"": ""24000"",
                        ""price_text"": ""240.00cr"",
                        ""levy_permille"": 10,
                        ""venue_permille"": 30,
                        ""parent_royalty_permille"": 0,
                        ""derived"": false,
                        ""terms_line"": ""28d term, 7d grace"",
                        ""split_line"": ""96% author / 3% venue / 1% levy""
                    },
                    {
                        ""code"": ""short"",
                        ""name"": ""Invalid Row""
                    }
                ],
                ""own"": {
                    ""quotes"": [{ ""code"": ""a4b7c2d9"", ""price_minor"": ""24000"", ""price_text"": ""240.00cr"", ""expires_in_seconds"": 45, ""expired"": false }],
                    ""waiting"": [{ ""code"": ""b1c2d3e4"" }, ""d5e6f708""]
                }
            }"));

            Assert.IsNotNull(market);
            Assert.AreEqual(1, market.Rows.Count, "Rows without an 8-char listing code must be ignored");
            Assert.AreEqual("a4b7c2d9", market.Rows[0].Code);
            Assert.AreEqual("II", market.Rows[0].ClassWord);
            Assert.AreEqual("240.00cr", market.Rows[0].PriceText);
            Assert.AreEqual("28d term, 7d grace", market.Rows[0].TermsLine);
            Assert.AreEqual("f0e1d2c3", market.NextAfterCode);
            Assert.AreEqual(1, market.OwnQuotes.Count);
            Assert.AreEqual(2, market.WaitingCodes.Count, "Must decode live `{\"code\": \"...\"}` waiting objects and string fallback");
            Assert.AreEqual("b1c2d3e4", market.WaitingCodes[0]);
            Assert.AreEqual("d5e6f708", market.WaitingCodes[1]);

            // Real `query.rs:472-563` `mind.holdings` paid + owner schemas (with `owner_licence`).
            var paidHoldings = TacSimMindTerminal.DecodeMindHoldingsView(JsonNode.Parse(@"{
                ""schema"": { ""id"": ""mind.holdings"", ""version"": 1 },
                ""open"": true,
                ""section"": ""paid"",
                ""after"": 19,
                ""rows"": [
                    {
                        ""kind"": ""paid"",
                        ""licence"": 7,
                        ""house"": false,
                        ""name"": ""Convoy Escort IV"",
                        ""form"": ""sealed"",
                        ""transferable"": true,
                        ""phase"": { ""state"": ""active"", ""days_left"": 28 },
                        ""revoked"": false,
                        ""selling"": false,
                        ""renewal"": ""offered"",
                        ""actions"": [""renew_pay"", ""renew_decline"", ""resell""]
                    }
                ]
            }"));
            Assert.IsNotNull(paidHoldings);
            Assert.AreEqual("paid", paidHoldings.Section);
            Assert.AreEqual(19L, paidHoldings.NextAfterNumber);
            Assert.AreEqual(1, paidHoldings.Rows.Count);
            Assert.AreEqual(7, paidHoldings.Rows[0].Number);
            Assert.AreEqual(7L, paidHoldings.Rows[0].LicenceId);
            Assert.AreEqual(0L, paidHoldings.Rows[0].FiledId);
            Assert.AreEqual(28, paidHoldings.Rows[0].DaysLeft);

            var ownerHoldings = TacSimMindTerminal.DecodeMindHoldingsView(JsonNode.Parse(@"{
                ""schema"": { ""id"": ""mind.holdings"", ""version"": 1 },
                ""open"": true,
                ""section"": ""owner"",
                ""after"": null,
                ""rows"": [
                    {
                        ""kind"": ""owner"",
                        ""filed"": 41,
                        ""owner_licence"": 7000003,
                        ""name"": ""Escort Author V1"",
                        ""dependent"": false,
                        ""phase"": { ""state"": ""active"", ""days_left"": 88 },
                        ""selling"": false,
                        ""actions"": [""publish"", ""withdraw"", ""charter""]
                    },
                    {
                        ""kind"": ""owner"",
                        ""filed"": 42,
                        ""owner_licence"": null,
                        ""name"": ""Unlicenced Original"",
                        ""dependent"": true,
                        ""phase"": { ""state"": ""active"", ""days_left"": 10 },
                        ""selling"": true,
                        ""actions"": [""publish"", ""withdraw""]
                    }
                ]
            }"));
            Assert.IsNotNull(ownerHoldings);
            Assert.AreEqual(2, ownerHoldings.Rows.Count);
            Assert.AreEqual(41L, ownerHoldings.Rows[0].FiledId);
            Assert.AreEqual(7000003L, ownerHoldings.Rows[0].LicenceId);
            Assert.AreEqual(42L, ownerHoldings.Rows[1].FiledId);
            Assert.IsNull(ownerHoldings.Rows[1].LicenceId);

            // Real `author_trace.rs` / `trace_feed.rs` `arena.mind` schema (`Outcome::Applied`, settled, landed_ms, rom_drop, kernels, rules, omitted).
            var trace = TacSimMindTerminal.DecodeMindTraceView(JsonNode.Parse(@"{
                ""schema"": { ""id"": ""arena.mind"", ""version"": 1 },
                ""available"": true,
                ""unit"": 42,
                ""omitted_thinks"": 2,
                ""traces"": [
                    {
                        ""settled"": true,
                        ""fault"": ""ops"",
                        ""rom_drop"": ""strikes"",
                        ""think_seq"": 11,
                        ""t_ms"": 850,
                        ""landed_ms"": 910,
                        ""rom"": false,
                        ""outcome"": { ""state"": ""applied"" },
                        ""diagnostics"": {
                            ""ops"": 412,
                            ""fees"": 18,
                            ""kernels"": [{ ""kernel"": ""best_hostile"", ""fee"": 10 }],
                            ""rules"": [{
                                ""slot"": 1,
                                ""description"": ""hold fire when hot"",
                                ""matches"": true,
                                ""terms"": [{ ""description"": ""heat_high"", ""value"": false }]
                            }],
                            ""omitted"": {
                                ""author_lines"": 1,
                                ""author_chars"": 0,
                                ""kernel_calls"": 0,
                                ""rules"": 0,
                                ""rule_terms"": 0,
                                ""detail_chars"": 0,
                                ""intents"": 2
                            }
                        },
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
            string joinedTrace = string.Join("\n", trace.Lines);
            StringAssert.Contains("Think 11", joinedTrace);
            StringAssert.Contains("applied", joinedTrace);
            StringAssert.Contains("settled", joinedTrace);
            StringAssert.Contains("Landing: 910 ms", joinedTrace);
            StringAssert.Contains("Fault: ops", joinedTrace);
            StringAssert.Contains("ROM drop: strikes", joinedTrace);
            StringAssert.Contains("Kernel best_hostile: 10 fees", joinedTrace);
            StringAssert.Contains("Rule 1: hold fire when hot = true", joinedTrace);
            StringAssert.Contains("heat_high = false", joinedTrace);
            StringAssert.Contains("author_lines=1, intents=2", joinedTrace);
            StringAssert.Contains("refused: too_hot", joinedTrace);
        }

        [Test]
        public void MindTerminal_OwnerSealing_UsesOwnerLicenceNeverFiledId_AndRespectsServerActions()
        {
            using var ws = new MushWsClient();
            var go = new GameObject("TestMindSeal");
            try
            {
                var term = go.AddComponent<TacSimMindTerminal>();
                term.BuildTerminalUi();
                term.BindClient(ws);
                term.SetTerminalOpen(true);
                DrainOutboundFrames(ws);

                // Inject owner holdings panel with 1 valid `owner_licence` and 1 null `owner_licence`.
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 1, ""ts"": 1000, ""t"": ""panel"",
                    ""d"": {
                        ""feed"": ""mind.holdings"",
                        ""kind"": ""keyframe"",
                        ""data"": {
                            ""schema"": { ""id"": ""mind.holdings"", ""version"": 1 },
                            ""open"": true,
                            ""section"": ""owner"",
                            ""after"": null,
                            ""rows"": [
                                {
                                    ""kind"": ""owner"",
                                    ""filed"": 41,
                                    ""owner_licence"": 7000003,
                                    ""name"": ""Escort <color=red>Author</color>"",
                                    ""dependent"": false,
                                    ""phase"": { ""state"": ""active"", ""days_left"": 88 },
                                    ""selling"": false,
                                    ""actions"": [""publish""]
                                },
                                {
                                    ""kind"": ""owner"",
                                    ""filed"": 42,
                                    ""owner_licence"": null,
                                    ""name"": ""NoLicenceOriginal"",
                                    ""dependent"": false,
                                    ""phase"": { ""state"": ""active"", ""days_left"": 88 },
                                    ""selling"": false,
                                    ""actions"": [""withdraw""]
                                }
                            ]
                        }
                    }
                }"));
                ws.PumpMainThread();

                Assert.AreEqual(2, term.VisibleHoldingRowCount);
                // Direct helper must refuse null `owner_licence` and NEVER send `filed:42`.
                Assert.IsFalse(term.TrySealHoldingRow(term.HoldingsPage.Rows[1]));
                Assert.AreEqual(0, DrainOutboundFrames(ws).Count);

                // Direct helper on valid `owner_licence` emits `+ship/mind filed:7000003 go`, NOT `filed:41`!
                Assert.IsTrue(term.TrySealHoldingRow(term.HoldingsPage.Rows[0]));
                var frames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, frames.Count);
                Assert.AreEqual("+ship/mind filed:7000003 go", frames[0].Str("d"));

                // Verify UI row buttons: row 0 has two-click Seal Ship + Publish (NO Withdraw since server omitted it),
                // row 1 has disabled "No Lic" button + Withdraw (NO Publish).
                var row0 = FindPath(go.transform, "MindTerminalCanvas/MindTerminalWindow/HoldingsAndSealingPane/HoldingsRowsViewport/HoldingsRowsViewport_Content/HoldRow_0");
                var row1 = FindPath(go.transform, "MindTerminalCanvas/MindTerminalWindow/HoldingsAndSealingPane/HoldingsRowsViewport/HoldingsRowsViewport_Content/HoldRow_1");
                Assert.IsNotNull(row0);
                Assert.IsNotNull(row1);

                // RichText in owner name must be escaped (`U-M01`).
                var row0Lbl = row0.Find("Txt").GetComponent<Text>();
                StringAssert.DoesNotContain("<color=red>", row0Lbl.text);
                StringAssert.Contains("‹color=red›", row0Lbl.text);

                var row0Btns = row0.GetComponentsInChildren<Button>(true);
                Assert.IsTrue(row0Btns[0].interactable, "Row 0 with owner_licence must allow Seal Ship");
                Assert.IsTrue(row0Btns[1].gameObject.activeSelf, "Row 0 advertised publish");
                Assert.IsFalse(row0Btns[2].gameObject.activeSelf, "Row 0 did NOT advertise withdraw");

                // Two-click confirmation on row 0 Seal Ship (`U-M15`): click 1 arms, click 2 commits.
                row0Btns[0].onClick.Invoke();
                Assert.AreEqual(0, DrainOutboundFrames(ws).Count, "First click must arm confirm without sealing");
                row0Btns[0].onClick.Invoke();
                frames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, frames.Count);
                Assert.AreEqual("+ship/mind filed:7000003 go", frames[0].Str("d"));

                var row1Btns = row1.GetComponentsInChildren<Button>(true);
                Assert.IsFalse(row1Btns[0].interactable, "Row 1 with null owner_licence must disable Seal button");
                Assert.AreEqual("No Lic", row1Btns[0].GetComponentInChildren<Text>().text);
                Assert.IsFalse(row1Btns[1].gameObject.activeSelf, "Row 1 did NOT advertise publish");
                Assert.IsTrue(row1Btns[2].gameObject.activeSelf, "Row 1 advertised withdraw");

                // Verify paid row with `actions: ["renew_pay", "renew_decline"]` hides "Renew" and shows Pay Renew + Decline (`48d H-43`).
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 2, ""ts"": 1010, ""t"": ""panel"",
                    ""d"": {
                        ""feed"": ""mind.holdings"",
                        ""kind"": ""keyframe"",
                        ""data"": {
                            ""schema"": { ""id"": ""mind.holdings"", ""version"": 1 },
                            ""open"": true,
                            ""section"": ""paid"",
                            ""after"": null,
                            ""rows"": [
                                {
                                    ""kind"": ""paid"",
                                    ""licence"": 9,
                                    ""house"": false,
                                    ""name"": ""Paid Renewal"",
                                    ""form"": ""sealed"",
                                    ""transferable"": false,
                                    ""phase"": { ""state"": ""grace"", ""days_left"": 3 },
                                    ""revoked"": false,
                                    ""selling"": false,
                                    ""renewal"": ""offered"",
                                    ""actions"": [""renew_pay"", ""renew_decline""]
                                }
                            ]
                        }
                    }
                }"));
                ws.PumpMainThread();

                var paidRow0Btns = row0.GetComponentsInChildren<Button>(true);
                Assert.AreEqual("Pay Renew", paidRow0Btns[1].GetComponentInChildren<Text>().text);
                Assert.AreEqual("Decline", paidRow0Btns[2].GetComponentInChildren<Text>().text);
            }
            finally
            {
                Object.DestroyImmediate(go);
            }
        }

        [Test]
        public void MindTerminal_TwentyRowMarketPage_WaitingEscrowRefund_AndTwoClickPay()
        {
            using var ws = new MushWsClient();
            var go = new GameObject("TestMindMarket20");
            try
            {
                var term = go.AddComponent<TacSimMindTerminal>();
                term.BuildTerminalUi();
                term.BindClient(ws);
                term.SetTerminalOpen(true);
                DrainOutboundFrames(ws);

                var rowsJson = new StringBuilder();
                for (int i = 0; i < 20; i++)
                {
                    if (i > 0) rowsJson.Append(',');
                    string hex = $"a00000{i:x2}";
                    rowsJson.Append($"{{\"code\":\"{hex}\",\"name\":\"Doctrine {i}\",\"class\":\"II\",\"form\":\"sealed\",\"transferable\":true,\"price_minor\":\"15000\",\"price_text\":\"150.00cr\",\"terms_line\":\"28d term\",\"split_line\":\"96/3/1\"}}");
                }

                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 5, ""ts"": 2000, ""t"": ""panel"",
                    ""d"": {{
                        ""feed"": ""mind.market"",
                        ""kind"": ""keyframe"",
                        ""data"": {{
                            ""schema"": {{ ""id"": ""mind.market"", ""version"": 1 }},
                            ""open"": true,
                            ""notice"": null,
                            ""after"": ""a0000013"",
                            ""rows"": [{rowsJson}],
                            ""own"": {{
                                ""quotes"": [{{ ""code"": ""a0000000"", ""price_minor"": ""15000"", ""price_text"": ""150.00cr"", ""expires_in_seconds"": 60, ""expired"": false }}],
                                ""waiting"": [{{ ""code"": ""b1c2d3e4"" }}]
                            }}
                        }}
                    }}
                }}"));
                ws.PumpMainThread();

                // All 20 listings + 1 own quote + 1 waiting escrow must be rendered (not truncated to 8 rows; `48d H-43`).
                Assert.AreEqual(22, term.VisibleMarketRowCount);

                // Row 0 displays decoded terms/split and requires two-click confirm on Pay (`U-M15`).
                var mktRow0 = FindPath(go.transform, "MindTerminalCanvas/MindTerminalWindow/MarketPane/MarketRowsViewport/MarketRowsViewport_Content/MktRow_0");
                var mktRow0Text = mktRow0.Find("Txt").GetComponent<Text>().text;
                StringAssert.Contains("28d term", mktRow0Text);
                StringAssert.Contains("96/3/1", mktRow0Text);
                var mktRow0Btns = mktRow0.GetComponentsInChildren<Button>(true);
                mktRow0Btns[1].onClick.Invoke(); // Click 1: arm Pay
                Assert.AreEqual(0, DrainOutboundFrames(ws).Count);
                mktRow0Btns[1].onClick.Invoke(); // Click 2: commit Pay
                var payFrames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, payFrames.Count);
                Assert.AreEqual("pay", payFrames[0]["d"]["request"]["request"]["action"].Str("act"));
                Assert.AreEqual("a0000000", payFrames[0]["d"]["request"]["request"]["action"].Str("code"));

                // Row 21 is the waiting escrow row with Complete + Refund (`U-H25`).
                var waitRow = FindPath(go.transform, "MindTerminalCanvas/MindTerminalWindow/MarketPane/MarketRowsViewport/MarketRowsViewport_Content/MktRow_21");
                Assert.IsNotNull(waitRow);
                var waitBtns = waitRow.GetComponentsInChildren<Button>(true);
                Assert.AreEqual("Refund", waitBtns[1].GetComponentInChildren<Text>().text);
                waitBtns[1].onClick.Invoke(); // Click 1: arm Refund
                Assert.AreEqual(0, DrainOutboundFrames(ws).Count);
                waitBtns[1].onClick.Invoke(); // Click 2: emit Refund
                var refFrames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, refFrames.Count);
                Assert.AreEqual("refund", refFrames[0]["d"]["request"]["request"]["action"].Str("act"));
                Assert.AreEqual("b1c2d3e4", refFrames[0]["d"]["request"]["request"]["action"].Str("code"));
            }
            finally
            {
                Object.DestroyImmediate(go);
            }
        }

        [Test]
        public void MindTerminal_Workshop_RealDraftSchema_Bounds_RequestIdCorrelation_Diagnostics_AndAllStatuses()
        {
            using var ws = new MushWsClient();
            var go = new GameObject("TestMindWorkshop");
            try
            {
                var term = go.AddComponent<TacSimMindTerminal>();
                term.BuildTerminalUi();
                term.BindClient(ws);
                term.SetTerminalOpen(true);
                DrainOutboundFrames(ws);

                var srcField = typeof(TacSimMindTerminal).GetField("_rhaiSourceInput", BindingFlags.Instance | BindingFlags.NonPublic);
                var srcInput = srcField?.GetValue(term) as InputField;
                Assert.IsNotNull(srcInput);
                Assert.IsFalse(srcInput.interactable, "Source editor must be disabled when LoadedDraft == null (`U-H27`)");

                // 20-mind library must render all 20 buttons (not capped at 6; `48d H-43`).
                var libNames = new StringBuilder();
                for (int i = 0; i < 20; i++)
                {
                    if (i > 0) libNames.Append(',');
                    libNames.Append($"\"mind_{i:D2}\"");
                }
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 10, ""ts"": 3000, ""t"": ""invoke_result"",
                    ""d"": {{ ""id"": 1, ""result"": {{ ""service"": ""mind"", ""result"": {{ ""status"": ""library"", ""names"": [{libNames}] }} }} }}
                }}"));
                ws.PumpMainThread();
                Assert.AreEqual(20, term.VisibleLibraryButtonCount);

                // Request read -> receive real `MindEditBase` (`{edit, revision}`) + 7-field `MindDraft`.
                term.RequestReadMind("escort_v1");
                var readReq = DrainOutboundFrames(ws);
                Assert.AreEqual(1, readReq.Count);
                ulong readReqId = (ulong)readReq[0]["d"].Long("id");

                string initialDraft = BuildCanonicalMindDraftJson("fn think(ctx) { ctx.hold(); }");
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 11, ""ts"": 3010, ""t"": ""invoke_result"",
                    ""d"": {{
                        ""id"": {readReqId},
                        ""result"": {{
                            ""service"": ""mind"",
                            ""result"": {{
                                ""status"": ""read"",
                                ""name"": ""escort_v1"",
                                ""revision"": null,
                                ""base"": {{ ""edit"": 3, ""revision"": 1 }},
                                ""draft"": {initialDraft},
                                ""rules_text"": {{}},
                                ""rules_source"": {{}},
                                ""diagnostics"": []
                            }}
                        }}
                    }}
                }}"));
                ws.PumpMainThread();

                Assert.AreEqual("escort_v1", term.LoadedMindName);
                Assert.IsTrue(srcInput.interactable);
                Assert.IsFalse(term.IsDraftDirty);

                // Bounds validation: >32 KiB or NUL must reject AND revert `_rhaiSourceInput.text` (`U-H27`).
                srcInput.text = "bad\0source";
                term.UpdateWorkingSource("bad\0source");
                Assert.IsFalse(term.IsDraftDirty);
                Assert.AreEqual("fn think(ctx) { ctx.hold(); }", srcInput.text);

                string huge = new string('a', 32 * 1024 + 5);
                srcInput.text = huge;
                term.UpdateWorkingSource(huge);
                Assert.IsFalse(term.IsDraftDirty);
                Assert.AreEqual("fn think(ctx) { ctx.hold(); }", srcInput.text);

                // Edit draft -> SaveWorkingMindRevision -> sends `replace` first.
                term.UpdateWorkingSource("fn think(ctx) { ctx.guard(); }");
                Assert.IsTrue(term.IsDraftDirty);
                term.SaveWorkingMindRevision("v2 notes");
                var replaceFrames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, replaceFrames.Count);
                Assert.AreEqual("replace", replaceFrames[0]["d"]["request"]["request"].Str("op"));
                ulong replaceReqId = (ulong)replaceFrames[0]["d"].Long("id");

                // Unrelated late `error` with different requestId must NOT abort active `Replace -> Save` chain (`U-H28`).
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 12, ""ts"": 3020, ""t"": ""invoke_result"",
                    ""d"": { ""id"": 99999, ""result"": { ""service"": ""mind"", ""result"": { ""status"": ""error"", ""code"": ""stale"", ""message"": ""Old job failed"" } } }
                }"));
                ws.PumpMainThread();

                // Out-of-order `read` must NOT overwrite dirty draft (`U-H28`).
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 13, ""ts"": 3025, ""t"": ""invoke_result"",
                    ""d"": {{ ""id"": 88888, ""result"": {{ ""service"": ""mind"", ""result"": {{ ""status"": ""read"", ""name"": ""other"", ""revision"": null, ""base"": {{ ""edit"": 1, ""revision"": null }}, ""draft"": {initialDraft}, ""rules_text"": {{}}, ""rules_source"": {{}}, ""diagnostics"": [] }} }} }}
                }}"));
                ws.PumpMainThread();
                Assert.AreEqual("escort_v1", term.LoadedMindName);
                Assert.IsTrue(term.IsDraftDirty);

                // User keeps editing WHILE `replace` is in flight -> `edited` must keep `IsDraftDirty == true` and send fresh `replace` (`U-H28`).
                term.UpdateWorkingSource("fn think(ctx) { ctx.engage(); }");
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 14, ""ts"": 3030, ""t"": ""invoke_result"",
                    ""d"": {{ ""id"": {replaceReqId}, ""result"": {{ ""service"": ""mind"", ""result"": {{ ""status"": ""edited"", ""name"": ""escort_v1"", ""base"": {{ ""edit"": 4, ""revision"": 1 }} }} }} }}
                }}"));
                ws.PumpMainThread();
                Assert.IsTrue(term.IsDraftDirty, "In-flight local edit must preserve IsDraftDirty");
                var secondReplace = DrainOutboundFrames(ws);
                Assert.AreEqual(1, secondReplace.Count);
                Assert.AreEqual("replace", secondReplace[0]["d"]["request"]["request"].Str("op"));
                ulong secondReplaceId = (ulong)secondReplace[0]["d"].Long("id");

                // Second `edited` completes with no newer local edit -> chains `save` with updated `base: {edit: 5, revision: 1}`.
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 15, ""ts"": 3040, ""t"": ""invoke_result"",
                    ""d"": {{ ""id"": {secondReplaceId}, ""result"": {{ ""service"": ""mind"", ""result"": {{ ""status"": ""edited"", ""name"": ""escort_v1"", ""base"": {{ ""edit"": 5, ""revision"": 1 }} }} }} }}
                }}"));
                ws.PumpMainThread();
                Assert.IsFalse(term.IsDraftDirty);
                var saveFrames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, saveFrames.Count);
                Assert.AreEqual("save", saveFrames[0]["d"]["request"]["request"].Str("op"));
                Assert.AreEqual(5L, saveFrames[0]["d"]["request"]["request"]["base"].Long("edit"));
                ulong saveReqId = (ulong)saveFrames[0]["d"].Long("id");

                // Server replies `queued` (non-final!) then `checked` with compilation diagnostic on SAME `saveReqId` (`structured.rs:490-501`, `U-H26`).
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 16, ""ts"": 3050, ""t"": ""invoke_result"",
                    ""d"": {{ ""id"": {saveReqId}, ""result"": {{ ""service"": ""mind"", ""result"": {{ ""status"": ""queued"", ""job"": 77 }} }} }}
                }}"));
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 17, ""ts"": 3060, ""t"": ""invoke_result"",
                    ""d"": {{
                        ""id"": {saveReqId},
                        ""result"": {{
                            ""service"": ""mind"",
                            ""result"": {{
                                ""status"": ""checked"",
                                ""job"": 77,
                                ""reports"": [],
                                ""diagnostics"": [{{ ""field"": ""source"", ""message"": ""unexpected token ';'"", ""line"": 12, ""column"": 5 }}]
                            }}
                        }}
                    }}
                }}"));
                ws.PumpMainThread();

                StringAssert.Contains("Save rejected", term.StatusBanner);
                StringAssert.DoesNotContain("Check complete", term.StatusBanner);
                Assert.AreEqual(1, term.LastDiagnostics.Count);
                Assert.AreEqual(12, term.LastDiagnostics[0].Line);
                StringAssert.Contains("[source @ 12:5] unexpected token ';'", term.WorkshopOutputText);

                // Check returns 6 isolated `MindPictureReport`s -> all 6 rendered (`U-H26`).
                term.CheckWorkingMind();
                var chkFrames = DrainOutboundFrames(ws);
                ulong chkReqId = (ulong)chkFrames[0]["d"].Long("id");
                var repsJson = new StringBuilder();
                for (int p = 0; p < 6; p++)
                {
                    if (p > 0) repsJson.Append(',');
                    repsJson.Append($"{{\"picture\":\"pic_{p + 1}\",\"armed\":{(p % 2 == 0 ? "true" : "false")},\"operations\":{100 + p},\"fees\":{10 + p},\"intents\":[\"hold\"],\"fault\":null,\"station_refusals\":[],\"class_violations\":[]}}");
                }
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 18, ""ts"": 3070, ""t"": ""invoke_result"",
                    ""d"": {{ ""id"": {chkReqId}, ""result"": {{ ""service"": ""mind"", ""result"": {{ ""status"": ""checked"", ""job"": 78, ""reports"": [{repsJson}], ""diagnostics"": [] }} }} }}
                }}"));
                ws.PumpMainThread();
                Assert.AreEqual(6, term.LastReports.Count);
                StringAssert.Contains("pic_1", term.WorkshopOutputText);
                StringAssert.Contains("pic_6", term.WorkshopOutputText);

                // `Create` -> `edited` must reset `LoadedDraft = null` cleanly so stale draft is never paired with new base (`U-H28`).
                term.RequestCreateMindFromStandard("fresh_v2");
                var createFrames = DrainOutboundFrames(ws);
                ulong createReqId = (ulong)createFrames[0]["d"].Long("id");
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 19, ""ts"": 3080, ""t"": ""invoke_result"",
                    ""d"": {{ ""id"": {createReqId}, ""result"": {{ ""service"": ""mind"", ""result"": {{ ""status"": ""edited"", ""name"": ""fresh_v2"", ""base"": {{ ""edit"": 0, ""revision"": null }} }} }} }}
                }}"));
                ws.PumpMainThread();
                Assert.AreEqual("fresh_v2", term.LoadedMindName);
                Assert.IsNull(term.LoadedDraft, "Create response must clear stale LoadedDraft until read arrives");
                Assert.IsFalse(srcInput.interactable);
                DrainOutboundFrames(ws);

                // Verify all 5 previously dropped `MindResponse` statuses (`U-H28`).
                term.HandleInvokeResult(501, "mind", JsonNode.Parse(@"{""status"":""cancelled"",""job"":79}"));
                StringAssert.Contains("cancelled", term.StatusBanner);

                term.HandleInvokeResult(502, "mind", JsonNode.Parse(@"{""status"":""filed_source"",""licence"":""7"",""source_sha256"":""0123456789abcdef"",""first"":1,""lines"":[""1: fn think(ctx) {}""]}"));
                StringAssert.Contains("paid source", term.StatusBanner);
                StringAssert.Contains("1: fn think(ctx) {}", term.WorkshopOutputText);

                term.HandleInvokeResult(503, "mind", JsonNode.Parse(@"{""status"":""removal_requested"",""name"":""fresh_v2""}"));
                StringAssert.Contains("Removal requested", term.StatusBanner);

                term.HandleInvokeResult(504, "mind", JsonNode.Parse(@"{""status"":""removed"",""name"":""fresh_v2""}"));
                StringAssert.Contains("Removed working mind", term.StatusBanner);
                Assert.AreEqual(string.Empty, term.LoadedMindName);

                term.HandleInvokeResult(505, "mind", JsonNode.Parse(@"{""status"":""pruned"",""revisions"":3,""source_candidates_retained"":2}"));
                StringAssert.Contains("Pruned 3", term.StatusBanner);
            }
            finally
            {
                Object.DestroyImmediate(go);
            }
        }

        [Test]
        public void MindTerminal_LoginBlocking_TopStripPill_FocusChecks_LiveTraceUnfollow_AndSessionReset()
        {
            using var ws = new MushWsClient();
            var hudGo = new GameObject("TestHudForMind");
            var termGo = new GameObject("TestMindLogin");
            try
            {
                var hud = hudGo.AddComponent<TacSimHudController>();
                hud.BuildHud();
                hud.BindWsClient(ws);

                var term = termGo.AddComponent<TacSimMindTerminal>();
                term.BuildTerminalUi();
                term.BindClient(ws);
                term.BindHud(hud);

                // Launcher pill sits docked inside TopStrip (`y = -58..-16`), strictly above top-right gameplay cards (`y <= -74`) (`U-H29`).
                Assert.IsNotNull(term.LauncherPillRect);
                Assert.GreaterOrEqual(term.LauncherPillRect.offsetMin.y, -64f, "Launcher pill must stay inside TopStrip above gameplay cards");
                Assert.LessOrEqual(term.LauncherPillRect.offsetMax.y, -10f);

                // While Login Overlay is active (`ShowLoginOverlay(true)`), F10 & SetTerminalOpen(true) are blocked and sortingOrder < 20 (`U-H29`).
                Assert.IsTrue(hud.IsLoginOverlayVisible);
                Assert.IsFalse(term.CanOpenViaHotkey);
                Assert.IsFalse(term.TryHandleHotkeyToggle());
                term.SetTerminalOpen(true);
                Assert.IsFalse(term.IsTerminalOpen, "MindTerminal must not open over Login Overlay");
                Assert.Less(term.TerminalCanvas.sortingOrder, 20, "MindTerminal canvas must stay below Login Overlay");

                // Hide Login Overlay -> terminal opens, HUD `IsModalOpen` and `IsTextInputFocused` reflect MindTerminal (`48d H-43`).
                hud.ShowLoginOverlay(false);
                term.SetTerminalOpen(true);
                Assert.IsTrue(term.IsTerminalOpen);
                Assert.IsTrue(hud.IsModalOpen, "HUD IsModalOpen must include TacSimMindTerminal");

                var srcField = typeof(TacSimMindTerminal).GetField("_rhaiSourceInput", BindingFlags.Instance | BindingFlags.NonPublic);
                var srcInput = srcField?.GetValue(term) as InputField;
                srcInput.isFocused = true;
                Assert.IsTrue(hud.IsTextInputFocused, "HUD IsTextInputFocused must include TacSimMindTerminal inputs");
                srcInput.isFocused = false;

                // Follow live trace -> closing terminal sends unfollow (`follow: false`) automatically (`U-M16`).
                DrainOutboundFrames(ws);
                term.RequestUnitMindTrace(42, follow: true);
                Assert.IsTrue(term.IsFollowingTrace);
                DrainOutboundFrames(ws);

                term.SetTerminalOpen(false);
                Assert.IsFalse(term.IsFollowingTrace);
                var unfollowFrames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, unfollowFrames.Count);
                Assert.AreEqual("trace", unfollowFrames[0]["d"]["request"]["request"].Str("op"));
                Assert.IsFalse(unfollowFrames[0]["d"]["request"]["request"].Bool("follow", true));

                // Plain-text safety on status banner & trace output (`U-M01`).
                var statusTxt = typeof(TacSimMindTerminal).GetField("_statusBannerText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(term) as Text;
                var traceTxt = typeof(TacSimMindTerminal).GetField("_traceOutputText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(term) as Text;
                Assert.IsFalse(statusTxt.supportRichText);
                Assert.IsFalse(traceTxt.supportRichText);

                // Re-open and load data, then trigger `ShowLoginOverlay(true)` -> session reset wipes terminal state (`U-H12`).
                term.SetTerminalOpen(true);
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 20, ""ts"": 4000, ""t"": ""panel"",
                    ""d"": { ""feed"": ""mind.market"", ""kind"": ""keyframe"", ""data"": { ""schema"": { ""id"": ""mind.market"", ""version"": 1 }, ""open"": true, ""notice"": null, ""after"": null, ""rows"": [], ""own"": { ""quotes"": [], ""waiting"": [] } } }
                }"));
                ws.PumpMainThread();
                Assert.IsNotNull(term.MarketPage);

                hud.ShowLoginOverlay(true);
                Assert.IsFalse(term.IsTerminalOpen);
                Assert.IsNull(term.MarketPage);
                Assert.IsNull(term.HoldingsPage);
                Assert.IsNull(term.LoadedDraft);
                Assert.AreEqual(0, term.LibraryNames.Count);
            }
            finally
            {
                Object.DestroyImmediate(termGo);
                Object.DestroyImmediate(hudGo);
            }
        }

        [Test]
        public void MindTerminal_And_Hud_PreserveDirtyDraftAndCommsOnHelloReconnect_ButWipeOnFatalDisconnect()
        {
            using var ws = new MushWsClient();
            var hudGo = new GameObject("ReconnectHud");
            var termGo = new GameObject("ReconnectMindTerm");
            try
            {
                var hud = hudGo.AddComponent<TacSimHudController>();
                hud.BuildHud();
                hud.BindWsClient(ws);
                hud.ShowLoginOverlay(false);

                var term = termGo.AddComponent<TacSimMindTerminal>();
                term.BuildTerminalUi();
                term.BindClient(ws);
                term.BindHud(hud);

                // Populate live session state: comms log, active hull, open Mind Terminal with dirty Rhai draft.
                hud.AppendLogLine("[CONNECTED] Attached as Captain");
                hud.SetActiveHullRef("#77");
                term.SetTerminalOpen(true);

                DrainOutboundFrames(ws);
                term.RequestReadMind("escort_v1");
                var readFrames = DrainOutboundFrames(ws);
                Assert.AreEqual(1, readFrames.Count);
                ulong readReqId = (ulong)readFrames[0]["d"].Long("id");

                string initialDraft = BuildCanonicalMindDraftJson("fn think(ctx) { ctx.hold(); }");
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{
                    ""seq"": 1, ""ts"": 5000, ""t"": ""invoke_result"",
                    ""d"": {{
                        ""id"": {readReqId},
                        ""result"": {{
                            ""service"": ""mind"",
                            ""result"": {{
                                ""status"": ""read"",
                                ""name"": ""escort_v1"",
                                ""revision"": null,
                                ""base"": {{ ""edit"": 1, ""revision"": 0 }},
                                ""draft"": {initialDraft},
                                ""rules_text"": {{}},
                                ""rules_source"": {{}},
                                ""diagnostics"": []
                            }}
                        }}
                    }}
                }}"));
                ws.PumpMainThread();

                var srcInput = typeof(TacSimMindTerminal).GetField("_rhaiSourceInput", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(term) as InputField;
                srcInput.text = "// unsaved work\nfn think(ctx) { ctx.hold(); }";
                srcInput.onValueChanged.Invoke(srcInput.text);
                Assert.IsTrue(term.IsDraftDirty);

                // Simulate fresh login hello / transient WS auto-reconnect hello -> MUST NOT wipe session or dirty edits!
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 2, ""ts"": 5100, ""t"": ""hello"",
                    ""d"": { ""server"": ""rustmush"", ""panels"": [""mind.market"", ""mind.holdings"", ""arena.mind"", ""port.trade""] }
                }"));
                ws.PumpMainThread();

                var commsTxt = typeof(TacSimHudController).GetField("_commsLogText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud) as Text;
                StringAssert.Contains("[CONNECTED] Attached as Captain", commsTxt.text, "Transient hello/reconnect must preserve comms log");
                Assert.AreEqual("#77", hud.ActiveHullRef, "Transient hello/reconnect must preserve active hull ref");
                Assert.IsTrue(term.IsTerminalOpen, "Transient hello/reconnect must keep Mind Terminal open");
                Assert.IsNotNull(term.LoadedDraft, "Transient hello/reconnect must preserve loaded draft");
                Assert.IsTrue(term.IsDraftDirty, "Transient hello/reconnect must preserve dirty flag");
                StringAssert.Contains("// unsaved work", term.LoadedDraft.Str("source"));

                // Fatal session disconnect (`OnFatalSessionEnded`) MUST wipe both HUD and Mind Terminal state (`U-H12`).
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 3, ""ts"": 5200, ""t"": ""disconnect"",
                    ""d"": { ""reason"": ""session expired"" }
                }"));
                ws.PumpMainThread();

                Assert.AreEqual(string.Empty, commsTxt.text, "Fatal session disconnect must wipe comms log");
                Assert.IsNull(hud.ActiveHullRef, "Fatal session disconnect must clear active hull ref");
                Assert.IsFalse(term.IsTerminalOpen, "Fatal session disconnect must close Mind Terminal");
                Assert.IsNull(term.LoadedDraft, "Fatal session disconnect must clear loaded draft");
                Assert.IsFalse(term.IsDraftDirty, "Fatal session disconnect must clear dirty flag");
            }
            finally
            {
                Object.DestroyImmediate(termGo);
                Object.DestroyImmediate(hudGo);
            }
        }
    }
}
