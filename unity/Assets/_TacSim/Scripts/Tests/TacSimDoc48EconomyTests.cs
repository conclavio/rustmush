using System.Collections.Generic;
using System.Reflection;
using NUnit.Framework;
using TacSim.Arena;
using TacSim.Core;
using TacSim.Net;
using TacSim.Space;
using TacSim.Surface;
using TacSim.UI;
using TacSim.World;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.Tests
{
    public sealed class TacSimDoc48EconomyTests
    {
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

        private static Button FindButton(Transform root, string labelOrName)
        {
            var buttons = root.GetComponentsInChildren<Button>(true);
            for (int i = 0; i < buttons.Length; i++)
            {
                if (buttons[i].gameObject.name == labelOrName || buttons[i].gameObject.name == "Btn_" + labelOrName)
                    return buttons[i];
                var txt = buttons[i].GetComponentInChildren<Text>();
                if (txt != null && txt.text == labelOrName)
                    return buttons[i];
            }
            return null;
        }

        [Test]
        public void Track6_Doc48_Full24WeekHistory_PerTabIsolation_AndStaleHistoryClearing()
        {
            // Full 25-line server `+market/history` output (header + 24 IC weeks; `48d H-39`).
            var full25Lines = new List<string>
            {
                "<color=#BBD8EE>House prices of Structural Alloys here, by IC week, newest first (24):</color>",
                "<color=#BBD8EE>  this IC week: ask 420.00 cr, bid 360.00 cr</color>",
                "<color=#BBD8EE>  last IC week: ask 400.00 cr, bid 345.00 cr</color>",
            };
            for (int w = 2; w < 24; w++)
            {
                string bid = w == 23 ? "none" : $"{330 - w}.00 cr";
                full25Lines.Add($"<color=#BBD8EE>  {w} IC weeks ago: ask {380 - w}.00 cr, bid {bid}</color>");
            }
            Assert.AreEqual(25, full25Lines.Count);

            var rootGo = new GameObject("TestDoc48History");
            try
            {
                var hud = rootGo.AddComponent<TacSimHudController>();
                hud.BuildHud();
                hud.ShowLoginOverlay(false);
                hud.BindTradeState(new TacSimTradeAndShipState());

                var emitted = new List<string>();
                hud.OnCommandSubmitted += emitted.Add;

                hud.OpenTradeAndSailingModal("FREEPORT COMMODITY EXCHANGE", openSailingTab: false);
                hud.SwitchTradeTerminalTab(TradeTerminalTab.History, autoQuery: true);
                CollectionAssert.Contains(emitted, "+market/history tacsim:volatiles");

                // Ingest all 25 lines -> buffer must hold >= 28 lines so header + week 0 are NOT evicted (`48d H-39`).
                foreach (string ln in full25Lines)
                    hud.AppendModalResponseLine(ln);

                Assert.AreEqual("Structural Alloys", hud.EconomyPanels.LatestHistory.GoodLabel);
                Assert.AreEqual(24, hud.EconomyPanels.LatestHistory.RowsNewestFirst.Count);
                Assert.AreEqual("420.00 cr", hud.EconomyPanels.LatestHistory.LatestAsk);
                Assert.AreEqual("360.00 cr", hud.EconomyPanels.LatestHistory.LatestBid);
                StringAssert.Contains("·", hud.EconomyPanels.LatestHistory.BidSparkline);

                // Room speech must NOT pollute trade/economy tab echo buffers (`U-M14`).
                hud.AppendModalResponseLine("<b>Margit</b> says, \"Try the metals bid today.\"");
                var histOut = typeof(TacSimEconomyPanels).GetField("_historyOutputText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud.EconomyPanels) as Text;
                StringAssert.DoesNotContain("Margit", histOut.text);

                // Jobs tab has its own separate echo buffer — switching tabs must NOT show History text in Jobs (`U-M14`).
                hud.SwitchTradeTerminalTab(TradeTerminalTab.Jobs, autoQuery: true);
                CollectionAssert.Contains(emitted, "+market/jobs");
                hud.AppendModalResponseLine("<color=#BBD8EE>Job #9: Haul 10t metals to stickney</color>");
                var jobsOut = typeof(TacSimEconomyPanels).GetField("_jobsOutputText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud.EconomyPanels) as Text;
                StringAssert.Contains("Job #9", jobsOut.text);
                StringAssert.DoesNotContain("Structural Alloys", jobsOut.text);

                // Switching back to History & receiving `"No price history"` must clear stale `LatestHistory` (`U-M14`).
                hud.SwitchTradeTerminalTab(TradeTerminalTab.History, autoQuery: false);
                hud.AppendModalResponseLine("<color=#BBD8EE>No price history is kept here for that good.</color>", resetPreviousResponses: true);
                Assert.AreEqual(0, hud.EconomyPanels.LatestHistory.RowsNewestFirst.Count);
            }
            finally
            {
                Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Track6_Doc48_EscortHullDbref_SurveyAcceptGo_DecoupledHaulGood_AndCanonicalPortsDock()
        {
            using var ws = new MushWsClient();
            var rootGo = new GameObject("TestDoc48JobsAndPorts");
            try
            {
                var hud = rootGo.AddComponent<TacSimHudController>();
                hud.BuildHud();
                hud.BindWsClient(ws);
                hud.ShowLoginOverlay(false);

                var tradeState = new TacSimTradeAndShipState();
                hud.BindTradeState(tradeState);

                var emitted = new List<string>();
                hud.OnCommandSubmitted += emitted.Add;

                // 1. Verify canonical corridor ports (`ceres`, `stickney`, `luna`, `deimos`, `titan`) — NO `aitken`/`occator`/`rheasilvia` (`48d H-40`).
                var portButtons = FindPath(rootGo.transform, "TacSimCanvas/GameplayHud/TradeAndSailingTerminal/SailingTabBody/CorridorPortBar/CorridorPortButtons");
                Assert.IsNotNull(portButtons);
                Assert.AreEqual(5, portButtons.childCount);
                Assert.IsNull(FindButton(portButtons, "Occator Rim"));
                Assert.IsNull(FindButton(portButtons, "Rheasilvia (Vesta)"));

                var lunaBtn = FindButton(portButtons, "Luna (Aitken)");
                Assert.IsNotNull(lunaBtn);
                lunaBtn.onClick.Invoke();
                Assert.AreEqual("luna", tradeState.SelectedDestinationPort);

                // Dynamic 6-DOF Dock button emits `+helm/dock luna` instead of hardcoded `+helm/dock ceres` (`48d H-40`).
                hud.RefreshContextualQuickActions("interior:bridge", isSeatedAtHelm: true);
                emitted.Clear();
                var dockBtn = FindButton(rootGo.transform, "6-DOF Dock");
                Assert.IsNotNull(dockBtn);
                dockBtn.onClick.Invoke();
                CollectionAssert.AreEqual(new[] { "+helm/dock luna" }, emitted);

                // 2. Without active `#<dbref>`, `Post Escort` MUST NOT send hardcoded `"ship"` (`48d C-21`).
                hud.OpenTradeAndSailingModal("EXCHANGE", openSailingTab: false);
                hud.SwitchTradeTerminalTab(TradeTerminalTab.Jobs, autoQuery: false);
                emitted.Clear();
                var postEscortBtn = FindButton(hud.EconomyPanels.JobsTabBody.transform, "Post Escort");
                Assert.IsNotNull(postEscortBtn);
                postEscortBtn.onClick.Invoke();
                postEscortBtn.onClick.Invoke();
                Assert.AreEqual(0, emitted.Count, "Post Escort must never emit `+escort/post ship ...` without a #<dbref>");

                // Hydrate `port.trade` with real `ship.hull_id = "#42"` -> two-click confirm previews then emits `+escort/post #42 ... go` (`48d C-21`, `U-M15`).
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                    ""seq"": 1, ""ts"": 1000, ""t"": ""panel"",
                    ""d"": {
                        ""feed"": ""port.trade"",
                        ""kind"": ""keyframe"",
                        ""data"": {
                            ""enabled"": true,
                            ""venue"": ""ceres"",
                            ""ship"": { ""hull_id"": ""#42"", ""hull_name"": ""Slack Water"", ""port"": ""ceres"" }
                        }
                    }
                }"));
                ws.PumpMainThread();
                Assert.AreEqual("#42", hud.ActiveHullRef);

                emitted.Clear();
                postEscortBtn.onClick.Invoke(); // Click 1: preview without `go`
                Assert.AreEqual(1, emitted.Count);
                Assert.AreEqual("+escort/post #42 to=luna fee=250", emitted[0]);
                postEscortBtn.onClick.Invoke(); // Click 2: commit with `go`
                Assert.AreEqual(2, emitted.Count);
                Assert.AreEqual("+escort/post #42 to=luna fee=250 go", emitted[1]);

                // 3. `Accept Survey` uses real survey ID (`ahuna-a`, NOT `"1"`) and emits preview then `go` (`48d H-42`, `U-M15`).
                emitted.Clear();
                var acceptSurveyBtn = FindButton(hud.EconomyPanels.JobsTabBody.transform, "Accept Survey");
                Assert.IsNotNull(acceptSurveyBtn);
                acceptSurveyBtn.onClick.Invoke();
                Assert.AreEqual("+survey/accept ahuna-a", emitted[0]);
                acceptSurveyBtn.onClick.Invoke();
                Assert.AreEqual("+survey/accept ahuna-a go", emitted[1]);

                // 4. `Post Haul` is decoupled from History tab's `SelectedHistoryGood` (`U-M15`).
                hud.SwitchTradeTerminalTab(TradeTerminalTab.History, autoQuery: false);
                var metalsHistoryBtn = FindButton(hud.EconomyPanels.HistoryTabBody.transform, "Metals");
                metalsHistoryBtn.onClick.Invoke();
                Assert.AreEqual("metals", hud.EconomyPanels.SelectedHistoryGood);
                Assert.AreEqual("volatiles", hud.EconomyPanels.SelectedHaulGood, "History selection must not silently mutate Jobs haul good");

                hud.SwitchTradeTerminalTab(TradeTerminalTab.Jobs, autoQuery: false);
                emitted.Clear();
                var postHaulBtn = FindButton(hud.EconomyPanels.JobsTabBody.transform, "Post Haul");
                postHaulBtn.onClick.Invoke();
                postHaulBtn.onClick.Invoke();
                Assert.AreEqual("+haul/post 10 volatiles to=luna fee=250 go", emitted[1]);
            }
            finally
            {
                Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaCockpitOverlays_EmitValidVerbs_AvoidCommsOverlap_AndEscapeRichText()
        {
            var rootGo = new GameObject("TestArenaOverlays");
            try
            {
                var hud = rootGo.AddComponent<TacSimHudController>();
                hud.BuildHud();
                hud.ShowLoginOverlay(false);

                var arenaGo = new GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<OocArenaController>();
                arena.BuildArenaTerrain(null, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var spaceGo = new GameObject("Space");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<SpaceFlightController>();
                space.BuildSpaceStage(null);

                var surfaceGo = new GameObject("Surface");
                surfaceGo.transform.SetParent(rootGo.transform, false);
                var surface = surfaceGo.AddComponent<SurfaceMiningController>();

                hud.Overlays.BindControllers(surface, arena, space);

                var verbs = new List<string>();
                hud.Overlays.OnArenaCockpitVerbRequested += verbs.Add;

                // Activate arena with radiating ECM (`EmconSilent == false`) and malicious RichText callsign/banner.
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 88, ""seated"": true, ""callsign"": ""<color=red>R1</color>"",
                    ""party"": ""R"", ""unit_type"": ""tacsim.unit.line_tank"", ""power"": ""online"",
                    ""weapons_free"": true,
                    ""weapons"": [{ ""id"": ""<size=99>ac10</size>"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 10, ""rounds_max"": 20, ""percent"": 75 }]
                }"));
                arena.State.EmconSilent = false;
                arena.State.MatchBanner = "<size=999>EVIL BANNER</size>";
                hud.Overlays.RefreshArenaHud();

                // `U-C05`: ALL STOP emits `"speed 0"`, EMCON toggles `"emcon off"` / `"emcon on"`, SQUAD: ADVANCE emits `"order all advance <spot>"`.
                var stopBtn = FindButton(rootGo.transform, "ALL STOP [X]");
                var emconBtn = FindButton(rootGo.transform, "Btn_EMCON: ON");
                var advBtn = FindButton(rootGo.transform, "Btn_SQUAD: ADVANCE");
                Assert.IsNotNull(stopBtn);
                Assert.IsNotNull(emconBtn);
                Assert.IsNotNull(advBtn);

                stopBtn.onClick.Invoke();
                emconBtn.onClick.Invoke();
                arena.State.EmconSilent = true;
                emconBtn.onClick.Invoke();
                advBtn.onClick.Invoke();
                hud.Overlays.SelectAdvanceSpot("B");
                advBtn.onClick.Invoke();

                CollectionAssert.AreEqual(
                    new[] { "speed 0", "emcon off", "emcon on", "order all advance A", "order all advance B" },
                    verbs);

                // `U-M17`: OrbitalMiningAndTacBar and ArenaSquadOrderBar never overlap CommsDrawer (`14..436`) or OwnPlatformCard (`-430..-16`).
                var orbitalRt = FindPath(rootGo.transform, "TacSimCanvas/GameplayHud/OrbitalMiningAndTacBar").GetComponent<RectTransform>();
                var squadRt = FindPath(rootGo.transform, "TacSimCanvas/GameplayHud/ArenaBattleCockpitHud/ArenaSquadOrderBar").GetComponent<RectTransform>();
                Assert.GreaterOrEqual(orbitalRt.offsetMin.x, 452f);
                Assert.GreaterOrEqual(squadRt.offsetMin.x, 452f);
                Assert.LessOrEqual(squadRt.offsetMax.x, -446f);

                // `U-M01` + `48d M-37`: RichText injection neutralized across Arena, Space, and Docking HUDs.
                var armorTxt = typeof(TacSimMissionOverlays).GetField("_arenaOwnArmorText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud.Overlays) as Text;
                var wpnTxt = typeof(TacSimMissionOverlays).GetField("_arenaWeaponText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud.Overlays) as Text;
                var bannerTxt = typeof(TacSimMissionOverlays).GetField("_arenaBannerText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud.Overlays) as Text;
                StringAssert.DoesNotContain("<color=red>", armorTxt.text);
                StringAssert.Contains("‹color=red›", armorTxt.text);
                StringAssert.DoesNotContain("<size=99>", wpnTxt.text);
                Assert.IsFalse(bannerTxt.supportRichText);

                string mountSummary = TacSimMissionOverlays.FormatSpaceMountRowSummary(new SpaceMountTelemetry
                {
                    Id = "<size=88>laser_1</size>",
                    Family = "laser",
                    Mode = "<color=red>offense</color>",
                    State = "ready"
                });
                StringAssert.DoesNotContain("<size=88>", mountSummary);
                StringAssert.DoesNotContain("<color=red>", mountSummary);

                var dockSession = DockingClientSession.FromFeed(JsonNode.Parse(@"{
                    ""session_id"": ""s1"", ""active"": true, ""target_label"": ""<color=red>Ceres</color>"",
                    ""dx_mm"": 0, ""dy_mm"": 0, ""dz_mm"": 1000, ""vx_mmps"": 0, ""vy_mmps"": 0, ""vz_mmps"": 200
                }"));
                hud.UpdateDockingHud(dockSession, assistOn: true);
                var dockTxt = typeof(TacSimHudController).GetField("_dockingTelemetryText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud) as Text;
                StringAssert.DoesNotContain("<color=red>", dockTxt.text);
                StringAssert.Contains("‹color=red›", dockTxt.text);
            }
            finally
            {
                Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void HudModals_SingleFireCoopEnrol_ServerRequestTextCancel_QuickActions_PasswordClear_AndCardPool()
        {
            var rootGo = new GameObject("TestHudFixes");
            try
            {
                var hud = rootGo.AddComponent<TacSimHudController>();
                hud.BuildHud();
                hud.ShowLoginOverlay(false);

                var emitted = new List<string>();
                hud.OnCommandSubmitted += emitted.Add;

                // 1. `U-H30`: `+coop/enrol` SubmitNpcInput clears input immediately so it never double-submits.
                hud.ShowDetailModal("CO-OP", "Enter name:", commandPrefix: "+coop/enrol", defaultInputValue: "Copper Kestrel 12");
                hud.SubmitNpcInput();
                hud.SubmitNpcInput();
                CollectionAssert.AreEqual(new[] { "+coop/enrol Copper Kestrel 12" }, emitted);
                Assert.AreEqual(string.Empty, hud.GetModalInputValue());

                // 2. `U-H31`: `ShowServerRequestModal` supports `RequestKind::Text` / `Target` input AND sends `"__cancel__"` on close.
                var choices = new List<(ulong id, string choice)>();
                hud.OnRequestChoiceSubmitted += (r, c) => choices.Add((r.Id, c));

                var textReq = new MushServerRequest { Id = 10, Epoch = "e1", Kind = "text", Prompt = "Enter memo:" };
                hud.ShowServerRequestModal(textReq);
                var npcInput = typeof(TacSimHudController).GetField("_npcSayInput", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud) as InputField;
                Assert.IsTrue(npcInput.gameObject.activeSelf, "Text request must enable modal input field");
                npcInput.text = "Manifest verified";
                FindButton(rootGo.transform, "SUBMIT").onClick.Invoke();
                Assert.AreEqual(1, choices.Count);
                Assert.AreEqual((10UL, "Manifest verified"), choices[0]);

                var targetReq = new MushServerRequest { Id = 11, Epoch = "e1", Kind = "target", Prompt = "Select target:" };
                hud.ShowServerRequestModal(targetReq);
                Assert.IsTrue(npcInput.gameObject.activeSelf, "Target request must enable modal input field");
                hud.TryCloseTopmostModal(); // Simulate ESC
                Assert.AreEqual(2, choices.Count);
                Assert.AreEqual((11UL, "__cancel__"), choices[1]);

                // 3. `U-H30`: Priority-ordered single-layer Escape closes ONLY topmost modal per press.
                hud.Overlays.ShowArenaLobbyModal(true);
                hud.OpenTradeAndSailingModal("TRADE", openSailingTab: false);
                Assert.IsTrue(hud.Overlays.IsLobbyModalOpen);
                Assert.IsTrue(hud.TryCloseTopmostModal()); // Closes Trade only
                Assert.IsTrue(hud.Overlays.IsLobbyModalOpen, "Escape must close only one modal layer at a time");
                Assert.IsTrue(hud.TryCloseTopmostModal()); // Closes Lobby
                Assert.IsFalse(hud.Overlays.IsLobbyModalOpen);

                // 4. `U-H32`: Quick-action room wiring (`interior:bay`, `room.p11_service`, `room.c07_exchange`, `room.c05_registry`).
                emitted.Clear();
                hud.RefreshContextualQuickActions("interior:bay", isSeatedAtHelm: false);
                Assert.IsNull(FindButton(rootGo.transform, "Lease Rig"), "interior:bay must not expose invalid +tv/lease go");
                FindButton(rootGo.transform, "Rig Status (+tv)").onClick.Invoke();
                CollectionAssert.AreEqual(new[] { "+tv" }, emitted);

                emitted.Clear();
                hud.RefreshContextualQuickActions("room.p11_service", isSeatedAtHelm: false);
                FindButton(rootGo.transform, "Shipyard (+yard)").onClick.Invoke();
                FindButton(rootGo.transform, "Bay Status (+bay ship)").onClick.Invoke();
                CollectionAssert.AreEqual(new[] { "+yard", "+bay ship" }, emitted);

                emitted.Clear();
                hud.RefreshContextualQuickActions("room.c07_exchange", isSeatedAtHelm: false);
                FindButton(rootGo.transform, "Lease Rig Preview").onClick.Invoke();
                CollectionAssert.AreEqual(new[] { "+tv/lease" }, emitted);

                emitted.Clear();
                hud.RefreshContextualQuickActions("room.c05_registry", isSeatedAtHelm: false);
                FindButton(rootGo.transform, "Refit Dray-P").onClick.Invoke();
                CollectionAssert.AreEqual(new[] { "+coop/refit prospector" }, emitted);

                // 5. `U-M12`: Listing cards reuse pooled GameObjects across refreshes & disable BUY on full hold (`HOLD FULL`).
                var trade = new TacSimTradeAndShipState();
                hud.BindTradeState(trade);
                hud.OpenTradeAndSailingModal("EXCHANGE", openSailingTab: false);
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true, ""venue"": ""ceres"", ""balance_text"": ""5000.00cr"",
                    ""listings"": [{ ""id"": 101, ""side"": ""ask"", ""good"": ""metals"", ""good_label"": ""Metals"", ""available_qty"": 50, ""unit_price_minor"": 20000, ""price_text"": ""200.00cr"", ""vendor_name"": ""Freeport"" }],
                    ""ship"": { ""stowage"": { ""cargo_kg"": 100000, ""cargo_limit_kg"": 100000, ""cargo_l"": 120000, ""cargo_limit_l"": 120000, ""tank_kg"": 0, ""tank_limit_kg"": 80000, ""stores_days"": 0, ""stores_limit_days"": 120 } }
                }"));
                var listingsRoot = FindPath(rootGo.transform, "TacSimCanvas/GameplayHud/TradeAndSailingTerminal/ExchangeTabBody/ListingsViewport/ListingsContent");
                var cardBefore = listingsRoot.Find("Listing_101").gameObject;

                // Select MAX FIT / ALL while hold is 100% full -> button must say HOLD FULL and be non-interactable, reusing the SAME pooled card!
                FindButton(rootGo.transform, "MAX FIT / ALL").onClick.Invoke();
                var cardAfter = listingsRoot.Find("Listing_101").gameObject;
                Assert.AreSame(cardBefore, cardAfter, "Listing cards must be pooled/reused across refreshes");
                var buyBtn = cardAfter.GetComponentInChildren<Button>(true);
                Assert.IsFalse(buyBtn.interactable, "Full hold must disable BUY instead of clamping to BUY 1");
                Assert.AreEqual("HOLD FULL", buyBtn.GetComponentInChildren<Text>().text);

                // 6. `U-H12`: Password cleared on SubmitAuth & ShowLoginOverlay(true) resets HUD session state.
                var passInput = typeof(TacSimHudController).GetField("_passwordInput", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud) as InputField;
                passInput.text = "SuperSecret123!";
                string capturedPass = null;
                hud.OnAuthSubmitted += (_, _, p, _, _, _) => capturedPass = p;
                FindButton(rootGo.transform, "LOGIN TO MUSHD").onClick.Invoke();
                Assert.AreEqual("SuperSecret123!", capturedPass);
                Assert.AreEqual(string.Empty, passInput.text, "Plaintext password must be cleared immediately on submit");

                passInput.text = "LeftoverPass";
                hud.AppendLogLine("Sensitive previous session line");
                hud.ShowLoginOverlay(true, "Disconnected");
                Assert.AreEqual(string.Empty, passInput.text);
                var commsTxt = typeof(TacSimHudController).GetField("_commsLogText", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(hud) as Text;
                Assert.AreEqual(string.Empty, commsTxt.text, "ShowLoginOverlay(true) must clear previous session comms log");
            }
            finally
            {
                Object.DestroyImmediate(rootGo);
            }
        }
    }
}
