using System.Collections.Generic;
using NUnit.Framework;
using TacSim.Core;
using TacSim.Net;
using TacSim.Space;
using TacSim.World;

namespace TacSim.Tests
{
    public sealed class TacSimEditModeTests
    {
        [Test]
        public void Json_RoundTrips_NestedObjectsAndArrays()
        {
            string sample = "{\"seq\":42,\"ts\":1700000,\"e\":{\"t\":\"ping\",\"d\":{\"nonce\":991}},\"arr\":[true,-15,3.5,\"a\\\"b\"]}";
            var parsed = JsonNode.Parse(sample);
            Assert.IsTrue(parsed.IsObject);
            Assert.AreEqual(42L, parsed.Long("seq"));
            Assert.AreEqual("ping", parsed["e"].Str("t"));
            Assert.AreEqual(991L, parsed["e"]["d"].Long("nonce"));
            Assert.AreEqual(4, parsed["arr"].Count);
            Assert.AreEqual("a\"b", parsed["arr"][3].AsString());

            var reparsed = JsonNode.Parse(parsed.ToJson());
            Assert.AreEqual(991L, reparsed["e"]["d"].Long("nonce"));
            Assert.AreEqual(-15L, reparsed["arr"][1].AsLong());
        }

        [Test]
        public void Protocol_Parses_FlattenedMushdEnvelope_AndBuildsCanonicalPanelSubscribe()
        {
            // Canonical mush-proto Envelope uses #[serde(flatten)] on OutputEvent (top-level "t" and "d")
            string wire = @"{
                ""seq"": 7,
                ""ts"": 1000,
                ""t"": ""room"",
                ""d"": {
                    ""id"": ""#18"",
                    ""name"": ""Visiting Berth Gallery"",
                    ""desc"": [{ ""text"": ""Armourglass "" }, { ""text"": ""over the berths."" }],
                    ""contents"": [{ ""id"": ""#44"", ""name"": ""Watch Officer Kade"" }],
                    ""exits"": [{ ""name"": ""Pressure Lobby <N>"", ""aliases"": [""n"", ""north""] }],
                    ""ambient"": {
                        ""atlas"": {
                            ""pack"": ""neighborhood.ceres_freeport"",
                            ""entry"": ""room.c01_berth_gallery"",
                            ""surface"": ""floor 1"",
                            ""details"": [""berth_board"", ""cradles""]
                        },
                        ""tacsim"": {
                            ""mode"": ""berth_gallery"",
                            ""port_entry"": ""ceres_port"",
                            ""docked_count"": 8,
                            ""own_hull"": ""#201"",
                            ""own_hull_name"": ""Salt Moth"",
                            ""own_hull_docked_here"": true
                        },
                        ""npcs"": [""#44""],
                        ""ic_time"": { ""unix"": 9000, ""hour"": 14, ""minute"": 30, ""season"": ""winter"", ""time_of_day"": ""afternoon"" }
                    }
                }
            }";

            var env = MushProtocol.ParseEnvelope(wire);
            Assert.IsNotNull(env);
            Assert.AreEqual(7UL, env.Seq);
            Assert.AreEqual("room", env.EventType);

            var rv = MushProtocol.ParseRoomView(env.Data);
            Assert.IsNotNull(rv);
            Assert.AreEqual("#18", rv.Id);
            Assert.AreEqual("Armourglass over the berths.", rv.Desc);
            Assert.AreEqual("room.c01_berth_gallery", rv.Atlas.Entry);
            Assert.AreEqual(2, rv.Atlas.Details.Count);
            Assert.AreEqual("berth_gallery", rv.TacSim.Mode);
            Assert.AreEqual("Salt Moth", rv.TacSim.OwnHullName);
            Assert.IsTrue(rv.TacSim.OwnHullDockedHere);
            Assert.IsTrue(rv.NpcIds.Contains("#44"));
            Assert.AreEqual(14, rv.IcTime.Hour);

            var sub = JsonNode.Parse(MushProtocol.BuildPanelSubscribeCommand(3UL, "space.cockpit"));
            Assert.AreEqual("req", sub.Str("t"));
            Assert.AreEqual(3L, sub["d"].Long("id"));
            Assert.AreEqual("panel_subscribe", sub["d"]["q"].Str("t"));
            Assert.AreEqual("space.cockpit", sub["d"]["q"]["d"].Str("feed"));
        }

        [Test]
        public void Docking60Hz_BitmaskRoundTripsAllSixAxesAndFlags()
        {
            var input = new DockingControlInput
            {
                TransX = -1,
                TransY = 1,
                ClosurePulse = 1,
                PitchTrim = -1,
                YawTrim = 1,
                RollTrim = -1,
                ToggleDampeners = true,
                Abort = false
            };

            ushort mask = input.ToBitmask();
            var unpacked = DockingControlInput.FromBitmask(mask);
            Assert.AreEqual(input, unpacked);
            Assert.AreEqual(6u, unpacked.ActiveAxisPulses());
        }

        [Test]
        public void Docking60Hz_GuidanceReachesFiveGreenGates_AndEncodesValidRleTape()
        {
            var session = DockingClientSession.CreateStandardPortPractice("dock.401.1", "Ceres Berth 01");
            Assert.IsFalse(session.IsClamped, "Standard challenge pose must start outside the 5 alignment gates");

            for (int i = 0; i < DockingClientSession.MaxTapeFrames && !session.IsClamped; i++)
            {
                var guidance = session.ComputeGuidanceAssistInput();
                session.Step60Fps(guidance);
            }

            Assert.IsTrue(session.IsClamped, $"Expected all 5 gates green within 3600 frames, stepped={session.FramesStepped}, gates={session.EvaluateGates().GreenCount}");
            Assert.LessOrEqual(session.DzMm, 500L);
            Assert.GreaterOrEqual(session.VzMmps, 100L);
            Assert.LessOrEqual(session.VzMmps, 800L);

            string cmd = session.FormatTapeCommand();
            Assert.IsTrue(cmd.StartsWith("+helm/dock tape dock.401.1 "));
            Assert.IsTrue(cmd.Contains("*"));
        }

        [Test]
        public void ContentLoader_LoadsRealCeresFreeportAndDrayDefinitions()
        {
            var ceres = TacSimContentLoader.LoadCeresFreeport();
            Assert.IsNotNull(ceres);
            Assert.GreaterOrEqual(ceres.Rooms.Count, 12, "Ceres Freeport pack should expose its 3-deck room set");
            Assert.IsTrue(ceres.Rooms.ContainsKey("room.c01_berth_gallery"));
            Assert.IsTrue(ceres.Rooms.ContainsKey("room.c03_arrivals"));
            Assert.IsTrue(ceres.Rooms.ContainsKey("room.c14_dockmaster"));

            var dray = TacSimContentLoader.LoadDrayTemplate();
            Assert.IsNotNull(dray);
            Assert.AreEqual(6, dray.Compartments.Count, "Dray should load all 6 walkable compartments (airlock, passage, bridge, crew, hold, engineering)");
            Assert.AreEqual(5, dray.Exits.Count, "Dray should load all 5 interior exit pairs");

            // Verify shortest-path BFS from Arrivals -> Registry & Co-op Desk -> Berth Gallery
            var toCoop = ceres.FindExitPath("room.c03_arrivals", "room.c05_registry");
            Assert.AreEqual(1, toCoop.Count);
            Assert.AreEqual("goto n", toCoop[0].command);

            var coopToBerth = ceres.FindExitPath("room.c05_registry", "room.c01_berth_gallery");
            Assert.AreEqual(3, coopToBerth.Count);
            Assert.AreEqual("goto s", coopToBerth[0].command);
            Assert.AreEqual("goto s", coopToBerth[1].command);
            Assert.AreEqual("goto s", coopToBerth[2].command);

            var airlockToBridge = dray.FindExitPath("airlock", "bridge");
            Assert.AreEqual(2, airlockToBridge.Count);

            // Verify cast/ceres_freeport.json merges enabled cast NPCs & flags dormant faction principals as Enabled=false
            var ilse = ceres.FindSeedNpcByName("Ilse Oduya-Brandt");
            Assert.IsNotNull(ilse);
            Assert.IsTrue(ilse.Enabled);
            Assert.AreEqual("room.c05_registry", ilse.RoomId);

            var kalu = ceres.FindSeedNpcByName("Matthias Kalu");
            Assert.IsNotNull(kalu);
            Assert.IsFalse(kalu.Enabled, "Dormant faction principal Matthias Kalu must stay unspawned until enabled");

            // Slice 1B: Gender-aware cast routing for male vs female Ceres Freeport NPCs
            Assert.IsFalse(ilse.IsMale);
            var dov = ceres.FindSeedNpcByName("Dov Akintola");
            Assert.IsNotNull(dov);
            Assert.IsTrue(dov.IsMale);
            var nkem = ceres.FindSeedNpcByName("Nkem Halvorsen");
            Assert.IsNotNull(nkem);
            Assert.IsTrue(nkem.IsMale);
            var corin = ceres.FindSeedNpcByName("Corin Mbatha");
            Assert.IsNotNull(corin);
            Assert.IsTrue(corin.IsMale);
        }

        [Test]
        public void Slice1C_GoodsTableAndTradeState_ParseAuthoritativeMushdMarketAndGauges()
        {
            var goods = TacSimContentLoader.LoadGoodsTable();
            Assert.IsNotNull(goods);
            Assert.GreaterOrEqual(goods.Count, 6);

            var state = new TacSimTradeAndShipState(goods);
            var volatiles = state.FindGoodByNameOrCode("water ice");
            Assert.IsNotNull(volatiles);
            Assert.AreEqual("volatiles", volatiles.Id);
            Assert.AreEqual(1000, volatiles.MassKgPerUnit);
            Assert.AreEqual(1400, volatiles.VolumeLPerUnit);
            Assert.IsFalse(volatiles.IsConsumable);

            var propellant = state.FindGoodByNameOrCode("propellant");
            Assert.IsNotNull(propellant);
            Assert.AreEqual("stow.propellant", propellant.Stowage);
            Assert.IsTrue(propellant.IsConsumable);

            // Empty Dray (0/100 t, 0/120 m³): water ice cubes out at floor(120,000 / 1,400) = 85 t before 100 t mass!
            Assert.AreEqual(85, state.ComputeMaxAdditionalUnitsForHold("volatiles"));
            // Metal concentrate (400 L/t) weighs out at 100 t before 120 m³ volume!
            Assert.AreEqual(100, state.ComputeMaxAdditionalUnitsForHold("metals"));

            // Parse canonical crates/mush-market listing_line() ask & standing-bid strings
            state.NotifyMarketRefreshRequested();
            Assert.IsTrue(state.IngestServerLine("#4 water ice — 120 × 100.00cr [ceres] at Occator Water", 1f, out _));
            Assert.IsTrue(state.IngestServerLine("#9 structural alloy stock — wanted 80 × 410.00cr [ceres] by Freeport Works", 1f, out _));
            Assert.IsTrue(state.IngestServerLine("#11 yard fittings, crated — wanted 40 × 395.00cr [ceres] by Freeport Works — not buying", 1f, out _));

            Assert.AreEqual(3, state.Listings.Count);
            Assert.AreEqual(4UL, state.Listings[0].ListingId);
            Assert.AreEqual("volatiles", state.Listings[0].GoodCode);
            Assert.IsFalse(state.Listings[0].IsWantedBid);
            Assert.AreEqual("Occator Water", state.Listings[0].VendorName);

            Assert.AreEqual(9UL, state.Listings[1].ListingId);
            Assert.AreEqual("structural_alloys", state.Listings[1].GoodCode);
            Assert.IsTrue(state.Listings[1].IsWantedBid);
            Assert.IsTrue(state.Listings[1].IsFunded);

            Assert.IsTrue(state.Listings[2].IsWantedBid);
            Assert.IsFalse(state.Listings[2].IsFunded, "Standing bid with '— not buying' must mark IsFunded=false");

            // Standing-bid sell MUST omit '#' prefix per lot_sell_args()
            Assert.AreEqual("+market/sell 9=25", TacSimTradeAndShipState.FormatOneClickSellCommand(9UL, 25, sellAll: false));
            Assert.AreEqual("+market/sell 9=all", TacSimTradeAndShipState.FormatOneClickSellCommand(9UL, 25, sellAll: true));

            // Correlated 1-click buy Quote -> Accept verification (never blind text-scraping!)
            // `mush-market` reads quantity from the `=` right-hand side (`+market/quote #<id>=<qty>`)
            string quoteCmd = state.BeginOneClickBuy(4UL, 15, currentTimeS: 10f, displayedUnitPriceMinor: 10000);
            Assert.AreEqual("+market/quote #4=15", quoteCmd);

            // Unrelated text quote MUST NOT trigger blind auto-accept
            Assert.IsFalse(state.IngestServerLine("Quote #987: 999 × 500.00cr = 499500.00cr — good until 2026-11-17T03:00:00Z.", 11f, out var unrelatedFollowUps));
            Assert.IsNull(unrelatedFollowUps);

            // Unrelated open_quotes row in port.trade MUST NOT match pending intent (#4, qty 15)
            var unrelatedFeed = JsonNode.Parse(@"{""enabled"":true,""open_quotes"":[{""quote_id"":987,""listing_id"":99,""quantity"":999,""total_minor"":49950000}]}");
            Assert.IsFalse(state.TryResolvePendingQuoteFromFeed(unrelatedFeed, 11f, out string badAccept, out _));
            Assert.IsNull(badAccept);

            // Price move upward above displayed max total MUST hold auto-accept and warn pilot
            var movedPriceFeed = JsonNode.Parse(@"{""enabled"":true,""open_quotes"":[{""quote_id"":26,""listing_id"":4,""quantity"":15,""total_minor"":165000,""total_text"":""1650.00cr""}]}");
            Assert.IsTrue(state.TryResolvePendingQuoteFromFeed(movedPriceFeed, 11f, out string heldAccept, out string priceWarn));
            Assert.IsNull(heldAccept);
            Assert.IsNotNull(priceWarn);

            // Exact correlated quote within displayed price emits `+market/accept #27`
            state.BeginOneClickBuy(4UL, 15, currentTimeS: 12f, displayedUnitPriceMinor: 10000);
            var validQuoteFeed = JsonNode.Parse(@"{""enabled"":true,""open_quotes"":[{""quote_id"":27,""listing_id"":4,""quantity"":15,""total_minor"":150000,""total_text"":""1500.00cr""}]}");
            Assert.IsTrue(state.TryResolvePendingQuoteFromFeed(validQuoteFeed, 13f, out string goodAccept, out string noWarn));
            Assert.AreEqual("+market/accept #27", goodAccept);
            Assert.IsNull(noWarn);

            // Parse canonical crates/tacsim/src/port/gauges.rs +ship & +credit lines
            Assert.IsTrue(state.IngestServerLine("Hold: cargo 29/100 t · 11.6/120 m³ · tank 42/80 t · locker 20/120 crew-days.", 12f, out _));
            Assert.AreEqual(29, state.HoldMassTonnes);
            Assert.AreEqual(11.6f, state.HoldVolumeM3, 0.01f);
            Assert.AreEqual(42, state.TankTonnes);
            Assert.AreEqual(20, state.StoresCrewDays);

            // Must parse AVAILABLE balance (375.00cr), not POSTED balance (500.00cr)!
            Assert.IsTrue(state.IngestServerLine("Credit: 500.00cr posted, 125.00cr held, 375.00cr available.", 12f, out _));
            Assert.AreEqual("375.00cr", state.CreditsText);

            // Canonical sailing plan clear phrase ("The sailing plan is cleared.")
            Assert.IsTrue(state.IngestServerLine("Order 1 filed: buy volatiles 10. The sailing plan is at revision 1.", 12f, out _));
            Assert.AreEqual(1, state.SailingOrderLines.Count);
            Assert.IsTrue(state.IngestServerLine("The sailing plan is cleared.", 13f, out _));
            Assert.AreEqual(0, state.SailingOrderLines.Count);
        }

        [Test]
        public void Slice2_LoadsMultiPortNeighborhoods_OocSimPods_AndDrayPVehicleBay()
        {
            // Stickney Yards (Phobos) + cast
            var stickney = TacSimContentLoader.LoadNeighborhoodPackById("neighborhood.stickney_yards");
            Assert.IsNotNull(stickney);
            Assert.IsTrue(stickney.Rooms.ContainsKey("room.p01_berth_gallery"));
            Assert.IsTrue(stickney.Rooms.ContainsKey("room.p03_customs"));
            Assert.IsTrue(stickney.Rooms.ContainsKey("room.p08_exchange"));
            Assert.AreEqual("room.p01_berth_gallery", stickney.GetBerthGalleryRoomEntry());

            // Aitken Highport (Luna)
            var aitken = TacSimContentLoader.LoadNeighborhoodPackById("neighborhood.aitken_highport");
            Assert.IsNotNull(aitken);
            Assert.IsTrue(aitken.Rooms.ContainsKey("room.l01_berth_gallery"));
            Assert.IsTrue(aitken.Rooms.ContainsKey("room.l03_arrivals"));

            // OOC Simulator Centre (Welcome Hall -> Simulator Lobby -> Red Room / Blue Room + 12 Sim Pods)
            var ooc = TacSimContentLoader.LoadNeighborhoodPackById("neighborhood.ooc_area");
            Assert.IsNotNull(ooc);
            Assert.IsTrue(ooc.Rooms.ContainsKey("room.simulator_lobby"));
            Assert.IsTrue(ooc.Rooms.ContainsKey("room.sim_red"));
            Assert.IsTrue(ooc.Rooms.ContainsKey("room.sim_blue"));

            var lobbyToRed = ooc.FindExitPath("room.simulator_lobby", "room.sim_red");
            Assert.AreEqual(1, lobbyToRed.Count);
            var lobbyToBlue = ooc.FindExitPath("room.simulator_lobby", "room.sim_blue");
            Assert.AreEqual(1, lobbyToBlue.Count);

            int redPods = 0, bluePods = 0;
            for (int i = 0; i < ooc.SeedItems.Count; i++)
            {
                if (ooc.SeedItems[i].RoomId == "room.sim_red" && ooc.SeedItems[i].IsInterior) redPods++;
                if (ooc.SeedItems[i].RoomId == "room.sim_blue" && ooc.SeedItems[i].IsInterior) bluePods++;
            }
            Assert.AreEqual(6, redPods, "Red Room must carry all 6 Red Sim Pods");
            Assert.AreEqual(6, bluePods, "Blue Room must carry all 6 Blue Sim Pods");

            // Dray-P Prospector adds 7th Vehicle Bay ("bay") + +tv/bay in|out routing
            var drayP = TacSimContentLoader.LoadShipTemplate("tacsim.unit.dray_p");
            Assert.IsNotNull(drayP);
            Assert.IsTrue(drayP.Compartments.ContainsKey("bay"), "Dray-P must expose its stern Vehicle Bay");
            var holdToBay = drayP.FindExitPath("hold", "bay");
            Assert.AreEqual(1, holdToBay.Count);
            Assert.AreEqual("+tv/bay in", holdToBay[0].command);

            // Customs Patrol Corvette interior
            var corvette = TacSimContentLoader.LoadShipTemplate("tacsim.unit.corvette");
            Assert.IsNotNull(corvette);
            Assert.IsTrue(corvette.Compartments.ContainsKey("bridge"));
        }

        [Test]
        public void Slice3_DecodesSiteMapRleGrid_AndPlanetarySurfaceHeightfield()
        {
            var occator = TacSimTerrainDecoder.BuildPlanetarySiteGrid("occator_rim", side: 64);
            Assert.IsNotNull(occator);
            Assert.AreEqual(64, occator.Width);
            Assert.AreEqual(64, occator.Height);
            Assert.AreEqual('H', occator.RawGlyphs[32 * 64 + 32], "Central pad cell must carry landed hull glyph H");

            // Decode RLE rows on the `site.map` / `arena.map` wire format
            var rowsJson = JsonNode.Parse(@"[[[ "","", 3 ], [ ""3"", 1 ], [ ""@"", 1 ], [ ""^"", 3 ]]]");
            char[] decoded = TacSimTerrainDecoder.DecodeRleGlyphRows(rowsJson, 8, 1);
            Assert.AreEqual(',', decoded[0]);
            Assert.AreEqual(',', decoded[2]);
            Assert.AreEqual('3', decoded[3]);
            Assert.AreEqual('@', decoded[4]);
            Assert.AreEqual('^', decoded[7]);
        }

        [Test]
        public void OocArena_ZigzagLeb128DeltaElev_RoundTrips_AndLoads256x256MarsCanyon()
        {
            // Verify exact zigzag-leb128-delta elevation codec matching crates/tacsim/src/match_/cockpit/frames.rs
            int[] sampleCm = { 0, 125, -340, -340, 1850, 420, -15, 0 };
            string encoded = TacSimTerrainDecoder.EncodeZigzagLeb128DeltaElevCm(sampleCm);
            int[] decoded = TacSimTerrainDecoder.DecodeZigzagLeb128DeltaElevCm(encoded, sampleCm.Length);
            Assert.AreEqual(sampleCm, decoded);

            // Decode full synthetic `arena.map` v1 frame
            var mapNode = JsonNode.Parse($@"{{
                ""match"": 19,
                ""surface"": ""mars_canyon"",
                ""biome"": ""desert"",
                ""cell_cm"": 1000,
                ""width"": 4,
                ""height"": 2,
                ""origin_cell"": [0, 0],
                ""rows"": [[["":"", 2], [""="", 2]], [[""^"", 4]]],
                ""spots"": [{{ ""letter"": ""A"", ""name"": ""West Ford"", ""cell"": [1, 1] }}],
                ""elev"": {{ ""encoding"": ""zigzag-leb128-delta"", ""unit_cm"": 1, ""data"": ""{encoded}"" }}
            }}");

            var liveGrid = TacSimTerrainDecoder.DecodeArenaMapFrame(mapNode);
            Assert.IsNotNull(liveGrid);
            Assert.AreEqual(4, liveGrid.Width);
            Assert.AreEqual(2, liveGrid.Height);
            Assert.AreEqual(TerrainCellClass.Sand, liveGrid.GetCellClass(0, 0));
            Assert.AreEqual(TerrainCellClass.Road, liveGrid.GetCellClass(2, 0));
            Assert.AreEqual(TerrainCellClass.Rock, liveGrid.GetCellClass(0, 1));
            Assert.AreEqual(18.50f, liveGrid.ElevMeters[4], 0.01f);
            Assert.AreEqual(1, liveGrid.KeySpots.Count);
            Assert.AreEqual("A", liveGrid.KeySpots[0].Letter);

            // Verify authored 256x256 4-quadrant Mars Canyon arena + Red/Blue team spawns load from crates/tacsim/content/
            var mars = TacSimTerrainDecoder.LoadStaticArena("mars_canyon");
            Assert.IsNotNull(mars);
            Assert.AreEqual(256, mars.Width);
            Assert.AreEqual(256, mars.Height);
            Assert.GreaterOrEqual(mars.KeySpots.Count, 2);
            Assert.GreaterOrEqual(mars.Spawns.Count, 12, "Mars Canyon must load 6 Red + 6 Blue team spawn poses");

            // Verify unseated `cockpit_cleared()` frame (`{"seated":false}`) never activates the 3D Arena overlay on login!
            var unseated = JsonNode.Parse(@"{""schema"":{""id"":""arena.cockpit"",""version"":9,""retain"":""latest""},""seated"":false}");
            Assert.IsFalse(TacSim.Arena.OocArenaController.IsLiveCockpitFrameSeated(unseated));

            var seated = JsonNode.Parse(@"{""schema"":{""id"":""arena.cockpit"",""version"":9},""match"":42,""callsign"":""R1"",""party"":""R""}");
            Assert.IsTrue(TacSim.Arena.OocArenaController.IsLiveCockpitFrameSeated(seated));
        }

        [Test]
        public void P0_JsonMergeObjectDelta_DeletesNullKeys_AndMergesNestedObjects()
        {
            var state = JsonNode.Parse(@"{""callsign"":""R1"",""lock"":{""target"":""B2"",""odds"":37},""banner"":""HOLD ZONE""}");
            var delta = JsonNode.Parse(@"{""lock"":null,""banner"":null,""speed"":42,""nested"":{""a"":1}}");
            state.MergeObjectDelta(delta);

            Assert.AreEqual("R1", state.Str("callsign"));
            Assert.IsFalse(state.ContainsKey("lock"), "Null delta value must remove key from merged panel state");
            Assert.IsFalse(state.ContainsKey("banner"), "Null delta value must remove cleared banner key");
            Assert.AreEqual(42, state.Int("speed"));
            Assert.AreEqual(1, state["nested"].Int("a"));
        }

        [Test]
        public void P0_RichTextRenderer_StylesSpansCommandLinks_AndEscapesAngleBrackets()
        {
            var rich = JsonNode.Parse(@"{
                ""spans"": [
                    { ""text"": ""Alert <N>: "", ""tag"": ""alert"", ""style"": { ""bold"": true } },
                    { ""text"": ""Salt Moth"", ""style"": { ""fg"": { ""rgb"": [85, 255, 170] }, ""italic"": true } },
                    { ""text"": "" "" },
                    { ""text"": ""+ship/board"", ""link"": { ""cmd"": ""+ship/board"" } }
                ]
            }");

            string markup = MushProtocol.RenderRichTextToUnityMarkup(rich);
            Assert.IsTrue(markup.Contains("‹N›"), "Literal angle brackets in span text must be escaped so uGUI RichText cannot break");
            Assert.IsTrue(markup.Contains("<color=#FF7766><b>Alert ‹N›: </b></color>"));
            Assert.IsTrue(markup.Contains("<color=#55FFAA><i>Salt Moth</i></color>"));
            Assert.IsTrue(markup.Contains("[+ship/board]"));

            // Verify ClientCommand::Caps and Composing serialization
            var caps = JsonNode.Parse(MushProtocol.BuildCapsCommand(120, 40));
            Assert.AreEqual("caps", caps.Str("t"));
            Assert.AreEqual("web", caps["d"].Str("kind"));
            Assert.AreEqual("true_color", caps["d"].Str("color"));
            Assert.IsTrue(caps["d"].Bool("structured"));

            var compCmd = JsonNode.Parse(MushProtocol.BuildComposingCommand(true));
            Assert.AreEqual("composing", compCmd.Str("t"));
            Assert.IsTrue(compCmd["d"].Bool("active"));

            var compEvt = MushProtocol.ParseComposing(JsonNode.Parse(@"{""actor"":{""id"":""#77"",""name"":""Ilse""},""active"":true}"));
            Assert.IsNotNull(compEvt);
            Assert.AreEqual("#77", compEvt.Actor.Id);
            Assert.IsTrue(compEvt.Active);
        }

        [Test]
        public void P0_ViewCursorAndWsReconnect_EnforceSequenceContinuityAndCloseCodes()
        {
            var cursor = new ViewStreamCursor();

            // Canonical Rust ViewEnvelope has NO "kind" field — base_sequence presence distinguishes deltas!
            var orphanDelta = JsonNode.Parse(@"{""view"":""arena.cockpit"",""generation"":""g1"",""base_sequence"":1,""sequence"":2,""data"":{""speed"":3}}");
            Assert.AreEqual(ViewCursorDisposition.NeedsResubscribe, MushProtocol.EvaluateViewCursor(cursor, orphanDelta));

            // Canonical snapshot (no base_sequence) establishes baseline
            var snap = JsonNode.Parse(@"{""view"":""arena.cockpit"",""generation"":""g1"",""sequence"":5,""data"":{""speed"":2,""health"":100}}");
            Assert.AreEqual(ViewCursorDisposition.ApplySnapshot, MushProtocol.EvaluateViewCursor(cursor, snap));
            Assert.AreEqual(5UL, cursor.LastSequence);

            // Canonical contiguous delta (with base_sequence, no "kind") merges without losing health!
            var delta6 = JsonNode.Parse(@"{""view"":""arena.cockpit"",""generation"":""g1"",""base_sequence"":5,""sequence"":6,""data"":{""speed"":3}}");
            Assert.AreEqual(ViewCursorDisposition.ApplyDelta, MushProtocol.EvaluateViewCursor(cursor, delta6));
            Assert.AreEqual(6UL, cursor.LastSequence);

            // Verify end-to-end MushWsClient ViewEnvelope delta preserves existing snapshot fields
            using (var ws = new MushWsClient())
            {
                JsonNode latestView = null;
                ws.OnPanelUpdated += (id, payload) => { if (id == "arena.cockpit") latestView = payload; };
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":1,""ts"":10,""t"":""view"",""d"":{""view"":""arena.cockpit"",""generation"":""g1"",""sequence"":5,""data"":{""speed"":2,""health"":100}}}"));
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":2,""ts"":11,""t"":""view"",""d"":{""view"":""arena.cockpit"",""generation"":""g1"",""base_sequence"":5,""sequence"":6,""data"":{""speed"":3}}}"));
                ws.PumpMainThread();
                Assert.IsNotNull(latestView);
                Assert.AreEqual(3, latestView.Int("speed"));
                Assert.AreEqual(100, latestView.Int("health"), "Canonical ViewEnvelope delta must merge into snapshot without dropping unchanged keys");
            }

            // Duplicate sequence is ignored
            Assert.AreEqual(ViewCursorDisposition.IgnoreDuplicateOrStale, MushProtocol.EvaluateViewCursor(cursor, delta6));

            // Gap in base_sequence triggers clean resubscribe
            var gappedDelta = JsonNode.Parse(@"{""view"":""arena.cockpit"",""generation"":""g1"",""base_sequence"":8,""sequence"":9,""data"":{}}");
            Assert.AreEqual(ViewCursorDisposition.NeedsResubscribe, MushProtocol.EvaluateViewCursor(cursor, gappedDelta));

            // Generation rotation on delta triggers resubscribe
            var wrongGen = JsonNode.Parse(@"{""view"":""arena.cockpit"",""generation"":""g2"",""base_sequence"":6,""sequence"":7,""data"":{}}");
            Assert.AreEqual(ViewCursorDisposition.NeedsResubscribe, MushProtocol.EvaluateViewCursor(cursor, wrongGen));

            // WS Close Code & Query invariants
            Assert.AreEqual(WsCloseDisposition.FatalSessionEnded, MushWsClient.ClassifyCloseCode(4401));
            Assert.AreEqual(WsCloseDisposition.Cooloff60s, MushWsClient.ClassifyCloseCode(4429));
            Assert.AreEqual(WsCloseDisposition.ReconnectBackoff, MushWsClient.ClassifyCloseCode(1012));
            Assert.AreEqual("char=42", MushWsClient.BuildWsQuery("42", 0UL));
            Assert.AreEqual("char=42&resume=108", MushWsClient.BuildWsQuery("42", 108UL));

            // Bare remote hosts default to https:// / wss://; loopback stays http:// / ws://
            Assert.AreEqual("https://dev1.conclav.io", MushAuthClient.NormalizeBaseUrl("dev1.conclav.io"));
            Assert.AreEqual("wss://dev1.conclav.io", MushAuthClient.ToWebSocketBaseUrl("dev1.conclav.io"));
            Assert.AreEqual("http://127.0.0.1:8080", MushAuthClient.NormalizeBaseUrl("127.0.0.1:8080"));
            Assert.AreEqual("http://localhost:8080", MushAuthClient.NormalizeBaseUrl("localhost:8080"));
        }

        [Test]
        public void P0_WsClient_ResetsSeqOnFreshIncarnation_AndEpochFencesDisconnectedQueue()
        {
            using (var ws = new MushWsClient())
            {
                int dispatchedRooms = 0;
                ws.OnRoomReceived += _ => dispatchedRooms++;

                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":1000,""ts"":1,""t"":""room"",""d"":{""id"":""#1"",""name"":""Arrivals""}}"));
                ws.PumpMainThread();
                Assert.AreEqual(1000UL, ws.LastSeq);
                Assert.AreEqual(1, dispatchedRooms);

                // Non-replayed resume resets cursor to 0, then fresh incarnation seq=1 updates LastSeq=1!
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":0,""ts"":2,""t"":""resumed"",""d"":{""replayed"":false}}"));
                ws.PumpMainThread();
                Assert.AreEqual(0UL, ws.LastSeq);

                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":1,""ts"":3,""t"":""room"",""d"":{""id"":""#1"",""name"":""Arrivals""}}"));
                ws.PumpMainThread();
                Assert.AreEqual(1UL, ws.LastSeq);
                Assert.AreEqual(2, dispatchedRooms);

                // Queued inbound message followed by Disconnect() must NOT dispatch on PumpMainThread()!
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":2,""ts"":4,""t"":""room"",""d"":{""id"":""#2"",""name"":""Concourse""}}"));
                ws.Disconnect();
                ws.PumpMainThread();
                Assert.AreEqual(2, dispatchedRooms, "Disconnect() must drain and epoch-fence queued envelopes");
            }
        }

        [Test]
        public void DynamicPackSync_VerifiesSha256DiffsManifest_AndOverridesLoader()
        {
            try
            {
                string sampleJson = @"{""id"":""tacsim.goods"",""goods"":[]}";
                string expectedSha = TacSimContentSync.ComputeSha256Hex(sampleJson);
                Assert.AreEqual(64, expectedSha.Length);

                var manifestNode = JsonNode.Parse($@"{{
                    ""version"": 1,
                    ""packs"": [
                        {{ ""id"": ""tacsim.goods"", ""category"": ""economy"", ""sha256"": ""{expectedSha}"", ""bytes"": {sampleJson.Length} }},
                        {{ ""id"": ""neighborhood.ceres_freeport"", ""category"": ""atlas"", ""sha256"": ""deadbeef"", ""bytes"": 120 }}
                    ]
                }}");

                var remote = TacSimContentSync.ParseManifestPacks(manifestNode);
                Assert.AreEqual(2, remote.Count);

                var local = new Dictionary<string, string> { { "tacsim.goods", expectedSha } };
                var changed = TacSimContentSync.ComputeChangedPacks(local, remote);
                Assert.AreEqual(1, changed.Count);
                Assert.AreEqual("neighborhood.ceres_freeport", changed[0].Id);

                var sync = new TacSimContentSync();
                Assert.IsFalse(sync.StoreVerifiedPack("tacsim.goods", "0000badsha", sampleJson), "Tampered digest must be rejected");
                Assert.IsTrue(sync.StoreVerifiedPack("tacsim.goods", expectedSha, sampleJson), "Matching SHA-256 pack must register");
                Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("tacsim.goods", out string fetched));
                Assert.AreEqual(sampleJson, fetched);
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
            }
        }

        [Test]
        public void AdditiveServerWire_ParsesTypedOccupantsDockedHullsSimulatorAndPortTradeFeed()
        {
            var roomJson = JsonNode.Parse(@"{
                ""id"": ""#90"",
                ""name"": ""Simulator Lobby"",
                ""desc"": ""Pod bays north and south."",
                ""contents"": [
                    { ""id"": ""#12"", ""name"": ""Pilot Vance"" },
                    { ""id"": ""#91"", ""name"": ""Selector Console"" }
                ],
                ""ambient"": {
                    ""atlas"": {
                        ""pack"": ""neighborhood.ooc_area"",
                        ""entry"": ""room.simulator_lobby"",
                        ""neighborhood_pack"": ""neighborhood.ooc_area"",
                        ""neighborhood_entry"": ""room.simulator_lobby""
                    },
                    ""players"": [""#12""],
                    ""things"": [""#91""],
                    ""tacsim"": {
                        ""mode"": ""simulator"",
                        ""room"": ""lobby"",
                        ""in_pod"": false,
                        ""match_state"": ""boarding"",
                        ""ai_fill"": 6,
                        ""my_team"": ""red"",
                        ""my_slot"": 0,
                        ""my_unit"": ""tacsim.unit.heavy_mk1"",
                        ""slots"": [
                            { ""team"": ""red"", ""index"": 0, ""pilot_id"": ""#12"", ""pilot_name"": ""Pilot Vance"", ""unit"": ""tacsim.unit.heavy_mk1"", ""seated"": false }
                        ],
                        ""own_stowage"": {
                            ""cargo_kg"": 34000, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 47600, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 62000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 45, ""stores_limit_days"": 120
                        },
                        ""docked_hulls"": [
                            { ""id"": ""#201"", ""name"": ""Salt Moth"", ""mark"": ""FPC-D-0409"", ""unit"": ""tacsim.unit.dray"" }
                        ]
                    }
                }
            }");

            var rv = MushProtocol.ParseRoomView(roomJson);
            Assert.IsTrue(rv.PlayerIds.Contains("#12"));
            Assert.IsTrue(rv.ThingIds.Contains("#91"));
            Assert.AreEqual("simulator", rv.TacSim.Mode);
            Assert.AreEqual("boarding", rv.TacSim.SimMatchState);
            Assert.AreEqual(6, rv.TacSim.SimAiFill);
            Assert.AreEqual("red", rv.TacSim.SimMyTeam);
            Assert.AreEqual(1, rv.TacSim.SimSlots.Count);
            Assert.AreEqual(1, rv.TacSim.DockedHulls.Count);
            Assert.AreEqual("Salt Moth", rv.TacSim.DockedHulls[0].Name);

            var trade = new TacSimTradeAndShipState();
            trade.ApplyStowageAmbient(rv.TacSim.OwnStowage);
            Assert.AreEqual(34, trade.HoldMassTonnes);
            Assert.AreEqual(47.6f, trade.HoldVolumeM3, 0.05f);
            Assert.AreEqual(62, trade.TankTonnes);
            Assert.AreEqual(45, trade.StoresCrewDays);

            var feed = JsonNode.Parse(@"{
                ""enabled"": true,
                ""venue"": ""ceres"",
                ""balance_text"": ""41250.00cr"",
                ""listings"": [
                    { ""id"": 5, ""side"": ""ask"", ""good"": ""volatiles"", ""good_label"": ""water ice"", ""available_qty"": 90, ""price_text"": ""102.00cr"", ""vendor_name"": ""Occator Water"" },
                    { ""id"": 12, ""side"": ""bid"", ""good"": ""structural_alloys"", ""good_label"": ""structural alloy stock"", ""available_qty"": 60, ""price_text"": ""415.00cr"", ""vendor_name"": ""Freeport Works"" }
                ],
                ""ship"": {
                    ""stowage"": {
                        ""cargo_kg"": 50000, ""cargo_limit_kg"": 100000,
                        ""cargo_l"": 70000, ""cargo_limit_l"": 120000,
                        ""tank_kg"": 75000, ""tank_limit_kg"": 80000,
                        ""stores_days"": 90, ""stores_limit_days"": 120
                    }
                }
            }");
            trade.ApplyPortTradeFeed(feed);
            Assert.AreEqual("41250.00cr", trade.CreditsText);
            Assert.AreEqual(2, trade.Listings.Count);
            Assert.IsFalse(trade.Listings[0].IsWantedBid);
            Assert.IsTrue(trade.Listings[1].IsWantedBid);
            Assert.AreEqual(50, trade.HoldMassTonnes);
            Assert.AreEqual(75, trade.TankTonnes);

            // Consolidated single-key `ambient.occupants`
            var occRoom = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                ""id"": ""#10"",
                ""name"": ""Concourse"",
                ""ambient"": {
                    ""occupants"": {
                        ""npcs"": [""#21""],
                        ""dormant"": [""#22""],
                        ""players"": [""#23""],
                        ""things"": [""#24""]
                    }
                }
            }"));
            Assert.IsTrue(occRoom.NpcIds.Contains("#21"));
            Assert.IsTrue(occRoom.DormantNpcIds.Contains("#22"));
            Assert.IsTrue(occRoom.PlayerIds.Contains("#23"));
            Assert.IsTrue(occRoom.ThingIds.Contains("#24"));
        }

        [Test]
        public void P0_ContentSync_IsolatesServers_NotifiesWarmCache_AndPacesWithinFloodBudget()
        {
            string tempDir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimSyncTest_" + System.Guid.NewGuid().ToString("N"));
            try
            {
                string serverAGoods = @"{""goods"":[{""id"":""metals"",""label"":""Server A Metal""}]}";
                string shaA = TacSimContentSync.ComputeSha256Hex(serverAGoods);

                var sync = new TacSimContentSync();
                var tradeState = new TacSimTradeAndShipState();
                int updateEvents = 0;
                sync.OnPacksUpdated += count =>
                {
                    updateEvents += 1;
                    tradeState.ReloadGoodsCatalog();
                };

                sync.InitializeDiskCacheForServer("https://server-a.example.com", tempDir);
                // Deliver manifest first so `goods` is requested in-flight, then deliver the verified pack reply.
                using (var ws = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{""op"":""manifest"",""packs"":[{{""id"":""goods"",""category"":""market"",""sha256"":""{shaA}"",""bytes"":{serverAGoods.Length}}}]}}"), 0.5f);
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""goods"",""requested_id"":""goods"",""sha256"":""{shaA}"",""json"":{JsonNode.FromString(serverAGoods).ToJson()}}}"), 1f);
                }
                Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _));
                Assert.IsTrue(tradeState.GoodsCatalog.Exists(g => g.Label == "Server A Metal"));

                // Switching to a COLD Server B (empty cache) MUST clear Server A's dynamic packs AND fire OnPacksUpdated to invalidate loaded models!
                int beforeColdSwitch = updateEvents;
                sync.InitializeDiskCacheForServer("https://server-b.example.com", tempDir);
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Switching servers must clear previous server's dynamic packs");
                Assert.Greater(updateEvents, beforeColdSwitch, "Cold-server switch must still fire OnPacksUpdated so loaded models invalidate");
                Assert.IsFalse(tradeState.GoodsCatalog.Exists(g => g.Label == "Server A Metal"), "Cold-server switch must evict Server A models from loaded catalog");

                // Switching back to Server A with warm disk cache MUST keep cached bytes private until
                // the current session's live manifest authorizes the pack ID & digest (no pre-manifest leakage!),
                // and then hydrate from cache with zero outbound downloads and fire OnPacksUpdated!
                sync.InitializeDiskCacheForServer("https://server-a.example.com", tempDir);
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Warm disk cache must stay private until live manifest authorizes each pack");
                int beforeWarmManifest = updateEvents;
                using (var wsWarm = new MushWsClient())
                {
                    var warmManifest = JsonNode.Parse($@"{{""op"":""manifest"",""packs"":[{{""id"":""goods"",""category"":""economy"",""sha256"":""{shaA}"",""bytes"":{serverAGoods.Length}}}]}}");
                    sync.HandleWebSocketContentPanel(wsWarm, warmManifest, nowRealtime: 4f);
                    Assert.AreEqual(0, wsWarm.OutboundQueueCount, "Authorized warm-cache pack must not be re-downloaded");
                }
                Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out string warmJson));
                Assert.AreEqual(serverAGoods, warmJson);
                Assert.Greater(updateEvents, beforeWarmManifest, "Manifest-authorized warm-cache activation must raise OnPacksUpdated so already-constructed models refresh");

                // Pruning check: delivering an authoritative empty manifest MUST delete removed packs from memory AND saved manifest!
                var emptyManifest = JsonNode.Parse(@"{""op"":""manifest"",""packs"":[]}");
                using (var wsPrune = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(wsPrune, emptyManifest, nowRealtime: 5f);
                }
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Pruned pack must be evicted from memory");
                // Re-hydrating Server A's disk cache and presenting a manifest requesting `goods` proves it was deleted from disk manifest (must re-fetch!)
                sync.InitializeDiskCacheForServer("https://server-a.example.com", tempDir);
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Pruned pack must not reappear from disk manifest");
                using (var wsAfterPrune = new MushWsClient())
                {
                    var manifestAfterPrune = JsonNode.Parse($@"{{""op"":""manifest"",""packs"":[{{""id"":""goods"",""category"":""economy"",""sha256"":""{shaA}"",""bytes"":{serverAGoods.Length}}}]}}");
                    sync.HandleWebSocketContentPanel(wsAfterPrune, manifestAfterPrune, nowRealtime: 6f);
                    Assert.AreEqual(1, wsAfterPrune.OutboundQueueCount, "Pruned pack must no longer be cached on disk and must require re-fetch");
                }

                // Inter-batch pacing check: 37-pack manifest with instantaneous completions at t=10f MUST emit strictly 4 requests at t=10f!
                sync.InitializeDiskCacheForServer("https://server-c.example.com", tempDir);
                var packsArr = JsonNode.NewArray();
                var packPayloads = new System.Collections.Generic.List<(string id, string sha, string json)>();
                for (int i = 0; i < 37; i++)
                {
                    string pj = $@"{{""unit"":{i}}}";
                    string psha = TacSimContentSync.ComputeSha256Hex(pj);
                    packPayloads.Add(($"unit_{i}", psha, pj));
                    var p = JsonNode.NewObject();
                    p["id"] = JsonNode.FromString($"unit_{i}");
                    p["category"] = JsonNode.FromString("unit");
                    p["sha256"] = JsonNode.FromString(psha);
                    p["bytes"] = JsonNode.FromLong(pj.Length);
                    packsArr.Add(p);
                }
                var bigManifest = JsonNode.NewObject();
                bigManifest["op"] = JsonNode.FromString("manifest");
                bigManifest["packs"] = packsArr;

                using (var ws = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(ws, bigManifest, nowRealtime: 10.0f);
                    Assert.AreEqual(4, ws.OutboundQueueCount, "Initial batch at t=10.0f must emit at most 4 pack requests");

                    // Immediately complete all 4 in-flight packs at the exact same timestamp t=10.0f:
                    int updatesBeforeBatch = updateEvents;
                    for (int i = 0; i < 4; i++)
                    {
                        var (pid, psha, pjson) = packPayloads[i];
                        var reply = JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""{pid}"",""category"":""unit"",""sha256"":""{psha}"",""json"":{JsonNode.FromString(pjson).ToJson()}}}");
                        sync.HandleWebSocketContentPanel(ws, reply, nowRealtime: 10.0f);
                    }
                    Assert.AreEqual(updatesBeforeBatch, updateEvents, "Partial cold-sync batch must coalesce and NOT rebuild scenes mid-sync");
                    // Completing a batch at t=10.0f MUST NOT emit another batch before MinBatchIntervalSec (0.45s)!
                    sync.PumpPackQueue(ws, nowRealtime: 10.10f);
                    Assert.AreEqual(4, ws.OutboundQueueCount, "Pacing must hold between batches before 0.45s has elapsed");

                    // Advancing past 0.45s dispatches exactly the next batch of 4 (total 8):
                    sync.PumpPackQueue(ws, nowRealtime: 10.50f);
                    Assert.AreEqual(8, ws.OutboundQueueCount, "Next batch of 4 dispatches once 0.45s interval elapses");

                    // Drain the rest of the 37-pack cold sync and verify OnPacksUpdated fires ONCE at completion!
                    float simTime = 10.50f;
                    int nextCompleteIdx = 4;
                    while (!sync.IsSyncComplete && simTime < 60f)
                    {
                        while (nextCompleteIdx < ws.OutboundQueueCount && nextCompleteIdx < packPayloads.Count)
                        {
                            var (pid, psha, pjson) = packPayloads[nextCompleteIdx++];
                            var reply = JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""{pid}"",""category"":""unit"",""sha256"":""{psha}"",""json"":{JsonNode.FromString(pjson).ToJson()}}}");
                            sync.HandleWebSocketContentPanel(ws, reply, nowRealtime: simTime);
                        }
                        simTime += 0.50f;
                        sync.PumpPackQueue(ws, nowRealtime: simTime);
                    }
                    Assert.IsTrue(sync.IsSyncComplete);
                    Assert.AreEqual(updatesBeforeBatch + 1, updateEvents, "37-pack cold sync must trigger exactly ONE coalesced OnPacksUpdated rebuild at completion");
                }

                // Finite retry check: a pack that repeatedly times out MUST stop after MaxPackAttempts (3), never looping forever!
                sync.InitializeDiskCacheForServer("https://server-d.example.com", tempDir);
                string retryJson = @"{""x"":1}";
                string retrySha = TacSimContentSync.ComputeSha256Hex(retryJson);
                var oneManifest = JsonNode.Parse($@"{{""op"":""manifest"",""packs"":[{{""id"":""flaky_pack"",""category"":""unit"",""sha256"":""{retrySha}"",""bytes"":7}}]}}");
                using (var wsRetry = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(wsRetry, oneManifest, nowRealtime: 0f);
                    for (int cycle = 1; cycle <= 10; cycle++)
                    {
                        sync.PumpPackQueue(wsRetry, nowRealtime: cycle * 6.0f);
                    }
                    Assert.AreEqual(TacSimContentSync.MaxPackAttempts, wsRetry.OutboundQueueCount, "Timed-out pack must stop retrying at MaxPackAttempts");
                }
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
                try { if (System.IO.Directory.Exists(tempDir)) System.IO.Directory.Delete(tempDir, recursive: true); } catch { }
            }
        }

        [Test]
        public void P0_SurfaceSiteMap512_RigHopperTelemetry_AndDockingTapeBudgetHold()
        {
            // 1. Parse canonical `tv::hopper_text` ("3 of 6 t", "12 of 12 t", "empty", and "3/6 t")
            Assert.IsTrue(TacSim.Surface.SurfaceMiningController.TryParseHopperTonnes("3 of 6 t", out int loadedOf, out int capOf, out bool hasCapOf));
            Assert.AreEqual(3, loadedOf);
            Assert.AreEqual(6, capOf);
            Assert.IsTrue(hasCapOf);

            Assert.IsTrue(TacSim.Surface.SurfaceMiningController.TryParseHopperTonnes("12 of 12 t", out int loaded12, out int cap12, out bool hasCap12));
            Assert.AreEqual(12, loaded12);
            Assert.AreEqual(12, cap12);
            Assert.IsTrue(hasCap12);

            Assert.IsTrue(TacSim.Surface.SurfaceMiningController.TryParseHopperTonnes("empty", out int emptyT, out _, out bool hasCapEmpty));
            Assert.AreEqual(0, emptyT);
            Assert.IsFalse(hasCapEmpty, "'empty' carries no capacity figure so prior unit capacity is preserved");

            // 2. Verify 28x28 drive window (`origin [114, 114]`, rig `[128, 128]`) maps to +145m (never double-subtracting to -995m!)
            var driveWindowFrame = JsonNode.Parse(@"{
                ""width"": 28,
                ""height"": 28,
                ""cell_cm"": 1000,
                ""rows"": [[["","", 28]]],
                ""you"": [14, 14],
                ""origin_cell"": [114, 114],
                ""site"": {
                    ""site"": ""Occator Rim"",
                    ""addressed"": 1,
                    ""grid"": { ""x0"": 114, ""y0"": 114, ""span"": 1, ""cell_cm"": 1000 },
                    ""clock"": { ""now_ic"": 1005, ""rate"": 2.0 },
                    ""rigs"": [
                        {
                            ""slot"": 1,
                            ""cell"": [128, 128],
                            ""charge_pct"": 82,
                            ""hopper"": ""3 of 6 t"",
                            ""leg"": {
                                ""started_ic"": 1000,
                                ""waypoints"": [
                                    { ""x_cm"": 128500, ""y_cm"": 128500, ""at_s"": 0 },
                                    { ""x_cm"": 130500, ""y_cm"": 128500, ""at_s"": 10 },
                                    { ""x_cm"": 130500, ""y_cm"": 132500, ""at_s"": 30 }
                                ]
                            }
                        }
                    ]
                }
            }");
            var driveGrid = TacSimTerrainDecoder.DecodeSiteMapFrame(driveWindowFrame);
            Assert.IsNotNull(driveGrid);
            Assert.AreEqual(114, driveGrid.OriginCellX);
            Assert.AreEqual(114, driveGrid.OriginCellY);
            UnityEngine.Vector3 rigLocalViaSite = driveGrid.SiteCellToLocalPos(128, 128, 0f);
            UnityEngine.Vector3 rigLocalViaCell = driveGrid.CellToLocalPos(driveGrid.SiteToLocalCellX(128), driveGrid.SiteToLocalCellY(128), 0f);
            Assert.AreEqual(145f, rigLocalViaSite.x, 0.05f, "Rig at [128,128] on window [114,114] must land at +145m, never -995m");
            Assert.AreEqual(145f, rigLocalViaSite.z, 0.05f);
            Assert.AreEqual(145f, rigLocalViaCell.x, 0.05f);
            Assert.AreEqual(145f, rigLocalViaCell.z, 0.05f);

            // 3. Verify `{x_cm, y_cm, at_s}` object waypoint parsing & piecewise IC leg interpolation
            var wps = new System.Collections.Generic.List<TacSim.Surface.RoverLegWaypoint>();
            var legJson = driveWindowFrame["site"]["rigs"][0]["leg"];
            Assert.IsTrue(TacSim.Surface.SurfaceMiningController.ParseLegWaypoints(legJson, out long startedIc, wps));
            Assert.AreEqual(1000L, startedIc);
            Assert.AreEqual(3, wps.Count);
            // At t=+5s IC: halfway along East leg (128500 -> 130500 = 129500 cm -> +155m local X, +145m local Z)
            Assert.IsTrue(TacSim.Surface.SurfaceMiningController.EvaluateLegPositionCm(wps, 5.0, out double x5, out double y5, out float h5, out _, out bool mov5));
            Assert.AreEqual(129500.0, x5, 0.5);
            Assert.AreEqual(128500.0, y5, 0.5);
            Assert.AreEqual(90f, h5, 0.5f, "Eastward segment should face ~90 deg");
            Assert.IsTrue(mov5);
            UnityEngine.Vector3 pos5 = driveGrid.SiteCmToLocalPos(x5, y5, 0f);
            Assert.AreEqual(155f, pos5.x, 0.05f);
            Assert.AreEqual(145f, pos5.z, 0.05f);
            // At t=+20s IC: halfway along North turn leg (130500, 130500 -> +165m local X, +165m local Z, facing 0 deg North)
            Assert.IsTrue(TacSim.Surface.SurfaceMiningController.EvaluateLegPositionCm(wps, 20.0, out double x20, out double y20, out float h20, out _, out bool mov20));
            Assert.AreEqual(130500.0, x20, 0.5);
            Assert.AreEqual(130500.0, y20, 0.5);
            Assert.AreEqual(0f, h20, 0.5f, "Northward segment should face 0 deg");
            Assert.IsTrue(mov20);

            // 4. Genuine Rust-emitted `site.map` fixtures (`site_map_refused.json`, `site_map_512_site.json`, `site_map_512_drive.json`)
            var refusedNode = JsonNode.Parse(ReadRustFixtureJson("site_map_refused.json"));
            Assert.IsNull(TacSimTerrainDecoder.DecodeSiteMapFrame(refusedNode), "Refused/empty site.map frame must return null, never inventing a 4x4 terrain patch");

            var site512Node = JsonNode.Parse(ReadRustFixtureJson("site_map_512_site.json"));
            var site512Grid = TacSimTerrainDecoder.DecodeSiteMapFrame(site512Node);
            Assert.IsNotNull(site512Grid);
            Assert.IsTrue(site512Grid.IsFromLiveServerFeed);
            Assert.AreEqual(32, site512Grid.Width);
            Assert.AreEqual(32, site512Grid.Height);
            Assert.AreEqual(16, site512Grid.SpanCells);
            Assert.AreEqual(160f, site512Grid.CellSizeM, 0.01f);
            Assert.IsTrue(site512Grid.HasAuthoritativeHullCell);
            Assert.AreEqual(16, site512Grid.HullFrameX);
            Assert.AreEqual(16, site512Grid.HullFrameY);

            var drive512Node = JsonNode.Parse(ReadRustFixtureJson("site_map_512_drive.json"));
            var grid = TacSimTerrainDecoder.DecodeSiteMapFrame(drive512Node);
            Assert.IsNotNull(grid);
            Assert.IsTrue(grid.IsFromLiveServerFeed);
            Assert.AreEqual(28, grid.Width);
            Assert.AreEqual(1, grid.SpanCells);
            Assert.AreEqual(242, grid.OriginCellX);
            Assert.AreEqual(238, grid.OriginCellY);
            Assert.IsTrue(grid.HasAuthoritativeHullCell, "Pad placement reads authoritative site.marks.hull");
            Assert.AreEqual(14, grid.HullFrameX);
            Assert.AreEqual(18, grid.HullFrameY);
            Assert.AreEqual(14, grid.SiteToLocalCellX(256));
            Assert.AreEqual(18, grid.SiteToLocalCellY(256));
            Assert.AreEqual(256, grid.LocalToSiteCellX(14));
            Assert.AreEqual(257, grid.LocalToSiteCellX(15));
            Assert.IsTrue(grid.IsSiteCellInWindow(256, 252));
            Assert.IsFalse(grid.IsSiteCellInWindow(100, 100), "Out-of-window other rigs must be skipped rather than pinned to window edge");
            Assert.IsFalse(grid.IsSiteCellInWindow(300, 300));

            // 5. Canonical `space.holotank` (`x_km`/`y_km`/`z_km`, `you: true`) & `space.cockpit` (`fire.lock` object, `mining`)
            var ownShipNode = JsonNode.Parse(@"{""id"":""ship.7"",""you"":true,""x_km"":3.8,""y_km"":0.0,""z_km"":0.0}");
            var contactAhead = JsonNode.Parse(@"{""id"":""corvette_1"",""kind"":""ship"",""you"":false,""x_km"":15.8,""y_km"":0.0,""z_km"":0.0}");
            var contactStarboardHigh = JsonNode.Parse(@"{""id"":""raider_1"",""kind"":""ship"",""you"":false,""x_km"":3.8,""y_km"":42.0,""z_km"":12.0}");
            UnityEngine.Vector3 offAhead = TacSim.Space.SpaceFlightController.ComputeRelativeStageOffset(contactAhead, 3.8, 0.0, 0.0, 85f, 1);
            UnityEngine.Vector3 offStarboard = TacSim.Space.SpaceFlightController.ComputeRelativeStageOffset(contactStarboardHigh, 3.8, 0.0, 0.0, 85f, 2);
            Assert.Greater(UnityEngine.Vector3.Distance(offAhead, offStarboard), 60f, "Widely separated x_km/y_km/z_km server contacts must never collapse to the same position");
            Assert.Greater(offAhead.z, 50f, "+x_km ahead maps to +Z forward");
            Assert.AreEqual(0f, offAhead.x, 0.01f);
            Assert.Greater(offStarboard.x, 70f, "+y_km starboard maps to +X");
            Assert.Greater(offStarboard.y, 15f, "+z_km elevation maps to +Y");
            Assert.IsNotNull(ownShipNode);

            var fireWithObjectLock = JsonNode.Parse(@"{""mounts"":[],""lock_id"":""raider_1"",""lock"":{""id"":""raider_1"",""label"":""Corsair"",""range_m"":14200,""facet"":""fore""}}");
            Assert.IsTrue(TacSim.Space.SpaceFlightController.ExtractLockedContactIdFromFireNode(fireWithObjectLock, out string parsedLock, out bool parsedClear));
            Assert.AreEqual("raider_1", parsedLock);
            Assert.IsFalse(parsedClear);

            var fireCleared = JsonNode.Parse(@"{""mounts"":[],""lock_id"":null,""lock"":null}");
            Assert.IsTrue(TacSim.Space.SpaceFlightController.ExtractLockedContactIdFromFireNode(fireCleared, out string emptyLock, out bool shouldClear));
            Assert.AreEqual(string.Empty, emptyLock);
            Assert.IsTrue(shouldClear);

            // 6. Alternating RCS stress input hits IsTapeBudgetExhausted before exceeding 8,192-char gateway limit
            var stress = DockingClientSession.CreateStandardPortPractice("dock.stress.1", "Ceres");
            for (int f = 0; f < 1800 && !stress.IsTapeBudgetExhausted; f++)
            {
                stress.Step60Fps(new DockingControlInput { TransX = (sbyte)(f % 2 == 0 ? 1 : -1) });
            }
            Assert.IsTrue(stress.IsTapeBudgetExhausted);
            Assert.LessOrEqual(stress.FormatTapeCommand().Length, 8192);

            // 7. Completed rover leg holds endpoint without drifting backward toward leg start!
            var legRun = new System.Collections.Generic.List<TacSim.Surface.RoverLegWaypoint>(wps);
            UnityEngine.Vector3 curRoverLocal = new UnityEngine.Vector3(145f, 0f, 145f);
            UnityEngine.Vector3 fallbackTarget = new UnityEngine.Vector3(145f, 0f, 145f); // stale start target before completion
            bool hasDriveWp = true;
            float heading = 90f;
            float spd = 0f;
            int scx = 128, scy = 128;
            // Frame at leg completion (elapsed = 30.0s -> endpoint x_cm=130500, y_cm=132500 -> local (165m, 185m))
            TacSim.Surface.SurfaceMiningController.AdvanceRoverKinematicsStep(
                driveGrid, legRun, 30.0, 1.0, 0.05f,
                ref curRoverLocal, ref fallbackTarget, ref hasDriveWp, ref heading, ref spd, ref scx, ref scy);
            Assert.AreEqual(0, legRun.Count, "Completed leg clears waypoint list");
            Assert.IsFalse(hasDriveWp, "Completed leg clears fallback _hasDriveWaypoint");
            Assert.AreEqual(165f, curRoverLocal.x, 0.05f);
            Assert.AreEqual(185f, curRoverLocal.z, 0.05f);
            // Subsequent Update frame while awaiting server completion frame MUST hold (165m, 185m), never reversing toward 145m!
            TacSim.Surface.SurfaceMiningController.AdvanceRoverKinematicsStep(
                driveGrid, legRun, 31.0, 1.0, 0.1f,
                ref curRoverLocal, ref fallbackTarget, ref hasDriveWp, ref heading, ref spd, ref scx, ref scy);
            Assert.AreEqual(165f, curRoverLocal.x, 0.01f, "Rover must hold completed endpoint, never reversing toward old target");
            Assert.AreEqual(185f, curRoverLocal.z, 0.01f);

            // 8. Genuine Rust-emitted live-reconciled neighborhood fixture (`reconciled_neighborhood.json`)
            try
            {
                string reconciledJson = ReadRustFixtureJson("reconciled_neighborhood.json");
                TacSimContentLoader.RegisterDynamicPack("neighborhood.test_station", reconciledJson);
                var reconciledPack = TacSimContentLoader.LoadNeighborhoodPackById("neighborhood.test_station");
                Assert.IsNotNull(reconciledPack);
                Assert.IsTrue(reconciledPack.Rooms.ContainsKey("room.existing"));
                var movedRoom = reconciledPack.Rooms["room.existing"];
                Assert.AreEqual(100f, movedRoom.MinX, 0.01f);
                Assert.AreEqual(200f, movedRoom.MinY, 0.01f);
                Assert.AreEqual(120f, movedRoom.MaxX, 0.01f);
                Assert.AreEqual(220f, movedRoom.MaxY, 0.01f);
                Assert.AreEqual(3, movedRoom.FloorIndex);

                Assert.IsTrue(reconciledPack.Rooms.ContainsKey("room.synth"), "Synthesized room with footprint.rect must not be discarded");
                var synthRoom = reconciledPack.Rooms["room.synth"];
                Assert.AreEqual(40f, synthRoom.MinX, 0.01f);
                Assert.AreEqual(80f, synthRoom.MaxY, 0.01f);
                Assert.AreEqual(2, synthRoom.FloorIndex);
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
            }

            // 9. Orbital velocity-match workflow (`space:helm_match` -> `+helm/match A`, gating laser & drones on matched_rock)
            string matchCmd = TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:helm_match", "A", true, "", out string matchRefusal);
            Assert.AreEqual("+helm/match A", matchCmd);
            Assert.IsNull(matchRefusal);

            // Unmatched laser & drones MUST refuse without emitting a command
            Assert.IsNull(TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:mine_laser", "A", true, "", out string laserRefusal));
            Assert.IsNotNull(laserRefusal);
            Assert.IsNull(TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:mine_drones", "A", true, "B", out string droneRefusal));
            Assert.IsNotNull(droneRefusal);

            // Authoritative `cockpit.mining` confirms velocity match on Rock A -> laser & drones permitted
            var matchedCockpit = JsonNode.Parse(@"{""mining"":{""cluster"":""ceres_halo"",""matched_rock"":""A"",""approaching_rock"":null,""active_drones"":1,""laser"":{""rock"":""A"",""yield_t"":4}}}");
            TacSim.Space.SpaceFlightController.ParseClusterMiningState(
                matchedCockpit, out bool hasMining, out string authMatched, out string authApproaching, out int authDrones, out string authLaserRock);
            Assert.IsTrue(hasMining);
            Assert.AreEqual("A", authMatched);
            Assert.AreEqual(string.Empty, authApproaching);
            Assert.AreEqual(1, authDrones);
            Assert.AreEqual("A", authLaserRock);

            Assert.AreEqual("+mine/laser A", TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:mine_laser", "A", hasMining, authMatched, out _));
            Assert.AreEqual("+mine/drones A 1", TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:mine_drones", "A", hasMining, authMatched, out _));

            // Leaving cluster (`mining` absent) clears `hasMiningBlock`
            var outsideClusterCockpit = JsonNode.Parse(@"{""yaw_deg"":15,""emcon"":""STD""}");
            TacSim.Space.SpaceFlightController.ParseClusterMiningState(
                outsideClusterCockpit, out bool outsideHasMining, out string outsideMatched, out _, out _, out _);
            Assert.IsFalse(outsideHasMining);
            Assert.AreEqual(string.Empty, outsideMatched);

            // 10. Zero/unknown buy price refuses to arm auto-accept; sailing-plan header reset, `struck`, & `No sailing plan`
            var ts = new TacSimTradeAndShipState();
            Assert.IsNull(ts.BeginOneClickBuy(99UL, 5, 1.0f, displayedUnitPriceMinor: 0), "Zero displayed price must never arm an unbounded auto-accept");
            Assert.IsTrue(ts.IngestServerLine("The Salt Moth's sailing plan (FPC-D-0401), revision 3: running.", 1f, out _));
            Assert.IsTrue(ts.IngestServerLine("  1. buy 20 t water ice — next", 1f, out _));
            Assert.IsTrue(ts.IngestServerLine("  2. depart Stickney Yards — pending", 1f, out _));
            Assert.AreEqual(2, ts.SailingOrderLines.Count);
            // New listing header resets previous rows instead of duplicating
            Assert.IsTrue(ts.IngestServerLine("The Salt Moth's sailing plan (FPC-D-0401), revision 4: running.", 2f, out _));
            Assert.AreEqual(0, ts.SailingOrderLines.Count);
            Assert.IsTrue(ts.IngestServerLine("  1. buy 20 t water ice — next", 2f, out _));
            Assert.IsTrue(ts.IngestServerLine("  2. depart Stickney Yards — pending", 2f, out _));
            // Striking order 2 removes only order 2
            Assert.IsTrue(ts.IngestServerLine("Order 2 is struck from the sailing plan.", 3f, out _));
            Assert.AreEqual(1, ts.SailingOrderLines.Count);
            // "No sailing plan" clears remaining rows
            Assert.IsTrue(ts.IngestServerLine("No sailing plan is filed for the Salt Moth.", 4f, out _));
            Assert.AreEqual(0, ts.SailingOrderLines.Count);
        }

        [Test]
        public void P0_Coordinator_SurfaceRoverRecovery_ExitsSurfaceMode_AndReturnToBaySendsOnce()
        {
            bool hadPreExistingEventSystem = UnityEngine.Object.FindAnyObjectByType<UnityEngine.EventSystems.EventSystem>() != null;
            var rootGo = new UnityEngine.GameObject("TestCoordinatorRoot");
            TacSimGameCoordinator coord = null;
            try
            {
                // Pre-attach isolated child subsystem GameObjects under `rootGo` so running in an open game scene
                // never mutates or destroys pre-existing scene objects (`StationWorld`, `Main Camera`, `TacSimHud`, etc.).
                coord = rootGo.AddComponent<TacSimGameCoordinator>();
                coord.Audio = rootGo.AddComponent<TacSimAudioSynthesizer>();

                var stationGo = new UnityEngine.GameObject("TestStationWorld");
                stationGo.transform.SetParent(rootGo.transform, false);
                coord.StationWorld = stationGo.AddComponent<StationWorldController>();

                var shipGo = new UnityEngine.GameObject("TestShipInterior");
                shipGo.transform.SetParent(rootGo.transform, false);
                coord.ShipInterior = shipGo.AddComponent<ShipInteriorController>();

                var spaceGo = new UnityEngine.GameObject("TestSpaceStage");
                spaceGo.transform.SetParent(rootGo.transform, false);
                coord.SpaceStage = spaceGo.AddComponent<SpaceFlightController>();

                var surfGo = new UnityEngine.GameObject("TestSurfaceStage");
                surfGo.transform.SetParent(rootGo.transform, false);
                coord.SurfaceStage = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();

                var arenaGo = new UnityEngine.GameObject("TestArenaStage");
                arenaGo.transform.SetParent(rootGo.transform, false);
                coord.ArenaStage = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();

                var camGo = new UnityEngine.GameObject("TestCameraRig", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                coord.CameraRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                coord.CameraRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();

                var playerGo = new UnityEngine.GameObject("TestPlayer", typeof(UnityEngine.CharacterController));
                playerGo.transform.SetParent(rootGo.transform, false);
                coord.Player = playerGo.AddComponent<TacSim.Character.ThirdPersonPlayerController>();
                coord.Player.VisualSpawner = playerGo.AddComponent<TacSim.Character.CharacterVisualSpawner>();

                var hudGo = new UnityEngine.GameObject("TestHud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();

                coord.EnsureSubsystems();

                var dispatched = new List<string>();
                coord.OnCommandDispatched += dispatched.Add;

                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onPanel = typeof(TacSimGameCoordinator).GetMethod("HandleServerPanelUpdated", flags);
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                var onRefreshSync = typeof(TacSimGameCoordinator).GetMethod("RefreshLoadedContentAfterSync", flags);
                var onSyncPos = typeof(TacSimGameCoordinator).GetMethod("Sync3DPositionWithServer", flags);
                var awaitingInitField = typeof(TacSimGameCoordinator).GetField("_awaitingInitialRoomSync", flags);
                var simConnField = typeof(TacSimGameCoordinator).GetField("_simulateConnectedForTests", flags);
                var nextMoveField = typeof(TacSimGameCoordinator).GetField("_nextMoveStepAllowedAt", flags);
                Assert.IsNotNull(onPanel);
                Assert.IsNotNull(onRoom);
                Assert.IsNotNull(onRefreshSync);
                Assert.IsNotNull(onSyncPos);
                Assert.IsNotNull(awaitingInitField);
                Assert.IsNotNull(simConnField);
                Assert.IsNotNull(nextMoveField);
                simConnField.SetValue(coord, true);

                // 1. Ceres Freeport multi-room walk check: walking ahead through Pressure Lobby into the 100m-wide Berth Gallery
                // (36m from room center) must NOT bounce the player backwards or flicker ActiveRoomEntryId back to Pressure Lobby!
                awaitingInitField.SetValue(coord, true);
                var arrivalsView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#103"",
                    ""name"": ""Port Watch Arrivals"",
                    ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c03_arrivals"", ""surface"": ""station"" } }
                }"));
                onRoom.Invoke(coord, new object[] { arrivalsView });
                Assert.AreEqual("room.c03_arrivals", coord.StationWorld.ActiveRoomEntryId);

                // Player walks south through Pressure Lobby into Berth Gallery doorway at atlas (125, 122) — 35.3m from Berth Gallery center (160, 117.5)
                float pubElev = coord.StationWorld.Pack.GetFloorElevationY(1);
                UnityEngine.Vector3 galleryDoorwayPos = coord.StationWorld.AtlasToUnity(125f, pubElev + 0.15f, 122f);
                coord.Player.TeleportTo(galleryDoorwayPos, 180f);
                nextMoveField.SetValue(coord, -1f);
                onSyncPos.Invoke(coord, null);
                Assert.AreEqual("room.c01_berth_gallery", coord.StationWorld.ActiveRoomEntryId);

                // Intermediate `RoomView` for `room.c02_pressure` arrives acknowledging hop 1 while player is already in `room.c01_berth_gallery`:
                var pressureView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#102"",
                    ""name"": ""Pressure Lobby"",
                    ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c02_pressure"", ""surface"": ""station"" } }
                }"));
                onRoom.Invoke(coord, new object[] { pressureView });
                Assert.AreEqual("room.c01_berth_gallery", coord.StationWorld.ActiveRoomEntryId, "Lagging intermediate RoomView must not flicker ActiveRoomEntryId back to old room");
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, galleryDoorwayPos), 0.01f, "Lagging intermediate RoomView must not bounce walking avatar back");

                // Final `RoomView` for 100m-wide `room.c01_berth_gallery` arrives while player is >32m from center:
                var galleryView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#101"",
                    ""name"": ""Visiting Berth Gallery"",
                    ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c01_berth_gallery"", ""surface"": ""station"" } }
                }"));
                onRoom.Invoke(coord, new object[] { galleryView });
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, galleryDoorwayPos), 0.01f, "Wide room >32m from center must not rubber-band avatar to center");

                // 2. Cold-cache standalone check: player enters an unsynced neighborhood (`neighborhood.test_station`),
                // initially falling back to Ceres Freeport; once sync completes, RefreshLoadedContentAfterSync switches immediately.
                var coldStationView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#500"",
                    ""name"": ""Live Moved Room"",
                    ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.test_station"", ""entry"": ""room.existing"", ""surface"": ""station"" } }
                }"));
                onRoom.Invoke(coord, new object[] { coldStationView });
                Assert.AreEqual("neighborhood.ceres_freeport", coord.StationWorld.LoadedPackId);

                TacSimContentLoader.RegisterDynamicPack("neighborhood.test_station", ReadRustFixtureJson("reconciled_neighborhood.json"));
                onRefreshSync.Invoke(coord, null);
                Assert.AreEqual("neighborhood.test_station", coord.StationWorld.LoadedPackId, "Post-sync refresh must rebuild into the player's actual room pack rather than keeping the fallback");
                Assert.AreEqual("room.existing", coord.StationWorld.ActiveRoomEntryId);

                // 3. Board settled hull's Vehicle Bay (`bay`)
                var bayRoomView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#810"",
                    ""name"": ""Salt Moth - Vehicle Bay"",
                    ""ambient"": {
                        ""tacsim"": {
                            ""mode"": ""interior"",
                            ""unit"": ""dray_p"",
                            ""room"": ""bay"",
                            ""hull_name"": ""Salt Moth"",
                            ""mark"": ""FPC-D-0401"",
                            ""class"": ""Dray-P"",
                            ""settled_pad"": true
                        }
                    }
                }"));
                onRoom.Invoke(coord, new object[] { bayRoomView });
                Assert.AreEqual(TacSimActiveStage.ShipInteriorWalk, coord.ActiveStage);
                Assert.AreEqual(TacSim.Character.CameraRigMode.ThirdPersonWalk, coord.CameraRig.Mode);
                Assert.IsFalse(coord.SurfaceStage.IsExcursionActive);
                Assert.IsFalse(coord.SurfaceStage.RigOnGround);
                Assert.IsFalse(coord.Player.PhysicsSuspended);
                Assert.IsTrue(coord.Hud.IsWalkQuickActionsVisible, "Walking in Vehicle Bay displays QuickActionStrip");

                UnityEngine.Vector3 bayDeckPos = coord.Player.transform.position;
                UnityEngine.Quaternion bayDeckRot = coord.Player.transform.rotation;

                // 4. Server emits live `site.map` with Rig 1 on the ground (`you: [14, 14]`, `rigs[0].cell: [256, 256]`)
                var driveFrame = JsonNode.Parse(ReadRustFixtureJson("site_map_512_drive.json"));
                onPanel.Invoke(coord, new object[] { "site.map", driveFrame });
                Assert.IsTrue(coord.SurfaceStage.RigOnGround);
                Assert.IsTrue(coord.SurfaceStage.IsExcursionActive);
                Assert.AreEqual(TacSimActiveStage.PlanetarySurface, coord.ActiveStage);
                Assert.AreEqual(TacSim.Character.CameraRigMode.GroundVehicleChase, coord.CameraRig.Mode);
                Assert.IsTrue(coord.Player.PhysicsSuspended, "Inactive walking avatar physics must be suspended while ship floor is disabled");
                Assert.IsFalse(coord.Hud.IsWalkQuickActionsVisible, "PlanetarySurface hides QuickActionStrip beneath SurfaceRoverActions");

                // Simulate 80 seconds (1,600 frames) of `Player.SimulateStep(0.05f)` while ship floor is disabled:
                // avatar must NOT fall 50 km below the ship!
                for (int step = 0; step < 1600; step++)
                    coord.Player.SimulateStep(0.05f);
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, bayDeckPos), 0.001f, "Suspended avatar must not fall while ship floor is inactive");
                Assert.AreEqual(0f, coord.Player.VerticalVelocity, 0.001f);

                // Even if transform were externally displaced while suspended, recovery must restore the saved deck pose and zero velocity:
                coord.Player.transform.position = bayDeckPos + new UnityEngine.Vector3(0f, -50000f, 0f);

                // 5. Click `RETURN TO BAY` button: must send `+tv/1 recover go` ONCE and NOT exit optimistically!
                UnityEngine.UI.Button returnToBayBtn = null;
                UnityEngine.UI.Button recoverRampBtn = null;
                var buttons = coord.Hud.gameObject.GetComponentsInChildren<UnityEngine.UI.Button>(true);
                for (int i = 0; i < buttons.Length; i++)
                {
                    if (buttons[i].gameObject.name == "Btn_RETURN TO BAY") returnToBayBtn = buttons[i];
                    if (buttons[i].gameObject.name == "Btn_RECOVER RAMP") recoverRampBtn = buttons[i];
                }
                Assert.IsNotNull(returnToBayBtn, "Expected RETURN TO BAY button on surface HUD");
                Assert.IsNotNull(recoverRampBtn, "Expected RECOVER RAMP button on surface HUD");

                dispatched.Clear();
                returnToBayBtn.onClick.Invoke();
                Assert.AreEqual(1, dispatched.Count, "RETURN TO BAY must send recovery once, not twice");
                Assert.AreEqual("+tv/1 recover go", dispatched[0]);

                // Verify command echo uses literal `>` in Unity uGUI Text, never HTML entity `&gt;`:
                coord.ExecutePlayerCommand("look");
                var commsField = typeof(TacSim.UI.TacSimHudController).GetField("_commsLogText", flags);
                var commsText = commsField?.GetValue(coord.Hud) as UnityEngine.UI.Text;
                Assert.IsNotNull(commsText);
                StringAssert.Contains("> look", commsText.text);
                StringAssert.DoesNotContain("&gt;", commsText.text);
                Assert.IsTrue(coord.SurfaceStage.IsExcursionActive, "RETURN TO BAY must wait for server confirmation instead of exiting optimistically");
                Assert.AreEqual(TacSimActiveStage.PlanetarySurface, coord.ActiveStage);

                // Intervening `site.map` while rover is still driving home keeps surface mode cleanly active
                onPanel.Invoke(coord, new object[] { "site.map", driveFrame });
                Assert.IsTrue(coord.SurfaceStage.IsExcursionActive);
                Assert.AreEqual(TacSimActiveStage.PlanetarySurface, coord.ActiveStage);

                // 6. Server emits post-recovery `site.map` (`you: null`, `rigs[0].cell: null`, no room change):
                // coordinator MUST exit surface mode, reconcile ActiveStage, restore walk camera, and restore avatar deck pose + zero velocity!
                var recoveredFrame = JsonNode.Parse(ReadRustFixtureJson("site_map_512_recovered.json"));
                onPanel.Invoke(coord, new object[] { "site.map", recoveredFrame });
                Assert.IsFalse(coord.SurfaceStage.RigOnGround, "Recovered rig must clear RigOnGround");
                Assert.IsFalse(coord.SurfaceStage.IsExcursionActive, "Recovered rig must exit surface excursion");
                Assert.AreEqual(TacSimActiveStage.ShipInteriorWalk, coord.ActiveStage, "Recovered rig must leave PlanetarySurface stage");
                Assert.AreEqual(TacSim.Character.CameraRigMode.ThirdPersonWalk, coord.CameraRig.Mode, "Recovered rig must restore walk camera");
                Assert.IsFalse(coord.Player.PhysicsSuspended);
                Assert.IsTrue(coord.Hud.IsWalkQuickActionsVisible, "Recovering rig restores QuickActionStrip on ShipInteriorWalk");
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, bayDeckPos), 0.001f, "Recovery must restore exact saved bay deck pose");
                Assert.Less(UnityEngine.Quaternion.Angle(coord.Player.transform.rotation, bayDeckRot), 0.1f);
                Assert.AreEqual(0f, coord.Player.VerticalVelocity, 0.001f);

                // 7. Re-launch, click `RECOVER RAMP`, and verify `tv.status` (`state: "bay"`) AND `site_map_refused.json` also reconcile cleanly!
                onPanel.Invoke(coord, new object[] { "site.map", driveFrame });
                Assert.IsTrue(coord.SurfaceStage.IsExcursionActive);
                Assert.AreEqual(TacSimActiveStage.PlanetarySurface, coord.ActiveStage);

                dispatched.Clear();
                recoverRampBtn.onClick.Invoke();
                Assert.AreEqual(1, dispatched.Count);
                Assert.AreEqual("+tv/1 recover go", dispatched[0]);

                var bayStatus = JsonNode.Parse(@"{""schema"":{""id"":""tv.status"",""shape"":""status"",""version"":1},""rig"":{""slot"":1,""state"":""bay""}}");
                onPanel.Invoke(coord, new object[] { "tv.status", bayStatus });
                Assert.IsFalse(coord.SurfaceStage.RigOnGround);
                Assert.IsFalse(coord.SurfaceStage.IsExcursionActive);
                Assert.AreEqual(TacSimActiveStage.ShipInteriorWalk, coord.ActiveStage);
                Assert.AreEqual(TacSim.Character.CameraRigMode.ThirdPersonWalk, coord.CameraRig.Mode);
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, bayDeckPos), 0.001f);

                onPanel.Invoke(coord, new object[] { "site.map", driveFrame });
                Assert.IsTrue(coord.SurfaceStage.IsExcursionActive);
                var refusedFrame = JsonNode.Parse(ReadRustFixtureJson("site_map_refused.json"));
                onPanel.Invoke(coord, new object[] { "site.map", refusedFrame });
                Assert.IsFalse(coord.SurfaceStage.RigOnGround);
                Assert.IsFalse(coord.SurfaceStage.IsExcursionActive);
                Assert.AreEqual(TacSimActiveStage.ShipInteriorWalk, coord.ActiveStage);
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
                if (!hadPreExistingEventSystem)
                {
                    var createdEs = UnityEngine.Object.FindAnyObjectByType<UnityEngine.EventSystems.EventSystem>();
                    if (createdEs != null)
                        UnityEngine.Object.DestroyImmediate(createdEs.gameObject);
                }
                if (rootGo != null)
                    UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void P0_PlaytestReviewFixes_TypedExits_StationGeometry_HudLayout_IdleClip_AndSyncExtractor()
        {
            bool hadPreExistingEventSystem = UnityEngine.Object.FindAnyObjectByType<UnityEngine.EventSystems.EventSystem>() != null;
            var rootGo = new UnityEngine.GameObject("TestPlaytestReviewRoot");
            UnityEngine.GameObject mockDoorFrame = null;
            TacSimAssetCatalog tempCatalog = null;
            string tempProjectDir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimPkgExtract_" + System.Guid.NewGuid().ToString("N"));

            try
            {
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                coord.Audio = rootGo.AddComponent<TacSimAudioSynthesizer>();

                var stationGo = new UnityEngine.GameObject("TestStationWorld");
                stationGo.transform.SetParent(rootGo.transform, false);
                coord.StationWorld = stationGo.AddComponent<StationWorldController>();

                var shipGo = new UnityEngine.GameObject("TestShipInterior");
                shipGo.transform.SetParent(rootGo.transform, false);
                coord.ShipInterior = shipGo.AddComponent<ShipInteriorController>();

                var spaceGo = new UnityEngine.GameObject("TestSpaceStage");
                spaceGo.transform.SetParent(rootGo.transform, false);
                coord.SpaceStage = spaceGo.AddComponent<SpaceFlightController>();

                var surfGo = new UnityEngine.GameObject("TestSurfaceStage");
                surfGo.transform.SetParent(rootGo.transform, false);
                coord.SurfaceStage = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();

                var arenaGo = new UnityEngine.GameObject("TestArenaStage");
                arenaGo.transform.SetParent(rootGo.transform, false);
                coord.ArenaStage = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();

                var camGo = new UnityEngine.GameObject("TestCameraRig", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                coord.CameraRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                coord.CameraRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();

                var playerGo = new UnityEngine.GameObject("TestPlayer", typeof(UnityEngine.CharacterController));
                playerGo.transform.SetParent(rootGo.transform, false);
                coord.Player = playerGo.AddComponent<TacSim.Character.ThirdPersonPlayerController>();
                coord.Player.VisualSpawner = playerGo.AddComponent<TacSim.Character.CharacterVisualSpawner>();

                var hudGo = new UnityEngine.GameObject("TestHud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();

                coord.EnsureSubsystems();

                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                var onSyncPos = typeof(TacSimGameCoordinator).GetMethod("Sync3DPositionWithServer", flags);
                var awaitingInitField = typeof(TacSimGameCoordinator).GetField("_awaitingInitialRoomSync", flags);
                var simConnField = typeof(TacSimGameCoordinator).GetField("_simulateConnectedForTests", flags);
                var nextMoveField = typeof(TacSimGameCoordinator).GetField("_nextMoveStepAllowedAt", flags);
                simConnField.SetValue(coord, true);

                var dispatched = new List<string>();
                coord.OnCommandDispatched += dispatched.Add;

                // 1A. Stationary typed exit on Station (`Port Watch Arrivals` -> `concourse` -> `Freeport Concourse`):
                // Standing still without walking in 3D MUST teleport the avatar into Freeport Concourse and dispatch ZERO reverse exits!
                awaitingInitField.SetValue(coord, true);
                var arrivalsView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#103"",
                    ""name"": ""Port Watch Arrivals"",
                    ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c03_arrivals"", ""surface"": ""station"" } }
                }"));
                onRoom.Invoke(coord, new object[] { arrivalsView });
                UnityEngine.Vector3 arrivalsSpawn = coord.StationWorld.GetRoomSpawnPosition("room.c03_arrivals");
                UnityEngine.Vector3 concourseSpawn = coord.StationWorld.GetRoomSpawnPosition("room.c04_concourse");
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, arrivalsSpawn), 0.05f);

                coord.ExecutePlayerCommand("concourse");
                dispatched.Clear();

                var concourseView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#104"",
                    ""name"": ""Freeport Concourse"",
                    ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c04_concourse"", ""surface"": ""station"" } }
                }"));
                onRoom.Invoke(coord, new object[] { concourseView });

                Assert.AreEqual("room.c04_concourse", coord.StationWorld.ActiveRoomEntryId, "Typed station exit must update active room to Freeport Concourse");
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, concourseSpawn), 0.05f, "Stationary typed station exit must teleport avatar to destination room");

                // Advance past the 220 ms post-RoomView move cooldown (`_nextMoveStepAllowedAt`) where the live playtest previously bounced at ~230 ms!
                nextMoveField.SetValue(coord, -1f);
                for (int frame = 0; frame < 10; frame++)
                    onSyncPos.Invoke(coord, null);
                Assert.AreEqual(0, dispatched.Count, "Stationary typed station exit must never dispatch a reverse exit bounce ('w')");

                // 1B. Stationary typed exit aboard Ship Interior (`Passage` -> `bridge` -> `Bridge`):
                // Standing still in `corridor` and typing `bridge` MUST teleport the avatar to `bridge` and dispatch ZERO reverse exits!
                var corridorView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#801"",
                    ""name"": ""Salt Moth - Passage"",
                    ""ambient"": { ""tacsim"": { ""mode"": ""interior"", ""unit"": ""dray"", ""room"": ""corridor"", ""hull_name"": ""Salt Moth"", ""class"": ""Dray"" } }
                }"));
                onRoom.Invoke(coord, new object[] { corridorView });
                Assert.AreEqual("corridor", coord.ShipInterior.ActiveCompartmentId);

                coord.ExecutePlayerCommand("bridge");
                dispatched.Clear();

                var bridgeView = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#802"",
                    ""name"": ""Salt Moth - Bridge"",
                    ""ambient"": { ""tacsim"": { ""mode"": ""interior"", ""unit"": ""dray"", ""room"": ""bridge"", ""hull_name"": ""Salt Moth"", ""class"": ""Dray"" } }
                }"));
                onRoom.Invoke(coord, new object[] { bridgeView });

                UnityEngine.Vector3 bridgeSpawn = coord.ShipInterior.GetCompartmentSpawnWorldPos("bridge");
                Assert.AreEqual("bridge", coord.ShipInterior.ActiveCompartmentId);
                Assert.Less(UnityEngine.Vector3.Distance(coord.Player.transform.position, bridgeSpawn), 0.05f, "Stationary typed ship exit must teleport avatar to bridge");

                nextMoveField.SetValue(coord, -1f);
                for (int frame = 0; frame < 10; frame++)
                    onSyncPos.Invoke(coord, null);
                Assert.AreEqual(0, dispatched.Count, "Stationary typed ship exit must never dispatch a reverse compartment command");

                // 2. Station World status boards (`BoardText` unstretched), grounded plaques + 180-deg facing labels, and suppressed cyan doorposts:
                float elev0 = coord.StationWorld.Pack.GetFloorElevationY(0);
                float elev1 = coord.StationWorld.Pack.GetFloorElevationY(1);
                float elev2 = coord.StationWorld.Pack.GetFloorElevationY(2);
                var allTransforms = coord.StationWorld.GetComponentsInChildren<UnityEngine.Transform>(true);
                int boardCount = 0;
                int plaqueCount = 0;
                int diffuserCount = 0;
                int ceilingCount = 0;
                var plaquesByRoom = new Dictionary<UnityEngine.Transform, List<UnityEngine.Transform>>();

                for (int i = 0; i < allTransforms.Length; i++)
                {
                    var tr = allTransforms[i];
                    if (tr.name == "BoardText")
                    {
                        boardCount++;
                        Assert.AreEqual(UnityEngine.Vector3.one, tr.parent.localScale, "BoardText parent StatusBoardRoot must stay at uniform (1,1,1) scale");
                        Assert.AreEqual(UnityEngine.Vector3.one, tr.localScale, "BoardText must not be stretched across doorways");
                    }
                    else if (tr.name == "CeilingLightDiffuser")
                    {
                        diffuserCount++;
                    }
                    else if (tr.name == "RoofCeiling" || tr.name.EndsWith("_CeilingSoffit", System.StringComparison.Ordinal))
                    {
                        ceilingCount++;
                    }
                    else if (tr.name.StartsWith("Plaque_", System.StringComparison.Ordinal))
                    {
                        plaqueCount++;
                        float distFromDeck = UnityEngine.Mathf.Min(
                            UnityEngine.Mathf.Abs(tr.position.y - elev0),
                            UnityEngine.Mathf.Min(
                                UnityEngine.Mathf.Abs(tr.position.y - elev1),
                                UnityEngine.Mathf.Abs(tr.position.y - elev2)));
                        Assert.Less(distFromDeck, 0.02f, $"Plaque '{tr.name}' must sit flush on deck elevation rather than floating in midair");

                        var labelBillboard = tr.Find("PlaqueLabelBillboard");
                        Assert.IsNotNull(labelBillboard, $"Plaque '{tr.name}' must have PlaqueLabelBillboard");
                        Assert.IsNotNull(labelBillboard.GetComponent<WorldBillboard>(), "Plaque label must carry WorldBillboard so text never renders mirrored");
                        Assert.Less(UnityEngine.Quaternion.Angle(labelBillboard.localRotation, UnityEngine.Quaternion.Euler(0f, 180f, 0f)), 0.5f);

                        if (tr.parent != null)
                        {
                            if (!plaquesByRoom.TryGetValue(tr.parent, out var list))
                            {
                                list = new List<UnityEngine.Transform>();
                                plaquesByRoom[tr.parent] = list;
                            }
                            list.Add(tr);
                        }
                    }
                }
                Assert.Greater(boardCount, 0, "Expected Ceres Freeport diegetic status boards");
                Assert.Greater(plaqueCount, 0, "Expected Ceres Freeport detail plaques ([CASES], [ROLLER DOOR], etc.)");
                Assert.Greater(diffuserCount, 0, "Expected recessed ceiling luminaire fixtures");
                Assert.Greater(ceilingCount, 0, "Expected matte coffered ceiling soffits/roofs");

                // Verify plaques within the same room (e.g. [LAY BOARD] & [LEDGER SCREEN] in Registry) never overlap!
                foreach (var kv in plaquesByRoom)
                {
                    var roomPlaques = kv.Value;
                    for (int a = 0; a < roomPlaques.Count; a++)
                    {
                        for (int b = a + 1; b < roomPlaques.Count; b++)
                        {
                            float sep = UnityEngine.Vector3.Distance(roomPlaques[a].position, roomPlaques[b].position);
                            Assert.GreaterOrEqual(sep, 2.5f, $"Plaques '{roomPlaques[a].name}' and '{roomPlaques[b].name}' in '{kv.Key.name}' must not overlap");
                        }
                    }
                }

                // Verify stairs NEVER spawn cyan DoorPost_A / DoorPost_B even in fallback mode (`Catalog == null`)!
                var portals = coord.StationWorld.GetComponentsInChildren<ExitPortalTrigger>(true);
                int stairPortals = 0;
                for (int p = 0; p < portals.Length; p++)
                {
                    if (UnityEngine.Mathf.Abs(portals[p].axisAToB.y) > 0.2f)
                    {
                        stairPortals++;
                        Assert.IsNull(portals[p].transform.Find("DoorPost_A"), "Inter-floor stair portals must never spawn cyan DoorPost_A bars");
                        Assert.IsNull(portals[p].transform.Find("DoorPost_B"), "Inter-floor stair portals must never spawn cyan DoorPost_B bars");
                    }
                }
                Assert.Greater(stairPortals, 0, "Expected inter-floor stair portals in Ceres Freeport");

                // Verify WorldBillboard clamps speech bubbles away from walls!
                var testActorGo = new UnityEngine.GameObject("WallCornerActor");
                testActorGo.transform.SetParent(rootGo.transform, false);
                testActorGo.transform.position = new UnityEngine.Vector3(9.8f, 0f, -9.8f);
                var testBbGo = new UnityEngine.GameObject("Billboard");
                testBbGo.transform.SetParent(testActorGo.transform, false);
                var testBb = testBbGo.AddComponent<WorldBillboard>();
                testBb.SetRoomSafeBounds(-10f, 10f, -10f, 10f, wallMarginM: 1.5f);
                var bbLateUpdate = typeof(WorldBillboard).GetMethod("LateUpdate", flags);
                Assert.IsNotNull(bbLateUpdate);
                bbLateUpdate.Invoke(testBb, null);
                Assert.LessOrEqual(testBbGo.transform.position.x, 8.501f, "Speech billboard must clamp inside east wall margin");
                Assert.GreaterOrEqual(testBbGo.transform.position.z, -8.501f, "Speech billboard must clamp inside south wall margin");

                // Rebuild a secondary StationWorld with commercial door prefabs assigned -> zero fallback cyan DoorPost_A / DoorPost_B!
                mockDoorFrame = UnityEngine.GameObject.CreatePrimitive(UnityEngine.PrimitiveType.Cube);
                mockDoorFrame.name = "MockCommercialDoorArch";
                tempCatalog = UnityEngine.ScriptableObject.CreateInstance<TacSimAssetCatalog>();
                tempCatalog.stationDoorFrame = mockDoorFrame;
                tempCatalog.stationLockedDoor = mockDoorFrame;

                var commercialStationGo = new UnityEngine.GameObject("TestCommercialStation");
                commercialStationGo.transform.SetParent(rootGo.transform, false);
                var commercialStation = commercialStationGo.AddComponent<StationWorldController>();
                commercialStation.BuildStation(tempCatalog, "neighborhood.ceres_freeport");

                var commTransforms = commercialStation.GetComponentsInChildren<UnityEngine.Transform>(true);
                bool foundViewingHandrail = false;
                bool foundViewingStanchion = false;
                bool foundDockingBoom = false;
                for (int i = 0; i < commTransforms.Length; i++)
                {
                    string tn = commTransforms[i].name;
                    Assert.AreNotEqual("DoorPost_A", tn, "Fallback cyan DoorPost_A must be suppressed when commercial door prefabs exist");
                    Assert.AreNotEqual("DoorPost_B", tn, "Fallback cyan DoorPost_B must be suppressed when commercial door prefabs exist");
                    Assert.AreNotEqual("ViewingRail", tn, "Floating neon-yellow ViewingRail bar must be replaced by stanchion handrail");
                    Assert.IsFalse(tn.StartsWith("UmbilicalArm_", System.StringComparison.Ordinal), "Orange UmbilicalArm blocks must be replaced by sleek DockingBoom");
                    Assert.IsFalse(tn.StartsWith("UnlitWorkingBerth_", System.StringComparison.Ordinal), "Floating grey UnlitWorkingBerth slabs must be replaced");
                    if (tn == "ViewingHandrail") foundViewingHandrail = true;
                    if (tn == "ViewingStanchion_0") foundViewingStanchion = true;
                    if (tn == "DockingBoom_0") foundDockingBoom = true;
                }
                Assert.IsTrue(foundViewingHandrail, "Expected brushed-steel ViewingHandrail in Berth Gallery");
                Assert.IsTrue(foundViewingStanchion, "Expected grounded ViewingStanchion posts in Berth Gallery");
                Assert.IsTrue(foundDockingBoom, "Expected sleek DockingBoom on exterior cradles");

                // 3. HUD narrow-aspect (`1100 x 756`) non-overlap check: CommsDrawer right edge < QuickActionStrip / Helm / Rover left edge
                UnityEngine.RectTransform commsRt = null;
                UnityEngine.RectTransform quickStripRt = null;
                UnityEngine.RectTransform roverBarRt = null;
                var hudRects = coord.Hud.GetComponentsInChildren<UnityEngine.RectTransform>(true);
                for (int i = 0; i < hudRects.Length; i++)
                {
                    if (hudRects[i].name == "CommsDrawer") commsRt = hudRects[i];
                    else if (hudRects[i].name == "QuickActionStrip") quickStripRt = hudRects[i];
                    else if (hudRects[i].name == "SurfaceRoverActions") roverBarRt = hudRects[i];
                }
                Assert.IsNotNull(commsRt);
                Assert.IsNotNull(quickStripRt);
                Assert.IsNotNull(roverBarRt);
                Assert.AreEqual(0f, quickStripRt.anchorMin.x, 0.001f);
                Assert.AreEqual(1f, quickStripRt.anchorMax.x, 0.001f);
                Assert.Less(commsRt.offsetMax.x, quickStripRt.offsetMin.x, "CommsDrawer must never overlap QuickActionStrip on narrow viewports");
                Assert.Less(commsRt.offsetMax.x, roverBarRt.offsetMin.x, "CommsDrawer must never overlap SurfaceRoverActions on narrow viewports");

                // 4. Checked-in `TacSim_Idle.anim` contains explicit upright leg/hip Humanoid muscles (`Lower Leg Stretch = 0.96`)
                string idleAnimYaml = ReadUnityAssetText("Generated/TacSim_Idle.anim");
                StringAssert.Contains("attribute: RootT.y", idleAnimYaml);
                StringAssert.Contains("attribute: Left Lower Leg Stretch", idleAnimYaml);
                StringAssert.Contains("attribute: Right Lower Leg Stretch", idleAnimYaml);
                StringAssert.Contains("value: 0.96", idleAnimYaml);
                StringAssert.Contains("attribute: Left Upper Leg Front-Back", idleAnimYaml);
                StringAssert.Contains("value: 0.5", idleAnimYaml);

                // 5. Synchronous `.unitypackage` extractor (`<guid>/asset` preceding `<guid>/pathname` + zip-slip guard)
                byte[] expectedPrefabBytes = System.Text.Encoding.UTF8.GetBytes("%YAML 1.1\nPrefab: Trooper\n");
                byte[] expectedMetaBytes = System.Text.Encoding.UTF8.GetBytes("fileFormatVersion: 2\nguid: 0123456789abcdef0123456789abcdef\n");
                using (var pkgStream = BuildSyntheticUnityPackage(
                    ("0123456789abcdef0123456789abcdef/asset", expectedPrefabBytes),
                    ("0123456789abcdef0123456789abcdef/asset.meta", expectedMetaBytes),
                    ("0123456789abcdef0123456789abcdef/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/IdaFaber/Prefabs/Girl/SK_F_TROOPER.prefab\n00")),
                    ("ffffffffffffffffffffffffffffffff/asset", System.Text.Encoding.UTF8.GetBytes("malicious")),
                    ("ffffffffffffffffffffffffffffffff/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/../../outside_escape.txt\n"))))
                {
                    int count = UnityPackageArchiveExtractor.ExtractPackageStream(pkgStream, tempProjectDir);
                    Assert.AreEqual(2, count, "Valid Assets/ payload + .meta sidecar written synchronously; traversal entry rejected");
                }

                string extractedAsset = System.IO.Path.Combine(tempProjectDir, "Assets", "IdaFaber", "Prefabs", "Girl", "SK_F_TROOPER.prefab");
                string extractedMeta = extractedAsset + ".meta";
                Assert.IsTrue(System.IO.File.Exists(extractedAsset), "Extracted asset must exist synchronously before returning");
                Assert.IsTrue(System.IO.File.Exists(extractedMeta), "Extracted .meta must exist synchronously before returning");
                CollectionAssert.AreEqual(expectedPrefabBytes, System.IO.File.ReadAllBytes(extractedAsset));
                CollectionAssert.AreEqual(expectedMetaBytes, System.IO.File.ReadAllBytes(extractedMeta));
                Assert.IsFalse(System.IO.File.Exists(System.IO.Path.GetFullPath(System.IO.Path.Combine(tempProjectDir, "..", "outside_escape.txt"))));
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
                if (!hadPreExistingEventSystem)
                {
                    var createdEs = UnityEngine.Object.FindAnyObjectByType<UnityEngine.EventSystems.EventSystem>();
                    if (createdEs != null)
                        UnityEngine.Object.DestroyImmediate(createdEs.gameObject);
                }
                if (mockDoorFrame != null)
                    UnityEngine.Object.DestroyImmediate(mockDoorFrame);
                if (tempCatalog != null)
                    UnityEngine.Object.DestroyImmediate(tempCatalog);
                if (rootGo != null)
                    UnityEngine.Object.DestroyImmediate(rootGo);
                if (System.IO.Directory.Exists(tempProjectDir))
                    System.IO.Directory.Delete(tempProjectDir, recursive: true);
            }
        }

        [Test]
        public void P0_Track3_PackSanitizer_CliUrl_ShipInteriorCrewAndGauges_AndPersistentActors()
        {
            bool hadPreExistingEventSystem = UnityEngine.Object.FindAnyObjectByType<UnityEngine.EventSystems.EventSystem>() != null;
            var rootGo = new UnityEngine.GameObject("TestTrack3Root");
            UnityEngine.GameObject mockFenrisShell = null;
            TacSimAssetCatalog tempCatalog = null;

            try
            {
                // 1. Standalone Fallback Pack Sanitizer (`TacSimContentLoader.SanitizePackJsonForClient`)
                string rawPackJson = @"{
                    ""manifest"": { ""id"": ""neighborhood.test_port"", ""version"": 1 },
                    ""brief"": ""GM-only design brief"",
                    ""notes"": ""Private author notes"",
                    ""secrets"": [""master_key""],
                    ""entries"": {
                        ""room.public_hall"": {
                            ""kind"": ""room"",
                            ""def"": {
                                ""name"": ""Public Hall"",
                                ""access"": ""public"",
                                ""desc"": ""Warm deck lighting and open stalls."",
                                ""details"": { ""stalls"": ""Fresh coffee."" },
                                ""secrets"": [""trapdoor""]
                            }
                        },
                        ""room.vault"": {
                            ""kind"": ""room"",
                            ""def"": {
                                ""name"": ""Council Vault"",
                                ""access"": ""restricted"",
                                ""desc"": ""Classified bullion stacks."",
                                ""details"": { ""keypad"": ""Code 9941"" }
                            }
                        },
                        ""exit.main_door"": {
                            ""kind"": ""exit"",
                            ""def"": { ""from"": ""room.public_hall"", ""to"": ""room.vault"", ""portal"": { ""visibility"": ""public"" } }
                        },
                        ""exit.smuggler_vent"": {
                            ""kind"": ""exit"",
                            ""def"": { ""from"": ""room.public_hall"", ""to"": ""room.vault"", ""portal"": { ""visibility"": ""hidden"" } }
                        },
                        ""npc.ilse"": {
                            ""kind"": ""npc"",
                            ""def"": {
                                ""name"": ""Ilse Oduya-Brandt"",
                                ""room"": ""room.public_hall"",
                                ""short_desc"": ""Co-op registrar"",
                                ""prompt"": ""Confidential LLM persona prompt"",
                                ""agenda"": ""Private faction goal""
                            }
                        },
                        ""secret.plot"": { ""kind"": ""secret"", ""def"": { ""text"": ""Hidden plot"" } },
                        ""lore.old"": { ""kind"": ""lore"", ""def"": { ""text"": ""Hidden lore"" } },
                        ""quest.arc"": { ""kind"": ""quest"", ""def"": { ""text"": ""Hidden quest"" } },
                        ""lane.vesta"": { ""kind"": ""lane"", ""def"": { ""text"": ""Vendor schedule"" } }
                    }
                }";

                string sanitizedText = TacSimContentLoader.SanitizePackJsonForClient(rawPackJson);
                var sanitizedRoot = JsonNode.Parse(sanitizedText);
                Assert.IsFalse(sanitizedRoot.ContainsKey("brief"), "Top-level brief must be stripped");
                Assert.IsFalse(sanitizedRoot.ContainsKey("notes"), "Top-level notes must be stripped");
                Assert.IsFalse(sanitizedRoot.ContainsKey("secrets"), "Top-level secrets must be stripped");

                var entries = sanitizedRoot["entries"].AsObject();
                Assert.IsNotNull(entries);
                Assert.IsTrue(entries.ContainsKey("room.public_hall"));
                Assert.IsTrue(entries.ContainsKey("room.vault"));
                Assert.IsTrue(entries.ContainsKey("exit.main_door"));
                Assert.IsTrue(entries.ContainsKey("npc.ilse"));
                Assert.IsFalse(entries.ContainsKey("exit.smuggler_vent"), "Hidden exits must be stripped from client fallback packs");
                Assert.IsFalse(entries.ContainsKey("secret.plot"), "Secret entries must be stripped");
                Assert.IsFalse(entries.ContainsKey("lore.old"), "Lore entries must be stripped");
                Assert.IsFalse(entries.ContainsKey("quest.arc"), "Quest entries must be stripped");
                Assert.IsFalse(entries.ContainsKey("lane.vesta"), "Vendor lane schedules must be stripped");

                Assert.AreEqual("Warm deck lighting and open stalls.", entries["room.public_hall"]["def"].Str("desc"));
                Assert.IsFalse(entries["room.public_hall"]["def"].ContainsKey("secrets"));
                Assert.IsFalse(entries["room.vault"]["def"].ContainsKey("desc"), "Restricted room desc must be stripped");
                Assert.IsFalse(entries["room.vault"]["def"].ContainsKey("details"), "Restricted room details must be stripped");
                Assert.AreEqual("Ilse Oduya-Brandt", entries["npc.ilse"]["def"].Str("name"));
                Assert.IsFalse(entries["npc.ilse"]["def"].ContainsKey("prompt"), "NPC LLM prompt must be stripped");
                Assert.IsFalse(entries["npc.ilse"]["def"].ContainsKey("agenda"), "NPC private agenda must be stripped");

                // 2. Standalone CLI `--server` & `TACSIM_SERVER_URL` Resolution Priority
                Assert.AreEqual(
                    "https://play.conclavio.dev:8443",
                    TacSim.UI.TacSimHudController.ResolveInitialServerUrl(
                        new[] { "TacSim.exe", "--server", "https://play.conclavio.dev:8443/" },
                        "http://env.example:4201",
                        "http://saved.example:4201"));
                Assert.AreEqual(
                    "http://10.0.0.12:4201",
                    TacSim.UI.TacSimHudController.ResolveInitialServerUrl(
                        new[] { "TacSim.exe", "--server=http://10.0.0.12:4201/" },
                        null,
                        "http://saved.example:4201"));
                Assert.AreEqual(
                    "http://env.example:4201",
                    TacSim.UI.TacSimHudController.ResolveInitialServerUrl(
                        new[] { "TacSim.exe" },
                        "http://env.example:4201/",
                        "http://saved.example:4201"));
                Assert.AreEqual(
                    "http://saved.example:4201",
                    TacSim.UI.TacSimHudController.ResolveInitialServerUrl(
                        new[] { "TacSim.exe" },
                        string.Empty,
                        "http://saved.example:4201/"));

                // 3. ShipInterior Overhaul: No colliding FenrisVisualHullShell, unstretched GaugeRoot, stairwell cutouts, Bridge canopy & co-op crewmates
                mockFenrisShell = UnityEngine.GameObject.CreatePrimitive(UnityEngine.PrimitiveType.Cube);
                mockFenrisShell.name = "MockFenrisShell";
                tempCatalog = UnityEngine.ScriptableObject.CreateInstance<TacSimAssetCatalog>();
                tempCatalog.fenrisExteriorInteriorPrefab = mockFenrisShell;

                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                coord.AssetCatalog = tempCatalog;
                coord.Audio = rootGo.AddComponent<TacSimAudioSynthesizer>();

                var stationGo = new UnityEngine.GameObject("Track3Station");
                stationGo.transform.SetParent(rootGo.transform, false);
                coord.StationWorld = stationGo.AddComponent<StationWorldController>();

                var shipGo = new UnityEngine.GameObject("Track3Ship");
                shipGo.transform.SetParent(rootGo.transform, false);
                coord.ShipInterior = shipGo.AddComponent<ShipInteriorController>();

                var spaceGo = new UnityEngine.GameObject("Track3Space");
                spaceGo.transform.SetParent(rootGo.transform, false);
                coord.SpaceStage = spaceGo.AddComponent<SpaceFlightController>();

                var surfGo = new UnityEngine.GameObject("Track3Surface");
                surfGo.transform.SetParent(rootGo.transform, false);
                coord.SurfaceStage = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();

                var arenaGo = new UnityEngine.GameObject("Track3Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                coord.ArenaStage = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();

                var camGo = new UnityEngine.GameObject("Track3Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                coord.CameraRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                coord.CameraRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();

                var playerGo = new UnityEngine.GameObject("Track3Player", typeof(UnityEngine.CharacterController));
                playerGo.transform.SetParent(rootGo.transform, false);
                coord.Player = playerGo.AddComponent<TacSim.Character.ThirdPersonPlayerController>();
                coord.Player.VisualSpawner = playerGo.AddComponent<TacSim.Character.CharacterVisualSpawner>();

                var hudGo = new UnityEngine.GameObject("Track3Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();

                coord.EnsureSubsystems();

                var shipTransforms = coord.ShipInterior.GetComponentsInChildren<UnityEngine.Transform>(true);
                int gaugeCount = 0;
                int shipDiffuserCount = 0;
                bool foundBridgeCanopy = false;
                bool foundRampHandrail = false;
                bool foundPassageStairCutout = false;
                bool foundBridgeStairCutout = false;

                for (int i = 0; i < shipTransforms.Length; i++)
                {
                    var tr = shipTransforms[i];
                    Assert.AreNotEqual("FenrisVisualHullShell", tr.name, "Monolithic 1-deck Fenris shell must not be spawned inside multi-deck Dray compartments");
                    if (tr.name == "GaugeText")
                    {
                        gaugeCount++;
                        Assert.AreEqual("GaugeRoot", tr.parent.name, "GaugeText must sit under uniform-scale GaugeRoot");
                        Assert.AreEqual(UnityEngine.Vector3.one, tr.parent.localScale, "GaugeRoot must maintain uniform (1,1,1) scale");
                        Assert.AreEqual(UnityEngine.Vector3.one, tr.localScale, "GaugeText must never be stretched 2.4x horizontally");
                    }
                    else if (tr.name == "CeilingLightDiffuser")
                    {
                        shipDiffuserCount++;
                    }
                    else if (tr.name == "BridgeCanopyGlass")
                    {
                        foundBridgeCanopy = true;
                    }
                    else if (tr.name == "RampRail_L")
                    {
                        foundRampHandrail = true;
                    }
                    else if (tr.name == "DeckPlate_Port" && tr.parent != null && tr.parent.name == "Compartment_passage")
                    {
                        foundPassageStairCutout = true;
                    }
                    else if (tr.name == "DeckPlate_Fwd" && tr.parent != null && tr.parent.name == "Compartment_bridge")
                    {
                        foundBridgeStairCutout = true;
                    }
                }
                Assert.Greater(gaugeCount, 0, "Expected compartment GaugeText displays aboard Dray");
                Assert.Greater(shipDiffuserCount, 0, "Expected recessed CeilingLightDiffuser luminaires aboard Dray");
                Assert.IsTrue(foundBridgeCanopy, "Expected framed BridgeCanopyGlass forward viewport on Bridge");
                Assert.IsTrue(foundRampHandrail, "Expected sloped companionway safety handrails");
                Assert.IsTrue(foundPassageStairCutout, "Expected stairwell cutout in Central Passage deck above Hold companionway");
                Assert.IsTrue(foundBridgeStairCutout, "Expected stairwell cutout in Bridge deck above Bridge companionway");

                // Co-op crewmate spawning + persistent pooling + speech bubbles aboard ShipInterior
                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                var onNarrative = typeof(TacSimGameCoordinator).GetMethod("HandleServerNarrative", flags);
                var shipCrewField = typeof(ShipInteriorController).GetField("_crewActors", flags);
                var shipCrewDict = shipCrewField.GetValue(coord.ShipInterior) as System.Collections.IDictionary;
                Assert.IsNotNull(shipCrewDict);

                var bridgeCrewRv1 = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#802"",
                    ""name"": ""Salt Moth - Bridge"",
                    ""contents"": [
                        { ""id"": ""#10"", ""name"": ""Self Captain"" },
                        { ""id"": ""#22"", ""name"": ""Maya Vance"" },
                        { ""id"": ""#88"", ""name"": ""Drive ROM Slate"" }
                    ],
                    ""ambient"": {
                        ""players"": [""#10"", ""#22""],
                        ""things"": [""#88""],
                        ""tacsim"": { ""mode"": ""interior"", ""unit"": ""dray"", ""room"": ""bridge"", ""hull_name"": ""Salt Moth"", ""class"": ""Dray"" }
                    }
                }"));
                coord.ShipInterior.ApplyRoomView(bridgeCrewRv1, selfActorId: "#10");
                Assert.AreEqual(1, shipCrewDict.Count, "Only co-op crewmate #22 (excluding self #10 and Thing #88) must spawn");
                var mayaGo1 = shipCrewDict["#22"] as UnityEngine.GameObject;
                Assert.IsNotNull(mayaGo1);

                // Enter ship via coordinator so `_isAboardShip` is true, then verify repeat RoomView keeps same GameObject AND narrative lights up SpeechBubble!
                onRoom.Invoke(coord, new object[] { bridgeCrewRv1 });
                coord.ShipInterior.ApplyRoomView(bridgeCrewRv1, selfActorId: "#10");
                var mayaGo2 = shipCrewDict["#22"] as UnityEngine.GameObject;
                Assert.AreSame(mayaGo1, mayaGo2, "Repeat ShipInterior RoomView must reuse existing crewmate GameObject without T-pose flicker");

                var crewSpeech = new MushNarrativeEvent
                {
                    Kind = "say",
                    Actor = new MushObjRef { Id = "#22", Name = "Maya Vance" },
                    Text = "Tankage green, ready to undock."
                };
                onNarrative.Invoke(coord, new object[] { crewSpeech });
                var mayaBb = mayaGo2.GetComponentInChildren<WorldBillboard>(true);
                Assert.IsNotNull(mayaBb);
                Assert.IsTrue(mayaBb.IsSpeechActive, "Shipmate narrative must activate SpeechBubble aboard ship");
                StringAssert.Contains("Tankage green", mayaBb.CurrentSpeechText);

                // Departed crewmate is removed cleanly
                var bridgeEmptyRv = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#802"",
                    ""name"": ""Salt Moth - Bridge"",
                    ""contents"": [ { ""id"": ""#10"", ""name"": ""Self Captain"" } ],
                    ""ambient"": {
                        ""players"": [""#10""],
                        ""tacsim"": { ""mode"": ""interior"", ""unit"": ""dray"", ""room"": ""bridge"", ""hull_name"": ""Salt Moth"", ""class"": ""Dray"" }
                    }
                }"));
                coord.ShipInterior.ApplyRoomView(bridgeEmptyRv, selfActorId: "#10");
                Assert.AreEqual(0, shipCrewDict.Count, "Departed shipmate must be removed cleanly");

                // 4. StationWorld Persistent Non-Destroying Actor Pool across `RoomView` updates
                var stationActorsField = typeof(StationWorldController).GetField("_spawnedActors", flags);
                var stationActorsDict = stationActorsField.GetValue(coord.StationWorld) as System.Collections.IDictionary;
                Assert.IsNotNull(stationActorsDict);

                var registryRv1 = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#105"",
                    ""name"": ""Registry & Co-op Desk"",
                    ""contents"": [
                        { ""id"": ""#501"", ""name"": ""Ilse Oduya-Brandt"" },
                        { ""id"": ""#602"", ""name"": ""Pilot Kade"" }
                    ],
                    ""ambient"": {
                        ""npcs"": [""#501""],
                        ""players"": [""#602""],
                        ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c05_registry"", ""surface"": ""station"" }
                    }
                }"));
                coord.StationWorld.ApplyRoomView(registryRv1, selfActorId: "#10");
                Assert.AreEqual(2, stationActorsDict.Count);
                var ilseGo1 = stationActorsDict["#501"] as UnityEngine.GameObject;
                Assert.IsNotNull(ilseGo1);

                coord.StationWorld.ShowActorSpeech("#501", "Ilse Oduya-Brandt", "Sign on the line, captain.");
                var ilseBb = ilseGo1.GetComponentInChildren<WorldBillboard>(true);
                Assert.IsTrue(ilseBb.IsSpeechActive);

                // Second RoomView (e.g. player types `look` or #602 leaves): Ilse's GameObject AND active speech bubble stay intact!
                var registryRv2 = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#105"",
                    ""name"": ""Registry & Co-op Desk"",
                    ""contents"": [
                        { ""id"": ""#501"", ""name"": ""Ilse Oduya-Brandt"" }
                    ],
                    ""ambient"": {
                        ""npcs"": [""#501""],
                        ""players"": [],
                        ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c05_registry"", ""surface"": ""station"" }
                    }
                }"));
                coord.StationWorld.ApplyRoomView(registryRv2, selfActorId: "#10");
                Assert.AreEqual(1, stationActorsDict.Count, "Departed remote pilot #602 must be removed while keeping #501");
                Assert.AreSame(ilseGo1, stationActorsDict["#501"], "StationWorld must preserve existing NPC GameObject across RoomView refreshes");
                Assert.IsTrue(ilseBb.IsSpeechActive, "Existing actor SpeechBubble must survive RoomView refresh");
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
                if (!hadPreExistingEventSystem)
                {
                    var createdEs = UnityEngine.Object.FindAnyObjectByType<UnityEngine.EventSystems.EventSystem>();
                    if (createdEs != null)
                        UnityEngine.Object.DestroyImmediate(createdEs.gameObject);
                }
                if (mockFenrisShell != null)
                    UnityEngine.Object.DestroyImmediate(mockFenrisShell);
                if (tempCatalog != null)
                    UnityEngine.Object.DestroyImmediate(tempCatalog);
                if (rootGo != null)
                    UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        private static System.IO.MemoryStream BuildSyntheticUnityPackage(params (string entryName, byte[] payload)[] entries)
        {
            var gzOut = new System.IO.MemoryStream();
            using (var gz = new System.IO.Compression.GZipStream(gzOut, System.IO.Compression.CompressionLevel.Fastest, leaveOpen: true))
            {
                for (int i = 0; i < entries.Length; i++)
                {
                    byte[] header = new byte[512];
                    byte[] nameBytes = System.Text.Encoding.ASCII.GetBytes(entries[i].entryName);
                    System.Array.Copy(nameBytes, 0, header, 0, System.Math.Min(nameBytes.Length, 100));

                    byte[] modeBytes = System.Text.Encoding.ASCII.GetBytes("0000644\0");
                    System.Array.Copy(modeBytes, 0, header, 100, modeBytes.Length);

                    string octalSize = System.Convert.ToString(entries[i].payload.Length, 8).PadLeft(11, '0') + "\0";
                    byte[] sizeBytes = System.Text.Encoding.ASCII.GetBytes(octalSize);
                    System.Array.Copy(sizeBytes, 0, header, 124, 12);

                    header[156] = (byte)'0';
                    for (int c = 148; c < 156; c++) header[c] = (byte)' ';
                    int checksum = 0;
                    for (int c = 0; c < 512; c++) checksum += header[c];
                    string octalSum = System.Convert.ToString(checksum, 8).PadLeft(6, '0') + "\0 ";
                    System.Array.Copy(System.Text.Encoding.ASCII.GetBytes(octalSum), 0, header, 148, 8);

                    gz.Write(header, 0, 512);
                    if (entries[i].payload.Length > 0)
                    {
                        gz.Write(entries[i].payload, 0, entries[i].payload.Length);
                        int rem = entries[i].payload.Length % 512;
                        if (rem != 0)
                            gz.Write(new byte[512 - rem], 0, 512 - rem);
                    }
                }
                gz.Write(new byte[1024], 0, 1024);
            }
            gzOut.Position = 0;
            return gzOut;
        }

        private static string ReadUnityAssetText(string relativeUnderTacSim)
        {
            string normalized = relativeUnderTacSim.Replace('/', System.IO.Path.DirectorySeparatorChar);
            string dir = System.IO.Directory.GetCurrentDirectory();
            for (int depth = 0; depth < 6 && !string.IsNullOrEmpty(dir); depth++)
            {
                string candidateA = System.IO.Path.Combine(dir, "unity", "Assets", "_TacSim", normalized);
                if (System.IO.File.Exists(candidateA))
                    return System.IO.File.ReadAllText(candidateA);
                string candidateB = System.IO.Path.Combine(dir, "Assets", "_TacSim", normalized);
                if (System.IO.File.Exists(candidateB))
                    return System.IO.File.ReadAllText(candidateB);
                dir = System.IO.Directory.GetParent(dir)?.FullName;
            }
            throw new System.IO.FileNotFoundException($"Missing Unity asset '{relativeUnderTacSim}'.");
        }

        private static string ReadRustFixtureJson(string fileName)
        {
            string dir = System.IO.Directory.GetCurrentDirectory();
            for (int depth = 0; depth < 6 && !string.IsNullOrEmpty(dir); depth++)
            {
                string candidateA = System.IO.Path.Combine(dir, "unity", "Assets", "_TacSim", "Scripts", "Tests", "Fixtures~", fileName);
                if (System.IO.File.Exists(candidateA))
                    return System.IO.File.ReadAllText(candidateA);
                string candidateB = System.IO.Path.Combine(dir, "Assets", "_TacSim", "Scripts", "Tests", "Fixtures~", fileName);
                if (System.IO.File.Exists(candidateB))
                    return System.IO.File.ReadAllText(candidateB);
                dir = System.IO.Directory.GetParent(dir)?.FullName;
            }
            throw new System.IO.FileNotFoundException($"Missing Rust-pinned cross-language fixture '{fileName}'.");
        }

        [Test]
        public void Track5_Doc49_SpaceFireMounts_5BandAspect_AndGroundMultiMountTelemetry()
        {
            var rootGo = new UnityEngine.GameObject("TestDoc49Root");
            try
            {
                // 1. Verify CC.9 `space.cockpit.fire` parser across all 4 mount families & 5-band hull lock
                var cockpitJson = JsonNode.Parse(@"{
                    ""yaw_deg"": 42,
                    ""pitch_deg"": -6,
                    ""thrust_permille"": 500,
                    ""emcon"": ""ACTIVE"",
                    ""fire"": {
                        ""mounts"": [
                            { ""id"": ""laser_dorsal"", ""family"": ""laser"", ""ready_in_ms"": 0, ""state"": ""ready"", ""mode"": ""offence"", ""damage"": 98 },
                            { ""id"": ""laser_ventral"", ""family"": ""laser"", ""ready_in_ms"": 0, ""state"": ""ready"", ""mode"": ""defence"" },
                            { ""id"": ""tube_fore"", ""family"": ""tubes"", ""ready_in_ms"": 0, ""state"": ""ready"", ""rounds"": 3, ""rounds_max"": 4 },
                            { ""id"": ""rail_spinal"", ""family"": ""spinal"", ""ready_in_ms"": 2400, ""state"": ""cycling"", ""rounds"": 31, ""rounds_max"": 32, ""flight_ms"": 2200 },
                            { ""id"": ""pdc_port"", ""family"": ""point_defence"", ""ready_in_ms"": 0, ""state"": ""ready"", ""rounds"": 4000 }
                        ],
                        ""lock_id"": ""raider.7.1"",
                        ""lock"": {
                            ""id"": ""raider.7.1"",
                            ""label"": ""Belt Raider"",
                            ""range_m"": 42000,
                            ""facet"": ""top""
                        }
                    }
                }");

                var parsedMounts = new List<TacSim.Space.SpaceMountTelemetry>();
                TacSim.Space.SpaceFlightController.ParseSpaceFireTelemetry(
                    cockpitJson["fire"],
                    parsedMounts,
                    out var parsedLock);

                Assert.AreEqual(5, parsedMounts.Count);
                Assert.AreEqual("offense", parsedMounts[0].Mode, "Wire 'offence' normalizes to US 'offense'");
                Assert.AreEqual(98, parsedMounts[0].DamageAtLock);
                Assert.AreEqual("defense", parsedMounts[1].Mode, "Wire 'defence' normalizes to US 'defense'");
                Assert.AreEqual(3, parsedMounts[2].Rounds);
                Assert.AreEqual(4, parsedMounts[2].RoundsMax);
                Assert.AreEqual(31, parsedMounts[3].Rounds);
                Assert.AreEqual(32, parsedMounts[3].RoundsMax);
                Assert.AreEqual(2200, parsedMounts[3].FlightMsAtLock);
                Assert.AreEqual(4000, parsedMounts[4].Rounds);
                Assert.AreEqual(-1, parsedMounts[4].RoundsMax, "Rotary PDC omits rounds_max");

                Assert.IsNotNull(parsedLock);
                Assert.AreEqual("raider.7.1", parsedLock.ShipId);
                Assert.AreEqual("Belt Raider", parsedLock.Label);
                Assert.AreEqual(42000L, parsedLock.RangeM);
                Assert.AreEqual("top", parsedLock.Facet);

                // 2. Verify live SpaceFlightController 3D 5-Band Aspect Indicator & TacSimMissionOverlays Fire-Control Card
                var spaceGo = new UnityEngine.GameObject("SpaceStage");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<SpaceFlightController>();
                space.BuildSpaceStage(null);

                var hudGo = new UnityEngine.GameObject("Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                var hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, null, space);
                hud.Overlays.SetHelmSeatedVisible(true);

                var holotankJson = JsonNode.Parse(@"{
                    ""contacts"": [
                        { ""id"": ""ship.1"", ""you"": true, ""x_km"": 0.0, ""y_km"": 0.0, ""z_km"": 0.0 },
                        { ""id"": ""raider.7.1"", ""label"": ""Belt Raider"", ""kind"": ""ship"", ""relation"": ""hostile"", ""you"": false, ""locked"": true, ""x_km"": 42.0, ""y_km"": 0.0, ""z_km"": 0.0 }
                    ]
                }");
                space.ApplyHolotankFeed(holotankJson);
                space.ApplyCockpitFeed(cockpitJson);
                hud.Overlays.UpdateOrbitalTargetReadout(
                    space.SelectedRockId,
                    space.LockedContactId,
                    null,
                    null,
                    space.ActiveLock?.Facet);

                Assert.IsTrue(hud.Overlays.IsSpaceFireCardVisible);
                StringAssert.Contains("[TOP]", hud.Overlays.SpaceLockSummaryText);
                StringAssert.Contains("42.0 km", hud.Overlays.SpaceLockSummaryText);
                Assert.AreEqual(5, hud.Overlays.VisibleSpaceMountRowCount);
                StringAssert.Contains("98 DMG", hud.Overlays.GetSpaceMountRowText(0));
                StringAssert.Contains("DEFENSE SCREEN", hud.Overlays.GetSpaceMountRowText(1));
                StringAssert.Contains("3/4", hud.Overlays.GetSpaceMountRowText(2));
                StringAssert.Contains("CYCLING 2.4s", hud.Overlays.GetSpaceMountRowText(3));
                StringAssert.Contains("2.2s FLIGHT", hud.Overlays.GetSpaceMountRowText(3));
                StringAssert.Contains("4000 LEFT", hud.Overlays.GetSpaceMountRowText(4));

                var lockInd = spaceGo.transform.Find("LockedAspectIndicator");
                Assert.IsNotNull(lockInd);
                Assert.IsTrue(lockInd.gameObject.activeSelf);
                var bandArc = lockInd.Find("StrikeBandArc");
                Assert.IsNotNull(bandArc);
                Assert.AreEqual(6.4f, bandArc.localPosition.y, 0.05f, "'top' facet elevates strike arc over dorsal hull");

                // 3. Verify Doc 49 Ground Arena multi-mount, `weapons_free`, & `armed` condition-slot decoding
                var arenaGo = new UnityEngine.GameObject("ArenaStage");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                hud.Overlays.BindControllers(null, arena, space);

                var groundFrame = JsonNode.Parse(@"{
                    ""match"": 77,
                    ""seated"": true,
                    ""callsign"": ""R1"",
                    ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"",
                    ""power"": ""online"",
                    ""weapons_free"": false,
                    ""weapons"": [
                        {
                            ""id"": ""ac10"",
                            ""ready"": false,
                            ""ready_in_ms"": 10000,
                            ""rounds"": 18,
                            ""rounds_max"": 20,
                            ""percent"": 62,
                            ""heat_per_shot"": 120,
                            ""armed"": { ""slot"": 1, ""target"": ""lock"", ""percent"": 55, ""keeps"": 2 }
                        },
                        {
                            ""id"": ""coax_mg"",
                            ""ready"": true,
                            ""ready_in_ms"": 0,
                            ""rounds"": 90,
                            ""rounds_max"": 100,
                            ""percent"": 48,
                            ""heat_per_shot"": 15,
                            ""armed"": null
                        }
                    ]
                }");
                arena.ApplyLiveCockpitFrame(groundFrame);
                Assert.IsFalse(arena.State.WeaponsFree);
                Assert.AreEqual(2, arena.State.Weapons.Count);
                Assert.AreEqual(1, arena.State.Weapons[0].ArmedSlot);
                Assert.AreEqual(55, arena.State.Weapons[0].ArmedMinPercent);
                Assert.AreEqual(2, arena.State.Weapons[0].ArmedKeeps);
                Assert.AreEqual(0, arena.State.Weapons[1].ArmedSlot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Security_U5_PathTraversalAndUncSlugs_AreRejectedBeforeFilesystemAccess()
        {
            Assert.IsTrue(TacSimContentLoader.IsSafeContentSlug("ceres_freeport"));
            Assert.IsTrue(TacSimContentLoader.IsSafeContentSlug("neighborhood.ooc_area"));
            Assert.IsTrue(TacSimContentLoader.IsSafeContentSlug("mars-canyon.v1"));

            string[] unsafeSlugs =
            {
                "",
                ".",
                "..",
                "../secret",
                "..\\secret",
                "a..b",
                "foo/bar",
                "foo\\bar",
                @"\\attacker.invalid\share\leak",
                "//attacker.invalid/share/leak",
                @"C:\Windows\win.ini",
                "C:relative",
                "bad slug",
                new string('a', 129)
            };
            for (int i = 0; i < unsafeSlugs.Length; i++)
            {
                Assert.IsFalse(TacSimContentLoader.IsSafeContentSlug(unsafeSlugs[i]), $"Expected slug rejected: '{unsafeSlugs[i]}'");
            }

            string[] unsafeRelPaths =
            {
                @"\\attacker.invalid\share\file.json",
                "//attacker.invalid/share/file.json",
                @"C:\Windows\System32\drivers\etc\hosts",
                "/etc/passwd",
                "C:atlas/foo.json",
                "../../Cargo.toml",
                @"atlas\..\..\Cargo.toml",
                "atlas/../market/goods.json"
            };
            for (int i = 0; i < unsafeRelPaths.Length; i++)
            {
                Assert.AreEqual(string.Empty, TacSimContentLoader.ResolveRepoContentPath(unsafeRelPaths[i]), $"Repo resolver must reject '{unsafeRelPaths[i]}'");
                Assert.AreEqual(string.Empty, TacSimContentLoader.ResolveMushAtlasContentPath(unsafeRelPaths[i]), $"Atlas resolver must reject '{unsafeRelPaths[i]}'");
            }

            // Even if an attacker supplies a valid path to an existing JSON file outside content roots, loaders must fall back without reading it.
            string tempFile = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "tacsim_u5_" + System.Guid.NewGuid().ToString("N"));
            string tempJsonPath = tempFile + ".json";
            try
            {
                System.IO.File.WriteAllText(tempJsonPath, @"{""entries"":{""room.pwn"":{""kind"":""room"",""def"":{""name"":""PWNED""}}}}");

                var nb = TacSimContentLoader.LoadNeighborhoodPackById(tempFile);
                Assert.IsNotNull(nb);
                Assert.IsFalse(nb.Rooms.ContainsKey("room.pwn"), "Absolute traversal path must not be read by LoadNeighborhoodPackById");

                var nbUnc = TacSimContentLoader.LoadNeighborhoodPackById(@"\\attacker.invalid\share\evil");
                Assert.IsNotNull(nbUnc);

                var ship = TacSimContentLoader.LoadShipTemplate(@"..\..\evil");
                Assert.IsNotNull(ship);
                Assert.AreEqual("tacsim.unit.dray", ship.UnitId);

                var arena = TacSimTerrainDecoder.LoadStaticArena(@"\\attacker.invalid\share\arena");
                Assert.IsNotNull(arena);
                Assert.AreEqual("mars_canyon", arena.Id);

                var site = TacSimTerrainDecoder.BuildPlanetarySiteGrid(@"..\..\secret", side: 64);
                Assert.IsNotNull(site);
                Assert.AreEqual("occator_rim", site.Id);
            }
            finally
            {
                try { if (System.IO.File.Exists(tempJsonPath)) System.IO.File.Delete(tempJsonPath); } catch { }
            }
        }

        [Test]
        public void Remediation_UC01_StandaloneBuilderResolvesDirectories_AndStagesRealPacksFromUnityCwd()
        {
            string prevCwd = System.IO.Directory.GetCurrentDirectory();
            string tempOut = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimStageTest_" + System.Guid.NewGuid().ToString("N"));
            try
            {
                // Locate <repo>/unity to replicate the Unity Editor working directory where File.Exists previously failed!
                string dir = prevCwd;
                string unityDir = null;
                for (int i = 0; i < 6 && !string.IsNullOrEmpty(dir); i++)
                {
                    string candidate = System.IO.Path.Combine(dir, "unity");
                    if (System.IO.Directory.Exists(candidate) &&
                        System.IO.Directory.Exists(System.IO.Path.Combine(dir, "crates", "tacsim", "content")))
                    {
                        unityDir = candidate;
                        break;
                    }
                    dir = System.IO.Directory.GetParent(dir)?.FullName;
                }
                Assert.IsNotNull(unityDir, "Expected <repo>/unity directory alongside crates/tacsim/content");
                System.IO.Directory.SetCurrentDirectory(unityDir);

                Assert.IsTrue(System.IO.Directory.Exists(TacSimContentLoader.ResolveRepoContentPath("atlas")), "atlas directory must resolve from unity/ cwd");
                Assert.IsTrue(System.IO.Directory.Exists(TacSimContentLoader.ResolveRepoContentPath("cast")), "cast directory must resolve from unity/ cwd");
                Assert.IsTrue(System.IO.Directory.Exists(TacSimContentLoader.ResolveRepoContentPath("units")), "units directory must resolve from unity/ cwd");

                int staged = TacSim.Editor.TacSimStandaloneBuilder.StageEmbeddedFallbackPacks(tempOut, refreshAssetDatabase: false);
                Assert.GreaterOrEqual(staged, 15, "Standalone builder must stage neighborhoods, casts, units, regions, arenas, and goods");
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "neighborhood.ceres_freeport.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "neighborhood.stickney_yards.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "neighborhood.ooc_area.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "cast.ceres_freeport.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "dray.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "dray_p.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "goods.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "arena.mars_canyon.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "region.occator_rim.json")));
            }
            finally
            {
                System.IO.Directory.SetCurrentDirectory(prevCwd);
                if (System.IO.Directory.Exists(tempOut))
                    System.IO.Directory.Delete(tempOut, recursive: true);
            }
        }

        [Test]
        public void Security_U3_JsonDepthAndNumericGuards_PreventStackOverflowAndNonFiniteValues()
        {
            // Depth 64 must parse; depth 65 (and deep 4000-level nesting) must throw FormatException without StackOverflowException.
            string depth64 = new string('[', JsonNode.MaxDepth) + new string(']', JsonNode.MaxDepth);
            var ok = JsonNode.Parse(depth64);
            Assert.IsTrue(ok.IsArray);

            string depth65 = new string('[', JsonNode.MaxDepth + 1) + new string(']', JsonNode.MaxDepth + 1);
            Assert.Throws<System.FormatException>(() => JsonNode.Parse(depth65));

            string deepObj = string.Concat(System.Linq.Enumerable.Repeat("{\"a\":", 500)) + "1" + new string('}', 500);
            Assert.Throws<System.FormatException>(() => JsonNode.Parse(deepObj));

            // Clone and ToJson must also guard manually constructed graphs deeper than MaxDepth.
            JsonNode manualRoot = JsonNode.NewArray();
            JsonNode cur = manualRoot;
            for (int i = 0; i < JsonNode.MaxDepth + 2; i++)
            {
                var next = JsonNode.NewArray();
                cur.Add(next);
                cur = next;
            }
            Assert.Throws<System.InvalidOperationException>(() => manualRoot.Clone());
            Assert.Throws<System.InvalidOperationException>(() => manualRoot.ToJson());

            // Non-finite numbers and out-of-range integer coercions must be rejected/clamped safely.
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("1e309"));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("-1e309"));
            Assert.IsTrue(JsonNode.FromDouble(double.NaN).IsNull);
            Assert.IsTrue(JsonNode.FromDouble(double.PositiveInfinity).IsNull);
            Assert.IsTrue(JsonNode.FromDouble(double.NegativeInfinity).IsNull);

            Assert.AreEqual(7.5, JsonNode.FromString("NaN").AsDouble(7.5));
            Assert.AreEqual(7.5, JsonNode.FromString("Infinity").AsDouble(7.5));

            var hugeFloat = JsonNode.FromDouble(1e30);
            Assert.AreEqual(42L, hugeFloat.AsLong(42L));
            Assert.AreEqual(99, JsonNode.FromLong((long)int.MaxValue + 10L).AsInt(99));
            Assert.AreEqual(99, JsonNode.FromLong((long)int.MinValue - 10L).AsInt(99));
        }

        [Test]
        public void Security_U3_WsClientBounds_RejectBinaryOversize_CapQueues_AndBoundReplyDepth()
        {
            // Frame validation: binary rejected immediately; >4 MiB rejected immediately; <=4 MiB text allowed.
            Assert.IsFalse(MushWsClient.TryValidateInboundFrame(
                System.Net.WebSockets.WebSocketMessageType.Binary, 0, 16, out var binStatus, out _));
            Assert.AreEqual(System.Net.WebSockets.WebSocketCloseStatus.InvalidMessageType, binStatus);

            Assert.IsFalse(MushWsClient.TryValidateInboundFrame(
                System.Net.WebSockets.WebSocketMessageType.Text, MushWsClient.MaxInboundMessageBytes, 1, out var bigStatus, out _));
            Assert.AreEqual(System.Net.WebSockets.WebSocketCloseStatus.MessageTooBig, bigStatus);

            Assert.IsTrue(MushWsClient.TryValidateInboundFrame(
                System.Net.WebSockets.WebSocketMessageType.Text, MushWsClient.MaxInboundMessageBytes - 10, 10, out _, out _));

            using (var ws = new MushWsClient())
            {
                // Inbound queue cap (2048)
                var env = MushProtocol.ParseEnvelope(@"{""seq"":1,""ts"":1,""t"":""sys"",""d"":{""text"":""hi""}}");
                for (int i = 0; i < MushWsClient.MaxInboundQueueCount + 100; i++)
                    ws.InjectSyntheticEnvelope(env);
                Assert.AreEqual(MushWsClient.MaxInboundQueueCount, ws.InboundQueueCount);
                ws.Disconnect();

                // Outbound queue cap (256) on a fresh pre-disconnect test client
                using (var wsOut = new MushWsClient())
                {
                    for (int i = 0; i < MushWsClient.MaxOutboundQueueCount + 50; i++)
                        wsOut.SendLine("look");
                    Assert.AreEqual(MushWsClient.MaxOutboundQueueCount, wsOut.OutboundQueueCount);
                }

                // Panel feed map cap (64)
                for (int i = 0; i < MushWsClient.MaxPanelFeeds + 20; i++)
                {
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{""seq"":{i + 1},""ts"":1,""t"":""panel"",""d"":{{""feed"":""f_{i}"",""kind"":""keyframe"",""data"":{{""n"":{i}}}}}}}"));
                    ws.PumpMainThread();
                }
                Assert.AreEqual(MushWsClient.MaxPanelFeeds, ws.PanelFeedCount);
                ws.Disconnect();

                // View cursor map cap (64)
                for (int i = 0; i < MushWsClient.MaxViewCursors + 20; i++)
                {
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{""seq"":{i + 1},""ts"":1,""t"":""view"",""d"":{{""view"":""v_{i}"",""generation"":""g1"",""sequence"":1,""data"":{{""n"":{i}}}}}}}"));
                    ws.PumpMainThread();
                }
                Assert.AreEqual(MushWsClient.MaxViewCursors, ws.ViewCursorCount);
                ws.Disconnect();

                // Iterative reply unwrap: depth 1 dispatches; depth 2 and depth 30 are ignored without recursion overflow.
                int rooms = 0;
                ws.OnRoomReceived += _ => rooms++;

                var replyDepth1 = MushProtocol.ParseEnvelope(@"{""seq"":1,""ts"":1,""t"":""reply"",""d"":{""event"":{""t"":""room"",""d"":{""id"":""#1"",""name"":""OK""}}}}");
                ws.InjectSyntheticEnvelope(replyDepth1);
                ws.PumpMainThread();
                Assert.AreEqual(1, rooms, "Depth-1 reply envelope must unwrap and dispatch");

                var replyDepth2 = MushProtocol.ParseEnvelope(@"{""seq"":2,""ts"":2,""t"":""reply"",""d"":{""event"":{""t"":""reply"",""d"":{""event"":{""t"":""room"",""d"":{""id"":""#2"",""name"":""Nested""}}}}}}");
                ws.InjectSyntheticEnvelope(replyDepth2);
                ws.PumpMainThread();
                Assert.AreEqual(1, rooms, "Depth-2 reply envelope must be refused (max depth <= 1)");

                JsonNode nested = JsonNode.Parse(@"{""t"":""room"",""d"":{""id"":""#3"",""name"":""Deep""}}");
                for (int i = 0; i < 30; i++)
                {
                    var wrapper = JsonNode.NewObject();
                    wrapper["t"] = JsonNode.FromString("reply");
                    var wd = JsonNode.NewObject();
                    wd["event"] = nested;
                    wrapper["d"] = wd;
                    nested = wrapper;
                }
                ws.InjectSyntheticEnvelope(new MushEnvelope { Seq = 3, TimestampMs = 3, EventType = "reply", Data = nested["d"] });
                ws.PumpMainThread();
                Assert.AreEqual(1, rooms, "Deeply nested reply chain must be ignored without stack overflow");
            }
        }

        [Test]
        public void Security_U4_TerrainDecoder_GuardsInt32OverflowBadBase64AndParameterClamps()
        {
            var grid = new DecodedTerrainGrid
            {
                Width = 256,
                Height = 256,
                Classes = new TerrainCellClass[256 * 256],
                ElevMeters = new float[256 * 256]
            };

            // Invalid base64 must not throw FormatException.
            string badB64Atlas = @"{""entries"":{""field.q0_0"":{""def"":{""classes"":""!!!not-base64!!!"",""data"":""!!!""}}}}";
            Assert.IsFalse(TacSimTerrainDecoder.TryStitchAtlasQuadrantTiles(badB64Atlas, grid));

            // tw = 65535, th = 65535 previously overflowed signed int32 (`tw * th == -131071`) or attempted multi-GB alloc!
            byte[] overflowHeader = { 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x00, 0x02 };
            string overflowB64 = System.Convert.ToBase64String(overflowHeader);
            string overflowAtlas = $@"{{""entries"":{{""field.q0_0"":{{""def"":{{""classes"":""{overflowB64}"",""data"":""AAAA""}}}}}}}}";
            Assert.IsFalse(TacSimTerrainDecoder.TryStitchAtlasQuadrantTiles(overflowAtlas, grid));

            // tw = 513 (exceeds 512 cap) and tw = 0 must be rejected.
            byte[] over512Header = { 0x01, 0x02, 0x01, 0x00, 0x01, 0x00, 0x02 };
            string over512Atlas = $@"{{""entries"":{{""field.q0_0"":{{""def"":{{""classes"":""{System.Convert.ToBase64String(over512Header)}"",""data"":""AAAA""}}}}}}}}";
            Assert.IsFalse(TacSimTerrainDecoder.TryStitchAtlasQuadrantTiles(over512Atlas, grid));

            byte[] zeroHeader = { 0x00, 0x00, 0x04, 0x00 };
            string zeroAtlas = $@"{{""entries"":{{""field.q0_0"":{{""def"":{{""classes"":""{System.Convert.ToBase64String(zeroHeader)}"",""data"":""AAAA""}}}}}}}}";
            Assert.IsFalse(TacSimTerrainDecoder.TryStitchAtlasQuadrantTiles(zeroAtlas, grid));

            // Valid 2x2 quadrant tile still stitches cleanly.
            byte[] validClasses = { 2, 0, 2, 0, 4, 0, (byte)TerrainCellClass.Rock };
            byte[] validElev = { 0x64, 0x00, 0xC8, 0x00, 0x2C, 0x01, 0x90, 0x01 }; // 100, 200, 300, 400 cm
            string validAtlas = $@"{{""entries"":{{""field.q0_0"":{{""def"":{{""classes"":""{System.Convert.ToBase64String(validClasses)}"",""data"":""{System.Convert.ToBase64String(validElev)}""}}}}}}}}";
            Assert.IsTrue(TacSimTerrainDecoder.TryStitchAtlasQuadrantTiles(validAtlas, grid));
            Assert.AreEqual(TerrainCellClass.Rock, grid.GetCellClass(0, 0));
            Assert.AreEqual(1.0f, grid.ElevMeters[0], 0.01f);
            Assert.AreEqual(4.0f, grid.ElevMeters[1 * 256 + 1], 0.01f);

            // Clamps on span, baseCellCm, unit_cm, and side.
            var siteFrame = JsonNode.Parse(@"{""width"":4,""height"":4,""cell_cm"":99999999,""rows"":[[["","",4]],[["","",4]],[["","",4]],[["","",4]]],""site"":{""grid"":{""x0"":0,""y0"":0,""span"":99999,""cell_cm"":99999999}}}");
            var decodedSite = TacSimTerrainDecoder.DecodeSiteMapFrame(siteFrame);
            Assert.IsNotNull(decodedSite);
            Assert.LessOrEqual(decodedSite.SpanCells, 64);
            Assert.LessOrEqual(decodedSite.CellSizeM, 64000f);

            var clampedSmall = TacSimTerrainDecoder.BuildPlanetarySiteGrid("occator_rim", side: -5);
            Assert.AreEqual(8, clampedSmall.Width);
            var clampedBig = TacSimTerrainDecoder.BuildPlanetarySiteGrid("occator_rim", side: 999999);
            Assert.AreEqual(TacSimTerrainDecoder.MaxGridDimension, clampedBig.Width);
        }

        [Test]
        public void Security_U6_ContentSync_RejectsUnsolicitedOrMismatchedPacks_EnforcesQuotas_AndUsesSafeHexNames()
        {
            string tempDir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimU6_" + System.Guid.NewGuid().ToString("N"));
            try
            {
                var sync = new TacSimContentSync();
                sync.InitializeDiskCacheForServer("https://u6.example.com", tempDir);

                // 1. Unsolicited op=="pack" (not in _inFlightPacks) must be rejected with zero memory/disk writes.
                string rogueJson = @"{""rogue"":true}";
                string rogueSha = TacSimContentSync.ComputeSha256Hex(rogueJson);
                using (var ws = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""rogue"",""sha256"":""{rogueSha}"",""json"":{JsonNode.FromString(rogueJson).ToJson()}}}"), 1f);
                }
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("rogue", out _), "Unsolicited pack must not register");
                Assert.AreEqual(0, sync.CachedPackCount);

                // 2. Empty SHA-256 and self-referential SHA-256 mismatching manifest entry must be rejected.
                Assert.IsFalse(sync.StoreVerifiedPack("empty_sha", "", rogueJson), "Empty expected SHA-256 must be rejected");
                Assert.IsFalse(sync.StoreVerifiedPack("null_sha", null, rogueJson), "Null expected SHA-256 must be rejected");

                string manifestJson = @"{""ok"":1}";
                string manifestSha = TacSimContentSync.ComputeSha256Hex(manifestJson);
                string tamperedJson = @"{""ok"":2}";
                string tamperedSha = TacSimContentSync.ComputeSha256Hex(tamperedJson);
                using (var ws = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{""op"":""manifest"",""packs"":[{{""id"":""legit"",""category"":""unit"",""sha256"":""{manifestSha}"",""bytes"":8}}]}}"), 2f);
                    // Server sends tampered payload with self-consistent tamperedSha (does NOT match manifestSha!)
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""legit"",""requested_id"":""legit"",""sha256"":""{tamperedSha}"",""json"":{JsonNode.FromString(tamperedJson).ToJson()}}}"), 2.1f);
                }
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("legit", out _), "Self-referential digest mismatching manifest must be rejected");

                // 3. Per-pack 2 MiB quota must reject oversized packs.
                string over2MiB = new string('x', TacSimContentSync.MaxPackBytes + 1);
                string overSha = TacSimContentSync.ComputeSha256Hex(over2MiB);
                Assert.IsFalse(sync.StoreVerifiedPack("huge", overSha, over2MiB), "Packs > 2 MiB must be rejected");

                // 4. Safe SHA-256 hex cache filenames prevent slug collisions and Windows reserved device names (`CON`, `NUL`).
                string f1 = TacSimContentSync.CacheFileNameForPackId("a/b");
                string f2 = TacSimContentSync.CacheFileNameForPackId("a_b");
                string fCon = TacSimContentSync.CacheFileNameForPackId("CON");
                Assert.AreNotEqual(f1, f2, "Distinct IDs with punctuation must not collide in cache filenames");
                Assert.AreEqual(69, fCon.Length, "Cache filename must be 64-char SHA-256 hex + '.json'");
                Assert.IsFalse(fCon.StartsWith("CON", System.StringComparison.OrdinalIgnoreCase));
                Assert.IsTrue(sync.StoreVerifiedPack("CON", manifestSha, manifestJson), "Pack named CON must persist safely via SHA-256 hex filename");

                // 5. Total file count quota (256 files) must reject the 257th distinct pack.
                var countSync = new TacSimContentSync();
                for (int i = 0; i < TacSimContentSync.MaxCachedPacks; i++)
                {
                    string pj = $"{{\"i\":{i}}}";
                    Assert.IsTrue(countSync.StoreVerifiedPack($"p_{i}", TacSimContentSync.ComputeSha256Hex(pj), pj));
                }
                string overflowPack = @"{""i"":999}";
                Assert.IsFalse(countSync.StoreVerifiedPack("p_overflow", TacSimContentSync.ComputeSha256Hex(overflowPack), overflowPack), "257th pack must be rejected by MaxCachedPacks quota");

                // 6. Total byte quota (64 MiB) must reject once cumulative cached bytes would exceed 64 MiB.
                var byteSync = new TacSimContentSync();
                string chunk1MiB = "{\"d\":\"" + new string('a', (1024 * 1024) - 8) + "\"}";
                string chunkSha = TacSimContentSync.ComputeSha256Hex(chunk1MiB);
                for (int i = 0; i < 64; i++)
                    Assert.IsTrue(byteSync.StoreVerifiedPack($"mb_{i}", chunkSha, chunk1MiB));
                Assert.IsFalse(byteSync.StoreVerifiedPack("mb_65", chunkSha, chunk1MiB), "65th 1 MiB pack must be rejected by 64 MiB cache quota");
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
                try { if (System.IO.Directory.Exists(tempDir)) System.IO.Directory.Delete(tempDir, recursive: true); } catch { }
            }
        }

        [Test]
        public void Remediation_UC02_UH16_StationWallsCutAllMultiDoorways_AndSealPartialOverlapVoid()
        {
            var rootGo = new UnityEngine.GameObject("TestMultiDoorStation");
            try
            {
                var sw = rootGo.AddComponent<StationWorldController>();

                // U-C02: Stickney Yards `room.p04_concourse` has 4 north doorways along y=155
                sw.BuildStation(null, "neighborhood.stickney_yards");
                Assert.AreEqual("neighborhood.stickney_yards", sw.LoadedPackId);
                var concourseTr = rootGo.transform.Find("room.p04_concourse");
                Assert.IsNotNull(concourseTr);

                var pack = sw.Pack;
                var northDoorCentersX = new List<float>();
                foreach (var ex in pack.Exits)
                {
                    if (ex.FromRoom == "room.p04_concourse" &&
                        pack.Rooms.TryGetValue(ex.ToRoom, out var dst) &&
                        System.Math.Abs(dst.MinY - 155f) < 0.6f)
                    {
                        northDoorCentersX.Add(ex.PortalA.sqrMagnitude > 0.01f ? ex.PortalA.x : (dst.MinX + dst.MaxX) * 0.5f);
                    }
                }
                Assert.GreaterOrEqual(northDoorCentersX.Count, 4, "Stickney concourse must have 4 north exits");

                // Every colliding wall along north edge (atlas y=155 -> unity z=155-OriginY) must leave all 4 doorway centers unobstructed!
                float northZ = sw.AtlasToUnity(0f, 0f, 155f).z;
                for (int i = 0; i < concourseTr.childCount; i++)
                {
                    var ch = concourseTr.GetChild(i);
                    if (!ch.name.StartsWith("Wall_H_Seg")) continue;
                    if (UnityEngine.Mathf.Abs(ch.position.z - northZ) > 0.5f) continue;
                    if (ch.GetComponent<UnityEngine.BoxCollider>() == null) continue;
                    float segMinX = ch.position.x - ch.localScale.x * 0.5f;
                    float segMaxX = ch.position.x + ch.localScale.x * 0.5f;
                    for (int d = 0; d < northDoorCentersX.Count; d++)
                    {
                        float doorUnityX = sw.AtlasToUnity(northDoorCentersX[d], 0f, 155f).x;
                        Assert.IsFalse(doorUnityX > segMinX + 0.1f && doorUnityX < segMaxX - 0.1f,
                            $"Wall segment [{segMinX:F1}..{segMaxX:F1}] seals Stickney doorway at x={doorUnityX:F1}");
                    }
                }

                // U-H16: Aitken Highport `room.l08_receiving` (x=140..210, y=140) overlaps `l12_dockmaster` (140..185)
                // and MUST still build a residual south wall covering x=185..210!
                sw.BuildStation(null, "neighborhood.aitken_highport");
                var recvTr = rootGo.transform.Find("room.l08_receiving");
                Assert.IsNotNull(recvTr);
                float southZ = sw.AtlasToUnity(0f, 0f, 140f).z;
                float expectedMidX = sw.AtlasToUnity(197.5f, 0f, 140f).x;
                bool foundResidualSouthWall = false;
                for (int i = 0; i < recvTr.childCount; i++)
                {
                    var ch = recvTr.GetChild(i);
                    if (!ch.name.StartsWith("Wall_H")) continue;
                    if (UnityEngine.Mathf.Abs(ch.position.z - southZ) > 0.5f) continue;
                    float segMinX = ch.position.x - ch.localScale.x * 0.5f;
                    float segMaxX = ch.position.x + ch.localScale.x * 0.5f;
                    if (expectedMidX >= segMinX && expectedMidX <= segMaxX)
                        foundResidualSouthWall = true;
                }
                Assert.IsTrue(foundResidualSouthWall, "Aitken l08_receiving must seal residual south wall across x=185..210");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Remediation_UC03_UH18_OocGroundSurfaceFloor0_TransitPortalArch_AndUniformPropRoots()
        {
            var rootGo = new UnityEngine.GameObject("TestOocStation");
            try
            {
                var ooc = TacSimContentLoader.LoadNeighborhoodPackById("neighborhood.ooc_area");
                Assert.IsNotNull(ooc);
                Assert.AreEqual(0, ooc.Rooms["room.welcome_hall"].FloorIndex);
                Assert.AreEqual(0, ooc.Rooms["room.bus_stop"].FloorIndex, "\"surface\": \"ground\" must map to FloorIndex 0");
                Assert.AreEqual(0, ooc.Rooms["room.simulator_lobby"].FloorIndex, "\"surface\": \"ground\" must map to FloorIndex 0");
                Assert.AreEqual(0, ooc.Rooms["room.sim_red"].FloorIndex);
                Assert.AreEqual(0, ooc.Rooms["room.sim_blue"].FloorIndex);
                Assert.AreEqual(0f, ooc.GetFloorElevationY(ooc.Rooms["room.simulator_lobby"].FloorIndex), 0.001f);

                var sw = rootGo.AddComponent<StationWorldController>();
                sw.BuildStation(null, "neighborhood.ooc_area");

                bool foundTransitArch = false;
                bool foundSelectorRoot = false;
                bool foundKioskRoot = false;
                int uniformSimPods = 0;

                var allTr = rootGo.GetComponentsInChildren<UnityEngine.Transform>(true);
                for (int i = 0; i < allTr.Length; i++)
                {
                    var tr = allTr[i];
                    if (tr.name.StartsWith("TransitPortal_")) foundTransitArch = true;
                    if (tr.name == "SimulatorSelectorConsole")
                    {
                        foundSelectorRoot = true;
                        Assert.AreEqual(UnityEngine.Vector3.one, tr.localScale, "SimulatorSelectorConsole root must have uniform scale");
                    }
                    if (tr.name == "ReturnToIcKiosk")
                    {
                        foundKioskRoot = true;
                        Assert.AreEqual(UnityEngine.Vector3.one, tr.localScale, "ReturnToIcKiosk root must have uniform scale");
                    }
                    if (tr.name.StartsWith("RED_Pod_") || tr.name.StartsWith("BLUE_Pod_"))
                    {
                        Assert.AreEqual(UnityEngine.Vector3.one, tr.localScale, $"{tr.name} root must have uniform scale");
                        uniformSimPods++;
                    }
                }

                Assert.IsTrue(foundTransitArch, "Non-adjacent OOC exit.hall_simulator_lobby must spawn explicit TransitPortalArch");
                Assert.IsTrue(foundSelectorRoot);
                Assert.IsTrue(foundKioskRoot);
                Assert.AreEqual(12, uniformSimPods);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Remediation_UC04_UH15_UH17_UH18_UH19_ShipHysteresis_HalcyonRelay_PersistentAnchor_AndHoloTank()
        {
            var rootGo = new UnityEngine.GameObject("TestShipFixes");
            try
            {
                var ship = rootGo.AddComponent<ShipInteriorController>();
                ship.BuildInterior(null, "dray_p");

                // U-C04: Vertical hysteresis must NOT trap player in lower deck room!
                UnityEngine.Vector3 bridgeFloorWorld = ship.transform.position + new UnityEngine.Vector3(0f, 1.85f, 8.0f);
                Assert.AreEqual("bridge", ship.TryFindContainingCompartment(bridgeFloorWorld, currentCompId: "passage", hysteresisM: 0.55f),
                    "Standing on Bridge deck (Y=+1.85m) must never be trapped in 'passage'");

                UnityEngine.Vector3 passageFloorWorld = ship.transform.position + new UnityEngine.Vector3(0f, -1.55f, 2.0f);
                Assert.AreEqual("passage", ship.TryFindContainingCompartment(passageFloorWorld, currentCompId: "hold", hysteresisM: 0.55f),
                    "Standing on Passage deck (Y=-1.55m) must never be trapped in 'hold'");

                // U-H15: Engineering & Bay meet flush at Z=-14.0 and floor Y=-5.3
                var eng = ship.Template.Compartments["engineering"];
                var bay = ship.Template.Compartments["bay"];
                Assert.AreEqual(eng.LocalCenter.z - eng.LocalSize.z * 0.5f, bay.LocalCenter.z + bay.LocalSize.z * 0.5f, 0.01f,
                    "Engineering aft bulkhead and Bay forward bulkhead must meet flush with zero 0.5m chasm");
                Assert.AreEqual(eng.LocalCenter.y - eng.LocalSize.y * 0.5f, bay.LocalCenter.y - bay.LocalSize.y * 0.5f, 0.01f,
                    "Engineering and Bay floors must be flush");

                // U-H18: Unscaled airlock & holotank roots
                var allTr = rootGo.GetComponentsInChildren<UnityEngine.Transform>(true);
                bool checkedLock = false, checkedPedestal = false;
                for (int i = 0; i < allTr.Length; i++)
                {
                    if (allTr[i].name == "OuterAirlockDoor") { checkedLock = true; Assert.AreEqual(UnityEngine.Vector3.one, allTr[i].localScale); }
                    if (allTr[i].name == "HoloTankPedestal") { checkedPedestal = true; Assert.AreEqual(UnityEngine.Vector3.one, allTr[i].localScale); }
                }
                Assert.IsTrue(checkedLock);
                Assert.IsTrue(checkedPedestal);

                // U-H19: HoloTank observer subtraction, (dy, dz, dx) axis match, disc clamp, and torpedoes[]
                var holoJson = JsonNode.Parse(@"{
                    ""range_km"": 100.0,
                    ""contacts"": [
                        { ""id"": ""ship.1"", ""you"": true, ""x_km"": 10.0, ""y_km"": 20.0, ""z_km"": 5.0 },
                        { ""id"": ""ahead_1"", ""you"": false, ""x_km"": 60.0, ""y_km"": 20.0, ""z_km"": 5.0, ""relation"": ""hostile"" },
                        { ""id"": ""far_1"", ""you"": false, ""x_km"": 5000.0, ""y_km"": 5000.0, ""z_km"": 0.0, ""relation"": ""neutral"" }
                    ],
                    ""torpedoes"": [
                        { ""id"": ""t1"", ""x_km"": 10.0, ""y_km"": 40.0, ""z_km"": 5.0, ""tti_s"": 12 }
                    ]
                }");
                ship.PopulateHoloBlips(holoJson);
                var blipsRoot = ship.transform.Find("HoloTankPedestal")?.Find("HoloTankContacts");
                Assert.IsNotNull(blipsRoot);
                Assert.AreEqual(4, blipsRoot.childCount, "Expected own-ship center blip + 2 external contacts + 1 torpedo blip");
                var aheadBlip = blipsRoot.GetChild(1);
                Assert.Greater(aheadBlip.localPosition.z, 0.25f, "+dx_km ahead maps to +Z on holotank matching SpaceFlightController");
                Assert.AreEqual(0f, aheadBlip.localPosition.x, 0.02f);
                var farBlip = blipsRoot.GetChild(2);
                float farRadius = UnityEngine.Mathf.Sqrt(farBlip.localPosition.x * farBlip.localPosition.x + farBlip.localPosition.z * farBlip.localPosition.z);
                Assert.LessOrEqual(farRadius, 0.71f, "Distant blips must clamp within holotank disc radius");
                var torpBlip = blipsRoot.GetChild(3);
                Assert.Greater(torpBlip.localPosition.x, 0.1f, "+dy_km starboard torpedo maps to +X");

                // U-H17 & U-H15: BridgeCameraAnchor survives rebuild + Halcyon has valid 'relay' compartment bounds
                var originalAnchor = ship.BridgeCameraAnchor;
                Assert.IsNotNull(originalAnchor);
                int rebuiltEvents = 0;
                ship.OnInteriorRebuilt += () => rebuiltEvents++;
                ship.BuildInterior(null, "halcyon");
                Assert.AreSame(originalAnchor, ship.BridgeCameraAnchor, "BridgeCameraAnchor must persist across interior rebuilds");
                Assert.AreEqual(1, rebuiltEvents);
                Assert.IsTrue(ship.Template.Compartments.ContainsKey("relay"), "Halcyon must expose its 'relay' compartment");
                Assert.Greater(ship.Template.Compartments["relay"].LocalCenter.x, 3.0f, "Halcyon relay must sit starboard off passage, not detached at Z=-48");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Remediation_UC05_UH24_OocArenaLiveSchema_Verbs_OriginWindow_AndTerrainElevation()
        {
            var rootGo = new UnityEngine.GameObject("TestArenaFixes");
            try
            {
                var arena = rootGo.AddComponent<TacSim.Arena.OocArenaController>();
                var emitted = new List<string>();
                arena.OnLiveCockpitVerbRequested += emitted.Add;

                // U-H24: Windowed arena.map with OriginCell (50, 60) + vertex-exact elevation sampling
                int[] elevCm = new int[16];
                for (int i = 0; i < 16; i++) elevCm[i] = i * 200;
                string encElev = TacSimTerrainDecoder.EncodeZigzagLeb128DeltaElevCm(elevCm);
                var mapNode = JsonNode.Parse($@"{{
                    ""match"": 88,
                    ""surface"": ""mars_canyon"",
                    ""biome"": ""desert"",
                    ""cell_cm"": 1000,
                    ""width"": 4,
                    ""height"": 4,
                    ""origin_cell"": [50, 60],
                    ""rows"": [[["":"", 4]], [["":"", 4]], [["":"", 4]], [["":"", 4]]],
                    ""spots"": [{{ ""letter"": ""A"", ""name"": ""West Ford"", ""cell"": [52, 62] }}],
                    ""elev"": {{ ""encoding"": ""zigzag-leb128-delta"", ""unit_cm"": 1, ""data"": ""{encElev}"" }}
                }}");
                arena.ApplyLiveArenaMap(mapNode);
                Assert.IsNotNull(arena.TerrainGrid);
                Assert.AreEqual(0f, arena.TerrainGrid.SampleElevationMeters(0f, 0f), 0.001f);
                Assert.AreEqual(2.0f, arena.TerrainGrid.SampleElevationMeters(10f, 0f), 0.001f,
                    "Elevation at vertex (1,0) must match ElevMeters[1] with zero -0.5 cell offset");

                UnityEngine.Vector3 localFromWorld = arena.ArenaWorldMetersToLocalPos(520f, 630f);
                Assert.AreEqual(20f, localFromWorld.x, 0.01f, "OriginCellX=50 (500m) must be subtracted from world X");
                Assert.AreEqual(30f, localFromWorld.z, 0.01f, "OriginCellY=60 (600m) must be subtracted from world Y");

                // U-C05: Live frames.rs cockpit schema: string ecm, viewer-relative facing, array odds[]
                var liveFrame = JsonNode.Parse(@"{
                    ""match"": 88,
                    ""seated"": true,
                    ""callsign"": ""R1"",
                    ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"",
                    ""x_m"": 515,
                    ""y_m"": 615,
                    ""heading"": 90,
                    ""ecm"": ""off"",
                    ""contacts"": [
                        {
                            ""tag"": ""B1"",
                            ""side"": ""B"",
                            ""kind"": ""tank"",
                            ""cell"": [52, 62],
                            ""range_m"": 240,
                            ""bearing"": 45,
                            ""facing"": ""front_right"",
                            ""odds"": [
                                { ""weapon"": ""ac10"", ""p_permille"": 640, ""percent"": 64 },
                                { ""weapon"": ""mg"", ""p_permille"": 400, ""percent"": 40 }
                            ]
                        }
                    ]
                }");
                arena.ApplyLiveCockpitFrame(liveFrame);
                Assert.IsTrue(arena.State.HasEcm);
                Assert.IsTrue(arena.State.EmconSilent, "ecm=='off' means EMCON silent");
                Assert.AreEqual(1, arena.State.Contacts.Count);
                Assert.AreEqual(64, arena.State.Contacts[0].HitOddsPct, "Array odds[] must decode max hit percent");
                Assert.AreNotEqual(0f, arena.State.Contacts[0].FacingDeg, "Viewer-relative 'front_right' must compute world heading");

                // Valid server verbs for stop, emcon toggle, and squad advance
                arena.ExecuteCockpitAction("stop");
                arena.ExecuteCockpitAction("emcon");
                arena.ExecuteCockpitAction("order all advance");
                Assert.AreEqual("speed 0", emitted[0]);
                Assert.AreEqual("emcon on", emitted[1], "When silent, EMCON toggle emits 'emcon on'");
                Assert.AreEqual("order all advance A", emitted[2], "Advance must supply key spot letter");

                // Flip ecm -> "on" and verify toggle emits "emcon off"
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{""match"":88,""seated"":true,""callsign"":""R1"",""party"":""R"",""ecm"":""on""}"));
                Assert.IsFalse(arena.State.EmconSilent);
                emitted.Clear();
                arena.ExecuteCockpitAction("emcon");
                Assert.AreEqual("emcon off", emitted[0]);

                // ecm -> null clears HasEcm
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{""match"":88,""seated"":true,""callsign"":""R1"",""party"":""R"",""ecm"":null}"));
                Assert.IsFalse(arena.State.HasEcm);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Remediation_UH01_UH07_TacSimJsonDepthLimit_StrictGrammar_AndU64()
        {
            // U-H01: Depth > MaxDepth (64) throws FormatException instead of StackOverflowException
            string deep = new string('[', 80) + new string(']', 80);
            Assert.Throws<System.FormatException>(() => JsonNode.Parse(deep));

            // U-H07: Strict JSON EOF, containers, numbers, and escapes
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("{\"a\":1} trailing"));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("{\"a\":1"));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("[1,2"));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("-"));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("1e400"));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("01"));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("\"\\x\""));
            Assert.Throws<System.FormatException>(() => JsonNode.Parse("\"\\u 41 \""));

            // U-H07: Full unsigned 64-bit integer precision (> long.MaxValue)
            ulong big = 18446744073709551610UL;
            var node = JsonNode.Parse($"{{\"id\":{big}}}");
            Assert.AreEqual(big, node.ULong("id"));
            var roundTripped = JsonNode.Parse(node.ToJson());
            Assert.AreEqual(big, roundTripped.ULong("id"));
        }

        [Test]
        public void Remediation_UH02_UnityPackageExtractorSecurityGuards()
        {
            string tempDir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimPkgSec_" + System.Guid.NewGuid().ToString("N"));
            try
            {
                // 1. Decompression / entry size bomb cap
                byte[] bigPayload = new byte[512];
                using (var bomb = BuildSyntheticUnityPackage(
                    ("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/asset", bigPayload),
                    ("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Big.bin\n"))))
                {
                    Assert.Throws<System.IO.InvalidDataException>(() =>
                        UnityPackageArchiveExtractor.ExtractPackageStream(bomb, tempDir, maxSingleEntryBytes: 128, maxTotalUncompressedBytes: 256));
                }

                // 2. Block overwriting Assets/_TacSim/ and any Editor/*.cs script
                using (var maliciousPkg = BuildSyntheticUnityPackage(
                    ("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/asset", System.Text.Encoding.UTF8.GetBytes("// evil")),
                    ("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/_TacSim/Scripts/Runtime/Evil.cs\n")),
                    ("cccccccccccccccccccccccccccccccc/asset", System.Text.Encoding.UTF8.GetBytes("// rce")),
                    ("cccccccccccccccccccccccccccccccc/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Editor/AutoRun.cs\n"))))
                {
                    int extracted = UnityPackageArchiveExtractor.ExtractPackageStream(maliciousPkg, tempDir);
                    Assert.AreEqual(0, extracted, "Package must never overwrite Assets/_TacSim or Editor/*.cs scripts");
                }
                Assert.IsFalse(System.IO.File.Exists(System.IO.Path.Combine(tempDir, "Assets", "_TacSim", "Scripts", "Runtime", "Evil.cs")));
                Assert.IsFalse(System.IO.File.Exists(System.IO.Path.Combine(tempDir, "Assets", "Vendor", "Editor", "AutoRun.cs")));
            }
            finally
            {
                if (System.IO.Directory.Exists(tempDir))
                    System.IO.Directory.Delete(tempDir, recursive: true);
            }
        }

        [Test]
        public void Remediation_UH04_UH06_UH08_UH09_NetAuthProtocolAndContentSyncHardening()
        {
            // U-H06: wss:// normalization, non-JSON 502 error handling, token redaction
            Assert.AreEqual("https://play.conclav.io", MushAuthClient.NormalizeBaseUrl("wss://play.conclav.io"));
            Assert.AreEqual("http://127.0.0.1:8080", MushAuthClient.NormalizeBaseUrl("ws://127.0.0.1:8080"));
            string htmlErr = MushAuthClient.ExtractErrorMessageFromResponse(502, "<html><body>502 Bad Gateway</body></html>", "HTTP 502");
            StringAssert.Contains("502", htmlErr);
            string redacted = MushAuthClient.RedactSensitiveTokens("Failed with Bearer rustmush.v1.SuperSecret999 token");
            StringAssert.DoesNotContain("SuperSecret999", redacted);
            StringAssert.Contains("[REDACTED]", redacted);

            // U-H04: Strict HTTP 401 detection (no port :4010 false positive)
            Assert.IsFalse(MushWsClient.IsAuthRejectionException(new System.Exception("Connection refused to 127.0.0.1:4010")));
            Assert.IsTrue(MushWsClient.IsAuthRejectionException(new System.Exception("The server returned status code '401' when status code '101' was expected.")));

            // U-H08: ANSI/idx256 color support (legacy uGUI-safe tags only — never literal <u>), RichText string hex injection block, base_sequence=0 & stale snapshot <= & schema_version
            var rich = JsonNode.Parse(@"{
                ""spans"": [
                    { ""text"": ""Red"", ""style"": { ""fg"": { ""ansi"": 1 }, ""underline"": true } },
                    { ""text"": ""256"", ""style"": { ""fg"": { ""idx256"": 196 } } },
                    { ""text"": ""Inject7"", ""style"": { ""fg"": ""#a><b>x"" } },
                    { ""text"": ""Inject9"", ""style"": { ""fg"": ""#a><b>xxx"" } }
                ]
            }");
            string markup = MushProtocol.RenderRichTextToUnityMarkup(rich);
            StringAssert.Contains("<color=#CC4444><i>Red</i></color>", markup);
            StringAssert.DoesNotContain("<u>", markup, "Legacy UnityEngine.UI.Text does not support <u> tags");
            StringAssert.Contains("<color=#FF0000>256</color>", markup);
            StringAssert.DoesNotContain("<b>x", markup, "7-char malformed string color must never inject RichText tags");
            StringAssert.DoesNotContain("<b>xxx", markup, "9-char malformed string color must never inject RichText tags");
            StringAssert.Contains("Inject7", markup);
            StringAssert.Contains("Inject9", markup);

            // Dynamic corridor port resolution for action:start_docking
            Assert.AreEqual("stickney", TacSimGameCoordinator.ResolveDockingTargetPort("stickney_orbit", null, null, "neighborhood.ceres_freeport", "luna"));
            Assert.AreEqual("luna", TacSimGameCoordinator.ResolveDockingTargetPort(null, "aitken_highport", null, "neighborhood.ceres_freeport", "ceres"));
            Assert.AreEqual("deimos", TacSimGameCoordinator.ResolveDockingTargetPort(null, null, null, "neighborhood.voltaire_station", null));
            Assert.AreEqual("titan", TacSimGameCoordinator.ResolveDockingTargetPort(null, null, null, "neighborhood.ooc_area", "titan"));
            Assert.AreEqual("ceres", TacSimGameCoordinator.ResolveDockingTargetPort(null, null, null, "neighborhood.ooc_area", null));

            var cursor = new ViewStreamCursor();
            Assert.AreEqual(ViewCursorDisposition.ApplySnapshot,
                MushProtocol.EvaluateViewCursor(cursor, JsonNode.Parse(@"{""view"":""v"",""generation"":""g1"",""sequence"":5,""schema_version"":1,""data"":{}}")));
            Assert.AreEqual(ViewCursorDisposition.IgnoreDuplicateOrStale,
                MushProtocol.EvaluateViewCursor(cursor, JsonNode.Parse(@"{""view"":""v"",""generation"":""g1"",""sequence"":5,""schema_version"":1,""data"":{}}")),
                "Duplicate snapshot sequence (==) must be ignored");
            Assert.AreEqual(ViewCursorDisposition.NeedsResubscribe,
                MushProtocol.EvaluateViewCursor(cursor, JsonNode.Parse(@"{""view"":""v"",""generation"":""g1"",""base_sequence"":0,""sequence"":6,""data"":{}}")),
                "base_sequence: 0 must not bypass contiguity check when cursor.LastSequence == 5");
            Assert.AreEqual(ViewCursorDisposition.ApplySnapshot,
                MushProtocol.EvaluateViewCursor(cursor, JsonNode.Parse(@"{""view"":""v"",""generation"":""g1"",""sequence"":5,""schema_version"":2,""data"":{}}")),
                "Incremented schema_version snapshot must be accepted");

            Assert.AreEqual("abcd1234", MushProtocol.NormalizeListingCode("ABCD1234"));
            Assert.IsNull(MushProtocol.NormalizeListingCode("not-hex!"));

            // U-H09: Manifest-pinned SHA256 rejection & offline retry preservation
            try
            {
                string goodJson = @"{""id"":""pinned_pack"",""v"":1}";
                string goodSha = TacSimContentSync.ComputeSha256Hex(goodJson);
                string evilJson = @"{""id"":""pinned_pack"",""v"":999}";
                string evilSha = TacSimContentSync.ComputeSha256Hex(evilJson);

                var sync = new TacSimContentSync();
                using (var ws = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{
                        ""op"":""manifest"",
                        ""packs"":[{{""id"":""pinned_pack"",""category"":""unit"",""sha256"":""{goodSha}"",""bytes"":{goodJson.Length}}}]
                    }}"), nowRealtime: 1f);
                    sync.PumpPackQueue(ws, nowRealtime: 1f);

                    // Reply with tampered JSON and self-consistent evil SHA (or empty SHA) MUST be rejected against manifest pin!
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{
                        ""op"":""pack"",""found"":true,""id"":""pinned_pack"",""sha256"":""{evilSha}"",""json"":{JsonNode.FromString(evilJson).ToJson()}
                    }}"), nowRealtime: 2f);
                    Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("pinned_pack", out _), "Tampered pack must not bypass manifest-pinned SHA256");

                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{
                        ""op"":""pack"",""found"":true,""id"":""pinned_pack"",""sha256"":"""",""json"":{JsonNode.FromString(evilJson).ToJson()}
                    }}"), nowRealtime: 3f);
                    Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("pinned_pack", out _), "Empty reply SHA256 must never skip verification");

                    // Disconnecting must pause retries instead of draining attempts offline
                    ws.Disconnect();
                    int beforeOffline = ws.OutboundQueueCount;
                    for (int step = 1; step <= 10; step++)
                        sync.PumpPackQueue(ws, nowRealtime: 10f + step * 10f);
                    Assert.AreEqual(beforeOffline, ws.OutboundQueueCount, "Offline PumpPackQueue must not drain retries");
                }
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
            }
        }

        [Test]
        public void Remediation_UH12_UH13_UH14_UH20_UH21_UH22_UH23_WorldSpaceDockingSurfaceFixes()
        {
            var rootGo = new UnityEngine.GameObject("TestSpaceDockSurface");
            try
            {
                // U-H12: Trade state berth.docked_hulls, stale quote exclusion, full-hold == 0
                var trade = new TacSimTradeAndShipState();
                var berthFeed = JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""open_quotes"": [{""quote_id"": 10, ""listing_id"": 4, ""quantity"": 5, ""total_minor"": 50000}],
                    ""berth"": {
                        ""docked_hulls"": [{""id"":""#301"",""name"":""Copper Lantern"",""mark"":""FPC-D-0411"",""unit"":""dray""}]
                    },
                    ""ship"": { ""stowage"": { ""cargo_kg"": 100000, ""cargo_limit_kg"": 100000, ""cargo_l"": 120000, ""cargo_limit_l"": 120000, ""tank_kg"": 80000, ""tank_limit_kg"": 80000, ""stores_days"": 120, ""stores_limit_days"": 120 } }
                }");
                trade.ApplyPortTradeFeed(berthFeed);
                Assert.IsTrue(trade.HasAuthoritativeBerthFeed);
                Assert.AreEqual(1, trade.BerthDockedHulls.Count);
                Assert.AreEqual("Copper Lantern", trade.BerthDockedHulls[0].Name);
                Assert.AreEqual(0, trade.ComputeMaxAdditionalUnitsForHold("volatiles"), "Full hold must return 0 max buy units");

                // Pre-existing open quote #10 must NOT auto-accept for a newly started BeginOneClickBuy!
                trade.BeginOneClickBuy(4UL, 5, currentTimeS: 20f, displayedUnitPriceMinor: 10000);
                Assert.IsFalse(trade.TryResolvePendingQuoteFromFeed(berthFeed, 21f, out string staleAccept, out _));
                Assert.IsNull(staleAccept);

                // U-H12: 0 docked hulls hides all placeholder cradles
                var stationGo = new UnityEngine.GameObject("BerthStation");
                stationGo.transform.SetParent(rootGo.transform, false);
                var station = stationGo.AddComponent<StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");
                station.ApplyBerthDockedHulls(new List<TacSimDockedHullRef>());
                var allStationTr = stationGo.GetComponentsInChildren<UnityEngine.Transform>(true);
                int activeCradleShips = 0;
                for (int i = 0; i < allStationTr.Length; i++)
                {
                    if (allStationTr[i].name.StartsWith("MooredShip_") && allStationTr[i].gameObject.activeSelf)
                        activeCradleShips++;
                }
                Assert.AreEqual(0, activeCradleShips, "0 docked hulls must hide all placeholder exterior cradles");

                // U-H13 & U-H20 & U-H21: SpaceFlightController KeyboardSuspended, refused guard, torpedoes, roll sign, pooling
                var spaceGo = new UnityEngine.GameObject("SpaceTest");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<SpaceFlightController>();
                space.BuildSpaceStage(null);
                space.StartDockingFromFeed(JsonNode.Parse(@"{
                    ""session_id"": ""dock.99"",
                    ""dx_mm"": 10, ""dy_mm"": 10, ""dz_mm"": 100,
                    ""dampeners"": false,
                    ""max_frames"": 600,
                    ""start_pose"": { ""dx_mm"": 750, ""dy_mm"": -300, ""dz_mm"": 2800 }
                }"));
                // U-H22: FromFeed seeds from start_pose with DampenersOn=true and respects max_frames=600
                Assert.IsNotNull(space.ActiveDockingSession);
                Assert.AreEqual(750L, space.ActiveDockingSession.DxMm, "FromFeed must seed from start_pose");
                Assert.IsTrue(space.ActiveDockingSession.DampenersOn, "FromFeed with start_pose must start DampenersOn=true matching replay_docking_tape");
                Assert.AreEqual(600, space.ActiveDockingSession.EffectiveMaxFrames);

                // U-H13: KeyboardSuspended blocks Backspace abort & hotkeys
                space.KeyboardSuspended = true;
                Assert.IsFalse(space.ProcessDockingHotkeys(keyXDown: true, keyGDown: true, keyBackspaceDown: true));
                Assert.IsFalse(space.HasPendingDockingAbort);
                Assert.AreEqual(default(DockingControlInput), space.SamplePilotDockingInput());
                space.KeyboardSuspended = false;
                Assert.IsTrue(space.ProcessDockingHotkeys(keyXDown: false, keyGDown: false, keyBackspaceDown: true));
                Assert.IsTrue(space.HasPendingDockingAbort);

                // U-H20: Refused cockpit ignored; holotank torpedoes rendered; contact Transform reused across updates
                space.ApplyCockpitFeed(JsonNode.Parse(@"{""yaw_deg"":45,""pitch_deg"":10,""roll_deg"":30,""speed_mps"":10}"));
                space.ApplyCockpitFeed(JsonNode.Parse(@"{""refused"":true,""yaw_deg"":180}"));
                Assert.AreEqual(45f, space.ShipYawDeg, 0.01f, "Refused cockpit frame must not overwrite ship state");
                space.ApplyCockpitFeed(JsonNode.Parse(@"{""yaw_deg"":45,""pitch_deg"":10,""roll_deg"":30,""speed_mps"":35}"));
                Assert.Greater(space.ThrottlePermille, 0f, "Speed delta without thrust_permille must activate drive plume");

                // U-H21: Positive roll_deg rolls +Z positive (matching bubble.rs starboard UP)
                var posRoll = SpaceFlightController.ComputePlayerShipRotation(0f, 0f, 30f);
                var negRoll = SpaceFlightController.ComputePlayerShipRotation(0f, 0f, -30f);
                Assert.Greater(posRoll.z, 0f);
                Assert.Less(negRoll.z, 0f);

                var holoWithTorp = JsonNode.Parse(@"{
                    ""contacts"": [
                        {""id"":""ship.1"",""you"":true,""x_km"":0,""y_km"":0,""z_km"":0},
                        {""id"":""raider.1"",""label"":""Corsair"",""kind"":""ship"",""relation"":""hostile"",""you"":false,""x_km"":20,""y_km"":5,""z_km"":0,""vx_kms"":-0.2,""vy_kms"":0.1,""vz_kms"":0}
                    ],
                    ""torpedoes"": [{""id"":""t9"",""x_km"":10,""y_km"":2,""z_km"":0,""target_x_km"":0,""target_y_km"":0,""target_z_km"":0,""tti_s"":14}]
                }");
                space.ApplyHolotankFeed(holoWithTorp);
                Assert.AreEqual(1, space.LiveTorpedoCount);
                var contactsContainer = spaceGo.transform.Find("OrbitalContacts");
                var contactFirst = contactsContainer.Find("NavalContact_raider.1");
                Assert.IsNotNull(contactFirst);
                space.ApplyHolotankFeed(holoWithTorp);
                Assert.AreSame(contactFirst, contactsContainer.Find("NavalContact_raider.1"), "Pooling must reuse existing contact Transform");

                // U-H13: Player avatar hides visuals when suspended for vehicle stage
                var playerGo = new UnityEngine.GameObject("PlayerVisTest", typeof(UnityEngine.CharacterController));
                playerGo.transform.SetParent(rootGo.transform, false);
                var player = playerGo.AddComponent<TacSim.Character.ThirdPersonPlayerController>();
                var spawner = playerGo.AddComponent<TacSim.Character.CharacterVisualSpawner>();
                player.VisualSpawner = spawner;
                spawner.BuildPlayerVisual(null, PlayerAvatarPreset.StellarTrooperFemale);
                player.SetStagePhysicsSuspended(true);
                Assert.IsTrue(player.AreVisualsHidden);
                player.SetStagePhysicsSuspended(false);
                Assert.IsFalse(player.AreVisualsHidden);

                // U-H14 & U-H23: site.map '=' maps to Ice, landed pad survives null hull mark, TvRec decoded
                var surfGo = new UnityEngine.GameObject("SurfaceTest");
                surfGo.transform.SetParent(rootGo.transform, false);
                var surf = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surf.BuildSurfaceStage(null, "occator_rim");
                var siteFrame1 = JsonNode.Parse(@"{
                    ""width"": 4, ""height"": 4, ""cell_cm"": 1000, ""origin_cell"": [10, 10], ""you"": [2, 2],
                    ""rows"": [[["","", 2], [""="", 2]], [["","", 4]], [["","", 4]], [["","", 4]]],
                    ""site"": { ""site"": ""Occator Rim"", ""addressed"": 1, ""marks"": { ""hull"": [1, 1] }, ""rigs"": [{""slot"":1,""cell"":[12,12],""charge_pct"":80,""hopper"":""2 of 6 t""}] }
                }");
                surf.ApplySiteMapFrame(siteFrame1);
                Assert.AreEqual(TerrainCellClass.Ice, surf.TerrainGrid.GetCellClass(2, 0), "site.map '=' must decode as Ice");
                Assert.IsTrue(surf.IsPadVisible);

                // Second frame: rover moves onto hull cell -> server emits marks.hull = null; carrier pad MUST stay visible!
                var siteFrame2 = JsonNode.Parse(@"{
                    ""width"": 4, ""height"": 4, ""cell_cm"": 1000, ""origin_cell"": [10, 10], ""you"": [1, 1],
                    ""rows"": [[["","", 2], [""="", 2]], [["","", 4]], [["","", 4]], [["","", 4]]],
                    ""site"": { ""site"": ""Occator Rim"", ""addressed"": 1, ""marks"": { ""hull"": null }, ""rigs"": [{""slot"":1,""cell"":[11,11],""charge_pct"":78,""hopper"":""2 of 6 t""}] }
                }");
                surf.ApplySiteMapFrame(siteFrame2);
                Assert.IsTrue(surf.IsPadVisible, "Landed carrier pad must remain visible when rover covers 'H' cell");

                // U-H23: TvRec tv.status decoding (`pose`, `state: "held"`)
                surf.ApplyTvStatusFrame(JsonNode.Parse(@"{
                    ""rig"": { ""slot"": 1, ""site"": ""occator_rim"", ""state"": ""held"", ""pose"": { ""x_cm"": 12500, ""y_cm"": 13500, ""heading_mdeg"": 90000 } }
                }"));
                Assert.IsTrue(surf.RigOnGround);
                Assert.IsTrue(surf.IsRigHeld, "TvRec state=='held' must flag IsRigHeld");
                Assert.AreEqual(12, surf.AuthoritativeSiteCellX);
                Assert.AreEqual(13, surf.AuthoritativeSiteCellY);

                // U-H12: Coordinator EnsureSubsystems() idempotency, R1..R6 1-based slot index, and empty slots clear
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                coord.Audio = rootGo.AddComponent<TacSimAudioSynthesizer>();
                coord.StationWorld = station;
                var shipGo2 = new UnityEngine.GameObject("IdemShip");
                shipGo2.transform.SetParent(rootGo.transform, false);
                coord.ShipInterior = shipGo2.AddComponent<ShipInteriorController>();
                coord.SpaceStage = space;
                coord.SurfaceStage = surf;
                var arenaGo2 = new UnityEngine.GameObject("IdemArena");
                arenaGo2.transform.SetParent(rootGo.transform, false);
                coord.ArenaStage = arenaGo2.AddComponent<TacSim.Arena.OocArenaController>();
                var camGo2 = new UnityEngine.GameObject("IdemCam", typeof(UnityEngine.Camera));
                camGo2.transform.SetParent(rootGo.transform, false);
                coord.CameraRig = camGo2.AddComponent<TacSim.Character.TacSimCameraRig>();
                coord.CameraRig.ControlledCamera = camGo2.GetComponent<UnityEngine.Camera>();
                coord.Player = player;
                var hudGo2 = new UnityEngine.GameObject("IdemHud");
                hudGo2.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo2.AddComponent<TacSim.UI.TacSimHudController>();

                coord.EnsureSubsystems();
                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var wsField = typeof(TacSimGameCoordinator).GetField("_ws", flags);
                var ws1 = wsField.GetValue(coord);
                coord.EnsureSubsystems();
                Assert.AreSame(ws1, wsField.GetValue(coord), "Re-calling EnsureSubsystems() must not replace/leak _ws");

                var applySimAmbient = typeof(TacSimGameCoordinator).GetMethod("ApplySimulatorRoomAmbient", flags);
                var applySimTrade = typeof(TacSimGameCoordinator).GetMethod("ApplySimulatorTradeRoster", flags);
                var amb = new TacSimRoomAmbient
                {
                    Mode = "simulator",
                    SimMatchState = "boarding",
                    SimSlots = new List<TacSimSimulatorSlot> { new TacSimSimulatorSlot { Team = "red", Index = 1, PilotName = "Vance", Unit = "line_tank", Seated = true } }
                };
                applySimAmbient.Invoke(coord, new object[] { amb });
                StringAssert.Contains("R1", coord.ArenaStage.LiveLobbySlotSummary, "1-based server slot index 1 must render R1, never R2");
                StringAssert.DoesNotContain("R2", coord.ArenaStage.LiveLobbySlotSummary);

                applySimTrade.Invoke(coord, new object[] { JsonNode.Parse(@"{""match_state"":""idle"",""slots"":[]}") });
                Assert.AreEqual(string.Empty, coord.ArenaStage.LiveLobbySlotSummary, "Empty port.trade.simulator.slots must clear stale lobby summary");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void PR693_Net_Auth401UnitySafe_CancelBeforeConfirm_AndStubInputFieldContract()
        {
            // Item 1: Unity 6000 (.NET Standard 2.1) safe 401 auth rejection detection
            Assert.IsFalse(MushWsClient.IsAuthRejectionException(new System.Net.Http.HttpRequestException("Connection refused to 127.0.0.1:4010")));
            Assert.IsTrue(MushWsClient.IsAuthRejectionException(new System.Net.Http.HttpRequestException("Response status code does not indicate success: 401 (Unauthorized).")));
            Assert.IsTrue(MushWsClient.IsAuthRejectionException(new System.Exception("HTTP/1.1 401 Unauthorized")));
            Assert.IsTrue(MushWsClient.IsAuthRejectionException(new System.Exception("Server closed with close code 4401")));
            Assert.IsFalse(MushWsClient.IsAuthRejectionException(new System.Exception("HTTP/1.1 503 Service Unavailable")));

            // Item 2: InputField.isFocused must be read-only (matching real Unity 6000 CS0200),
            // and InputField.text must strip embedded NULs & fire onValueChanged.
            var isFocusedProp = typeof(UnityEngine.UI.InputField).GetProperty("isFocused");
            Assert.IsNotNull(isFocusedProp);
            Assert.IsNull(isFocusedProp.GetSetMethod(nonPublic: false), "InputField.isFocused must not expose a public setter");
            var camMainProp = typeof(UnityEngine.Camera).GetProperty("main");
            Assert.IsNotNull(camMainProp);
            Assert.IsNull(camMainProp.GetSetMethod(nonPublic: false), "Camera.main must be get-only matching real Unity");
            Assert.IsNotNull(typeof(UnityEngine.UI.InputField).GetMethod("ActivateInputField"));
            Assert.IsNotNull(typeof(UnityEngine.UI.InputField).GetMethod("DeactivateInputField"));
            Assert.IsNotNull(typeof(UnityEngine.UI.InputField).GetMethod("SetTextWithoutNotify"));

            var inputGo = new UnityEngine.GameObject("StubInputFieldContract");
            try
            {
                var field = inputGo.AddComponent<UnityEngine.UI.InputField>();
                var observed = new List<string>();
                field.onValueChanged.AddListener(v => observed.Add(v));
                field.text = "hello\0world";
                Assert.AreEqual("helloworld", field.text, "InputField.text must strip embedded NUL characters");
                Assert.AreEqual(1, observed.Count);
                Assert.AreEqual("helloworld", observed[0]);

                field.SetTextWithoutNotify("silent\0update");
                Assert.AreEqual("silentupdate", field.text);
                Assert.AreEqual(1, observed.Count, "SetTextWithoutNotify must not invoke onValueChanged");

                field.ActivateInputField();
                field.DeactivateInputField();
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(inputGo);
            }

            // Item 7: Closing/cancelling a confirm prompt MUST encode Cancel (abstention), NOT Confirm(false) (disagreement)
            var cancelConfirm = JsonNode.Parse(MushProtocol.BuildChoiceResponse(42UL, "__cancel__", "ep-9", "confirm"));
            Assert.AreEqual("respond", cancelConfirm.Str("t"));
            Assert.AreEqual(42L, cancelConfirm["d"].Long("id"));
            Assert.AreEqual("ep-9", cancelConfirm["d"].Str("epoch"));
            Assert.AreEqual("cancel", cancelConfirm["d"]["choice"].Str("t"), "Closing a confirm prompt must emit RespondChoice::Cancel");
            Assert.IsFalse(cancelConfirm["d"]["choice"].ContainsKey("d"), "Cancel variant must carry no boolean payload");

            var confirmTrue = JsonNode.Parse(MushProtocol.BuildChoiceResponse(42UL, "true", "ep-9", "confirm"));
            Assert.AreEqual("confirm", confirmTrue["d"]["choice"].Str("t"));
            Assert.IsTrue(confirmTrue["d"]["choice"].Bool("d", false));

            var confirmFalse = JsonNode.Parse(MushProtocol.BuildChoiceResponse(42UL, "false", "ep-9", "confirm"));
            Assert.AreEqual("confirm", confirmFalse["d"]["choice"].Str("t"));
            Assert.IsFalse(confirmFalse["d"]["choice"].Bool("d", true));

            var parsedConfirm = MushProtocol.ParseRequest(JsonNode.Parse(@"{""epoch"":""ep-9"",""id"":42,""kind"":""confirm"",""prompt"":""Proceed?""}"));
            Assert.AreEqual(2, parsedConfirm.Options.Count);
            Assert.AreEqual("true", parsedConfirm.Options[0].Key);
            Assert.AreEqual("Confirm", parsedConfirm.Options[0].Label);
            Assert.AreEqual("false", parsedConfirm.Options[1].Key);
            Assert.AreEqual("Decline", parsedConfirm.Options[1].Label);
        }

        [Test]
        public void PR693_ContentSync_StaffManifestDoesNotLeakToRegularCharacterSession()
        {
            string tempDir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimStaffLeakTest_" + System.Guid.NewGuid().ToString("N"));
            try
            {
                string staffWingJson = @"{""id"":""neighborhood.staff_wing"",""entries"":{""room.staff_secret"":{""kind"":""room""}}}";
                string staffWingSha = TacSimContentSync.ComputeSha256Hex(staffWingJson);
                string staffFreeportJson = @"{""id"":""neighborhood.ceres_freeport"",""projection"":""staff""}";
                string staffFreeportSha = TacSimContentSync.ComputeSha256Hex(staffFreeportJson);
                string playerFreeportJson = @"{""id"":""neighborhood.ceres_freeport"",""projection"":""player""}";
                string playerFreeportSha = TacSimContentSync.ComputeSha256Hex(playerFreeportJson);
                string sharedGoodsJson = @"{""goods"":[{""id"":""volatiles"",""label"":""Water Ice""}]}";
                string sharedGoodsSha = TacSimContentSync.ComputeSha256Hex(sharedGoodsJson);

                // Session 1: Staff character logs into dev1.conclav.io and caches staff-only + staff-projected packs on disk.
                var staffSync = new TacSimContentSync();
                staffSync.InitializeDiskCacheForServer("https://dev1.conclav.io", tempDir);
                using (var wsStaff = new MushWsClient())
                {
                    var staffManifest = JsonNode.Parse($@"{{
                        ""op"":""manifest"",
                        ""packs"":[
                            {{""id"":""neighborhood.staff_wing"",""category"":""atlas"",""sha256"":""{staffWingSha}"",""bytes"":{staffWingJson.Length}}},
                            {{""id"":""neighborhood.ceres_freeport"",""category"":""atlas"",""sha256"":""{staffFreeportSha}"",""bytes"":{staffFreeportJson.Length}}},
                            {{""id"":""goods"",""category"":""economy"",""sha256"":""{sharedGoodsSha}"",""bytes"":{sharedGoodsJson.Length}}}
                        ]
                    }}");
                    staffSync.HandleWebSocketContentPanel(wsStaff, staffManifest, nowRealtime: 1f);
                    staffSync.HandleWebSocketContentPanel(wsStaff, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""neighborhood.staff_wing"",""category"":""atlas"",""sha256"":""{staffWingSha}"",""json"":{JsonNode.FromString(staffWingJson).ToJson()}}}"), 1.1f);
                    staffSync.HandleWebSocketContentPanel(wsStaff, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""neighborhood.ceres_freeport"",""category"":""atlas"",""sha256"":""{staffFreeportSha}"",""json"":{JsonNode.FromString(staffFreeportJson).ToJson()}}}"), 1.2f);
                    staffSync.HandleWebSocketContentPanel(wsStaff, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""goods"",""category"":""economy"",""sha256"":""{sharedGoodsSha}"",""json"":{JsonNode.FromString(sharedGoodsJson).ToJson()}}}"), 1.3f);
                }
                Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("neighborhood.staff_wing", out _));
                Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("neighborhood.ceres_freeport", out string staffLoaded));
                Assert.AreEqual(staffFreeportJson, staffLoaded);

                // Session 2: Regular non-staff character logs into the SAME server & cache directory.
                var playerSync = new TacSimContentSync();
                playerSync.InitializeDiskCacheForServer("https://dev1.conclav.io", tempDir);

                // Before current session's live manifest arrives, ZERO cached packs from the staff session may be registered!
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("neighborhood.staff_wing", out _), "Staff-only pack must not be pre-registered from disk cache");
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("neighborhood.ceres_freeport", out _), "Staff-projected neighborhood must not be pre-registered before live manifest");
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Cached goods pack must stay private until live manifest authorizes its digest");

                // Current regular character's live manifest authorizes ONLY `goods` (same digest) and `neighborhood.ceres_freeport` (player digest).
                using (var wsPlayer = new MushWsClient())
                {
                    var playerManifest = JsonNode.Parse($@"{{
                        ""op"":""manifest"",
                        ""packs"":[
                            {{""id"":""neighborhood.ceres_freeport"",""category"":""atlas"",""sha256"":""{playerFreeportSha}"",""bytes"":{playerFreeportJson.Length}}},
                            {{""id"":""goods"",""category"":""economy"",""sha256"":""{sharedGoodsSha}"",""bytes"":{sharedGoodsJson.Length}}}
                        ]
                    }}");
                    playerSync.HandleWebSocketContentPanel(wsPlayer, playerManifest, nowRealtime: 10f);

                    // `goods` matched both ID and digest -> authorized immediately from warm disk cache with zero fetch!
                    Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out string authorizedGoods));
                    Assert.AreEqual(sharedGoodsJson, authorizedGoods);

                    // `neighborhood.staff_wing` was omitted -> NEVER registered!
                    Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("neighborhood.staff_wing", out _), "Staff-only pack must stay excluded after regular manifest");

                    // `neighborhood.ceres_freeport` had a staff digest mismatch -> staff version NOT registered; exactly 1 fetch queued for player version!
                    Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("neighborhood.ceres_freeport", out _), "Mismatched staff projection must not be registered while player projection downloads");
                    Assert.AreEqual(1, wsPlayer.OutboundQueueCount, "Only the mismatched pack should be fetched over WebSocket");

                    // Deliver player-projected `neighborhood.ceres_freeport` -> registered cleanly!
                    playerSync.HandleWebSocketContentPanel(wsPlayer, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""neighborhood.ceres_freeport"",""category"":""atlas"",""sha256"":""{playerFreeportSha}"",""json"":{JsonNode.FromString(playerFreeportJson).ToJson()}}}"), 10.1f);
                    Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("neighborhood.ceres_freeport", out string loadedPlayer));
                    Assert.AreEqual(playerFreeportJson, loadedPlayer);
                }
            }
            finally
            {
                TacSimContentLoader.ClearDynamicPacks();
                try { if (System.IO.Directory.Exists(tempDir)) System.IO.Directory.Delete(tempDir, recursive: true); } catch { }
            }
        }

        [Test]
        public void Security_U7_LoopbackChecks_NonLoopbackWsGuard_AndNonJsonAuthErrorHandling()
        {
            // 0.0.0.0 must NOT be classified as loopback.
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority("0.0.0.0"));
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority("0.0.0.0:8080"));
            Assert.AreEqual("https://0.0.0.0:8080", MushAuthClient.NormalizeBaseUrl("0.0.0.0:8080"));
            Assert.IsTrue(MushAuthClient.IsLoopbackAuthority("localhost:8080"));
            Assert.IsTrue(MushAuthClient.IsLoopbackAuthority("127.0.0.1:8080"));
            Assert.IsTrue(MushAuthClient.IsLoopbackAuthority("[::1]:8080"));

            // Non-loopback ws:// must be rejected in non-dev mode; wss:// and loopback ws:// remain allowed.
            Assert.Throws<System.InvalidOperationException>(() =>
                MushWsClient.BuildValidatedWebSocketUri("http://dev1.conclav.io", "42", 0, allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushWsClient.BuildValidatedWebSocketUri("http://0.0.0.0:8080", "42", 0, allowInsecureNonLoopback: false));

            var wss = MushWsClient.BuildValidatedWebSocketUri("https://dev1.conclav.io", "42", 0, allowInsecureNonLoopback: false);
            Assert.AreEqual("wss", wss.Scheme);
            var loopback = MushWsClient.BuildValidatedWebSocketUri("http://127.0.0.1:8080", "42", 0, allowInsecureNonLoopback: false);
            Assert.AreEqual("ws", loopback.Scheme);
            var v6Loopback = MushWsClient.BuildValidatedWebSocketUri("http://[::1]:8080", "42", 0, allowInsecureNonLoopback: false);
            Assert.AreEqual("ws", v6Loopback.Scheme);

            // Non-JSON non-2xx auth error responses (e.g. HTML 502 from reverse proxy) must not throw FormatException.
            Assert.AreEqual("<html>502 Bad Gateway</html>", MushAuthClient.ExtractErrorMessage("<html>502 Bad Gateway</html>", "HTTP 502"));
            Assert.AreEqual("invalid credentials", MushAuthClient.ExtractErrorMessage(@"{""error"":""invalid credentials""}", "HTTP 401"));
            Assert.AreEqual("HTTP 500", MushAuthClient.ExtractErrorMessage("", "HTTP 500"));
        }

        [Test]
        public void PR693_Coordinator_UnseatedPassengerDoesNotEnterPilotDockingUi()
        {
            var rootGo = new UnityEngine.GameObject("PR693DockingPassengerTest");
            try
            {
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                var spaceGo = new UnityEngine.GameObject("Space");
                spaceGo.transform.SetParent(rootGo.transform, false);
                coord.SpaceStage = spaceGo.AddComponent<SpaceFlightController>();
                coord.SpaceStage.BuildSpaceStage(null);

                var stationGo = new UnityEngine.GameObject("Station");
                stationGo.transform.SetParent(rootGo.transform, false);
                coord.StationWorld = stationGo.AddComponent<StationWorldController>();

                var shipGo = new UnityEngine.GameObject("Ship");
                shipGo.transform.SetParent(rootGo.transform, false);
                coord.ShipInterior = shipGo.AddComponent<ShipInteriorController>();

                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                coord.CameraRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                coord.CameraRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();
                coord.CameraRig.SetMode(TacSim.Character.CameraRigMode.ThirdPersonWalk);

                var playerGo = new UnityEngine.GameObject("Player", typeof(UnityEngine.CharacterController));
                playerGo.transform.SetParent(rootGo.transform, false);
                coord.Player = playerGo.AddComponent<TacSim.Character.ThirdPersonPlayerController>();
                coord.Player.IsSeatedAtHelm = false;

                var audioGo = new UnityEngine.GameObject("Audio");
                audioGo.transform.SetParent(rootGo.transform, false);
                coord.Audio = audioGo.AddComponent<TacSimAudioSynthesizer>();

                var hudGo = new UnityEngine.GameObject("Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();
                coord.Hud.BuildHud();
                coord.Hud.ShowLoginOverlay(false);

                var stageDirector = new TacSimStageDirector(
                    coord.StationWorld,
                    coord.ShipInterior,
                    coord.SpaceStage,
                    null,
                    null,
                    coord.CameraRig,
                    coord.Player,
                    coord.Audio,
                    coord.Hud);

                var dockingFrame = JsonNode.Parse(@"{
                    ""session_id"": ""dock.passenger.1"",
                    ""dx_mm"": 200, ""dy_mm"": -100, ""dz_mm"": 1500,
                    ""max_frames"": 600,
                    ""start_pose"": { ""dx_mm"": 200, ""dy_mm"": -100, ""dz_mm"": 1500 }
                }");

                // 1. Unseated passenger aboard ship receives broadcast `space.docking`:
                // must NOT enter docking session, bow camera, or walk suspension!
                coord.HandleDockingPanelFrame(dockingFrame);
                stageDirector.SyncFrame(isAboardShip: true, isTextInputFocused: false);

                Assert.IsFalse(coord.SpaceStage.IsDockingActive, "Unseated passenger must not activate pilot docking session");
                Assert.AreEqual(TacSim.Character.CameraRigMode.ThirdPersonWalk, coord.CameraRig.Mode, "Unseated passenger must stay in ThirdPersonWalk camera");
                Assert.IsFalse(coord.Player.InputSuspended, "Unseated passenger walking must not be suspended by another pilot's docking broadcast");
                Assert.IsTrue(coord.CameraRig.OrbitInputEnabled, "Unseated passenger orbit camera must stay enabled");

                // 2. Seated helm pilot receives `space.docking`: enters BowDockingCollar & docking session!
                coord.Player.IsSeatedAtHelm = true;
                coord.HandleDockingPanelFrame(dockingFrame);
                stageDirector.SyncFrame(isAboardShip: true, isTextInputFocused: false);

                Assert.IsTrue(coord.SpaceStage.IsDockingActive, "Seated helm pilot must activate docking session");
                Assert.AreEqual(TacSim.Character.CameraRigMode.BowDockingCollar, coord.CameraRig.Mode, "Seated helm pilot must switch to BowDockingCollar");
                Assert.IsTrue(coord.Player.InputSuspended, "Seated helm pilot walking must be suspended during docking");
                Assert.IsFalse(coord.CameraRig.OrbitInputEnabled);

                // 3. Defense-in-depth: if player unseats while a docking session still exists, stage director does not suspend walking.
                coord.Player.IsSeatedAtHelm = false;
                stageDirector.SyncFrame(isAboardShip: true, isTextInputFocused: false);
                Assert.IsFalse(coord.Player.InputSuspended, "Unseated player must never have walking suspended by SpaceStage.IsDockingActive");

                // 4. Terminal docking frame always closes an active docking session cleanly.
                coord.HandleDockingPanelFrame(JsonNode.Parse(@"{""refused"":true,""closed"":true,""outcome"":""clamped""}"));
                Assert.IsFalse(coord.SpaceStage.IsDockingActive, "Terminal closed frame must close active docking session");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void PR693_WorldBillboard_CameraFacingOffsetNeverCrossesRoomWallBounds()
        {
            var disabledPriorMainCams = new List<UnityEngine.Camera>();
            var rootGo = new UnityEngine.GameObject("PR693BillboardBoundsTest");
            try
            {
                // Disable any pre-existing MainCamera in the scene so our tagged test camera resolves as Camera.main
                // (Camera.main is get-only in real Unity 6000.6.7f1).
                while (UnityEngine.Camera.main != null)
                {
                    var existing = UnityEngine.Camera.main;
                    existing.enabled = false;
                    disabledPriorMainCams.Add(existing);
                }

                var camGo = new UnityEngine.GameObject("OutsideCam", typeof(UnityEngine.Camera));
                camGo.tag = "MainCamera";
                camGo.transform.SetParent(rootGo.transform, false);
                var testCam = camGo.GetComponent<UnityEngine.Camera>();
                Assert.AreSame(testCam, UnityEngine.Camera.main, "Active MainCamera-tagged test camera must resolve as Camera.main");

                var actorGo = new UnityEngine.GameObject("CornerActor");
                actorGo.transform.SetParent(rootGo.transform, false);
                var bbGo = new UnityEngine.GameObject("Billboard");
                bbGo.transform.SetParent(actorGo.transform, false);
                var bb = bbGo.AddComponent<WorldBillboard>();
                bb.SetRoomSafeBounds(-10f, 10f, -10f, 10f, wallMarginM: 1.5f);

                var lateUpdate = typeof(WorldBillboard).GetMethod("LateUpdate", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
                Assert.IsNotNull(lateUpdate);

                // Reproduce exact native Unity failure: actor at SE corner (9.8, -9.8) with Camera.main further SE!
                actorGo.transform.position = new UnityEngine.Vector3(9.8f, 0f, -9.8f);
                camGo.transform.position = new UnityEngine.Vector3(12f, 2f, -20f);
                lateUpdate.Invoke(bb, null);
                Assert.LessOrEqual(bbGo.transform.position.x, 8.5001f, "Camera-facing offset must not push billboard past east safe bound");
                Assert.GreaterOrEqual(bbGo.transform.position.z, -8.5001f, "Camera-facing offset must not push billboard past south safe bound");

                // Opposite NW corner with camera outside NW wall
                actorGo.transform.position = new UnityEngine.Vector3(-9.8f, 0f, 9.8f);
                camGo.transform.position = new UnityEngine.Vector3(-20f, 2f, 20f);
                lateUpdate.Invoke(bb, null);
                Assert.GreaterOrEqual(bbGo.transform.position.x, -8.5001f, "Camera-facing offset must not push billboard past west safe bound");
                Assert.LessOrEqual(bbGo.transform.position.z, 8.5001f, "Camera-facing offset must not push billboard past north safe bound");

                // Actor in room interior still shifts toward camera within safe bounds
                actorGo.transform.position = new UnityEngine.Vector3(0f, 0f, 0f);
                camGo.transform.position = new UnityEngine.Vector3(0f, 2f, -5f);
                lateUpdate.Invoke(bb, null);
                Assert.Less(bbGo.transform.position.z, -0.20f, "Interior billboard should still apply camera-facing offset");
                Assert.GreaterOrEqual(bbGo.transform.position.z, -8.5001f);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
                for (int i = 0; i < disabledPriorMainCams.Count; i++)
                {
                    if (disabledPriorMainCams[i] != null)
                        disabledPriorMainCams[i].enabled = true;
                }
            }
        }

        [Test]
        public void PR693_StandaloneBuilder_ValidatesBeforeSwapAndFailsOnIncompleteOrUndeletableRetiredPack()
        {
            string tempOut = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimStagingSafety_" + System.Guid.NewGuid().ToString("N"));
            string emptySrc = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimEmptyContent_" + System.Guid.NewGuid().ToString("N"));
            try
            {
                System.IO.Directory.CreateDirectory(tempOut);
                System.IO.Directory.CreateDirectory(emptySrc);

                // 1. Pre-existing staged files MUST remain untouched when source content is incomplete, AND build must fail!
                string prevGoodPath = System.IO.Path.Combine(tempOut, "previous_good.json");
                System.IO.File.WriteAllText(prevGoodPath, "{\"ok\":true}");

                Assert.Throws<UnityEditor.Build.BuildFailedException>(() =>
                    TacSim.Editor.TacSimStandaloneBuilder.StageEmbeddedFallbackPacksFromRoots(
                        tempOut,
                        tacsimContentRoot: emptySrc,
                        mushAtlasContentRoot: emptySrc,
                        refreshAssetDatabase: false));
                Assert.IsTrue(System.IO.File.Exists(prevGoodPath), "Failed staging must not wipe previously staged packs before validating replacement set");

                // 2. When a retired file CANNOT be deleted, staging MUST throw BuildFailedException instead of silently retaining it!
                string retiredLockedPath = System.IO.Path.Combine(tempOut, "retired_locked.json");
                System.IO.File.WriteAllText(retiredLockedPath, "{\"retired\":true}");

                string tacsimRoot = System.IO.Path.GetDirectoryName(TacSimContentLoader.ResolveRepoContentPath("atlas"));
                string atlasRoot = System.IO.Path.GetDirectoryName(TacSimContentLoader.ResolveMushAtlasContentPath("neighborhood.ooc_area.json"));

                Assert.Throws<UnityEditor.Build.BuildFailedException>(() =>
                    TacSim.Editor.TacSimStandaloneBuilder.StageEmbeddedFallbackPacksFromRoots(
                        tempOut,
                        tacsimContentRoot: tacsimRoot,
                        mushAtlasContentRoot: atlasRoot,
                        refreshAssetDatabase: false,
                        deleteFile: path => throw new System.IO.IOException($"Locked file: {path}")));

                // 3. Clean staging succeeds and removes all retired files (including .meta sidecars) cleanly!
                string retiredMetaPath = retiredLockedPath + ".meta";
                System.IO.File.WriteAllText(retiredMetaPath, "fileFormatVersion: 2");
                int staged = TacSim.Editor.TacSimStandaloneBuilder.StageEmbeddedFallbackPacksFromRoots(
                    tempOut,
                    tacsimContentRoot: tacsimRoot,
                    mushAtlasContentRoot: atlasRoot,
                    refreshAssetDatabase: false);
                Assert.GreaterOrEqual(staged, 15);
                Assert.IsFalse(System.IO.File.Exists(prevGoodPath), "Retired previous_good.json must be removed on clean swap");
                Assert.IsFalse(System.IO.File.Exists(retiredLockedPath), "Retired pack must be removed on clean swap");
                Assert.IsFalse(System.IO.File.Exists(retiredMetaPath), "Retired pack .meta sidecar must be removed on clean swap");
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "goods.json")));
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempOut, "dray.json")));
            }
            finally
            {
                try { if (System.IO.Directory.Exists(tempOut)) System.IO.Directory.Delete(tempOut, recursive: true); } catch { }
                try { if (System.IO.Directory.Exists(emptySrc)) System.IO.Directory.Delete(emptySrc, recursive: true); } catch { }
            }
        }

        [Test]
        public void PR693_WsClient_CoalescesTelemetryPreservesReliableEventsAndGuardsStaleEpochSendFailure()
        {
            // Item 8A: Telemetry flood (3,000 frames) coalesces to O(1) without dropping reliable invoke_result / request events!
            using (var ws = new MushWsClient())
            {
                var invokeIds = new List<ulong>();
                var requestIds = new List<ulong>();
                int latestCockpitYaw = -1;
                int latestHoloTick = -1;

                ws.OnInvokeResultReceived += (id, _, _) => invokeIds.Add(id);
                ws.OnServerRequestReceived += req => requestIds.Add(req.Id);
                ws.OnPanelUpdated += (feed, data) =>
                {
                    if (feed == "space.cockpit") latestCockpitYaw = data.Int("yaw_deg", -1);
                    if (feed == "space.holotank") latestHoloTick = data.Int("tick", -1);
                };

                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":1,""ts"":10,""t"":""invoke_result"",""d"":{""id"":101,""result"":{""service"":""mind"",""result"":{""status"":""saved""}}}}"));
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":2,""ts"":11,""t"":""request"",""d"":{""epoch"":""e1"",""id"":202,""kind"":""confirm"",""prompt"":""Confirm?""}}"));

                for (int i = 0; i < 1500; i++)
                {
                    ulong seqA = (ulong)(3 + i * 2);
                    ulong seqB = (ulong)(4 + i * 2);
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{""seq"":{seqA},""ts"":20,""t"":""panel"",""d"":{{""feed"":""space.cockpit"",""kind"":""keyframe"",""data"":{{""yaw_deg"":{i}}}}}}}"));
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{""seq"":{seqB},""ts"":20,""t"":""panel"",""d"":{{""feed"":""space.holotank"",""kind"":""keyframe"",""data"":{{""tick"":{i}}}}}}}"));
                }

                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":3003,""ts"":30,""t"":""invoke_result"",""d"":{""id"":102,""result"":{""service"":""mind"",""result"":{""status"":""checked""}}}}"));

                Assert.IsFalse(ws.InboundOverflowDetected, "Replaceable telemetry bursts must coalesce instead of overflowing");
                Assert.AreEqual(2, ws.PendingTelemetryInboundCount, "3,000 cockpit/holotank frames must coalesce to 2 latest entries");
                Assert.AreEqual(3, ws.PendingReliableInboundCount, "All 3 reliable events must remain queued");

                ws.PumpMainThread();

                CollectionAssert.AreEqual(new ulong[] { 101UL, 102UL }, invokeIds, "No reliable invoke_result may be dropped under telemetry backpressure");
                CollectionAssert.AreEqual(new ulong[] { 202UL }, requestIds, "No reliable server request may be dropped under telemetry backpressure");
                Assert.AreEqual(1499, latestCockpitYaw, "Latest coalesced space.cockpit keyframe must be delivered");
                Assert.AreEqual(1499, latestHoloTick, "Latest coalesced space.holotank keyframe must be delivered");
                Assert.AreEqual(3003UL, ws.LastSeq);
            }

            // Item 8B: Terminal space.docking (closed: true) must supersede earlier queued open docking frame!
            using (var ws = new MushWsClient())
            {
                bool sawClosedLast = false;
                ws.OnPanelUpdated += (feed, data) =>
                {
                    if (feed == "space.docking")
                        sawClosedLast = data.Bool("closed", false);
                };
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":10,""ts"":1,""t"":""panel"",""d"":{""feed"":""space.docking"",""kind"":""keyframe"",""data"":{""session_id"":""s1""}}}"));
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":11,""ts"":2,""t"":""panel"",""d"":{""feed"":""space.docking"",""kind"":""keyframe"",""data"":{""closed"":true,""refused"":true,""outcome"":""clamped""}}}"));
                ws.PumpMainThread();
                Assert.IsTrue(sawClosedLast, "Terminal closed space.docking frame must not be overwritten by earlier queued open frame");
            }

            // Item 8B-delta: A reliable panel delta invalidation must not discard an earlier queued telemetry keyframe for the same feed
            using (var ws = new MushWsClient())
            {
                int yawSeen = -1;
                ws.OnPanelUpdated += (feed, data) => { if (feed == "space.cockpit") yawSeen = data.Int("yaw_deg", -1); };
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":10,""ts"":1,""t"":""panel"",""d"":{""feed"":""space.cockpit"",""kind"":""keyframe"",""data"":{""yaw_deg"":77}}}"));
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":11,""ts"":2,""t"":""panel"",""d"":{""feed"":""space.cockpit"",""kind"":""delta"",""data"":{}}}"));
                ws.PumpMainThread();
                Assert.AreEqual(77, yawSeen, "Panel delta must not drop earlier queued keyframe for the same feed");
            }

            // Item 8C: Genuine reliable queue overflow (> 2,048 reliable events) MUST NOT evict oldest events or advance _lastSeq past omitted events!
            using (var ws = new MushWsClient())
            {
                var receivedIds = new List<ulong>();
                ws.OnInvokeResultReceived += (id, _, _) => receivedIds.Add(id);

                int totalInjected = MushWsClient.MaxInboundQueueSize + 50;
                for (int i = 1; i <= totalInjected; i++)
                {
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{""seq"":{i},""ts"":50,""t"":""invoke_result"",""d"":{{""id"":{i},""result"":{{""service"":""mind"",""result"":{{""status"":""ok""}}}}}}}}"));
                }

                Assert.IsTrue(ws.InboundOverflowDetected, "Exceeding MaxInboundQueueSize with reliable events must latch overflow for resync");
                Assert.AreEqual(MushWsClient.MaxInboundQueueSize, ws.PendingReliableInboundCount);

                for (int pump = 0; pump < 40 && ws.PendingReliableInboundCount > 0; pump++)
                    ws.PumpMainThread();

                Assert.AreEqual(MushWsClient.MaxInboundQueueSize, receivedIds.Count);
                Assert.AreEqual(1UL, receivedIds[0], "Oldest reliable event (seq=1) must NEVER be silently dropped");
                Assert.AreEqual((ulong)MushWsClient.MaxInboundQueueSize, receivedIds[receivedIds.Count - 1]);
                Assert.AreEqual((ulong)MushWsClient.MaxInboundQueueSize, ws.LastSeq, "_lastSeq must NEVER advance past unqueued events on overflow");
            }

            // Item 9: Failed send on a stale socket epoch MUST NOT clear _isConnected or enqueue a drop for the new epoch!
            using (var ws = new MushWsClient())
            {
                var flags = System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic;
                var epochField = typeof(MushWsClient).GetField("_sessionEpoch", flags);
                var connectedField = typeof(MushWsClient).GetField("_isConnected", flags);
                var dropsField = typeof(MushWsClient).GetField("_socketDrops", flags);
                var failMethod = typeof(MushWsClient).GetMethod(
                    "HandleSocketLoopFailureForEpoch",
                    flags,
                    null,
                    new[] { typeof(long), typeof(int?), typeof(string) },
                    null);
                Assert.IsNotNull(epochField);
                Assert.IsNotNull(connectedField);
                Assert.IsNotNull(dropsField);
                Assert.IsNotNull(failMethod);

                // Active connection is on epoch 2
                epochField.SetValue(ws, 2L);
                connectedField.SetValue(ws, true);

                // Old socket from epoch 1 hits a send exception -> MUST NOT disable epoch 2!
                bool staleHandled = (bool)failMethod.Invoke(ws, new object[] { 1L, null, "WebSocket send failed (aborted)" });
                Assert.IsFalse(staleHandled, "Stale epoch failure must be ignored");
                Assert.IsTrue((bool)connectedField.GetValue(ws), "Stale socket send failure must NOT clear _isConnected for active epoch 2");
                var drops = dropsField.GetValue(ws) as System.Collections.Concurrent.ConcurrentQueue<(long epoch, int? code, string message)>;
                Assert.IsNotNull(drops);
                Assert.AreEqual(0, drops.Count, "Stale socket send failure must NOT enqueue a socket drop");

                // Current socket on epoch 2 fails -> MUST mark disconnected and enqueue drop!
                bool currentHandled = (bool)failMethod.Invoke(ws, new object[] { 2L, null, "WebSocket send failed (broken pipe)" });
                Assert.IsTrue(currentHandled, "Current epoch failure must be handled");
                Assert.IsFalse((bool)connectedField.GetValue(ws), "Current epoch failure must set _isConnected = false");
                Assert.AreEqual(1, drops.Count, "Current epoch failure must enqueue drop for reconnect");
            }
        }

        [Test]
        public void Security_EXT_P1_2_AuthRejectsRemoteHttp_DisablesRedirects_AndProjectSettingsHardened()
        {
            // 1. Remote cleartext http:// (and ws:// -> http://) MUST be rejected before POST in release/non-dev mode.
            Assert.Throws<System.InvalidOperationException>(() =>
                MushAuthClient.BuildValidatedAuthEndpointUri("http://dev1.conclav.io", "/api/auth/login", allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushAuthClient.BuildValidatedAuthEndpointUri("http://0.0.0.0:8080", "/api/auth/login", allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushAuthClient.BuildValidatedAuthEndpointUri("ws://play.conclav.io:8080", "/api/auth/register", allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushAuthClient.BuildValidatedAuthEndpointUri("ftp://play.conclav.io", "/api/auth/login", allowInsecureNonLoopback: true));

            // LoginAsync / RegisterAsync reject remote HTTP immediately before sending credentials in non-dev CI builds.
            Assert.ThrowsAsync<System.InvalidOperationException>(async () =>
                await MushAuthClient.LoginAsync("http://dev1.conclav.io", "pilot", "hunter2"));
            Assert.ThrowsAsync<System.InvalidOperationException>(async () =>
                await MushAuthClient.RegisterAsync("http://dev1.conclav.io", "pilot", "hunter2", "Pilot"));

            // HTTPS and loopback HTTP remain allowed.
            var httpsUri = MushAuthClient.BuildValidatedAuthEndpointUri("https://dev1.conclav.io", "/api/auth/login", allowInsecureNonLoopback: false);
            Assert.AreEqual("https://dev1.conclav.io/api/auth/login", httpsUri.AbsoluteUri);
            var loopV4 = MushAuthClient.BuildValidatedAuthEndpointUri("http://127.0.0.1:8080", "/api/auth/login", allowInsecureNonLoopback: false);
            Assert.AreEqual("http://127.0.0.1:8080/api/auth/login", loopV4.AbsoluteUri);
            var loopHost = MushAuthClient.BuildValidatedAuthEndpointUri("http://localhost:8080", "/api/auth/login", allowInsecureNonLoopback: false);
            Assert.AreEqual("http://localhost:8080/api/auth/login", loopHost.AbsoluteUri);
            var loopV6 = MushAuthClient.BuildValidatedAuthEndpointUri("http://[::1]:8080", "/api/auth/login", allowInsecureNonLoopback: false);
            Assert.AreEqual("http://[::1]:8080/api/auth/login", loopV6.AbsoluteUri);
            // Dev-mode opt-in still allows remote HTTP.
            var devRemote = MushAuthClient.BuildValidatedAuthEndpointUri("http://dev1.conclav.io", "/api/auth/login", allowInsecureNonLoopback: true);
            Assert.AreEqual("http://dev1.conclav.io/api/auth/login", devRemote.AbsoluteUri);

            // 2. Credential-bearing POST request disables automatic redirects and attaches 64 KiB bounded download handler.
            using (var req = MushAuthClient.CreateAuthPostRequest(httpsUri, System.Text.Encoding.UTF8.GetBytes("{}"), out BoundedDownloadHandler handler))
            {
                Assert.AreEqual(0, req.redirectLimit, "Auth POST requests must set redirectLimit = 0 to block redirect downgrades");
                Assert.AreEqual(UnityEngine.Networking.UnityWebRequest.kHttpVerbPOST, req.method);
                Assert.AreSame(handler, req.downloadHandler);
                Assert.AreEqual(MushAuthClient.MaxAuthResponseBytes, handler.MaxBytes);

#if !UNITY_5_3_OR_NEWER
                // 3xx redirect status must be rejected cleanly (uses writable UnityWebRequest stubs in CI).
                req.responseCode = 307;
                req.result = UnityEngine.Networking.UnityWebRequest.Result.Success;
                Assert.Throws<System.InvalidOperationException>(() =>
                    MushAuthClient.ProcessAuthResponse("https://dev1.conclav.io", httpsUri, req, handler, allowInsecureNonLoopback: false));

                // Defense-in-depth: mutated req.url downgrading to cleartext HTTP or shifting host must be rejected.
                req.responseCode = 200;
                req.url = "http://dev1.conclav.io/api/auth/login";
                Assert.Throws<System.InvalidOperationException>(() =>
                    MushAuthClient.ProcessAuthResponse("https://dev1.conclav.io", httpsUri, req, handler, allowInsecureNonLoopback: false));

                req.url = "https://evil.example/api/auth/login";
                Assert.Throws<System.InvalidOperationException>(() =>
                    MushAuthClient.ProcessAuthResponse("https://dev1.conclav.io", httpsUri, req, handler, allowInsecureNonLoopback: false));

                // Valid HTTPS 200 JSON response succeeds.
                req.url = httpsUri.AbsoluteUri;
                byte[] okBody = System.Text.Encoding.UTF8.GetBytes(@"{""token"":""tok_123"",""account"":""ada"",""characters"":[{""key"":""c1"",""name"":""Ada""}]}");
                Assert.IsTrue(handler.FeedChunkForTest(okBody, okBody.Length));
                var session = MushAuthClient.ProcessAuthResponse("https://dev1.conclav.io", httpsUri, req, handler, allowInsecureNonLoopback: false);
                Assert.AreEqual("tok_123", session.Token);
                Assert.AreEqual("ada", session.Account);
                Assert.AreEqual(1, session.Characters.Count);
#endif
            }

            // 3. ProjectSettings.asset must set insecureHttpOption: 1 (DevelopmentOnly), not 2 (AlwaysAllowed).
            string projectSettingsPath = System.IO.Path.GetFullPath(
                System.IO.Path.Combine(System.AppContext.BaseDirectory, "..", "..", "..", "..", "ProjectSettings", "ProjectSettings.asset"));
            if (!System.IO.File.Exists(projectSettingsPath))
            {
                projectSettingsPath = System.IO.Path.GetFullPath(
                    System.IO.Path.Combine(System.IO.Directory.GetCurrentDirectory(), "unity", "ProjectSettings", "ProjectSettings.asset"));
            }
            Assert.IsTrue(System.IO.File.Exists(projectSettingsPath), $"Expected ProjectSettings.asset at {projectSettingsPath}");
            string settingsText = System.IO.File.ReadAllText(projectSettingsPath);
            StringAssert.Contains("insecureHttpOption: 1", settingsText);
            StringAssert.DoesNotContain("insecureHttpOption: 2", settingsText);
        }

        [Test]
        public void Security_EXT_P2_6_BoundedDownloadHandlerCapsAuthResponsesAt64KiB()
        {
            Assert.AreEqual(64 * 1024, MushAuthClient.MaxAuthResponseBytes);

            // 1. Exact 64 KiB across multiple chunks succeeds without overflow.
            using (var exact = new BoundedDownloadHandler(MushAuthClient.MaxAuthResponseBytes))
            {
                byte[] chunk = new byte[8 * 1024];
                for (int i = 0; i < chunk.Length; i++) chunk[i] = (byte)'a';
                for (int c = 0; c < 8; c++)
                    Assert.IsTrue(exact.FeedChunkForTest(chunk, chunk.Length), $"Chunk {c} within 64 KiB limit should succeed");
                Assert.IsFalse(exact.Overflowed);
                Assert.AreEqual(64 * 1024, exact.ReceivedBytes);
                Assert.AreEqual(64 * 1024, exact.data.Length);
                Assert.AreEqual(64 * 1024, exact.text.Length);

                // One byte past 64 KiB overflows and immediately returns false + clears memory.
                Assert.IsFalse(exact.FeedChunkForTest(new byte[] { (byte)'!' }, 1));
                Assert.IsTrue(exact.Overflowed);
                Assert.AreEqual(0, exact.data.Length);
                Assert.AreEqual(string.Empty, exact.text);
            }

            // 2. Oversized Content-Length header triggers immediate overflow and request abort callback.
            var httpsUri = MushAuthClient.BuildValidatedAuthEndpointUri("https://dev1.conclav.io", "/api/auth/login", allowInsecureNonLoopback: false);
            using (var req = MushAuthClient.CreateAuthPostRequest(httpsUri, System.Text.Encoding.UTF8.GetBytes("{}"), out BoundedDownloadHandler handler))
            {
                handler.FeedContentLengthHeaderForTest((ulong)MushAuthClient.MaxAuthResponseBytes + 1UL);
                Assert.IsTrue(handler.Overflowed);
#if !UNITY_5_3_OR_NEWER
                Assert.IsTrue(req.aborted, "Oversized Content-Length must abort UnityWebRequest immediately");
#endif
                Assert.IsFalse(handler.FeedChunkForTest(new byte[] { (byte)'x' }, 1));

#if !UNITY_5_3_OR_NEWER
                // Both 200 success and 500 error responses must reject cleanly on overflow.
                req.responseCode = 200;
                req.result = UnityEngine.Networking.UnityWebRequest.Result.Success;
                var ex200 = Assert.Throws<System.InvalidOperationException>(() =>
                    MushAuthClient.ProcessAuthResponse("https://dev1.conclav.io", httpsUri, req, handler, allowInsecureNonLoopback: false));
                StringAssert.Contains("exceeded maximum allowed size", ex200.Message);

                req.responseCode = 500;
                req.result = UnityEngine.Networking.UnityWebRequest.Result.ProtocolError;
                var ex500 = Assert.Throws<System.InvalidOperationException>(() =>
                    MushAuthClient.ProcessAuthResponse("https://dev1.conclav.io", httpsUri, req, handler, allowInsecureNonLoopback: false));
                StringAssert.Contains("exceeded maximum allowed size", ex500.Message);
#endif
            }

            // 3. Chunked response without Content-Length that exceeds 64 KiB aborts request and rejects.
            using (var req = MushAuthClient.CreateAuthPostRequest(httpsUri, System.Text.Encoding.UTF8.GetBytes("{}"), out BoundedDownloadHandler handler))
            {
                byte[] chunk = new byte[16 * 1024];
                for (int c = 0; c < 4; c++)
                    Assert.IsTrue(handler.FeedChunkForTest(chunk, chunk.Length));
#if !UNITY_5_3_OR_NEWER
                Assert.IsFalse(req.aborted);
#endif
                Assert.IsFalse(handler.FeedChunkForTest(new byte[] { (byte)'z' }, 1));
                Assert.IsTrue(handler.Overflowed);
#if !UNITY_5_3_OR_NEWER
                Assert.IsTrue(req.aborted, "Chunked overflow without Content-Length must abort UnityWebRequest");
#endif
            }
        }

        [Test]
        public void Security_EXT_P1_6_ExitGotoPrefixing_BayAllowlist_AndSingleHopRoomBoundWalker()
        {
            // 1. Safe exit formatting rejects blank/control names and always prefixes authored exits with `goto `.
            Assert.AreEqual("goto n", TacSimContentLoader.FormatSafeExitCommand("n"));
            Assert.AreEqual("goto @destroy me", TacSimContentLoader.FormatSafeExitCommand("@destroy me"));
            Assert.AreEqual("goto QUIT", TacSimContentLoader.FormatSafeExitCommand("QUIT"));
            Assert.AreEqual("goto +tv/bay in", TacSimContentLoader.FormatSafeExitCommand("+tv/bay in"));
            Assert.IsNull(TacSimContentLoader.FormatSafeExitCommand(""));
            Assert.IsNull(TacSimContentLoader.FormatSafeExitCommand("   "));
            Assert.IsNull(TacSimContentLoader.FormatSafeExitCommand("North\nQUIT"));
            Assert.IsNull(TacSimContentLoader.FormatSafeExitCommand("North\r\n@boot"));
            Assert.IsNull(TacSimContentLoader.FormatSafeExitCommand("North\0evil"));
            Assert.IsNull(TacSimContentLoader.FormatSafeExitCommand("North\u009b2J"));

            // Static ship bay actions are ONLY allowlisted when explicitly flagged isStaticAction=true.
            Assert.AreEqual("+tv/bay in", TacSimContentLoader.FormatShipTraversalCommand("+tv/bay in", isStaticAction: true));
            Assert.AreEqual("+tv/bay out", TacSimContentLoader.FormatShipTraversalCommand("+tv/bay out", isStaticAction: true));
            Assert.AreEqual("goto +tv/bay in", TacSimContentLoader.FormatShipTraversalCommand("+tv/bay in", isStaticAction: false));
            Assert.IsNull(TacSimContentLoader.FormatShipTraversalCommand("@destroy me", isStaticAction: true));

            // 2. Hostile authored station & ship packs cannot inject bare commands or control characters.
            var hostileStation = TacSimContentLoader.ParseNeighborhoodJson(@"{
                ""manifest"": { ""id"": ""neighborhood.hostile"", ""version"": 1, ""kind"": ""neighborhood"" },
                ""entries"": {
                    ""room.a"": { ""kind"": ""room"", ""def"": { ""name"": ""A"", ""footprint"": { ""rect"": [[0,0],[10,10]] } } },
                    ""room.b"": { ""kind"": ""room"", ""def"": { ""name"": ""B"", ""footprint"": { ""rect"": [[10,0],[20,10]] } } },
                    ""room.c"": { ""kind"": ""room"", ""def"": { ""name"": ""C"", ""footprint"": { ""rect"": [[20,0],[30,10]] } } },
                    ""room.d"": { ""kind"": ""room"", ""def"": { ""name"": ""D"", ""footprint"": { ""rect"": [[30,0],[40,10]] } } },
                    ""exit.ab"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.a"", ""to"": ""room.b"", ""names"": [""@destroy me"", ""QUIT""] } },
                    ""exit.bc"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.b"", ""to"": ""room.c"", ""names"": [""East;e\nQUIT"", ""West;w""] } },
                    ""exit.bd"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.b"", ""to"": ""room.d"", ""names"": [""   "", """"] } }
                }
            }", "neighborhood.hostile");
            var ab = hostileStation.FindExitPath("room.a", "room.b");
            Assert.AreEqual(1, ab.Count);
            Assert.AreEqual("goto @destroy me", ab[0].command);
            var ba = hostileStation.FindExitPath("room.b", "room.a");
            Assert.AreEqual(1, ba.Count);
            Assert.AreEqual("goto QUIT", ba[0].command);
            Assert.AreEqual(0, hostileStation.FindExitPath("room.b", "room.c").Count);
            Assert.AreEqual(0, hostileStation.FindExitPath("room.b", "room.d").Count);

            var hostileShip = TacSimContentLoader.ParseShipUnitJson(@"{
                ""entries"": {
                    ""tacsim.unit.evil"": {
                        ""kind"": ""unit"",
                        ""def"": {
                            ""label"": ""Evil"",
                            ""interior"": {
                                ""rooms"": [
                                    { ""id"": ""airlock"", ""kind"": ""walkable"", ""name"": ""Airlock"" },
                                    { ""id"": ""passage"", ""kind"": ""walkable"", ""name"": ""Passage"" },
                                    { ""id"": ""bridge"", ""kind"": ""walkable"", ""name"": ""Bridge"" }
                                ],
                                ""exits"": [
                                    { ""from"": ""airlock"", ""to"": ""passage"", ""names"": [""Fake Bay;+tv/bay in"", ""Fore;fore""] },
                                    { ""from"": ""passage"", ""to"": ""bridge"", ""names"": [""Up;u\nQUIT"", ""Down;d""] }
                                ]
                            }
                        }
                    }
                }
            }", "tacsim.unit.evil");
            var evilShipHop = hostileShip.FindExitPath("airlock", "passage");
            Assert.AreEqual(1, evilShipHop.Count);
            Assert.AreEqual("goto +tv/bay in", evilShipHop[0].command);
            Assert.AreEqual(0, hostileShip.FindExitPath("passage", "bridge").Count);

            // Real Dray-P preserves +tv/bay in / +tv/bay out while prefixing authored interior hops with `goto `.
            var drayP = TacSimContentLoader.LoadShipTemplate("tacsim.unit.dray_p");
            var toBay = drayP.FindExitPath("airlock", "bay");
            Assert.GreaterOrEqual(toBay.Count, 3);
            for (int i = 0; i < toBay.Count - 1; i++)
                StringAssert.StartsWith("goto ", toBay[i].command);
            Assert.AreEqual("+tv/bay in", toBay[toBay.Count - 1].command);
            var outBay = drayP.FindExitPath("bay", "hold");
            Assert.AreEqual(1, outBay.Count);
            Assert.AreEqual("+tv/bay out", outBay[0].command);

            // FindExitToCeresFreeport prefixes matched exits with `goto ` and rejects control-char aliases.
            var limboHostile = new MushRoomView
            {
                Name = "Limbo",
                Exits = new List<MushExitRef>
                {
                    new MushExitRef { Name = "Ceres Freeport", Aliases = new List<string> { "ceres\nQUIT" } },
                    new MushExitRef { Name = "Ceres Arrivals", Aliases = new List<string> { "@destroy me" } }
                }
            };
            Assert.AreEqual("goto @destroy me", TacSimGameCoordinator.FindExitToCeresFreeport(limboHostile));

            // 3. Direct RoomBoundNavWalker unit checks (1 hop at a time, bound to source room, cancel on mismatch/action).
            var walker = new RoomBoundNavWalker();
            var multiHops = new List<(string command, string targetRoom)>
            {
                ("goto n", "room.b"),
                ("goto e", "room.c"),
                ("goto n", "room.d")
            };
            var hop1 = walker.PlanAndDispatchOne(NavWalkerScope.Station, "room.a", multiHops, 10f);
            Assert.IsTrue(hop1.HasValue);
            Assert.AreEqual("room.a", hop1.Value.FromRoom);
            Assert.AreEqual("room.b", hop1.Value.TargetRoom);
            Assert.AreEqual("goto n", hop1.Value.Command);
            Assert.AreEqual(2, walker.QueuedCount);
            Assert.IsFalse(walker.PlanAndDispatchOne(NavWalkerScope.Station, "room.a", multiHops, 10.2f).HasValue);

            var r2 = walker.OnAuthoritativeRoomUpdate(NavWalkerScope.Station, "room.b", 10.5f, allowAdvance: true);
            Assert.AreEqual("room.b", r2.AcknowledgedTarget);
            Assert.IsFalse(r2.CanceledOnMismatch);
            Assert.IsTrue(r2.NextHopToSend.HasValue);
            Assert.AreEqual("room.b", r2.NextHopToSend.Value.FromRoom);
            Assert.AreEqual("room.c", r2.NextHopToSend.Value.TargetRoom);
            Assert.AreEqual("goto e", r2.NextHopToSend.Value.Command);
            Assert.AreEqual(1, walker.QueuedCount);

            // Mismatch cancels remaining queue.
            var rMismatch = walker.OnAuthoritativeRoomUpdate(NavWalkerScope.Station, "room.trap", 11.0f, allowAdvance: true);
            Assert.IsTrue(rMismatch.CanceledOnMismatch);
            Assert.IsNull(rMismatch.AcknowledgedTarget);
            Assert.IsFalse(rMismatch.NextHopToSend.HasValue);
            Assert.AreEqual(0, walker.QueuedCount);
            Assert.IsFalse(walker.InFlight.HasValue);

            // 4. End-to-end TacSimGameCoordinator FlushPendingRoomMovesBeforeAction sends ONLY ONE hop at a time,
            //    advances on matching RoomView, and cancels remaining hops on mismatch or explicit command!
            var rootGo = new UnityEngine.GameObject("ExtP16CoordinatorWalkerTest");
            try
            {
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                var stationGo = new UnityEngine.GameObject("Station");
                stationGo.transform.SetParent(rootGo.transform, false);
                coord.StationWorld = stationGo.AddComponent<StationWorldController>();
                var shipGo = new UnityEngine.GameObject("Ship");
                shipGo.transform.SetParent(rootGo.transform, false);
                coord.ShipInterior = shipGo.AddComponent<ShipInteriorController>();
                var spaceGo = new UnityEngine.GameObject("Space");
                spaceGo.transform.SetParent(rootGo.transform, false);
                coord.SpaceStage = spaceGo.AddComponent<SpaceFlightController>();
                var surfaceGo = new UnityEngine.GameObject("Surface");
                surfaceGo.transform.SetParent(rootGo.transform, false);
                coord.SurfaceStage = surfaceGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                coord.ArenaStage = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                coord.CameraRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                coord.CameraRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();
                var playerGo = new UnityEngine.GameObject("Player", typeof(UnityEngine.CharacterController));
                playerGo.transform.SetParent(rootGo.transform, false);
                coord.Player = playerGo.AddComponent<TacSim.Character.ThirdPersonPlayerController>();
                coord.Player.VisualSpawner = playerGo.AddComponent<TacSim.Character.CharacterVisualSpawner>();
                var hudGo = new UnityEngine.GameObject("Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();
                coord.EnsureSubsystems();

                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                var onFlush = typeof(TacSimGameCoordinator).GetMethod("FlushPendingRoomMovesBeforeAction", flags);
                var simConnField = typeof(TacSimGameCoordinator).GetField("_simulateConnectedForTests", flags);
                var awaitingInitField = typeof(TacSimGameCoordinator).GetField("_awaitingInitialRoomSync", flags);
                simConnField.SetValue(coord, true);
                awaitingInitField.SetValue(coord, true);

                var dispatched = new List<string>();
                coord.OnCommandDispatched += dispatched.Add;

                MushRoomView MakeStationRoom(string id, string name, string entry) =>
                    MushProtocol.ParseRoomView(JsonNode.Parse(
                        $@"{{""id"":""{id}"",""name"":""{name}"",""ambient"":{{""atlas"":{{""pack"":""neighborhood.ceres_freeport"",""entry"":""{entry}"",""surface"":""station""}}}}}}"));

#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 100f;
#endif
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#101", "Visiting Berth Gallery", "room.c01_berth_gallery") });

                // Walk 2 hops north (c01_berth_gallery -> c02_pressure -> c03_arrivals) and call Flush twice before RoomView.
                coord.Player.TeleportTo(coord.StationWorld.GetRoomSpawnPosition("room.c03_arrivals"), 0f);
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 101f;
#endif
                onFlush.Invoke(coord, null);
                onFlush.Invoke(coord, null);
                Assert.AreEqual(1, dispatched.Count, "Multi-hop flush must send ONLY ONE hop before RoomView confirmation");
                Assert.AreEqual("goto n", dispatched[0]);

                // Authoritative RoomView confirming c02_pressure advances ONLY hop 2 (`goto n`).
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 101.5f;
#endif
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#102", "Pressure Lobby", "room.c02_pressure") });
                Assert.AreEqual(2, dispatched.Count);
                Assert.AreEqual("goto n", dispatched[1]);

                // Authoritative RoomView confirming c03_arrivals completes with no extra commands.
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 102f;
#endif
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#103", "Port Watch Arrivals", "room.c03_arrivals") });
                Assert.AreEqual(2, dispatched.Count);

                // Mismatch cancellation: walk 2 hops back south, flush hop 1, then receive RoomView STILL in c03_arrivals -> hop 2 canceled!
                dispatched.Clear();
                coord.Player.TeleportTo(coord.StationWorld.GetRoomSpawnPosition("room.c01_berth_gallery"), 180f);
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 105f;
#endif
                onFlush.Invoke(coord, null);
                Assert.AreEqual(1, dispatched.Count);
                Assert.AreEqual("goto s", dispatched[0]);
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 105.5f;
#endif
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#103", "Port Watch Arrivals", "room.c03_arrivals") });
                Assert.AreEqual(1, dispatched.Count, "Mismatched RoomView must cancel remaining queued hops");

                // Unrelated explicit command cancels remaining queued hops!
                dispatched.Clear();
                coord.Player.TeleportTo(coord.StationWorld.GetRoomSpawnPosition("room.c01_berth_gallery"), 180f);
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 110f;
#endif
                onFlush.Invoke(coord, null);
                Assert.AreEqual(1, dispatched.Count);
                coord.ExecutePlayerCommand("look");
                dispatched.Clear();
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 110.5f;
#endif
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#102", "Pressure Lobby", "room.c02_pressure") });
                Assert.AreEqual(0, dispatched.Count, "Explicit command must cancel queued multi-hop navigation");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        private static System.IO.MemoryStream BuildSyntheticTarWithTypeFlag(string entryName, byte typeFlag, byte[] payload)
        {
            var gzOut = new System.IO.MemoryStream();
            using (var gz = new System.IO.Compression.GZipStream(gzOut, System.IO.Compression.CompressionLevel.Fastest, leaveOpen: true))
            {
                byte[] header = new byte[512];
                byte[] nameBytes = System.Text.Encoding.ASCII.GetBytes(entryName);
                System.Array.Copy(nameBytes, 0, header, 0, System.Math.Min(nameBytes.Length, 100));
                byte[] modeBytes = System.Text.Encoding.ASCII.GetBytes("0000644\0");
                System.Array.Copy(modeBytes, 0, header, 100, modeBytes.Length);
                int len = payload != null ? payload.Length : 0;
                string octalSize = System.Convert.ToString(len, 8).PadLeft(11, '0') + "\0";
                System.Array.Copy(System.Text.Encoding.ASCII.GetBytes(octalSize), 0, header, 124, 12);
                header[156] = typeFlag;
                for (int c = 148; c < 156; c++) header[c] = (byte)' ';
                int checksum = 0;
                for (int c = 0; c < 512; c++) checksum += header[c];
                string octalSum = System.Convert.ToString(checksum, 8).PadLeft(6, '0') + "\0 ";
                System.Array.Copy(System.Text.Encoding.ASCII.GetBytes(octalSum), 0, header, 148, 8);
                gz.Write(header, 0, 512);
                if (len > 0)
                {
                    gz.Write(payload, 0, len);
                    int rem = len % 512;
                    if (rem != 0) gz.Write(new byte[512 - rem], 0, 512 - rem);
                }
                gz.Write(new byte[1024], 0, 1024);
            }
            gzOut.Position = 0;
            return gzOut;
        }

        [Test]
        public void Security_P2_CLIENT_4_UnityPackageExtractor_BlocksAllScripts_WindowsTrailingDots_NonRegularTarTypes_AndMetaCap()
        {
            string tempDir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "TacSimP2Client4_" + System.Guid.NewGuid().ToString("N"));
            try
            {
                // 1. Executable/script/assembly/shader extensions blocked EVERYWHERE under Assets/ (not just Editor/),
                //    Windows trailing dots/spaces & reserved device names rejected, while valid passive assets succeed.
                byte[] payload = System.Text.Encoding.UTF8.GetBytes("payload");
                using (var pkg = BuildSyntheticUnityPackage(
                    ("00000000000000000000000000000001/asset", payload),
                    ("00000000000000000000000000000001/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/RuntimeEvil.cs\n")),
                    ("00000000000000000000000000000002/asset", payload),
                    ("00000000000000000000000000000002/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Plugin.asmdef\n")),
                    ("00000000000000000000000000000003/asset", payload),
                    ("00000000000000000000000000000003/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Ref.asmref\n")),
                    ("00000000000000000000000000000004/asset", payload),
                    ("00000000000000000000000000000004/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Native.dll\n")),
                    ("00000000000000000000000000000005/asset", payload),
                    ("00000000000000000000000000000005/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/csc.rsp\n")),
                    ("00000000000000000000000000000006/asset", payload),
                    ("00000000000000000000000000000006/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Legacy.js\n")),
                    ("00000000000000000000000000000007/asset", payload),
                    ("00000000000000000000000000000007/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Legacy.boo\n")),
                    ("00000000000000000000000000000008/asset", payload),
                    ("00000000000000000000000000000008/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Custom.shader\n")),
                    ("00000000000000000000000000000009/asset", payload),
                    ("00000000000000000000000000000009/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/_TacSim./Prefabs/Bypass.prefab\n")),
                    ("0000000000000000000000000000000a/asset", payload),
                    ("0000000000000000000000000000000a/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/_TacSim /Prefabs/BypassSpace.prefab\n")),
                    ("0000000000000000000000000000000b/asset", payload),
                    ("0000000000000000000000000000000b/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Editor./Spoof.prefab\n")),
                    ("0000000000000000000000000000000c/asset", payload),
                    ("0000000000000000000000000000000c/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Sneaky.cs.\n")),
                    ("0000000000000000000000000000000d/asset", payload),
                    ("0000000000000000000000000000000d/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/CON.prefab\n")),
                    ("0000000000000000000000000000000e/asset", payload),
                    ("0000000000000000000000000000000e/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/SafeProp.prefab\n"))))
                {
                    int written = UnityPackageArchiveExtractor.ExtractPackageStream(pkg, tempDir);
                    Assert.AreEqual(1, written, "Only the safe passive .prefab asset must extract");
                }
                Assert.IsTrue(System.IO.File.Exists(System.IO.Path.Combine(tempDir, "Assets", "Vendor", "SafeProp.prefab")));
                Assert.IsFalse(System.IO.File.Exists(System.IO.Path.Combine(tempDir, "Assets", "Vendor", "RuntimeEvil.cs")));
                Assert.IsFalse(System.IO.File.Exists(System.IO.Path.Combine(tempDir, "Assets", "Vendor", "Custom.shader")));
                Assert.IsFalse(System.IO.Directory.Exists(System.IO.Path.Combine(tempDir, "Assets", "_TacSim")));

                // 2. Non-regular tar typeFlags (hardlink '1', symlink '2', char dev '3', FIFO '6') throw InvalidDataException;
                //    directory entry ('5') is accepted cleanly.
                byte[] disallowedFlags = { (byte)'1', (byte)'2', (byte)'3', (byte)'4', (byte)'6' };
                for (int i = 0; i < disallowedFlags.Length; i++)
                {
                    using (var badTar = BuildSyntheticTarWithTypeFlag("aaaa/asset", disallowedFlags[i], new byte[0]))
                    {
                        Assert.Throws<System.IO.InvalidDataException>(() =>
                            UnityPackageArchiveExtractor.ExtractPackageStream(badTar, tempDir),
                            $"Tar typeFlag '{(char)disallowedFlags[i]}' must be rejected");
                    }
                }
                using (var dirTar = BuildSyntheticTarWithTypeFlag("aaaa/", (byte)'5', new byte[0]))
                {
                    Assert.AreEqual(0, UnityPackageArchiveExtractor.ExtractPackageStream(dirTar, tempDir));
                }

                // 3. Oversized asset.meta (> MaxMetaHeaderBytes = 64 KiB) throws InvalidDataException.
                byte[] oversizedMeta = new byte[UnityPackageArchiveExtractor.MaxMetaHeaderBytes + 1];
                using (var bigMetaPkg = BuildSyntheticUnityPackage(
                    ("dddddddddddddddddddddddddddddddd/pathname", System.Text.Encoding.UTF8.GetBytes("Assets/Vendor/Mesh.prefab\n")),
                    ("dddddddddddddddddddddddddddddddd/asset.meta", oversizedMeta)))
                {
                    Assert.Throws<System.IO.InvalidDataException>(() =>
                        UnityPackageArchiveExtractor.ExtractPackageStream(bigMetaPkg, tempDir));
                }
            }
            finally
            {
                if (System.IO.Directory.Exists(tempDir))
                    System.IO.Directory.Delete(tempDir, recursive: true);
            }
        }

        [Test]
        public void Security_P2_CLIENT_8_FallbackPackSanitizer_DropsCompoundStaffLocks_AndRedactsNonStaffLocks()
        {
            string rawPack = @"{
                ""manifest"": { ""id"": ""neighborhood.lock_test"", ""version"": 1 },
                ""entries"": {
                    ""room.public"": { ""kind"": ""room"", ""def"": { ""name"": ""Public"", ""access"": ""public"" } },
                    ""exit.exact_staff"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.public"", ""to"": ""room.public"", ""lock"": ""staff"" } },
                    ""exit.compound_wiz"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.public"", ""to"": ""room.public"", ""lock"": ""#1|flag^WIZARD"" } },
                    ""exit.compound_roy"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.public"", ""to"": ""room.public"", ""lock"": ""flag^ROYALTY&BADGE:OPS"" } },
                    ""exit.portal_staff"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.public"", ""to"": ""room.public"", ""portal"": { ""lock"": ""flag^WIZARD"" } } },
                    ""exit.player_key"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.public"", ""to"": ""room.public"", ""lock"": ""#42&KEY:SECRET_CODE"" } },
                    ""exit.portal_key"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.public"", ""to"": ""room.public"", ""portal"": { ""lock"": ""ATTR:PASS_99"" } } },
                    ""exit.empty_lock"": { ""kind"": ""exit"", ""def"": { ""from"": ""room.public"", ""to"": ""room.public"", ""lock"": ""   "", ""portal"": { ""lock"": false } } }
                }
            }";

            string sanitized = TacSimContentLoader.SanitizePackJsonForClient(rawPack);
            StringAssert.DoesNotContain("WIZARD", sanitized);
            StringAssert.DoesNotContain("ROYALTY", sanitized);
            StringAssert.DoesNotContain("SECRET_CODE", sanitized);
            StringAssert.DoesNotContain("#42", sanitized);
            StringAssert.DoesNotContain("PASS_99", sanitized);

            var entries = JsonNode.Parse(sanitized)["entries"].AsObject();
            Assert.IsFalse(entries.ContainsKey("exit.exact_staff"));
            Assert.IsFalse(entries.ContainsKey("exit.compound_wiz"));
            Assert.IsFalse(entries.ContainsKey("exit.compound_roy"));
            Assert.IsFalse(entries.ContainsKey("exit.portal_staff"));
            Assert.AreEqual("locked", entries["exit.player_key"]["def"].Str("lock"));
            Assert.AreEqual("locked", entries["exit.portal_key"]["def"]["portal"].Str("lock"));
            Assert.IsFalse(entries["exit.empty_lock"]["def"].ContainsKey("lock"));
            Assert.IsFalse(entries["exit.empty_lock"]["def"]["portal"].ContainsKey("lock"));
        }

        [Test]
        public void Security_P2_CLIENT_9_NonExitCommandInterpolation_ControlChars_AndStrictTokenAllowlists()
        {
            // 1. MushWsClient.SendLine rejects C0/DEL/C1 control characters centrally; BuildLineCommand strips them.
            using (var ws = new MushWsClient())
            {
                ws.SendLine("look");
                Assert.AreEqual(1, ws.OutboundQueueCount);
                ws.SendLine("look\nQUIT");
                ws.SendLine("look\r\n@destroy me");
                ws.SendLine("look\0bad");
                ws.SendLine("look\u007fbad");
                ws.SendLine("look\u009b2J");
                Assert.AreEqual(1, ws.OutboundQueueCount, "SendLine must reject all C0/DEL/C1 control characters");
            }
            string builtLine = MushProtocol.BuildLineCommand("hello\r\nQUIT\u009bworld");
            var builtNode = JsonNode.Parse(builtLine);
            Assert.AreEqual("helloQUITworld", builtNode.Str("d"));

            // 2. Strict single-token allowlist (^[A-Za-z0-9._-]+$, no leading - / . / ..).
            Assert.IsTrue(MushProtocol.TryValidateSafeCommandToken("A", out _));
            Assert.IsTrue(MushProtocol.TryValidateSafeCommandToken("dock.401.1", out _));
            Assert.IsTrue(MushProtocol.TryValidateSafeCommandToken("raider_7-a", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeCommandToken("", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeCommandToken("A B", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeCommandToken("A\tB", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeCommandToken("A\nQUIT", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeCommandToken("-flag", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeCommandToken("..bad", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeCommandToken("a;QUIT", out _));

            // 3. Safe keyword / NPC name validation rejects control chars and leading switches.
            Assert.IsTrue(MushProtocol.TryValidateSafeKeywordOrName("Mooring Plaque", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeKeywordOrName("-switch", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeKeywordOrName("/switch", out _));
            Assert.IsFalse(MushProtocol.TryValidateSafeKeywordOrName("Plaque\nQUIT", out _));

            // 4. Orbital mining command builder rejects injected rock tokens.
            Assert.IsNull(TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:helm_match", "A\nQUIT", true, "A", out _));
            Assert.IsNull(TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:helm_match", "A --force", true, "A", out _));
            Assert.IsNull(TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:helm_match", "-bad", true, "-bad", out _));
            Assert.AreEqual("+helm/match A", TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:helm_match", "a", true, "A", out _));

            // 5. DockingClientSession refuses crafted session_id in FromFeed and FormatTapeCommand.
            var badDock = DockingClientSession.FromFeed(JsonNode.Parse(@"{""session_id"":""dock.1 extra"",""dx_mm"":0,""dy_mm"":0,""dz_mm"":100}"));
            Assert.IsNotNull(badDock);
            Assert.AreEqual(string.Empty, badDock.SessionId, "DockingClientSession.FromFeed must clear unsafe session_id");
            Assert.AreEqual(string.Empty, badDock.FormatTapeCommand(), "FormatTapeCommand must refuse cleared/unsafe session_id");
            var manualBad = new DockingClientSession { SessionId = "dock.1\nQUIT" };
            Assert.AreEqual(string.Empty, manualBad.FormatTapeCommand(), "FormatTapeCommand must refuse unsafe session_id");

            // 6. Arena ExecuteCockpitAction and MissionOverlays ResolveSquadAdvanceCommand refuse injected tokens.
            var rootGo = new UnityEngine.GameObject("P2Client9ArenaTest");
            try
            {
                var arena = rootGo.AddComponent<TacSim.Arena.OocArenaController>();
                var emitted = new List<string>();
                arena.OnLiveCockpitVerbRequested += emitted.Add;
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 1, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""contacts"": [{""designator"": ""B1 --evil"", ""kind"": ""tank"", ""cell"": [10,10], ""range_m"": 100, ""bearing"": 0}]
                }"));
                arena.ExecuteCockpitAction("lock B1 --evil");
                arena.ExecuteCockpitAction("fire\nQUIT");
                Assert.AreEqual(0, emitted.Count, "Injected designator and control-char cockpit actions must not emit commands");
                arena.ExecuteCockpitAction("lock B1");
                Assert.AreEqual(1, emitted.Count);
                Assert.AreEqual("lock B1", emitted[0]);

                var overlays = rootGo.AddComponent<TacSim.UI.TacSimMissionOverlays>();
                overlays.BindControllers(null, arena, null);
                var badGrid = new DecodedTerrainGrid
                {
                    Width = 10, Height = 10,
                    Classes = new TerrainCellClass[100],
                    ElevMeters = new float[100],
                    KeySpots = new List<ArenaKeySpotDef> { new ArenaKeySpotDef { Letter = "A\nQUIT", CellX = 5, CellY = 5 } }
                };
                arena.BuildArenaTerrain(null, badGrid);
                Assert.IsNull(overlays.ResolveSquadAdvanceCommand(), "Unsafe keyspot letter must be refused instead of interpolated into squad advance");
                badGrid.KeySpots.Add(new ArenaKeySpotDef { Letter = "B", CellX = 6, CellY = 6 });
                Assert.AreEqual("order all advance B", overlays.ResolveSquadAdvanceCommand(), "Valid keyspot letter must resolve cleanly");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Security_P2_CLIENT_10_UnityRichTextDisabledByDefault_AndEscapesUntrustedStrings()
        {
            var rootGo = new UnityEngine.GameObject("P2Client10RichTextTest");
            try
            {
                // 1. Station Berth Gallery escapes both OwnHullName AND peer-controlled rv.Contents hull name,
                //    and all 3D TMP signs disable richText by default.
                var stationGo = new UnityEngine.GameObject("StationRichText");
                stationGo.transform.SetParent(rootGo.transform, false);
                var station = stationGo.AddComponent<StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                var berthRoom = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#101"",
                    ""name"": ""Visiting Berth Gallery"",
                    ""ambient"": {
                        ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c01_berth_gallery"", ""surface"": ""station"" },
                        ""tacsim"": {
                            ""own_hull"": ""#500"",
                            ""own_hull_name"": ""<size=999>OwnEvil</size>"",
                            ""own_hull_docked_here"": true
                        }
                    },
                    ""contents"": [
                        { ""id"": ""#501"", ""name"": ""<color=#ff0000>PeerDray</color> (Dray-class)"", ""kind"": ""thing"" }
                    ]
                }"));
                station.ApplyRoomView(berthRoom, "#99");

                var allTmps = stationGo.GetComponentsInChildren<TMPro.TextMeshPro>(true);
                Assert.Greater(allTmps.Length, 0);
                bool sawOwnEscaped = false;
                bool sawPeerEscaped = false;
                for (int i = 0; i < allTmps.Length; i++)
                {
                    // Diegetic status boards explicitly opt in to color markup after EscapeUnityRichText;
                    // all other 3D signs (room headers, cradles, transits, plaques) must disable TMP richText!
                    if (allTmps[i].name != "BoardText")
                        Assert.IsFalse(allTmps[i].richText, $"3D TMP '{allTmps[i].name}' must have richText disabled by default");
                    string txt = allTmps[i].text ?? string.Empty;
                    StringAssert.DoesNotContain("<size=999>", txt);
                    StringAssert.DoesNotContain("<color=#ff0000>", txt);
                    if (txt.Contains("OWNEVIL")) sawOwnEscaped = true;
                    if (txt.Contains("PEERDRAY")) sawPeerEscaped = true;
                }
                Assert.IsTrue(sawOwnEscaped, "Expected escaped OwnHullName on Berth Gallery cradle sign");
                Assert.IsTrue(sawPeerEscaped, "Expected escaped peer docked hull name on Berth Gallery cradle sign");

                // 2. Arena & vehicle visual factory escape callsigns/designators and disable TMP richText.
                var arenaGo = new UnityEngine.GameObject("ArenaRichText");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 7, ""seated"": true, ""callsign"": ""<mark=#ff0>R1</mark>"", ""party"": ""R"",
                    ""contacts"": [{""designator"": ""B1"", ""kind"": ""tank"", ""class_label"": ""<size=500>Heavy</size>"", ""cell"": [10,10], ""range_m"": 100, ""bearing"": 0}]
                }"));
                var arenaRig = TacSimVehicleVisualFactory.SpawnArenaCombatUnit(null, arenaGo.transform, "tank", "<size=999>CALL</size>", "R", UnityEngine.Vector3.zero, 0f);
                var arenaUnitTmps = arenaRig.Root.GetComponentsInChildren<TMPro.TextMeshPro>(true);
                Assert.Greater(arenaUnitTmps.Length, 0);
                for (int i = 0; i < arenaUnitTmps.Length; i++)
                {
                    StringAssert.DoesNotContain("<size=999>", arenaUnitTmps[i].text ?? string.Empty);
                    StringAssert.Contains("‹size=999›", arenaUnitTmps[i].text ?? string.Empty);
                }

                var roverRig = TacSimVehicleVisualFactory.SpawnSurfaceRover(null, arenaGo.transform, "<size=999>ROVER</size>", UnityEngine.Vector3.zero, 0f);
                var roverTmps = roverRig.Root.GetComponentsInChildren<TMPro.TextMeshPro>(true);
                Assert.Greater(roverTmps.Length, 0);
                for (int i = 0; i < roverTmps.Length; i++)
                {
                    Assert.IsFalse(roverTmps[i].richText);
                    StringAssert.DoesNotContain("<size=999>", roverTmps[i].text ?? string.Empty);
                }

                // 3. Mining refusal notice escapes untrusted action names.
                TacSimGameCoordinator.ResolveOrbitalMiningCommand("space:<size=999>evil</size>", "A", true, "A", out string refusal);
                StringAssert.DoesNotContain("<size=999>", refusal ?? string.Empty);
                StringAssert.Contains("‹size=999›", refusal ?? string.Empty);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Security_P2_CLIENT_11_JsonValidation_RoomBounds_WallSlotCap_WsByteBudget_HandshakeLane_EntityCaps_AndPendingExpiry()
        {
            // 1. StoreVerifiedPack rejects valid-SHA-256 malformed JSON and non-object JSON.
            var sync = new TacSimContentSync();
            string brokenJson = "{ not valid json";
            Assert.IsFalse(sync.StoreVerifiedPack("broken", TacSimContentSync.ComputeSha256Hex(brokenJson), brokenJson));
            string arrayJson = "[1, 2, 3]";
            Assert.IsFalse(sync.StoreVerifiedPack("arr", TacSimContentSync.ComputeSha256Hex(arrayJson), arrayJson));
            string strJson = "\"just a string\"";
            Assert.IsFalse(sync.StoreVerifiedPack("str", TacSimContentSync.ComputeSha256Hex(strJson), strJson));

            // 2. ContentModels parsers never throw FormatException on malformed JSON.
            Assert.IsNotNull(TacSimContentLoader.ParseNeighborhoodJson("{ malformed", "neighborhood.bad"));
            Assert.IsNotNull(TacSimContentLoader.ParseGoodsJson("{ malformed"));
            Assert.IsNotNull(TacSimContentLoader.ParseShipUnitJson("{ malformed", "tacsim.unit.dray"));
            Assert.DoesNotThrow(() => TacSimContentLoader.MergeCastJsonIntoPack(TacSimContentLoader.LoadCeresFreeport(), "{ malformed"));

            // 3. Huge room coordinates / spans are clamped and StationWorldController terminates quickly with bounded mullions/stanchions.
            var hugePack = TacSimContentLoader.ParseNeighborhoodJson(@"{
                ""manifest"": { ""id"": ""neighborhood.huge"", ""version"": 1 },
                ""entries"": {
                    ""room.giant"": {
                        ""kind"": ""room"",
                        ""def"": {
                            ""name"": ""Giant"",
                            ""kind"": ""indoor"",
                            ""footprint"": { ""rect"": [[0, 0], [1e20, 1e20]] },
                            ""details"": { ""d1"": ""a"", ""d2"": ""b"", ""d3"": ""c"" }
                        }
                    }
                }
            }", "neighborhood.huge");
            var giant = hugePack.Rooms["room.giant"];
            Assert.LessOrEqual(giant.MaxX - giant.MinX, TacSimContentLoader.MaxRoomSpanMeters + 0.01f);
            Assert.LessOrEqual(giant.MaxY - giant.MinY, TacSimContentLoader.MaxRoomSpanMeters + 0.01f);

            // Even if a synthetic AtlasRoomDef bypasses ParseNeighborhoodJson with 1e20 bounds,
            // StationWorldController wall-slot scans and mullion/stanchion builders must terminate and cap objects!
            var rootGo = new UnityEngine.GameObject("P2Client11WorldTest");
            try
            {
                var station = rootGo.AddComponent<StationWorldController>();
                var sw = System.Diagnostics.Stopwatch.StartNew();
                var rawHugePack = new AtlasNeighborhoodPack { PackId = "neighborhood.raw_huge", Name = "RawHuge" };
                rawHugePack.Rooms["room.c01_berth_gallery"] = new AtlasRoomDef
                {
                    EntryId = "room.c01_berth_gallery",
                    Name = "Huge Berth",
                    Kind = "indoor",
                    MinX = 0f,
                    MaxX = 1e20f,
                    MinY = 0f,
                    MaxY = 1e20f,
                    Details = new Dictionary<string, string> { ["a"] = "1", ["b"] = "2", ["c"] = "3" }
                };
                // Invoke private BuildRoomGeometry via BuildStation using dynamic pack registration
                TacSimContentLoader.RegisterDynamicPack("neighborhood.raw_huge_test", @"{
                    ""manifest"": { ""id"": ""neighborhood.raw_huge_test"", ""version"": 1 },
                    ""entries"": {
                        ""room.c01_berth_gallery"": {
                            ""kind"": ""room"",
                            ""def"": {
                                ""name"": ""Huge Berth"",
                                ""kind"": ""indoor"",
                                ""footprint"": { ""rect"": [[0, 0], [1e20, 1e20]] },
                                ""details"": { ""a"": ""1"", ""b"": ""2"", ""c"": ""3"" }
                            }
                        }
                    }
                }");
                try
                {
                    station.BuildStation(null, "neighborhood.raw_huge_test");
                }
                finally
                {
                    TacSimContentLoader.ClearDynamicPacks();
                }
                sw.Stop();
                Assert.Less(sw.ElapsedMilliseconds, 5000, "Station build on extreme bounds must not hang");
                int mullions = 0;
                int stanchions = 0;
                var allTr = rootGo.GetComponentsInChildren<UnityEngine.Transform>(true);
                for (int i = 0; i < allTr.Length; i++)
                {
                    if (allTr[i].name == "ViewportMullion") mullions++;
                    if (allTr[i].name == "ViewingStanchion") stanchions++;
                }
                Assert.LessOrEqual(mullions, 64);
                Assert.LessOrEqual(stanchions, 64);

                // 4. MushWsClient enforces 16 MiB cumulative inbound byte budget AND preserves handshake `caps` under ping floods.
                using (var ws = new MushWsClient())
                {
                    var env = MushProtocol.ParseEnvelope(@"{""seq"":1,""ts"":1,""t"":""sys"",""d"":{""text"":""chunk""}}");
                    bool ok1 = ws.InjectSyntheticEnvelope(env, 4 * 1024 * 1024);
                    bool ok2 = ws.InjectSyntheticEnvelope(env, 4 * 1024 * 1024);
                    bool ok3 = ws.InjectSyntheticEnvelope(env, 4 * 1024 * 1024);
                    bool ok4 = ws.InjectSyntheticEnvelope(env, 4 * 1024 * 1024);
                    bool ok5 = ws.InjectSyntheticEnvelope(env, 1024);
                    Assert.IsTrue(ok1 && ok2 && ok3 && ok4, "Up to 16 MiB inbound queued bytes must be accepted");
                    Assert.IsFalse(ok5, "Exceeding 16 MiB cumulative inbound queue bytes must latch overflow");
                    Assert.IsTrue(ws.InboundOverflowDetected);
                }
                using (var wsPrio = new MushWsClient())
                {
                    var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                    var enqHandshake = typeof(MushWsClient).GetMethod("EnqueueHandshakeRaw", flags);
                    enqHandshake.Invoke(wsPrio, new object[] { MushProtocol.BuildCapsCommand(120, 40) });
                    // Flood 100 server pings -> pongs go to _priorityOutbound and must NEVER evict handshake `caps`!
                    for (int p = 0; p < 100; p++)
                    {
                        wsPrio.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope($@"{{""seq"":0,""ts"":{p},""t"":""ping"",""d"":{{""nonce"":""n{p}""}}}}"));
                        wsPrio.PumpMainThread();
                    }
                    Assert.IsTrue(wsPrio.TryDequeueOutbound(out string firstOut));
                    StringAssert.Contains("\"t\":\"caps\"", firstOut, "Handshake caps must survive a 100-ping flood without eviction");
                }

                // 5. SpaceFlightController caps rocks/contacts/torpedoes and TacSimMindTerminal expires & caps pending requests.
                var spaceGo = new UnityEngine.GameObject("SpaceCapTest");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<SpaceFlightController>();
                space.BuildSpaceStage(null);
                var contactsArr = JsonNode.NewArray();
                for (int c = 0; c < 200; c++)
                {
                    var co = JsonNode.NewObject();
                    co["id"] = JsonNode.FromString($"c.{c}");
                    co["label"] = JsonNode.FromString($"Ship {c}");
                    co["kind"] = JsonNode.FromString("ship");
                    co["x_km"] = JsonNode.FromDouble(c + 1);
                    co["y_km"] = JsonNode.FromDouble(0);
                    co["z_km"] = JsonNode.FromDouble(0);
                    contactsArr.Add(co);
                }
                var torpArr = JsonNode.NewArray();
                for (int t = 0; t < 150; t++)
                {
                    var to = JsonNode.NewObject();
                    to["id"] = JsonNode.FromString($"t.{t}");
                    to["x_km"] = JsonNode.FromDouble(t + 1);
                    to["y_km"] = JsonNode.FromDouble(0);
                    to["z_km"] = JsonNode.FromDouble(0);
                    torpArr.Add(to);
                }
                var floodHolo = JsonNode.NewObject();
                floodHolo["contacts"] = contactsArr;
                floodHolo["torpedoes"] = torpArr;
                space.ApplyHolotankFeed(floodHolo);
                Assert.LessOrEqual(space.LiveTorpedoCount, 64, "Torpedoes must be capped at 64");

                var mindGo = new UnityEngine.GameObject("MindPendingCap");
                mindGo.transform.SetParent(rootGo.transform, false);
                var mind = mindGo.AddComponent<TacSim.UI.TacSimMindTerminal>();
                using (var wsMind = new MushWsClient())
                {
                    mind.BindClient(wsMind);
                    for (int r = 1; r <= TacSim.UI.TacSimMindTerminal.MaxPendingRequests + 20; r++)
                        mind.RequestUnitMindTrace(r, false);
                    Assert.AreEqual(TacSim.UI.TacSimMindTerminal.MaxPendingRequests, mind.PendingRequestCount,
                        "Pending mind invokes must be bounded by MaxPendingRequests");
                    mind.PruneExpiredPendingRequests(UnityEngine.Time.realtimeSinceStartup + TacSim.UI.TacSimMindTerminal.PendingRequestTimeoutSec + 5f);
                    Assert.AreEqual(0, mind.PendingRequestCount, "Expired pending mind invokes must be pruned");
                }
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Security_P2_CLIENT_12_LoopbackParsing_UriValidation_CredentialRedaction_AndDisconnectTokenClearing()
        {
            // 1. Fail-closed loopback checks on blank, userinfo, query, and fragment edge cases.
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority(""));
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority("   "));
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority("localhost:80@evil.com"));
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority("[::1]:80@evil.com"));
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority("localhost?x=1"));
            Assert.IsFalse(MushAuthClient.IsLoopbackAuthority("localhost#frag"));
            Assert.IsFalse(MushAuthClient.IsLoopbackUrl(""));
            Assert.IsFalse(MushAuthClient.IsLoopbackUrl("localhost:80@evil.com"));
            Assert.IsFalse(MushAuthClient.IsLoopbackUrl("https://localhost:80@evil.com"));
            Assert.IsTrue(MushAuthClient.IsLoopbackAuthority("localhost:8080"));
            Assert.IsTrue(MushAuthClient.IsLoopbackAuthority("127.0.0.1:8080"));
            Assert.IsTrue(MushAuthClient.IsLoopbackAuthority("[::1]:8080"));

            // 2. NormalizeBaseUrl does NOT choose cleartext http:// for userinfo-spoofed localhost@evil.com.
            StringAssert.StartsWith("https://", MushAuthClient.NormalizeBaseUrl("localhost:80@evil.com"));

            // 3. BuildValidatedAuthEndpointUri and BuildValidatedWebSocketUri reject UserInfo, Query, and Fragment.
            Assert.Throws<System.InvalidOperationException>(() =>
                MushAuthClient.BuildValidatedAuthEndpointUri("https://user:pass@play.conclav.io", "/api/auth/login", allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushAuthClient.BuildValidatedAuthEndpointUri("https://play.conclav.io?x=1", "/api/auth/login", allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushAuthClient.BuildValidatedAuthEndpointUri("https://play.conclav.io#frag", "/api/auth/login", allowInsecureNonLoopback: false));

            Assert.Throws<System.InvalidOperationException>(() =>
                MushWsClient.BuildValidatedWebSocketUri("https://user:pass@play.conclav.io", "char1", 0, allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushWsClient.BuildValidatedWebSocketUri("https://play.conclav.io?x=1", "char1", 0, allowInsecureNonLoopback: false));
            Assert.Throws<System.InvalidOperationException>(() =>
                MushWsClient.BuildValidatedWebSocketUri("https://play.conclav.io#frag", "char1", 0, allowInsecureNonLoopback: false));

            var cleanAuthUri = MushAuthClient.BuildValidatedAuthEndpointUri("https://dev1.conclav.io", "/api/auth/login", allowInsecureNonLoopback: false);
            Assert.AreEqual("https://dev1.conclav.io/api/auth/login", cleanAuthUri.AbsoluteUri);
            var cleanWsUri = MushWsClient.BuildValidatedWebSocketUri("https://dev1.conclav.io", "char1", 42, allowInsecureNonLoopback: false);
            Assert.AreEqual("wss://dev1.conclav.io/ws?char=char1&resume=42", cleanWsUri.AbsoluteUri);

            // 4. SanitizeUrlForDisplayOrStorage strips embedded credentials, query, and fragment.
            string sanitized = MushAuthClient.SanitizeUrlForDisplayOrStorage("https://alice:SuperSecretPass@play.conclav.io:8443/path?tok=secret#frag");
            StringAssert.DoesNotContain("SuperSecretPass", sanitized);
            StringAssert.DoesNotContain("alice", sanitized);
            StringAssert.DoesNotContain("tok=secret", sanitized);
            StringAssert.DoesNotContain("#frag", sanitized);
            Assert.AreEqual("https://play.conclav.io:8443/path", sanitized);

            // 5. MushWsClient clears _activeAuth in Disconnect() AND on server `disconnect` event.
            var authField = typeof(MushWsClient).GetField("_activeAuth", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            Assert.IsNotNull(authField);
            using (var ws = new MushWsClient())
            {
                authField.SetValue(ws, new MushAuthSession { Token = "secret-tok-1", BaseHttpUrl = "https://dev1.conclav.io" });
                Assert.IsNotNull(authField.GetValue(ws));
                ws.Disconnect();
                Assert.IsNull(authField.GetValue(ws), "Disconnect() must clear _activeAuth");

                authField.SetValue(ws, new MushAuthSession { Token = "secret-tok-2", BaseHttpUrl = "https://dev1.conclav.io" });
                ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{""seq"":10,""ts"":1,""t"":""disconnect"",""d"":{""reason"":""bye""}}"));
                ws.PumpMainThread();
                Assert.IsNull(authField.GetValue(ws), "Server disconnect envelope must clear _activeAuth");
            }
        }

        [Test]
        public void Security_P2_CLIENT_13_AutoTransitGatedOnInitialLimboSync_AndNavWalkerCanceledOnDynamicPackRebuild()
        {
            var rootGo = new UnityEngine.GameObject("P2Client13CoordTest");
            try
            {
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                var stationGo = new UnityEngine.GameObject("Station");
                stationGo.transform.SetParent(rootGo.transform, false);
                coord.StationWorld = stationGo.AddComponent<StationWorldController>();
                var shipGo = new UnityEngine.GameObject("Ship");
                shipGo.transform.SetParent(rootGo.transform, false);
                coord.ShipInterior = shipGo.AddComponent<ShipInteriorController>();
                var spaceGo = new UnityEngine.GameObject("Space");
                spaceGo.transform.SetParent(rootGo.transform, false);
                coord.SpaceStage = spaceGo.AddComponent<SpaceFlightController>();
                var surfaceGo = new UnityEngine.GameObject("Surface");
                surfaceGo.transform.SetParent(rootGo.transform, false);
                coord.SurfaceStage = surfaceGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                coord.ArenaStage = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                coord.CameraRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                coord.CameraRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();
                var playerGo = new UnityEngine.GameObject("Player", typeof(UnityEngine.CharacterController));
                playerGo.transform.SetParent(rootGo.transform, false);
                coord.Player = playerGo.AddComponent<TacSim.Character.ThirdPersonPlayerController>();
                coord.Player.VisualSpawner = playerGo.AddComponent<TacSim.Character.CharacterVisualSpawner>();
                var hudGo = new UnityEngine.GameObject("Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();
                coord.EnsureSubsystems();

                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                var onFlush = typeof(TacSimGameCoordinator).GetMethod("FlushPendingRoomMovesBeforeAction", flags);
                var onRefresh = typeof(TacSimGameCoordinator).GetMethod("RefreshLoadedContentAfterSync", flags);
                var simConnField = typeof(TacSimGameCoordinator).GetField("_simulateConnectedForTests", flags);
                var awaitingInitField = typeof(TacSimGameCoordinator).GetField("_awaitingInitialRoomSync", flags);
                var navWalkerField = typeof(TacSimGameCoordinator).GetField("_navWalker", flags);
                simConnField.SetValue(coord, true);
                awaitingInitField.SetValue(coord, true);

                var dispatched = new List<string>();
                coord.OnCommandDispatched += dispatched.Add;

                // 1. Initial sync into normal station room (NOT Limbo) clears _awaitingInitialRoomSync;
                //    subsequently entering a non-atlas room with a "Ceres Freeport" exit must NOT auto-transit!
                var normalStationRoom = MushProtocol.ParseRoomView(JsonNode.Parse(
                    @"{""id"":""#101"",""name"":""Visiting Berth Gallery"",""ambient"":{""atlas"":{""pack"":""neighborhood.ceres_freeport"",""entry"":""room.c01_berth_gallery"",""surface"":""station""}}}"));
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 200f;
#endif
                onRoom.Invoke(coord, new object[] { normalStationRoom });
                Assert.AreEqual(0, dispatched.Count);

                var trapNonAtlasRoom = new MushRoomView
                {
                    Id = "#777",
                    Name = "Limbo",
                    Exits = new List<MushExitRef>
                    {
                        new MushExitRef { Name = "Ceres Freeport", Aliases = new List<string> { "ceres" } }
                    }
                };
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 201f;
#endif
                onRoom.Invoke(coord, new object[] { trapNonAtlasRoom });
                Assert.AreEqual(0, dispatched.Count, "Visiting a non-atlas room after initial sync must never trigger Limbo auto-transit");

                // 2. Non-Limbo room on initial sync also must NOT auto-transit even if it has a Ceres exit.
                awaitingInitField.SetValue(coord, true);
                var nonLimboRoom = new MushRoomView
                {
                    Id = "#778",
                    Name = "Smuggler Lounge",
                    Exits = new List<MushExitRef>
                    {
                        new MushExitRef { Name = "Ceres Freeport", Aliases = new List<string> { "ceres" } }
                    }
                };
                Assert.IsFalse(TacSimGameCoordinator.IsLimboStartRoom(nonLimboRoom));
                onRoom.Invoke(coord, new object[] { nonLimboRoom });
                Assert.AreEqual(0, dispatched.Count, "Non-Limbo initial room must never auto-transit");

                // 3. Initial sync directly into Limbo DOES perform one-shot auto-transit with `goto ` prefix.
                awaitingInitField.SetValue(coord, true);
                Assert.IsTrue(TacSimGameCoordinator.IsLimboStartRoom(trapNonAtlasRoom));
                onRoom.Invoke(coord, new object[] { trapNonAtlasRoom });
                Assert.AreEqual(1, dispatched.Count);
                Assert.AreEqual("goto ceres", dispatched[0]);

                // 4. RefreshLoadedContentAfterSync cancels _navWalker on dynamic station pack rebuild!
                dispatched.Clear();
                onRoom.Invoke(coord, new object[] { normalStationRoom });
                coord.Player.TeleportTo(coord.StationWorld.GetRoomSpawnPosition("room.c03_arrivals"), 0f);
#if !UNITY_5_3_OR_NEWER
                UnityEngine.Time.time = 210f;
#endif
                onFlush.Invoke(coord, null);
                var walker = (RoomBoundNavWalker)navWalkerField.GetValue(coord);
                Assert.IsTrue(walker.InFlight.HasValue, "Expected an in-flight navigation hop before pack rebuild");
                Assert.Greater(walker.QueuedCount, 0, "Expected queued hops before pack rebuild");

                onRefresh.Invoke(coord, null);
                Assert.IsFalse(walker.InFlight.HasValue, "Dynamic pack rebuild must cancel in-flight nav walker hop");
                Assert.AreEqual(0, walker.QueuedCount, "Dynamic pack rebuild must clear queued nav walker hops");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void DisplayOptions_DpiAwareSizingResolutionListUiScaleAndEscModalClose()
        {
            // 1. Windows DPI scale snapping (96 / 120 / 144 / 192 / invalid 0 DPI)
            Assert.AreEqual(1.0f, TacSim.UI.TacSimDisplayOptions.ComputeWindowsDpiScale(0f), 0.001f);
            Assert.AreEqual(1.0f, TacSim.UI.TacSimDisplayOptions.ComputeWindowsDpiScale(96f), 0.001f);
            Assert.AreEqual(1.25f, TacSim.UI.TacSimDisplayOptions.ComputeWindowsDpiScale(120f), 0.001f);
            Assert.AreEqual(1.5f, TacSim.UI.TacSimDisplayOptions.ComputeWindowsDpiScale(144f), 0.001f);
            Assert.AreEqual(2.0f, TacSim.UI.TacSimDisplayOptions.ComputeWindowsDpiScale(192f), 0.001f);

            // 2. Default windowed size scales up generously for 4K / ultrawide high-DPI monitors
            var size1080 = TacSim.UI.TacSimDisplayOptions.ComputeDefaultWindowSize(1920, 1080, 1.0f);
            Assert.AreEqual(1600, size1080.Width);
            Assert.AreEqual(900, size1080.Height);

            var size4k = TacSim.UI.TacSimDisplayOptions.ComputeDefaultWindowSize(3840, 2160, 1.5f);
            Assert.GreaterOrEqual(size4k.Width, 2100);
            Assert.LessOrEqual(size4k.Width, (int)(3840 * 0.90f));
            Assert.LessOrEqual(size4k.Height, (int)(2160 * 0.88f));

            var sizeUltrawide = TacSim.UI.TacSimDisplayOptions.ComputeDefaultWindowSize(5120, 1440, 1.5f);
            Assert.GreaterOrEqual(sizeUltrawide.Width, 2000);
            Assert.LessOrEqual(sizeUltrawide.Height, (int)(1440 * 0.88f));

            // 3. Resolution list deduplicates hardware modes and clamps strictly to monitor bounds
            var choices = TacSim.UI.TacSimDisplayOptions.BuildResolutionChoices(
                new[]
                {
                    new UnityEngine.Resolution { width = 2560, height = 1440 },
                    new UnityEngine.Resolution { width = 2560, height = 1440 },
                    new UnityEngine.Resolution { width = 7680, height = 4320 }
                },
                monitorWidth: 2560,
                monitorHeight: 1440);
            Assert.AreEqual(2560, choices[0].Width);
            Assert.AreEqual(1440, choices[0].Height);
            Assert.IsFalse(choices.Exists(c => c.Width > 2560 || c.Height > 1440));
            Assert.AreEqual(1, choices.FindAll(c => c.Width == 2560 && c.Height == 1440).Count);

            // 4. UI Scale updates scaled reference resolution cleanly
            UnityEngine.Vector2 ref150 = TacSim.UI.TacSimDisplayOptions.ComputeScaledReferenceResolution(1.5f);
            Assert.AreEqual(1280f, ref150.x, 0.5f);
            Assert.AreEqual(720f, ref150.y, 0.5f);

            // 5. Hud wires DisplayOptions modal and ESC closes it before lower layers
            var go = new UnityEngine.GameObject("HudDisplayOptionsTest");
            try
            {
                var hud = go.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.DisplayOptions, "BuildHud must wire TacSimDisplayOptions");
                Assert.IsFalse(hud.DisplayOptions.IsOpen);

                hud.DisplayOptions.SetOpen(true);
                Assert.IsTrue(hud.DisplayOptions.IsOpen);
                Assert.IsTrue(hud.TryCloseTopmostModal(), "ESC / TryCloseTopmostModal must close DisplayOptions first");
                Assert.IsFalse(hud.DisplayOptions.IsOpen);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(go);
            }
        }

        [Test]
        public void SolarSystemOrrery_DecodesSkyChartComputesLogScaleLagRings3DProjectionBridgeHoloAndEscClose()
        {
            const string skyChartJson = @"{
                ""schema"": { ""id"": ""sky.chart"", ""version"": 1, ""shape"": ""grid"" },
                ""chart"": {
                    ""date"": ""2351-05-13"",
                    ""ic_now"": 74152540800,
                    ""model"": ""ephemeris"",
                    ""rate_permille"": 1000,
                    ""vantage"": { ""port"": ""ceres"", ""body"": ""ceres"", ""hull"": ""dray"", ""payload_kg"": 42000, ""occupants"": 2 },
                    ""scale"": { ""kind"": ""log"", ""au_max"": 10.0, ""r0_au"": 0.3 },
                    ""bodies"": [
                        { ""id"": ""earth"", ""label"": ""Earth"", ""glyph"": ""E"", ""parent"": null, ""r_au"": 1.010, ""lon_deg"": 232.0, ""lat_deg"": 0.0, ""x_au"": -0.62, ""y_au"": -0.80, ""z_au"": 0.0, ""period_days"": 365.25, ""here"": false, ""from_here_au"": 2.412, ""lag_s"": 1203.0 },
                        { ""id"": ""luna"", ""label"": ""Luna"", ""glyph"": ""l"", ""parent"": ""earth"", ""r_au"": 1.012, ""lon_deg"": 232.2, ""lat_deg"": 0.1, ""x_au"": -0.62, ""y_au"": -0.80, ""z_au"": 0.0, ""period_days"": 365.25, ""here"": false, ""from_here_au"": 2.414, ""lag_s"": 1204.0 },
                        { ""id"": ""mars"", ""label"": ""Mars"", ""glyph"": ""M"", ""parent"": null, ""r_au"": 1.524, ""lon_deg"": 120.0, ""lat_deg"": 1.8, ""x_au"": -0.76, ""y_au"": 1.32, ""z_au"": 0.05, ""period_days"": 687.0, ""here"": false, ""from_here_au"": 1.482, ""lag_s"": 739.0 },
                        { ""id"": ""ceres"", ""label"": ""Ceres"", ""glyph"": ""C"", ""parent"": null, ""r_au"": 2.768, ""lon_deg"": 45.0, ""lat_deg"": 9.4, ""x_au"": 1.95, ""y_au"": 1.95, ""z_au"": 0.45, ""period_days"": 1680.0, ""here"": true, ""from_here_au"": null, ""lag_s"": null }
                    ],
                    ""routes"": [
                        {
                            ""to"": ""stickney"",
                            ""venue"": ""stickney_yards"",
                            ""body"": ""mars"",
                            ""legs"": [ { ""from"": ""ceres"", ""to"": ""mars"" } ],
                            ""loaded"": { ""total_s"": 194400, ""burn_t"": 18.4, ""crew_days"": 4.5 },
                            ""empty"": { ""total_s"": 151200, ""burn_t"": 12.1, ""crew_days"": 3.5 },
                            ""direct"": { ""km"": 221704044, ""total_s"": 180000, ""burn_t"": 17.0 }
                        }
                    ],
                    ""window"": [
                        {
                            ""to"": ""stickney"",
                            ""now"": { ""total_s"": 194400, ""burn_t"": 18.4, ""crew_days"": 4.5 },
                            ""fastest"": { ""offset_days"": 42, ""total_s"": 158400, ""burn_t"": 14.2 },
                            ""cheapest"": { ""offset_days"": 48, ""total_s"": 162000, ""burn_t"": 13.8 },
                            ""worst"": { ""offset_days"": 210, ""total_s"": 312000, ""burn_t"": 31.5 }
                        }
                    ]
                }
            }";

            var frame = TacSim.Space.TacSimSolarChartFrame.FromJson(TacSim.Core.JsonNode.Parse(skyChartJson));
            Assert.IsNotNull(frame);
            Assert.AreEqual("2351-05-13", frame.Date);
            Assert.AreEqual(4, frame.Bodies.Count);
            Assert.AreEqual(1, frame.Routes.Count);
            Assert.AreEqual(1, frame.Windows.Count);
            Assert.AreEqual("ceres", frame.FindHereBody()?.Id);

            // Logarithmic radial scale matches web/src/lib/chart.ts formula exactly
            double fCeres = TacSim.Space.TacSimSolarChartFrame.RadialFraction(2.768, 10.0, 0.3);
            double expected = System.Math.Log(1.0 + 2.768 / 0.3) / System.Math.Log(1.0 + 10.0 / 0.3);
            Assert.AreEqual(expected, fCeres, 1e-6);

            // Light-lag ring sampling & 3D tilt elevation stems
            var lag12 = frame.ComputeLagRingUv(12, 1.0f, UnityEngine.Vector2.zero, tiltDeg: 38f, yawDeg: 0f, steps: 48);
            Assert.AreEqual(49, lag12.Count);

            var ceres = frame.FindBody("ceres");
            var uvTilted = frame.ProjectBodyToUv(ceres, 1.0f, UnityEngine.Vector2.zero, tiltDeg: 38f, yawDeg: 0f, out var footTilted);
            Assert.Greater(uvTilted.y, footTilted.y, "Inclined Ceres (+9.4 deg lat) must lift above ecliptic plane in 3D tilt mode");

            // Hit-testing selects nearest primary body cleanly
            var hit = frame.FindNearestPrimaryBody(uvTilted, 1.0f, UnityEngine.Vector2.zero, tiltDeg: 38f, yawDeg: 0f);
            Assert.AreEqual("ceres", hit?.Id);

            // Route dossier formats passage, burn, and 1-year launch window
            string dossier = frame.BuildBodyAndRouteDossier(frame.FindBody("mars"), frame.Routes[0]);
            Assert.IsTrue(dossier.Contains("STICKNEY"));
            Assert.IsTrue(dossier.Contains("1-YEAR LAUNCH WINDOW"));
            Assert.IsTrue(dossier.Contains("54.0 h"));

            // HUD SolarMap modal integration + Bridge 3D Holo-Tank solar system projection
            var rootGo = new UnityEngine.GameObject("SolarOrreryHudTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.SolarMap);

                var cmds = new List<string>();
                hud.SolarMap.OnCommandRequested += cmds.Add;
                hud.SolarMap.ApplyChartFrame(frame);
                Assert.AreEqual("stickney", hud.SolarMap.SelectedPortId);

                hud.SolarMap.SetOpen(true);
                Assert.IsTrue(hud.SolarMap.IsOpen);
                hud.SolarMap.DispatchPlotCourse();
                hud.SolarMap.DispatchQueueDepart();
                Assert.Contains("+ship/plan stickney", cmds);
                Assert.Contains("+ship/order depart stickney", cmds);

                Assert.IsTrue(hud.TryCloseTopmostModal(), "ESC must close Solar System Orrery");
                Assert.IsFalse(hud.SolarMap.IsOpen);

                // Bridge 3D Holo-Tank projects solar system when docked
                var shipGo = new UnityEngine.GameObject("BridgeHoloSolarTest");
                shipGo.transform.SetParent(rootGo.transform, false);
                var ship = shipGo.AddComponent<TacSim.World.ShipInteriorController>();
                ship.BuildInterior(null);
                ship.ApplySolarChartFrame(frame);
                Assert.IsTrue(ship.IsHoloShowingSolarSystem, "Docked Bridge Holo-Tank must display live Solar System projection");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void MmorpgHud_UnitFramesMinimapTabbedChatHistoryAndNumberedHotbar()
        {
            // 1. Tabbed chat classification routes Combat / System / Comms / Local cleanly
            Assert.AreEqual(TacSim.UI.ChatLogTab.Combat, TacSim.UI.TacSimHudController.ClassifyLogCategory("<color=#FFBB55>[TAC]</color> Lock acquired on R2."));
            Assert.AreEqual(TacSim.UI.ChatLogTab.System, TacSim.UI.TacSimHudController.ClassifyLogCategory("<color=#88CCFF>[NET]</color> Connected."));
            Assert.AreEqual(TacSim.UI.ChatLogTab.Comms, TacSim.UI.TacSimHudController.ClassifyLogCategory("<color=#66BBFF>[COMMS]</color> Welcome to Ceres Freeport."));
            Assert.AreEqual(TacSim.UI.ChatLogTab.Local, TacSim.UI.TacSimHudController.ClassifyLogCategory("Dockmaster nods toward Berth 3."));

            var rootGo = new UnityEngine.GameObject("MmorpgHudCoreTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.UnitFrames, "BuildHud must wire WoW/EQ Player & Target Unit Frames");
                Assert.IsNotNull(hud.Minimap, "BuildHud must wire Unified Corner Tactical Minimap");

                // 2. Numbered 1-9 action hotbar records slots and triggers commands cleanly
                var dispatched = new List<string>();
                hud.OnCommandSubmitted += dispatched.Add;
                hud.RefreshContextualQuickActions("room.c07_exchange", isSeatedAtHelm: false);
                Assert.GreaterOrEqual(hud.QuickActionSlotCount, 3);
                Assert.IsTrue(hud.TriggerQuickActionSlot(0));
                Assert.AreEqual("ui:open_exchange", dispatched[0]);

                // 3. Command history recall (Up/Down)
                hud.RecordCommandHistory("look");
                hud.RecordCommandHistory("+ship");
                Assert.AreEqual("+ship", hud.RecallCommandHistory(-1));
                Assert.AreEqual("look", hud.RecallCommandHistory(-1));
                Assert.AreEqual("+ship", hud.RecallCommandHistory(+1));

                // 4. Tab switching & log filtering
                hud.AppendLogLine("[TAC] Fired laser pulse.");
                hud.AppendLogLine("[NET] Sync complete.");
                hud.SelectChatLogTab(TacSim.UI.ChatLogTab.Combat);
                Assert.AreEqual(TacSim.UI.ChatLogTab.Combat, hud.ActiveChatTab);

                // 5. Player & Target unit frames update and clear cleanly
                hud.UnitFrames.UpdatePlayerFrame(
                    "STN", UnityEngine.Color.cyan, "Phoenix", "Ceres Freeport",
                    0.5f, "HOLD 50/100 t", UnityEngine.Color.cyan,
                    0.75f, "FUEL 60/80 t", UnityEngine.Color.yellow, "CREDITS: 12,000 cr");
                Assert.AreEqual("Phoenix", hud.UnitFrames.PlayerTitleText);
                Assert.IsFalse(hud.UnitFrames.IsTargetVisible);

                hud.UnitFrames.SetTarget("NPC", UnityEngine.Color.green, "Dockmaster Voss", "Port Authority", 1f, "IN RANGE", UnityEngine.Color.green);
                Assert.IsTrue(hud.UnitFrames.IsTargetVisible);
                Assert.AreEqual("Dockmaster Voss", hud.UnitFrames.TargetTitleText);
                hud.UnitFrames.ClearTarget();
                Assert.IsFalse(hud.UnitFrames.IsTargetVisible);

                // 6. Minimap renders Station blueprint, shifts mid-left when tactical panel busy, and handles click-to-navigate
                var stGo = new UnityEngine.GameObject("MinimapStationTest");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null);
                hud.Minimap.BindWorldSources(station, null, null, null, null);
                hud.Minimap.SyncMinimap(TacSim.Core.TacSimActiveStage.StationWalk, UnityEngine.Vector3.zero, 90f, rightCornerOccupied: false, forceRedraw: true);
                Assert.IsFalse(hud.Minimap.IsDockedMidLeft);
                Assert.IsTrue(hud.Minimap.HeaderText.Contains("DECK"));

                hud.Minimap.SyncMinimap(TacSim.Core.TacSimActiveStage.ShipHelmSeated, UnityEngine.Vector3.zero, 45f, rightCornerOccupied: true, forceRedraw: true);
                Assert.IsTrue(hud.Minimap.IsDockedMidLeft, "Minimap must shift cleanly to Mid-Left when top-right combat/docking panel is open");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void TacticalHudAndSheet_CharacterSheetTabsAvatarSwapAnd3DBracketProjection()
        {
            // 1. Pure-math 3D screen-space bracket projection: on-screen, off-screen clamp, behind-camera perimeter mirror
            TacSim.UI.TacSimTacticalBrackets.ComputeCanvasPositionFromScreenPoint(
                new UnityEngine.Vector3(960f, 540f, 25f), 1920f, 1080f, new UnityEngine.Vector2(1920f, 1080f), 50f,
                out var centerPos, out bool behindCenter, out bool clampedCenter);
            Assert.IsFalse(behindCenter);
            Assert.IsFalse(clampedCenter);
            Assert.AreEqual(0f, centerPos.x, 1f);
            Assert.AreEqual(0f, centerPos.y, 1f);

            TacSim.UI.TacSimTacticalBrackets.ComputeCanvasPositionFromScreenPoint(
                new UnityEngine.Vector3(3500f, 540f, 25f), 1920f, 1080f, new UnityEngine.Vector2(1920f, 1080f), 50f,
                out var offRightPos, out bool behindRight, out bool clampedRight);
            Assert.IsFalse(behindRight);
            Assert.IsTrue(clampedRight, "Off-screen target must clamp inside canvas margin");
            Assert.AreEqual(910f, offRightPos.x, 1f);

            TacSim.UI.TacSimTacticalBrackets.ComputeCanvasPositionFromScreenPoint(
                new UnityEngine.Vector3(1200f, 700f, -15f), 1920f, 1080f, new UnityEngine.Vector2(1920f, 1080f), 50f,
                out var behindPos, out bool isBehind, out bool clampedBehind);
            Assert.IsTrue(isBehind, "Negative screen Z must flag target as behind camera");
            Assert.IsTrue(clampedBehind, "Behind-camera target must clamp to viewport perimeter");
            Assert.Less(behindPos.x, 0f, "Target behind-right should mirror to left edge indicator");

            var rootGo = new UnityEngine.GameObject("TacticalHudAndSheetTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.CharacterSheet, "BuildHud must wire [C] Character/Ship Sheet modal");
                Assert.IsNotNull(hud.TacticalBrackets, "BuildHud must wire pooled 3D Tactical Brackets layer");

                var camGo = new UnityEngine.GameObject("MainCamera");
                camGo.tag = "MainCamera";
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.AddComponent<UnityEngine.Camera>();

                var spaceGo = new UnityEngine.GameObject("SpaceTest");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<TacSim.Space.SpaceFlightController>();
                space.BuildSpaceStage(null);
                space.ApplyHolotankFeed(JsonNode.Parse(@"{
                    ""own"": { ""id"": ""own1"", ""x_km"": 0, ""y_km"": 0, ""z_km"": 0 },
                    ""contacts"": [{ ""id"": ""V1"", ""label"": ""Vanguard"", ""x_km"": 12, ""y_km"": 4, ""z_km"": 0, ""iff"": ""hostile"" }],
                    ""rocks"": [{ ""id"": ""A"", ""x_km"": -5, ""y_km"": 2, ""z_km"": 0 }]
                }"));
                space.ApplyCockpitFeed(JsonNode.Parse(@"{
                    ""throttle_permille"": 500,
                    ""emcon"": ""ghost"",
                    ""fire"": {
                        ""lock"": { ""ship_id"": ""V1"", ""label"": ""Vanguard"", ""range_m"": 12600, ""facet"": ""left"" },
                        ""mounts"": [{ ""id"": ""L1"", ""family"": ""laser"", ""state"": ""ready"", ""damage_at_lock"": 42 }]
                    }
                }"));

                var surfaceGo = new UnityEngine.GameObject("SurfaceTest");
                surfaceGo.transform.SetParent(rootGo.transform, false);
                var surface = surfaceGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surface.BuildSurfaceStage(null, "occator_rim");
                surface.EnterSurfaceMode();

                var arenaGo = new UnityEngine.GameObject("ArenaTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(null, null);
                arena.State.Callsign = "R1";
                arena.State.ArmorFrontPct = 82;
                arena.State.ArmorLeftPct = 44;
                arena.State.ArmorRearPct = 20;
                arena.State.Heat = 650;
                arena.State.HeatMax = 1000;
                arena.State.LockedDesignator = "B1";
                arena.State.LockHitPercent = 73;
                arena.State.Contacts.Add(new TacSim.Arena.ArenaContactState
                {
                    Designator = "B1",
                    ClassLabel = "line_tank",
                    Party = "B",
                    Friendly = false,
                    CellX = 12,
                    CellY = 14,
                    RangeM = 480f
                });

                var trade = new TacSim.World.TacSimTradeAndShipState();

                // 2. Character/Ship Sheet tabs, live avatar swap event, live mount readout & ESC close
                hud.CharacterSheet.BindSources(trade, space, surface, arena);
                hud.CharacterSheet.SetPilotContext("Captain Vance", "Ceres Freeport", "Phoenix", TacSim.Core.TacSimActiveStage.ShipHelmSeated);
                hud.CharacterSheet.SetOpen(true);
                Assert.IsTrue(hud.CharacterSheet.IsOpen);
                Assert.IsTrue(hud.IsModalOpen, "Open CharacterSheet must report IsModalOpen=true");
                StringAssert.Contains("Captain Vance", hud.CharacterSheet.PilotSummarySnapshot);

                TacSim.Core.PlayerAvatarPreset? swapped = null;
                hud.CharacterSheet.OnAvatarPresetSelected += p => swapped = p;
                hud.CharacterSheet.ApplyAvatarChoice(TacSim.Core.PlayerAvatarPreset.CyberpunkStreetFighterMale, notify: true);
                Assert.AreEqual(TacSim.Core.PlayerAvatarPreset.CyberpunkStreetFighterMale, swapped);

                hud.CharacterSheet.SelectTab(TacSim.UI.CharacterSheetTab.SpaceFit);
                Assert.AreEqual(TacSim.UI.CharacterSheetTab.SpaceFit, hud.CharacterSheet.ActiveTab);
                StringAssert.Contains("L1", hud.CharacterSheet.SpaceMountsSnapshot);
                StringAssert.Contains("READY", hud.CharacterSheet.SpaceMountsSnapshot);

                Assert.IsTrue(hud.TryCloseTopmostModal(), "ESC must close CharacterSheet modal");
                Assert.IsFalse(hud.CharacterSheet.IsOpen);

                // 3. 3D Tactical Brackets & Schematics across Station, Space, Surface, and Arena stages
                hud.TacticalBrackets.BindControllers(space, surface, arena);

                hud.TacticalBrackets.SyncBrackets(TacSim.Core.TacSimActiveStage.StationWalk, cam);
                Assert.AreEqual(0, hud.TacticalBrackets.ActiveBracketCount);
                Assert.IsFalse(hud.TacticalBrackets.IsArenaArmorSchematicVisible);
                Assert.IsFalse(hud.TacticalBrackets.IsSurfaceCompassVisible);

                hud.TacticalBrackets.SyncBrackets(TacSim.Core.TacSimActiveStage.ShipHelmSeated, cam);
                Assert.GreaterOrEqual(hud.TacticalBrackets.ActiveBracketCount, 1, "Orbital space stage must project contact/rock brackets");

                hud.TacticalBrackets.SyncBrackets(TacSim.Core.TacSimActiveStage.PlanetarySurface, cam);
                Assert.IsTrue(hud.TacticalBrackets.IsSurfaceCompassVisible, "Planetary surface stage must show rover compass strip");
                StringAssert.Contains("HDG", hud.TacticalBrackets.SurfaceCompassSnapshot);

                hud.TacticalBrackets.SyncBrackets(TacSim.Core.TacSimActiveStage.SimulatorArena, cam);
                Assert.IsTrue(hud.TacticalBrackets.IsArenaArmorSchematicVisible, "Simulator arena stage must show 4-quadrant directional armor schematic");
                Assert.GreaterOrEqual(hud.TacticalBrackets.ActiveBracketCount, 1, "Simulator arena stage must project enemy unit brackets");
                StringAssert.Contains("LOCK B1", hud.TacticalBrackets.ArenaArmorSummarySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Wayfinder3DAndOpsDirectory_PlotsMultiDeckRibbonDirectoryPresenceAndArrival()
        {
            // 1. Pure-math 3D ribbon vertex chain builder
            var pts = TacSim.World.TacSimWayfinder3D.BuildRouteWorldPoints(
                new UnityEngine.Vector3(0f, 6f, 0f),
                new List<UnityEngine.Vector3> { new UnityEngine.Vector3(10f, 6f, 0f), new UnityEngine.Vector3(10f, 0f, 12f) },
                floorLiftM: 0.3f);
            Assert.AreEqual(3, pts.Count);
            Assert.AreEqual(6.3f, pts[0].y, 0.01f);
            Assert.AreEqual(0.3f, pts[2].y, 0.01f);

            var rootGo = new UnityEngine.GameObject("WayfinderAndOpsDirTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.OpsDirectory, "BuildHud must wire [N] Ops Directory & Wayfinder Banner");

                var stGo = new UnityEngine.GameObject("StationWf");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                var shipGo = new UnityEngine.GameObject("ShipWf");
                shipGo.transform.SetParent(rootGo.transform, false);
                var ship = shipGo.AddComponent<TacSim.World.ShipInteriorController>();
                ship.BuildInterior(null);

                var wfGo = new UnityEngine.GameObject("WayfinderWf");
                wfGo.transform.SetParent(rootGo.transform, false);
                var wf = wfGo.AddComponent<TacSim.World.TacSimWayfinder3D>();
                wf.EnsureVisuals();

                var trade = new TacSim.World.TacSimTradeAndShipState();
                hud.OpsDirectory.BindSources(station, ship, wf, trade, null, null, null);

                // 2. Plot 3D station wayfinder trail from arrivals to exchange
                string startRoom = station.ActiveRoomEntryId;
                string targetRoom = null;
                foreach (var r in station.Pack.Rooms.Values)
                {
                    if (!string.Equals(r.EntryId, startRoom, System.StringComparison.OrdinalIgnoreCase) &&
                        station.FindExitPath(startRoom, r.EntryId).Count > 0)
                    {
                        targetRoom = r.EntryId;
                        break;
                    }
                }
                Assert.IsNotNull(targetRoom, "Ceres Freeport pack must have a reachable target room");
                Assert.IsTrue(wf.PlotStationRoute(station, startRoom, targetRoom, station.GetRoomSpawnPosition(startRoom)));
                Assert.IsTrue(wf.HasActiveRoute);
                Assert.GreaterOrEqual(wf.RenderedPointCount, 2);
                Assert.Greater(wf.DistanceMeters, 0.5f);

                // 3. Top-center Wayfinder Compass Banner reflects active trail & hides in Arena
                hud.OpsDirectory.SyncWayfinderBanner(TacSim.Core.TacSimActiveStage.StationWalk);
                Assert.IsTrue(hud.OpsDirectory.IsWayfinderBannerVisible);
                StringAssert.Contains("HOP", hud.OpsDirectory.WayfinderBannerText);

                hud.OpsDirectory.SyncWayfinderBanner(TacSim.Core.TacSimActiveStage.SimulatorArena);
                Assert.IsFalse(hud.OpsDirectory.IsWayfinderBannerVisible, "Wayfinder banner must yield to Arena scoreboard");

                // 4. [N] Directory modal populates rooms, room presence cast, and Active Ops summary, and closes on ESC
                var rv = new MushRoomView { Name = "Arrivals" };
                rv.Contents.Add(new MushObjRef { Id = "#10", Name = "Dockmaster Voss" });
                rv.NpcIds.Add("#10");
                rv.Contents.Add(new MushObjRef { Id = "#20", Name = "Pilot Kira" });
                rv.PlayerIds.Add("#20");
                hud.OpsDirectory.UpdateLiveRoom(rv, TacSim.Core.TacSimActiveStage.StationWalk);
                hud.OpsDirectory.SetOpen(true);
                Assert.IsTrue(hud.OpsDirectory.IsOpen);
                Assert.IsTrue(hud.IsModalOpen);
                Assert.GreaterOrEqual(hud.OpsDirectory.VisibleDirectoryRowCount, 2);

                hud.OpsDirectory.SelectTab(TacSim.UI.OpsDirectoryTab.RoomPresence);
                Assert.AreEqual(2, hud.OpsDirectory.VisiblePresenceRowCount);

                hud.OpsDirectory.SelectTab(TacSim.UI.OpsDirectoryTab.ActiveOps);
                StringAssert.Contains(wf.TargetDisplayName, hud.OpsDirectory.OpsSummarySnapshot);

                Assert.IsTrue(hud.TryCloseTopmostModal(), "ESC must close OpsDirectory modal");
                Assert.IsFalse(hud.OpsDirectory.IsOpen);

                // 5. Arrival auto-clears the 3D wayfinder ribbon and fires OnArrivedAtDestination
                string arrivedName = null;
                wf.OnArrivedAtDestination += n => arrivedName = n;
                station.HighlightActiveRoom(targetRoom);
                wf.SyncProgress(TacSim.Core.TacSimActiveStage.StationWalk, station, ship, station.GetRoomSpawnPosition(targetRoom), 0.016f);
                Assert.IsFalse(wf.HasActiveRoute, "Arriving in destination room must auto-clear active wayfinder trail");
                Assert.IsNotNull(arrivedName);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void CombatSpaceSurfaceVfxAndCallouts_TacticalGridRoverSplineArenaLockBeamAndPopups()
        {
            var rootGo = new UnityEngine.GameObject("CombatVfxAndCalloutsTest");
            try
            {
                // 1. SpaceFlightController 3D Homeworld/EVE tactical reference plane, drop-lines & lock tether
                var spaceGo = new UnityEngine.GameObject("SpaceVfxTest");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<TacSim.Space.SpaceFlightController>();
                space.BuildSpaceStage(null);
                Assert.IsTrue(space.IsTacticalGridVisible, "Orbital tactical reference plane must build active");
                Assert.AreEqual(0, space.ActiveDropLineCount);

                space.ApplyHolotankFeed(JsonNode.Parse(@"{
                    ""own"": { ""id"": ""own1"", ""x_km"": 0, ""y_km"": 0, ""z_km"": 0 },
                    ""contacts"": [{ ""id"": ""C1"", ""label"": ""Corsair"", ""x_km"": 18, ""y_km"": -6, ""z_km"": 4, ""iff"": ""hostile"", ""locked"": true }],
                    ""rocks"": [{ ""id"": ""A"", ""x_km"": 5, ""y_km"": 7, ""z_km"": -2 }]
                }"));
                Assert.GreaterOrEqual(space.ActiveDropLineCount, 2, "Perceived orbital contacts & rocks must draw vertical drop-lines to tactical plane");
                Assert.IsTrue(space.HasActiveLockTether, "Locked contact must render 3D trajectory lock tether");

                space.SetTacticalGridVisible(false);
                Assert.IsFalse(space.IsTacticalGridVisible);

                // 2. OrbitalTacticalHigh camera mode configuration
                var camGo = new UnityEngine.GameObject("CamRigTest", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var rig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                rig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();
                rig.SetMode(TacSim.Character.CameraRigMode.OrbitalTacticalHigh);
                Assert.AreEqual(TacSim.Character.CameraRigMode.OrbitalTacticalHigh, rig.Mode);
                Assert.AreEqual(8000f, rig.ControlledCamera.farClipPlane, 1f);

                // 3. SurfaceMiningController 3D rover route spline over live site.map leg
                var surfGo = new UnityEngine.GameObject("SurfaceSplineTest");
                surfGo.transform.SetParent(rootGo.transform, false);
                var surf = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surf.BuildSurfaceStage(null, "occator_rim");
                surf.EnterSurfaceMode();
                surf.ApplySiteMapFrame(JsonNode.Parse(@"{
                    ""width"": 8,
                    ""height"": 8,
                    ""site"": {
                        ""site"": ""Occator Rim"",
                        ""grid"": { ""x0"": 0, ""y0"": 0, ""span"": 1, ""cell_cm"": 1000 },
                        ""rigs"": [{
                            ""slot"": 1,
                            ""cell"": [1, 1],
                            ""leg"": {
                                ""started_ic"": 100,
                                ""waypoints"": [{""x_cm"": 1500, ""y_cm"": 1500, ""at_s"": 0}, {""x_cm"": 6500, ""y_cm"": 6500, ""at_s"": 10}]
                            }
                        }]
                    },
                    ""rows"": [""........"",""........"",""........"",""........"",""........"",""........"",""........"",""........""],
                    ""you"": [1, 1]
                }"));
                Assert.IsTrue(surf.IsRouteSplineVisible, "Active rover drive leg must render terrain-hugging 3D route spline");
                Assert.GreaterOrEqual(surf.RouteSplinePointCount, 2);

                // 4. OocArenaController 3D weapon range ring & target lock laser beam
                var arenaGo = new UnityEngine.GameObject("ArenaVfxTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(null, null);
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""seated"": true, ""match"": 42, ""callsign"": ""R1"", ""party"": ""R"", ""unit_type"": ""tacsim.unit.line_tank"",
                    ""x_cm"": 12000, ""y_cm"": 12000, ""lock_designator"": ""B1"", ""lock_range_m"": 260, ""lock_percent"": 78,
                    ""contacts"": [{ ""designator"": ""B1"", ""party"": ""B"", ""relation"": ""hostile"", ""cell"": [18, 18], ""range_m"": 260 }]
                }"));
                arena.SyncArenaTacticalVisuals();
                Assert.IsTrue(arena.IsArenaRangeRingVisible, "Live arena player rig must render 3D tactical range ring");
                Assert.IsTrue(arena.IsArenaLockBeamActive, "Locked arena contact must render 3D target lock laser line");

                // 5. Pooled Floating Combat & Telemetry Callouts lifecycle
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.FloatingCallouts);
                hud.FloatingCallouts.ShowCallout("TORPEDO SALVO AWAY", UnityEngine.Color.yellow, durationSec: 1.0f);
                Assert.AreEqual(1, hud.FloatingCallouts.ActivePopupCount);
                Assert.AreEqual("TORPEDO SALVO AWAY", hud.FloatingCallouts.LastShownText);
                hud.FloatingCallouts.StepAnimation(0.5f);
                Assert.AreEqual(1, hud.FloatingCallouts.ActivePopupCount);
                hud.FloatingCallouts.StepAnimation(0.6f);
                Assert.AreEqual(0, hud.FloatingCallouts.ActivePopupCount, "Expired callouts must cleanly deactivate into pool");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void CommandPaletteAndKeybindMacros_FuzzyMatchFastTrailSolarInspectAndCustomMacros()
        {
            // 1. Multi-token & ordered subsequence fuzzy matching
            Assert.Greater(TacSim.UI.TacSimCommandPalette.ScoreFuzzyMatch("solar port chart & plan: stickney yards", "stickney"), 0);
            Assert.Greater(TacSim.UI.TacSimCommandPalette.ScoreFuzzyMatch("ship active sailing orders +ship/orders", "shor"), 0);
            Assert.Greater(TacSim.UI.TacSimCommandPalette.ScoreFuzzyMatch("world step out of character +ooc", "+ooc"), 0);
            Assert.AreEqual(-1, TacSim.UI.TacSimCommandPalette.ScoreFuzzyMatch("ceres freeport exchange", "titan"));

            var rootGo = new UnityEngine.GameObject("CommandPaletteAndMacrosTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.CommandPalette, "BuildHud must wire Ctrl+K Command Palette");
                Assert.IsNotNull(hud.KeybindHelp, "BuildHud must wire F1 Keybind Help & Custom Macros");

                var stGo = new UnityEngine.GameObject("StationPal");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                var shipGo = new UnityEngine.GameObject("ShipPal");
                shipGo.transform.SetParent(rootGo.transform, false);
                var ship = shipGo.AddComponent<TacSim.World.ShipInteriorController>();
                ship.BuildInterior(null);

                hud.CommandPalette.BindWorldSources(station, ship);
                hud.CommandPalette.SetActiveStage(TacSim.Core.TacSimActiveStage.StationWalk);

                // 2. Solar port search + execution
                string inspectedPort = null;
                hud.CommandPalette.OnInspectSolarPortRequested += p => inspectedPort = p;
                hud.CommandPalette.SetOpen(true);
                Assert.IsTrue(hud.CommandPalette.IsOpen);
                Assert.IsTrue(hud.IsModalOpen);
                hud.CommandPalette.SetQuery("stickney");
                Assert.GreaterOrEqual(hud.CommandPalette.FilteredMatchCount, 1);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual("stickney", inspectedPort);
                Assert.IsFalse(hud.CommandPalette.IsOpen, "Executing a palette entry must close the palette");

                // 3. 3D Station Wayfinder trail dispatch from palette
                string plottedRoom = null;
                hud.CommandPalette.OnPlotStationRoomRequested += r => plottedRoom = r;
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("3d trail");
                Assert.GreaterOrEqual(hud.CommandPalette.FilteredMatchCount, 1);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.IsFalse(string.IsNullOrEmpty(plottedRoom));

                // 4. Verified +ooc command search + execution
                string sentCmd = null;
                hud.CommandPalette.OnCommandRequested += c => sentCmd = c;
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("+ooc");
                Assert.GreaterOrEqual(hud.CommandPalette.FilteredMatchCount, 1);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual("+ooc", sentCmd);

                // 5. F1 Keybinds & F5-F8 Custom Macros
                hud.KeybindHelp.SetOpen(true);
                Assert.IsTrue(hud.KeybindHelp.IsOpen);
                Assert.IsTrue(hud.IsModalOpen);
                Assert.AreEqual("+ship", hud.KeybindHelp.GetMacroCommand(0));
                hud.KeybindHelp.SetMacroSlot(0, "Hold Check", "+ship/hold", persist: false);
                Assert.AreEqual("Hold Check", hud.KeybindHelp.GetMacroLabel(0));
                Assert.AreEqual("+ship/hold", hud.KeybindHelp.GetMacroCommand(0));

                string macroLbl = null;
                string macroCmd = null;
                hud.KeybindHelp.OnMacroTriggered += (l, c) => { macroLbl = l; macroCmd = c; };
                Assert.IsTrue(hud.KeybindHelp.TriggerMacroSlot(0));
                Assert.AreEqual("Hold Check", macroLbl);
                Assert.AreEqual("+ship/hold", macroCmd);

                // 6. ESC closes CommandPalette before KeybindHelp
                hud.CommandPalette.SetOpen(true);
                Assert.IsTrue(hud.TryCloseTopmostModal());
                Assert.IsFalse(hud.CommandPalette.IsOpen);
                Assert.IsTrue(hud.KeybindHelp.IsOpen);
                Assert.IsTrue(hud.TryCloseTopmostModal());
                Assert.IsFalse(hud.KeybindHelp.IsOpen);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void SocialMailComsysAndQuestJournal_LivePanelsWhoMailChannelsAndPinnedObjectiveTracker()
        {
            // 1. Protocol builders & quick-mail validation
            string whoReq = MushProtocol.BuildWhoQueryCommand(7UL);
            StringAssert.Contains("\"who\"", whoReq);
            string mailReq = MushProtocol.BuildMailListQueryCommand(8UL, "inbox");
            StringAssert.Contains("\"mail_list\"", mailReq);
            StringAssert.Contains("\"inbox\"", mailReq);

            Assert.IsTrue(TacSim.UI.TacSimSocialAndMailModal.TryBuildQuickMailCommand("Voss", "Cargo Eta", "Arriving 14:00 UTC", out string mailCmd, out _));
            Assert.AreEqual("@mail/quick Voss=Cargo Eta/Arriving 14:00 UTC", mailCmd);
            Assert.IsFalse(TacSim.UI.TacSimSocialAndMailModal.TryBuildQuickMailCommand("Voss", "Bad/Subj", "Body", out _, out _));

            var rootGo = new UnityEngine.GameObject("SocialAndJournalTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.SocialModal, "BuildHud must wire [O] Social/Mail/Comsys modal");
                Assert.IsNotNull(hud.QuestJournal, "BuildHud must wire [J] Quest/Passage/Survey/Claims journal");

                // 2. Live WHO, Mail & Comsys ingestion via MushWsClient events
                using (var ws = new MushWsClient())
                {
                    List<MushWhoEntry> gotWho = null;
                    int gotMailNotice = -1;
                    ws.OnWhoReceived += list => gotWho = list;
                    ws.OnMailNoticeReceived += u => gotMailNotice = u;

                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                        ""seq"": 1, ""t"": ""who"",
                        ""d"": { ""entries"": [
                            { ""name"": ""Kira"", ""alias"": ""KR"", ""idle_s"": 12, ""conn_s"": 720, ""doing"": ""Hauling volatiles"" },
                            { ""name"": ""Soren"", ""idle_s"": 90, ""conn_s"": 3600 }
                        ] }
                    }"));
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                        ""seq"": 2, ""t"": ""mail_notice"", ""d"": { ""unread"": 3 }
                    }"));
                    ws.PumpMainThread();

                    Assert.IsNotNull(gotWho);
                    Assert.AreEqual(2, gotWho.Count);
                    Assert.AreEqual(3, gotMailNotice);

                    hud.SocialModal.ApplyWhoEntries(gotWho);
                    Assert.AreEqual(2, hud.SocialModal.OnlinePilotCount);
                }

                // 3. Mail panel keyframe & unread badge sync
                int lastUnreadBadge = -1;
                hud.SocialModal.OnUnreadMailCountChanged += u => lastUnreadBadge = u;
                hud.SocialModal.ApplyMailPanel(JsonNode.Parse(@"{
                    ""folder"": ""inbox"",
                    ""folders"": [{""name"": ""inbox"", ""count"": 2, ""unread"": 1}, {""name"": ""sent"", ""count"": 1, ""unread"": 0}],
                    ""entries"": [
                        {""index"": 1, ""id"": 101, ""from"": {""id"": ""#4"", ""name"": ""Dockmaster""}, ""subject"": ""Berth fee"", ""status"": ""unread""},
                        {""index"": 2, ""id"": 102, ""from"": {""id"": ""#9"", ""name"": ""Co-op""}, ""subject"": ""Charter"", ""status"": ""read""}
                    ],
                    ""quota"": {""used"": 3, ""max"": 200}
                }"));
                Assert.AreEqual(2, hud.SocialModal.MailEntryCount);
                Assert.AreEqual(1, hud.SocialModal.UnreadMailCount);
                Assert.AreEqual(1, lastUnreadBadge);

                // 4. Comsys panel keyframe & live channel message ingestion
                hud.SocialModal.ApplyComsysPanel(JsonNode.Parse(@"{
                    ""channels"": [
                        {""name"": ""Public"", ""alias"": ""pub"", ""ic"": false, ""gagged"": false},
                        {""name"": ""Corridor"", ""alias"": ""cor"", ""ic"": true, ""gagged"": false}
                    ]
                }"));
                Assert.AreEqual(2, hud.SocialModal.ChannelCount);
                Assert.AreEqual("Public", hud.SocialModal.SelectedChannelName);
                hud.SocialModal.IngestChannelMessage(new MushChannelEvent
                {
                    Channel = "Corridor",
                    Actor = new MushObjRef { Id = "#7", Name = "Kira" },
                    Text = "Clear approach to Stickney."
                });

                // 5. [J] Quest Journal capture, tab queries & Pinned Objective Tracker card
                var journalCmds = new List<string>();
                hud.QuestJournal.OnCommandRequested += journalCmds.Add;
                hud.QuestJournal.SetOpen(true, TacSim.UI.QuestJournalTab.Quests);
                Assert.IsTrue(hud.QuestJournal.IsOpen);
                Assert.IsTrue(hud.IsModalOpen);
                Assert.Contains("+quest", journalCmds);

                Assert.IsTrue(hud.QuestJournal.IngestServerOutput("Deliver 10t volatiles to Stickney Yards", null));
                Assert.IsTrue(hud.QuestJournal.IngestServerOutput("Reward: 4,200 cr", null));
                Assert.AreEqual(2, hud.QuestJournal.GetTabLineCount(TacSim.UI.QuestJournalTab.Quests));

                hud.QuestJournal.SelectTab(TacSim.UI.QuestJournalTab.Claims, queryServer: true);
                Assert.Contains("+claim/list", journalCmds);

                hud.QuestJournal.SelectTab(TacSim.UI.QuestJournalTab.Quests, queryServer: false);
                hud.QuestJournal.PinActiveTabSummary();
                Assert.IsTrue(hud.QuestJournal.HasPinnedObjective);
                StringAssert.Contains("Stickney", hud.QuestJournal.PinnedTitle);

                hud.QuestJournal.SyncTrackerVisibility(TacSim.Core.TacSimActiveStage.StationWalk, rightCornerBusy: false);
                Assert.IsTrue(hud.QuestJournal.IsTrackerVisible, "Pinned objective card must show below minimap when right corner is free");
                hud.QuestJournal.SyncTrackerVisibility(TacSim.Core.TacSimActiveStage.SimulatorArena, rightCornerBusy: true);
                Assert.IsFalse(hud.QuestJournal.IsTrackerVisible, "Pinned objective card must hide in Arena / busy HUD");

                // 6. ESC closes SocialModal then QuestJournal cleanly
                hud.SocialModal.SetOpen(true);
                Assert.IsTrue(hud.TryCloseTopmostModal());
                Assert.IsFalse(hud.SocialModal.IsOpen);
                Assert.IsTrue(hud.QuestJournal.IsOpen);
                Assert.IsTrue(hud.TryCloseTopmostModal());
                Assert.IsFalse(hud.QuestJournal.IsOpen);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void BridgeHoloTableAndDockingCorridor_InteractiveHoloClickToggle3DGatesAnd6DofHudReticle()
        {
            // 1. Pure-math 6-DOF Docking Alignment HUD Reticle offsets
            TacSim.UI.TacSimTacticalBrackets.ComputeDockingReticleOffsets(
                dxMm: 150, dyMm: -150, yawMdeg: 3000, pitchMdeg: -3000, rollMdeg: 2500, vzMmps: 500,
                out UnityEngine.Vector2 latPx, out UnityEngine.Vector2 attPx, out float rollDeg, out float closureNorm);
            Assert.AreEqual(26f, latPx.x, 0.1f);
            Assert.AreEqual(-26f, latPx.y, 0.1f);
            Assert.AreEqual(24f, attPx.x, 0.1f);
            Assert.AreEqual(24f, attPx.y, 0.1f);
            Assert.AreEqual(-2.5f, rollDeg, 0.05f);
            Assert.AreEqual(0.5f, closureNorm, 0.01f);

            var rootGo = new UnityEngine.GameObject("BridgeHoloAndDockingTest");
            try
            {
                // 2. Interactive 3D Bridge Holo-Table: clickable blips, clickable planets & mode switching
                var shipGo = new UnityEngine.GameObject("ShipHoloTest");
                shipGo.transform.SetParent(rootGo.transform, false);
                var ship = shipGo.AddComponent<TacSim.World.ShipInteriorController>();
                ship.BuildInterior(null, "dray");

                var chart = TacSim.Space.TacSimSolarChartFrame.FromJson(JsonNode.Parse(@"{
                    ""vantage"": ""ceres"",
                    ""scale"": { ""r0_au"": 2.0, ""au_max"": 10.0 },
                    ""bodies"": [
                        { ""id"": ""ceres"", ""name"": ""Ceres"", ""x_au"": 2.7, ""y_au"": 0.1, ""z_au"": 0.0, ""here"": true },
                        { ""id"": ""mars"", ""name"": ""Mars"", ""x_au"": 1.5, ""y_au"": 0.2, ""z_au"": 0.0 }
                    ],
                    ""routes"": [
                        { ""to"": ""stickney"", ""label"": ""Stickney Yards"", ""body"": ""mars"", ""legs"": [{ ""from"": ""ceres"", ""to"": ""mars"" }] }
                    ]
                }"));
                ship.ApplySolarChartFrame(chart);
                Assert.IsTrue(ship.IsHoloShowingSolarSystem);
                Assert.GreaterOrEqual(ship.ClickableHoloNodeCount, 2, "Solar bodies on Bridge Holo-Table must expose clickable BridgeHoloMarkers");

                string clickedPlanet = null;
                ship.OnHoloPlanetClicked += p => clickedPlanet = p;
                var planetMarkers = ship.GetComponentsInChildren<TacSim.World.BridgeHoloMarker>(true);
                Assert.GreaterOrEqual(planetMarkers.Length, 2);
                Assert.IsTrue(ship.TryHandleHoloMarkerClick(planetMarkers[1]));
                Assert.AreEqual("stickney", clickedPlanet, "Clicking Mars/Stickney holo body must emit its corridor port slug");

                // Switch to live tactical contacts and verify mode toggling between Local Bubble and Solar System
                ship.PopulateHoloBlips(JsonNode.Parse(@"{
                    ""range_km"": 100,
                    ""contacts"": [
                        { ""id"": ""own"", ""you"": true, ""x_km"": 0, ""y_km"": 0, ""z_km"": 0 },
                        { ""id"": ""C1"", ""x_km"": 20, ""y_km"": 5, ""z_km"": 3, ""relation"": ""hostile"" }
                    ]
                }"));
                Assert.IsFalse(ship.IsHoloShowingSolarSystem);
                string clickedContact = null;
                bool clickedIsRock = true;
                ship.OnHoloContactClicked += (id, rock) => { clickedContact = id; clickedIsRock = rock; };
                var tacMarkers = ship.GetComponentsInChildren<TacSim.World.BridgeHoloMarker>(true);
                Assert.AreEqual(1, tacMarkers.Length);
                Assert.IsTrue(ship.TryHandleHoloMarkerClick(tacMarkers[0]));
                Assert.AreEqual("C1", clickedContact);
                Assert.IsFalse(clickedIsRock);

                ship.SetHoloDisplayMode(TacSim.World.BridgeHoloMode.SolarSystem);
                Assert.IsTrue(ship.IsHoloShowingSolarSystem, "Forcing SolarSystem mode must project the solar orrery even while contacts exist");
                ship.SetHoloDisplayMode(TacSim.World.BridgeHoloMode.LocalBubble);
                Assert.IsFalse(ship.IsHoloShowingSolarSystem, "Forcing LocalBubble mode must return to local tactical contacts");

                // 3. 3D Illuminated Bow Docking Corridor + 6-DOF Alignment HUD Crosshair
                var spaceGo = new UnityEngine.GameObject("SpaceDockCorridorTest");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<TacSim.Space.SpaceFlightController>();
                space.BuildSpaceStage(null);
                Assert.AreEqual(5, space.DockingCorridorGateCount, "Bow docking corridor must build 5 open 4-rail alignment gates");
                Assert.AreEqual(4, space.DockingCorridorRailCount, "Bow docking corridor must build 4 longitudinal guide rails");
                Assert.IsTrue(space.HasDockingGhostSight, "Bow docking collar must build 3D lateral alignment ghost sight");

                space.StartDockingFromFeed(JsonNode.Parse(@"{
                    ""session_id"": ""dock.hud8"",
                    ""target_label"": ""Ceres Bay 4"",
                    ""start_pose"": { ""dx_mm"": 120, ""dy_mm"": -80, ""dz_mm"": 1800, ""vz_mmps"": 400, ""roll_mdeg"": 1500 }
                }"));
                Assert.IsTrue(space.IsDockingActive);

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.TacticalBrackets.BindControllers(space, null, null);
                var camGo = new UnityEngine.GameObject("DockHudCam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                hud.TacticalBrackets.SyncBrackets(TacSim.Core.TacSimActiveStage.ShipHelmSeated, camGo.GetComponent<UnityEngine.Camera>());

                Assert.IsTrue(hud.TacticalBrackets.IsDockingReticleVisible, "Active 6-DOF docking must activate center HUD alignment reticle");
                Assert.AreEqual(0, hud.TacticalBrackets.ActiveBracketCount, "Normal space brackets must hide cleanly during 6-DOF docking");
                Assert.AreEqual(-1.5f, hud.TacticalBrackets.DockingRollBarAngleDeg, 0.05f);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void TacticalOverviewAndSurfaceScanner_RangeSortedContactsOreDriveToAndHoverHalo()
        {
            var rootGo = new UnityEngine.GameObject("TacticalOverviewAndSurfaceTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.TacticalOverview, "BuildHud must wire [T] Tactical Overview drawer");

                var spaceGo = new UnityEngine.GameObject("SpaceOvTest");
                spaceGo.transform.SetParent(rootGo.transform, false);
                var space = spaceGo.AddComponent<TacSim.Space.SpaceFlightController>();
                space.BuildSpaceStage(null);
                space.ApplyHolotankFeed(JsonNode.Parse(@"{
                    ""own"": { ""id"": ""own"", ""x_km"": 0, ""y_km"": 0, ""z_km"": 0 },
                    ""contacts"": [
                        { ""id"": ""FAR"", ""label"": ""Far Freighter"", ""x_km"": 80, ""y_km"": 0, ""z_km"": 0, ""iff"": ""neutral"" },
                        { ""id"": ""NEAR"", ""label"": ""Near Raider"", ""x_km"": 12, ""y_km"": 0, ""z_km"": 0, ""iff"": ""hostile"", ""locked"": true }
                    ],
                    ""rocks"": [{ ""id"": ""A"", ""x_km"": 5, ""y_km"": 0, ""z_km"": 0 }]
                }"));

                var surfGo = new UnityEngine.GameObject("SurfOvTest");
                surfGo.transform.SetParent(rootGo.transform, false);
                var surf = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surf.BuildSurfaceStage(null, "occator_rim");
                surf.EnterSurfaceMode();
                Assert.IsTrue(surf.HasHoverDriveCursor, "Surface stage must create 3D terrain hover drive cursor ring");
                surf.ApplySiteMapFrame(JsonNode.Parse(@"{
                    ""width"": 8, ""height"": 8,
                    ""site"": {
                        ""site"": ""Occator Rim"",
                        ""grid"": { ""x0"": 0, ""y0"": 0, ""span"": 1, ""cell_cm"": 1000 },
                        ""rigs"": [{ ""slot"": 1, ""cell"": [1, 1] }],
                        ""readings"": [
                            { ""cell"": [6, 6], ""text"": ""Trace silicates"" },
                            { ""cell"": [2, 2], ""text"": ""Rich volatiles vein"" }
                        ]
                    },
                    ""rows"": [""........"",""........"",""........"",""........"",""........"",""........"",""........"",""........""],
                    ""you"": [1, 1]
                }"));
                Assert.AreEqual(2, surf.ActiveBeaconHaloCount, "Scanned ore pockets must spawn 3D beacon pillars with ground yield halo rings");

                var arenaGo = new UnityEngine.GameObject("ArenaOvTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(null, null);
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""seated"": true, ""match"": 7, ""callsign"": ""R1"", ""party"": ""R"",
                    ""contacts"": [
                        { ""designator"": ""B2"", ""party"": ""B"", ""relation"": ""hostile"", ""cell"": [22, 22], ""range_m"": 420 },
                        { ""designator"": ""B1"", ""party"": ""B"", ""relation"": ""hostile"", ""cell"": [14, 14], ""range_m"": 180 }
                    ]
                }"));

                hud.TacticalOverview.BindControllers(space, surf, arena);

                // 1. Hidden in Station walk; visible & range-sorted in Space
                hud.TacticalOverview.SyncOverview(TacSim.Core.TacSimActiveStage.StationWalk, forceImmediate: true);
                Assert.IsFalse(hud.TacticalOverview.IsVisible);

                hud.TacticalOverview.SyncOverview(TacSim.Core.TacSimActiveStage.ShipHelmSeated, forceImmediate: true);
                Assert.IsTrue(hud.TacticalOverview.IsVisible);
                Assert.AreEqual(3, hud.TacticalOverview.VisibleRowCount);
                Assert.AreEqual("A", hud.TacticalOverview.FirstRowId, "Nearest rock (5 km) must sort first in All filter");

                hud.TacticalOverview.SetFilter(TacSim.UI.TacticalOverviewFilter.Primary); // Ships only
                Assert.AreEqual(2, hud.TacticalOverview.VisibleRowCount);
                Assert.AreEqual("NEAR", hud.TacticalOverview.FirstRowId, "Nearest ship (12 km) must sort first in Ships filter");

                string clickedSpaceId = null;
                hud.TacticalOverview.OnSpaceRowClicked += (id, _) => clickedSpaceId = id;
                Assert.IsTrue(hud.TacticalOverview.ClickRowByIndex(0));
                Assert.AreEqual("NEAR", clickedSpaceId);

                // 2. Surface stage lists scanned ore readings & dispatches rover drive coordinates
                hud.TacticalOverview.SetFilter(TacSim.UI.TacticalOverviewFilter.All);
                hud.TacticalOverview.SyncOverview(TacSim.Core.TacSimActiveStage.PlanetarySurface, forceImmediate: true);
                Assert.IsTrue(hud.TacticalOverview.IsVisible);
                Assert.AreEqual(2, hud.TacticalOverview.VisibleRowCount);
                int driveX = -1, driveY = -1;
                hud.TacticalOverview.OnSurfaceBeaconDriveClicked += (x, y) => { driveX = x; driveY = y; };
                Assert.IsTrue(hud.TacticalOverview.ClickRowByIndex(0));
                Assert.AreEqual(2, driveX);
                Assert.AreEqual(2, driveY);

                // 3. Arena stage sorts nearest hostile first & dispatches lock
                hud.TacticalOverview.SyncOverview(TacSim.Core.TacSimActiveStage.SimulatorArena, forceImmediate: true);
                Assert.AreEqual(2, hud.TacticalOverview.VisibleRowCount);
                Assert.AreEqual("B1", hud.TacticalOverview.FirstRowId, "Nearest arena target (180m) must sort first");
                string lockedArena = null;
                hud.TacticalOverview.OnArenaContactLockClicked += d => lockedArena = d;
                Assert.IsTrue(hud.TacticalOverview.ClickRowByIndex(0));
                Assert.AreEqual("B1", lockedArena);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void BridgeAlertLightingCanopyVistaAndQuickComms_ReactsToThreatsVantageAndPrefixBar()
        {
            var rootGo = new UnityEngine.GameObject("BridgeAlertVistaCommsTest");
            try
            {
                // 1. Bridge exterior planetary vista & dynamic alert lighting states
                var shipGo = new UnityEngine.GameObject("ShipAlertTest");
                shipGo.transform.SetParent(rootGo.transform, false);
                var ship = shipGo.AddComponent<TacSim.World.ShipInteriorController>();
                ship.BuildInterior(null, "dray");

                Assert.IsTrue(ship.HasBridgeExteriorPlanet, "Bridge forward canopy must spawn distant local planetary body");
                Assert.AreEqual(TacSim.World.ShipAlertLevel.Normal, ship.ActiveAlertLevel);

                var alerts = new List<TacSim.World.ShipAlertLevel>();
                ship.OnAlertLevelChanged += alerts.Add;

                // Hostile contact triggers Caution (Yellow Alert)
                ship.PopulateHoloBlips(JsonNode.Parse(@"{
                    ""range_km"": 100,
                    ""contacts"": [
                        { ""id"": ""own"", ""you"": true, ""x_km"": 0, ""y_km"": 0, ""z_km"": 0 },
                        { ""id"": ""H1"", ""x_km"": 25, ""y_km"": 0, ""z_km"": 0, ""relation"": ""hostile"" }
                    ]
                }"));
                Assert.AreEqual(TacSim.World.ShipAlertLevel.Caution, ship.ActiveAlertLevel);

                // Live torpedo triggers RedAlert
                ship.PopulateHoloBlips(JsonNode.Parse(@"{
                    ""range_km"": 100,
                    ""contacts"": [{ ""id"": ""own"", ""you"": true, ""x_km"": 0, ""y_km"": 0, ""z_km"": 0 }],
                    ""torpedoes"": [{ ""id"": ""T1"", ""x_km"": 15, ""y_km"": 0, ""z_km"": 0, ""tti_s"": 8 }]
                }"));
                Assert.AreEqual(TacSim.World.ShipAlertLevel.RedAlert, ship.ActiveAlertLevel);

                // Clear space returns to Normal
                ship.PopulateHoloBlips(JsonNode.Parse(@"{ ""range_km"": 100, ""contacts"": [] }"));
                Assert.AreEqual(TacSim.World.ShipAlertLevel.Normal, ship.ActiveAlertLevel);
                Assert.AreEqual(3, alerts.Count);

                // Port vantage theme updates exterior planet slug
                ship.ApplyVistaPortTheme("stickney");
                Assert.AreEqual("stickney", ship.ActiveVistaPortSlug);

                // 2. Quick-Comms prefix strip exists inside CommsDrawer and pre-fills chat input
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                UnityEngine.Transform prefixBar = null;
                var allTr = rootGo.GetComponentsInChildren<UnityEngine.Transform>(true);
                for (int i = 0; i < allTr.Length; i++)
                {
                    if (allTr[i].name == "QuickCommsPrefixBar") { prefixBar = allTr[i]; break; }
                }
                Assert.IsNotNull(prefixBar, "CommsDrawer must build QuickCommsPrefixBar");
                Assert.AreEqual(5, prefixBar.childCount, "Expected SAY / POSE / PAGE / WHO / CTRL+K prefix pills");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaReplayAndObserverTheater_DecodesInterpolatesSpawns3DRigsAndWiresHudControls()
        {
            // 1. Decode pushed grid trigger from `SIM REPLAY <match>`
            var gridPayload = JsonNode.Parse(@"{
                ""schema"": { ""shape"": ""grid"" },
                ""match"": 42,
                ""keyframe"": { ""index"": 1 }
            }");
            Assert.IsTrue(TacSim.Arena.TacSimArenaReplayModel.TryDecodeReplayGridTrigger(gridPayload, out ulong trigMatch, out int trigKf));
            Assert.AreEqual(42UL, trigMatch);
            Assert.AreEqual(1, trigKf);

            // 2. Decode full `arena.replay` document & test smooth interpolation + shortest-arc heading turn
            var docPayload = JsonNode.Parse(@"{
                ""schema"": { ""shape"": ""replay"" },
                ""match"": 42,
                ""arena"": ""proving_ground"",
                ""cell_cm"": 1000,
                ""width"": 64,
                ""height"": 64,
                ""summary_ms"": 5000,
                ""end_ms"": 12000,
                ""outcome"": ""red"",
                ""reason"": ""elimination"",
                ""mode"": ""hold"",
                ""roster"": [
                    { ""callsign"": ""R1"", ""team"": ""red"" },
                    { ""callsign"": ""B1"", ""team"": ""blue"" }
                ],
                ""keyframes"": [
                    [0, [[10, 10, 350], [40, 40, 180]]],
                    [5000, [[20, 10, 10], [30, 40, 180]]],
                    [10000, [[24, 14, 45], [30, 40, 180]]]
                ],
                ""shots"": [
                    [4500, 0, 1, ""hit""],
                    [8000, 0, 1, ""hit""]
                ],
                ""destroyed"": [
                    [8000, 1]
                ]
            }");
            Assert.IsTrue(TacSim.Arena.TacSimArenaReplayModel.TryDecodeReplayDocument(docPayload, out var doc));
            Assert.AreEqual(42UL, doc.MatchId);
            Assert.AreEqual(2, doc.Roster.Count);
            Assert.AreEqual(3, doc.Keyframes.Count);
            Assert.AreEqual(2, doc.Shots.Count);
            Assert.AreEqual(1, doc.Destroyed.Count);

            // Halfway between t=0 (350 deg) and t=5000 (10 deg) -> x=15, heading=0 deg (shortest arc through north!)
            var mid = TacSim.Arena.TacSimArenaReplayModel.SampleInterpolatedFrame(doc, 2500f);
            Assert.IsTrue(mid.BetweenKeyframes);
            Assert.IsFalse(mid.Held);
            Assert.AreEqual(15f, mid.Units[0].Value.CellX, 0.01f);
            Assert.AreEqual(0f, mid.Units[0].Value.HeadingDeg, 0.1f);
            Assert.AreEqual(1, mid.RedAliveCount);
            Assert.AreEqual(1, mid.BlueAliveCount);
            StringAssert.Contains("Between keyframes 1 and 2", TacSim.Arena.TacSimArenaReplayModel.FormatFrameSourceLabel(doc, mid));

            // After wreck at 8000 ms, B1 is marked wrecked and BlueAliveCount drops to 0
            var afterWreck = TacSim.Arena.TacSimArenaReplayModel.SampleInterpolatedFrame(doc, 10000f);
            Assert.IsFalse(afterWreck.BetweenKeyframes);
            Assert.IsTrue(afterWreck.IsWrecked[1]);
            Assert.AreEqual(1, afterWreck.RedAliveCount);
            Assert.AreEqual(0, afterWreck.BlueAliveCount);

            // 3. OocArenaController + TacSimReplayControlsHud integration (Replay -> Keyframe Step -> Live Observer -> Clear)
            var rootGo = new UnityEngine.GameObject("ArenaReplayTheaterTest");
            try
            {
                var arenaGo = new UnityEngine.GameObject("ArenaReplayCtrl");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(null, null);

                int docSubRequests = 0;
                arena.OnRequestReplayDocumentSubscribe += () => docSubRequests++;

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.ReplayHud, "BuildHud must wire TacSimReplayControlsHud");
                hud.ReplayHud.BindArena(arena);

                // Pushed grid trigger enters replay mode & asks for full replay doc once
                arena.HandleReplayPanelFrame(gridPayload);
                Assert.IsTrue(arena.IsReplayModeActive);
                Assert.IsTrue(arena.IsStageVisible);
                Assert.AreEqual(1, docSubRequests);

                // Full replay document spawns 3D rigs for both teams and jumps to triggered keyframe index 1 (5000 ms)
                arena.HandleReplayPanelFrame(docPayload);
                Assert.AreEqual(5000f, arena.ReplayCursorMs, 0.1f);
                Assert.AreEqual(2, arena.ActiveReplayRigCount);
                Assert.IsNotNull(arena.SpectatorFocusTransform);

                hud.ReplayHud.RefreshView();
                Assert.IsTrue(hud.ReplayHud.IsVisible);
                Assert.AreEqual(2, hud.ReplayHud.VisibleRosterRowCount);
                StringAssert.Contains("MATCH #42", hud.ReplayHud.HeaderBannerText);
                StringAssert.Contains("[HOLD]", hud.ReplayHud.HeaderBannerText);

                // Step forward keyframe to 10000 ms & cycle focused unit to B1
                arena.StepReplayKeyframe(+1);
                Assert.AreEqual(10000f, arena.ReplayCursorMs, 0.1f);
                arena.CycleReplayFocusUnit(+1);
                Assert.AreEqual(1, arena.ReplayFocusUnitIndex);

                // Live staff `arena.observer` snapshot seamlessly replaces replay mode
                var obsPayload = JsonNode.Parse(@"{
                    ""match"": 77,
                    ""now_ms"": 18000,
                    ""tempo"": ""Brisk"",
                    ""floor_ms"": 1000,
                    ""cell_cm"": 1000,
                    ""width"": 64,
                    ""height"": 64,
                    ""in_flight"": 2,
                    ""units"": [
                        [""R1"", ""R"", 12, 14, 90, 501, ""p""],
                        [""B1"", ""B"", 38, 22, 270, 502, ""pmw""]
                    ]
                }");
                arena.HandleObserverPanelFrame(obsPayload);
                Assert.IsTrue(arena.IsLiveObserverActive);
                Assert.IsFalse(arena.IsReplayModeActive);
                Assert.AreEqual(77UL, arena.ActiveSpectatorMatchId);
                Assert.AreEqual(2, arena.ActiveReplayRigCount);
                hud.ReplayHud.RefreshView();
                StringAssert.Contains("[LIVE OBSERVER]", hud.ReplayHud.HeaderBannerText);
                StringAssert.Contains("IN FLIGHT: 2", hud.ReplayHud.HeaderBannerText);

                // `watching: false` clears live observer cleanly
                arena.HandleObserverPanelFrame(JsonNode.Parse(@"{ ""watching"": false }"));
                Assert.IsFalse(arena.IsLiveObserverActive);
                Assert.IsFalse(arena.IsStageVisible);
                hud.ReplayHud.RefreshView();
                Assert.IsFalse(hud.ReplayHud.IsVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void AudioMixerCameraPrefsAndHudDensityPresets_ApplyAndPersistCleanly()
        {
            var rootGo = new UnityEngine.GameObject("AudioMixerCamHudTest");
            try
            {
                // 1. Audio Mixer channels, effective volumes & lazy Red Alert Klaxon clip
                var audio = rootGo.AddComponent<TacSim.Core.TacSimAudioSynthesizer>();
                audio.InitializeAudioGraph();
                Assert.AreEqual(100, audio.MasterVolumePct);
                Assert.AreEqual(80, audio.AmbientVolumePct);
                Assert.AreEqual(90, audio.SfxVolumePct);

                audio.SetMasterVolumePct(50);
                audio.SetAmbientVolumePct(50);
                audio.SetSfxVolumePct(40);
                Assert.AreEqual(0.085f * 0.25f, audio.EffectiveAmbientVolume, 0.0005f);
                Assert.AreEqual(0.22f * 0.20f, audio.EffectiveSfxVolume, 0.0005f);

                float[] klaxon = TacSim.Core.TacSimAudioSynthesizer.GenerateRedAlertKlaxonSamples();
                Assert.Greater(klaxon.Length, 5000);
                audio.PlayRedAlertKlaxon();
                Assert.AreEqual(12, audio.AllocatedClipCount, "Lazy Red Alert klaxon should allocate 12th clip");
                audio.DisposeClips();
                Assert.AreEqual(0, audio.AllocatedClipCount);

                // 2. Camera preferences (Sensitivity, Zoom Speed, Invert-Y, FOV)
                var camGo = new UnityEngine.GameObject("CamPrefsTest", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var rig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                rig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();
                rig.ApplyCameraPreferences(150, 125, invertY: true, fovDeg: 72, savePrefs: false);
                Assert.AreEqual(150, rig.OrbitSensitivityPct);
                Assert.AreEqual(125, rig.ZoomSpeedPct);
                Assert.IsTrue(rig.InvertY);
                Assert.AreEqual(72, rig.FieldOfViewDeg);
                Assert.AreEqual(72f, rig.ControlledCamera.fieldOfView, 0.01f);
                Assert.AreEqual(4.2f, rig.mouseSensitivity, 0.01f);

                // 3. 3-Tab Options Console & F9 Cinematic HUD Density Toggle
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.DisplayOptions.BindSubsystems(audio, rig);
                hud.DisplayOptions.SelectTab(TacSim.UI.TacSimOptionsTab.Audio);
                Assert.AreEqual(TacSim.UI.TacSimOptionsTab.Audio, hud.DisplayOptions.ActiveTab);
                hud.DisplayOptions.SelectTab(TacSim.UI.TacSimOptionsTab.CameraAndHud);
                Assert.AreEqual(TacSim.UI.TacSimOptionsTab.CameraAndHud, hud.DisplayOptions.ActiveTab);

                hud.DisplayOptions.SetHudDensityPreset(TacSim.UI.TacSimHudDensityPreset.Compact, save: false);
                Assert.AreEqual(TacSim.UI.TacSimHudDensityPreset.Compact, TacSim.UI.TacSimDisplayOptions.CurrentHudDensity);
                Assert.IsFalse(hud.IsCinematicHudActive);

                // Toggle F9 Cinematic mode hides UnitFrames & Minimap, then restores Compact cleanly
                Assert.AreEqual(TacSim.UI.TacSimHudDensityPreset.Cinematic, hud.DisplayOptions.ToggleCinematicHudMode());
                Assert.IsTrue(hud.IsCinematicHudActive);
                Assert.IsFalse(hud.UnitFrames.IsFrameRootVisible);
                Assert.IsFalse(hud.Minimap.IsVisible);

                Assert.AreEqual(TacSim.UI.TacSimHudDensityPreset.Compact, hud.DisplayOptions.ToggleCinematicHudMode());
                Assert.IsFalse(hud.IsCinematicHudActive);
                Assert.IsTrue(hud.UnitFrames.IsFrameRootVisible);
                Assert.IsTrue(hud.Minimap.IsVisible);

                // Reset back to Standard for other tests
                hud.DisplayOptions.SetHudDensityPreset(TacSim.UI.TacSimHudDensityPreset.Standard, save: false);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void LiveRpScenesAndSignalContacts_ParsesRoomGlancePanelFramesAndDispatchesInspectVerbs()
        {
            // 1. Parse RoomView.ambient["scene"] glance
            var rv = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                ""id"": ""#101"",
                ""name"": ""Ceres Exchange Floor"",
                ""ambient"": {
                    ""scene"": {
                        ""id"": 19,
                        ""title"": ""Broker Dispute"",
                        ""kind"": ""timestop"",
                        ""mode"": ""turn"",
                        ""status"": ""active"",
                        ""round"": 2,
                        ""turn"": 1,
                        ""current_actor"": { ""id"": ""#42"", ""name"": ""Nadia"" }
                    }
                }
            }"));
            Assert.IsNotNull(rv.Scene);
            Assert.AreEqual(19L, rv.Scene.Id);
            Assert.AreEqual("Broker Dispute", rv.Scene.Title);
            Assert.AreEqual("Nadia", rv.Scene.CurrentActor.Name);

            var rootGo = new UnityEngine.GameObject("SceneAndSignalIntelTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.SceneIntelModal, "BuildHud must wire [L] TacSimSceneAndIntelModal");

                string turnAlertScene = null;
                hud.SceneIntelModal.OnLocalPlayerTurnStarted += s => turnAlertScene = s;
                hud.SceneIntelModal.SetLocalCharacter("#42", "Nadia");

                // 2. Room glance activates scene banner & fires local-turn alert when current_actor matches player
                hud.SceneIntelModal.ApplyRoomSceneGlance(rv.Scene);
                Assert.IsTrue(hud.SceneIntelModal.IsSceneBannerVisible);
                Assert.IsTrue(hud.SceneIntelModal.IsLocalPlayersTurn);
                Assert.AreEqual("Broker Dispute", turnAlertScene);

                // 3. Full `scenes` panel keyframe populates participant roster & null clears cleanly
                hud.SceneIntelModal.ApplyScenesPanel(JsonNode.Parse(@"{
                    ""id"": 19,
                    ""title"": ""Broker Dispute"",
                    ""kind"": ""timestop"",
                    ""mode"": ""turn"",
                    ""status"": ""paused"",
                    ""round"": 2,
                    ""turn"": 0,
                    ""current_actor"": { ""id"": ""#55"", ""name"": ""Ilse"" },
                    ""facilitator"": { ""id"": ""#42"", ""name"": ""Nadia"" },
                    ""anchor"": { ""id"": ""#101"", ""name"": ""Ceres Exchange Floor"" },
                    ""participants"": [
                        { ""id"": ""#55"", ""name"": ""Ilse"" },
                        { ""id"": ""#42"", ""name"": ""Nadia"" }
                    ],
                    ""observers"": [{ ""id"": ""#77"", ""name"": ""Dov"" }]
                }"));
                Assert.IsFalse(hud.SceneIntelModal.IsLocalPlayersTurn);
                Assert.AreEqual(2, hud.SceneIntelModal.ActiveScene.Participants.Count);
                Assert.AreEqual(1, hud.SceneIntelModal.ActiveScene.Observers.Count);

                // 4. `tactics.contacts` propagation ledger populates clickable rows & dispatches sanitized `+contact <id>`
                hud.SceneIntelModal.ApplySignalContactsPanel(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""tactics.contacts"", ""shape"": ""list"" },
                    ""truncated"": false,
                    ""rows"": [
                        { ""id"": ""C-01"", ""gen"": 2, ""rev"": 4, ""state"": ""live"", ""age"": ""3s"", ""band"": ""thermal"", ""source"": ""own"" },
                        { ""id"": ""C-02"", ""gen"": 1, ""rev"": 1, ""state"": ""stale"", ""age"": ""48s"", ""band"": ""radar"", ""source"": ""relay"" }
                    ]
                }"));
                Assert.AreEqual(2, hud.SceneIntelModal.SignalContactCount);
                Assert.AreEqual(2, hud.SceneIntelModal.VisibleContactRowCount);

                string dispatchedCmd = null;
                hud.SceneIntelModal.OnCommandRequested += c => dispatchedCmd = c;
                hud.SceneIntelModal.InspectContactById("C-01");
                Assert.AreEqual("+contact C-01", dispatchedCmd);

                // Rejects injected whitespace/control characters
                dispatchedCmd = null;
                hud.SceneIntelModal.InspectContactById("C-01\nQUIT");
                Assert.IsNull(dispatchedCmd);

                // Null scenes panel clears active scene & banner; ESC closes modal
                hud.SceneIntelModal.SetOpen(true);
                Assert.IsTrue(hud.TryCloseTopmostModal());
                Assert.IsFalse(hud.SceneIntelModal.IsOpen);
                hud.SceneIntelModal.ApplyScenesPanel(JsonNode.Null);
                Assert.IsNull(hud.SceneIntelModal.ActiveScene);
                Assert.IsFalse(hud.SceneIntelModal.IsSceneBannerVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void StationExitSignageTrafficAndOrbitalVelocityScanner_SpawnsAndAnimates3DVisuals()
        {
            var rootGo = new UnityEngine.GameObject("StationSignageAndSpaceVectorsTest");
            try
            {
                // 1. Station 3D Illuminated Doorway Destination Lintels & Animated Exterior Port Traffic Shuttles
                var stGo = new UnityEngine.GameObject("StationSignageTest");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                Assert.GreaterOrEqual(station.DoorwaySignCount, 12, "Ceres Freeport doorways must spawn illuminated overhead destination lintel signs");
                Assert.AreEqual(3, station.ExteriorTrafficShuttleCount, "Berth Gallery exterior orbital lane must spawn 3 animated port traffic shuttles");

                // 2. Orbital Space 3D Newtonian Velocity Vector & Asteroid Spectral Scanner Halo Ring
                var spGo = new UnityEngine.GameObject("SpaceVectorsTest");
                spGo.transform.SetParent(rootGo.transform, false);
                var space = spGo.AddComponent<TacSim.Space.SpaceFlightController>();
                space.BuildSpaceStage(null);

                Assert.IsFalse(space.IsVelocityVectorVisible, "Velocity vector should stay hidden at zero velocity");
                Assert.IsFalse(space.IsRockScannerRingVisible, "Rock scanner ring should stay hidden when no asteroid is targeted");

                space.ApplyHolotankFeed(JsonNode.Parse(@"{
                    ""range_km"": 200,
                    ""contacts"": [
                        { ""id"": ""own"", ""you"": true, ""x_km"": 0, ""y_km"": 0, ""z_km"": 0, ""vx_kms"": 0.12, ""vy_kms"": 0.04, ""vz_kms"": 0.0 }
                    ],
                    ""rocks"": [
                        { ""id"": ""A"", ""x_km"": 4, ""y_km"": 1, ""z_km"": 0, ""spectral"": ""C-type volatiles"" }
                    ]
                }"));

                Assert.IsTrue(space.IsVelocityVectorVisible, "Non-zero ownship velocity must project 3D Newtonian velocity vector");
                Assert.Greater(space.OwnVelocityUnityMps.magnitude, 100f);
                Assert.IsTrue(space.IsRockScannerRingVisible, "Selected asteroid rock must project 3D spectral scanner halo ring");

                // Reset clears velocity vector & scanner halo cleanly
                space.ResetSessionState();
                Assert.IsFalse(space.IsVelocityVectorVisible);
                Assert.IsFalse(space.IsRockScannerRingVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void LivePaletteCommandsNpcThinkingAndComsysBacklog_HydrateAndRouteAuthoritatively()
        {
            // 1. Canonical Query::Commands & Query::ChannelBacklog wire frames
            string cmdsReq = MushProtocol.BuildCommandsQueryCommand(12UL);
            StringAssert.Contains("\"t\":\"commands\"", cmdsReq);
            string backlogReq = MushProtocol.BuildChannelBacklogQueryCommand(13UL, "Public", beforeSeq: 50UL, limit: 25);
            StringAssert.Contains("\"t\":\"channel_backlog\"", backlogReq);
            StringAssert.Contains("\"channel\":\"Public\"", backlogReq);
            StringAssert.Contains("\"before_seq\":50", backlogReq);
            StringAssert.Contains("\"limit\":25", backlogReq);

            // 2. Argument detection heuristic for live server commands
            Assert.IsTrue(TacSim.UI.TacSimCommandPalette.CommandTakesArguments("addcom <alias>=<channel>", "addcom"));
            Assert.IsTrue(TacSim.UI.TacSimCommandPalette.CommandTakesArguments("+helm/approach <rock>", "+helm/approach"));
            Assert.IsFalse(TacSim.UI.TacSimCommandPalette.CommandTakesArguments("inventory", "inventory"));

            var rootGo = new UnityEngine.GameObject("Wave3PR5_LiveHydrationTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                // 3. Live `commands.list` palette hydration + smart prefill vs direct execution
                string sentCmd = null;
                string prefilledCmd = null;
                hud.CommandPalette.OnCommandRequested += c => sentCmd = c;
                hud.CommandPalette.OnPrefillCommandRequested += p => prefilledCmd = p;

                hud.CommandPalette.ApplyServerCommandsList(JsonNode.Parse(@"{
                    ""commands"": [
                        { ""name"": ""+ship"", ""aliases"": [], ""switches"": [""hold""], ""usage"": ""+ship"", ""summary"": ""Already curated"", ""module"": ""tacsim"" },
                        { ""name"": ""inventory"", ""aliases"": [""inv""], ""switches"": [], ""usage"": ""inventory"", ""summary"": ""List carried items"", ""module"": ""core"" },
                        { ""name"": ""addcom"", ""aliases"": [], ""switches"": [], ""usage"": ""addcom <alias>=<channel>"", ""summary"": ""Join a comsys channel"", ""module"": ""comsys"" }
                    ],
                    ""truncated"": false
                }"));
                Assert.AreEqual(3, hud.CommandPalette.ServerCommandCount);

                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("inv");
                Assert.GreaterOrEqual(hud.CommandPalette.FilteredMatchCount, 1, "Alias 'inv' should match live 'inventory' server command");
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual("inventory", sentCmd, "Zero-arg live server command must dispatch immediately");

                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("addcom");
                Assert.GreaterOrEqual(hud.CommandPalette.FilteredMatchCount, 1);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual("addcom ", prefilledCmd, "Argument-taking server command must pre-fill chat input");

                // 4. Comsys keyframe + delta backlog passthrough via MushWsClient
                using (var ws = new MushWsClient())
                {
                    var backlogRequested = new List<string>();
                    hud.SocialModal.OnChannelBacklogRequested += ch => backlogRequested.Add(ch);
                    ws.OnPanelUpdated += (feed, data) =>
                    {
                        if (feed == "comsys")
                            hud.SocialModal.ApplyComsysPanel(data);
                    };

                    // Keyframe hydrates channels & triggers backlog request for logged default channel
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                        ""seq"": 1, ""ts"": 1000, ""t"": ""panel"",
                        ""d"": {
                            ""feed"": ""comsys"", ""kind"": ""keyframe"",
                            ""data"": {
                                ""channels"": [
                                    { ""name"": ""Public"", ""alias"": ""pub"", ""title"": """", ""gagged"": false, ""logged"": true, ""ic"": false, ""seq"": 12, ""last_read"": 9, ""unread"": 3 },
                                    { ""name"": ""Trade"", ""alias"": ""trd"", ""title"": """", ""gagged"": false, ""logged"": true, ""ic"": true, ""seq"": 4, ""last_read"": 4, ""unread"": 0 }
                                ]
                            }
                        }
                    }"));
                    ws.PumpMainThread();

                    Assert.AreEqual(2, hud.SocialModal.ChannelCount);
                    Assert.AreEqual(1, backlogRequested.Count, "Auto-selected logged channel must trigger channel_backlog request");
                    Assert.AreEqual("Public", backlogRequested[0]);

                    // Delta frame carrying `backlog` (newest-first) must pass through MushWsClient & populate chronological transcript
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                        ""seq"": 2, ""ts"": 1010, ""t"": ""panel"",
                        ""d"": {
                            ""feed"": ""comsys"", ""kind"": ""delta"",
                            ""data"": {
                                ""backlog"": {
                                    ""channel"": ""Public"",
                                    ""before_seq"": null,
                                    ""messages"": [
                                        { ""seq"": 12, ""actor"": { ""id"": ""#22"", ""name"": ""Voss"" }, ""title"": ""Pilot"", ""kind"": ""say"", ""text"": { ""spans"": [ { ""text"": ""Berth 4 clear."" } ] }, ""ts"": 1005 },
                                        { ""seq"": 11, ""actor"": { ""id"": ""#21"", ""name"": ""Kestrel"" }, ""title"": null, ""kind"": ""pose"", ""text"": { ""spans"": [ { ""text"": ""checks the cargo manifest."" } ] }, ""ts"": 1002 }
                                    ]
                                }
                            }
                        }
                    }"));
                    ws.PumpMainThread();

                    Assert.AreEqual(2, hud.SocialModal.GetChannelLineCount("Public"), "Comsys delta backlog slice must populate channel transcript");
                    hud.SocialModal.SelectChannel("Trade");
                    Assert.AreEqual(2, backlogRequested.Count, "Selecting a second logged channel must fetch its backlog");
                    Assert.AreEqual("Trade", backlogRequested[1]);
                }

                // 5. Live 3D NPC thinking bubble on StationWorldController & open NPC dialog modal
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var stationGo = new UnityEngine.GameObject("ThinkingTestStation");
                stationGo.transform.SetParent(rootGo.transform);
                var station = stationGo.AddComponent<StationWorldController>();
                station.BuildStation(catalog, "ceres_freeport");
                var rv = MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                    ""id"": ""#10"", ""name"": ""Arrivals"", ""desc"": ""Concourse"",
                    ""atlas"": { ""pack"": ""ceres_freeport"", ""entry"": ""room.c03_arrivals"", ""surface"": ""station"" },
                    ""contents"": [
                        { ""id"": ""#77"", ""name"": ""Harbor Clerk"", ""kind"": ""thing"" }
                    ]
                }"));
                station.ApplyRoomView(rv, "#1");

                station.SetActorThinking("#77", "Harbor Clerk", true);
                Assert.AreEqual("thinking...", station.GetActorSpeechText("#77"), "Active npc.thinking must display 3D thinking bubble above NPC");

                hud.ShowDetailModal("HARBOR CLERK", "Welcome to Ceres.", npcName: "Harbor Clerk");
                hud.SetNpcModalThinking("Harbor Clerk", true);
                Assert.IsTrue(hud.IsNpcModalThinking, "Open NPC modal must reflect live npc.thinking active state");

                // Speech arriving clears modal thinking & replaces 3D bubble; subsequent thinking=false must NOT wipe spoken line!
                station.ShowActorSpeech("#77", "Harbor Clerk", "Berth four is open.");
                station.SetActorThinking("#77", "Harbor Clerk", false);
                Assert.AreEqual("Berth four is open.", station.GetActorSpeechText("#77"), "Clearing npc.thinking must never erase the NPC's actual spoken line");

                hud.AppendModalResponseLine("<b>Harbor Clerk</b> says, \"Berth four is open.\"");
                Assert.IsFalse(hud.IsNpcModalThinking, "NPC speech arriving in open dialog modal must clear thinking indicator");

                // Session reset clears server command table cleanly
                hud.ResetSessionState();
                Assert.AreEqual(0, hud.CommandPalette.ServerCommandCount);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void NarrativeBacklogCommsSearchAndTimestamps_PaginatesFiltersAndStripsRichTags()
        {
            // 1. Canonical Query::NarrativeBacklog wire frame (initial & paged)
            string initReq = MushProtocol.BuildNarrativeBacklogQueryCommand(20UL, beforeSeq: null, limit: 30);
            StringAssert.Contains("\"t\":\"narrative_backlog\"", initReq);
            StringAssert.Contains("\"limit\":30", initReq);
            StringAssert.DoesNotContain("\"before_seq\"", initReq);

            string pagedReq = MushProtocol.BuildNarrativeBacklogQueryCommand(21UL, beforeSeq: 41UL, limit: 25);
            StringAssert.Contains("\"before_seq\":41", pagedReq);
            StringAssert.Contains("\"limit\":25", pagedReq);

            // 2. Rich-text tag stripping avoids false positives on markup words like "color"
            Assert.AreEqual("Voss says, \"Berth 4 clear.\"", TacSim.UI.TacSimHudController.StripUnityRichTextTags("<color=#66EEAA><b>Voss</b></color> says, \"Berth 4 clear.\""));

            var rootGo = new UnityEngine.GameObject("Wave3PR6_NarrativeBacklogTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                ulong? requestedBeforeSeq = 999UL;
                int requestCount = 0;
                hud.OnNarrativeBacklogRequested += bSeq => { requestedBeforeSeq = bSeq; requestCount++; };

                // 3. Delta `narrative` backlog passthrough via MushWsClient
                using (var ws = new MushWsClient())
                {
                    ws.OnPanelUpdated += (feed, data) =>
                    {
                        if (feed == "narrative")
                            hud.ApplyNarrativeBacklog(data);
                    };

                    hud.AppendLogLine("<b>Kestrel</b> says, \"Current live line.\"");

                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                        ""seq"": 50, ""ts"": 1700000100000, ""t"": ""panel"",
                        ""d"": {
                            ""feed"": ""narrative"", ""kind"": ""delta"",
                            ""data"": {
                                ""backlog"": {
                                    ""before_seq"": null, ""more"": true, ""evicted"": false,
                                    ""lines"": [
                                        { ""seq"": 45, ""ts"": 1700000050000, ""kind"": ""say"", ""actor"": { ""id"": ""#22"", ""name"": ""Voss"" }, ""text"": { ""spans"": [ { ""text"": ""Dockmaster cleared berth 4."" } ] } },
                                        { ""seq"": 41, ""ts"": 1700000010000, ""kind"": ""pose"", ""actor"": { ""id"": ""#23"", ""name"": ""Mira"" }, ""text"": { ""spans"": [ { ""text"": ""waves from the gantry."" } ] } }
                                    ]
                                }
                            }
                        }
                    }"));
                    ws.PumpMainThread();

                    Assert.AreEqual(2, hud.NarrativeBacklogLoadedCount);
                    Assert.AreEqual(41UL, hud.OldestNarrativeBacklogSeq);
                    Assert.IsTrue(hud.HasMoreNarrativeBacklog);

                    // Paging earlier dispatches before_seq = 41
                    hud.RequestEarlierNarrativeBacklog();
                    Assert.AreEqual(1, requestCount);
                    Assert.AreEqual(41UL, requestedBeforeSeq);

                    // Duplicate seqs are ignored cleanly
                    ws.InjectSyntheticEnvelope(MushProtocol.ParseEnvelope(@"{
                        ""seq"": 51, ""ts"": 1700000105000, ""t"": ""panel"",
                        ""d"": {
                            ""feed"": ""narrative"", ""kind"": ""delta"",
                            ""data"": {
                                ""backlog"": {
                                    ""before_seq"": 41, ""more"": false, ""evicted"": false,
                                    ""lines"": [
                                        { ""seq"": 41, ""ts"": 1700000010000, ""kind"": ""pose"", ""actor"": { ""id"": ""#23"", ""name"": ""Mira"" }, ""text"": { ""spans"": [ { ""text"": ""waves from the gantry."" } ] } }
                                    ]
                                }
                            }
                        }
                    }"));
                    ws.PumpMainThread();
                    Assert.AreEqual(2, hud.NarrativeBacklogLoadedCount, "Duplicate backlog sequence IDs must not double-count");
                    Assert.IsFalse(hud.HasMoreNarrativeBacklog);
                }

                // 4. Comms search filter matches plain text & ignores rich-text color tags
                hud.SetCommsSearchQuery("berth 4");
                Assert.AreEqual(1, hud.VisibleFilteredLineCount, "Search for 'berth 4' should isolate Voss's backlog line");

                hud.SetCommsSearchQuery("color");
                Assert.AreEqual(0, hud.VisibleFilteredLineCount, "Search for 'color' must not false-match Unity <color=...> markup tags");

                hud.SetCommsSearchQuery(string.Empty);
                Assert.GreaterOrEqual(hud.VisibleFilteredLineCount, 3);

                // 5. Timestamps toggle
                hud.SetShowCommsTimestamps(true, persist: false);
                Assert.IsTrue(hud.ShowCommsTimestamps);
                hud.AppendLogLine("[SYS] Telemetry link verified.");
                hud.SetCommsSearchQuery("Telemetry link");
                Assert.AreEqual(1, hud.VisibleFilteredLineCount);

                // Reset clears backlog state & search query cleanly
                hud.ResetSessionState();
                Assert.AreEqual(0, hud.NarrativeBacklogLoadedCount);
                Assert.IsNull(hud.OldestNarrativeBacklogSeq);
                Assert.AreEqual(string.Empty, hud.CommsSearchQuery);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void BridgeHoloProjectorRingsSolarOrbitsAndRoverSlopeHeadlights_RenderAndAlignCleanly()
        {
            var rootGo = new UnityEngine.GameObject("Wave3PR7_HoloAndRoverTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();

                // 1. 3D Bridge Holo-Table volumetric range rings, sweep arm & 3D solar orbital tracks
                var shipGo = new UnityEngine.GameObject("TestBridgeShip");
                shipGo.transform.SetParent(rootGo.transform);
                var ship = shipGo.AddComponent<ShipInteriorController>();
                ship.BuildInterior(catalog, "tacsim.unit.dray");

                Assert.AreEqual(3, ship.HoloGridRingCount, "Bridge holo-table must render 3 concentric tactical range rings");
                Assert.IsTrue(ship.HasHoloSweepArm, "Bridge holo-table must render animated radar sweep arm");

                // Invariant check: HoloTankContacts childCount remains strictly 4 for ownship + 3 live contacts!
                ship.PopulateHoloBlips(JsonNode.Parse(@"{
                    ""range_km"": 500,
                    ""contacts"": [
                        { ""id"": ""own"", ""you"": true, ""x_km"": 0, ""y_km"": 0, ""z_km"": 0 },
                        { ""id"": ""C1"", ""kind"": ""ship"", ""relation"": ""hostile"", ""x_km"": 120, ""y_km"": 40, ""z_km"": 15 },
                        { ""id"": ""C2"", ""kind"": ""ship"", ""relation"": ""friendly"", ""x_km"": -90, ""y_km"": 60, ""z_km"": -10 },
                        { ""id"": ""A"", ""kind"": ""rock"", ""x_km"": 50, ""y_km"": -80, ""z_km"": 5 }
                    ]
                }"));
                var holoContacts = shipGo.transform.Find("HoloTankPedestal")?.Find("HoloTankContacts");
                Assert.IsNotNull(holoContacts);
                Assert.AreEqual(4, holoContacts.childCount, "HoloTankContacts childCount invariant must stay exact (ownship + 3 contacts)");
                Assert.AreEqual(0, ship.HoloSolarOrbitRingCount, "Local tactical bubble mode must hide solar orbit rings");

                // Switch to Solar System mode -> renders 3D orbital track rings for primary solar bodies
                var chart = TacSimSolarChartFrame.FromJson(JsonNode.Parse(@"{
                    ""ic_time"": ""2350-05-01"",
                    ""vantage"": { ""body"": ""ceres"", ""port"": ""ceres"" },
                    ""scale"": { ""r0_au"": 1.5, ""au_max"": 12.0 },
                    ""bodies"": [
                        { ""id"": ""earth"", ""label"": ""Earth"", ""r_au"": 1.0, ""lon_deg"": 15.0, ""lat_deg"": 0.0, ""here"": false },
                        { ""id"": ""mars"", ""label"": ""Mars"", ""r_au"": 1.52, ""lon_deg"": 110.0, ""lat_deg"": 1.8, ""here"": false },
                        { ""id"": ""ceres"", ""label"": ""Ceres"", ""r_au"": 2.77, ""lon_deg"": 230.0, ""lat_deg"": 10.6, ""here"": true }
                    ],
                    ""routes"": []
                }"));
                ship.ApplySolarChartFrame(chart);
                ship.SetHoloDisplayMode(BridgeHoloMode.SolarSystem);
                Assert.AreEqual(3, ship.HoloSolarOrbitRingCount, "Solar System holo-table mode must draw concentric 3D orbital rings per primary body");

                // Alert escalation tints holo-grid rings without throwing
                ship.SetAlertLevel(ShipAlertLevel.RedAlert);
                Assert.AreEqual(ShipAlertLevel.RedAlert, ship.ActiveAlertLevel);

                // 2. Surface Prospector Rover dual headlights + terrain-slope alignment
                var surfGo = new UnityEngine.GameObject("TestSurfaceStage");
                surfGo.transform.SetParent(rootGo.transform);
                var surf = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surf.BuildSurfaceStage(catalog, "occator_rim");

                Assert.AreEqual(2, surf.RoverHeadlightCount, "Prospector rover must mount dual forward spotlights");
                Assert.IsFalse(surf.IsRoverDustPlumeActive, "Stationary rover must keep regolith dust plume hidden");

                // Synthetic sloped terrain grid -> ComputeTerrainAlignedRotation yields non-zero slope tilt clamped <= 28 deg
                var grid = new DecodedTerrainGrid
                {
                    Width = 8,
                    Height = 8,
                    CellSizeM = 10f,
                    ElevMeters = new float[64],
                    Classes = new TerrainCellClass[64]
                };
                for (int y = 0; y < 8; y++)
                {
                    for (int x = 0; x < 8; x++)
                        grid.ElevMeters[y * 8 + x] = y * 3.5f + x * 1.2f; // steady uphill ramp
                }
                var aligned = TacSim.Surface.SurfaceMiningController.ComputeTerrainAlignedRotation(grid, new UnityEngine.Vector3(35f, 0f, 35f), headingDeg: 0f, out float tiltDeg);
                Assert.Greater(tiltDeg, 5f, "Sloped terrain must tilt rover chassis onto local grade");
                Assert.LessOrEqual(tiltDeg, 40f);
                Assert.Greater(UnityEngine.Quaternion.Angle(UnityEngine.Quaternion.identity, aligned), 5f);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void GroundChartModalAndMultiRigQuarryOps_DecodesBodyGroundSwitchesRigsAndDispatchesPlans()
        {
            // 1. Equirectangular lat/lon projection & wash palette
            var uvEq = TacSim.UI.GroundChartFrame.ProjectLatLonToUv(latMdeg: 0, lonMdeg: 180000);
            Assert.AreEqual(0.5f, uvEq.x, 0.001f);
            Assert.AreEqual(0.5f, uvEq.y, 0.001f);
            Assert.AreEqual(218, TacSim.UI.TacSimGroundChartModal.WashColorForGlyph('*').r);

            var rootGo = new UnityEngine.GameObject("Wave4PR1_GroundChartTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.GroundChart, "BuildHud must wire [G] Planetary Ground Survey modal");

                string lastCmd = null;
                string lastRefreshFam = null;
                string lastSailingVenue = null;
                hud.GroundChart.OnCommandRequested += c => lastCmd = c;
                hud.GroundChart.OnRefreshGroundChartRequested += f => lastRefreshFam = f;
                hud.GroundChart.OnOpenSailingPlanRequested += v => lastSailingVenue = v;

                // 2. Decode live `body.ground` frame with RLE rows, ports & sites
                hud.GroundChart.ApplyGroundFrame(JsonNode.Parse(@"{
                    ""width"": 8, ""height"": 4, ""title"": ""Ceres — Volatiles Wash"",
                    ""rows"": [ ""8."", ""4-4+"", ""2+4*2."", ""8."" ],
                    ""ground"": {
                        ""body"": ""ceres"", ""name"": ""Ceres"",
                        ""family"": ""volatiles"", ""family_word"": ""volatiles"",
                        ""families"": [ ""volatiles"", ""metals"", ""strategic"" ],
                        ""footer"": ""Registry R0 survey — Ceres"",
                        ""ports"": [
                            { ""glyph"": ""P"", ""port"": ""Ceres Freeport"", ""lat_mdeg"": 12000, ""lon_mdeg"": 90000, ""drawn"": true }
                        ],
                        ""sites"": [
                            { ""letter"": ""A"", ""site"": ""Occator Rim"", ""venue"": ""occator"", ""lat_mdeg"": 19800, ""lon_mdeg"": 239300, ""survey"": ""rich volatiles"", ""tenure"": ""open"", ""passage"": ""1h 20m / 0.8 t"", ""here"": true, ""best"": ""rich"", ""dark"": false, ""best_line"": ""Best: rich brines at rim"" },
                            { ""letter"": ""B"", ""site"": ""Juling Shadow"", ""venue"": ""juling"", ""lat_mdeg"": -35900, ""lon_mdeg"": 168200, ""survey"": ""fair ice"", ""tenure"": ""claimed"", ""passage"": ""2h 10m / 1.3 t"", ""here"": false, ""best"": ""fair"", ""dark"": true, ""best_line"": ""Best: dark permanent ice"" }
                        ],
                        ""lines"": [ ""A Occator Rim — rich volatiles"" ]
                    }
                }"));

                Assert.IsNotNull(hud.GroundChart.CurrentFrame);
                Assert.AreEqual(2, hud.GroundChart.CurrentFrame.Sites.Count);
                Assert.AreEqual("occator", hud.GroundChart.SelectedSiteVenue, "Default selection should highlight active `here` landing site");

                // Selecting site B & dispatching verified commands
                hud.GroundChart.SelectSite("juling", "B");
                hud.GroundChart.DispatchPlanPassage();
                Assert.AreEqual("+ship/plan juling", lastCmd);
                hud.GroundChart.DispatchQueueDepart();
                Assert.AreEqual("+ship/order depart juling", lastCmd);
                hud.GroundChart.DispatchChartWindow();
                Assert.AreEqual("+chart juling", lastCmd);

                hud.GroundChart.SelectFamily("metals");
                Assert.AreEqual("metals", hud.GroundChart.ActiveFamily);
                Assert.AreEqual("metals", lastRefreshFam);

                hud.GroundChart.SetOpen(true);
                Assert.IsTrue(hud.IsModalOpen);
                hud.GroundChart.DispatchOpenSailingPlan();
                Assert.IsFalse(hud.GroundChart.IsOpen);
                Assert.AreEqual("juling", lastSailingVenue);

                // Esc closes GroundChart cleanly
                hud.GroundChart.SetOpen(true);
                Assert.IsTrue(hud.TryCloseTopmostModal());
                Assert.IsFalse(hud.GroundChart.IsOpen);

                // 3. Multi-Rig Quarry Operations (`site.map` with 2 rigs, run, plan, queued & presence_line)
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var surfGo = new UnityEngine.GameObject("Wave4PR1_MultiRigSurface");
                surfGo.transform.SetParent(rootGo.transform);
                var surf = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surf.BuildSurfaceStage(catalog, "occator_rim");

                int selectedSlotEvent = 0;
                surf.OnRigSlotSelected += s => selectedSlotEvent = s;

                surf.ApplySiteMapFrame(JsonNode.Parse(@"{
                    ""width"": 4, ""height"": 4, ""title"": ""Occator Rim"",
                    ""rows"": [ ""4,"", ""4,"", ""4,"", ""4,"" ],
                    ""you"": [ 1, 1 ],
                    ""site"": {
                        ""site"": ""Occator Rim"", ""family_word"": ""volatiles"", ""addressed"": 1,
                        ""grid"": { ""x0"": 120, ""y0"": 120, ""span"": 1, ""cell_cm"": 1000 },
                        ""hold"": { ""room_t"": 74 },
                        ""presence_line"": ""Presence: 1 other flight in basin."",
                        ""rigs"": [
                            {
                                ""slot"": 1, ""mark"": ""@"", ""cell"": [121, 121], ""charge_pct"": 82, ""hopper"": ""3 of 6 t"",
                                ""line"": ""Rig 1 (82%, 3/6 t)"",
                                ""run"": { ""text"": ""Drilling pocket A (2/4 cycles)"" },
                                ""plan"": { ""text"": ""Plan #3 live: step 2/5"" },
                                ""queued"": ""Unload queued at pad""
                            },
                            {
                                ""slot"": 2, ""mark"": ""2"", ""cell"": [122, 122], ""charge_pct"": 95, ""hopper"": ""0 of 6 t"",
                                ""line"": ""Rig 2 (95%, 0/6 t)""
                            }
                        ]
                    }
                }"));

                Assert.AreEqual(2, surf.OwnedRigs.Count, "All owned rigs from site.rigs[] must be retained for multi-rig switching");
                Assert.AreEqual("Drilling pocket A (2/4 cycles)", surf.ActiveRunSummary);
                Assert.AreEqual("Plan #3 live: step 2/5", surf.ActivePlanSummary);
                Assert.AreEqual("Unload queued at pad", surf.ActiveQueuedSummary);
                Assert.AreEqual("Presence: 1 other flight in basin.", surf.PresenceLine);

                surf.SelectRigSlot(2);
                Assert.AreEqual(2, surf.AddressedRigSlot);
                Assert.AreEqual(2, selectedSlotEvent);

                hud.ResetSessionState();
                Assert.IsNull(hud.GroundChart.CurrentFrame);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaDetachmentCommandDeck_ValidatesProjectionPrefillsSlotsAndDispatchesWilcoNegative()
        {
            // 1. Strict decoder parity with `web/src/lib/detachment.ts` & `crates/tacsim/src/detachment/live.rs`
            var validJson = JsonNode.Parse(@"{
                ""id"": ""42"", ""revision"": ""7"", ""term"": ""3"", ""leader_slot"": 1,
                ""all_order_prefill"": ""ORDER DT:42:7:3 ALL "",
                ""slots"": [
                    { ""slot"": 2, ""callsign"": ""R2"", ""pilot"": ""Ada"", ""controller"": ""player"", ""order_prefill"": ""ORDER DT:42:7:3 2 "" },
                    { ""slot"": 1, ""callsign"": ""R1"", ""pilot"": null, ""controller"": ""npc"", ""order_prefill"": ""ORDER DT:42:7:3 1 "" }
                ],
                ""my_order"": { ""sequence"": ""9"", ""from"": ""R1"", ""objective"": ""Hold ridge A"", ""response"": null },
                ""wilco_command"": ""WILCO DT:42:7 9"",
                ""negative_command"": ""NEGATIVE DT:42:7 9""
            }");
            Assert.IsTrue(TacSim.Arena.ArenaDetachmentState.TryDecode(validJson, out var det));
            Assert.AreEqual("42", det.Id);
            Assert.AreEqual(2, det.Slots.Count);
            Assert.AreEqual(1, det.Slots[0].Slot, "Slots must be sorted ascending by slot number");
            Assert.AreEqual("R1", det.Slots[0].Callsign);
            Assert.AreEqual(2, det.Slots[1].Slot);
            Assert.AreEqual("WILCO DT:42:7 9", det.WilcoCommand);

            // Tampered / trimmed / mismatched projected commands MUST reject the entire projection!
            var badPrefillJson = JsonNode.Parse(@"{
                ""id"": ""42"", ""revision"": ""7"", ""term"": ""3"", ""leader_slot"": 1,
                ""all_order_prefill"": ""ORDER DT:42:7:3 ALL"",
                ""slots"": [ { ""slot"": 1, ""callsign"": ""R1"", ""pilot"": null, ""controller"": ""player"", ""order_prefill"": ""ORDER DT:42:7:3 1 "" } ],
                ""my_order"": null, ""wilco_command"": null, ""negative_command"": null
            }");
            Assert.IsFalse(TacSim.Arena.ArenaDetachmentState.TryDecode(badPrefillJson, out _), "Missing trailing space in all_order_prefill must invalidate projection");

            var badWilcoJson = JsonNode.Parse(@"{
                ""id"": ""42"", ""revision"": ""7"", ""term"": ""3"", ""leader_slot"": 1,
                ""all_order_prefill"": ""ORDER DT:42:7:3 ALL "",
                ""slots"": [ { ""slot"": 1, ""callsign"": ""R1"", ""pilot"": null, ""controller"": ""player"", ""order_prefill"": ""ORDER DT:42:7:3 1 "" } ],
                ""my_order"": { ""sequence"": ""9"", ""from"": ""R1"", ""objective"": ""Hold ridge A"", ""response"": null },
                ""wilco_command"": ""WILCO DT:42:7 99"",
                ""negative_command"": ""NEGATIVE DT:42:7 9""
            }");
            Assert.IsFalse(TacSim.Arena.ArenaDetachmentState.TryDecode(badWilcoJson, out _), "Mismatched wilco_command sequence must invalidate projection");

            // 2. End-to-end OocArenaController + TacSimMissionOverlays Detachment Command Deck wiring
            var rootGo = new UnityEngine.GameObject("Wave4PR2_DetachmentTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);

                int newOrderEvents = 0;
                arena.OnNewDetachmentOrderReceived += _ => newOrderEvents++;

                string lastVerb = null;
                string lastPrefill = null;
                hud.Overlays.OnArenaCockpitVerbRequested += v => lastVerb = v;
                hud.Overlays.OnPrefillChatRequested += p => lastPrefill = p;

                var frame = JsonNode.Parse(@"{
                    ""match"": 77, ""seated"": true, ""callsign"": ""R2"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 24000, ""y_cm"": 128000, ""heading_deg"": 90, ""speed_kph"": 12,
                    ""weapons"": [], ""contacts"": [],
                    ""detachment"": {
                        ""id"": ""42"", ""revision"": ""7"", ""term"": ""3"", ""leader_slot"": 1,
                        ""all_order_prefill"": ""ORDER DT:42:7:3 ALL "",
                        ""slots"": [
                            { ""slot"": 1, ""callsign"": ""R1"", ""pilot"": ""Rex"", ""controller"": ""player"", ""order_prefill"": ""ORDER DT:42:7:3 1 "" },
                            { ""slot"": 2, ""callsign"": ""R2"", ""pilot"": ""Ada"", ""controller"": ""player"", ""order_prefill"": ""ORDER DT:42:7:3 2 "" }
                        ],
                        ""my_order"": { ""sequence"": ""9"", ""from"": ""R1"", ""objective"": ""Flank North Ridge"", ""response"": null },
                        ""wilco_command"": ""WILCO DT:42:7 9"",
                        ""negative_command"": ""NEGATIVE DT:42:7 9""
                    }
                }");

                arena.ApplyLiveCockpitFrame(frame);
                Assert.AreEqual(1, newOrderEvents, "New unacknowledged detachment order must fire OnNewDetachmentOrderReceived once");
                // Applying the same sequence again must NOT duplicate notifications
                arena.ApplyLiveCockpitFrame(frame);
                Assert.AreEqual(1, newOrderEvents, "Repeated cockpit ticks for the same order sequence must not re-fire alert");

                Assert.IsTrue(hud.Overlays.IsDetachmentBarVisible);
                Assert.IsTrue(hud.Overlays.IsDetachmentOrderBannerVisible);
                Assert.AreEqual(3, hud.Overlays.VisibleDetachmentSlotButtonCount, "ALL + 2 detachment slots must be visible");
                StringAssert.Contains("Flank North Ridge", hud.Overlays.DetachmentOrderText);

                // Prefill ALL and Slot 2 commands into chat
                hud.Overlays.PrefillDetachmentSlot(0);
                Assert.AreEqual("ORDER DT:42:7:3 ALL ", lastPrefill);
                hud.Overlays.PrefillDetachmentSlot(2);
                Assert.AreEqual("ORDER DT:42:7:3 2 ", lastPrefill);

                // Squad advance routes through exact AllOrderPrefill when detachment projection is valid
                string advCmd = hud.Overlays.ResolveSquadAdvanceCommand();
                StringAssert.StartsWith("ORDER DT:42:7:3 ALL advance ", advCmd);

                // WILCO & NEGATIVE dispatch exact server-projected commands
                hud.Overlays.DispatchDetachmentWilco();
                Assert.AreEqual("WILCO DT:42:7 9", lastVerb);
                hud.Overlays.DispatchDetachmentNegative();
                Assert.AreEqual("NEGATIVE DT:42:7 9", lastVerb);

                // Invalid detachment projection disables slot/order controls cleanly
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 77, ""seated"": true, ""callsign"": ""R2"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""weapons"": [], ""contacts"": [],
                    ""detachment"": { ""id"": ""bad"", ""revision"": ""7"", ""term"": ""3"", ""slots"": [] }
                }"));
                Assert.IsNull(arena.State.Detachment);
                Assert.IsTrue(arena.State.DetachmentProjectionInvalid);
                Assert.IsTrue(hud.Overlays.IsDetachmentBarVisible);
                Assert.IsFalse(hud.Overlays.IsDetachmentOrderBannerVisible);
                Assert.AreEqual(0, hud.Overlays.VisibleDetachmentSlotButtonCount);
                StringAssert.StartsWith("order all advance ", hud.Overlays.ResolveSquadAdvanceCommand());

                arena.ResetSessionState();
                Assert.IsNull(arena.State.Detachment);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void StationAndShip3DAvatarTargeting_SelectsWithRingCyclesTabAndDispatchesActions()
        {
            var rootGo = new UnityEngine.GameObject("Wave4PR3_AvatarTargetingTest");
            try
            {
                var coord = rootGo.AddComponent<TacSim.Core.TacSimGameCoordinator>();
                coord.EnsureSubsystems();

                Assert.IsNotNull(coord.TargetRing, "Coordinator must initialize TacSimTargetRing3D");
                Assert.IsFalse(coord.TargetRing.IsRingVisible, "Selection ring must stay hidden until a target is focused/selected");

                // 1. Spawn a station NPC + a remote player pilot in the active room via RoomView
                var rv = new MushRoomView
                {
                    Id = "#10",
                    Name = "Arrivals Hall",
                    Atlas = new AtlasRoomAmbient { Pack = "neighborhood.ceres_freeport", Entry = "room.c03_arrivals" }
                };
                rv.Contents.Add(new MushObjRef { Id = "#101", Name = "Vance" });
                rv.Contents.Add(new MushObjRef { Id = "#102", Name = "PilotAria" });
                rv.NpcIds.Add("#101");
                rv.PlayerIds.Add("#102");

                coord.StationWorld.ApplyRoomView(rv, "#999");

                var targets = coord.StationWorld.GetActiveRoomTargetables();
                Assert.GreaterOrEqual(targets.Count, 2, "Station room must expose spawned NPC & pilot targetables");

                WorldInteractable npcWi = null;
                WorldInteractable pilotWi = null;
                for (int i = 0; i < targets.Count; i++)
                {
                    if (targets[i].actorId == "#101") npcWi = targets[i];
                    else if (targets[i].actorId == "#102") pilotWi = targets[i];
                }
                Assert.IsNotNull(npcWi);
                Assert.IsNotNull(pilotWi);
                Assert.IsFalse(npcWi.isRemotePlayer);
                Assert.IsTrue(pilotWi.isRemotePlayer);

                // 2. Single LMB click selects remote pilot from across the room (> 10m) WITHOUT triggering interaction modal
                coord.Player.TeleportTo(pilotWi.transform.position + new UnityEngine.Vector3(0f, 0f, -12f), 0f);
                bool triggered = coord.Player.HandleLeftClickOnInteractable(pilotWi, 10.0f);
                Assert.IsFalse(triggered, "Single click from across room must select target rather than firing modal");
                Assert.AreSame(pilotWi, coord.Player.SelectedTarget);
                Assert.IsTrue(coord.TargetRing.IsRingVisible, "Selecting a remote pilot must illuminate the 3D ground ring");
                Assert.AreSame(pilotWi.transform, coord.TargetRing.CurrentTargetTransform);
                Assert.AreEqual(TacSim.World.TacSimTargetRing3D.ColorForAffinity(TacSim.World.TargetRingAffinity.RemotePilot), coord.TargetRing.RingColor);

                // Target Unit Frame shows Pilot crest + PAGE / LOOK / MAIL action pills!
                Assert.IsTrue(coord.Hud.UnitFrames.IsTargetVisible);
                Assert.AreEqual("PAGE", coord.Hud.UnitFrames.TargetAction1Label);
                Assert.AreEqual("LOOK", coord.Hud.UnitFrames.TargetAction2Label);
                Assert.AreEqual("MAIL", coord.Hud.UnitFrames.TargetAction3Label);

                // Clicking MAIL opens Social & Mail modal addressed to PilotAria
                coord.Hud.UnitFrames.TriggerTargetAction(2);
                Assert.IsTrue(coord.Hud.SocialModal.IsOpen);
                Assert.AreEqual(TacSim.UI.SocialModalTab.Mail, coord.Hud.SocialModal.ActiveTab);
                coord.Hud.SocialModal.SetOpen(false);

                // 3. Selecting an NPC switches ground ring to emerald and populates TALK / LOOK / SAY
                coord.Player.SelectTarget(npcWi);
                Assert.AreSame(npcWi.transform, coord.TargetRing.CurrentTargetTransform);
                Assert.AreEqual(TacSim.World.TacSimTargetRing3D.ColorForAffinity(TacSim.World.TargetRingAffinity.Npc), coord.TargetRing.RingColor);
                Assert.AreEqual("TALK", coord.Hud.UnitFrames.TargetAction1Label);
                Assert.AreEqual("LOOK", coord.Hud.UnitFrames.TargetAction2Label);
                Assert.AreEqual("SAY", coord.Hud.UnitFrames.TargetAction3Label);

                // 4. Double-click within reach triggers interaction directly
                coord.Player.TeleportTo(npcWi.transform.position + new UnityEngine.Vector3(0f, 0f, -2.5f), 0f);
                bool interactedOnNpc = false;
                var prevHandler = npcWi.OnInteracted;
                npcWi.OnInteracted = _ => interactedOnNpc = true;
                try
                {
                    // First click updates click timer; second click within 0.45s triggers!
                    coord.Player.HandleLeftClickOnInteractable(npcWi, 20.0f);
                    bool dbl = coord.Player.HandleLeftClickOnInteractable(npcWi, 20.2f);
                    Assert.IsTrue(dbl && interactedOnNpc, "Double-click on selected target within reach must trigger interaction");
                }
                finally
                {
                    npcWi.OnInteracted = prevHandler;
                }

                // 5. [Tab] cycling advances cleanly between room targets
                var beforeTab = coord.Player.SelectedTarget;
                coord.CycleWalkTargetForTest();
                Assert.IsNotNull(coord.Player.SelectedTarget);
                Assert.AreNotSame(beforeTab, coord.Player.SelectedTarget, "[Tab] in station walk must cycle to the next room targetable");

                // 6. Clearing target hides both the 3D ground ring and Target Unit Frame
                coord.Player.ClearSelectedTarget();
                Assert.IsFalse(coord.TargetRing.IsRingVisible);
                Assert.IsFalse(coord.Hud.UnitFrames.IsTargetVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void LiveSheetVitalsAndUnitChassis_DecodesSchemaGuardsVitalsRevisionsAndRendersUnit()
        {
            // 1. Query::Sheet wire encoding
            string sheetReq = MushProtocol.BuildSheetQueryCommand(9UL);
            var parsedReq = JsonNode.Parse(sheetReq);
            Assert.AreEqual("req", parsedReq.Str("t"));
            Assert.AreEqual(9UL, parsedReq["d"].ULong("id", 0));
            Assert.AreEqual("sheet", parsedReq["d"]["q"].Str("t"));

            using (var ws = new MushWsClient())
            {
                ulong qid = ws.SendSheetQuery();
                Assert.Greater(qid, 0UL);
                Assert.IsTrue(ws.TryDequeueOutbound(out string rawOut));
                StringAssert.Contains("\"sheet\"", rawOut);
            }

            var rootGo = new UnityEngine.GameObject("Wave4PR4_SheetVitalsUnitTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsNotNull(hud.CharacterSheet);

                // 2. Schema-driven character sheet (`*.sheet`) with dots, track, enum, map, & hidden staff_only field
                bool sheetOk = hud.CharacterSheet.ApplySheetPanel(JsonNode.Parse(@"{
                    ""target"": ""#12"",
                    ""approved"": { ""state"": ""approved"", ""approved"": true },
                    ""schema"": {
                        ""ruleset"": ""demo_ruleset"", ""version"": 2,
                        ""sections"": [
                            {
                                ""id"": ""core"", ""label"": ""Attributes"", ""layout"": ""grid"",
                                ""fields"": [
                                    { ""id"": ""str"", ""label"": ""Strength"", ""ty"": { ""kind"": ""dots"", ""min"": 1, ""max"": 5 }, ""default"": 1 },
                                    { ""id"": ""role"", ""label"": ""Role"", ""ty"": { ""kind"": ""enum"", ""options"": [""pilot"", ""medic""] }, ""default"": ""pilot"" },
                                    { ""id"": ""secret"", ""label"": ""Staff Note"", ""staff_only"": true, ""ty"": { ""kind"": ""text"", ""max_len"": 80 }, ""default"": """" }
                                ]
                            },
                            {
                                ""id"": ""health"", ""label"": ""Tracks"", ""layout"": ""tracks"",
                                ""fields"": [
                                    {
                                        ""id"": ""wound"", ""label"": ""Condition"",
                                        ""ty"": { ""kind"": ""track"", ""size"": { ""fixed"": 4 }, ""levels"": [ { ""id"": ""bruised"", ""label"": ""Bruised"", ""penalty"": 0 }, { ""id"": ""hurt"", ""label"": ""Hurt"", ""penalty"": -1 } ] },
                                        ""default"": []
                                    }
                                ]
                            },
                            {
                                ""id"": ""powers"", ""label"": ""Specialties"", ""layout"": ""map"",
                                ""fields"": [
                                    { ""id"": ""specs"", ""label"": ""Specialties"", ""ty"": { ""kind"": ""map"", ""value"": { ""kind"": ""dots"", ""min"": 0, ""max"": 5 } }, ""default"": {} }
                                ]
                            }
                        ]
                    },
                    ""data"": {
                        ""str"": 4,
                        ""role"": ""pilot"",
                        ""wound"": [""lethal"", ""bashing"", null, null],
                        ""specs"": { ""Gunnery"": 3, ""Astrogation"": 2 }
                    }
                }"));
                Assert.IsTrue(sheetOk);
                Assert.IsNotNull(hud.CharacterSheet.LiveSheet);
                Assert.IsTrue(hud.CharacterSheet.LiveSheet.Approved);
                hud.CharacterSheet.SelectTab(TacSim.UI.CharacterSheetTab.RulesetAndUnit);
                StringAssert.Contains("DEMO_RULESET v2", hud.CharacterSheet.RulesetSheetSnapshot);
                StringAssert.Contains("●●●●○ 4/5", hud.CharacterSheet.RulesetSheetSnapshot);
                StringAssert.Contains("[X][/][ ][ ] Hurt (-1)", hud.CharacterSheet.RulesetSheetSnapshot);
                StringAssert.Contains("Astrogation", hud.CharacterSheet.RulesetSheetSnapshot);
                StringAssert.DoesNotContain("Staff Note", hud.CharacterSheet.RulesetSheetSnapshot, "Unset staff_only fields stripped by server must stay hidden");

                // 3. Monotonic `*.vitals` store: stale revision ignored, revision 0 & subject change & null clear honored
                Assert.IsTrue(hud.CharacterSheet.ApplyVitalsPanel("tactics.vitals", JsonNode.Parse(@"{
                    ""subject"": { ""id"": ""#50"", ""name"": ""Ceres Dray"" }, ""revision"": 5,
                    ""vitals"": [
                        { ""id"": ""heat"", ""label"": ""Thermal"", ""role"": ""pool"", ""state"": ""warn"", ""value"": { ""kind"": ""bounded"", ""current"": 65, ""min"": 0, ""max"": 100 } },
                        { ""id"": ""hull"", ""label"": ""Hull"", ""role"": ""dominant"", ""state"": ""ok"", ""value"": { ""kind"": ""text"", ""text"": ""Nominal"" } }
                    ]
                }")));
                Assert.AreEqual(1, hud.CharacterSheet.LiveVitals.GroupCount);
                StringAssert.Contains("Thermal", hud.CharacterSheet.PilotSummarySnapshot);
                StringAssert.Contains("65/100 (65%)", hud.CharacterSheet.UnitChassisSnapshot);

                // Stale lower revision for same subject MUST be rejected
                Assert.IsFalse(hud.CharacterSheet.ApplyVitalsPanel("tactics.vitals", JsonNode.Parse(@"{
                    ""subject"": { ""id"": ""#50"", ""name"": ""Ceres Dray"" }, ""revision"": 3,
                    ""vitals"": []
                }")));
                // Watermark reset allows next frame after reconnect/world restart
                hud.CharacterSheet.ResetVitalsWatermarks();
                Assert.IsTrue(hud.CharacterSheet.ApplyVitalsPanel("tactics.vitals", JsonNode.Parse(@"{
                    ""subject"": { ""id"": ""#50"", ""name"": ""Ceres Dray"" }, ""revision"": 1,
                    ""vitals"": [
                        { ""id"": ""heat"", ""label"": ""Thermal"", ""role"": ""pool"", ""state"": ""ok"", ""value"": { ""kind"": ""bounded"", ""current"": 10, ""min"": 0, ""max"": 100 } }
                    ]
                }")));
                // Null payload clears group cleanly
                Assert.IsTrue(hud.CharacterSheet.ApplyVitalsPanel("tactics.vitals", JsonNode.Null));
                Assert.AreEqual(0, hud.CharacterSheet.LiveVitals.GroupCount);

                // 4. Tactical Unit Chassis (`tactics.unit`) keyframe
                Assert.IsTrue(hud.CharacterSheet.ApplyUnitChassisPanel(JsonNode.Parse(@"{
                    ""object"": { ""id"": ""#77"", ""name"": ""Line Tank R1"" },
                    ""schema"": ""tacsim:unit:line_tank@1#abc"", ""revision"": 12, ""state"": ""warn"", ""record"": ""HELD"",
                    ""mobility"": { ""mode"": ""tracked"", ""switching"": null, ""modes"": [] },
                    ""cells"": [
                        { ""addr"": ""unit/heat"", ""label"": ""Heat"", ""current"": 340, ""min"": 0, ""max"": 1000, ""state"": ""ok"" },
                        { ""addr"": ""facet/hull/front"", ""label"": ""Front"", ""current"": 72, ""min"": 0, ""max"": 100, ""state"": ""warn"" }
                    ],
                    ""sections"": [
                        { ""id"": ""hull"", ""condition"": ""scarred"", ""state"": ""warn"", ""facets"": [ { ""id"": ""front"", ""cells"": [""facet/hull/front""] } ], ""slots"": [] }
                    ],
                    ""components"": [
                        { ""id"": ""main_gun"", ""definition"": ""railgun"", ""install"": ""turret"", ""condition"": ""operational"", ""state"": ""ok"", ""asset"": false }
                    ],
                    ""capabilities"": [ { ""id"": ""direct_fire"", ""source"": ""main_gun"" } ],
                    ""truncated"": false
                }")));
                Assert.IsNotNull(hud.CharacterSheet.LiveUnitChassis);
                StringAssert.Contains("Line Tank R1", hud.CharacterSheet.UnitChassisSnapshot);
                StringAssert.Contains("scarred", hud.CharacterSheet.UnitChassisSnapshot);
                StringAssert.Contains("front:72/100", hud.CharacterSheet.UnitChassisSnapshot);
                StringAssert.Contains("direct_fire", hud.CharacterSheet.UnitChassisSnapshot);

                hud.CharacterSheet.ResetSessionState();
                Assert.IsNull(hud.CharacterSheet.LiveSheet);
                Assert.IsNull(hud.CharacterSheet.LiveUnitChassis);
                Assert.AreEqual(0, hud.CharacterSheet.LiveVitals.GroupCount);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaPredictiveHelmAndCoalescedThrottle_MatchesTuiWebPhysicsAndLeash()
        {
            // 1. Predictive Helm: 1-deg tap precision, continuous turn rate, ORDER_GAP_MS lead orders, and leash clamp
            var helm = new TacSim.Arena.ArenaPredictiveHelm();
            helm.Observe(new TacSim.Arena.ArenaHelmFrame
            {
                TMs = 1_000,
                HeadingMdeg = 90_000,
                Tempo = "alert",
                TurnRateMdegS = 30_000,
                CanTurn = true
            }, nowMs: 1_000);
            Assert.AreEqual(90f, helm.PredictedHeadingDeg(1_000).Value, 0.01f);

            // Short tap right (50ms) orders exactly 1 degree (HEADING 91 QUIET) and holds 91
            helm.Start(+1, 1_000);
            var tapOrders = helm.TakeOrders();
            CollectionAssert.AreEqual(new[] { "HEADING 91 QUIET" }, tapOrders);
            helm.Stop(1_050);
            Assert.AreEqual(91_000, helm.OrderedMdeg, "Releasing inside first degree must preserve the 1-deg tap");

            // Held turn right swings smoothly at 30 deg/s after TAP_MS and emits lead orders every ORDER_GAP_MS
            helm.Observe(new TacSim.Arena.ArenaHelmFrame
            {
                TMs = 2_000,
                HeadingMdeg = 91_000,
                Tempo = "engaged",
                TurnRateMdegS = 30_000,
                CanTurn = true
            }, nowMs: 2_000);
            helm.Start(+1, 2_000);
            helm.TakeOrders();
            helm.Tick(2_250);
            var holdOrders = helm.TakeOrders();
            Assert.AreEqual(1, holdOrders.Count, "Tick at ORDER_GAP_MS must emit a lead HEADING QUIET order");
            StringAssert.StartsWith("HEADING ", holdOrders[0]);
            StringAssert.EndsWith(" QUIET", holdOrders[0]);

            // Leash prevents predicted heading from outrunning the last server frame beyond 1.5 pulses
            float farPred = helm.PredictedHeadingDeg(6_000).Value;
            float deltaFromSeen = System.Math.Abs(TacSim.Arena.ArenaPredictiveHelm.ArcMdeg(91_000, UnityEngine.Mathf.RoundToInt(farPred * 1000f))) * 0.001f;
            Assert.LessOrEqual(deltaFromSeen, 23.0f, "Engaged tempo (500ms) 1.5-pulse leash at 30 deg/s must clamp <= 22.5 deg");
            helm.Stop(6_000);

            // Immobile/wrecked hull cannot turn
            helm.Reset();
            helm.Observe(new TacSim.Arena.ArenaHelmFrame
            {
                TMs = 1_000,
                HeadingMdeg = 180_000,
                Tempo = "alert",
                TurnRateMdegS = 30_000,
                CanTurn = false
            }, nowMs: 1_000);
            helm.Start(+1, 1_000);
            Assert.AreEqual(180f, helm.PredictedHeadingDeg(1_500).Value, 0.01f, "Immobile hull must not visually swing");

            // 2. Coalesced Throttle: 10% step, 250ms coalescing, reverse clamp, immediate SPEED 0 QUIET
            var thr = new TacSim.Arena.ArenaCoalescedThrottle();
            thr.Observe(new TacSim.Arena.ArenaThrottleFrame { SpeedKph = 0, MaxKph = 54, ReverseKph = 20, Stuck = false });
            thr.Bump(+1, 1_000); // step = round(5.4) = 5 kph, sent immediately
            CollectionAssert.AreEqual(new[] { "SPEED 5 QUIET" }, thr.TakeOrders());

            // Rapid bumps inside 250ms coalesce into one order when the gap opens!
            thr.Bump(+1, 1_050); // 10
            thr.Bump(+1, 1_100); // 15
            Assert.AreEqual(0, thr.TakeOrders().Count, "Bumps inside SPEED_COALESCE_MS must coalesce");
            thr.Tick(1_250);
            CollectionAssert.AreEqual(new[] { "SPEED 15 QUIET" }, thr.TakeOrders());

            // All-stop bypasses coalesce gap immediately
            thr.Stop(1_260);
            CollectionAssert.AreEqual(new[] { "SPEED 0 QUIET" }, thr.TakeOrders());

            // Reverse clamping
            thr.Set(-99, 2_000);
            CollectionAssert.AreEqual(new[] { "SPEED -20 QUIET" }, thr.TakeOrders());
        }

        [Test]
        public void ArenaArmorPaperDoll_RendersAllShippedChassisWithTwoToneSeamsAndLiveDamage()
        {
            // 1. All shipped chassis resolve cleanly & compute 2-tone neighbor shading
            var tank = TacSim.Arena.TacSimArmorPaperDoll.FindForUnitType("tacsim.unit.line_tank");
            var biped = TacSim.Arena.TacSimArmorPaperDoll.FindForUnitType("tacsim.unit.biped_mk1");
            var scout = TacSim.Arena.TacSimArmorPaperDoll.FindForUnitType("tacsim.unit.scout_mk1");
            var drone = TacSim.Arena.TacSimArmorPaperDoll.FindForUnitType("tacsim.unit.drone_mk1");
            var wrecker = TacSim.Arena.TacSimArmorPaperDoll.FindForUnitType("tacsim.unit.tv_wrecker");
            Assert.AreEqual("top", tank.View);
            Assert.AreEqual("upright", biped.View);
            Assert.AreEqual(24, tank.Width);
            Assert.AreEqual(16, tank.Height);
            Assert.AreEqual(24, scout.Width);
            Assert.AreEqual(10, drone.Width);
            Assert.AreEqual(16, wrecker.Width);

            // Exact condition color ramp parity with web/src/lib/look.ts
            Assert.AreEqual(0x46, TacSim.Arena.TacSimArmorPaperDoll.ConditionColor(100, 100, 0).r);
            Assert.AreEqual(0xbe, TacSim.Arena.TacSimArmorPaperDoll.ConditionColor(100, 100, 0).g);
            Assert.AreEqual(0xe6, TacSim.Arena.TacSimArmorPaperDoll.ConditionColor(50, 100, 0).r);
            Assert.AreEqual(0xdc, TacSim.Arena.TacSimArmorPaperDoll.ConditionColor(10, 100, 0).r);
            Assert.AreEqual(0x4a, TacSim.Arena.TacSimArmorPaperDoll.ConditionColor(0, 100, 0).r);

            // 2. Live Arena cockpit hydrates full Sections[] and paints the RawImage Armor Paper-Doll
            var rootGo = new UnityEngine.GameObject("Wave4PR6_ArmorDollTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 88, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.biped_mk2"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""sections"": [
                        { ""id"": ""head"", ""condition"": ""intact"", ""internals"": 30, ""internals_max"": 30, ""facets"": [] },
                        { ""id"": ""torso"", ""condition"": ""scarred"", ""internals"": 45, ""internals_max"": 100, ""facets"": [] },
                        { ""id"": ""left_arm"", ""condition"": ""destroyed"", ""internals"": 0, ""internals_max"": 40, ""facets"": [] },
                        { ""id"": ""right_arm"", ""condition"": ""intact"", ""internals"": 40, ""internals_max"": 40, ""facets"": [] },
                        { ""id"": ""left_leg"", ""condition"": ""crippled"", ""internals"": 12, ""internals_max"": 60, ""facets"": [] },
                        { ""id"": ""right_leg"", ""condition"": ""intact"", ""internals"": 60, ""internals_max"": 60, ""facets"": [] }
                    ],
                    ""weapons"": [], ""contacts"": []
                }"));

                Assert.AreEqual(6, arena.State.Sections.Count, "All 6 mech sections must be preserved for per-part paper-doll rendering");
                Assert.IsTrue(hud.Overlays.IsArmorDollVisible, "Arena cockpit OwnPlatformCard must render live armor paper-doll schematic");
                Assert.AreEqual("tacsim.unit.biped_mk2", hud.Overlays.ActiveArmorDollUnitType);
                Assert.IsNotNull(hud.Overlays.ArmorDollTexture);
                Assert.AreEqual(24 * TacSim.Arena.TacSimArmorPaperDoll.CellScalePx + 2, hud.Overlays.ArmorDollTexture.width);
                Assert.AreEqual(16 * TacSim.Arena.TacSimArmorPaperDoll.CellScalePx + 2, hud.Overlays.ArmorDollTexture.height);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DCoverPropsAndTerrainTilt_BuildsBatchedCoverAlignsSlopesAndSyncsSightObjective()
        {
            // 1. Batched 3D Arena cover environment builder (woods, rocks, walls, water)
            var grid = new DecodedTerrainGrid
            {
                Id = "test_valley",
                DisplayName = "Test Valley",
                Biome = "temperate",
                Width = 12,
                Height = 12,
                CellSizeM = 10f,
                Classes = new TerrainCellClass[144],
                ElevMeters = new float[144]
            };
            for (int y = 0; y < 12; y++)
            {
                for (int x = 0; x < 12; x++)
                {
                    int idx = y * 12 + x;
                    grid.ElevMeters[idx] = y * 3.0f + x * 0.8f; // sloped valley hillside
                    if (x < 3) grid.Classes[idx] = TerrainCellClass.WoodsHeavy;
                    else if (x == 4) grid.Classes[idx] = TerrainCellClass.Rock;
                    else if (x == 6 && y >= 3 && y <= 8) grid.Classes[idx] = TerrainCellClass.Wall;
                    else if (x >= 10) grid.Classes[idx] = TerrainCellClass.Water;
                    else grid.Classes[idx] = TerrainCellClass.Ground;
                }
            }

            var rootGo = new UnityEngine.GameObject("Wave5PR1_ArenaEnvAndTiltTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, grid);

                Assert.Greater(arena.BuiltTreePropCount, 0, "WoodsHeavy cells must batch 3D canopy trees");
                Assert.Greater(arena.BuiltRockPropCount, 0, "Rock cells must batch 3D boulder outcrops");
                Assert.Greater(arena.BuiltWallPropCount, 0, "Wall cells must batch 3D ferrocrete cover barriers");
                Assert.Greater(arena.BuiltWaterCellCount, 0, "Water cells must batch translucent water sheet");

                // 2. Terrain-slope alignment: tank tilts sharply on hillside, biped damps tilt cleanly
                var tankRot = TacSim.Arena.OocArenaController.ComputeArenaChassisRotation(
                    grid, new UnityEngine.Vector3(60f, 0f, 60f), headingDeg: 0f, isBiped: false, out float tankTilt);
                var mechRot = TacSim.Arena.OocArenaController.ComputeArenaChassisRotation(
                    grid, new UnityEngine.Vector3(60f, 0f, 60f), headingDeg: 0f, isBiped: true, out float mechTilt);
                Assert.Greater(tankTilt, 5f, "Tank chassis must conform to local hillside grade");
                Assert.Less(mechTilt, tankTilt, "Gyro-stabilized biped mech must damp slope tilt relative to tracked tank");
                Assert.Greater(UnityEngine.Quaternion.Angle(UnityEngine.Quaternion.identity, tankRot), 5f);
                Assert.Greater(UnityEngine.Quaternion.Angle(UnityEngine.Quaternion.identity, mechRot), 1f);

                // 3. Authoritative sight_m, scope, and live dynamic objective sync
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 91, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 6000, ""y_cm"": 6000, ""heading_deg"": 45, ""speed_kph"": 28,
                    ""sight_m"": 180, ""scope"": ""hostile"", ""tempo"": ""engaged"",
                    ""objective"": { ""cell"": [7, 7], ""letter"": ""C"", ""radius_cells"": 3 },
                    ""weapons"": [], ""contacts"": []
                }"));

                Assert.AreEqual(180f, arena.State.SightMeters, 0.01f);
                Assert.AreEqual("hostile", arena.State.Scope);
                Assert.IsTrue(arena.State.HasDynamicObjective);
                Assert.AreEqual("C", arena.State.ObjectiveLetter);
                Assert.IsTrue(arena.IsDynamicObjectiveRingVisible, "Live frame.objective must display the 3D golden capture zone ring");
                Assert.IsTrue(arena.IsPlayerDustPlumeActive, "Moving tank at 28 kph must activate track dust plume");
                Assert.Greater(arena.LastPlayerSlopeTiltDeg, 4f);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DCombatVfxAndCameraShake_CyclesThreeCamerasDecaysShakeDrivesDamageSmokeTracersAndHitEvents()
        {
            var rootGo = new UnityEngine.GameObject("Wave5PR2_CombatVfxCameraTest");
            try
            {
                // 1. 3-mode Arena Camera Cycle (Chase -> Cupola -> Overhead -> Chase) & Impulse Camera Shake decay
                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var camRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                camRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();

                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                coord.CameraRig = camRig;

                camRig.SetMode(TacSim.Character.CameraRigMode.GroundVehicleChase);
                Assert.AreEqual(TacSim.Character.CameraRigMode.GroundVehicleChase, camRig.Mode);
                coord.ToggleGroundVehicleCameraMode();
                Assert.AreEqual(TacSim.Character.CameraRigMode.GroundVehicleCupola, camRig.Mode);
                coord.ToggleGroundVehicleCameraMode();
                Assert.AreEqual(TacSim.Character.CameraRigMode.GroundVehicleOverhead, camRig.Mode);
                Assert.AreEqual(8000f, camRig.ControlledCamera.farClipPlane, 0.1f, "Overhead camera must keep 8km battlefield draw distance");
                coord.ToggleGroundVehicleCameraMode();
                Assert.AreEqual(TacSim.Character.CameraRigMode.GroundVehicleChase, camRig.Mode);

                camRig.TriggerCameraShake(0.60f, 0.25f);
                Assert.IsTrue(camRig.IsCameraShakeActive);
                camRig.StepCameraShake(0.05f);
                Assert.Greater(camRig.CurrentShakeOffset.magnitude, 0.01f, "Active shake must emit non-zero camera impulse offset");
                camRig.StepCameraShake(0.30f);
                Assert.IsFalse(camRig.IsCameraShakeActive, "Shake must cleanly stop after duration elapses");
                Assert.AreEqual(UnityEngine.Vector3.zero, camRig.CurrentShakeOffset);

                // 2. Unit Damage Smoke (scarred/crippled) vs Wreck Smoke states
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var rig = TacSimVehicleVisualFactory.SpawnArenaCombatUnit(catalog, rootGo.transform, "tacsim.unit.line_tank", "B1", "B", UnityEngine.Vector3.zero, 90f);
                rig.SetDamageState("intact", false);
                Assert.IsFalse(rig.IsDamageSmokeActive);
                Assert.IsFalse(rig.WreckSmokeVisual.activeSelf);

                rig.SetDamageState("scarred", false);
                Assert.IsTrue(rig.IsDamageSmokeActive, "Scarred contact must emit battle damage smoke");
                Assert.IsFalse(rig.WreckSmokeVisual.activeSelf);

                rig.SetDamageState("crippled", false);
                Assert.IsTrue(rig.IsDamageSmokeActive, "Crippled contact must emit intensified damage smoke");
                Assert.Greater(rig.DamageSmokeVisual.transform.localScale.x, 1.2f);

                rig.SetDamageState("wreck", true);
                Assert.IsFalse(rig.IsDamageSmokeActive, "Wrecked hull transitions from damage smoke to wreck fire marker");
                Assert.IsTrue(rig.WreckSmokeVisual.activeSelf);

                // 3. Live Arena VFX: damaged contact smoke, animated ballistic/laser tracers, muzzle flash, and incoming armor hit detection
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                string hitFacet = null;
                int hitDelta = 0;
                bool hitInternal = false;
                arena.OnPlayerArmorDamaged += (facet, delta, isInternal) =>
                {
                    hitFacet = facet;
                    hitDelta = delta;
                    hitInternal = isInternal;
                };

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 92, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 12,
                    ""lock_designator"": ""B1"",
                    ""sections"": [
                        { ""id"": ""hull"", ""condition"": ""intact"", ""internals"": 100, ""internals_max"": 100,
                          ""facets"": [
                            { ""id"": ""front"", ""armor"": 100, ""armor_max"": 100 },
                            { ""id"": ""left"", ""armor"": 100, ""armor_max"": 100 },
                            { ""id"": ""right"", ""armor"": 100, ""armor_max"": 100 },
                            { ""id"": ""rear"", ""armor"": 100, ""armor_max"": 100 }
                          ] }
                    ],
                    ""weapons"": [],
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""unit_type"": ""tacsim.unit.line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [120, 100], ""range_m"": 200, ""bearing_deg"": 90, ""facing"": ""front"", ""health"": ""crippled"", ""reads_dead"": false }
                    ],
                    ""notices"": [
                        { ""kind"": ""fire"", ""weapon"": ""main_cannon"", ""designator"": ""B1"", ""text"": ""R1 fires main_cannon at B1."" }
                    ]
                }"));

                Assert.IsTrue(arena.TryGetContactRig("B1", out var b1Rig));
                Assert.IsTrue(b1Rig.IsDamageSmokeActive, "Live crippled contact rig must show damage smoke");
                Assert.Greater(arena.ActiveShotTracerCount, 0, "Weapon fire notice must spawn animated 3D shot tracer");
                Assert.IsTrue(arena.IsMuzzleFlashActive, "Firing weapon must illuminate player muzzle flash");

                arena.StepTracersForTests(0.12f);
                Assert.Greater(arena.ActiveShotTracerCount, 0, "Ballistic tracer must still be in-flight at 0.12s");
                arena.StepTracersForTests(0.50f);
                Assert.AreEqual(0, arena.ActiveShotTracerCount, "Expired tracers must be destroyed cleanly");
                Assert.IsFalse(arena.IsMuzzleFlashActive, "Muzzle flash must extinguish after brief burst");

                // Incoming armor & internal hit on next frame triggers OnPlayerArmorDamaged + impact burst!
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 92, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 12,
                    ""lock_designator"": ""B1"",
                    ""sections"": [
                        { ""id"": ""hull"", ""condition"": ""scarred"", ""internals"": 88, ""internals_max"": 100,
                          ""facets"": [
                            { ""id"": ""front"", ""armor"": 65, ""armor_max"": 100 },
                            { ""id"": ""left"", ""armor"": 100, ""armor_max"": 100 },
                            { ""id"": ""right"", ""armor"": 100, ""armor_max"": 100 },
                            { ""id"": ""rear"", ""armor"": 100, ""armor_max"": 100 }
                          ] }
                    ],
                    ""weapons"": [],
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""unit_type"": ""tacsim.unit.line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [120, 100], ""range_m"": 200, ""bearing_deg"": 90, ""facing"": ""front"", ""health"": ""crippled"", ""reads_dead"": false }
                    ],
                    ""notices"": []
                }"));

                Assert.AreEqual("FRONT", hitFacet, "Front armor drop must report FRONT facet");
                Assert.AreEqual(35, hitDelta, "Front armor drop 100 -> 65 must report 35% delta");
                Assert.IsTrue(hitInternal, "Internals drop 100 -> 88 must flag internalHit");
                Assert.IsTrue(arena.PlayerRig.IsDamageSmokeActive, "Damaged player hull must emit battle damage smoke");
                Assert.Greater(arena.ActiveShotTracerCount, 0, "Incoming armor hit must spawn impact shockwave burst");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaCompassTapeStandingRulesAndWeaponsDeck_ScrollsBearingsTogglesMindRulesFiresIndividualWeaponsAndHandlesHotkeys()
        {
            // 1. Pure compass tape wrap-around math across North (355 -> 005 = +10 deg, 005 -> 355 = -10 deg)
            float posRight = TacSim.UI.TacSimTacticalBrackets.ComputeCompassTapeOffsetPx(355f, 5f, 420f, 140f, out bool visRight);
            float posLeft = TacSim.UI.TacSimTacticalBrackets.ComputeCompassTapeOffsetPx(5f, 355f, 420f, 140f, out bool visLeft);
            Assert.IsTrue(visRight && visLeft, "+/- 10 deg across North must remain visible inside 140 deg FOV tape");
            Assert.Greater(posRight, 0f, "005 deg must plot right of 355 deg");
            Assert.Less(posLeft, 0f, "355 deg must plot left of 005 deg");
            Assert.AreEqual(posRight, -posLeft, 0.01f);

            TacSim.UI.TacSimTacticalBrackets.ComputeCompassTapeOffsetPx(90f, 270f, 420f, 140f, out bool visBehind);
            Assert.IsFalse(visBehind, "Bearing directly behind (180 deg delta) must report withinTape=false");

            var rootGo = new UnityEngine.GameObject("Wave5PR3_CompassRulesWeaponsTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.GetComponent<UnityEngine.Camera>();

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.TacticalBrackets.BindControllers(null, null, arena);

                var emittedVerbs = new List<string>();
                arena.OnLiveCockpitVerbRequested += emittedVerbs.Add;
                hud.Overlays.OnArenaCockpitVerbRequested += v => arena.ExecuteCockpitAction(v);

                // 2. Hydrate frame with receiver, posture, standing rules, detachment order, weapons, locked contact, objective
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 95, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 18,
                    ""sight_m"": 240, ""scope"": ""hostile"", ""tempo"": ""engaged"", ""posture"": ""guard"",
                    ""weapons_free"": false,
                    ""receiver"": { ""budget"": 40, ""budget_max"": 100, ""reach_percent"": 72 },
                    ""objective"": { ""cell"": [150, 100], ""letter"": ""B"", ""radius_cells"": 4 },
                    ""lock_designator"": ""B1"", ""lock_range_m"": 180, ""lock_percent"": 68,
                    ""rules"": [],
                    ""weapons"": [
                        { ""id"": ""main_gun"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 12, ""rounds_max"": 12, ""percent"": 68 },
                        { ""id"": ""coax_laser"", ""ready"": false, ""ready_in_ms"": 1400, ""rounds"": 99, ""rounds_max"": 99, ""percent"": 54 }
                    ],
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""unit_type"": ""tacsim.unit.line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [118, 100], ""range_m"": 180, ""bearing_deg"": 90, ""facing"": ""front"", ""health"": ""intact"" }
                    ],
                    ""detachment"": {
                        ""id"": ""7"", ""revision"": ""2"", ""term"": ""1"",
                        ""leader_slot"": 1,
                        ""all_order_prefill"": ""ORDER DT:7:2:1 ALL "",
                        ""slots"": [
                            { ""slot"": 1, ""callsign"": ""R1"", ""pilot"": ""Alpha"", ""controller"": ""player"", ""order_prefill"": ""ORDER DT:7:2:1 1 "" }
                        ],
                        ""my_order"": { ""sequence"": ""3"", ""from"": ""Alpha"", ""objective"": ""Hold B"", ""response"": null },
                        ""wilco_command"": ""WILCO DT:7:2 3"",
                        ""negative_command"": ""NEGATIVE DT:7:2 3""
                    }
                }"));

                hud.Overlays.RefreshArenaHud();
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false);

                // Compass tape & receiver/posture telemetry
                Assert.IsTrue(arena.State.HasReceiver);
                Assert.AreEqual(72, arena.State.ReceiverReachPercent);
                Assert.AreEqual("guard", arena.State.Posture);
                Assert.IsTrue(hud.TacticalBrackets.IsArenaCompassVisible);
                Assert.AreEqual("090°", hud.TacticalBrackets.ArenaCompassHeadingSnapshot);
                StringAssert.Contains("SCOPE HOSTILE", hud.TacticalBrackets.ArenaCompassTelemetrySnapshot);
                StringAssert.Contains("SIGHT 240m", hud.TacticalBrackets.ArenaCompassTelemetrySnapshot);
                StringAssert.Contains("GUARD", hud.TacticalBrackets.ArenaCompassTelemetrySnapshot);
                StringAssert.Contains("RCVR 72%", hud.TacticalBrackets.ArenaCompassTelemetrySnapshot);
                Assert.IsTrue(hud.TacticalBrackets.ArenaCompassLockPipVisible, "Locked hostile bearing must plot on compass tape");
                Assert.IsTrue(hud.TacticalBrackets.ArenaCompassObjectivePipVisible, "Dynamic objective bearing must plot on compass tape");

                // 3. Standing Rules UI: Auto-Lock ON, Auto-Fire ON (emits FIRE FREE first because weapons_free=false!), Odds cycle
                Assert.IsFalse(hud.Overlays.IsAutoLockArmedUi);
                Assert.IsFalse(hud.Overlays.IsAutoFireArmedUi);
                Assert.AreEqual(2, hud.Overlays.ActiveWeaponSlotButtonCount, "Two weapon slots must populate pooled buttons");

                hud.Overlays.ClickAutoLockToggleForTests();
                CollectionAssert.Contains(emittedVerbs, "LOCK nearest WHEN nolock");

                emittedVerbs.Clear();
                hud.Overlays.ClickAutoFireToggleForTests();
                CollectionAssert.AreEqual(new[] { "FIRE FREE", "FIRE ALL WHEN odds >= 60" }, emittedVerbs);

                // Server confirms slots #1 (lock) and #2 (fire) -> cycling odds updates rule immediately; toggling disarms via exact MIND RULE <slot> OFF
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 95, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""weapons_free"": true,
                    ""rules"": [
                        { ""slot"": 1, ""action"": ""lock"" },
                        { ""slot"": 2, ""action"": ""fire"" }
                    ],
                    ""weapons"": [
                        { ""id"": ""main_gun"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 12, ""rounds_max"": 12, ""percent"": 68 },
                        { ""id"": ""coax_laser"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 99, ""rounds_max"": 99, ""percent"": 54 }
                    ],
                    ""contacts"": []
                }"));
                hud.Overlays.RefreshArenaHud();
                Assert.IsTrue(hud.Overlays.IsAutoLockArmedUi);
                Assert.IsTrue(hud.Overlays.IsAutoFireArmedUi);

                emittedVerbs.Clear();
                hud.Overlays.CycleAutoFireOddsForTests();
                Assert.AreEqual(80, hud.Overlays.AutoFireOddsThresholdPct);
                CollectionAssert.AreEqual(new[] { "FIRE ALL WHEN odds >= 80" }, emittedVerbs);

                emittedVerbs.Clear();
                hud.Overlays.ClickAutoLockToggleForTests();
                hud.Overlays.ClickAutoFireToggleForTests();
                CollectionAssert.AreEqual(new[] { "MIND RULE 1 OFF", "MIND RULE 2 OFF" }, emittedVerbs);

                // 4. Individual weapon slot fire buttons
                emittedVerbs.Clear();
                hud.Overlays.ClickWeaponSlotButtonForTests(1);
                CollectionAssert.AreEqual(new[] { "fire coax_laser" }, emittedVerbs);

                // 5. Doc 21 Combat Hotkeys (F, G, N, M, Z, L, Y) + KeyboardSuspended gate
                emittedVerbs.Clear();
                Assert.IsTrue(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.F));
                Assert.IsTrue(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.G));
                Assert.IsTrue(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.N));
                Assert.IsTrue(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.M));
                Assert.IsTrue(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.Z));
                CollectionAssert.AreEqual(new[] { "fire hold", "fire all", "scan", "emcon off", "startup" }, emittedVerbs);

                arena.KeyboardSuspended = true;
                emittedVerbs.Clear();
                Assert.IsFalse(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.G), "Hotkeys must be ignored while chat/modal suspends keyboard");
                Assert.AreEqual(0, emittedVerbs.Count);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaTargetInspectionPaperDollScanActionAnd3DBattlefieldMarkers_DecodesConditionRendersContactDollEmitsScanAndPlacesMarkers()
        {
            // 1. Verify BandLetterColor & RenderContactScanToTexture for unscanned, scanned (g/y/r/d), and wrecked contacts
            var tankDoll = TacSim.Arena.TacSimArmorPaperDoll.FindForUnitType("tacsim.unit.line_tank");
            Assert.IsNotNull(tankDoll);
            UnityEngine.Color32[] pix = null;
            UnityEngine.Texture2D tex = null;
            try
            {
                // Unscanned -> steel radar silhouette (distinct from green/yellow/red/destroyed)
                tex = TacSim.Arena.TacSimArmorPaperDoll.RenderContactScanToTexture(tankDoll, null, null, false, tex, ref pix);
                Assert.IsNotNull(tex);
                var unscannedCol0 = TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('?', 0);
                var greenCol0 = TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('g', 0);
                var yellowCol0 = TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('y', 0);
                var redCol0 = TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('r', 0);
                var deadCol0 = TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('d', 0);
                Assert.AreNotEqual(unscannedCol0, greenCol0);
                Assert.AreNotEqual(greenCol0, yellowCol0);
                Assert.AreNotEqual(yellowCol0, redCol0);
                Assert.AreNotEqual(redCol0, deadCol0);

                // Scanned -> hull='y', turret='g', left_track='r', right_track='d', engine='g'
                var secs = new[] { "hull", "turret", "left_track", "right_track", "engine" };
                tex = TacSim.Arena.TacSimArmorPaperDoll.RenderContactScanToTexture(tankDoll, secs, "ygrdg", false, tex, ref pix);
                bool sawYellow = false, sawGreen = false, sawRed = false, sawDead = false;
                for (int i = 0; i < pix.Length; i++)
                {
                    if (pix[i].Equals(yellowCol0) || pix[i].Equals(TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('y', 1))) sawYellow = true;
                    if (pix[i].Equals(greenCol0) || pix[i].Equals(TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('g', 1))) sawGreen = true;
                    if (pix[i].Equals(redCol0) || pix[i].Equals(TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('r', 1))) sawRed = true;
                    if (pix[i].Equals(deadCol0) || pix[i].Equals(TacSim.Arena.TacSimArmorPaperDoll.BandLetterColor('d', 1))) sawDead = true;
                }
                Assert.IsTrue(sawYellow && sawGreen && sawRed && sawDead, "Scanned target paper-doll must color hull/turret/tracks by exact scan band letters");
            }
            finally
            {
                if (tex != null) UnityEngine.Object.DestroyImmediate(tex);
            }

            // 2. Full live arena frame integration: scan_tables, condition, aspect, band, moving, fatal, motor, 3D markers, and TargetInspectCard
            var rootGo = new UnityEngine.GameObject("Wave6PR1_TargetInspectAndMarkersTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);

                var emitted = new List<string>();
                arena.OnLiveCockpitVerbRequested += emitted.Add;
                hud.Overlays.OnArenaCockpitVerbRequested += v => arena.ExecuteCockpitAction(v);

                // Frame without lock -> TargetInspectCard & 3D locked ground ring must both be hidden
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 101, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 10,
                    ""lock_designator"": """",
                    ""contacts"": []
                }"));
                hud.Overlays.RefreshArenaHud();
                Assert.IsFalse(hud.Overlays.IsTargetInspectCardVisible);
                Assert.IsFalse(arena.IsLockedTargetGroundRingVisible);
                Assert.IsFalse(arena.IsDriveWaypointBeaconVisible);

                // Frame with locked moving rear-exposed scanned contact B1 + active motor goal
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 101, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 24,
                    ""lock_designator"": ""B1"", ""lock_range_m"": 160, ""lock_percent"": 82,
                    ""motor"": { ""goal"": ""drive B"", ""outcome"": null },
                    ""scan_tables"": [[""hull"", ""turret"", ""left_track"", ""right_track"", ""engine""]],
                    ""contacts"": [
                        {
                            ""designator"": ""B1"", ""class"": ""line_tank"", ""unit_type"": ""tacsim.unit.line_tank"",
                            ""party"": ""B"", ""friendly"": false, ""cell"": [116, 100], ""range_m"": 160,
                            ""bearing_deg"": 90, ""facing"": ""rear"", ""aspect"": ""rear"", ""band"": ""close"",
                            ""moving"": true, ""age_ms"": 400, ""health"": ""scarred"", ""reads_dead"": false,
                            ""condition"": { ""table"": 0, ""bands"": ""ygrdg"", ""age_ms"": 1200 },
                            ""fatal"": [""hull""],
                            ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 82 }]
                        }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();

                Assert.IsTrue(arena.TryGetLockedContact(out var lockedB1));
                Assert.AreEqual("rear", lockedB1.Aspect);
                Assert.AreEqual("close", lockedB1.Band);
                Assert.IsTrue(lockedB1.Moving);
                Assert.AreEqual(400, lockedB1.AgeMs);
                Assert.AreEqual(0, lockedB1.ConditionTable);
                Assert.AreEqual("ygrdg", lockedB1.ConditionBands);
                CollectionAssert.AreEqual(new[] { "hull" }, lockedB1.FatalSections);
                Assert.AreEqual("drive B", arena.State.MotorGoal);

                Assert.IsTrue(arena.IsLockedTargetGroundRingVisible, "3D locked-target ground ring must highlight active locked contact");
                Assert.IsTrue(hud.Overlays.IsTargetInspectCardVisible, "Target inspection card must appear when a contact is locked");
                Assert.IsNotNull(hud.Overlays.TargetArmorDollTexture, "Locked contact paper-doll texture must be rendered");
                StringAssert.Contains("B1", hud.Overlays.TargetInspectSummarySnapshot);
                StringAssert.Contains("REAR", hud.Overlays.TargetInspectSummarySnapshot);
                StringAssert.Contains("CLOSE", hud.Overlays.TargetInspectSummarySnapshot);
                StringAssert.Contains("MOVING", hud.Overlays.TargetInspectSummarySnapshot);
                StringAssert.Contains("FATAL", hud.Overlays.TargetInspectSummarySnapshot);

                // One-click SCAN TARGET button emits verified `scan B1`, and injected scan target is refused
                emitted.Clear();
                hud.Overlays.ClickScanLockedTargetForTests();
                CollectionAssert.AreEqual(new[] { "scan B1" }, emitted);

                emitted.Clear();
                arena.ExecuteCockpitAction("scan B1 --evil");
                Assert.AreEqual(0, emitted.Count, "Injected scan token must be rejected");

                // 3D Autopilot Drive Waypoint Beacon + Course Line: drive B and drive 132,108 activate beacon; speed 0 clears it
                emitted.Clear();
                arena.ExecuteCockpitAction("drive 132,108");
                Assert.IsTrue(arena.State.HasDriveWaypoint);
                Assert.AreEqual(132, arena.State.DriveWaypointCellX);
                Assert.AreEqual(108, arena.State.DriveWaypointCellY);
                Assert.IsTrue(arena.IsDriveWaypointBeaconVisible, "3D drive waypoint beacon must illuminate when DRIVE <x>,<y> is ordered");
                Assert.IsTrue(arena.IsDriveCourseLineVisible, "3D drive course line must plot route from player to waypoint");
                CollectionAssert.AreEqual(new[] { "drive 132,108" }, emitted);

                arena.ExecuteCockpitAction("speed 0");
                Assert.IsFalse(arena.State.HasDriveWaypoint, "ALL STOP must clear active drive waypoint");
                Assert.IsFalse(arena.IsDriveWaypointBeaconVisible);
                Assert.IsFalse(arena.IsDriveCourseLineVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaTacticalMinimapCoverClickToLockAndDriveAndSealedMindDeck_RendersCoverDispatchesClicksAndSwitchesMindProfilesAndPosture()
        {
            var rootGo = new UnityEngine.GameObject("Wave6PR2_MinimapAndMindDeckTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();

                // Build a deterministic 64x64 grid containing WoodsHeavy, Water, Wall, and Road cells so minimap cover color-coding is verified
                var grid = new DecodedTerrainGrid
                {
                    Id = "minimap_test",
                    DisplayName = "Minimap Test",
                    Biome = "temperate",
                    Width = 64,
                    Height = 64,
                    CellSizeM = 10f,
                    OriginCellX = 0,
                    OriginCellY = 0,
                    SpanCells = 1,
                    Classes = new TerrainCellClass[64 * 64],
                    ElevMeters = new float[64 * 64],
                    KeySpots = new List<ArenaKeySpotDef>
                    {
                        new ArenaKeySpotDef { Letter = "B", Name = "Hilltop", CellX = 32, CellY = 32 }
                    },
                    HoldSpotLetter = "B",
                    HoldRadiusCells = 4
                };
                for (int y = 0; y < 64; y++)
                {
                    for (int x = 0; x < 64; x++)
                    {
                        int idx = y * 64 + x;
                        grid.ElevMeters[idx] = 2f;
                        if (x < 16 && y < 16) grid.Classes[idx] = TerrainCellClass.WoodsHeavy;
                        else if (x >= 48 && y < 16) grid.Classes[idx] = TerrainCellClass.Water;
                        else grid.Classes[idx] = TerrainCellClass.Ground;
                    }
                }
                arena.BuildArenaTerrain(catalog, grid);

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.Minimap.BindWorldSources(null, null, null, null, arena);

                var emittedVerbs = new List<string>();
                var combatLogs = new List<string>();
                arena.OnLiveCockpitVerbRequested += emittedVerbs.Add;
                arena.OnArenaCombatLog += combatLogs.Add;
                hud.Overlays.OnArenaCockpitVerbRequested += v => arena.ExecuteCockpitAction(v);

                string clickedContact = null;
                int clickedCellX = -1, clickedCellY = -1;
                hud.Minimap.OnArenaMinimapContactClicked += d =>
                {
                    clickedContact = d;
                    arena.ExecuteCockpitAction($"lock {d}");
                };
                hud.Minimap.OnArenaMinimapCellClicked += (cx, cy) =>
                {
                    clickedCellX = cx;
                    clickedCellY = cy;
                    arena.ExecuteCockpitAction($"drive {cx},{cy}");
                };

                // 1. Live frame with sealed Mind (Standard Doctrine, 2 profiles), posture=guard, objective, contact B1, and reclaim catchup
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 104, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 20000, ""y_cm"": 20000, ""heading_deg"": 45, ""speed_kph"": 18,
                    ""sight_m"": 160, ""scope"": ""hostile"", ""posture"": ""guard"",
                    ""lock_designator"": ""B1"",
                    ""objective"": { ""cell"": [32, 32], ""letter"": ""B"", ""radius_cells"": 4 },
                    ""mind"": {
                        ""name"": ""Standard Doctrine"",
                        ""profiles"": [""Sniper"", ""Line""],
                        ""active"": 0,
                        ""class"": ""I"",
                        ""core"": ""II"",
                        ""channels"": [""helm"", ""sensors"", ""fire""],
                        ""next_switch_ms"": null,
                        ""off"": false
                    },
                    ""catchup"": {
                        ""away_s"": 42,
                        ""cell_then"": [18, 18],
                        ""cell_now"": [20, 20],
                        ""contacts_new"": 2,
                        ""contacts_lost"": 1,
                        ""sections"": [{ ""id"": ""hull"", ""then"": ""intact"", ""now"": ""scarred"" }]
                    },
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""unit_type"": ""tacsim.unit.line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [48, 48], ""range_m"": 380, ""bearing_deg"": 45, ""facing"": ""front"", ""health"": ""intact"" }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();

                // Catchup log emitted once on reclaim
                Assert.IsTrue(arena.State.HasReceivedCatchup);
                Assert.AreEqual(42L, arena.State.LastCatchupAwaySec);
                Assert.IsTrue(combatLogs.Exists(l => l.Contains("[CATCH-UP]") && l.Contains("42s")),
                    "Reclaim catchup summary must be emitted to arena combat log");

                // 2. Sealed Mind Profile & Hull Posture Deck
                Assert.IsNotNull(arena.State.Mind);
                Assert.AreEqual("Standard Doctrine", arena.State.Mind.Name);
                Assert.AreEqual("Sniper", arena.State.Mind.ActiveProfileName);
                Assert.IsTrue(hud.Overlays.IsMindStripVisible, "Sealed mind strip must appear when hull carries a sealed mind");
                Assert.AreEqual(2, hud.Overlays.VisibleMindProfileButtonCount);
                StringAssert.Contains("STANDARD DOCTRINE", hud.Overlays.MindStripHeaderSnapshot);

                // Clicking active profile (0 = Sniper) must NOT emit a redundant verb; clicking inactive profile (1 = Line) emits `MIND PROFILE Line`
                emittedVerbs.Clear();
                hud.Overlays.ClickMindProfileForTests(0);
                Assert.AreEqual(0, emittedVerbs.Count, "Active mind profile must not re-send MIND PROFILE");
                hud.Overlays.ClickMindProfileForTests(1);
                CollectionAssert.AreEqual(new[] { "MIND PROFILE Line" }, emittedVerbs);

                // Posture switching: already `guard` is a no-op; clicking `hold` emits `MIND POSTURE hold`
                emittedVerbs.Clear();
                hud.Overlays.ClickMindPostureForTests("guard");
                Assert.AreEqual(0, emittedVerbs.Count, "Already-active posture must not re-send MIND POSTURE");
                hud.Overlays.ClickMindPostureForTests("hold");
                CollectionAssert.AreEqual(new[] { "MIND POSTURE hold" }, emittedVerbs);

                // 3. Upgraded Arena Tactical Minimap: cover rendering, sensor ring, objective, waypoint, click-to-lock, click-to-drive
                arena.ExecuteCockpitAction("drive 40,24");
                emittedVerbs.Clear();
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 45f, rightCornerOccupied: true, forceRedraw: true);

                Assert.AreEqual(1, hud.Minimap.LastArenaMinimapContactBlipCount);
                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasSensorRing, "Arena minimap must draw SightMeters sensor ring");
                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasObjective, "Arena minimap must draw dynamic hold objective");
                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasWaypoint, "Arena minimap must draw active drive waypoint");

                // Verify WoodsHeavy (bottom-left pixel) renders greener than Water (bottom-right pixel)
                var tex = hud.Minimap.MinimapTexture;
                Assert.IsNotNull(tex);
                var woodsPx = hud.Minimap.SampleMinimapPixelForTests(16, 16);
                var waterPx = hud.Minimap.SampleMinimapPixelForTests(tex.width - 16, 16);
                Assert.Greater(woodsPx.g, woodsPx.b, "Woods terrain on arena minimap must render forest green");
                Assert.Greater(waterPx.b, waterPx.g, "Water terrain on arena minimap must render water blue");

                // Clicking contact B1 at cell (48, 48) -> UV ~(48.5/64, 48.5/64) triggers click-to-lock!
                Assert.IsTrue(hud.Minimap.HandleClickAtUv(new UnityEngine.Vector2(48.5f / 64f, 48.5f / 64f)));
                Assert.AreEqual("B1", clickedContact);
                CollectionAssert.AreEqual(new[] { "lock B1" }, emittedVerbs);

                // Clicking open terrain at UV (0.25, 0.75) -> cell ~(16, 48) triggers click-to-drive!
                emittedVerbs.Clear();
                Assert.IsTrue(hud.Minimap.HandleClickAtUv(new UnityEngine.Vector2(0.25f, 0.75f)));
                Assert.AreEqual(16, clickedCellX);
                Assert.AreEqual(48, clickedCellY);
                CollectionAssert.AreEqual(new[] { "drive 16,48" }, emittedVerbs);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaHitmarkersDirectionalDamageArcsAndCombatNoticeFeed_RoutesNoticesPulsesArcsAndUpdatesKillFeed()
        {
            var rootGo = new UnityEngine.GameObject("Wave7PR1_CombatFeedbackTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.GetComponent<UnityEngine.Camera>();

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.TacticalBrackets.BindControllers(null, null, arena);

                var noticesSeen = new List<TacSim.Arena.ArenaCombatNotice>();
                arena.OnArenaNoticeReceived += n =>
                {
                    noticesSeen.Add(n);
                    if (n.Kind == "kill") hud.TacticalBrackets.TriggerArenaHitmarker(TacSim.UI.ArenaHitmarkerKind.Kill);
                    else if (n.Kind == "target_section_lost") hud.TacticalBrackets.TriggerArenaHitmarker(TacSim.UI.ArenaHitmarkerKind.SectionCrit);
                    else if (n.Kind == "hit") hud.TacticalBrackets.TriggerArenaHitmarker(TacSim.UI.ArenaHitmarkerKind.Hit);
                    else if (n.Kind == "miss") hud.TacticalBrackets.TriggerArenaHitmarker(TacSim.UI.ArenaHitmarkerKind.Miss);
                };

                // 1. Activate a drive waypoint first, then push a rich frame containing hit, target_section_lost, kill, miss, slot_fired, and drive_arrived notices
                arena.ExecuteCockpitAction("drive 120,120");
                Assert.IsTrue(arena.State.HasDriveWaypoint);

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 110, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 14,
                    ""heat"": { ""heat"": 980, ""max"": 1000, ""overheated"": true },
                    ""lock_designator"": ""B1"",
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""unit_type"": ""tacsim.unit.line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [115, 100], ""range_m"": 150, ""bearing_deg"": 90, ""facing"": ""front"", ""health"": ""crippled"" }
                    ],
                    ""notices"": [
                        { ""kind"": ""slot_fired"", ""slot"": 2, ""weapon"": ""main_gun"", ""designator"": ""B1"", ""percent"": 84, ""aspect"": ""rear"", ""text"": ""Rule #2 fired main_gun at B1."" },
                        { ""kind"": ""hit"", ""weapon"": ""main_gun"", ""designator"": ""B1"", ""section"": ""turret"", ""text"": ""Hit B1 turret."" },
                        { ""kind"": ""target_section_lost"", ""designator"": ""B1"", ""section"": ""turret"", ""text"": ""B1 turret destroyed!"" },
                        { ""kind"": ""kill"", ""designator"": ""B1"", ""text"": ""B1 destroyed!"" },
                        { ""kind"": ""miss"", ""weapon"": ""coax_laser"", ""designator"": ""B1"", ""text"": ""coax_laser missed B1."" },
                        { ""kind"": ""drive_arrived"", ""text"": ""Arrived at destination."" }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false);

                // Structured notice parsing & match gunnery counters
                Assert.AreEqual(6, noticesSeen.Count);
                Assert.AreEqual(1, arena.State.MatchHits);
                Assert.AreEqual(1, arena.State.MatchMisses);
                Assert.AreEqual(1, arena.State.MatchKills);
                Assert.IsFalse(arena.State.HasDriveWaypoint, "drive_arrived notice must clear active drive waypoint");

                // Hitmarker triggered and decays cleanly
                Assert.IsTrue(hud.TacticalBrackets.IsHitmarkerVisible);
                hud.TacticalBrackets.TriggerArenaHitmarker(TacSim.UI.ArenaHitmarkerKind.Kill);
                Assert.AreEqual(TacSim.UI.ArenaHitmarkerKind.Kill, hud.TacticalBrackets.ActiveHitmarkerKind);
                hud.TacticalBrackets.StepCombatFeedbackForTests(0.80f);
                Assert.IsFalse(hud.TacticalBrackets.IsHitmarkerVisible, "Hitmarker must fade out after duration");

                // 4-Quadrant Directional Damage Arcs (LEFT hit vs full-perimeter internal breach) + decay
                hud.TacticalBrackets.TriggerDirectionalDamageArc("LEFT", internalBreach: false);
                Assert.Greater(hud.TacticalBrackets.GetDamageArcAlpha("LEFT"), 0.8f);
                Assert.AreEqual(0f, hud.TacticalBrackets.GetDamageArcAlpha("RIGHT"), 0.001f);

                hud.TacticalBrackets.TriggerDirectionalDamageArc("INTERNALS", internalBreach: true);
                Assert.Greater(hud.TacticalBrackets.GetDamageArcAlpha("FRONT"), 0.7f);
                Assert.Greater(hud.TacticalBrackets.GetDamageArcAlpha("REAR"), 0.7f);
                Assert.Greater(hud.TacticalBrackets.GetDamageArcAlpha("RIGHT"), 0.7f);
                hud.TacticalBrackets.StepCombatFeedbackForTests(1.2f);
                Assert.AreEqual(0f, hud.TacticalBrackets.GetDamageArcAlpha("FRONT"), 0.001f);

                // Thermal overheat banner active because frame set overheated=true
                Assert.IsTrue(hud.TacticalBrackets.IsCriticalStatusBannerVisible);
                StringAssert.Contains("OVERHEAT", hud.TacticalBrackets.CriticalStatusBannerText);

                // Live Combat & Kill Feed ticker shows recent structured badges
                Assert.IsTrue(hud.Overlays.IsCombatFeedVisible);
                Assert.GreaterOrEqual(hud.Overlays.VisibleCombatFeedEntryCount, 4);
                StringAssert.Contains("KILL", hud.Overlays.CombatFeedSnapshot);
                StringAssert.Contains("B1", hud.Overlays.CombatFeedSnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaGunneryBallisticsObjectiveBracketAndDebriefCard_DecodesRangeBandsProjectsObjectiveAndShowsWreckDebrief()
        {
            var rootGo = new UnityEngine.GameObject("Wave7PR2_GunneryAndDebriefTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.GetComponent<UnityEngine.Camera>();
                cam.transform.position = TacSim.Arena.OocArenaController.ArenaStageOrigin + new UnityEngine.Vector3(100f, 35f, 80f);
                cam.transform.LookAt(TacSim.Arena.OocArenaController.ArenaStageOrigin + new UnityEngine.Vector3(150f, 0f, 100f));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.TacticalBrackets.BindControllers(null, null, arena);

                var emitted = new List<string>();
                arena.OnLiveCockpitVerbRequested += emitted.Add;
                hud.Overlays.OnArenaCockpitVerbRequested += v => arena.ExecuteCockpitAction(v);

                // 1. Live frame with weapon range_band, bands_cm, odds_floor_permille, blocked=true, dynamic objective, and rear-exposed contact
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 115, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""blocked"": true, ""odds_floor_permille"": 150, ""wreck"": false,
                    ""objective"": { ""cell"": [150, 100], ""letter"": ""B"", ""radius_cells"": 4 },
                    ""lock_designator"": ""B1"", ""lock_range_m"": 160, ""lock_percent"": 82,
                    ""weapons"": [
                        {
                            ""id"": ""main_gun"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 10, ""rounds_max"": 12, ""percent"": 82,
                            ""range_band"": { ""index"": 1, ""of"": 3, ""max_range_cm"": 45000, ""dispersion_mrad"": 12 },
                            ""bands_cm"": [15000, 30000, 45000]
                        }
                    ],
                    ""contacts"": [
                        {
                            ""designator"": ""B1"", ""class"": ""line_tank"", ""unit_type"": ""tacsim.unit.line_tank"",
                            ""party"": ""B"", ""friendly"": false, ""cell"": [116, 100], ""range_m"": 160,
                            ""bearing_deg"": 90, ""facing"": ""rear"", ""aspect"": ""rear"", ""band"": ""close"", ""moving"": true,
                            ""health"": ""scarred"", ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 82 }]
                        }
                    ],
                    ""notices"": [
                        { ""kind"": ""hit"", ""weapon"": ""main_gun"", ""designator"": ""B1"", ""section"": ""hull"", ""text"": ""Hit B1."" },
                        { ""kind"": ""kill"", ""designator"": ""B1"", ""text"": ""B1 destroyed!"" }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false);

                // Weapon range_band & blocked decoding
                Assert.IsTrue(arena.State.MovementBlocked);
                Assert.AreEqual(15, arena.State.OddsFloorPercent);
                Assert.IsTrue(arena.TryGetPrimaryWeaponGunnery(out var primary));
                Assert.AreEqual(1, primary.RangeBandIndex);
                Assert.AreEqual(3, primary.RangeBandTotal);
                Assert.AreEqual(450f, primary.MaxRangeM, 0.5f);
                Assert.AreEqual(12, primary.DispersionMrad);

                // Gunnery strip & 3D Objective + Aspect-Enriched Contact Brackets
                StringAssert.Contains("BAND 1/3", hud.TacticalBrackets.ArenaGunnerySnapshot);
                StringAssert.Contains("450m", hud.TacticalBrackets.ArenaGunnerySnapshot);
                StringAssert.Contains("12mr", hud.TacticalBrackets.ArenaGunnerySnapshot);
                StringAssert.Contains("BLOCKED", hud.TacticalBrackets.ArenaGunnerySnapshot);
                Assert.IsTrue(hud.TacticalBrackets.ArenaObjectiveBracketActive, "Active hold objective must project as a 3D viewport bracket");

                bool sawObjectiveTag = false;
                bool sawAspectTag = false;
                for (int b = 0; b < hud.TacticalBrackets.ActiveBracketCount; b++)
                {
                    string tag = hud.TacticalBrackets.GetActiveBracketTagSnapshot(b);
                    if (tag.Contains("HOLD ZONE") || tag.Contains("[B]")) sawObjectiveTag = true;
                    if (tag.Contains("B1") && tag.Contains("REAR")) sawAspectTag = true;
                }
                Assert.IsTrue(sawObjectiveTag, "Projected objective bracket must display [B] HOLD ZONE");
                Assert.IsTrue(sawAspectTag, "Locked contact bracket must display exposed REAR aspect");

                // Debrief card hidden while alive
                Assert.IsFalse(hud.Overlays.IsDebriefCardVisible);

                // 2. Wrecked transition reveals Wrecked / Match Debrief Card with live gunnery accuracy, kills, EXIT & DISMISS
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 115, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""off"",
                    ""wreck"": true,
                    ""weapons"": [],
                    ""contacts"": []
                }"));
                hud.Overlays.RefreshArenaHud();

                Assert.IsTrue(hud.Overlays.IsDebriefCardVisible, "Wrecked hull must open Match Debrief card");
                StringAssert.Contains("MATCH #115", hud.Overlays.DebriefSummarySnapshot);
                StringAssert.Contains("KILLS: <b>1</b>", hud.Overlays.DebriefSummarySnapshot);
                StringAssert.Contains("100%", hud.Overlays.DebriefSummarySnapshot);

                emitted.Clear();
                hud.Overlays.ClickDebriefExitForTests();
                CollectionAssert.AreEqual(new[] { "exit" }, emitted);

                hud.Overlays.ClickDebriefDismissForTests();
                Assert.IsFalse(hud.Overlays.IsDebriefCardVisible, "Dismissing debrief card must hide it for the current match");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaReplayTheaterMarkersSpectatorMinimapAndOverviewScanSort_JumpsEventsFocusesBlipsAndAvoidsMinimapOverlap()
        {
            var rootGo = new UnityEngine.GameObject("Wave8PR1_ReplayAndOverviewTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.ReplayHud.BindArena(arena);
                hud.Minimap.BindWorldSources(null, null, null, null, arena);
                hud.TacticalOverview.BindControllers(null, null, arena);

                // 1. Arena Tactical Overview: non-overlap shifting, ODDS sorting, and secondary SCAN action
                var scanned = new List<string>();
                var locked = new List<string>();
                hud.TacticalOverview.OnArenaContactLockClicked += locked.Add;
                hud.TacticalOverview.OnArenaContactScanClicked += scanned.Add;

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 120, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 10,
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [110, 100], ""range_m"": 100, ""bearing_deg"": 90, ""facing"": ""front"", ""aspect"": ""front"", ""band"": ""close"", ""health"": ""intact"", ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 38 }] },
                        { ""designator"": ""B2"", ""class"": ""biped_mk1"", ""party"": ""B"", ""friendly"": false, ""cell"": [128, 100], ""range_m"": 280, ""bearing_deg"": 90, ""facing"": ""rear"", ""aspect"": ""rear"", ""band"": ""medium"", ""moving"": true, ""health"": ""scarred"", ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 88 }] }
                    ]
                }"));

                hud.TacticalOverview.SetAvoidMidLeftMinimap(true);
                Assert.IsTrue(hud.TacticalOverview.IsAvoidingMidLeftMinimap);
                hud.TacticalOverview.SyncOverview(TacSimActiveStage.SimulatorArena, hideForOverlay: false, forceImmediate: true);
                Assert.AreEqual("B1", hud.TacticalOverview.FirstRowId, "Default DIST sort puts nearer B1 (100m) first");

                // Toggle to ODDS sort -> B2 (88%) rises above B1 (38%)!
                hud.TacticalOverview.ToggleSortMode();
                Assert.IsTrue(hud.TacticalOverview.SortArenaByOdds);
                Assert.AreEqual("B2", hud.TacticalOverview.FirstRowId, "ODDS sort must rank higher-odds B2 (88%) first");

                // Primary row click emits LOCK; secondary row click emits SCAN!
                Assert.IsTrue(hud.TacticalOverview.ClickRowByIndex(0));
                Assert.IsTrue(hud.TacticalOverview.ClickRowSecondaryByIndex(0));
                CollectionAssert.AreEqual(new[] { "B2" }, locked);
                CollectionAssert.AreEqual(new[] { "B2" }, scanned);

                // 2. Exit live cockpit -> load 3D Replay Document with hits & wrecks
                arena.ExitArena();
                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 77, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 20000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [
                        { ""callsign"": ""R1"", ""team"": ""red"" },
                        { ""callsign"": ""B1"", ""team"": ""blue"" }
                    ],
                    ""keyframes"": [
                        [0, [[16, 16, 90], [48, 48, 270]]],
                        [10000, [[24, 24, 90], [44, 44, 270]]],
                        [20000, [[32, 32, 90], [44, 44, 270]]]
                    ],
                    ""shots"": [
                        [5000, 0, 1, ""miss""],
                        [9000, 0, 1, ""hit""],
                        [14000, 0, 1, ""hit""]
                    ],
                    ""destroyed"": [
                        [15000, 1]
                    ]
                }"));
                hud.ReplayHud.RefreshView();

                Assert.IsTrue(arena.IsReplayModeActive);
                Assert.IsTrue(hud.ReplayHud.IsVisible);
                Assert.GreaterOrEqual(hud.ReplayHud.VisibleTimelineMarkerCount, 3, "Scrubber track must plot markers for hit shots and wrecks");
                Assert.IsTrue(arena.IsLockedTargetGroundRingVisible, "3D ground focus ring must highlight the followed spectator unit");

                // StepReplayToEvent jumps cleanly from 0ms -> 9000ms (first hit) -> 14000ms (second hit) -> 15000ms (wreck)!
                arena.SeekReplayMs(0f);
                hud.ReplayHud.ClickNextEventForTests();
                Assert.AreEqual(9000f, arena.ReplayCursorMs, 1f);
                hud.ReplayHud.ClickNextEventForTests();
                Assert.AreEqual(14000f, arena.ReplayCursorMs, 1f);
                hud.ReplayHud.ClickNextEventForTests();
                Assert.AreEqual(15000f, arena.ReplayCursorMs, 1f);
                StringAssert.Contains("3 SHOTS", hud.ReplayHud.ReplayStatsSnapshot);
                StringAssert.Contains("1 WRECK", hud.ReplayHud.ReplayStatsSnapshot);

                // 3. Spectator Minimap renders both replay units and supports click-to-focus-camera!
                int clickedUnitIdx = -1;
                hud.Minimap.OnReplayMinimapUnitClicked += idx =>
                {
                    clickedUnitIdx = idx;
                    arena.SelectReplayFocusUnit(idx);
                };
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 0f, rightCornerOccupied: true, forceRedraw: true);
                Assert.AreEqual(2, hud.Minimap.LastReplayMinimapUnitCount, "Spectator minimap must plot all visible replay roster units");

                // Click B1's position on the terrain grid (44.5/W, 44.5/H) -> focuses unit index 1!
                float gridW = arena.TerrainGrid.Width * System.Math.Max(1, arena.TerrainGrid.SpanCells);
                float gridH = arena.TerrainGrid.Height * System.Math.Max(1, arena.TerrainGrid.SpanCells);
                Assert.IsTrue(hud.Minimap.HandleClickAtUv(new UnityEngine.Vector2(
                    (44.5f - arena.TerrainGrid.OriginCellX) / gridW,
                    (44.5f - arena.TerrainGrid.OriginCellY) / gridH)));
                Assert.AreEqual(1, clickedUnitIdx);
                Assert.AreEqual(1, arena.ReplayFocusUnitIndex);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DVehicleRecoilHitFlashBeaconAndMindRulesDrawer_AnimatesRecoilFlashesHitsAndRemovesStandingRules()
        {
            var rootGo = new UnityEngine.GameObject("Wave8PR2_RecoilFlashRulesDrawerTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();

                // 1. Procedural 3D Rig: turret recoil kickback spring, impact hit-flash burst, and operational status beacon
                var rig = TacSimVehicleVisualFactory.SpawnArenaCombatUnit(
                    null, rootGo.transform, "tacsim.unit.line_tank", "R1", "R", UnityEngine.Vector3.zero, 90f);
                Assert.IsNotNull(rig.HitFlashVisual, "Arena rig must include a 3D hit-flash visual");
                Assert.IsNotNull(rig.StatusBeaconRenderer, "Arena rig must include an operational hull status beacon LED");
                Assert.IsFalse(rig.IsHitFlashActive);

                rig.PlayWeaponRecoil();
                Assert.Greater(rig.CurrentRecoilKickOffset, 0.4f, "PlayWeaponRecoil must apply local-Z recoil kickback even on procedural rigs");
                rig.UpdateArticulation(0.40f, 0f, null);
                Assert.AreEqual(0f, rig.CurrentRecoilKickOffset, 0.01f, "Recoil spring must recover smoothly to rest");

                rig.TriggerHitFlash(isCrit: true);
                Assert.IsTrue(rig.IsHitFlashActive, "TriggerHitFlash must illuminate the impact burst");
                rig.UpdateArticulation(0.35f, 0f, null);
                Assert.IsFalse(rig.IsHitFlashActive, "Hit-flash burst must extinguish cleanly after duration");

                // 2. Full live arena integration: hit notice flashes struck contact, mind sealed_rules & order decode, Rules drawer deletes rule slots
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);

                var emitted = new List<string>();
                arena.OnLiveCockpitVerbRequested += emitted.Add;
                hud.Overlays.OnArenaCockpitVerbRequested += v => arena.ExecuteCockpitAction(v);

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 125, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 15,
                    ""heat"": { ""heat"": 990, ""max"": 1000, ""overheated"": true },
                    ""rules"": [
                        { ""slot"": 1, ""action"": ""lock"" },
                        { ""slot"": 3, ""action"": ""fire"", ""weapon"": ""main_gun"", ""percent"": 60 }
                    ],
                    ""order"": {
                        ""squad"": ""red"", ""objective"": ""advance"", ""to"": [140, 110],
                        ""hold_fire"": true, ""expires_in_ms"": 45000
                    },
                    ""mind"": {
                        ""name"": ""Standard Doctrine"",
                        ""card"": { ""schema"": ""doctrine.v1"", ""values"": {} },
                        ""sealed_rules"": [""LOCK nearest WHEN nolock"", ""FIRE ALL WHEN odds >= 60""],
                        ""profiles"": [""Sniper"", ""Line""],
                        ""active"": 0, ""class"": ""I"", ""core"": ""II"",
                        ""channels"": [""helm"", ""sensors"", ""fire""], ""off"": false
                    },
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [115, 100], ""range_m"": 150, ""bearing_deg"": 90, ""facing"": ""front"", ""health"": ""scarred"" }
                    ],
                    ""notices"": [
                        { ""kind"": ""hit"", ""weapon"": ""main_gun"", ""designator"": ""B1"", ""section"": ""turret"", ""text"": ""Hit B1 turret."" }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();

                // Contact B1 flashed on hit notice!
                Assert.IsTrue(arena.TryGetContactRig("B1", out var b1Rig));
                Assert.IsTrue(b1Rig.IsHitFlashActive, "Hit notice on B1 must flash B1's 3D hit-impact shell");

                // Decoded sealed_rules, card schema, and standing order
                Assert.IsNotNull(arena.State.Mind);
                Assert.AreEqual("doctrine.v1", arena.State.Mind.CardSchema);
                Assert.AreEqual(2, arena.State.Mind.SealedRules.Count);
                Assert.IsNotNull(arena.State.StandingOrder);
                Assert.AreEqual("advance", arena.State.StandingOrder.Objective);
                Assert.IsTrue(arena.State.StandingOrder.HoldFire);
                Assert.AreEqual(140, arena.State.StandingOrder.ToCellX);

                // Toggle Rules & Mind Inspector drawer open -> shows 2 standing rule chips, sealed rules, and active order!
                Assert.IsFalse(hud.Overlays.IsRulesDrawerOpen);
                hud.Overlays.ToggleRulesDrawerForTests();
                Assert.IsTrue(hud.Overlays.IsRulesDrawerOpen);
                Assert.AreEqual(2, hud.Overlays.VisibleStandingRulePillCount, "Rules drawer must render one clickable removal pill per active standing rule");
                StringAssert.Contains("LOCK nearest WHEN nolock", hud.Overlays.RulesDrawerSnapshot);
                StringAssert.Contains("ADVANCE", hud.Overlays.RulesDrawerSnapshot);
                StringAssert.Contains("HOLD FIRE", hud.Overlays.RulesDrawerSnapshot);

                // Clicking standing rule pill #1 (slot 3) emits exact verified `MIND RULE 3 OFF`!
                emitted.Clear();
                hud.Overlays.ClickRemoveStandingRulePillForTests(1);
                CollectionAssert.AreEqual(new[] { "MIND RULE 3 OFF" }, emitted);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaTerrainCoverElevationLosAndBiomeAtmosphere_SamplesCoverDetectsRidgeLosAndUpdatesCompassTelemetry()
        {
            // 1. Biome lighting profiles differentiate desert, arctic, regolith (vacuum/no fog), and temperate
            var desert = TacSim.Arena.TacSimArenaEnvironmentBuilder.ResolveBiomeLighting("desert");
            var arctic = TacSim.Arena.TacSimArenaEnvironmentBuilder.ResolveBiomeLighting("arctic");
            var regolith = TacSim.Arena.TacSimArenaEnvironmentBuilder.ResolveBiomeLighting("ceres_regolith");
            Assert.AreEqual("desert", desert.BiomeKey);
            Assert.Greater(desert.SunColor.r, desert.SunColor.b, "Desert sunlight must be warm golden");
            Assert.AreEqual("arctic", arctic.BiomeKey);
            Assert.Greater(arctic.SunColor.b, arctic.SunColor.r, "Arctic sunlight must be crisp cool blue-white");
            Assert.AreEqual("regolith", regolith.BiomeKey);
            Assert.IsFalse(regolith.EnableFog, "Lunar/asteroid vacuum regolith must disable atmospheric fog");

            // 2. Deterministic 32x32 terrain with Heavy Woods hilltop at (6,6, elev=14m), valley target at (24,6, elev=2m), and ridge option at (15,6)
            var grid = new DecodedTerrainGrid
            {
                Id = "intel_test",
                DisplayName = "Intel Ridge",
                Biome = "arctic",
                Width = 32,
                Height = 32,
                CellSizeM = 10f,
                OriginCellX = 0,
                OriginCellY = 0,
                SpanCells = 1,
                Classes = new TerrainCellClass[32 * 32],
                ElevMeters = new float[32 * 32]
            };
            for (int y = 0; y < 32; y++)
            {
                for (int x = 0; x < 32; x++)
                {
                    int idx = y * 32 + x;
                    grid.Classes[idx] = (x >= 4 && x <= 8 && y >= 4 && y <= 8) ? TerrainCellClass.WoodsHeavy : TerrainCellClass.Ground;
                    grid.ElevMeters[idx] = (x <= 8) ? 14f : 2f;
                }
            }

            // Clear LOS from hilltop heavy woods (65m, 65m) down to valley target (245m, 65m)
            var clearSample = TacSim.Arena.TacSimArenaEnvironmentBuilder.EvaluateUnitTerrainContext(
                grid, 65f, 65f, 2.5f, hasTarget: true, targetLocalX: 245f, targetLocalZ: 65f);
            Assert.AreEqual("HEAVY WOODS", clearSample.CoverLabel);
            Assert.IsTrue(clearSample.HasCover);
            Assert.AreEqual("HIGH GROUND", clearSample.ElevationAdvantage);
            Assert.Greater(clearSample.ElevationDeltaM, 10f);
            Assert.IsFalse(clearSample.IsLineOfSightObstructed);
            Assert.AreEqual("CLEAR", clearSample.LineOfSightStatus);

            // Raise an intervening hill ridge at x=14..16 (elev=28m) -> LOS probe must detect RIDGE obstruction!
            for (int y = 0; y < 32; y++)
                for (int x = 14; x <= 16; x++)
                    grid.ElevMeters[y * 32 + x] = 28f;

            var ridgeSample = TacSim.Arena.TacSimArenaEnvironmentBuilder.EvaluateUnitTerrainContext(
                grid, 65f, 65f, 2.5f, hasTarget: true, targetLocalX: 245f, targetLocalZ: 65f);
            Assert.IsTrue(ridgeSample.IsLineOfSightObstructed, "Tall intervening hill must flag terrain LOS obstruction");
            Assert.AreEqual("RIDGE", ridgeSample.LineOfSightStatus);

            // 3. Full live Arena + Compass Telemetry integration
            var rootGo = new UnityEngine.GameObject("Wave9PR1_TerrainIntelTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, grid);
                Assert.AreEqual("arctic", arena.ActiveBiomeLighting.BiomeKey);

                var camGo = new UnityEngine.GameObject("Cam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.GetComponent<UnityEngine.Camera>();

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.TacticalBrackets.BindControllers(null, null, arena);

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 130, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 6500, ""y_cm"": 6500, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""sensor_height_cm"": 320, ""sight_m"": 260, ""scope"": ""hostile"",
                    ""lock_designator"": ""B1"", ""lock_range_m"": 180, ""lock_percent"": 55,
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [24, 6], ""range_m"": 180, ""bearing_deg"": 90, ""facing"": ""front"", ""health"": ""intact"" }
                    ]
                }"));
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false);

                Assert.AreEqual(3.2f, arena.State.SensorHeightM, 0.05f, "sensor_height_cm must decode to meters");
                Assert.AreEqual("HEAVY WOODS", arena.CurrentTerrainIntel.CoverLabel);
                Assert.AreEqual("HIGH GROUND", arena.CurrentTerrainIntel.ElevationAdvantage);
                Assert.IsTrue(arena.CurrentTerrainIntel.IsLineOfSightObstructed);
                Assert.AreEqual("RIDGE", arena.CurrentTerrainIntel.LineOfSightStatus);
                StringAssert.Contains("HEAVY WOODS", hud.TacticalBrackets.ArenaTerrainBadgeSnapshot);
                StringAssert.Contains("HIGH +", hud.TacticalBrackets.ArenaTerrainBadgeSnapshot);
                StringAssert.Contains("RIDGE", hud.TacticalBrackets.ArenaTerrainBadgeSnapshot);
                StringAssert.Contains("HEAVY WOODS", hud.TacticalBrackets.ArenaCompassTelemetrySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaCommandPaletteLiveTacticalActionsAnd3ButtonTargetFrame_PopulatesLockScanDriveMindAndFiresVerbs()
        {
            var rootGo = new UnityEngine.GameObject("Wave9PR2_ArenaPaletteAndUnitFramesTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.CommandPalette.BindWorldSources(null, null, arena);

                var emitted = new List<string>();
                arena.OnLiveCockpitVerbRequested += emitted.Add;
                hud.CommandPalette.OnArenaActionRequested += v => arena.ExecuteCockpitAction(v);
                hud.CommandPalette.OnCommandRequested += v => arena.ExecuteCockpitAction(v);

                // 1. Live Arena frame with hostile B1, Mind profiles (Sniper active, Line inactive), and KeySpots
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 135, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 14,
                    ""lock_designator"": ""B1"", ""lock_range_m"": 160, ""lock_percent"": 84,
                    ""mind"": {
                        ""name"": ""Standard Doctrine"", ""profiles"": [""Sniper"", ""Line""],
                        ""active"": 0, ""class"": ""I"", ""core"": ""II"", ""channels"": [""helm"", ""sensors"", ""fire""], ""off"": false
                    },
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [116, 100], ""range_m"": 160, ""bearing_deg"": 90, ""facing"": ""rear"", ""aspect"": ""rear"", ""band"": ""close"", ""health"": ""scarred"", ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 84 }] }
                    ]
                }"));

                hud.CommandPalette.SetActiveStage(TacSimActiveStage.SimulatorArena);
                hud.CommandPalette.SetOpen(true);

                // Fuzzy search "scan b1" -> executes verified `scan B1`!
                emitted.Clear();
                hud.CommandPalette.SetQuery("scan b1");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                CollectionAssert.AreEqual(new[] { "scan B1" }, emitted);

                // Fuzzy search "profile line" -> executes verified `MIND PROFILE Line`!
                emitted.Clear();
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("profile line");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                CollectionAssert.AreEqual(new[] { "MIND PROFILE Line" }, emitted);

                // Fuzzy search "volley" -> executes verified `fire all`!
                emitted.Clear();
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("volley");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                CollectionAssert.AreEqual(new[] { "fire all" }, emitted);

                // 2. Spectator Replay Mode in Command Palette -> lists Follow Unit entries!
                arena.ExitArena();
                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 88, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 10000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [
                        { ""callsign"": ""R1"", ""team"": ""red"" },
                        { ""callsign"": ""B1"", ""team"": ""blue"" }
                    ],
                    ""keyframes"": [[0, [[16, 16, 90], [48, 48, 270]]]],
                    ""shots"": [], ""destroyed"": []
                }"));

                int followedIdx = -1;
                hud.CommandPalette.OnReplayFollowUnitRequested += idx =>
                {
                    followedIdx = idx;
                    arena.SelectReplayFocusUnit(idx);
                };
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("follow b1");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual(1, followedIdx);
                Assert.AreEqual(1, arena.ReplayFocusUnitIndex);

                // 3. 3-Button Arena Target Unit Frame (FIRE / SCAN / VOLLEY) dispatches exact verified verbs
                emitted.Clear();
                hud.UnitFrames.SetTarget(
                    "LOCK",
                    new UnityEngine.Color(0.76f, 0.20f, 0.18f, 0.96f),
                    "CONTACT B1",
                    "160m · REAR · 84%",
                    0.84f,
                    "SOLUTION 84%",
                    new UnityEngine.Color(0.92f, 0.42f, 0.24f, 0.95f),
                    "FIRE", () => arena.ExecuteCockpitAction("fire"),
                    "SCAN", () => arena.ExecuteCockpitAction("scan B1"),
                    "VOLLEY", () => arena.ExecuteCockpitAction("fire all"));
                Assert.AreEqual("FIRE", hud.UnitFrames.TargetAction1Label);
                Assert.AreEqual("SCAN", hud.UnitFrames.TargetAction2Label);
                Assert.AreEqual("VOLLEY", hud.UnitFrames.TargetAction3Label);
                hud.UnitFrames.TriggerTargetAction(0);
                hud.UnitFrames.TriggerTargetAction(1);
                hud.UnitFrames.TriggerTargetAction(2);
                CollectionAssert.AreEqual(new[] { "fire", "scan B1", "fire all" }, emitted);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaGunnerOpticsZoomAndRangefinderReticle_CyclesFovAndDisplaysLiveBallistics()
        {
            var rootGo = new UnityEngine.GameObject("Wave10PR1_ArenaGunnerOpticsTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var camGo = new UnityEngine.GameObject("OpticsCam");
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.AddComponent<UnityEngine.Camera>();
                var rig = rootGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                rig.ControlledCamera = cam;
                rig.ApplyCameraPreferences(100, 100, false, 62, savePrefs: false);

                var arenaGo = new UnityEngine.GameObject("Arena");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.TacticalBrackets.BindControllers(null, null, arena);
                hud.Overlays.BindControllers(null, arena, null);

                // 1. Camera rig [Q] optics zoom cycles 1x -> 2.5x -> 4x -> 1x in GroundVehicleChase and smoothly narrows FOV
                rig.SetMode(TacSim.Character.CameraRigMode.GroundVehicleChase);
                Assert.AreEqual(0, rig.ArenaOpticsZoomLevel);
                Assert.IsFalse(rig.IsArenaOpticsZoomed);
                Assert.AreEqual(62f, rig.TargetOpticsFov, 0.1f);

                int cycleCalls = 0;
                arena.OnCycleOpticsZoomRequested += () =>
                {
                    cycleCalls++;
                    int lvl = rig.CycleArenaOpticsZoom();
                    hud.Overlays.UpdateArenaOpticsButton(lvl);
                };

                Assert.IsTrue(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.Q), "[Q] hotkey must trigger optics zoom cycle");
                Assert.AreEqual(1, cycleCalls);
                Assert.AreEqual(1, rig.ArenaOpticsZoomLevel);
                Assert.IsTrue(rig.IsArenaOpticsZoomed);
                Assert.AreEqual(2.5f, rig.ArenaOpticsMagnification, 0.01f);
                Assert.Less(rig.TargetOpticsFov, 35f, "2.5x gunner zoom must narrow camera FOV significantly");
                StringAssert.Contains("2.5x", hud.Overlays.ArenaOpticsButtonSnapshot);

                rig.StepOpticsFov(1.0f);
                Assert.AreEqual(rig.TargetOpticsFov, cam.fieldOfView, 0.25f, "StepOpticsFov must converge camera FOV smoothly toward TargetOpticsFov");

                Assert.IsTrue(arena.HandleArenaCombatHotkey(UnityEngine.KeyCode.Q));
                Assert.AreEqual(2, rig.ArenaOpticsZoomLevel);
                Assert.AreEqual(4.0f, rig.ArenaOpticsMagnification, 0.01f);
                Assert.Less(rig.TargetOpticsFov, 22f, "4x sniper zoom must narrow camera FOV further");
                StringAssert.Contains("4x", hud.Overlays.ArenaOpticsButtonSnapshot);

                // Switching to Overhead auto-resets optics zoom to 1x; pressing [Q] from Overhead snaps into 2.5x Cupola optics!
                rig.SetMode(TacSim.Character.CameraRigMode.GroundVehicleOverhead);
                Assert.AreEqual(0, rig.ArenaOpticsZoomLevel);
                Assert.IsFalse(rig.IsArenaOpticsZoomed);
                Assert.AreEqual(62f, cam.fieldOfView, 0.1f);

                Assert.AreEqual(1, rig.CycleArenaOpticsZoom(), "Pressing [Q] in Overhead mode must snap directly into 2.5x Cupola optics");
                Assert.AreEqual(TacSim.Character.CameraRigMode.GroundVehicleCupola, rig.Mode);
                Assert.AreEqual(1, rig.ArenaOpticsZoomLevel);

                // Leaving Arena into ThirdPersonWalk resets optics zoom cleanly
                rig.SetMode(TacSim.Character.CameraRigMode.ThirdPersonWalk);
                Assert.AreEqual(0, rig.ArenaOpticsZoomLevel);
                Assert.AreEqual(62f, cam.fieldOfView, 0.1f);

                // 2. Diegetic Gunner Optics Reticle HUD renders live rangefinder, aspect, LOS, and weapon readiness
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 140, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""sight_m"": 240, ""weapons_free"": true, ""heat"": 200, ""heat_max"": 1000,
                    ""lock_designator"": ""B1"", ""lock_range_m"": 180, ""lock_percent"": 84,
                    ""weapons"": [
                        { ""id"": ""main_gun"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 18, ""rounds_max"": 20, ""hit_percent"": 84, ""range_band"": 1, ""bands_cm"": [12000, 24000, 36000] }
                    ],
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [118, 100], ""range_m"": 180, ""bearing_deg"": 90, ""facing"": ""rear"", ""aspect"": ""rear"", ""band"": ""close"", ""health"": ""intact"", ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 84 }] }
                    ]
                }"));

                // Wide 1x Chase view hides gunner optics reticle
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false, arenaOpticsZoomLevel: 0, arenaCupolaView: false);
                Assert.IsFalse(hud.TacticalBrackets.IsGunnerOpticsReticleVisible, "Optics reticle should stay hidden in wide 1x chase view");

                // 2.5x Gunner Zoom shows optics reticle with locked target rangefinder & ready weapon status
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false, arenaOpticsZoomLevel: 1, arenaCupolaView: false);
                Assert.IsTrue(hud.TacticalBrackets.IsGunnerOpticsReticleVisible);
                StringAssert.Contains("2.5x", hud.TacticalBrackets.GunnerOpticsModeSnapshot);
                StringAssert.Contains("LOCK B1", hud.TacticalBrackets.GunnerOpticsRangefinderSnapshot);
                StringAssert.Contains("180m", hud.TacticalBrackets.GunnerOpticsRangefinderSnapshot);
                StringAssert.Contains("84%", hud.TacticalBrackets.GunnerOpticsRangefinderSnapshot);
                StringAssert.Contains("REAR", hud.TacticalBrackets.GunnerOpticsRangefinderSnapshot);
                StringAssert.Contains("RDY", hud.TacticalBrackets.GunnerOpticsWeaponStatusSnapshot);

                // Cupola 1x also displays cupola visor reticle
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false, arenaOpticsZoomLevel: 0, arenaCupolaView: true);
                Assert.IsTrue(hud.TacticalBrackets.IsGunnerOpticsReticleVisible);
                StringAssert.Contains("CUPOLA", hud.TacticalBrackets.GunnerOpticsModeSnapshot);

                // Overlay / non-arena stage hides reticle cleanly
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: true, arenaOpticsZoomLevel: 2, arenaCupolaView: true);
                Assert.IsFalse(hud.TacticalBrackets.IsGunnerOpticsReticleVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaCombatAudioPolishAndOpsDirectoryIntel_SynthesizesLazySfxModulatesPitchAndDrivesKeySpots()
        {
            // 1. Validate phase-continuous, bounded procedural PCM buffers for Arena Target Lock & Kill Confirm
            float[] lockPcm = TacSimAudioSynthesizer.GenerateTargetLockToneSamples();
            float[] killPcm = TacSimAudioSynthesizer.GenerateKillConfirmChordSamples();
            Assert.Greater(lockPcm.Length, 1000);
            Assert.Greater(killPcm.Length, 1000);
            float lockPeak = 0f, killPeak = 0f;
            for (int i = 0; i < lockPcm.Length; i++)
            {
                Assert.GreaterOrEqual(lockPcm[i], -1f);
                Assert.LessOrEqual(lockPcm[i], 1f);
                if (System.Math.Abs(lockPcm[i]) > lockPeak) lockPeak = System.Math.Abs(lockPcm[i]);
                if (i > 0) Assert.LessOrEqual(System.Math.Abs(lockPcm[i] - lockPcm[i - 1]), 0.45f, "Lock tone must be smooth");
            }
            for (int i = 0; i < killPcm.Length; i++)
            {
                Assert.GreaterOrEqual(killPcm[i], -1f);
                Assert.LessOrEqual(killPcm[i], 1f);
                if (System.Math.Abs(killPcm[i]) > killPeak) killPeak = System.Math.Abs(killPcm[i]);
                if (i > 0) Assert.LessOrEqual(System.Math.Abs(killPcm[i] - killPcm[i - 1]), 0.45f, "Kill chord must be smooth");
            }
            Assert.Greater(lockPeak, 0.15f);
            Assert.Greater(killPeak, 0.15f);

            var rootGo = new UnityEngine.GameObject("Wave10PR2_ArenaAudioAndOpsIntelTest");
            try
            {
                // 2. Preserve AllocatedClipCount == 11 on InitializeAudioGraph, lazy-allocate new Arena SFX, modulate engine pitch
                var synth = rootGo.AddComponent<TacSimAudioSynthesizer>();
                synth.InitializeAudioGraph();
                Assert.AreEqual(11, synth.AllocatedClipCount, "InitializeAudioGraph must keep exact 11 eager clips");

                synth.PlayForCommandLine("lock B1");
                Assert.AreEqual(12, synth.AllocatedClipCount, "First lock command lazy-allocates TargetLockTone clip");
                synth.PlayKillConfirmChord();
                Assert.AreEqual(13, synth.AllocatedClipCount, "First kill confirm lazy-allocates KillConfirmChord clip");

                synth.SetArenaEngineThrottle(0f);
                Assert.AreEqual(0.95f, synth.CurrentAmbientPitch, 0.01f);
                synth.SetArenaEngineThrottle(1f);
                Assert.AreEqual(1.22f, synth.CurrentAmbientPitch, 0.01f);
                synth.ResetAmbientPitch();
                Assert.AreEqual(1.0f, synth.CurrentAmbientPitch, 0.005f);

                synth.DisposeClips();
                Assert.AreEqual(0, synth.AllocatedClipCount, "DisposeClips must cleanly release all clips including lazy Arena SFX");

                // 3. [N] Ops Directory in SimulatorArena lists surveyed KeySpots with DRIVE/SQUAD ADV + rich Arena battle intel
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("ArenaOpsTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 145, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 6000, ""y_cm"": 6000, ""heading_deg"": 90, ""speed_kph"": 27, ""max_speed_kph"": 54,
                    ""lock_designator"": ""B1"", ""lock_range_m"": 180, ""lock_percent"": 82,
                    ""mind"": {
                        ""name"": ""Armored Doctrine"", ""profiles"": [""Sniper"", ""Line""],
                        ""active"": 0, ""class"": ""I"", ""core"": ""II"", ""channels"": [""helm"", ""sensors"", ""fire""], ""off"": false
                    },
                    ""order"": { ""squad"": ""ALPHA"", ""objective"": ""hold"", ""to"": [128, 128], ""hold_fire"": false, ""expires_in_ms"": 45000 },
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [78, 60], ""range_m"": 180, ""bearing_deg"": 90, ""facing"": ""front"", ""aspect"": ""front"", ""band"": ""close"", ""health"": ""scarred"", ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 82 }] }
                    ]
                }"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.OpsDirectory.BindSources(null, null, null, null, null, null, arena);

                var arenaVerbs = new List<string>();
                hud.OpsDirectory.OnArenaActionRequested += arenaVerbs.Add;

                hud.OpsDirectory.UpdateLiveRoom(null, TacSimActiveStage.SimulatorArena);
                hud.OpsDirectory.SetOpen(true);
                Assert.GreaterOrEqual(hud.OpsDirectory.VisibleDirectoryRowCount, 3, "Arena Ops Directory must list surveyed battlefield KeySpots");
                StringAssert.Contains("[", hud.OpsDirectory.GetDirectoryRowLabel(0));

                // Primary action on KeySpot row dispatches `drive <letter>`; secondary action dispatches `order all advance <letter>`
                Assert.IsTrue(hud.OpsDirectory.TriggerDirectoryRowAction(0, secondary: false));
                Assert.IsTrue(hud.OpsDirectory.TriggerDirectoryRowAction(0, secondary: true));
                Assert.AreEqual(2, arenaVerbs.Count);
                StringAssert.StartsWith("drive ", arenaVerbs[0]);
                StringAssert.StartsWith("order all advance ", arenaVerbs[1]);

                // Tab 2 Active Ops displays rich Arena match, Mind doctrine, Standing Order, and Lock telemetry
                hud.OpsDirectory.SelectTab(TacSim.UI.OpsDirectoryTab.ActiveOps);
                StringAssert.Contains("ACTIVE ARENA BATTLEFIELD INTEL", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("MATCH #145", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("Armored Doctrine", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("ALPHA", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("LOCK B1", hud.OpsDirectory.OpsSummarySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DImpactExplosionsAndFloatingCombatPopups_TriggerInLiveCombatAndReplayTheater()
        {
            var rootGo = new UnityEngine.GameObject("Wave11PR1_Arena3DImpactsAndPopupsTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("ArenaVfxTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                Assert.AreEqual(0, arena.ActiveImpactBurstCount);
                Assert.AreEqual(0, arena.ActiveWorldPopupCount);

                // 1. Live Arena combat frame with hostile B1, then hit / target_section_lost / kill / miss notices
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 150, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""lock_designator"": ""B1"", ""lock_range_m"": 140, ""lock_percent"": 88,
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [114, 100], ""range_m"": 140, ""bearing_deg"": 90, ""facing"": ""front"", ""aspect"": ""front"", ""band"": ""close"", ""health"": ""intact"", ""odds"": [{ ""weapon"": ""main_gun"", ""percent"": 88 }] }
                    ],
                    ""notices"": [
                        { ""kind"": ""hit"", ""designator"": ""B1"", ""section"": ""front"", ""weapon"": ""main_gun"" }
                    ]
                }"));
                Assert.GreaterOrEqual(arena.ActiveImpactBurstCount, 1, "Hit notice must spawn a pooled 3D impact burst");
                Assert.GreaterOrEqual(arena.ActiveWorldPopupCount, 1, "Hit notice must spawn a 3D floating combat popup");
                Assert.AreEqual(TacSim.Arena.ArenaImpactKind.Hit, arena.LastImpactKind);
                StringAssert.Contains("HIT", arena.LastWorldPopupText);

                // Target section destroyed -> SectionCrit fireball + CRIT popup
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 150, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""lock_designator"": ""B1"", ""lock_range_m"": 140, ""lock_percent"": 88,
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [114, 100], ""range_m"": 140, ""bearing_deg"": 90, ""facing"": ""front"", ""aspect"": ""front"", ""band"": ""close"", ""health"": ""crippled"" }
                    ],
                    ""notices"": [
                        { ""kind"": ""target_section_lost"", ""designator"": ""B1"", ""section"": ""turret"" }
                    ]
                }"));
                Assert.AreEqual(TacSim.Arena.ArenaImpactKind.SectionCrit, arena.LastImpactKind);
                StringAssert.Contains("CRIT", arena.LastWorldPopupText);
                StringAssert.Contains("TURRET", arena.LastWorldPopupText);

                // Kill notice -> Kill fireball + DESTROYED popup
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 150, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [114, 100], ""range_m"": 140, ""bearing_deg"": 90, ""facing"": ""front"", ""aspect"": ""front"", ""health"": ""wreck"" }
                    ],
                    ""notices"": [
                        { ""kind"": ""kill"", ""designator"": ""B1"" }
                    ]
                }"));
                Assert.AreEqual(TacSim.Arena.ArenaImpactKind.Kill, arena.LastImpactKind);
                StringAssert.Contains("DESTROYED", arena.LastWorldPopupText);

                // Decay pooled bursts & popups cleanly to zero
                arena.StepTracersForTests(2.0f);
                Assert.AreEqual(0, arena.ActiveImpactBurstCount, "Impact bursts must decay cleanly after duration");
                Assert.AreEqual(0, arena.ActiveWorldPopupCount, "Floating world popups must decay cleanly after duration");

                // 2. 3D Replay Theater crossing shot + wreck timestamps spawns impact bursts & WRECKED! popup
                arena.ExitArena();
                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 92, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 8000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [
                        { ""callsign"": ""R1"", ""team"": ""red"" },
                        { ""callsign"": ""B1"", ""team"": ""blue"" }
                    ],
                    ""keyframes"": [
                        [0, [[16, 16, 90], [48, 16, 270]]],
                        [8000, [[16, 16, 90], [48, 16, 270]]]
                    ],
                    ""shots"": [
                        [1000, 0, 1, ""hit""]
                    ],
                    ""destroyed"": [
                        [2000, 1]
                    ]
                }"));
                arena.SeekReplayMs(500f);
                arena.SetReplaySpeed(1f);
                arena.ToggleReplayPlaying(); // Pause -> ensure explicit play state below
                if (!arena.ReplayPlaying) arena.ToggleReplayPlaying();

                // Step across t=1000ms hit shot
                arena.StepReplayPlaybackForTests(0.8f); // 500 -> 1300ms
                Assert.GreaterOrEqual(arena.ActiveImpactBurstCount, 1, "Replay hit shot must spawn 3D impact burst");
                StringAssert.Contains("HIT", arena.LastWorldPopupText);

                // Step across t=2000ms wreck event
                arena.StepReplayPlaybackForTests(1.0f); // 1300 -> 2300ms
                Assert.AreEqual(TacSim.Arena.ArenaImpactKind.Kill, arena.LastImpactKind, "Replay wreck timestamp must spawn Kill fireball");
                StringAssert.Contains("WRECKED", arena.LastWorldPopupText);
                StringAssert.Contains("B1", arena.LastWorldPopupText);

                // Reset clears all pooled 3D bursts & popups cleanly
                arena.ResetSessionState();
                Assert.AreEqual(0, arena.ActiveImpactBurstCount);
                Assert.AreEqual(0, arena.ActiveWorldPopupCount);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaLobbyChassisSpecsQuickReplaysAndReplayTheaterControls_ParseDispatchAndStepEvents()
        {
            var rootGo = new UnityEngine.GameObject("Wave11PR2_LobbySpecsAndReplayTheaterTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("ArenaLobbyReplay");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.ReplayHud.BindArena(arena);

                // 1. Chassis tactical spec dossier updates cleanly for all 6 official chassis
                string[] chassis = { "line_tank", "line_tank_laser", "heavy_mk1", "biped_mk1", "biped_mk2", "scout_mk1" };
                foreach (string c in chassis)
                {
                    arena.SelectedLobbyUnit = c;
                    hud.Overlays.RefreshLobbySelectionHighlights();
                    Assert.IsFalse(string.IsNullOrEmpty(hud.Overlays.LobbyChassisSpecSnapshot), $"Spec missing for {c}");
                }
                arena.SelectedLobbyUnit = "heavy_mk1";
                hud.Overlays.RefreshLobbySelectionHighlights();
                StringAssert.Contains("700", hud.Overlays.LobbyChassisSpecSnapshot);
                StringAssert.Contains("Missiles", hud.Overlays.LobbyChassisSpecSnapshot);

                arena.SelectedLobbyUnit = "biped_mk2";
                hud.Overlays.RefreshLobbySelectionHighlights();
                StringAssert.Contains("Thermal", hud.Overlays.LobbyChassisSpecSnapshot);

                // 2. SIM HISTORY server output parser extracts match IDs and launches sanitized `sim replay <id>`
                var dispatched = new List<string>();
                hud.Overlays.OnCommandRequested += dispatched.Add;

                Assert.AreEqual(0, hud.Overlays.RecentReplayPillCount);
                hud.Overlays.IngestSimHistoryOutput("The center's last 3 matches, newest first:\n" +
                    "  #42  red wins  3m12s  4 v 4  2026-11-26 17:00Z  (you flew R1)\n" +
                    "  #41  blue wins [hold]  5m02s  6 v 6  2026-11-26 16:40Z\n" +
                    "  #999999999999999999999999999  invalid overflow\n" +
                    "Room object #12 is a bolted terminal.");
                Assert.AreEqual(2, hud.Overlays.RecentReplayPillCount, "Only valid SIM HISTORY rows should create replay quick-launch pills");

                Assert.IsTrue(hud.Overlays.ClickRecentReplayPillForTests(0));
                Assert.IsTrue(hud.Overlays.ClickRecentReplayPillForTests(1));
                CollectionAssert.AreEqual(new[] { "sim replay 42", "sim replay 41" }, dispatched);

                // 3. Replay Theater << EVENT / EVENT >> stepping, [Q] optics button sync, and hotkeys
                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 42, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 10000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [
                        { ""callsign"": ""R1"", ""team"": ""red"" },
                        { ""callsign"": ""B1"", ""team"": ""blue"" }
                    ],
                    ""keyframes"": [
                        [0, [[16, 16, 90], [48, 16, 270]]],
                        [5000, [[24, 16, 90], [40, 16, 270]]],
                        [10000, [[30, 16, 90], [38, 16, 270]]]
                    ],
                    ""shots"": [
                        [1500, 0, 1, ""hit""],
                        [6500, 1, 0, ""hit""]
                    ],
                    ""destroyed"": [
                        [8500, 1]
                    ]
                }"));
                hud.ReplayHud.RefreshView();

                // Step forward across events 1500 -> 6500 -> 8500, then backward via << EVENT to 6500 -> 1500
                arena.SeekReplayMs(0f);
                hud.ReplayHud.ClickNextEventForTests();
                Assert.AreEqual(1500f, arena.ReplayCursorMs, 1f);
                hud.ReplayHud.ClickNextEventForTests();
                Assert.AreEqual(6500f, arena.ReplayCursorMs, 1f);
                hud.ReplayHud.ClickNextEventForTests();
                Assert.AreEqual(8500f, arena.ReplayCursorMs, 1f);
                hud.ReplayHud.ClickPrevEventForTests();
                Assert.AreEqual(6500f, arena.ReplayCursorMs, 1f);

                // Hotkeys: [[] steps prev event, [Q] requests optics zoom, [Tab] cycles focused unit
                int opticsRequested = 0;
                arena.OnCycleOpticsZoomRequested += () => opticsRequested++;
                Assert.IsTrue(arena.HandleReplayTheaterHotkey(UnityEngine.KeyCode.LeftBracket));
                Assert.AreEqual(1500f, arena.ReplayCursorMs, 1f);
                Assert.IsTrue(arena.HandleReplayTheaterHotkey(UnityEngine.KeyCode.Q));
                Assert.AreEqual(1, opticsRequested);
                Assert.IsTrue(arena.HandleReplayTheaterHotkey(UnityEngine.KeyCode.Tab));
                Assert.AreEqual(1, arena.ReplayFocusUnitIndex);

                hud.ReplayHud.UpdateOpticsButton(2);
                StringAssert.Contains("4x", hud.ReplayHud.ReplayOpticsButtonSnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DWeaponEnvelopeEcmDomeMatchClockAndNoticeDetails_ProjectRingsAndRenderRichFeed()
        {
            var rootGo = new UnityEngine.GameObject("Wave12PR1_ArenaWeaponEcmClockTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("ArenaRings");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.Minimap.BindWorldSources(null, null, null, null, arena);

                // 1. Live frame with t_ms=165000 (02:45), active ECM pod ("on"), weapon range bands (360m max), and structured notices
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 155, ""seated"": true, ""t_ms"": 165000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""sight_m"": 240, ""ecm"": ""on"",
                    ""weapons"": [
                        { ""id"": ""cannon"", ""ready"": false, ""ready_in_ms"": 2000, ""rounds"": 22, ""rounds_max"": 24, ""hit_percent"": 76, ""range_band"": 2, ""bands_cm"": [12000, 24000, 36000] }
                    ],
                    ""notices"": [
                        { ""kind"": ""emcon_on"", ""detail"": { ""radius_m"": 140 } },
                        { ""kind"": ""identified"", ""designator"": ""B2"", ""was"": ""?1"" },
                        { ""kind"": ""refused"", ""weapon"": ""cannon"", ""refusal"": ""cycling"", ""detail"": { ""secs"": 2 } }
                    ]
                }"));

                Assert.AreEqual(165000L, arena.State.ArenaClockMs);
                Assert.AreEqual(140f, arena.State.EcmRadiusMeters, 0.1f);
                Assert.IsTrue(arena.State.IsEcmJammingActive);
                Assert.IsTrue(arena.IsWeaponEnvelopeRingVisible, "3D Primary Weapon Envelope Ring must be visible when weapon has max range > 15m");
                Assert.IsTrue(arena.IsEcmJammerRingVisible, "3D ECM Jammer Dome Ring must be visible when ECM pod is active");

                // Scoreboard displays live match clock & Combat Feed displays structured ECM radius / ID / Cycling details
                hud.Overlays.RefreshArenaHud();
                StringAssert.Contains("2:45", hud.Overlays.ArenaScoreboardSnapshot);
                Assert.AreEqual(3, hud.Overlays.VisibleCombatFeedEntryCount);
                StringAssert.Contains("140m", hud.Overlays.CombatFeedSnapshot);
                StringAssert.Contains("?1", hud.Overlays.CombatFeedSnapshot);
                StringAssert.Contains("B2", hud.Overlays.CombatFeedSnapshot);
                StringAssert.Contains("2s", hud.Overlays.CombatFeedSnapshot);

                // Arena Minimap renders Weapon Range Ring and active ECM Jammer Ring
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 90f, rightCornerOccupied: true, forceRedraw: true);
                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasSensorRing);
                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasWeaponRangeRing);
                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasEcmRing);

                // Switching ECM pod to "off" (EMCON silent) hides 3D & Minimap ECM rings cleanly while preserving weapon envelope
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 155, ""seated"": true, ""t_ms"": 170000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""sight_m"": 240, ""ecm"": ""off"",
                    ""weapons"": [
                        { ""id"": ""cannon"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 22, ""rounds_max"": 24, ""hit_percent"": 76, ""range_band"": 2, ""bands_cm"": [12000, 24000, 36000] }
                    ]
                }"));
                Assert.IsFalse(arena.State.IsEcmJammingActive);
                Assert.IsFalse(arena.IsEcmJammerRingVisible, "EMCON silent must hide 3D ECM Jammer Dome");
                Assert.IsTrue(arena.IsWeaponEnvelopeRingVisible);
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 90f, rightCornerOccupied: true, forceRedraw: true);
                Assert.IsFalse(hud.Minimap.LastArenaMinimapHasEcmRing);

                // Reset hides both 3D tactical rings cleanly
                arena.ResetSessionState();
                Assert.IsFalse(arena.IsWeaponEnvelopeRingVisible);
                Assert.IsFalse(arena.IsEcmJammerRingVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaCharacterSheetDossierAndOverviewLockBestIffCounts_AutoFocusesAndLocksHighestOdds()
        {
            var rootGo = new UnityEngine.GameObject("Wave12PR2_CharacterSheetAndOverviewTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("ArenaDossierTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 160, ""seated"": true, ""t_ms"": 125000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.biped_mk2"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""heading_deg"": 90, ""speed_kph"": 24,
                    ""sight_m"": 260, ""scope"": ""hostile"", ""posture"": ""guard"", ""ecm"": ""on"",
                    ""receiver"": { ""budget"": 80, ""budget_max"": 100, ""reach_percent"": 80 },
                    ""lock_designator"": ""B2"", ""lock_range_m"": 150, ""lock_percent"": 91,
                    ""mind"": {
                        ""name"": ""Standard Doctrine"", ""profiles"": [""Sniper"", ""Line""],
                        ""active"": 1, ""class"": ""I"", ""core"": ""II"", ""channels"": [""helm"", ""sensors"", ""fire""], ""off"": false
                    },
                    ""weapons"": [
                        { ""id"": ""cannon"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 20, ""rounds_max"": 24, ""hit_percent"": 91, ""range_band"": 1, ""bands_cm"": [12000, 24000, 36000], ""dispersion_mrad"": 6 }
                    ],
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [130, 100], ""range_m"": 300, ""bearing_deg"": 90, ""facing"": ""front"", ""aspect"": ""front"", ""band"": ""far"", ""health"": ""intact"", ""odds"": [{ ""weapon"": ""cannon"", ""percent"": 42 }] },
                        { ""designator"": ""B2"", ""class"": ""scout_mk1"", ""party"": ""B"", ""friendly"": false, ""cell"": [115, 100], ""range_m"": 150, ""bearing_deg"": 90, ""facing"": ""rear"", ""aspect"": ""rear"", ""band"": ""close"", ""health"": ""crippled"", ""odds"": [{ ""weapon"": ""cannon"", ""percent"": 91 }] },
                        { ""designator"": ""R2"", ""class"": ""heavy_mk1"", ""party"": ""R"", ""friendly"": true, ""cell"": [90, 100], ""range_m"": 100, ""bearing_deg"": 270, ""facing"": ""front"", ""aspect"": ""front"", ""band"": ""close"", ""health"": ""intact"" }
                    ]
                }"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.CharacterSheet.BindSources(null, null, null, arena);
                hud.TacticalOverview.BindControllers(null, null, arena);

                // 1. Opening [C] Character Sheet during SimulatorArena auto-focuses GroundArena tab with full Combat Dossier
                hud.CharacterSheet.SetPilotContext("Voss", "Proving Ground", "R1", TacSimActiveStage.SimulatorArena);
                hud.CharacterSheet.SetOpen(true);
                Assert.AreEqual(TacSim.UI.CharacterSheetTab.GroundArena, hud.CharacterSheet.ActiveTab,
                    "Opening [C] during SimulatorArena must auto-focus Ground & Arena Rigs dossier tab");
                StringAssert.Contains("MATCH #160", hud.CharacterSheet.ArenaRigDossierSnapshot);
                StringAssert.Contains("2:05", hud.CharacterSheet.ArenaRigDossierSnapshot);
                StringAssert.Contains("Standard Doctrine", hud.CharacterSheet.ArenaRigDossierSnapshot);
                StringAssert.Contains("Line", hud.CharacterSheet.ArenaRigDossierSnapshot);
                StringAssert.Contains("ECM POD: ACTIVE", hud.CharacterSheet.ArenaRigDossierSnapshot);
                StringAssert.Contains("RCVR 80%", hud.CharacterSheet.ArenaRigDossierSnapshot);

                var arenaActions = new List<string>();
                hud.CharacterSheet.OnArenaActionRequested += arenaActions.Add;
                Assert.IsTrue(hud.CharacterSheet.ClickArenaDossierActionForTests("scan"));
                Assert.IsTrue(hud.CharacterSheet.ClickArenaDossierActionForTests("emcon"));
                CollectionAssert.AreEqual(new[] { "scan", "emcon off" }, arenaActions);

                // 2. Tactical Overview displays live IFF counts on tabs, health/band tags, and 1-click Lock Best hostile
                hud.TacticalOverview.SyncOverview(TacSimActiveStage.SimulatorArena, hideForOverlay: false, forceImmediate: true);
                Assert.AreEqual(3, hud.TacticalOverview.VisibleRowCount);
                StringAssert.Contains("3", hud.TacticalOverview.GetFilterTabLabelForTests(0));
                StringAssert.Contains("2", hud.TacticalOverview.GetFilterTabLabelForTests(1));
                StringAssert.Contains("1", hud.TacticalOverview.GetFilterTabLabelForTests(2));

                // Even when sorted by distance (R2=100m, B2=150m, B1=300m), LockBestArenaTarget locks B2 (91% highest odds hostile!)
                string lockedBest = null;
                hud.TacticalOverview.OnArenaContactLockClicked += d => lockedBest = d;
                Assert.IsTrue(hud.TacticalOverview.LockBestArenaTarget());
                Assert.AreEqual("B2", lockedBest, "LockBestArenaTarget must lock the highest-odds live hostile contact (B2 at 91%)");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DContactHealthBarsRichNameplatesAndLiveHoldBeacons_UpdateVisualTelemetry()
        {
            var rootGo = new UnityEngine.GameObject("Wave13PR1_3DHealthBarsAndHoldBeaconsTest");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var arenaGo = new UnityEngine.GameObject("Arena3DBars");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                var grid = TacSimTerrainDecoder.LoadStaticArena("proving_ground");
                arena.BuildArenaTerrain(catalog, grid);

                Assert.GreaterOrEqual(grid.KeySpots.Count, 1, "Proving ground must define at least one KeySpot");
                var holdSpot = grid.KeySpots[0];
                for (int i = 0; i < grid.KeySpots.Count; i++)
                {
                    if (string.Equals(grid.KeySpots[i].Letter, grid.HoldSpotLetter, System.StringComparison.OrdinalIgnoreCase))
                    {
                        holdSpot = grid.KeySpots[i];
                        break;
                    }
                }
                UnityEngine.Vector3 holdLocal = grid.SiteCellToLocalPos(holdSpot.CellX, holdSpot.CellY, 0f);
                float siteCellM = grid.CellSizeM / System.Math.Max(1, grid.SpanCells);
                int holdWorldXcm = UnityEngine.Mathf.RoundToInt((grid.OriginCellX * siteCellM + holdLocal.x) * 100f);
                int holdWorldYcm = UnityEngine.Mathf.RoundToInt((grid.OriginCellY * siteCellM + holdLocal.z) * 100f);

                // 1. Live frame with 3 contacts at distinct health states (intact, scarred, crippled) & player inside hold zone
                arena.ApplyLiveCockpitFrame(JsonNode.Parse($@"{{
                    ""match"": 170, ""seated"": true, ""t_ms"": 90000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": {holdWorldXcm}, ""y_cm"": {holdWorldYcm}, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""lock_designator"": ""B2"", ""lock_range_m"": 140, ""lock_percent"": 84,
                    ""contacts"": [
                        {{ ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [220, 220], ""range_m"": 320, ""aspect"": ""front"", ""health"": ""intact"", ""odds"": [{{ ""percent"": 45 }}] }},
                        {{ ""designator"": ""B2"", ""class"": ""scout_mk1"", ""party"": ""B"", ""friendly"": false, ""cell"": [210, 210], ""range_m"": 140, ""aspect"": ""rear"", ""moving"": true, ""health"": ""crippled"", ""odds"": [{{ ""percent"": 84 }}] }},
                        {{ ""designator"": ""R2"", ""class"": ""heavy_mk1"", ""party"": ""R"", ""friendly"": true, ""cell"": [200, 200], ""range_m"": 90, ""aspect"": ""left"", ""health"": ""scarred"" }}
                    ]
                }}"));
                arena.SyncArenaTacticalVisuals();

                // Verify 3D contact health bars track intact / scarred / crippled fills accurately
                Assert.IsTrue(arena.IsContactHealthBarVisibleForTests("B1"));
                Assert.AreEqual(1.0f, arena.GetContactHealthBarFillFracForTests("B1"), 0.02f);
                Assert.IsTrue(arena.IsContactHealthBarVisibleForTests("R2"));
                Assert.AreEqual(0.62f, arena.GetContactHealthBarFillFracForTests("R2"), 0.02f);
                Assert.IsTrue(arena.IsContactHealthBarVisibleForTests("B2"));
                Assert.AreEqual(0.28f, arena.GetContactHealthBarFillFracForTests("B2"), 0.02f);

                // Verify rich 3D locked nameplate contains lock header, odds %, aspect, [CRIT], and moving marker
                string b2Tag = arena.GetContactNameplateTextForTests("B2");
                StringAssert.Contains(">> LOCKED <<", b2Tag);
                StringAssert.Contains("84%", b2Tag);
                StringAssert.Contains("REAR", b2Tag);
                StringAssert.Contains("[CRIT]", b2Tag);

                string r2Tag = arena.GetContactNameplateTextForTests("R2");
                StringAssert.Contains("[SCAR]", r2Tag);

                // Verify KeySpot label shows live distance in meters and RED ownership while sole Red player occupies Hold Zone
                Assert.AreEqual("RED", arena.ActiveHoldZoneVisualOwner);
                string holdLbl = arena.GetKeySpotLabelForTests(holdSpot.Letter);
                StringAssert.Contains("m", holdLbl);
                StringAssert.Contains("[RED]", holdLbl);

                // 2. Blue contact enters Hold Zone -> CONTESTED; B2 becomes wreck -> health bar hides and [WRECK] tag appears
                arena.ApplyLiveCockpitFrame(JsonNode.Parse($@"{{
                    ""match"": 170, ""seated"": true, ""t_ms"": 95000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": {holdWorldXcm}, ""y_cm"": {holdWorldYcm}, ""heading_deg"": 90, ""speed_kph"": 0,
                    ""contacts"": [
                        {{ ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [{holdSpot.CellX}, {holdSpot.CellY}], ""range_m"": 12, ""aspect"": ""front"", ""health"": ""intact"" }},
                        {{ ""designator"": ""B2"", ""class"": ""scout_mk1"", ""party"": ""B"", ""friendly"": false, ""cell"": [210, 210], ""range_m"": 140, ""health"": ""wreck"", ""reads_dead"": true }}
                    ]
                }}"));
                arena.SyncArenaTacticalVisuals();

                Assert.AreEqual("CONTESTED", arena.ActiveHoldZoneVisualOwner);
                StringAssert.Contains("[CONTESTED]", arena.GetKeySpotLabelForTests(holdSpot.Letter));
                Assert.IsFalse(arena.IsContactHealthBarVisibleForTests("B2"), "Wrecked contact must hide its 3D world-space health bar");
                StringAssert.Contains("[WRECK]", arena.GetContactNameplateTextForTests("B2"));

                // 3. Authoritative Blue HoldControlScore (< -5) flips Hold Zone beacon cleanly to BLUE
                arena.ApplyLiveCockpitFrame(JsonNode.Parse($@"{{
                    ""match"": 170, ""seated"": true, ""t_ms"": 100000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": {holdWorldXcm}, ""y_cm"": {holdWorldYcm}, ""hold_score"": -25,
                    ""contacts"": []
                }}"));
                arena.SyncArenaTacticalVisuals();
                Assert.AreEqual("BLUE", arena.ActiveHoldZoneVisualOwner);
                StringAssert.Contains("[BLUE]", arena.GetKeySpotLabelForTests(holdSpot.Letter));
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaTacticalSettingsVfxShakeAndSimLobbyHotbar_ScalesShakeTogglesVfxAndDispatchesVerbs()
        {
            var rootGo = new UnityEngine.GameObject("Wave13PR2_ArenaSettingsAndLobbyHotbarTest");
            try
            {
                var camGo = new UnityEngine.GameObject("ShakeCam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var camRig = camGo.AddComponent<TacSim.Character.TacSimCameraRig>();
                camRig.ControlledCamera = camGo.GetComponent<UnityEngine.Camera>();

                var arenaGo = new UnityEngine.GameObject("ArenaVfxPrefTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.DisplayOptions.BindSubsystems(null, camRig);
                hud.DisplayOptions.OnArenaVfxPreferencesChanged += () => arena.SyncArenaTacticalVisuals();

                // 1. Camera shake scaling: 100% -> 50% -> OFF cleanly scales & suppresses CurrentShakeOffset
                camRig.SetCameraShakeScalePct(100, savePrefs: false);
                camRig.TriggerCameraShake(1.0f, 0.30f);
                camRig.StepCameraShake(0.05f);
                float magFull = camRig.CurrentShakeOffset.magnitude;
                Assert.Greater(magFull, 0.05f, "100% shake scale must produce active shake displacement");

                hud.DisplayOptions.CycleCameraShakePreset(save: false); // 100 -> 50
                Assert.AreEqual(50, camRig.CameraShakeScalePct);
                StringAssert.Contains("50%", hud.DisplayOptions.ArenaShakeButtonSnapshot);
                // Reset active impulse and trigger fresh impulse at 50%
                camRig.StepCameraShake(1.0f);
                camRig.TriggerCameraShake(1.0f, 0.30f);
                camRig.StepCameraShake(0.05f);
                float magHalf = camRig.CurrentShakeOffset.magnitude;
                Assert.AreEqual(magFull * 0.5f, magHalf, 0.005f, "50% shake scale must halve impulse amplitude cleanly");

                hud.DisplayOptions.CycleCameraShakePreset(save: false); // 50 -> 0 (OFF)
                Assert.AreEqual(0, camRig.CameraShakeScalePct);
                StringAssert.Contains("OFF", hud.DisplayOptions.ArenaShakeButtonSnapshot);
                camRig.TriggerCameraShake(1.5f, 0.40f);
                camRig.StepCameraShake(0.05f);
                Assert.AreEqual(0f, camRig.CurrentShakeOffset.magnitude, 0.0001f, "OFF shake preset must fully suppress camera shake");

                // 2. 3D Tactical Rings & 3D Combat Popups toggles hide rings & suppress popups live
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 180, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""sight_m"": 240, ""ecm"": ""on"",
                    ""weapons"": [{ ""id"": ""cannon"", ""ready"": true, ""rounds"": 20, ""rounds_max"": 24, ""bands_cm"": [12000, 32000] }]
                }"));
                Assert.IsTrue(arena.IsArenaRangeRingVisible);
                Assert.IsTrue(arena.IsWeaponEnvelopeRingVisible);
                Assert.IsTrue(arena.IsEcmJammerRingVisible);

                hud.DisplayOptions.SetShowArenaTacticalRings(false, save: false);
                StringAssert.Contains("OFF", hud.DisplayOptions.ArenaRingsButtonSnapshot);
                Assert.IsFalse(arena.IsArenaRangeRingVisible, "Disabling 3D Rings must hide sensor range ring");
                Assert.IsFalse(arena.IsWeaponEnvelopeRingVisible, "Disabling 3D Rings must hide weapon envelope ring");
                Assert.IsFalse(arena.IsEcmJammerRingVisible, "Disabling 3D Rings must hide ECM jammer ring");

                hud.DisplayOptions.SetShowArenaCombatPopups(false, save: false);
                StringAssert.Contains("OFF", hud.DisplayOptions.ArenaPopupsButtonSnapshot);
                arena.SpawnWorldCombatPopup(UnityEngine.Vector3.zero, "SUPPRESSED", UnityEngine.Color.yellow);
                Assert.AreEqual(0, arena.ActiveWorldPopupCount, "Disabling 3D Popups must suppress floating world combat popups");

                // Reset restores Camera Shake 100%, 3D Rings ON, and 3D Popups ON
                hud.DisplayOptions.ResetCameraAndHudDefaults(save: false);
                Assert.AreEqual(100, camRig.CameraShakeScalePct);
                Assert.IsTrue(TacSim.Arena.OocArenaController.ShowArenaTacticalRings);
                Assert.IsTrue(TacSim.Arena.OocArenaController.ShowArenaCombatPopups);
                Assert.IsTrue(arena.IsArenaRangeRingVisible);
                arena.SpawnWorldCombatPopup(UnityEngine.Vector3.zero, "RESTORED", UnityEngine.Color.green);
                Assert.AreEqual(1, arena.ActiveWorldPopupCount);

                // 3. Simulator Lobby & Sim Pod Bay contextual 1-9 quick-action hotbar emits verified registered verbs
                var cmds = new List<string>();
                hud.OnCommandSubmitted += cmds.Add;
                hud.RefreshContextualQuickActions("room.simulator_lobby", isSeatedAtHelm: false);
                Assert.AreEqual(7, hud.QuickActionSlotCount);
                StringAssert.Contains("Match History", hud.GetQuickActionLabelForTests(4));
                StringAssert.Contains("Start Match", hud.GetQuickActionLabelForTests(5));
                Assert.IsTrue(hud.TriggerQuickActionSlot(3)); // sim status
                Assert.IsTrue(hud.TriggerQuickActionSlot(4)); // sim history
                Assert.IsTrue(hud.TriggerQuickActionSlot(5)); // sim start
                CollectionAssert.AreEqual(new[] { "sim status", "sim history", "sim start" }, cmds);

                cmds.Clear();
                hud.RefreshContextualQuickActions("room.sim_red", isSeatedAtHelm: false);
                Assert.AreEqual(6, hud.QuickActionSlotCount);
                Assert.IsTrue(hud.TriggerQuickActionSlot(0)); // board
                Assert.IsTrue(hud.TriggerQuickActionSlot(3)); // sim history
                Assert.IsTrue(hud.TriggerQuickActionSlot(4)); // lobby
                CollectionAssert.AreEqual(new[] { "board", "sim history", "lobby" }, cmds);
            }
            finally
            {
                TacSim.Arena.OocArenaController.ShowArenaTacticalRings = true;
                TacSim.Arena.OocArenaController.ShowArenaCombatPopups = true;
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaOffscreenThreatBracketsWaypointCompassAndHoldTint_ProjectAndClickToLock()
        {
            var rootGo = new UnityEngine.GameObject("Wave14PR1_OffscreenThreatsAndWaypointTest");
            try
            {
                var camGo = new UnityEngine.GameObject("ArenaCam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.GetComponent<UnityEngine.Camera>();
                cam.fieldOfView = 60f;
                cam.nearClipPlane = 0.5f;
                cam.farClipPlane = 4000f;

                var arenaGo = new UnityEngine.GameObject("ArenaBracketsTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                var grid = TacSimTerrainDecoder.LoadStaticArena("proving_ground");
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), grid);

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.TacticalBrackets.BindControllers(null, null, arena);

                // Place player at cell (100, 100) facing North (+Z), hostile B1 behind player at (100, 75),
                // friendly R2 behind player at (95, 75), and hold_score = +20 (RED).
                float siteCellM = grid.CellSizeM / System.Math.Max(1, grid.SpanCells);
                int ownXcm = UnityEngine.Mathf.RoundToInt((grid.OriginCellX + 100) * siteCellM * 100f);
                int ownYcm = UnityEngine.Mathf.RoundToInt((grid.OriginCellY + 100) * siteCellM * 100f);

                arena.ApplyLiveCockpitFrame(JsonNode.Parse($@"{{
                    ""match"": 190, ""seated"": true, ""t_ms"": 45000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": {ownXcm}, ""y_cm"": {ownYcm}, ""heading_deg"": 0, ""speed_kph"": 12,
                    ""hold_score"": 20,
                    ""contacts"": [
                        {{ ""designator"": ""B1"", ""class"": ""scout_mk1"", ""party"": ""B"", ""friendly"": false, ""cell"": [100, 75], ""range_m"": 160, ""aspect"": ""front"", ""health"": ""intact"", ""odds"": [{{ ""percent"": 62 }}] }},
                        {{ ""designator"": ""R2"", ""class"": ""line_tank"", ""party"": ""R"", ""friendly"": true, ""cell"": [95, 75], ""range_m"": 170, ""aspect"": ""front"", ""health"": ""intact"" }}
                    ]
                }}"));
                // Issue a drive waypoint so both 3D HUD waypoint bracket and Compass Tape pip activate
                arena.ExecuteCockpitAction("drive 120,120");
                arena.SyncArenaTacticalVisuals();

                // Position camera slightly behind player looking straight North (+Z), so B1 & R2 (south of player) are strictly behind camera
                UnityEngine.Vector3 ownWorld = arena.PlayerVehicleTransform.position;
                cam.transform.position = ownWorld + new UnityEngine.Vector3(0f, 8f, -12f);
                cam.transform.rotation = UnityEngine.Quaternion.LookRotation(UnityEngine.Vector3.forward, UnityEngine.Vector3.up);
                hud.TacticalBrackets.ScreenProjectionOverrideForTests = wp =>
                {
                    UnityEngine.Vector3 rel = UnityEngine.Quaternion.Inverse(cam.transform.rotation) * (wp - cam.transform.position);
                    float z = rel.z;
                    float sx = 960f + (rel.x / UnityEngine.Mathf.Max(1f, UnityEngine.Mathf.Abs(z))) * 900f;
                    float sy = 540f + (rel.y / UnityEngine.Mathf.Max(1f, UnityEngine.Mathf.Abs(z))) * 900f;
                    return new UnityEngine.Vector3(sx, sy, z);
                };

                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false);

                // 1. Hold Zone bracket carries RED ownership tag
                Assert.IsTrue(hud.TacticalBrackets.ArenaObjectiveBracketActive);
                StringAssert.Contains("RED", hud.TacticalBrackets.GetActiveBracketTagSnapshot(0));

                // 2. Drive Waypoint activates both 3D screen-space bracket and Compass Tape pip
                Assert.IsTrue(hud.TacticalBrackets.ArenaDriveWaypointBracketActive, "Active drive waypoint must project cyan HUD bracket");
                Assert.IsTrue(hud.TacticalBrackets.ArenaCompassWaypointPipVisible, "Active drive waypoint must light up Compass Tape pip");
                StringAssert.Contains("[WP]", hud.TacticalBrackets.GetActiveBracketTagSnapshot(1));

                // 3. Off-screen hostile B1 projects a clamped interactive THREAT edge indicator while off-screen friendly R2 stays hidden
                Assert.AreEqual(1, hud.TacticalBrackets.OffscreenThreatBracketCount, "Only off-screen hostile B1 should clamp as a threat edge indicator");
                string threatTag = hud.TacticalBrackets.GetActiveBracketTagSnapshot(2);
                StringAssert.Contains("[B1]", threatTag);
                StringAssert.Contains("THREAT", threatTag);

                // Clicking the off-screen threat edge bracket immediately emits lock designator "B1"
                string clickedDesig = null;
                hud.TacticalBrackets.OnArenaBracketClicked += d => clickedDesig = d;
                Assert.IsTrue(hud.TacticalBrackets.ClickBracketForTests(2));
                Assert.AreEqual("B1", clickedDesig);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaReplayRosterCombatStatsHotkeysAnd3DSpectatorBrackets_TrackAndFocusUnits()
        {
            var rootGo = new UnityEngine.GameObject("Wave14PR2_ReplayRosterStatsAnd3DBracketsTest");
            try
            {
                var camGo = new UnityEngine.GameObject("ReplayCam", typeof(UnityEngine.Camera));
                camGo.transform.SetParent(rootGo.transform, false);
                var cam = camGo.GetComponent<UnityEngine.Camera>();

                var arenaGo = new UnityEngine.GameObject("ArenaReplayStats");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.ReplayHud.BindArena(arena);
                hud.TacticalBrackets.BindControllers(null, null, arena);

                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 210, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 10000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [
                        { ""callsign"": ""R1"", ""team"": ""red"" },
                        { ""callsign"": ""B1"", ""team"": ""blue"" },
                        { ""callsign"": ""R2"", ""team"": ""red"" }
                    ],
                    ""keyframes"": [
                        [0, [[16, 16, 90], [48, 16, 270], [16, 32, 90]]],
                        [5000, [[24, 16, 90], [40, 16, 270], [24, 32, 90]]],
                        [10000, [[30, 16, 90], [38, 16, 270], [30, 32, 90]]]
                    ],
                    ""shots"": [
                        [1500, 0, 1, ""hit""],
                        [3000, 0, 1, ""miss""],
                        [6000, 0, 1, ""hit""]
                    ],
                    ""destroyed"": [
                        [6000, 1]
                    ]
                }"));

                // 1. Scrub to t=0 -> 0 hits/kills; scrub to t=7000 -> R1 shows 2H/3S and 1K + TOP GUN summary
                arena.SeekReplayMs(0f);
                hud.ReplayHud.RefreshView();
                StringAssert.DoesNotContain("1K", hud.ReplayHud.GetRosterRowSnapshotForTests(0));

                arena.SeekReplayMs(7000f);
                hud.ReplayHud.RefreshView();
                string r1Row = hud.ReplayHud.GetRosterRowSnapshotForTests(0);
                StringAssert.Contains("2H/3S", r1Row);
                StringAssert.Contains("1K", r1Row);
                StringAssert.Contains("R1", hud.ReplayHud.TopGunSummarySnapshot);
                StringAssert.Contains("TOP GUN", hud.ReplayHud.ReplayStatsSnapshot);

                // 2. [2] jumps camera directly to roster index 1 (B1); [R] rewinds replay to 0ms and resumes playing
                Assert.IsTrue(arena.HandleReplayTheaterHotkey(UnityEngine.KeyCode.Alpha2));
                Assert.AreEqual(1, arena.ReplayFocusUnitIndex);
                arena.ReplayPlaying = false;
                Assert.IsTrue(arena.HandleReplayTheaterHotkey(UnityEngine.KeyCode.R));
                Assert.AreEqual(0f, arena.ReplayCursorMs, 0.5f);
                Assert.IsTrue(arena.ReplayPlaying, "[R] hotkey must restart replay playback from 0:00");

                // 3. 3D Spectator Brackets hide live cockpit widgets and project clickable [CAM] + unit brackets
                arena.SelectReplayFocusUnit(0);
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.SimulatorArena, cam, hideForOverlay: false);
                Assert.IsFalse(hud.TacticalBrackets.IsArenaArmorSchematicVisible, "Live armor schematic must hide during Replay Theater");
                Assert.IsFalse(hud.TacticalBrackets.IsArenaCompassVisible, "Live compass tape must hide during Replay Theater");
                Assert.AreEqual(3, hud.TacticalBrackets.ActiveBracketCount, "All 3 active replay rigs must project clickable 3D brackets");
                StringAssert.Contains("[CAM]", hud.TacticalBrackets.GetActiveBracketTagSnapshot(0));
                StringAssert.Contains("R1", hud.TacticalBrackets.GetActiveBracketTagSnapshot(0));

                int clickedRosterIdx = -1;
                hud.TacticalBrackets.OnReplayBracketClicked += idx => clickedRosterIdx = idx;
                Assert.IsTrue(hud.TacticalBrackets.ClickBracketForTests(2));
                Assert.AreEqual(2, clickedRosterIdx, "Clicking 3D replay bracket for R2 must emit roster index 2");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DBiomeSunLightingRoadMarkersAndPerimeterBoundary_BuildFromTerrainGrid()
        {
            var rootGo = new UnityEngine.GameObject("Wave15PR1_BiomeSunRoadMarkersPerimeterTest");
            try
            {
                // 1. Biome atmosphere profiles provide distinct sky tints & angled directional sun vectors
                var desert = TacSim.Arena.TacSimArenaEnvironmentBuilder.ResolveBiomeLighting("desert");
                var arctic = TacSim.Arena.TacSimArenaEnvironmentBuilder.ResolveBiomeLighting("arctic");
                var regolith = TacSim.Arena.TacSimArenaEnvironmentBuilder.ResolveBiomeLighting("regolith");
                var temperate = TacSim.Arena.TacSimArenaEnvironmentBuilder.ResolveBiomeLighting("temperate");
                Assert.AreNotEqual(desert.SkyColor, arctic.SkyColor);
                Assert.AreNotEqual(regolith.SkyColor, temperate.SkyColor);
                Assert.Greater(desert.SunDirectionEuler.x, 10f);
                Assert.Greater(arctic.SunDirectionEuler.x, 10f);

                // 2. Build a synthetic terrain grid containing roads & platforms and verify batched 3D road markers
                var grid = TacSimTerrainDecoder.LoadStaticArena("proving_ground");
                for (int x = 4; x < System.Math.Min(grid.Width - 4, 48); x++)
                {
                    grid.Classes[12 * grid.Width + x] = TacSim.World.TerrainCellClass.Road;
                    grid.Classes[14 * grid.Width + x] = TacSim.World.TerrainCellClass.Platform;
                }

                var arenaGo = new UnityEngine.GameObject("ArenaLandmarks");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), grid);

                Assert.Greater(arena.BuiltRoadMarkerCount, 0, "Road and platform cells must spawn batched 3D guide markers");
                Assert.IsTrue(arena.IsPerimeterBoundaryBuilt, "Arena terrain build must project terrain-hugging 3D perimeter line");
                Assert.AreEqual(4, arena.BuiltPerimeterPylonCount, "Arena boundary must build 4 corner sector pylons (NW/NE/SE/SW)");
                Assert.AreEqual(UnityEngine.LightType.Directional, arena.ActiveSunLightType, "Arena sun light must be directional for crisp battlefield shading");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaWeaponCooldownBarsMindSwitchCooldownAndDebriefReplay_UpdateAndDispatch()
        {
            var rootGo = new UnityEngine.GameObject("Wave15PR2_WeaponBarsMindCooldownDebriefTest");
            try
            {
                var arenaGo = new UnityEngine.GameObject("ArenaCockpitHudTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);

                // 1. Frame with cycling cannon (4000ms), ready laser (84% locked hit odds), empty missile rack (0/8), and Mind switch CD 3500ms
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 225, ""seated"": true, ""t_ms"": 165000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.heavy_mk1"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""lock_designator"": ""B1"", ""lock_range_m"": 180, ""lock_percent"": 84,
                    ""mind"": {
                        ""name"": ""Aegis Doctrine"", ""profiles"": [""Line"", ""Sniper""], ""active"": 0,
                        ""class"": ""I"", ""core"": ""II"", ""channels"": [""helm"", ""sensors"", ""fire""],
                        ""next_switch_ms"": 3500, ""off"": false
                    },
                    ""weapons"": [
                        { ""id"": ""cannon"", ""ready"": false, ""ready_in_ms"": 4000, ""rounds"": 18, ""rounds_max"": 24, ""percent"": 76 },
                        { ""id"": ""laser"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 0, ""rounds_max"": 0, ""percent"": 84 },
                        { ""id"": ""missiles"", ""ready"": true, ""ready_in_ms"": 0, ""rounds"": 0, ""rounds_max"": 8, ""percent"": 62 }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();

                Assert.AreEqual(3, hud.Overlays.ActiveWeaponSlotButtonCount);
                Assert.AreEqual(0f, hud.Overlays.GetWeaponSlotCooldownFillForTests(0), 0.02f, "Peak cycling weapon starts at 0% recharge fill");
                Assert.AreEqual(1f, hud.Overlays.GetWeaponSlotCooldownFillForTests(1), 0.02f, "Ready laser must show 100% fill");
                StringAssert.Contains("84%", hud.Overlays.GetWeaponSlotLabelForTests(1));
                Assert.IsTrue(hud.Overlays.IsWeaponSlotInteractableForTests(1));
                StringAssert.Contains("OUT", hud.Overlays.GetWeaponSlotLabelForTests(2));
                Assert.IsFalse(hud.Overlays.IsWeaponSlotInteractableForTests(2), "Out-of-ammo weapon slot must be non-interactable");

                // Mind strip displays switch cooldown and disables inactive profile button while cooling down
                StringAssert.Contains("CD 3.5s", hud.Overlays.MindStripHeaderSnapshot);
                Assert.IsFalse(hud.Overlays.IsMindProfileButtonInteractableForTests(1), "Inactive Mind profile button must wait for switch cooldown");

                hud.Overlays.ToggleRulesDrawerForTests();
                StringAssert.Contains("CHANNELS:", hud.Overlays.RulesDrawerSnapshot);
                StringAssert.Contains("HELM", hud.Overlays.RulesDrawerSnapshot);

                // 2. Halfway through cannon reload (2000ms of 4000ms) -> 50% fill; Mind switch CD clears (0ms) -> Sniper button re-enables
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 225, ""seated"": true, ""t_ms"": 167000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.heavy_mk1"", ""mode"": ""skirmish"", ""power"": ""online"",
                    ""x_cm"": 10000, ""y_cm"": 10000, ""lock_designator"": ""B1"", ""lock_range_m"": 180, ""lock_percent"": 84,
                    ""mind"": {
                        ""name"": ""Aegis Doctrine"", ""profiles"": [""Line"", ""Sniper""], ""active"": 0,
                        ""class"": ""I"", ""core"": ""II"", ""channels"": [""helm"", ""sensors"", ""fire""],
                        ""next_switch_ms"": 0, ""off"": false
                    },
                    ""weapons"": [
                        { ""id"": ""cannon"", ""ready"": false, ""ready_in_ms"": 2000, ""rounds"": 18, ""rounds_max"": 24, ""percent"": 76 }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();
                Assert.AreEqual(0.5f, hud.Overlays.GetWeaponSlotCooldownFillForTests(0), 0.05f, "Half-cycled weapon must show 50% fill bar");
                Assert.IsTrue(hud.Overlays.IsMindProfileButtonInteractableForTests(1), "Clearing Mind switch cooldown must re-enable profile switching");

                // 3. Wrecked debrief card displays match survival clock (2:45) and 1-click REPLAY #225 emits `sim replay 225`
                var cmds = new List<string>();
                hud.Overlays.OnCommandRequested += cmds.Add;
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 225, ""seated"": true, ""t_ms"": 165000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.heavy_mk1"", ""mode"": ""skirmish"", ""power"": ""offline"", ""wreck"": true,
                    ""x_cm"": 10000, ""y_cm"": 10000
                }"));
                hud.Overlays.RefreshArenaHud();
                Assert.IsTrue(hud.Overlays.IsDebriefCardVisible);
                StringAssert.Contains("2:45", hud.Overlays.DebriefSummarySnapshot);
                Assert.IsTrue(hud.Overlays.ClickDebriefWatchReplayForTests());
                CollectionAssert.AreEqual(new[] { "sim replay 225" }, cmds);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaMinimapLockVectorHoldFactionTintAndReplayShotLines_RenderTelemetry()
        {
            var rootGo = new UnityEngine.GameObject("Wave16PR1_MinimapVectorsAndReplayShotsTest");
            try
            {
                var arenaGo = new UnityEngine.GameObject("ArenaMinimapEnhanced");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                var grid = TacSimTerrainDecoder.LoadStaticArena("proving_ground");
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), grid);

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Minimap.BindWorldSources(null, null, null, null, arena);

                // 1. Live Arena frame locked on B1 with RED hold control -> lock vector active & hold owner RED
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 230, ""seated"": true, ""t_ms"": 60000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 12000, ""y_cm"": 12000, ""heading_deg"": 45, ""sight_m"": 240,
                    ""hold_score"": 25, ""lock_designator"": ""B1"", ""lock_range_m"": 160, ""lock_percent"": 82,
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [18, 18], ""range_m"": 160, ""health"": ""intact"" }
                    ]
                }"));
                arena.SyncArenaTacticalVisuals();
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 45f, rightCornerOccupied: true, forceRedraw: true);

                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasLockLine, "Minimap must render target lock vector when locked onto a live contact");
                Assert.IsTrue(hud.Minimap.LastArenaMinimapHasObjective);
                Assert.AreEqual("RED", hud.Minimap.LastArenaMinimapHoldOwner);

                // Unlocking target and switching Hold Zone to BLUE clears lock vector and flips minimap hold tint to BLUE
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 230, ""seated"": true, ""t_ms"": 65000, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""x_cm"": 12000, ""y_cm"": 12000, ""heading_deg"": 45, ""sight_m"": 240,
                    ""hold_score"": -25, ""lock_designator"": """",
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [18, 18], ""range_m"": 160, ""health"": ""intact"" }
                    ]
                }"));
                arena.SyncArenaTacticalVisuals();
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 45f, rightCornerOccupied: true, forceRedraw: true);
                Assert.IsFalse(hud.Minimap.LastArenaMinimapHasLockLine, "Unlocking contact must clear minimap target lock vector");
                Assert.AreEqual("BLUE", hud.Minimap.LastArenaMinimapHoldOwner);

                // 2. Replay Theater minimap draws active firefight shot tracer lines within [curMs - 1500ms .. curMs]
                arena.ExitArena();
                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 231, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 8000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [
                        { ""callsign"": ""R1"", ""team"": ""red"" },
                        { ""callsign"": ""B1"", ""team"": ""blue"" }
                    ],
                    ""keyframes"": [
                        [0, [[16, 16, 90], [40, 16, 270]]],
                        [8000, [[22, 16, 90], [34, 16, 270]]]
                    ],
                    ""shots"": [
                        [1200, 0, 1, ""hit""],
                        [1800, 1, 0, ""miss""]
                    ],
                    ""destroyed"": []
                }"));

                arena.SeekReplayMs(2000f);
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 0f, rightCornerOccupied: true, forceRedraw: true);
                Assert.AreEqual(2, hud.Minimap.LastReplayMinimapShotLineCount, "Replay minimap must draw both recent shots within 1.5s window");

                arena.SeekReplayMs(6000f);
                hud.Minimap.SyncMinimap(TacSimActiveStage.SimulatorArena, UnityEngine.Vector3.zero, 0f, rightCornerOccupied: true, forceRedraw: true);
                Assert.AreEqual(0, hud.Minimap.LastReplayMinimapShotLineCount, "Expired replay shots older than 1.5s must clear from minimap");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaCommandPaletteSquadPostureReplayAndKeybindPresets_ExecuteAndSwapMacros()
        {
            var rootGo = new UnityEngine.GameObject("Wave16PR2_PaletteAndKeybindPresetsTest");
            try
            {
                var arenaGo = new UnityEngine.GameObject("ArenaPaletteTest");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.CommandPalette.BindWorldSources(null, null, arena);
                hud.CommandPalette.SetActiveStage(TacSimActiveStage.SimulatorArena);

                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""match"": 240, ""seated"": true, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""mode"": ""hold"", ""power"": ""online"",
                    ""posture"": ""return"", ""x_cm"": 10000, ""y_cm"": 10000
                }"));

                var arenaVerbs = new List<string>();
                var uiActs = new List<string>();
                hud.CommandPalette.OnArenaActionRequested += arenaVerbs.Add;
                hud.CommandPalette.OnUiActionRequested += uiActs.Add;

                // 1. Live Arena Command Palette executes Squad Engage, Hull Posture GUARD, and Gunner Optics cycle
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("squad engage");
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());

                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("posture guard");
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());

                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("gunner optics");
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());

                CollectionAssert.AreEqual(new[] { "order all engage", "MIND POSTURE guard" }, arenaVerbs);
                CollectionAssert.AreEqual(new[] { "ui:cycle_optics" }, uiActs);

                // 2. Replay Theater Command Palette executes Previous Highlight and Restart 0:00
                arena.ExitArena();
                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 241, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 10000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [{ ""callsign"": ""R1"", ""team"": ""red"" }, { ""callsign"": ""B1"", ""team"": ""blue"" }],
                    ""keyframes"": [[0, [[16, 16, 90], [40, 16, 270]]], [10000, [[24, 16, 90], [34, 16, 270]]]],
                    ""shots"": [[2000, 0, 1, ""hit""], [6000, 1, 0, ""hit""]],
                    ""destroyed"": [[8000, 1]]
                }"));
                arena.SeekReplayMs(7500f);

                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("previous combat highlight");
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual(6000f, arena.ReplayCursorMs, 1f, "Previous highlight palette entry must jump back to 6000ms hit");

                arena.ReplayPlaying = false;
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("restart replay");
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual(0f, arena.ReplayCursorMs, 1f);
                Assert.IsTrue(arena.ReplayPlaying);

                // 3. [F1] Keybind Help includes Replay Theater hotkeys and 1-click Arena / Space F5-F8 macro preset swapping
                StringAssert.Contains("[R] 0:00", hud.KeybindHelp.ArenaKeybindCardSnapshot);
                StringAssert.Contains("[1–6/Tab]", hud.KeybindHelp.ArenaKeybindCardSnapshot);

                hud.KeybindHelp.ApplyArenaMacroPreset(persist: false);
                Assert.AreEqual("scan", hud.KeybindHelp.GetMacroCommand(0));
                Assert.AreEqual("emcon", hud.KeybindHelp.GetMacroCommand(1));
                Assert.AreEqual("order all engage", hud.KeybindHelp.GetMacroCommand(2));
                Assert.AreEqual("speed 0", hud.KeybindHelp.GetMacroCommand(3));

                hud.KeybindHelp.ApplySpaceMacroPreset(persist: false);
                Assert.AreEqual("+ship", hud.KeybindHelp.GetMacroCommand(0));
                Assert.AreEqual("+tac/scan", hud.KeybindHelp.GetMacroCommand(3));
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaHitImpactAudioSynthesizerAndKillStreakMilestoneBanners_SynthesizeAndFade()
        {
            var rootGo = new UnityEngine.GameObject("Wave17PR1_ArenaAudioAndMilestoneBannersTest");
            try
            {
                // 1. Waveform synthesis bounds & non-silence checks
                float[] hitSamples = TacSimAudioSynthesizer.GenerateHitConfirmTickSamples();
                float[] impactSamples = TacSimAudioSynthesizer.GenerateArmorImpactThudSamples();
                Assert.Greater(hitSamples.Length, 1000);
                Assert.Greater(impactSamples.Length, 2000);
                float hitPeak = 0f, impPeak = 0f;
                for (int i = 0; i < hitSamples.Length; i++)
                {
                    Assert.IsFalse(float.IsNaN(hitSamples[i]) || float.IsInfinity(hitSamples[i]));
                    hitPeak = System.Math.Max(hitPeak, System.Math.Abs(hitSamples[i]));
                }
                for (int i = 0; i < impactSamples.Length; i++)
                {
                    Assert.IsFalse(float.IsNaN(impactSamples[i]) || float.IsInfinity(impactSamples[i]));
                    impPeak = System.Math.Max(impPeak, System.Math.Abs(impactSamples[i]));
                }
                Assert.Greater(hitPeak, 0.10f);
                Assert.LessOrEqual(hitPeak, 1.0f);
                Assert.Greater(impPeak, 0.15f);
                Assert.LessOrEqual(impPeak, 1.0f);

                // 2. Lazy allocation preserves AllocatedClipCount == 11 after InitializeAudioGraph()
                var audio = rootGo.AddComponent<TacSimAudioSynthesizer>();
                audio.InitializeAudioGraph();
                Assert.AreEqual(11, audio.AllocatedClipCount, "New Arena combat SFX clips must remain lazy-allocated");
                audio.PlayHitConfirmTick(isCrit: true);
                Assert.AreEqual(12, audio.AllocatedClipCount);
                audio.PlayArmorImpactThud();
                Assert.AreEqual(13, audio.AllocatedClipCount);
                audio.DisposeClips();
                Assert.AreEqual(0, audio.AllocatedClipCount, "DisposeClips must destroy lazy combat SFX clips cleanly");

                // 3. Kill-streak formatter & priority upper-center Milestone Banner lifecycle
                StringAssert.Contains("FIRST BLOOD", TacSim.UI.TacSimFloatingCallouts.FormatKillStreakTitle(1, "B1"));
                StringAssert.Contains("DOUBLE KILL", TacSim.UI.TacSimFloatingCallouts.FormatKillStreakTitle(2, "B2"));
                StringAssert.Contains("TRIPLE KILL", TacSim.UI.TacSimFloatingCallouts.FormatKillStreakTitle(3, "B3"));
                StringAssert.Contains("QUAD KILL", TacSim.UI.TacSimFloatingCallouts.FormatKillStreakTitle(4, "B4"));
                StringAssert.Contains("UNSTOPPABLE (5 KILLS)", TacSim.UI.TacSimFloatingCallouts.FormatKillStreakTitle(5, "B5"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                Assert.IsFalse(hud.FloatingCallouts.IsMilestoneBannerVisible);
                hud.FloatingCallouts.ShowMilestoneBanner("DOUBLE KILL — B2 DESTROYED!", UnityEngine.Color.red, 2.0f);
                Assert.IsTrue(hud.FloatingCallouts.IsMilestoneBannerVisible);
                StringAssert.Contains("DOUBLE KILL", hud.FloatingCallouts.ActiveMilestoneBannerText);
                hud.FloatingCallouts.StepAnimation(1.0f);
                Assert.IsTrue(hud.FloatingCallouts.IsMilestoneBannerVisible);
                hud.FloatingCallouts.StepAnimation(1.2f);
                Assert.IsFalse(hud.FloatingCallouts.IsMilestoneBannerVisible, "Expired milestone banner must hide cleanly");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaSignalIntelTabAndDetachmentBriefing_AutoFocusesRendersAndDispatchesVerbs()
        {
            var rootGo = new UnityEngine.GameObject("Wave17PR2_ArenaSignalIntelTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                var arenaGo = new UnityEngine.GameObject("ArenaIntelCtrl");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(null, null);
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""seated"": true,
                    ""match"": 312,
                    ""callsign"": ""R1"",
                    ""party"": ""R"",
                    ""mode"": ""hold"",
                    ""power"": ""online"",
                    ""scope"": ""hostile"",
                    ""sight_m"": 260,
                    ""sensor_height_cm"": 350,
                    ""ecm"": ""on"",
                    ""receiver"": { ""budget"": 8, ""budget_max"": 12, ""reach_percent"": 85 },
                    ""detachment"": {
                        ""id"": ""7"", ""revision"": ""2"", ""term"": ""1"", ""leader_slot"": 1,
                        ""all_order_prefill"": ""ORDER DT:7:2:1 ALL "",
                        ""slots"": [
                            { ""slot"": 1, ""callsign"": ""R1"", ""pilot"": ""Voss"", ""controller"": ""player"", ""order_prefill"": ""ORDER DT:7:2:1 1 "" },
                            { ""slot"": 2, ""callsign"": ""R2"", ""pilot"": null, ""controller"": ""automated"", ""order_prefill"": ""ORDER DT:7:2:1 2 "" }
                        ],
                        ""my_order"": { ""sequence"": ""3"", ""from"": ""R1"", ""objective"": ""HOLD RIDGE"", ""response"": null },
                        ""wilco_command"": ""WILCO DT:7:2 3"",
                        ""negative_command"": ""NEGATIVE DT:7:2 3""
                    },
                    ""mind"": {
                        ""name"": ""IRON GUARD"",
                        ""profiles"": [""return"", ""guard""],
                        ""active"": 1,
                        ""channels"": [""sight"", ""radar""]
                    },
                    ""contacts"": [
                        { ""designator"": ""R2"", ""class"": ""line_tank"", ""party"": ""R"", ""friendly"": true, ""relation"": ""friendly"", ""cell"": [12, 12], ""range_m"": 90, ""bearing_deg"": 180, ""aspect"": ""front"", ""health"": ""intact"" },
                        { ""designator"": ""B2"", ""class"": ""scout_buggy"", ""party"": ""B"", ""friendly"": false, ""relation"": ""hostile"", ""cell"": [22, 20], ""range_m"": 310, ""bearing_deg"": 65, ""aspect"": ""flank"", ""odds"": [{ ""weapon"": ""cannon"", ""percent"": 52 }], ""health"": ""scarred"", ""moving"": true },
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""relation"": ""hostile"", ""cell"": [16, 15], ""range_m"": 180, ""bearing_deg"": 45, ""aspect"": ""rear"", ""odds"": [{ ""weapon"": ""cannon"", ""percent"": 84 }], ""health"": ""intact"" }
                    ]
                }"));

                hud.SceneIntelModal.BindArena(arena);
                hud.SceneIntelModal.SetActiveStage(TacSim.Core.TacSimActiveStage.SimulatorArena);

                // 1. Opening [L] in SimulatorArena auto-focuses Tab 1 and renders live sensor/ECM/detachment intel
                hud.SceneIntelModal.SelectTab(TacSim.UI.SceneIntelTab.RpScene);
                hud.SceneIntelModal.SetOpen(true);
                Assert.AreEqual(TacSim.UI.SceneIntelTab.SignalContacts, hud.SceneIntelModal.ActiveTab, "Opening [L] in SimulatorArena must auto-focus Signal Contacts tab");
                Assert.IsTrue(hud.SceneIntelModal.IsArenaSignalModeActive);
                StringAssert.Contains("ARENA #312", hud.SceneIntelModal.ContactsHeaderSnapshot);
                StringAssert.Contains("260m", hud.SceneIntelModal.ContactsHeaderSnapshot);
                StringAssert.Contains("3.5m", hud.SceneIntelModal.ContactDetailSnapshot);
                StringAssert.Contains("JAMMING (120m)", hud.SceneIntelModal.ContactDetailSnapshot);
                StringAssert.Contains("85%", hud.SceneIntelModal.ContactDetailSnapshot);
                StringAssert.Contains("8/12", hud.SceneIntelModal.ContactDetailSnapshot);
                StringAssert.Contains("DETACHMENT #7", hud.SceneIntelModal.ContactDetailSnapshot);
                StringAssert.Contains("HOLD RIDGE", hud.SceneIntelModal.ContactDetailSnapshot);

                // 2. Hostile contacts sort ahead of friendlies and nearest-first (B1 before B2 before R2)
                Assert.AreEqual(3, hud.SceneIntelModal.VisibleContactRowCount);
                StringAssert.Contains("B1", hud.SceneIntelModal.GetContactRowSnapshotForTests(0));
                StringAssert.Contains("REAR", hud.SceneIntelModal.GetContactRowSnapshotForTests(0));
                StringAssert.Contains("84%", hud.SceneIntelModal.GetContactRowSnapshotForTests(0));
                StringAssert.Contains("B2", hud.SceneIntelModal.GetContactRowSnapshotForTests(1));
                StringAssert.Contains("R2", hud.SceneIntelModal.GetContactRowSnapshotForTests(2));

                // 3. Clicking a contact row locks it, updates dossier card, and contextual Arena buttons dispatch verified verbs
                var verbs = new List<string>();
                hud.SceneIntelModal.OnCommandRequested += verbs.Add;
                Assert.IsTrue(hud.SceneIntelModal.ClickContactRowForTests(0));
                StringAssert.Contains("TARGET B1", hud.SceneIntelModal.ContactDetailSnapshot);
                hud.SceneIntelModal.ClickPrimaryIntelActionForTests();
                hud.SceneIntelModal.ClickSecondaryIntelActionForTests();
                hud.SceneIntelModal.ClickArenaMindCardForTests();
                CollectionAssert.AreEqual(new[] { "lock B1", "scan", "emcon", "mind" }, verbs);

                // 4. Leaving SimulatorArena cleanly restores IC `tactics.contacts` propagation ledger mode
                arena.ExitArena();
                hud.SceneIntelModal.SetActiveStage(TacSim.Core.TacSimActiveStage.StationWalk);
                Assert.IsFalse(hud.SceneIntelModal.IsArenaSignalModeActive);
                StringAssert.Contains("SIGNAL PROPAGATION CONTACTS", hud.SceneIntelModal.ContactsHeaderSnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Arena3DHeadingTurretAimLinesAndAnimatedSmokePlumes_ProjectToggleAndBillow()
        {
            var rootGo = new UnityEngine.GameObject("Wave18PR1_3DAimLinesAndSmokePlumesTest");
            bool prevRings = TacSim.Arena.OocArenaController.ShowArenaTacticalRings;
            try
            {
                TacSim.Arena.OocArenaController.ShowArenaTacticalRings = true;
                var arenaGo = new UnityEngine.GameObject("ArenaAimLinesCtrl");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                // 1. Live cockpit projects 3D hull heading vector & turret bore sightline
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""seated"": true, ""match"": 401, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""power"": ""online"",
                    ""x_cm"": 12000, ""y_cm"": 12000, ""heading_deg"": 90,
                    ""lock_designator"": ""B1"", ""lock_range_m"": 140, ""lock_percent"": 76,
                    ""contacts"": [
                        { ""designator"": ""B1"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [12, 26], ""range_m"": 140, ""bearing_deg"": 0, ""health"": ""crippled"" }
                    ]
                }"));
                arena.SyncArenaTacticalVisuals();

                Assert.IsTrue(arena.IsPlayerHeadingLineVisible, "3D hull heading indicator must be visible when ShowArenaTacticalRings is enabled");
                Assert.IsTrue(arena.IsPlayerTurretAimLineVisible, "3D turret aim sightline must be visible when ShowArenaTacticalRings is enabled");

                // 2. Comfort toggle hides both 3D heading and turret aim sightlines cleanly
                TacSim.Arena.OocArenaController.ShowArenaTacticalRings = false;
                arena.SyncArenaTacticalVisuals();
                Assert.IsFalse(arena.IsPlayerHeadingLineVisible);
                Assert.IsFalse(arena.IsPlayerTurretAimLineVisible);

                TacSim.Arena.OocArenaController.ShowArenaTacticalRings = true;
                arena.SyncArenaTacticalVisuals();
                Assert.IsTrue(arena.IsPlayerHeadingLineVisible);
                Assert.IsTrue(arena.IsPlayerTurretAimLineVisible);

                // 3. Damaged & wrecked 3D rigs dynamically billow rising smoke & ember plumes
                Assert.IsTrue(arena.TryGetContactRig("B1", out var b1Rig));
                Assert.IsTrue(b1Rig.IsDamageSmokeActive, "Crippled rig must show damage smoke plume");
                b1Rig.UpdateArticulation(0.25f, 0f, null);
                float h1 = b1Rig.ActiveSmokePlumeHeightOffset;
                float phase1 = b1Rig.SmokePlumeAnimPhase;
                b1Rig.UpdateArticulation(0.35f, 0f, null);
                Assert.Greater(b1Rig.SmokePlumeAnimPhase, phase1, "Damage smoke plume animation phase must advance");
                Assert.AreNotEqual(h1, b1Rig.ActiveSmokePlumeHeightOffset, "Damage smoke puffs must animate vertically across ticks");

                b1Rig.SetDamageState("wreck", wrecked: true);
                Assert.IsTrue(b1Rig.IsWreckSmokeActive, "Wrecked rig must enable smoldering fire/smoke marker");
                Assert.IsNotNull(b1Rig.WreckSmokePlumeA);
                Assert.IsNotNull(b1Rig.WreckSmokePlumeB);
                float wreckY1 = b1Rig.WreckSmokePlumeA.localPosition.y;
                b1Rig.UpdateArticulation(0.40f, 0f, null);
                Assert.AreNotEqual(wreckY1, b1Rig.WreckSmokePlumeA.localPosition.y, "Wreck smoke plume must rise/billow dynamically");

                // 4. Player wreck disables player heading & turret aim sightlines
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""seated"": true, ""match"": 401, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""power"": ""offline"", ""wreck"": true,
                    ""x_cm"": 12000, ""y_cm"": 12000, ""heading_deg"": 90
                }"));
                arena.SyncArenaTacticalVisuals();
                Assert.IsFalse(arena.IsPlayerHeadingLineVisible);
                Assert.IsFalse(arena.IsPlayerTurretAimLineVisible);
            }
            finally
            {
                TacSim.Arena.OocArenaController.ShowArenaTacticalRings = prevRings;
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaCockpitContactAspectBadgesLockBestAndSectionBreakdown_SortFormatAndDispatch()
        {
            var rootGo = new UnityEngine.GameObject("Wave18PR2_CockpitAspectBadgesAndSectionsTest");
            try
            {
                var arenaGo = new UnityEngine.GameObject("ArenaHudBadgesCtrl");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);

                var verbs = new List<string>();
                hud.Overlays.OnArenaCockpitVerbRequested += verbs.Add;

                // 1. Live cockpit with multi-section armor & mixed contacts (B2 has higher hit odds + rear aspect than B1)
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""seated"": true, ""match"": 412, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""power"": ""online"",
                    ""x_cm"": 12000, ""y_cm"": 12000, ""heading_deg"": 90,
                    ""sections"": [
                        { ""id"": ""hull"", ""condition"": ""intact"", ""internals"": 100, ""internals_max"": 100, ""facets"": [{ ""id"": ""front"", ""armor"": 400, ""armor_max"": 400 }] },
                        { ""id"": ""turret"", ""condition"": ""scarred"", ""internals"": 74, ""internals_max"": 100, ""facets"": [] },
                        { ""id"": ""drive"", ""condition"": ""crippled"", ""internals"": 40, ""internals_max"": 100, ""facets"": [] }
                    ],
                    ""contacts"": [
                        { ""designator"": ""R2"", ""class"": ""line_tank"", ""party"": ""R"", ""friendly"": true, ""cell"": [13, 12], ""range_m"": 80, ""aspect"": ""front"", ""health"": ""intact"" },
                        { ""designator"": ""B1"", ""class"": ""heavy_mk1"", ""party"": ""B"", ""friendly"": false, ""cell"": [20, 12], ""range_m"": 160, ""aspect"": ""left"", ""band"": ""medium"", ""odds"": [{ ""weapon"": ""cannon"", ""percent"": 48 }], ""health"": ""scarred"" },
                        { ""designator"": ""B2"", ""class"": ""scout_mk1"", ""party"": ""B"", ""friendly"": false, ""cell"": [24, 14], ""range_m"": 210, ""aspect"": ""rear"", ""band"": ""close"", ""moving"": true, ""odds"": [{ ""weapon"": ""cannon"", ""percent"": 86 }], ""health"": ""intact"" },
                        { ""designator"": ""B3"", ""class"": ""line_tank"", ""party"": ""B"", ""friendly"": false, ""cell"": [18, 16], ""range_m"": 110, ""aspect"": ""front"", ""health"": ""wreck"", ""reads_dead"": true }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();

                // Highest hit-odds hostile B2 sorts ahead of B1, followed by friendly R2 and wreck B3
                string row0 = hud.Overlays.GetArenaContactRowLabelForTests(0);
                string row1 = hud.Overlays.GetArenaContactRowLabelForTests(1);
                string row2 = hud.Overlays.GetArenaContactRowLabelForTests(2);
                string row3 = hud.Overlays.GetArenaContactRowLabelForTests(3);
                StringAssert.Contains("B2", row0);
                StringAssert.Contains("REAR", row0);
                StringAssert.Contains("CLOSE", row0);
                StringAssert.Contains("86%", row0);
                StringAssert.Contains("▶", row0);
                StringAssert.Contains("B1", row1);
                StringAssert.Contains("LEFT", row1);
                StringAssert.Contains("MED", row1);
                StringAssert.Contains("48%", row1);
                StringAssert.Contains("R2", row2);
                StringAssert.Contains("B3", row3);

                // 1-click LOCK BEST button targets B2 and dispatches verified `lock B2` verb
                Assert.IsTrue(hud.Overlays.IsLockBestHostileInteractable);
                StringAssert.Contains("B2", hud.Overlays.LockBestHostileButtonSnapshot);
                StringAssert.Contains("86%", hud.Overlays.LockBestHostileButtonSnapshot);
                Assert.IsTrue(hud.Overlays.ClickLockBestHostileForTests());
                CollectionAssert.AreEqual(new[] { "lock B2" }, verbs);

                // Own-platform card renders per-section internal integrity breakdown
                StringAssert.Contains("SECTIONS:", hud.Overlays.ArenaOwnArmorSnapshot);
                StringAssert.Contains("HULL 100%", hud.Overlays.ArenaOwnArmorSnapshot);
                StringAssert.Contains("TURRET 74%", hud.Overlays.ArenaOwnArmorSnapshot);
                StringAssert.Contains("DRIVE 40%", hud.Overlays.ArenaOwnArmorSnapshot);

                // 2. When no live hostiles remain, LOCK BEST becomes non-interactable cleanly
                arena.ApplyLiveCockpitFrame(JsonNode.Parse(@"{
                    ""seated"": true, ""match"": 412, ""callsign"": ""R1"", ""party"": ""R"",
                    ""unit_type"": ""tacsim.unit.line_tank"", ""power"": ""online"",
                    ""x_cm"": 12000, ""y_cm"": 12000, ""heading_deg"": 90,
                    ""contacts"": [
                        { ""designator"": ""R2"", ""class"": ""line_tank"", ""party"": ""R"", ""friendly"": true, ""cell"": [13, 12], ""range_m"": 80, ""health"": ""intact"" }
                    ]
                }"));
                hud.Overlays.RefreshArenaHud();
                Assert.IsFalse(hud.Overlays.IsLockBestHostileInteractable);
                StringAssert.Contains("NO HOSTILES", hud.Overlays.LockBestHostileButtonSnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaLobbyQuickMatchPresetsAndCombatAudioPreview_EmitVerbsAndSynthesize()
        {
            var rootGo = new UnityEngine.GameObject("Wave19PR1_LobbyPresetsAndCombatAudioPreviewTest");
            try
            {
                var arenaGo = new UnityEngine.GameObject("ArenaLobbyPresetsCtrl");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var audio = rootGo.AddComponent<TacSimAudioSynthesizer>();
                audio.InitializeAudioGraph();

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(null, arena, null);
                hud.DisplayOptions.BindSubsystems(audio, null);

                // 1. Lobby Quick Match Presets emit exact verified `sim ai` + `sim mode` sequences and update checklist
                var cmds = new List<string>();
                hud.Overlays.OnCommandRequested += cmds.Add;
                hud.Overlays.ShowArenaLobbyModal(true);

                hud.Overlays.ApplyLobbyQuickPreset(1, "skirmish");
                Assert.AreEqual(1, arena.SelectedLobbyAiCount);
                Assert.AreEqual("skirmish", arena.SelectedLobbyMode);
                StringAssert.Contains("1v1", hud.Overlays.LobbySummarySnapshot);
                StringAssert.Contains("CHECKLIST:", hud.Overlays.LobbySummarySnapshot);

                hud.Overlays.ApplyLobbyQuickPreset(4, "skirmish");
                Assert.AreEqual(4, arena.SelectedLobbyAiCount);

                hud.Overlays.ApplyLobbyQuickPreset(6, "hold");
                Assert.AreEqual(6, arena.SelectedLobbyAiCount);
                Assert.AreEqual("hold", arena.SelectedLobbyMode);
                StringAssert.Contains("6v6", hud.Overlays.LobbySummarySnapshot);
                StringAssert.Contains("HOLD", hud.Overlays.LobbySummarySnapshot);

                CollectionAssert.AreEqual(
                    new[] { "sim ai 1", "sim mode skirmish", "sim ai 4", "sim mode skirmish", "sim ai 6", "sim mode hold" },
                    cmds);

                // 2. [Esc] Audio Mixer `TEST HIT/KILL` preview alternates hit-confirm tick and kill-confirm chord cleanly
                Assert.AreEqual(11, audio.AllocatedClipCount, "Audio graph starts with 11 baseline clips before combat preview");
                Assert.AreEqual("hit_tick", hud.DisplayOptions.TriggerCombatSfxPreview());
                Assert.AreEqual("hit_tick", hud.DisplayOptions.LastAudioPreviewKind);
                Assert.AreEqual(12, audio.AllocatedClipCount, "First combat preview must lazy-allocate HitConfirmTick");

                Assert.AreEqual("kill_chord", hud.DisplayOptions.TriggerCombatSfxPreview());
                Assert.AreEqual("kill_chord", hud.DisplayOptions.LastAudioPreviewKind);
                Assert.AreEqual(13, audio.AllocatedClipCount, "Second combat preview must lazy-allocate KillConfirmChord");

                audio.DisposeClips();
                Assert.AreEqual(0, audio.AllocatedClipCount);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ArenaReplayTeamRosterFilterAndClickableEventFeedSeek_FilterAndJump()
        {
            var rootGo = new UnityEngine.GameObject("Wave19PR2_ReplayTeamFilterAndSeekFeedTest");
            try
            {
                var arenaGo = new UnityEngine.GameObject("ArenaReplayFilterCtrl");
                arenaGo.transform.SetParent(rootGo.transform, false);
                var arena = arenaGo.AddComponent<TacSim.Arena.OocArenaController>();
                arena.BuildArenaTerrain(TacSimAssetCatalog.LoadOrDefault(), TacSimTerrainDecoder.LoadStaticArena("proving_ground"));

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.ReplayHud.BindArena(arena);

                arena.HandleReplayPanelFrame(JsonNode.Parse(@"{
                    ""schema"": { ""id"": ""arena.replay"", ""shape"": ""replay"", ""version"": 1 },
                    ""match"": 509, ""arena"": ""proving_ground"", ""cell_cm"": 1000, ""width"": 64, ""height"": 64,
                    ""end_ms"": 12000, ""outcome"": ""red"", ""reason"": ""elimination"", ""mode"": ""skirmish"",
                    ""roster"": [
                        { ""callsign"": ""R1"", ""team"": ""red"" },
                        { ""callsign"": ""R2"", ""team"": ""red"" },
                        { ""callsign"": ""B1"", ""team"": ""blue"" },
                        { ""callsign"": ""B2"", ""team"": ""blue"" }
                    ],
                    ""keyframes"": [
                        [0, [[10, 10, 90], [10, 14, 90], [42, 10, 270], [42, 14, 270]]],
                        [12000, [[18, 10, 90], [18, 14, 90], [36, 10, 270], [36, 14, 270]]]
                    ],
                    ""shots"": [
                        [2500, 0, 2, ""hit""],
                        [5000, 2, 0, ""miss""],
                        [8000, 1, 3, ""hit""]
                    ],
                    ""destroyed"": [
                        [9500, 3]
                    ]
                }"));
                arena.SeekReplayMs(11000f);
                hud.ReplayHud.RefreshView();

                // 1. Roster team filter toggles cleanly between ALL (4), RED (2), and BLUE (2)
                Assert.AreEqual(TacSim.UI.ReplayRosterTeamFilter.All, hud.ReplayHud.ActiveRosterTeamFilter);
                Assert.AreEqual(4, hud.ReplayHud.VisibleRosterRowCount);

                hud.ReplayHud.SetRosterTeamFilter(TacSim.UI.ReplayRosterTeamFilter.Red);
                Assert.AreEqual(2, hud.ReplayHud.VisibleRosterRowCount);
                StringAssert.Contains("R1", hud.ReplayHud.GetRosterRowSnapshotForTests(0));
                StringAssert.Contains("R2", hud.ReplayHud.GetRosterRowSnapshotForTests(1));

                hud.ReplayHud.SetRosterTeamFilter(TacSim.UI.ReplayRosterTeamFilter.Blue);
                Assert.AreEqual(2, hud.ReplayHud.VisibleRosterRowCount);
                StringAssert.Contains("B1", hud.ReplayHud.GetRosterRowSnapshotForTests(0));
                StringAssert.Contains("B2", hud.ReplayHud.GetRosterRowSnapshotForTests(1));

                hud.ReplayHud.SetRosterTeamFilter(TacSim.UI.ReplayRosterTeamFilter.All);
                Assert.AreEqual(4, hud.ReplayHud.VisibleRosterRowCount);

                // 2. Clickable timeline event feed lists recent wreck + shots newest-first and seeks cursor directly on click
                Assert.AreEqual(4, hud.ReplayHud.VisibleClickableEventRowCount, "Expected 1 wreck + 3 shots as clickable rows");
                StringAssert.Contains("B2", hud.ReplayHud.GetClickableEventRowLabelForTests(0));
                StringAssert.Contains("WRECKED", hud.ReplayHud.GetClickableEventRowLabelForTests(0));
                StringAssert.Contains("R1", hud.ReplayHud.GetClickableEventRowLabelForTests(3));

                // Click oldest shot row (index 3 -> 2500ms) jumps cursor straight to 2500ms and pauses
                arena.ReplayPlaying = true;
                Assert.IsTrue(hud.ReplayHud.ClickEventFeedRowForTests(3));
                Assert.AreEqual(2500f, arena.ReplayCursorMs, 1f, "Clicking event row must jump ReplayCursorMs directly to event timestamp");
                Assert.IsFalse(arena.ReplayPlaying, "Direct click-to-seek on event feed should pause for inspection");
                Assert.AreEqual(1, hud.ReplayHud.VisibleClickableEventRowCount, "At 2500ms only the first shot is in the visible feed");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void WowQuestSystem_MarkersDialogTrackerCompletionAndWayfinder_WorkAuthoritatively()
        {
            var rootGo = new UnityEngine.GameObject("WowQuestSystemTest");
            try
            {
                // 1. Parse RoomView ambient.quest_markers and spawn 3D floating '!' marker over Maren Halvorsen
                var roomJson = JsonNode.Parse(@"{
                    ""room"": { ""id"": ""#10"", ""name"": ""Port Watch Arrivals"" },
                    ""desc"": ""Arrivals gantry overlooking the dock ring."",
                    ""contents"": [
                        { ""id"": ""#42"", ""name"": ""Maren Halvorsen"" }
                    ],
                    ""exits"": [],
                    ""ambient"": {
                        ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c03_arrivals"" },
                        ""npc_ids"": [""#42""],
                        ""quest_markers"": [
                            {
                                ""npc_id"": ""#42"",
                                ""npc_name"": ""Maren Halvorsen"",
                                ""marker"": ""available"",
                                ""symbol"": ""!"",
                                ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                                ""quest_title"": ""First Berth: Enroll in the Belt Co-op"",
                                ""category"": ""guided"",
                                ""quest_count"": 1
                            }
                        ]
                    }
                }");
                var rv = MushProtocol.ParseRoomView(roomJson);
                Assert.IsNotNull(rv);
                Assert.AreEqual(1, rv.QuestMarkers.Count);
                Assert.AreEqual("available", rv.QuestMarkers[0].Marker);
                Assert.AreEqual("!", rv.QuestMarkers[0].Symbol);

                var stGo = new UnityEngine.GameObject("StationQuestMarkersTest");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");
                station.ApplyRoomView(rv, selfActorId: "#1");

                Assert.AreEqual(1, station.ActiveQuestMarkerCount, "NPC with available quest must display a 3D overhead '!' marker");
                var liveMarker = station.GetNpcQuestMarker("#42");
                Assert.IsNotNull(liveMarker);
                Assert.AreEqual("!", liveMarker.Symbol);
                var targetables = station.GetActiveRoomTargetables();
                Assert.IsTrue(targetables.Exists(w => w.actorId == "#42" && w.promptTitle.Contains("[!]")),
                    "NPC interaction prompt must reflect [!] Quest Available");

                // 2. Hydrate `tacsim.quests` panel with available, active, and completed quests
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var cmds = new List<string>();
                string wayfinderRoom = null;
                hud.QuestJournal.OnCommandRequested += cmds.Add;
                hud.QuestJournal.OnWayfinderRoomRequested += r => wayfinderRoom = r;

                var questsJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [
                        { ""npc_id"": ""#42"", ""npc_name"": ""Maren Halvorsen"", ""marker"": ""available"", ""symbol"": ""!"", ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""quest_title"": ""First Berth: Enroll in the Belt Co-op"", ""category"": ""guided"", ""quest_count"": 1 }
                    ],
                    ""available"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                            ""title"": ""First Berth: Enroll in the Belt Co-op"",
                            ""category"": ""guided"",
                            ""introduction"": ""Head down to the Co-op Desk and enroll for your first Dray."",
                            ""reward_summary"": ""Charter Dray hull + Co-op standing with Ilse"",
                            ""giver_id"": ""#42"",
                            ""giver_name"": ""Maren Halvorsen"",
                            ""turn_in_id"": ""#55"",
                            ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""allow_restart"": true,
                            ""accept_cmd"": ""+quest/accept #42=signedtok123"",
                            ""decline_cmd"": ""+quest/decline #42=signedtok123"",
                            ""checkpoints"": [
                                { ""id"": ""enroll"", ""title"": ""Enroll in Belt Co-op"", ""instructions"": ""Use +coop/enroll"", ""room_entry"": ""room.c05_registry"", ""room_name"": ""Co-op Registry"" }
                            ]
                        }
                    ],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""title"": ""Provisions and Reaction Mass"",
                            ""category"": ""guided"",
                            ""introduction"": ""Stock provisions and propellant before your first burn."",
                            ""reward_summary"": ""Fueled and provisioned for the Ceres-Phobos lane"",
                            ""giver_id"": ""#55"",
                            ""giver_name"": ""Ilse Oduya-Brandt"",
                            ""turn_in_id"": ""#55"",
                            ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""turn_in_here"": false,
                            ""allow_restart"": true,
                            ""ready_to_finish"": false,
                            ""next_step"": ""Buy 10t provisions"",
                            ""next_instructions"": ""Visit Co-op Chandlery and buy 10t provisions."",
                            ""next_room_entry"": ""room.c08_chandlery"",
                            ""next_room_name"": ""Co-op Chandlery"",
                            ""next_objective_ready"": false,
                            ""can_check_now"": false,
                            ""can_finish_now"": false,
                            ""check_cmd"": ""+quest/check #55=tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""finish_cmd"": ""+quest/finish #55=tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""abandon_cmd"": ""+quest/abandon #55=tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""checkpoints"": [
                                { ""id"": ""buy_provisions"", ""title"": ""Buy 10t provisions"", ""instructions"": ""Buy provisions"", ""room_entry"": ""room.c08_chandlery"", ""room_name"": ""Co-op Chandlery"", ""checked"": false, ""live_ready"": false },
                                { ""id"": ""buy_propellant"", ""title"": ""Buy 5t propellant"", ""instructions"": ""Buy propellant"", ""room_entry"": ""room.c07_exchange"", ""room_name"": ""Ceres Exchange"", ""checked"": true, ""live_ready"": true, ""detail_text"": ""Bought 5t propellant."" }
                            ]
                        }
                    ],
                    ""completed"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_00_done"",
                            ""title"": ""Arrived at Ceres Freeport"",
                            ""category"": ""guided"",
                            ""giver_name"": ""Maren Halvorsen"",
                            ""turn_in_name"": ""Maren Halvorsen"",
                            ""completion"": ""Cleared docking watch."",
                            ""reward_summary"": ""Port clearance recorded"",
                            ""completed_at_utc"": 1700000000
                        }
                    ]
                }");
                var panel = MushProtocol.ParseQuestsPanel(questsJson);
                Assert.IsNotNull(panel);
                Assert.AreEqual(1, panel.Available.Count);
                Assert.AreEqual(1, panel.Active.Count);
                Assert.AreEqual(1, panel.Completed.Count);

                hud.QuestJournal.ApplyQuestsPanel(panel);
                Assert.AreEqual(1, hud.QuestJournal.ActiveQuestCount);
                Assert.AreEqual(1, hud.QuestJournal.AvailableQuestCount);
                Assert.AreEqual(1, hud.QuestJournal.CompletedQuestCount);
                Assert.IsTrue(hud.QuestJournal.HasPinnedObjective, "Active server quest must auto-populate the HUD Quest Tracker");
                StringAssert.Contains("Provisions and Reaction Mass", hud.QuestJournal.PinnedTitle);
                Assert.IsTrue(hud.QuestJournal.HasQuestForNpc("#42"));

                // 3. Open WoW NPC Quest Dialog for Maren (#42) and 1-click Accept sends signed `+quest/accept`
                Assert.IsTrue(hud.QuestJournal.OpenNpcQuestDialog("#42"));
                Assert.IsTrue(hud.QuestJournal.IsQuestDialogOpen);
                Assert.IsTrue(hud.IsModalOpen);
                hud.QuestJournal.TriggerPrimaryDialogAction();
                Assert.AreEqual(1, cmds.Count);
                Assert.AreEqual("+quest/accept #42=signedtok123", cmds[0]);
                Assert.IsFalse(hud.QuestJournal.IsQuestDialogOpen, "Accepting an offer closes the NPC quest dialog");

                // 4. Open Quest Dialog for active quest: 2-click Abandon confirmation + ESC close + Wayfinder
                Assert.IsTrue(hud.QuestJournal.OpenQuestDialogByKey("tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"));
                Assert.IsTrue(hud.QuestJournal.IsQuestDialogOpen);
                hud.QuestJournal.TriggerDialogWayfinder();
                Assert.AreEqual("room.c08_chandlery", wayfinderRoom);

                hud.QuestJournal.AbandonDialogQuest(); // 1st click arms confirmation
                Assert.AreEqual(1, cmds.Count, "First abandon click must require confirmation");
                hud.QuestJournal.AbandonDialogQuest(); // 2nd click submits server-verified `+quest/abandon`
                Assert.AreEqual(2, cmds.Count);
                Assert.AreEqual("+quest/abandon #55=tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", cmds[1]);

                // 5. ESC closes Quest Dialog cleanly via HUD TryCloseTopmostModal
                Assert.IsTrue(hud.QuestJournal.OpenNpcQuestDialog("#42"));
                Assert.IsTrue(hud.TryCloseTopmostModal());
                Assert.IsFalse(hud.QuestJournal.IsQuestDialogOpen);

                // 6. Clearing markers removes 3D overhead marker cleanly
                station.ApplyQuestMarkers(System.Array.Empty<MushNpcQuestMarker>());
                Assert.AreEqual(0, station.ActiveQuestMarkerCount);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestMinimapBeacon_MilestonesAndCommandPalette_WorkEndToEnd()
        {
            var rootGo = new UnityEngine.GameObject("QuestPolishWave20TestRoot");
            try
            {
                var stGo = new UnityEngine.GameObject("StationMinimapQuestTest");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Minimap.BindWorldSources(station, null, null, null, null);
                hud.CommandPalette.BindWorldSources(station, null, null);
                hud.CommandPalette.BindQuestJournal(hud.QuestJournal);

                // Pick a real same-deck room and a cross-deck room from the loaded Ceres Freeport atlas
                Assert.IsTrue(station.Pack.Rooms.TryGetValue("room.c03_arrivals", out var arrivals));
                string sameFloorTarget = null;
                string otherFloorTarget = null;
                int otherFloorDelta = 0;
                foreach (var r in station.Pack.Rooms.Values)
                {
                    if (r.EntryId == arrivals.EntryId) continue;
                    if (sameFloorTarget == null && r.FloorIndex == arrivals.FloorIndex)
                        sameFloorTarget = r.EntryId;
                    if (otherFloorTarget == null && r.FloorIndex != arrivals.FloorIndex)
                    {
                        otherFloorTarget = r.EntryId;
                        otherFloorDelta = r.FloorIndex - arrivals.FloorIndex;
                    }
                }
                Assert.IsNotNull(sameFloorTarget, "Ceres Freeport atlas must have another room on arrivals deck");

                // 1. Initial hydration must NOT fire milestone celebration events
                var milestones = new List<(string kind, string title)>();
                hud.QuestJournal.OnQuestMilestoneReached += (k, t) => milestones.Add((k, t));

                var initialJson = JsonNode.Parse($@"{{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [
                        {{ ""npc_id"": ""#42"", ""npc_name"": ""Maren Halvorsen"", ""marker"": ""available"", ""symbol"": ""!"", ""quest_key"": ""q.enroll"", ""quest_title"": ""First Berth"", ""category"": ""guided"", ""quest_count"": 1 }}
                    ],
                    ""available"": [
                        {{
                            ""key"": ""q.enroll"",
                            ""title"": ""First Berth"",
                            ""category"": ""guided"",
                            ""introduction"": ""Enroll at the Co-op Desk."",
                            ""reward_summary"": ""Charter Dray"",
                            ""giver_id"": ""#42"",
                            ""giver_name"": ""Maren Halvorsen"",
                            ""turn_in_id"": ""#55"",
                            ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""accept_cmd"": ""+quest/accept #42=tok"",
                            ""checkpoints"": [
                                {{ ""id"": ""cp1"", ""title"": ""Enroll"", ""instructions"": ""Use +coop/enroll"", ""room_entry"": ""{sameFloorTarget}"", ""room_name"": ""Target Room"" }}
                            ]
                        }}
                    ],
                    ""active"": [],
                    ""completed"": []
                }}");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(initialJson));
                Assert.AreEqual(0, milestones.Count, "Initial panel hydration must not fire milestone callouts");

                // 2. Accepting a quest fires "accepted" and drives same-deck minimap gold beacon
                var activeJson = JsonNode.Parse($@"{{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [
                        {{
                            ""key"": ""q.enroll"",
                            ""title"": ""First Berth"",
                            ""category"": ""guided"",
                            ""introduction"": ""Enroll at the Co-op Desk."",
                            ""reward_summary"": ""Charter Dray"",
                            ""giver_id"": ""#42"",
                            ""giver_name"": ""Maren Halvorsen"",
                            ""turn_in_id"": ""#55"",
                            ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""ready_to_finish"": false,
                            ""can_finish_now"": false,
                            ""next_step"": ""Enroll in Belt Co-op"",
                            ""next_room_entry"": ""{sameFloorTarget}"",
                            ""next_room_name"": ""Co-op Registry"",
                            ""checkpoints"": [
                                {{ ""id"": ""cp1"", ""title"": ""Enroll"", ""instructions"": ""Use +coop/enroll"", ""room_entry"": ""{sameFloorTarget}"", ""room_name"": ""Co-op Registry"", ""checked"": false, ""live_ready"": false }}
                            ]
                        }}
                    ],
                    ""completed"": []
                }}");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(activeJson));
                Assert.AreEqual(1, milestones.Count);
                Assert.AreEqual("accepted", milestones[0].kind);
                Assert.AreEqual("First Berth", milestones[0].title);

                Assert.IsTrue(hud.QuestJournal.TryGetMinimapObjective(out string objRoom, out bool objTurnIn, out bool hasOffer));
                Assert.AreEqual(sameFloorTarget, objRoom);
                Assert.IsFalse(objTurnIn);
                Assert.IsFalse(hasOffer);

                hud.Minimap.SetQuestObjectiveRoom(objRoom, objTurnIn, hasOffer);
                hud.Minimap.SyncMinimap(TacSimActiveStage.StationWalk, station.GetRoomSpawnPosition("room.c03_arrivals"), 0f, false, forceRedraw: true);
                Assert.IsTrue(hud.Minimap.LastStationMinimapHasQuestBeacon, "Same-deck quest objective room must paint a gold beacon on the Station Minimap");
                Assert.AreEqual(0, hud.Minimap.LastStationMinimapQuestFloorDelta);

                // Verify actual golden beacon pixels exist in the minimap buffer
                bool foundGoldPixel = false;
                for (int y = 0; y < 196 && !foundGoldPixel; y++)
                {
                    for (int x = 0; x < 196; x++)
                    {
                        var px = hud.Minimap.SampleMinimapPixelForTests(x, y);
                        if (px.r >= 245 && px.g >= 200 && px.b <= 90)
                        {
                            foundGoldPixel = true;
                            break;
                        }
                    }
                }
                Assert.IsTrue(foundGoldPixel, "Station minimap must render visible golden quest beacon pixels");

                // 3. Cross-deck quest objective updates header with elevation arrow and floor delta
                if (!string.IsNullOrEmpty(otherFloorTarget))
                {
                    hud.Minimap.SetQuestObjectiveRoom(otherFloorTarget, isTurnIn: true, currentRoomHasOffer: false);
                    hud.Minimap.SyncMinimap(TacSimActiveStage.StationWalk, station.GetRoomSpawnPosition("room.c03_arrivals"), 0f, false, forceRedraw: true);
                    Assert.IsFalse(hud.Minimap.LastStationMinimapHasQuestBeacon);
                    Assert.AreEqual(otherFloorDelta, hud.Minimap.LastStationMinimapQuestFloorDelta);
                    StringAssert.Contains(otherFloorDelta > 0 ? "▲" : "▼", hud.Minimap.HeaderText);
                    StringAssert.Contains("?", hud.Minimap.HeaderText);
                }

                // 4. Transition to ready-to-turn-in fires "ready", then completion fires "completed"
                var readyJson = JsonNode.Parse($@"{{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [
                        {{
                            ""key"": ""q.enroll"",
                            ""title"": ""First Berth"",
                            ""category"": ""guided"",
                            ""introduction"": ""Enroll at the Co-op Desk."",
                            ""reward_summary"": ""Charter Dray"",
                            ""giver_id"": ""#42"",
                            ""giver_name"": ""Maren Halvorsen"",
                            ""turn_in_id"": ""#55"",
                            ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""ready_to_finish"": true,
                            ""can_finish_now"": true,
                            ""next_step"": ""Return to Ilse Oduya-Brandt"",
                            ""next_room_entry"": ""{sameFloorTarget}"",
                            ""next_room_name"": ""Co-op Registry"",
                            ""checkpoints"": [
                                {{ ""id"": ""cp1"", ""title"": ""Enroll"", ""instructions"": ""Done"", ""room_entry"": ""{sameFloorTarget}"", ""room_name"": ""Co-op Registry"", ""checked"": true, ""live_ready"": true }}
                            ]
                        }}
                    ],
                    ""completed"": []
                }}");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(readyJson));
                Assert.AreEqual(2, milestones.Count);
                Assert.AreEqual("ready", milestones[1].kind);

                // 5. Command Palette exposes live Quest Dialog + Quest Wayfinder entries and executes them
                string plottedRoom = null;
                hud.CommandPalette.OnPlotStationRoomRequested += r => plottedRoom = r;
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("wayfinder quest objective");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0, "Command palette must list live Quest Wayfinder entry");
                StringAssert.Contains("Co-op Registry", hud.CommandPalette.SelectedEntryTitle);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual(sameFloorTarget, plottedRoom, "Executing Quest Wayfinder palette entry must plot station wayfinder route");

                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("complete quest first berth");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0, "Command palette must list active Quest Dialog entry");
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.IsTrue(hud.QuestJournal.IsQuestDialogOpen, "Executing Quest Dialog palette entry must open NPC Quest Dialog modal");

                var doneJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [],
                    ""completed"": [
                        { ""key"": ""q.enroll"", ""title"": ""First Berth"", ""category"": ""guided"", ""giver_name"": ""Maren Halvorsen"", ""turn_in_name"": ""Ilse Oduya-Brandt"", ""completion"": ""Welcome aboard."", ""reward_summary"": ""Charter Dray"", ""completed_at_utc"": 1700000001 }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(doneJson));
                Assert.AreEqual(3, milestones.Count);
                Assert.AreEqual("completed", milestones[2].kind);
                Assert.AreEqual("First Berth", milestones[2].title);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestTradeExchangeGuidance_AndSurfaceMiningChecklist_HighlightObjectivesEndToEnd()
        {
            var rootGo = new UnityEngine.GameObject("QuestTradeAndSurfaceGuidanceTestRoot");
            try
            {
                var tradeState = new TacSimTradeAndShipState();
                var surfaceGo = new UnityEngine.GameObject("SurfaceQuestStage");
                surfaceGo.transform.SetParent(rootGo.transform, false);
                var surface = surfaceGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surface.BuildSurfaceStage(null, "occator_rim");

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.BindTradeState(tradeState);
                hud.Overlays.BindControllers(surface, null, null);

                // Populate exchange book with live port.trade schema: provisions (ASK), propellant (ASK), and carbon_ore (BID)
                tradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""balance_text"": ""5,000 cr"",
                    ""listings"": [
                        { ""id"": 101, ""side"": ""ask"", ""good"": ""tacsim:provisions"", ""good_label"": ""Provisions"", ""available_qty"": 30, ""unit_price_minor"": 1200, ""price_text"": ""12.00 cr"", ""funded"": true, ""vendor_name"": ""Co-op Chandlery"" },
                        { ""id"": 102, ""side"": ""ask"", ""good"": ""tacsim:propellant"", ""good_label"": ""Propellant"", ""available_qty"": 40, ""unit_price_minor"": 900, ""price_text"": ""9.00 cr"", ""funded"": true, ""vendor_name"": ""Ceres Exchange"" },
                        { ""id"": 103, ""side"": ""bid"", ""good"": ""tacsim:carbon_ore"", ""good_label"": ""Carbon Ore"", ""available_qty"": 20, ""unit_price_minor"": 4500, ""price_text"": ""45.00 cr"", ""funded"": true, ""vendor_name"": ""Kasei Receiving"" }
                    ]
                }"));

                // 1. Activate Step 2 quest (Buy 10 provisions -> Buy 5 propellant)
                var step2Json = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#20"",
                    ""room_name"": ""Co-op Chandlery"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""title"": ""Stock the Locker and Tanks"",
                            ""category"": ""guided"",
                            ""giver_id"": ""#55"", ""giver_name"": ""Ilse"", ""turn_in_id"": ""#55"", ""turn_in_name"": ""Ilse"",
                            ""next_step"": ""Buy at least 10 provisions at Ceres Co-op Chandlery"",
                            ""checkpoints"": [
                                { ""id"": ""buy_provisions"", ""title"": ""Buy at least 10 provisions at Ceres Co-op Chandlery"", ""instructions"": ""Request quote for 10 provisions."", ""checked"": false, ""live_ready"": false },
                                { ""id"": ""buy_propellant"", ""title"": ""Buy at least 5 tonnes of propellant at Ceres Exchange Hall"", ""instructions"": ""Request quote for 5 propellant."", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ],
                    ""completed"": []
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(step2Json));

                Assert.IsTrue(hud.QuestJournal.TryGetActiveTradeNeed(out var tradeNeed));
                Assert.AreEqual("provisions", tradeNeed.CommoditySlug);
                Assert.AreEqual(10, tradeNeed.TargetQuantity);
                Assert.IsTrue(tradeNeed.WantsBuy);

                hud.OpenTradeAndSailingModal("CO-OP CHANDLERY", openSailingTab: false);
                Assert.IsTrue(hud.IsExchangeQuestBannerVisible, "Exchange must display the active Quest Trade Need banner");
                StringAssert.Contains("PROVISIONS", hud.ExchangeQuestBannerText);
                Assert.AreEqual(1, hud.HighlightedQuestListingCount, "Exactly the matching provisions ASK row must be highlighted as [QUEST]");

                // 1-click SET QUEST QTY aligns selected trade lot to 10
                tradeState.SelectedTradeQuantity = 25;
                tradeState.SelectedSellAll = true;
                Assert.IsTrue(hud.ApplyQuestTradeQuantity());
                Assert.IsFalse(tradeState.SelectedSellAll);
                Assert.AreEqual(10, tradeState.SelectedTradeQuantity);

                // 2. Activate Step 7 quest (Sell 1t carbon_ore) -> highlights matching BID row and sets qty = 1
                var step7Json = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#30"",
                    ""room_name"": ""Service Counter"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore"",
                            ""title"": ""Deliver Your Mined Ore to Market"",
                            ""category"": ""guided"",
                            ""giver_id"": ""#60"", ""giver_name"": ""Rosalind"", ""turn_in_id"": ""#70"", ""turn_in_name"": ""Wren"",
                            ""next_step"": ""Sell at least 1 tonne of carbon ore at Stickney Yards"",
                            ""checkpoints"": [
                                { ""id"": ""sell_carbon_ore"", ""title"": ""Sell at least 1 tonne of carbon ore at Stickney Yards"", ""instructions"": ""Run +market/sell for carbon_ore."", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ],
                    ""completed"": []
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(step7Json));
                Assert.AreEqual(1, hud.HighlightedQuestListingCount, "Carbon ore BID listing must be highlighted for Step 7");
                Assert.IsTrue(hud.ApplyQuestTradeQuantity());
                Assert.AreEqual(1, tradeState.SelectedTradeQuantity);

                // 3. Activate Step 6 surface mining quest (Scan done, Sound pending, Mine carbon_ore pending)
                var step6Json = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#22"",
                    ""room_name"": ""Occator Rim"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                            ""title"": ""Scan, Sound, and Mine a Landing Site"",
                            ""category"": ""guided"",
                            ""giver_id"": ""#60"", ""giver_name"": ""Rosalind"", ""turn_in_id"": ""#60"", ""turn_in_name"": ""Rosalind"",
                            ""next_step"": ""Sound a detected mineral pocket on the surface"",
                            ""checkpoints"": [
                                { ""id"": ""scan_landing_site"", ""title"": ""Complete a surface scan at a Ceres landing site"", ""instructions"": ""Run +tv/1 scan go."", ""checked"": true, ""live_ready"": true },
                                { ""id"": ""sound_mineral_pocket"", ""title"": ""Sound a detected mineral pocket on the surface"", ""instructions"": ""Run +tv/1 sound go."", ""checked"": false, ""live_ready"": false },
                                { ""id"": ""mine_carbon_ore"", ""title"": ""Mine at least 1 tonne of carbon ore on the surface"", ""instructions"": ""Mine carbon_ore."", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ],
                    ""completed"": []
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(step6Json));

                // Surface mining quest must NOT show a bogus exchange trade banner
                Assert.IsFalse(hud.IsExchangeQuestBannerVisible);

                // Ground Chart modal displays the guided mining checklist banner and 1-click switches to carbon wash
                Assert.IsTrue(hud.GroundChart.HasQuestMiningBanner, "Ground Chart must show the Quest Mining Banner for Step 6");
                StringAssert.Contains("[✓]", hud.GroundChart.QuestMiningBannerText);
                StringAssert.Contains("carbon_ore", hud.GroundChart.QuestMiningBannerText);
                string requestedFamily = null;
                hud.GroundChart.OnRefreshGroundChartRequested += f => requestedFamily = f;
                Assert.IsTrue(hud.GroundChart.ApplyQuestWashFilter());
                Assert.AreEqual("carbon", hud.GroundChart.ActiveFamily);
                Assert.AreEqual("carbon", requestedFamily);

                // Surface Quarry HUD displays the guided mining checklist when excursion is active
                surface.EnterSurfaceMode();
                hud.Overlays.RefreshSurfaceHud();
                Assert.IsTrue(hud.Overlays.HasSurfaceQuestChecklist, "Surface Quarry HUD must display guided mining checklist");
                StringAssert.Contains("Scan, Sound, and Mine", hud.Overlays.SurfaceReadingsTextForTests);
                StringAssert.Contains("carbon_ore", hud.Overlays.SurfaceReadingsTextForTests);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestSolarOrrery_SailingPlanAndOpsDirectory_HighlightObjectivesEndToEnd()
        {
            var rootGo = new UnityEngine.GameObject("QuestOrreryAndOpsDirectoryTestRoot");
            try
            {
                var tradeState = new TacSimTradeAndShipState();
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var stationGo = new UnityEngine.GameObject("TestStationForOpsQuest");
                stationGo.transform.SetParent(rootGo.transform, false);
                var station = stationGo.AddComponent<StationWorldController>();
                station.BuildStation(catalog, "ceres_freeport");

                // Pick a real reachable destination room from the station pack for directory objective highlighting
                string targetRoomEntry = string.Empty;
                foreach (var kv in station.Pack.Rooms)
                {
                    if (!string.Equals(kv.Key, station.ActiveRoomEntryId, System.StringComparison.OrdinalIgnoreCase) &&
                        station.FindExitPath(station.ActiveRoomEntryId, kv.Key).Count > 0)
                    {
                        targetRoomEntry = kv.Key;
                        break;
                    }
                }
                Assert.IsFalse(string.IsNullOrEmpty(targetRoomEntry), "Station pack must expose at least one reachable room");

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.BindTradeState(tradeState);
                hud.OpsDirectory.BindSources(station, null, null, tradeState, null, null, null);

                // Populate Solar System Orrery with live sky.chart frame (Ceres -> Stickney route on Mars)
                var chart = TacSimSolarChartFrame.FromJson(JsonNode.Parse(@"{
                    ""ic_time"": ""2350-05-01"",
                    ""vantage"": { ""body"": ""ceres"", ""port"": ""ceres"" },
                    ""scale"": { ""r0_au"": 1.5, ""au_max"": 12.0 },
                    ""bodies"": [
                        { ""id"": ""mars"", ""label"": ""Mars"", ""r_au"": 1.52, ""lon_deg"": 110.0, ""lat_deg"": 1.8, ""here"": false },
                        { ""id"": ""ceres"", ""label"": ""Ceres"", ""r_au"": 2.77, ""lon_deg"": 230.0, ""lat_deg"": 10.6, ""here"": true }
                    ],
                    ""routes"": [
                        { ""to"": ""stickney"", ""body"": ""mars"", ""loaded"": { ""total_seconds"": 180000, ""burn_tonnes"": 12.5 } }
                    ]
                }"));
                hud.SolarMap.ApplyChartFrame(chart);

                // Populate RoomView with Maren Halvorsen in Port Watch Arrivals
                var rv = new MushRoomView
                {
                    Id = "#10",
                    Name = "Port Watch Arrivals",
                    Atlas = new AtlasRoomAmbient { Pack = "neighborhood.ceres_freeport", Entry = station.ActiveRoomEntryId }
                };
                rv.Contents.Add(new MushObjRef { Id = "#42", Name = "Maren Halvorsen" });
                rv.NpcIds.Add("#42");
                hud.OpsDirectory.UpdateLiveRoom(rv, TacSimActiveStage.StationWalk);

                // Feed active Step 3 quest targeting Stickney Yards + room objective + Maren available marker
                var panelJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [
                        {
                            ""npc_id"": ""#42"",
                            ""npc_name"": ""Maren Halvorsen"",
                            ""marker"": ""available"",
                            ""symbol"": ""!"",
                            ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                            ""quest_title"": ""First Papers at the Registry"",
                            ""category"": ""guided""
                        }
                    ],
                    ""available"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                            ""title"": ""First Papers at the Registry"",
                            ""category"": ""guided"",
                            ""giver_id"": ""#42"", ""giver_name"": ""Maren Halvorsen"",
                            ""turn_in_id"": ""#55"", ""turn_in_name"": ""Ilse"",
                            ""accept_cmd"": ""+quest/accept #42=tok1""
                        }
                    ],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_deliver_metals"",
                            ""title"": ""Deliver Metals to Stickney"",
                            ""category"": ""guided"",
                            ""giver_id"": ""#55"", ""giver_name"": ""Ilse"", ""turn_in_id"": ""#70"", ""turn_in_name"": ""Wren"",
                            ""next_step"": ""Plot passage to Stickney Yards (+ship/plan stickney)"",
                            ""next_room_entry"": """ + targetRoomEntry + @""",
                            ""checkpoints"": [
                                {
                                    ""id"": ""sail_stickney"",
                                    ""title"": ""Deliver 5 metals to Stickney Yards"",
                                    ""instructions"": ""Run +ship/plan stickney and +ship/order depart stickney."",
                                    ""room_entry"": """ + targetRoomEntry + @""",
                                    ""checked"": false,
                                    ""live_ready"": false
                                }
                            ]
                        }
                    ],
                    ""completed"": []
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(panelJson));

                // 1. QuestDestinationHint resolves stickney
                Assert.IsTrue(hud.QuestJournal.TryGetActiveDestinationPort(out var destHint));
                Assert.AreEqual("stickney", destHint.PortId);
                Assert.AreEqual("Stickney Yards", destHint.PortLabel);

                // 2. Solar System Orrery highlights Mars/Stickney with golden objective ring & 1-click focus
                Assert.AreEqual("stickney", hud.SolarMap.QuestDestinationPortId);
                Assert.IsTrue(hud.SolarMap.LastOrreryHasQuestDestinationRing, "Solar Orrery must draw golden quest objective ring around destination body");
                string solarCmd = null;
                hud.SolarMap.OnCommandRequested += c => solarCmd = c;
                Assert.IsTrue(hud.SolarMap.FocusQuestDestinationPort());
                Assert.AreEqual("stickney", hud.SolarMap.SelectedPortId);
                hud.SolarMap.DispatchPlotCourse();
                Assert.AreEqual("+ship/plan stickney", solarCmd);

                // 3. Sailing Plan tab shows golden destination banner, highlights Stickney button, and 1-click selects port
                tradeState.SelectedDestinationPort = "ceres";
                hud.OpenTradeAndSailingModal("SAILING PLAN", openSailingTab: true);
                Assert.IsTrue(hud.IsSailingQuestBannerVisible, "Sailing Plan must show golden Quest Destination banner");
                StringAssert.Contains("STICKNEY YARDS", hud.SailingQuestBannerText);
                Assert.AreEqual(TacSim.UI.TacSimHudController.CorridorDestinationIndex("stickney"), hud.HighlightedQuestPortIndex);
                Assert.IsTrue(hud.ApplyQuestDestinationPort());
                Assert.AreEqual("stickney", tradeState.SelectedDestinationPort);

                // 4. [N] Ops Directory highlights active quest station room & quest-marked NPC -> opens Quest Dialog directly
                string openedNpcDialogId = null;
                hud.OpsDirectory.OnOpenNpcQuestDialogRequested += id =>
                {
                    openedNpcDialogId = id;
                    hud.QuestJournal.OpenNpcQuestDialog(id);
                };
                hud.OpsDirectory.SetOpen(true);
                Assert.GreaterOrEqual(hud.OpsDirectory.HighlightedQuestDirectoryRowIndex, 0, "Deck Directory must highlight the active quest objective room");
                Assert.AreEqual(1, hud.OpsDirectory.HighlightedQuestPresenceRowCount, "Room Presence must badge Maren as a quest-marked NPC");
                Assert.IsTrue(hud.OpsDirectory.TriggerPresenceTalkButton(0));
                Assert.AreEqual("#42", openedNpcDialogId);
                Assert.IsTrue(hud.QuestJournal.IsQuestDialogOpen, "Clicking QUEST on a quest-marked NPC in Ops Directory must open the Quest Dialog");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestWayfinderGoldBeacon_AndCharacterSheetSummary_ReflectActiveQuestProgress()
        {
            var rootGo = new UnityEngine.GameObject("QuestWayfinderAndSheetTestRoot");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var stationGo = new UnityEngine.GameObject("StationForGoldWayfinder");
                stationGo.transform.SetParent(rootGo.transform, false);
                var station = stationGo.AddComponent<StationWorldController>();
                station.BuildStation(catalog, "ceres_freeport");

                string targetRoomEntry = string.Empty;
                foreach (var kv in station.Pack.Rooms)
                {
                    if (!string.Equals(kv.Key, station.ActiveRoomEntryId, System.StringComparison.OrdinalIgnoreCase) &&
                        station.FindExitPath(station.ActiveRoomEntryId, kv.Key).Count > 0)
                    {
                        targetRoomEntry = kv.Key;
                        break;
                    }
                }
                Assert.IsFalse(string.IsNullOrEmpty(targetRoomEntry));

                var wayfinderGo = new UnityEngine.GameObject("TestWayfinder3D");
                wayfinderGo.transform.SetParent(rootGo.transform, false);
                var wayfinder = wayfinderGo.AddComponent<TacSim.World.TacSimWayfinder3D>();
                wayfinder.EnsureVisuals();

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.OpsDirectory.BindSources(station, null, wayfinder, null, null, null, null);

                // 1. Quest wayfinder route switches ribbon & beacon to golden quest palette and prefixes compass banner
                UnityEngine.Vector3 startPos = station.GetRoomSpawnPosition(station.ActiveRoomEntryId);
                Assert.IsTrue(wayfinder.PlotStationRoute(station, station.ActiveRoomEntryId, targetRoomEntry, startPos, isQuestObjective: true));
                Assert.IsTrue(wayfinder.IsQuestTrail, "Quest wayfinder route must flag IsQuestTrail");
                Assert.Greater(wayfinder.ActiveTrailColor.r, 0.9f, "Quest wayfinder ribbon must use gold palette");
                Assert.Less(wayfinder.ActiveTrailColor.b, 0.4f, "Quest wayfinder ribbon must not be cyan");

                hud.OpsDirectory.SyncWayfinderBanner(TacSimActiveStage.StationWalk);
                Assert.IsTrue(hud.OpsDirectory.IsWayfinderBannerVisible);
                StringAssert.StartsWith("[QUEST]", hud.OpsDirectory.WayfinderBannerText);

                // Standard non-quest route restores cyan trail & standard banner
                Assert.IsTrue(wayfinder.PlotStationRoute(station, station.ActiveRoomEntryId, targetRoomEntry, startPos, isQuestObjective: false));
                Assert.IsFalse(wayfinder.IsQuestTrail);
                Assert.Greater(wayfinder.ActiveTrailColor.b, 0.8f, "Standard wayfinder ribbon must restore cyan palette");
                hud.OpsDirectory.SyncWayfinderBanner(TacSimActiveStage.StationWalk);
                StringAssert.DoesNotContain("[QUEST]", hud.OpsDirectory.WayfinderBannerText);

                // 2. [C] Character Sheet Pilot Summary reflects tracked active quest progress & completed count
                var questJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Ceres Concourse"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""title"": ""Stock the Locker and Tanks"",
                            ""category"": ""guided"",
                            ""next_step"": ""Buy at least 5 propellant at Ceres Exchange Hall"",
                            ""checkpoints"": [
                                { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""checked"": true, ""live_ready"": true },
                                { ""id"": ""buy_propellant"", ""title"": ""Buy 5 propellant"", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""First Papers at the Registry"" }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(questJson));
                hud.CharacterSheet.RefreshContents();

                StringAssert.Contains("ACTIVE QUEST", hud.CharacterSheet.PilotSummarySnapshot);
                StringAssert.Contains("Stock the Locker and Tanks", hud.CharacterSheet.PilotSummarySnapshot);
                StringAssert.Contains("(1/2)", hud.CharacterSheet.PilotSummarySnapshot);
                StringAssert.Contains("Completed: 1", hud.CharacterSheet.PilotSummarySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestBridgeHoloRing_TrackerNavButtonAndActiveOpsTab_SyncEndToEnd()
        {
            var rootGo = new UnityEngine.GameObject("QuestBridgeHoloTrackerOpsTestRoot");
            try
            {
                var catalog = TacSimAssetCatalog.LoadOrDefault();
                var shipGo = new UnityEngine.GameObject("TestBridgeHoloQuestShip");
                shipGo.transform.SetParent(rootGo.transform, false);
                var ship = shipGo.AddComponent<ShipInteriorController>();
                ship.BuildInterior(catalog, "tacsim.unit.dray");

                // 1. Solar System bridge holo-table highlights quest destination body with golden HoloQuestRing
                //    while preserving exact HoloTankContacts & HoloSolarOrbitRingCount invariants!
                var chart = TacSimSolarChartFrame.FromJson(JsonNode.Parse(@"{
                    ""ic_time"": ""2350-05-01"",
                    ""vantage"": { ""body"": ""ceres"", ""port"": ""ceres"" },
                    ""scale"": { ""r0_au"": 1.5, ""au_max"": 12.0 },
                    ""bodies"": [
                        { ""id"": ""earth"", ""label"": ""Earth"", ""r_au"": 1.0, ""lon_deg"": 15.0, ""lat_deg"": 0.0, ""here"": false },
                        { ""id"": ""mars"", ""label"": ""Mars"", ""r_au"": 1.52, ""lon_deg"": 110.0, ""lat_deg"": 1.8, ""here"": false },
                        { ""id"": ""phobos"", ""label"": ""Phobos"", ""parent"": ""mars"", ""r_au"": 1.52, ""lon_deg"": 110.0, ""lat_deg"": 1.8, ""here"": false },
                        { ""id"": ""ceres"", ""label"": ""Ceres"", ""r_au"": 2.77, ""lon_deg"": 230.0, ""lat_deg"": 10.6, ""here"": true }
                    ],
                    ""routes"": [
                        { ""to"": ""stickney"", ""venue"": ""Stickney Yards"", ""body"": ""phobos"", ""legs"": [] }
                    ]
                }"));
                ship.ApplySolarChartFrame(chart);
                ship.SetHoloDisplayMode(BridgeHoloMode.SolarSystem);
                Assert.IsFalse(ship.HasHoloQuestDestinationRing, "No quest ring before quest destination is set");
                Assert.AreEqual(3, ship.HoloSolarOrbitRingCount);

                ship.SetQuestDestinationPort("stickney");
                Assert.AreEqual("stickney", ship.QuestDestinationPortId);
                Assert.IsTrue(ship.HasHoloQuestDestinationRing, "Stickney (Phobos -> Mars) must mount golden HoloQuestRing on Mars sphere");
                Assert.AreEqual(3, ship.HoloSolarOrbitRingCount, "HoloSolarOrbitRingCount invariant must stay 3");
                var holoContacts = shipGo.transform.Find("HoloTankPedestal")?.Find("HoloTankContacts");
                Assert.IsNotNull(holoContacts);
                Assert.AreEqual(4, holoContacts.childCount, "HoloTankContacts childCount invariant must stay 4 (Sun + 3 primary bodies)");

                ship.SetQuestDestinationPort(string.Empty);
                Assert.IsFalse(ship.HasHoloQuestDestinationRing, "Clearing quest destination must remove golden HoloQuestRing");

                // 2. Quest Tracker Card NAV button + [M] Ops Directory Active Ops quest summary
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                string requestedRoom = string.Empty;
                hud.QuestJournal.OnWayfinderRoomRequested += r => requestedRoom = r;

                var questJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""title"": ""Stock the Locker and Tanks"",
                            ""category"": ""guided"",
                            ""next_room_entry"": ""room.c04_coop_registry"",
                            ""next_step"": ""Buy 5 propellant at Ceres Exchange Hall"",
                            ""checkpoints"": [
                                { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""checked"": true, ""live_ready"": true },
                                { ""id"": ""buy_propellant"", ""title"": ""Buy 5 propellant"", ""room_entry"": ""room.c04_coop_registry"", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""First Papers at the Registry"" }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(questJson));

                Assert.IsTrue(hud.QuestJournal.IsTrackerWayfinderVisible, "Tracker card must expose 1-click NAV button when active quest has next_room_entry");
                Assert.IsTrue(hud.QuestJournal.TriggerTrackerWayfinder());
                Assert.AreEqual("room.c04_coop_registry", requestedRoom);

                hud.OpsDirectory.SelectTab(TacSim.UI.OpsDirectoryTab.ActiveOps);
                StringAssert.Contains("ACTIVE GUIDED QUEST", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("Stock the Locker and Tanks", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("(1/2)", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("Completed: 1", hud.OpsDirectory.OpsSummarySnapshot);

                // Manual pin hides structured quest NAV button
                hud.QuestJournal.PinObjective("CUSTOM", "Manual Pin", "Check dock manifest");
                Assert.IsFalse(hud.QuestJournal.IsTrackerWayfinderVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestSurfaceOreBeacon_AndTacticalBrackets_HighlightMatchingPocketInGold()
        {
            var rootGo = new UnityEngine.GameObject("QuestSurfaceOreBeaconTestRoot");
            try
            {
                var surfGo = new UnityEngine.GameObject("SurfaceQuestOreTest");
                surfGo.transform.SetParent(rootGo.transform, false);
                var surf = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();
                surf.BuildSurfaceStage(null, "occator_rim");
                surf.EnterSurfaceMode();

                surf.ApplySiteMapFrame(JsonNode.Parse(@"{
                    ""width"": 8,
                    ""height"": 8,
                    ""site"": {
                        ""site"": ""Occator Rim"",
                        ""family"": ""volatiles"",
                        ""grid"": { ""x0"": 0, ""y0"": 0, ""span"": 1, ""cell_cm"": 1000 },
                        ""rigs"": [{ ""slot"": 1, ""cell"": [1, 1] }],
                        ""readings"": [
                            { ""cell"": [3, 3], ""text"": ""B1: rich carbon_ore pocket"" },
                            { ""cell"": [5, 5], ""text"": ""C2: trace silicates"" }
                        ]
                    },
                    ""rows"": [""........"",""........"",""........"",""........"",""........"",""........"",""........"",""........""],
                    ""you"": [1, 1]
                }"));

                Assert.AreEqual(2, surf.OreBeacons.Count);
                Assert.AreEqual(2, surf.ActiveBeaconHaloCount);
                Assert.AreEqual(0, surf.QuestMatchedBeaconCount);
                Assert.IsFalse(surf.OreBeacons[0].IsQuestTarget);

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.TacticalBrackets.BindControllers(null, surf, null);
                hud.TacticalBrackets.ScreenProjectionOverrideForTests = _ => new UnityEngine.Vector3(960f, 540f, 15f);

                // Activating surface mining quest target highlights matching carbon_ore pocket in gold
                surf.SetQuestMiningTarget("carbon_ore", "carbon");
                Assert.AreEqual(1, surf.QuestMatchedBeaconCount, "Exactly the matching carbon_ore reading must flag as quest beacon");
                Assert.AreEqual(2, surf.ActiveBeaconHaloCount, "Beacon count must remain invariant");
                Assert.IsTrue(surf.OreBeacons[0].IsQuestTarget);
                Assert.IsFalse(surf.OreBeacons[1].IsQuestTarget);

                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.PlanetarySurface, null, hideForOverlay: false);
                Assert.AreEqual(1, hud.TacticalBrackets.QuestOreBracketCount);
                bool foundQuestOreTag = false;
                for (int bi = 0; bi < hud.TacticalBrackets.ActiveBracketCount; bi++)
                {
                    if (hud.TacticalBrackets.GetActiveBracketTagSnapshot(bi).Contains("[QUEST] ORE", System.StringComparison.Ordinal))
                    {
                        foundQuestOreTag = true;
                        break;
                    }
                }
                Assert.IsTrue(foundQuestOreTag, "Active surface tactical brackets must include a [QUEST] ORE tag");
                StringAssert.Contains("QUEST: CARBON ORE", hud.TacticalBrackets.SurfaceCompassSnapshot);

                // Clearing quest mining target restores standard ore brackets and compass strip
                surf.SetQuestMiningTarget(string.Empty, string.Empty);
                Assert.AreEqual(0, surf.QuestMatchedBeaconCount);
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.PlanetarySurface, null, hideForOverlay: false);
                Assert.AreEqual(0, hud.TacticalBrackets.QuestOreBracketCount);
                StringAssert.DoesNotContain("QUEST:", hud.TacticalBrackets.SurfaceCompassSnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestGuidedTourProgress_AndSuggestedCommandPrefill_SyncAcrossJournalAndDialog()
        {
            var rootGo = new UnityEngine.GameObject("QuestGuidedTourProgressTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                string prefilled = string.Empty;
                hud.QuestJournal.OnPrefillCommandRequested += cmd => prefilled = cmd;

                var step5Json = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#12"",
                    ""room_name"": ""Registry & Co-op Desk"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""Step 1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""Step 2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""Step 3"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_04_return_haul"", ""title"": ""Step 4"" }
                    ],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"",
                            ""title"": ""Refit the Dray for Surface Mining"",
                            ""category"": ""guided"",
                            ""next_step"": ""Run +coop/refit prospector at Registry"",
                            ""checkpoints"": [
                                { ""id"": ""refit_dray_p"", ""title"": ""Refit your Dray into a Dray-P prospector"", ""instructions"": ""Run +coop/refit prospector."", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(step5Json));

                Assert.IsTrue(hud.QuestJournal.TryGetGuidedTourProgress(out int done, out int total, out int curStep, out bool allDone));
                Assert.AreEqual(4, done);
                Assert.AreEqual(7, total);
                Assert.AreEqual(5, curStep);
                Assert.IsFalse(allDone);
                Assert.AreEqual("TOUR [####>--] 4/7", hud.QuestJournal.GuidedTourProgressSnapshot);

                Assert.IsTrue(hud.QuestJournal.IsJournalPrefillVisible);
                Assert.IsTrue(hud.QuestJournal.TriggerJournalSuggestedPrefill());
                Assert.AreEqual("+coop/refit prospector", prefilled);

                // Step 1 Co-op enrollment suggests +coop/enroll
                var step1Json = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#12"",
                    ""room_name"": ""Registry & Co-op Desk"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""completed"": [],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                            ""title"": ""First Papers at the Registry"",
                            ""category"": ""guided"",
                            ""checkpoints"": [
                                { ""id"": ""enroll_coop"", ""title"": ""Enroll in the Belt Co-op"", ""instructions"": ""Run +coop/enroll <ship name>."", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(step1Json));
                Assert.AreEqual("TOUR [>------] 0/7", hud.QuestJournal.GuidedTourProgressSnapshot);
                Assert.IsTrue(hud.QuestJournal.TriggerJournalSuggestedPrefill());
                Assert.AreEqual("+coop/enroll ", prefilled);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestCoopRefitAction_CorridorCaptainMastery_AndCompletionBanner_WorkEndToEnd()
        {
            var rootGo = new UnityEngine.GameObject("QuestCoopMasteryBannerTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                // 1. Active Step 5 exposes TryGetActiveCoopAction -> +coop/refit prospector
                var step5Json = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#12"",
                    ""room_name"": ""Registry & Co-op Desk"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""completed"": [],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"",
                            ""title"": ""Refit the Dray for Surface Mining"",
                            ""category"": ""guided"",
                            ""checkpoints"": [
                                { ""id"": ""refit_dray_p"", ""title"": ""Refit your Dray"", ""instructions"": ""Run +coop/refit prospector."", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(step5Json));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveCoopAction(out string qTitle, out string qLbl, out string qCmd));
                Assert.AreEqual("Refit the Dray for Surface Mining", qTitle);
                Assert.AreEqual("Refit Dray-P", qLbl);
                Assert.AreEqual("+coop/refit prospector", qCmd);

                // 2. Completing all 7 guided quests switches Tour snapshot to CORRIDOR CAPTAIN [#######] 7/7
                var masteredJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#22"",
                    ""room_name"": ""Service Counter"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""active"": [],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_04_return_haul"", ""title"": ""S4"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"", ""title"": ""S5"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"", ""title"": ""S6"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore"", ""title"": ""S7"" }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(masteredJson));
                Assert.AreEqual("CORRIDOR CAPTAIN [#######] 7/7", hud.QuestJournal.GuidedTourProgressSnapshot);

                // 3. Milestone completion banner formats standard & 7/7 Corridor Captain celebration cleanly
                Assert.AreEqual(
                    "QUEST COMPLETED: FIRST PAPERS",
                    TacSim.UI.TacSimFloatingCallouts.FormatQuestCompletionBanner("First Papers", 1, 7, false));
                string tourBanner = TacSim.UI.TacSimFloatingCallouts.FormatQuestCompletionBanner("Sell Mined Ore", 7, 7, true);
                Assert.AreEqual("CORRIDOR CAPTAIN — GUIDED TOUR COMPLETE (7/7)", tourBanner);
                hud.FloatingCallouts.ShowMilestoneBanner(tourBanner, UnityEngine.Color.green, 3.0f);
                Assert.IsTrue(hud.FloatingCallouts.IsMilestoneBannerVisible);
                Assert.AreEqual(tourBanner, hud.FloatingCallouts.ActiveMilestoneBannerText);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestSpaceHudDestinationBanner_AndStationExitBreadcrumb_WorkEndToEnd()
        {
            var rootGo = new UnityEngine.GameObject("QuestSpaceHudAndBreadcrumbTestRoot");
            try
            {
                var station = rootGo.AddComponent<StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                // 1. Multi-hop BFS breadcrumb from Visiting Berth Gallery to Registry & Co-op Desk
                Assert.IsTrue(
                    station.TryGetNextQuestExitBreadcrumb(
                        "room.c01_berth_gallery",
                        "room.c05_registry",
                        out string nextEntry,
                        out string nextName,
                        out string nextCmd,
                        out int hops));
                Assert.IsFalse(string.IsNullOrEmpty(nextEntry));
                Assert.IsFalse(string.IsNullOrEmpty(nextName));
                Assert.IsTrue(nextCmd.StartsWith("goto ", System.StringComparison.OrdinalIgnoreCase), $"Unexpected exit cmd: {nextCmd}");
                Assert.GreaterOrEqual(hops, 2);

                // Arriving at target clears breadcrumb
                Assert.IsFalse(
                    station.TryGetNextQuestExitBreadcrumb(
                        "room.c05_registry",
                        "room.c05_registry",
                        out _, out _, out _, out _));

                // 2. HUD surfaces 1-click [Q] -> <NextRoom> (<N>h) quick action breadcrumb
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.RefreshContextualQuickActions("room.c01_berth_gallery", false);
                hud.SetQuestExitBreadcrumb(nextName, nextCmd, hops);
                Assert.AreEqual($"[Q] -> {nextName} ({hops}h)", hud.QuestExitBreadcrumbLabel);
                Assert.AreEqual(nextCmd, hud.QuestExitBreadcrumbCommand);

                hud.SetQuestExitBreadcrumb(string.Empty, string.Empty, 0);
                Assert.AreEqual(string.Empty, hud.QuestExitBreadcrumbLabel);

                // 3. Space flight tactical HUD displays golden [QUEST] DEST banner when seated at Bridge Helm
                hud.TacticalBrackets.SetSpaceQuestDestination("stickney", "Stickney Yards");
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.ShipHelmSeated, null);
                Assert.IsTrue(hud.TacticalBrackets.IsSpaceQuestBannerVisible);
                Assert.AreEqual(
                    "[QUEST] DEST: STICKNEY YARDS (+ship/plan stickney)",
                    hud.TacticalBrackets.SpaceQuestBannerSnapshot);

                // Hides automatically on StationWalk or when cleared
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.StationWalk, null);
                Assert.IsFalse(hud.TacticalBrackets.IsSpaceQuestBannerVisible);

                hud.TacticalBrackets.SetSpaceQuestDestination(string.Empty, string.Empty);
                hud.TacticalBrackets.SyncBrackets(TacSimActiveStage.ShipHelmSeated, null);
                Assert.IsFalse(hud.TacticalBrackets.IsSpaceQuestBannerVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestTrackerCard_ShowsTourProgressRouteStepQuickAccept_AndCorridorCaptainMastery()
        {
            var rootGo = new UnityEngine.GameObject("QuestTrackerRouteQuickAcceptTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                var submitted = new List<string>();
                hud.QuestJournal.OnCommandRequested += cmd => submitted.Add(cmd);

                // 1. Active guided quest + station route breadcrumb shows TOUR 1/7, Route line, and WALK (3h) quick action
                var activeJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#10"",
                    ""room_name"": ""Visiting Berth Gallery"",
                    ""room_markers"": [],
                    ""available"": [],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""First Papers"" }
                    ],
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""title"": ""Stock the Locker and Tanks"",
                            ""next_step"": ""Buy provisions at the Exchange"",
                            ""next_room_entry"": ""room.c07_exchange"",
                            ""checkpoints"": [
                                { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(activeJson));
                hud.QuestJournal.SetTrackerRouteBreadcrumb("Pressure Lobby", "goto n", 3);
                Assert.IsTrue(hud.QuestJournal.PinnedCategory.Contains("TOUR 1/7"), hud.QuestJournal.PinnedCategory);
                Assert.IsTrue(hud.QuestJournal.TrackerDetailSnapshot.Contains("Pressure Lobby (3h)"), hud.QuestJournal.TrackerDetailSnapshot);
                Assert.AreEqual("WALK (3h)", hud.QuestJournal.TrackerActionLabelSnapshot);

                hud.QuestJournal.TriggerTrackerQuickAction();
                Assert.Contains("goto n", submitted);

                // 2. Room offer without active quest shows ACCEPT [!] and submits server-signed accept command
                var offerJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#12"",
                    ""room_name"": ""Port Watch Arrivals"",
                    ""room_markers"": [],
                    ""active"": [],
                    ""completed"": [],
                    ""available"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                            ""title"": ""First Papers"",
                            ""giver_id"": ""#99"",
                            ""giver_name"": ""Maren Halvorsen"",
                            ""accept_cmd"": ""+quest/accept #99 tok42""
                        }
                    ]
                }");
                hud.QuestJournal.SetTrackerRouteBreadcrumb(string.Empty, string.Empty, 0);
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(offerJson));
                Assert.IsTrue(hud.QuestJournal.PinnedCategory.Contains("TOUR 0/7"), hud.QuestJournal.PinnedCategory);
                Assert.AreEqual("ACCEPT [!]", hud.QuestJournal.TrackerActionLabelSnapshot);
                hud.QuestJournal.TriggerTrackerQuickAction();
                Assert.Contains("+quest/accept #99 tok42", submitted);

                // 3. Completing all 7 guided quests displays emerald CORRIDOR CAPTAIN (7/7) mastery card on HUD tracker
                var masteredJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#22"",
                    ""room_name"": ""Service Counter"",
                    ""room_markers"": [],
                    ""active"": [],
                    ""available"": [],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_04_return_haul"", ""title"": ""S4"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"", ""title"": ""S5"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"", ""title"": ""S6"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore"", ""title"": ""S7"" }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(masteredJson));
                Assert.AreEqual("CORRIDOR CAPTAIN (7/7) [J]", hud.QuestJournal.PinnedCategory);
                Assert.IsTrue(hud.QuestJournal.TrackerDetailSnapshot.Contains("CORRIDOR CAPTAIN"), hud.QuestJournal.TrackerDetailSnapshot);
                Assert.AreEqual("LOG [J]", hud.QuestJournal.TrackerActionLabelSnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestTradeStowageReadiness_HighlightsTargetedGaugeAndCargoHoldProgress()
        {
            var rootGo = new UnityEngine.GameObject("QuestTradeStowageReadinessTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                var tradeState = new TacSimTradeAndShipState();
                hud.BindTradeState(tradeState);
                tradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""balance_text"": ""5,000.00 cr"",
                    ""listings"": [],
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 5000,
                            ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 2000,
                            ""cargo_limit_l"": 120000,
                            ""tank_kg"": 5200,
                            ""tank_limit_kg"": 80000,
                            ""stores_days"": 8,
                            ""stores_limit_days"": 120,
                            ""items"": [
                                { ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 5 }
                            ]
                        }
                    }
                }"));

                Assert.AreEqual(5, tradeState.GetHoldCommodityQuantity("propellant"));
                Assert.AreEqual(8, tradeState.GetHoldCommodityQuantity("provisions"));
                Assert.AreEqual(5, tradeState.GetHoldCommodityQuantity("metals"));

                // 1. Provisions quest (8/10 -> loading)
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#18"",
                    ""room_name"": ""Co-op Chandlery"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                        ""title"": ""Stock the Locker and Tanks"",
                        ""checkpoints"": [{
                            ""id"": ""buy_provisions"",
                            ""title"": ""Buy at least 10 provisions"",
                            ""instructions"": ""Buy 10 provisions."",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }]
                }")));
                hud.OpenTradeAndSailingModal("EXCHANGE", openSailingTab: false);
                Assert.AreEqual("[QUEST] PROVISIONS: 8/10 d", hud.TradeStowageQuestSummarySnapshot);

                // 2. Metals haul quest (5/5 -> READY)
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#17"",
                    ""room_name"": ""Exchange Hall"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Buy 5 tonnes of metals and deliver to Stickney Yards"",
                            ""instructions"": ""Buy 5 metals and sell at Stickney."",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }]
                }")));
                hud.RefreshTradeTerminal();
                Assert.AreEqual("[READY] METALS: 5/5 u", hud.TradeStowageQuestSummarySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestShipInteriorWayfinding_GuidesAirlockToBridge_AndHoldToVehicleBay()
        {
            var rootGo = new UnityEngine.GameObject("QuestShipWayfindingTestRoot");
            try
            {
                var ship = rootGo.AddComponent<ShipInteriorController>();
                ship.BuildInterior(null, "tacsim.unit.dray_p");

                // 1. Airlock -> Bridge multi-hop breadcrumb
                Assert.IsTrue(
                    ship.TryGetNextQuestCompartmentBreadcrumb(
                        "airlock",
                        "bridge",
                        out string nextCompId,
                        out string nextCompName,
                        out string nextCmd,
                        out int hops));
                Assert.AreEqual("passage", nextCompId);
                Assert.IsFalse(string.IsNullOrEmpty(nextCompName));
                Assert.IsTrue(nextCmd.StartsWith("goto ", System.StringComparison.OrdinalIgnoreCase), $"Unexpected ship cmd: {nextCmd}");
                Assert.GreaterOrEqual(hops, 2);

                // Already on Bridge -> clears cleanly
                Assert.IsFalse(
                    ship.TryGetNextQuestCompartmentBreadcrumb(
                        "bridge",
                        "bridge",
                        out _, out _, out _, out _));

                // 2. Cargo Hold -> Dray-P Vehicle Bay (+tv/bay in)
                if (ship.Template.Compartments.ContainsKey("bay"))
                {
                    Assert.IsTrue(
                        ship.TryGetNextQuestCompartmentBreadcrumb(
                            "hold",
                            "bay",
                            out string bayId,
                            out _,
                            out string bayCmd,
                            out int bayHops));
                    Assert.AreEqual("bay", bayId);
                    Assert.AreEqual("+tv/bay in", bayCmd);
                    Assert.AreEqual(1, bayHops);
                }

                // 3. QuestJournal resolves "bridge" for flight quests and "bay" for active surface mining quests
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""checkpoints"": [{
                            ""id"": ""sail_stickney"",
                            ""title"": ""Sail to Stickney Yards"",
                            ""instructions"": ""Run +ship/plan stickney and depart."",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }]
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment(out string flightTarget));
                Assert.AreEqual("bridge", flightTarget);

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan and Mine Carbon Ore"",
                        ""checkpoints"": [{
                            ""id"": ""mine_carbon_ore"",
                            ""title"": ""Mine 1t carbon ore"",
                            ""instructions"": ""Run +tv/1 mine go."",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }]
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment(out string mineTarget));
                Assert.AreEqual("bay", mineTarget);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestRefitEmptyHoldReadiness_AndSurfaceProspectingPrepRecoveryHints()
        {
            var rootGo = new UnityEngine.GameObject("QuestRefitAndMiningPrepTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                var tradeState = new TacSimTradeAndShipState();
                tradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 2000,
                            ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 1000,
                            ""cargo_limit_l"": 120000,
                            ""tank_kg"": 12000,
                            ""tank_limit_kg"": 80000,
                            ""stores_days"": 4,
                            ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 2 }]
                        }
                    }
                }"));

                // 1. Step 5 Refit Dray-P active with cargo + provisions in hold -> CLEAR HOLD FIRST
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"",
                        ""title"": ""Refit the Dray for Surface Mining"",
                        ""checkpoints"": [{
                            ""id"": ""refit_dray_p"",
                            ""title"": ""Refit your Dray into a Dray-P prospector"",
                            ""instructions"": ""Run +coop/refit prospector at Registry."",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }]
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetRefitHoldReadiness(tradeState, out var blocked));
                Assert.IsFalse(blocked.Ready);
                Assert.IsFalse(blocked.CargoEmpty);
                Assert.IsFalse(blocked.ProvisionsEmpty);
                Assert.AreEqual(12, blocked.PropellantTonnes);
                string blockedMarkup = TacSimGameCoordinator.FormatRefitHoldCheckMarkup(blocked);
                StringAssert.Contains("[CLEAR HOLD FIRST]", blockedMarkup);
                StringAssert.Contains("Cargo: 2t [CLEAR]", blockedMarkup);
                StringAssert.Contains("Stores: 4d [CLEAR]", blockedMarkup);

                // 2. Empty cargo & stores with propellant still in tank -> READY TO REFIT
                tradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0,
                            ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 0,
                            ""cargo_limit_l"": 120000,
                            ""tank_kg"": 12000,
                            ""tank_limit_kg"": 80000,
                            ""stores_days"": 0,
                            ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                Assert.IsTrue(hud.QuestJournal.TryGetRefitHoldReadiness(tradeState, out var ready));
                Assert.IsTrue(ready.Ready);
                string readyMarkup = TacSimGameCoordinator.FormatRefitHoldCheckMarkup(ready);
                StringAssert.Contains("[READY TO REFIT]", readyMarkup);
                StringAssert.Contains("Tank: 12t OK", readyMarkup);

                // 3. Step 6 surface prospecting prep hints (pre-scan) & post-mine rig recovery command
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""Run +tv/1 scan go."", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""sound_mineral_pocket"", ""title"": ""Sound pocket"", ""instructions"": ""Run +tv/1 sound go."", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""Run +tv/1 mine go."", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveSurfaceMiningStatus(out var preScan));
                StringAssert.Contains("+license", preScan.FormatPrepHintLine());
                StringAssert.Contains("+tv/lease", preScan.FormatPrepHintLine());
                StringAssert.Contains("+chart/ground", preScan.FormatPrepHintLine());

                // Once mine is complete, FormatPrepHintLine and TryGetSuggestedCommand point to +tv/1 recover go
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""ready_to_finish"": false,
                        ""can_check_now"": false,
                        ""can_finish_now"": false,
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""Run +tv/1 scan go."", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""sound_mineral_pocket"", ""title"": ""Sound pocket"", ""instructions"": ""Run +tv/1 sound go."", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""Run +tv/1 mine go."", ""checked"": true, ""live_ready"": true }
                        ]
                    }]
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveSurfaceMiningStatus(out var postMine));
                StringAssert.Contains("+tv/1 recover go", postMine.FormatPrepHintLine());
                Assert.IsTrue(hud.QuestJournal.TryGetSuggestedCommand(hud.QuestJournal.GetTrackedActiveQuest(), out string sugLbl, out string sugCmd));
                Assert.AreEqual("Recover Rig", sugLbl);
                Assert.AreEqual("+tv/1 recover go", sugCmd);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestCrossPortBerthBoarding_AndDockedAirlockDisembarkWayfinding()
        {
            var rootGo = new UnityEngine.GameObject("QuestCrossPortWayfindingTestRoot");
            try
            {
                // 1. Port prefix inference
                Assert.AreEqual("ceres", TacSim.UI.TacSimQuestJournal.InferStationPortFromRoomEntry("room.c08_chandlery"));
                Assert.AreEqual("stickney", TacSim.UI.TacSimQuestJournal.InferStationPortFromRoomEntry("room.p07_receiving"));
                Assert.AreEqual("luna", TacSim.UI.TacSimQuestJournal.InferStationPortFromRoomEntry("room.l01_berth_gallery"));
                Assert.AreEqual("deimos", TacSim.UI.TacSimQuestJournal.InferStationPortFromRoomEntry("room.d02_concourse"));
                Assert.AreEqual("titan", TacSim.UI.TacSimQuestJournal.InferStationPortFromRoomEntry("room.t04_deck"));
                Assert.IsNull(TacSim.UI.TacSimQuestJournal.InferStationPortFromRoomEntry("room.welcome_hall"));

                // 2. Ashore on Ceres with Stickney objective -> walks toward Berth Gallery, then emits +ship/board
                var station = rootGo.AddComponent<StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");
                Assert.IsTrue(
                    station.TryGetNextQuestExitBreadcrumb(
                        "room.c05_registry",
                        "room.p07_receiving",
                        out string nextEntry,
                        out _,
                        out string nextCmd,
                        out int hopsToBerth));
                Assert.IsFalse(string.IsNullOrEmpty(nextEntry));
                Assert.IsTrue(nextCmd.StartsWith("goto ", System.StringComparison.OrdinalIgnoreCase), $"Unexpected cmd: {nextCmd}");
                Assert.GreaterOrEqual(hopsToBerth, 2);

                Assert.IsTrue(
                    station.TryGetNextQuestExitBreadcrumb(
                        "room.c01_berth_gallery",
                        "room.p07_receiving",
                        out string shipId,
                        out string boardName,
                        out string boardCmd,
                        out int boardHops));
                Assert.AreEqual("ship", shipId);
                Assert.AreEqual("Board Ship", boardName);
                Assert.AreEqual("+ship/board", boardCmd);
                Assert.AreEqual(1, boardHops);

                // 3. Aboard ship docked at Stickney -> QuestJournal resolves "airlock" instead of "bridge"
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""next_room_entry"": ""room.p07_receiving"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Deliver 5t metals to Stickney Yards"",
                            ""instructions"": ""Check +ship/plan stickney and sell at Receiving."",
                            ""room_entry"": ""room.p07_receiving"",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }]
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("ceres", out string ceresTarget));
                Assert.AreEqual("bridge", ceresTarget);
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("stickney", out string stickneyTarget));
                Assert.AreEqual("airlock", stickneyTarget);

                // 4. ShipInteriorController routes bridge -> airlock, then emits +ship/leave in airlock
                var ship = rootGo.AddComponent<ShipInteriorController>();
                ship.BuildInterior(null, "tacsim.unit.dray");
                Assert.IsTrue(
                    ship.TryGetNextQuestCompartmentBreadcrumb(
                        "bridge",
                        "airlock",
                        out string nextCompId,
                        out _,
                        out string toLockCmd,
                        out int toLockHops));
                Assert.AreEqual("passage", nextCompId);
                Assert.IsTrue(toLockCmd.StartsWith("goto ", System.StringComparison.OrdinalIgnoreCase));
                Assert.GreaterOrEqual(toLockHops, 3);

                Assert.IsTrue(
                    ship.TryGetNextQuestCompartmentBreadcrumb(
                        "airlock",
                        "airlock",
                        out string ashoreId,
                        out string leaveName,
                        out string leaveCmd,
                        out int leaveHops));
                Assert.AreEqual("ashore", ashoreId);
                Assert.AreEqual("Cycle Ashore", leaveName);
                Assert.AreEqual("+ship/leave", leaveCmd);
                Assert.AreEqual(1, leaveHops);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestVehicleBayCradleMarkup_AndStickneyRefitDeskReadiness()
        {
            var preScan = new TacSim.UI.QuestSurfaceMiningStatus(
                "Scan, Sound, and Mine a Landing Site",
                "carbon_ore",
                "carbon",
                hasScanStep: true, scanDone: false,
                hasSoundStep: true, soundDone: false,
                hasMineStep: true, mineDone: false,
                nextStepText: "Scan site");
            string preMarkup = TacSimGameCoordinator.FormatVehicleBayQuestBannerMarkup(preScan);
            StringAssert.Contains("[QUEST] Scan, Sound, and Mine a Landing Site", preMarkup);
            StringAssert.Contains("Mine carbon_ore", preMarkup);
            StringAssert.Contains("+license", preMarkup);
            StringAssert.Contains("+tv/lease", preMarkup);
            StringAssert.Contains("+chart/ground", preMarkup);

            var postMine = new TacSim.UI.QuestSurfaceMiningStatus(
                "Scan, Sound, and Mine a Landing Site",
                "carbon_ore",
                "carbon",
                hasScanStep: true, scanDone: true,
                hasSoundStep: true, soundDone: true,
                hasMineStep: true, mineDone: true,
                nextStepText: "Recover rig");
            string postMarkup = TacSimGameCoordinator.FormatVehicleBayQuestBannerMarkup(postMine);
            StringAssert.Contains("[✓]", postMarkup);
            StringAssert.Contains("+tv/1 recover go", postMarkup);
        }

        [Test]
        public void QuestTrackerCard_CyclesMultipleActiveQuests_ShowsSimultaneousOffers_AndBoardingLabels()
        {
            var rootGo = new UnityEngine.GameObject("QuestTrackerMultiCycleTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                var submitted = new List<string>();
                hud.QuestJournal.OnCommandRequested += cmd => submitted.Add(cmd);

                var multiJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#18"",
                    ""room_name"": ""Visiting Berth Gallery"",
                    ""active"": [
                        { ""key"": ""q.first"", ""title"": ""First Active Quest"", ""next_step"": ""Board ship"", ""checkpoints"": [] },
                        { ""key"": ""q.second"", ""title"": ""Second Active Quest"", ""next_step"": ""Cycle ashore"", ""checkpoints"": [] }
                    ],
                    ""available"": [
                        { ""key"": ""q.offer"", ""title"": ""Room Side Offer"", ""giver_id"": ""#42"", ""giver_name"": ""Ilse"", ""accept_cmd"": ""+quest/accept #42 tok"" }
                    ]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(multiJson));
                Assert.IsTrue(hud.QuestJournal.IsTrackerCycleVisible);
                StringAssert.Contains("[1/2] First Active Quest", hud.QuestJournal.TrackerDetailSnapshot);
                StringAssert.Contains("[!] 1 offer here", hud.QuestJournal.TrackerDetailSnapshot);

                // Cross-port boarding route labels tracker action as BOARD SHIP and executes +ship/board
                hud.QuestJournal.SetTrackerRouteBreadcrumb("Board Ship", "+ship/board", 1);
                Assert.AreEqual("BOARD SHIP", hud.QuestJournal.TrackerActionLabelSnapshot);
                hud.QuestJournal.TriggerTrackerQuickAction();
                Assert.Contains("+ship/board", submitted);

                // Cycling advances to Second Active Quest ([2/2]) and supports ASHORE (+ship/leave)
                Assert.AreEqual("q.second", hud.QuestJournal.CycleTrackedQuest());
                StringAssert.Contains("[2/2] Second Active Quest", hud.QuestJournal.TrackerDetailSnapshot);
                hud.QuestJournal.SetTrackerRouteBreadcrumb("Cycle Ashore", "+ship/leave", 1);
                Assert.AreEqual("ASHORE", hud.QuestJournal.TrackerActionLabelSnapshot);
                hud.QuestJournal.TriggerTrackerQuickAction();
                Assert.Contains("+ship/leave", submitted);

                // Cycling wraps back to q.first
                Assert.AreEqual("q.first", hud.QuestJournal.CycleTrackedQuest());
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void CommandPalette_SurfacesQuestRouteNextStepFinishAndCycleTracked()
        {
            var rootGo = new UnityEngine.GameObject("CommandPaletteQuestShortcutsTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.CommandPalette.BindQuestJournal(hud.QuestJournal);

                var executed = new List<string>();
                var prefilled = new List<string>();
                hud.CommandPalette.OnCommandRequested += cmd => executed.Add(cmd);
                hud.CommandPalette.OnPrefillCommandRequested += cmd => prefilled.Add(cmd);

                var panelJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#18"",
                    ""room_name"": ""Stickney Yard Gate"",
                    ""active"": [
                        {
                            ""key"": ""q.refit"",
                            ""title"": ""Refit the Dray"",
                            ""next_step"": ""Refit your Dray at Stickney Yards (+coop/refit prospector)"",
                            ""next_instructions"": ""Use +coop/refit prospector with an empty hold."",
                            ""checkpoints"": []
                        },
                        {
                            ""key"": ""q.sell"",
                            ""title"": ""Sell Carbon Ore"",
                            ""turn_in_name"": ""Kowalski"",
                            ""next_step"": ""Turn in to Kowalski"",
                            ""ready_to_finish"": true,
                            ""can_finish_now"": true,
                            ""finish_cmd"": ""+quest/finish q.sell"",
                            ""checkpoints"": []
                        }
                    ],
                    ""available"": []
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(panelJson));

                // 1. Suggested step prefills +coop/refit prospector
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("quest step next refit");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.Contains("+coop/refit prospector", prefilled);

                // 2. Live route breadcrumb (+ship/board) appears and executes +ship/board
                hud.QuestJournal.SetTrackerRouteBreadcrumb("Board Ship", "+ship/board", 1);
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("quest route board ship");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                StringAssert.Contains("Board Ship", hud.CommandPalette.SelectedEntryTitle);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.Contains("+ship/board", executed);

                // 3. Cycle Tracked Quest advances to q.sell
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("cycle tracked quest");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.AreEqual("q.sell", hud.QuestJournal.TrackedQuestKey);

                // 4. Finish Now dispatches +quest/finish q.sell
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("finish now sell carbon");
                Assert.Greater(hud.CommandPalette.FilteredMatchCount, 0);
                Assert.IsTrue(hud.CommandPalette.ExecuteSelected());
                Assert.Contains("+quest/finish q.sell", executed);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void SailingQuestBanner_EvaluatesPreflightReadinessAndSwitchesToExchangeWhenIncomplete()
        {
            var rootGo = new UnityEngine.GameObject("SailingPreflightTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var trade = new TacSimTradeAndShipState();
                hud.BindTradeState(trade);

                var questJson = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#17"",
                    ""room_name"": ""Exchange Hall"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Deliver 5t metals to Stickney Yards"",
                            ""instructions"": ""Check +ship/plan stickney and sell at Stickney Yards."",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }],
                    ""available"": []
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(questJson));

                // Missing fuel, stores, and only 2/5 metals -> [CHECK BEFORE UNDOCK] + EXCHANGE button visible
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 2000, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 1000, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 0, ""tank_limit_kg"": 80000,
                            ""stores_days"": 0, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 2 }]
                        }
                    }
                }"));
                hud.OpenTradeAndSailingModal("SAILING PLAN", openSailingTab: true);
                Assert.IsTrue(hud.IsSailingQuestBannerVisible);
                Assert.IsTrue(hud.IsSailingExchangeJumpVisible);
                StringAssert.Contains("[CHECK BEFORE UNDOCK]", hud.SailingPreflightSummarySnapshot);
                StringAssert.Contains("Fuel 0t!", hud.SailingPreflightSummarySnapshot);
                StringAssert.Contains("Stores 0d!", hud.SailingPreflightSummarySnapshot);
                StringAssert.Contains("metals 2/5!", hud.SailingPreflightSummarySnapshot);
                StringAssert.Contains("[CHECK BEFORE UNDOCK]", hud.SailingQuestBannerText);

                // Clicking Exchange jump switches to Exchange tab
                hud.JumpFromSailingToExchange();
                Assert.AreEqual(TacSim.UI.TradeTerminalTab.Exchange, hud.ActiveTradeTab);

                // Top up fuel, stores, and 5/5 metals -> [PRE-FLIGHT OK] and hides EXCHANGE jump button
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 5000, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 2500, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 14000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 12, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 5 }]
                        }
                    }
                }"));
                hud.OpenTradeAndSailingModal("SAILING PLAN", openSailingTab: true);
                Assert.IsFalse(hud.IsSailingExchangeJumpVisible);
                StringAssert.Contains("[PRE-FLIGHT OK]", hud.SailingPreflightSummarySnapshot);
                StringAssert.Contains("Fuel 14t", hud.SailingPreflightSummarySnapshot);
                StringAssert.Contains("Stores 12d", hud.SailingPreflightSummarySnapshot);
                StringAssert.Contains("metals 5/5 OK", hud.SailingPreflightSummarySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void OriginPortProcurementWayfinding_RoutesToExchangeUntilCargoLoaded_ThenFlipsToBerthGallery()
        {
            var rootGo = new UnityEngine.GameObject("OriginPortProcurementTestRoot");
            try
            {
                var stGo = new UnityEngine.GameObject("CeresStation");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var trade = new TacSimTradeAndShipState();
                hud.BindTradeState(trade);

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#15"",
                    ""room_name"": ""Registry & Co-op Desk"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""next_room_entry"": ""room.p07_receiving"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards"",
                            ""instructions"": ""Buy metals at Ceres Exchange Hall, fly to Stickney, and +market/sell."",
                            ""room_entry"": ""room.p07_receiving"",
                            ""checked"": false,
                            ""live_ready"": false
                        }]
                    }],
                    ""available"": []
                }")));

                // Unloaded stowage -> false (default cross-port berth routing preserved)
                Assert.IsFalse(hud.QuestJournal.NeedsOriginPortProcurement(trade));

                // Loaded stowage with 2/5 metals -> needs origin Exchange first
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 2000, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 1000, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 12000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 2 }]
                        }
                    }
                }"));
                Assert.IsTrue(hud.QuestJournal.NeedsOriginPortProcurement(trade));

                // From Registry while procuring -> walks toward Exchange Hall (not Berth Gallery!)
                Assert.IsTrue(station.TryGetNextQuestExitBreadcrumb(
                    "room.c05_registry", "room.p07_receiving",
                    out string nextFromReg, out _, out _, out int hopsToExch,
                    needsOriginExchangeFirst: true));
                Assert.GreaterOrEqual(hopsToExch, 1);
                Assert.AreNotEqual("room.c01_berth_gallery", nextFromReg);

                // Standing at Exchange Hall while procuring -> returns false (stay here to buy)
                Assert.IsFalse(station.TryGetNextQuestExitBreadcrumb(
                    "room.c07_exchange", "room.p07_receiving",
                    out _, out _, out _, out _,
                    needsOriginExchangeFirst: true));

                // Top up to 5/5 metals -> NeedsOriginPortProcurement flips false and routes to Berth Gallery
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 5000, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 2500, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 12000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 5 }]
                        }
                    }
                }"));
                Assert.IsFalse(hud.QuestJournal.NeedsOriginPortProcurement(trade));
                Assert.IsTrue(station.TryGetNextQuestExitBreadcrumb(
                    "room.c01_berth_gallery", "room.p07_receiving",
                    out _, out string boardName, out string boardCmd, out int boardHops,
                    needsOriginExchangeFirst: false));
                Assert.AreEqual("Board Ship", boardName);
                Assert.AreEqual("+ship/board", boardCmd);
                Assert.AreEqual(1, boardHops);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Step6SurfaceExpeditionWayfinding_RoutesBerthBridgeBayBridgeAirlock()
        {
            var rootGo = new UnityEngine.GameObject("Step6ExpeditionWayfindingTestRoot");
            try
            {
                var stGo = new UnityEngine.GameObject("CeresStationStep6");
                stGo.transform.SetParent(rootGo.transform, false);
                var station = stGo.AddComponent<TacSim.World.StationWorldController>();
                station.BuildStation(null, "neighborhood.ceres_freeport");

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#17"",
                    ""room_name"": ""Exchange Hall"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""room_entry"": ""room.c07_exchange"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""room_entry"": ""room.c07_exchange"", ""checked"": false, ""live_ready"": false }
                        ]
                    }],
                    ""available"": []
                }")));

                // 1. Standing right in Rosalind's Exchange Hall ashore -> NeedsSurfaceExpeditionBoarding routes to Berth Gallery
                Assert.IsTrue(hud.QuestJournal.NeedsSurfaceExpeditionBoarding());
                Assert.IsTrue(station.TryGetNextQuestExitBreadcrumb(
                    "room.c07_exchange", "room.c07_exchange",
                    out _, out _, out _, out int hopsToBerth,
                    needsOriginExchangeFirst: false,
                    needsShipExpedition: true));
                Assert.GreaterOrEqual(hopsToBerth, 2);
                Assert.IsTrue(station.TryGetNextQuestExitBreadcrumb(
                    "room.c01_berth_gallery", "room.c07_exchange",
                    out _, out string boardName, out string boardCmd, out int boardHops,
                    needsOriginExchangeFirst: false,
                    needsShipExpedition: true));
                Assert.AreEqual("Board Ship", boardName);
                Assert.AreEqual("+ship/board", boardCmd);
                Assert.AreEqual(1, boardHops);

                // 2. Aboard Dray-P still docked at Ceres station -> bridge (fly to landing site first)
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("ceres", out string dockedTarget, settledPad: false));
                Assert.AreEqual("bridge", dockedTarget);

                // 3. Settled on a surface landing pad -> bay (launch & operate rig)
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("ceres", out string padTarget, settledPad: true));
                Assert.AreEqual("bay", padTarget);

                // 4. Mining finished -> bridge while still on pad, airlock once docked back at Ceres
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#50"",
                    ""room_name"": ""M/V Starling - Passage"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""room_entry"": ""room.c07_exchange"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""room_entry"": ""room.c07_exchange"", ""checked"": true, ""live_ready"": true }
                        ]
                    }],
                    ""available"": []
                }")));
                Assert.IsFalse(hud.QuestJournal.NeedsSurfaceExpeditionBoarding());
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment(null, out string liftOffTarget, settledPad: true));
                Assert.AreEqual("bridge", liftOffTarget);
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("ceres", out string returnTarget, settledPad: false));
                Assert.AreEqual("airlock", returnTarget);

                // 5. Post-mine off-world destination fallback routes Sailing Plan / Space HUD back to Ceres Freeport
                Assert.IsTrue(hud.QuestJournal.TryGetActiveDestinationPort(out var homeHint));
                Assert.AreEqual("ceres", homeHint.PortId);
                Assert.AreEqual("Ceres Freeport", homeHint.PortLabel);

                // Once standing at the turn-in room, destination hint clears
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#17"",
                    ""room_name"": ""Exchange Hall"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""turn_in_here"": true,
                        ""ready_to_finish"": true,
                        ""can_finish_now"": true,
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""room_entry"": ""room.c07_exchange"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""room_entry"": ""room.c07_exchange"", ""checked"": true, ""live_ready"": true }
                        ]
                    }],
                    ""available"": []
                }")));
                Assert.IsFalse(hud.QuestJournal.TryGetActiveDestinationPort(out _));
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestGuidedTourRoadmap_TracksAllSevenStepsAndRendersJournalHeader()
        {
            var rootGo = new UnityEngine.GameObject("QuestGuidedTourRoadmapTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                var json = JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#18"",
                    ""room_name"": ""Registry & Co-op Desk"",
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""Step 1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""Step 2"" }
                    ],
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""ready_to_finish"": true,
                        ""checkpoints"": []
                    }],
                    ""available"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_04_return_haul"",
                        ""title"": ""A Return Load for Ceres"",
                        ""checkpoints"": []
                    }]
                }");
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(json));

                var steps = new List<TacSim.UI.GuidedTourRoadmapStep>();
                Assert.IsTrue(hud.QuestJournal.TryGetGuidedTourRoadmap(steps));
                Assert.AreEqual(7, steps.Count);
                Assert.AreEqual(TacSim.UI.GuidedTourStepStatus.Completed, steps[0].Status);
                Assert.AreEqual("Co-op Charter", steps[0].ShortLabel);
                Assert.AreEqual(TacSim.UI.GuidedTourStepStatus.Completed, steps[1].Status);
                Assert.AreEqual(TacSim.UI.GuidedTourStepStatus.Active, steps[2].Status);
                Assert.IsTrue(steps[2].ReadyToTurnIn);
                Assert.AreEqual(TacSim.UI.GuidedTourStepStatus.Available, steps[3].Status);
                Assert.AreEqual(TacSim.UI.GuidedTourStepStatus.Upcoming, steps[4].Status);
                Assert.AreEqual("Sell Mined Ore", steps[6].ShortLabel);

                string snap = hud.QuestJournal.GuidedTourRoadmapSnapshot;
                Assert.IsTrue(snap.Contains("[✓] 1. Co-op Charter"), snap);
                Assert.IsTrue(snap.Contains("[?] 3. Metals Haul"), snap);
                Assert.IsTrue(snap.Contains("[!] 4. Return Load"), snap);
                Assert.IsTrue(snap.Contains("[○] 7. Sell Mined Ore"), snap);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestNextGuidedOfferLocator_RoutesTrackerWayfinderAndShipBetweenSteps()
        {
            var rootGo = new UnityEngine.GameObject("QuestNextGuidedOfferTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var submitted = new List<string>();
                string wayfinderRoom = string.Empty;
                hud.QuestJournal.OnCommandRequested += cmd => submitted.Add(cmd);
                hud.QuestJournal.OnWayfinderRoomRequested += room => wayfinderRoom = room;

                // Steps 1–3 completed, no active quest, no room offers -> locates Step 4 Jonas at Stickney Receiving
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#40"",
                    ""room_name"": ""Stickney Concourse"",
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" }
                    ],
                    ""active"": [],
                    ""available"": []
                }")));

                Assert.IsTrue(hud.QuestJournal.TryGetNextGuidedOffer(out var nextOffer));
                Assert.AreEqual(4, nextOffer.Step);
                Assert.AreEqual("Return Load", nextOffer.ShortLabel);
                Assert.AreEqual("Jonas Achterberg-Nkemelu", nextOffer.GiverLabel);
                Assert.AreEqual("room.p07_receiving", nextOffer.GiverRoomEntry);

                // Minimap objective & ship compartment resolve to giver room / airlock when docked at Stickney
                Assert.IsTrue(hud.QuestJournal.TryGetMinimapObjective(out string objRoom, out bool isTurnIn, out _));
                Assert.AreEqual("room.p07_receiving", objRoom);
                Assert.IsFalse(isTurnIn);
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("stickney", out string dockedTarget, settledPad: false));
                Assert.AreEqual("airlock", dockedTarget);
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("ceres", out string otherPortTarget, settledPad: false));
                Assert.AreEqual("bridge", otherPortTarget);

                // Tracker stays pinned with Next Guided card, wayfinder, and 1-click route walk
                Assert.IsTrue(hud.QuestJournal.HasPinnedObjective);
                Assert.IsTrue(hud.QuestJournal.PinnedCategory.Contains("NEXT GUIDED (4/7)"), hud.QuestJournal.PinnedCategory);
                Assert.IsTrue(hud.QuestJournal.IsTrackerWayfinderVisible);
                Assert.IsTrue(hud.QuestJournal.TriggerTrackerWayfinder());
                Assert.AreEqual("room.p07_receiving", wayfinderRoom);

                hud.QuestJournal.SetTrackerRouteBreadcrumb("Receiving Counter", "goto receiving", 1);
                Assert.AreEqual("WALK (1h)", hud.QuestJournal.TrackerActionLabelSnapshot);
                hud.QuestJournal.TriggerTrackerQuickAction();
                Assert.Contains("goto receiving", submitted);

                // Fresh player (0 completed) -> false so non-guided worlds remain quiet
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1, ""completed"": [], ""active"": [], ""available"": []
                }")));
                Assert.IsFalse(hud.QuestJournal.TryGetNextGuidedOffer(out _));
                Assert.IsFalse(hud.QuestJournal.HasPinnedObjective);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void GroundChartQuestRecommendation_AutoSelectsCarbonOrStarterSiteAndShowsReturnBanner()
        {
            var rootGo = new UnityEngine.GameObject("GroundChartQuestRecTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                // Active Step 6 quest before mining
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#17"",
                    ""room_name"": ""Exchange Hall"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));

                // Apply Ground Chart where Freeport Commons has carbon_ore reading -> auto-selected & banner shows Site
                hud.GroundChart.ApplyGroundFrame(JsonNode.Parse(@"{
                    ""width"": 8, ""height"": 4, ""title"": ""Ceres — Carbon Wash"",
                    ""rows"": [ ""8."", ""8."", ""8."", ""8."" ],
                    ""ground"": {
                        ""body"": ""ceres"", ""name"": ""Ceres"",
                        ""family"": ""carbon"", ""family_word"": ""carbon"",
                        ""families"": [ ""volatiles"", ""metals"", ""strategic"", ""carbon"" ],
                        ""ports"": [],
                        ""sites"": [
                            { ""letter"": ""A"", ""site"": ""Occator Rim"", ""venue"": ""occator"", ""lat_mdeg"": 19800, ""lon_mdeg"": 239300, ""tenure"": ""open"", ""here"": false, ""best"": ""water_ice"", ""best_line"": ""water_ice fair"" },
                            { ""letter"": ""C"", ""site"": ""Freeport Commons"", ""venue"": ""commons"", ""lat_mdeg"": 1000, ""lon_mdeg"": 12000, ""tenure"": ""open"", ""here"": false, ""best"": ""carbon_ore"", ""best_line"": ""carbon_ore rich"" }
                        ],
                        ""lines"": [ ""A Occator Rim"", ""C Freeport Commons"" ]
                    }
                }"));

                Assert.IsTrue(hud.GroundChart.TryGetRecommendedQuestSite(out var rec));
                Assert.AreEqual("commons", rec.Venue);
                Assert.AreEqual("commons", hud.GroundChart.SelectedSiteVenue);
                StringAssert.Contains("Freeport Commons", hud.GroundChart.QuestMiningBannerText);
                StringAssert.Contains("+ship/plan commons", hud.GroundChart.QuestMiningBannerText);

                // Post-mine -> banner switches to Return: Ceres Freeport (+ship/plan ceres)
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#50"",
                    ""room_name"": ""M/V Starling - Passage"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""ready_to_finish"": true,
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": true, ""live_ready"": true }
                        ]
                    }]
                }")));
                Assert.IsFalse(hud.GroundChart.TryGetRecommendedQuestSite(out _));
                StringAssert.Contains("Return: Ceres Freeport", hud.GroundChart.QuestMiningBannerText);
                StringAssert.Contains("+ship/plan ceres", hud.GroundChart.QuestMiningBannerText);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestJournalAndDialog_ShowLiveHoldProgressAndRefitReadinessFromTradeState()
        {
            var rootGo = new UnityEngine.GameObject("QuestJournalHoldReadinessTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var trade = new TacSimTradeAndShipState();
                hud.BindTradeState(trade);

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#17"",
                    ""room_name"": ""Exchange Hall"",
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                            ""title"": ""Metals Down the Corridor"",
                            ""giver_name"": ""Ilse Oduya-Brandt"",
                            ""turn_in_name"": ""Jonas Achterberg-Nkemelu"",
                            ""checkpoints"": [{
                                ""id"": ""deliver_metals"",
                                ""title"": ""Buy 5 tonnes of metals at Ceres and deliver to Stickney"",
                                ""instructions"": ""Buy 5t metals and +market/sell at Stickney."",
                                ""checked"": false,
                                ""live_ready"": false
                            }]
                        },
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"",
                            ""title"": ""Refit the Dray for Surface Mining"",
                            ""giver_name"": ""Ilse Oduya-Brandt"",
                            ""turn_in_name"": ""Rosalind Qureshi-Mbeki"",
                            ""checkpoints"": [{
                                ""id"": ""refit_dray_p"",
                                ""title"": ""Refit Dray-P"",
                                ""instructions"": ""Run +coop/refit prospector."",
                                ""checked"": false,
                                ""live_ready"": false
                            }]
                        }
                    ]
                }")));

                // Partial hold -> [HOLD] METALS: 2/5 u in Journal and Dialog
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 2000, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 1000, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 10000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 8, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 2 }]
                        }
                    }
                }"));
                StringAssert.Contains("[HOLD] METALS: 2/5 u", hud.QuestJournal.JournalBodySnapshot);

                hud.QuestJournal.OpenQuestDialogByKey("tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery");
                StringAssert.Contains("[HOLD] METALS: 2/5 u", hud.QuestJournal.DialogBodySnapshot);

                // Top up metals -> live TradeState event automatically refreshes open Quest Dialog to [HOLD READY]
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 5000, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 2500, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 10000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 8, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 5 }]
                        }
                    }
                }"));
                StringAssert.Contains("[HOLD READY] METALS: 5/5 u", hud.QuestJournal.DialogBodySnapshot);

                // Step 5 refit check in Dialog -> [REFIT CHECK] with cargo/stores, then [READY] when emptied
                hud.QuestJournal.OpenQuestDialogByKey("tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector");
                StringAssert.Contains("[REFIT CHECK]", hud.QuestJournal.DialogBodySnapshot);
                StringAssert.Contains("Stores 8d", hud.QuestJournal.DialogBodySnapshot);

                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 0, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 6000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 0, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                StringAssert.Contains("[READY] Cargo empty | Stores empty | Tank 6t OK", hud.QuestJournal.DialogBodySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void CharacterSheetPilotTab_ShowsGuidedTourProgressRoadmapAndNextOfferBetweenSteps()
        {
            var rootGo = new UnityEngine.GameObject("CharacterSheetGuidedRoadmapTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#40"",
                    ""room_name"": ""Stickney Concourse"",
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" }
                    ],
                    ""active"": [],
                    ""available"": []
                }")));
                hud.CharacterSheet.RefreshContents();

                string summary = hud.CharacterSheet.PilotSummarySnapshot;
                StringAssert.Contains("TOUR [###----] 3/7", summary);
                StringAssert.Contains("[✓] 1. Co-op Charter", summary);
                StringAssert.Contains("[○] 4. Return Load", summary);
                StringAssert.Contains("NEXT GUIDED (4/7):", summary);
                StringAssert.Contains("Jonas Achterberg-Nkemelu (Receiving Counter)", summary);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void SolarSystemMap_HighlightsBetweenStepsNextGuidedPortAndGroundButtonForStep6()
        {
            var rootGo = new UnityEngine.GameObject("SolarMapGuidedNextAndGroundTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                var ceresFrame = TacSim.Space.TacSimSolarChartFrame.FromJson(JsonNode.Parse(@"{
                    ""chart"": {
                        ""date"": ""2142-05-01"",
                        ""vantage"": { ""port"": ""Ceres Freeport"", ""body"": ""ceres"" },
                        ""bodies"": [
                            { ""id"": ""ceres"", ""label"": ""Ceres"", ""glyph"": ""C"", ""radius_au"": 2.77, ""lon_deg"": 120.0, ""here"": true },
                            { ""id"": ""mars"", ""label"": ""Mars"", ""glyph"": ""M"", ""radius_au"": 1.52, ""lon_deg"": 40.0, ""here"": false }
                        ],
                        ""routes"": [
                            { ""to"": ""stickney"", ""venue"": ""stickney"", ""body"": ""mars"", ""legs"": [{ ""from"": ""ceres"", ""to"": ""mars"" }] }
                        ]
                    }
                }"));
                hud.SolarMap.ApplyChartFrame(ceresFrame);

                // Between Steps 3 & 4 at Ceres -> Solar Map highlights Stickney Yards for Jonas
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" }
                    ],
                    ""active"": [],
                    ""available"": []
                }")));
                Assert.AreEqual("stickney", hud.SolarMap.QuestDestinationPortId);
                Assert.IsTrue(hud.SolarMap.LastOrreryHasQuestDestinationRing);
                Assert.IsTrue(hud.SolarMap.FocusQuestDestinationPort());
                Assert.AreEqual("stickney", hud.SolarMap.SelectedPortId);

                // Once already reading sky chart at Stickney Yards -> clears off-station destination highlight
                var stickneyFrame = TacSim.Space.TacSimSolarChartFrame.FromJson(JsonNode.Parse(@"{
                    ""chart"": {
                        ""date"": ""2142-05-02"",
                        ""vantage"": { ""port"": ""Stickney Yards"", ""body"": ""mars"" },
                        ""bodies"": [
                            { ""id"": ""ceres"", ""label"": ""Ceres"", ""glyph"": ""C"", ""radius_au"": 2.77, ""lon_deg"": 120.0, ""here"": false },
                            { ""id"": ""mars"", ""label"": ""Mars"", ""glyph"": ""M"", ""radius_au"": 1.52, ""lon_deg"": 40.0, ""here"": true }
                        ],
                        ""routes"": [
                            { ""to"": ""ceres"", ""venue"": ""ceres"", ""body"": ""ceres"", ""legs"": [{ ""from"": ""mars"", ""to"": ""ceres"" }] }
                        ]
                    }
                }"));
                hud.SolarMap.ApplyChartFrame(stickneyFrame);
                Assert.AreEqual(string.Empty, hud.SolarMap.QuestDestinationPortId);

                // Active Step 6 surface mining -> highlights GROUND [G]: CARBON button
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                        ]
                    }],
                    ""available"": [],
                    ""completed"": []
                }")));
                Assert.IsTrue(hud.SolarMap.GroundChartQuestHighlighted);
                Assert.AreEqual("GROUND [G]: CARBON", hud.SolarMap.GroundChartButtonLabel);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void CommandPalette_SurfacesBetweenStepsNextGuidedOfferRouteSailAndJournalShortcut()
        {
            var rootGo = new UnityEngine.GameObject("CommandPaletteNextGuidedTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                string prefilled = string.Empty;
                string plottedRoom = string.Empty;
                string sentCmd = string.Empty;
                hud.CommandPalette.OnPrefillCommandRequested += c => prefilled = c;
                hud.CommandPalette.OnPlotStationRoomRequested += r => plottedRoom = r;
                hud.CommandPalette.OnCommandRequested += c => sentCmd = c;

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" }
                    ],
                    ""active"": [],
                    ""available"": []
                }")));
                hud.QuestJournal.SetTrackerRouteBreadcrumb("Berth Gallery", "+ship/board", 1);

                hud.CommandPalette.SetOpen(true);

                // 1. Route entry executes +ship/board
                hud.CommandPalette.SetQuery("board ship");
                StringAssert.Contains("Board Ship", hud.CommandPalette.SelectedEntryTitle);
                hud.CommandPalette.ExecuteSelected();
                Assert.AreEqual("+ship/board", sentCmd);

                // 2. Sail entry prefills +ship/plan stickney
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("sail stickney");
                StringAssert.Contains("Sail -> Stickney Yards", hud.CommandPalette.SelectedEntryTitle);
                hud.CommandPalette.ExecuteSelected();
                Assert.AreEqual("+ship/plan stickney", prefilled);

                // 3. Wayfinder trail entry plots room.p07_receiving
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("wayfinder next guided");
                StringAssert.Contains("Wayfinder to Next Guided (4/7)", hud.CommandPalette.SelectedEntryTitle);
                hud.CommandPalette.ExecuteSelected();
                Assert.AreEqual("room.p07_receiving", plottedRoom);

                // 4. Offer entry opens [J] Quest Journal on Quests tab
                hud.CommandPalette.SetOpen(true);
                hud.CommandPalette.SetQuery("next guided offer");
                StringAssert.Contains("Next Guided (4/7): Return Load", hud.CommandPalette.SelectedEntryTitle);
                hud.CommandPalette.ExecuteSelected();
                Assert.IsTrue(hud.QuestJournal.IsOpen);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void SailingPlanTab_AutoSelectsBetweenStepsNextGuidedOfferPortAndShowsPreflightUntilAtPort()
        {
            var rootGo = new UnityEngine.GameObject("SailingNextGuidedTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var trade = new TacSim.World.TacSimTradeAndShipState();
                hud.BindTradeState(trade);

                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 0, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 12000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                trade.SelectedDestinationPort = "ceres";

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" }
                    ],
                    ""active"": [],
                    ""available"": []
                }")));

                hud.OpenTradeAndSailingModal("SAILING PLAN", openSailingTab: true);
                Assert.AreEqual("stickney", trade.SelectedDestinationPort);
                Assert.IsTrue(hud.IsSailingQuestBannerVisible);
                StringAssert.Contains("STICKNEY YARDS", hud.SailingQuestBannerText);
                StringAssert.Contains("[PRE-FLIGHT OK]", hud.SailingPreflightSummarySnapshot);

                // Once already at Stickney Yards -> off-station Next Guided sailing banner clears
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""stickney"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""cargo_l"": 0, ""cargo_limit_l"": 120000,
                            ""tank_kg"": 12000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                hud.RefreshTradeTerminal();
                Assert.IsFalse(hud.IsSailingQuestBannerVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestJournalAndDialog_ShowOptionalRestartableAndCompletedTourMastery()
        {
            var rootGo = new UnityEngine.GameObject("QuestCompletedAndRestartTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#15"",
                    ""room_name"": ""Registry & Co-op Desk"",
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""title"": ""First Papers at the Registry"",
                        ""giver_name"": ""Maren Halvorsen"",
                        ""turn_in_name"": ""Ilse Oduya-Brandt"",
                        ""allow_restart"": true,
                        ""reward_summary"": ""Guided Tour Step 1/7"",
                        ""next_step"": ""Enroll with Ilse""
                    }],
                    ""available"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                        ""title"": ""Stock the Locker and Tanks"",
                        ""giver_name"": ""Ilse Oduya-Brandt"",
                        ""turn_in_name"": ""Ilse Oduya-Brandt"",
                        ""allow_restart"": true,
                        ""reward_summary"": ""Guided Tour Step 2/7""
                    }],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"", ""turn_in_name"": ""Ilse"", ""completion"": ""Done 1."", ""reward_summary"": ""Step 1 Reward"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"", ""turn_in_name"": ""Ilse"", ""completion"": ""Done 2."", ""reward_summary"": ""Step 2 Reward"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"", ""turn_in_name"": ""Jonas"", ""completion"": ""Done 3."", ""reward_summary"": ""Step 3 Reward"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_04_return_haul"", ""title"": ""S4"", ""turn_in_name"": ""Ilse"", ""completion"": ""Done 4."", ""reward_summary"": ""Step 4 Reward"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"", ""title"": ""S5"", ""turn_in_name"": ""Rosalind"", ""completion"": ""Done 5."", ""reward_summary"": ""Step 5 Reward"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"", ""title"": ""S6"", ""turn_in_name"": ""Rosalind"", ""completion"": ""Done 6."", ""reward_summary"": ""Step 6 Reward"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore"", ""title"": ""S7"", ""turn_in_name"": ""Wren"", ""completion"": ""Done 7."", ""reward_summary"": ""Step 7 Reward"" }
                    ]
                }")));

                // Active & Available Journal sub-tabs + NPC Dialog display Optional · Restartable
                hud.QuestJournal.SetOpen(true, TacSim.UI.QuestJournalTab.Quests);
                hud.QuestJournal.SelectQuestSubTab("active");
                StringAssert.Contains("Optional · Restartable", hud.QuestJournal.JournalBodySnapshot);

                hud.QuestJournal.SelectQuestSubTab("available");
                StringAssert.Contains("Optional · Restartable", hud.QuestJournal.JournalBodySnapshot);

                hud.QuestJournal.OpenQuestDialogByKey("tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll");
                StringAssert.Contains("Optional · Restartable", hud.QuestJournal.DialogBodySnapshot);

                hud.QuestJournal.OpenQuestDialogByKey("tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel");
                StringAssert.Contains("Optional · Restartable", hud.QuestJournal.DialogBodySnapshot);

                // Completed sub-tab displays 7/7 Corridor Captain banner, [Step N/7], and REWARD
                hud.QuestJournal.SelectQuestSubTab("completed");
                string doneBody = hud.QuestJournal.JournalBodySnapshot;
                StringAssert.Contains("GUIDED CORRIDOR TOUR COMPLETE (7/7) — CORRIDOR CAPTAIN", doneBody);
                StringAssert.Contains("[Step 1/7]", doneBody);
                StringAssert.Contains("REWARD: Step 1 Reward", doneBody);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void NpcTalkDetailModal_InjectsQuestDialogAndDirectAcceptOrCompleteButtons()
        {
            var rootGo = new UnityEngine.GameObject("NpcTalkQuestButtonsTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                string sentCmd = string.Empty;
                hud.OnCommandSubmitted += c => sentCmd = c;

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_markers"": [{
                        ""npc_id"": ""#18"",
                        ""npc_name"": ""Maren Halvorsen"",
                        ""marker"": ""available"",
                        ""symbol"": ""!"",
                        ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""quest_title"": ""First Papers at the Registry""
                    }],
                    ""available"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""title"": ""First Papers at the Registry"",
                        ""giver_id"": ""#18"",
                        ""giver_name"": ""Maren Halvorsen"",
                        ""turn_in_id"": ""#19"",
                        ""turn_in_name"": ""Ilse Oduya-Brandt"",
                        ""accept_cmd"": ""+quest/accept #18=tok1""
                    }],
                    ""active"": [],
                    ""completed"": []
                }")));

                hud.ShowDetailModal("MAREN HALVORSEN", "Port Watch clerk", npcName: "Maren Halvorsen");
                Assert.IsTrue(hud.TriggerDetailModalButtonContaining("[!] ACCEPT NOW"));
                Assert.AreEqual("+quest/accept #18=tok1", sentCmd);

                // Re-open Talk modal and hand off directly to NPC Quest Dialog
                hud.ShowDetailModal("MAREN HALVORSEN", "Port Watch clerk", npcName: "Maren Halvorsen");
                Assert.IsTrue(hud.TriggerDetailModalButtonContaining("[!] QUEST DIALOG: First Papers at the Registry"));
                Assert.IsTrue(hud.QuestJournal.IsQuestDialogOpen);
                Assert.AreEqual("#18", hud.QuestJournal.DialogNpcId);

                // Ready-to-finish active quest on Ilse -> shows [?] TURN IN QUEST + [?] COMPLETE NOW
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_markers"": [{
                        ""npc_id"": ""#19"",
                        ""npc_name"": ""Ilse Oduya-Brandt"",
                        ""marker"": ""turn_in"",
                        ""symbol"": ""?"",
                        ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""quest_title"": ""First Papers at the Registry""
                    }],
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""title"": ""First Papers at the Registry"",
                        ""giver_id"": ""#18"",
                        ""giver_name"": ""Maren Halvorsen"",
                        ""turn_in_id"": ""#19"",
                        ""turn_in_name"": ""Ilse Oduya-Brandt"",
                        ""ready_to_finish"": true,
                        ""can_finish_now"": true,
                        ""finish_cmd"": ""+quest/finish #19=tok2""
                    }],
                    ""available"": [],
                    ""completed"": []
                }")));
                hud.ShowDetailModal("ILSE ODUYA-BRANDT", "Co-op Registrar", npcName: "Ilse Oduya-Brandt");
                Assert.IsTrue(hud.TriggerDetailModalButtonContaining("[?] COMPLETE NOW"));
                Assert.AreEqual("+quest/finish #19=tok2", sentCmd);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void OrbitalOpsBar_ShowsBetweenStepsNextGuidedDestinationAndStep6GroundChartAction()
        {
            var rootGo = new UnityEngine.GameObject("OrbitalOpsBarQuestTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var trade = new TacSim.World.TacSimTradeAndShipState();
                hud.BindTradeState(trade);
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{ ""enabled"": true, ""venue"": ""ceres"" }"));

                string orbitalCmd = string.Empty;
                bool groundOpened = false;
                hud.Overlays.OnCommandRequested += c => orbitalCmd = c;
                hud.Overlays.OnOpenGroundChartRequested += () => groundOpened = true;
                hud.Overlays.SetHelmSeatedVisible(true);

                // Between Steps 3 & 4 at Ceres -> Orbital Ops Bar shows Stickney destination & 1-click +ship/plan stickney
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" }
                    ],
                    ""active"": [],
                    ""available"": []
                }")));
                Assert.IsTrue(hud.Overlays.IsOrbitalQuestStripVisible);
                StringAssert.Contains("STICKNEY YARDS", hud.Overlays.OrbitalQuestBannerSnapshot);
                Assert.AreEqual("PLAN: STICKNEY", hud.Overlays.OrbitalQuestButtonLabel);
                Assert.IsTrue(hud.Overlays.TriggerOrbitalQuestActionForTests());
                Assert.AreEqual("+ship/plan stickney", orbitalCmd);

                // Step 6 surface mining -> Orbital Ops Bar shows carbon_ore & GROUND [G]: CARBON shortcut
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                        ]
                    }],
                    ""available"": [],
                    ""completed"": []
                }")));
                Assert.IsTrue(hud.Overlays.IsOrbitalQuestStripVisible);
                StringAssert.Contains("carbon_ore", hud.Overlays.OrbitalQuestBannerSnapshot);
                Assert.AreEqual("GROUND [G]: CARBON", hud.Overlays.OrbitalQuestButtonLabel);
                Assert.IsTrue(hud.Overlays.TriggerOrbitalQuestActionForTests());
                Assert.IsTrue(groundOpened);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ExchangeTerminal_AdvancesStep2DualItemByStowageAndVenueAndNarrowsLoadedStep3ToSellBids()
        {
            var rootGo = new UnityEngine.GameObject("ExchangeDualItemQuestTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var trade = new TacSim.World.TacSimTradeAndShipState();
                hud.BindTradeState(trade);

                // Step 2 active: 10 provisions + 5 propellant
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                        ""title"": ""Stock the Locker and Tanks"",
                        ""checkpoints"": [
                            { ""id"": ""buy_provisions"", ""title"": ""Buy at least 10 provisions at Ceres Co-op Chandlery"", ""instructions"": ""Buy 10 provisions."", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""buy_propellant"", ""title"": ""Buy at least 5 tonnes of propellant at Ceres Exchange Hall"", ""instructions"": ""Buy 5 propellant."", ""checked"": false, ""live_ready"": false }
                        ]
                    }],
                    ""available"": [],
                    ""completed"": []
                }")));

                // 1. At Exchange Hall with only propellant listed and 0 provisions -> picks propellant (5)
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": { ""hull_id"": ""#99"", ""stowage"": { ""stores_days"": 0, ""stores_limit_days"": 120, ""reaction_kg"": 0, ""max_reaction_kg"": 80000 } },
                    ""listings"": [
                        { ""id"": 12, ""side"": ""ask"", ""good"": ""tacsim:propellant"", ""good_label"": ""Propellant"", ""available_qty"": 40, ""unit_price_minor"": 1800, ""price_text"": ""18.00 cr"", ""funded"": true, ""vendor_name"": ""Exchange"" }
                    ]
                }"));
                hud.OpenTradeAndSailingModal("EXCHANGE", openSailingTab: false);
                StringAssert.Contains("PROPELLANT", hud.ExchangeQuestBannerText);
                Assert.AreEqual(1, hud.HighlightedQuestListingCount);
                Assert.IsTrue(hud.ApplyQuestTradeQuantity());
                Assert.AreEqual(5, trade.SelectedTradeQuantity);

                // 2. Both provisions and propellant listed, with 10 provisions already stowed -> advances to propellant & shows [OK] on provisions
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": { ""hull_id"": ""#99"", ""stowage"": { ""stores_days"": 10, ""stores_limit_days"": 120, ""reaction_kg"": 0, ""max_reaction_kg"": 80000 } },
                    ""listings"": [
                        { ""id"": 11, ""side"": ""ask"", ""good"": ""tacsim:provisions"", ""good_label"": ""Provisions"", ""available_qty"": 25, ""unit_price_minor"": 1000, ""price_text"": ""10.00 cr"", ""funded"": true, ""vendor_name"": ""Chandlery"" },
                        { ""id"": 12, ""side"": ""ask"", ""good"": ""tacsim:propellant"", ""good_label"": ""Propellant"", ""available_qty"": 40, ""unit_price_minor"": 1800, ""price_text"": ""18.00 cr"", ""funded"": true, ""vendor_name"": ""Exchange"" }
                    ]
                }"));
                hud.RefreshTradeTerminal();
                StringAssert.Contains("PROPELLANT", hud.ExchangeQuestBannerText);
                StringAssert.Contains("10 PROVISIONS [OK]", hud.ExchangeQuestBannerText);
                Assert.AreEqual(1, hud.HighlightedQuestListingCount);
                Assert.IsTrue(hud.QuestJournal.TryGetQuestHoldReadinessSummary(hud.QuestJournal.GetTrackedActiveQuest(), out string holdSummary, out bool holdReady));
                Assert.IsFalse(holdReady);
                StringAssert.Contains("[HOLD] PROPELLANT: 0/5 t", holdSummary);
                StringAssert.Contains("PROVISIONS 10/10d OK", holdSummary);

                // 3. Step 3 with 5t metals loaded in hold -> narrows from BUY/SELL to SELL and highlights Standing Bid only
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""checkpoints"": [
                            { ""id"": ""deliver_metals"", ""title"": ""Buy 5 tonnes of metals at Ceres and deliver them to Stickney Yards"", ""instructions"": ""Buy 5 metals and sell with +market/sell."", ""checked"": false, ""live_ready"": false }
                        ]
                    }],
                    ""available"": [],
                    ""completed"": []
                }")));
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""stickney"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 5000,
                            ""max_cargo_kg"": 100000,
                            ""reaction_kg"": 10000,
                            ""max_reaction_kg"": 80000,
                            ""stores_days"": 10,
                            ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""quantity"": 5 }]
                        }
                    },
                    ""listings"": [
                        { ""id"": 21, ""side"": ""ask"", ""good"": ""tacsim:metals"", ""good_label"": ""Metals"", ""available_qty"": 10, ""unit_price_minor"": 10000, ""price_text"": ""100.00 cr"", ""funded"": true, ""vendor_name"": ""Ask Desk"" },
                        { ""id"": 22, ""side"": ""bid"", ""good"": ""tacsim:metals"", ""good_label"": ""Metals"", ""available_qty"": 20, ""unit_price_minor"": 14500, ""price_text"": ""145.00 cr"", ""funded"": true, ""vendor_name"": ""Kasei Receiving"" }
                    ]
                }"));
                hud.RefreshTradeTerminal();
                StringAssert.Contains("SELL <b>5</b>", hud.ExchangeQuestBannerText);
                StringAssert.DoesNotContain("BUY / SELL", hud.ExchangeQuestBannerText);
                Assert.AreEqual(1, hud.HighlightedQuestListingCount, "Only Standing Bid must be highlighted once quest cargo is loaded in hold");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void ShipMinimapAndOpsDirectory_HighlightQuestTargetCompartmentAndShowBetweenStepsNextGuidedOffer()
        {
            var rootGo = new UnityEngine.GameObject("ShipMinimapAndOpsQuestTestRoot");
            try
            {
                var shipGo = new UnityEngine.GameObject("ShipInterior");
                shipGo.transform.SetParent(rootGo.transform, false);
                var ship = shipGo.AddComponent<ShipInteriorController>();
                ship.BuildInterior(new TacSimAssetCatalog(), "tacsim.unit.dray");

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Minimap.BindWorldSources(null, ship, null, null, null);
                hud.OpsDirectory.BindSources(null, ship, null, null, null, null, null);

                // Between Steps 3 & 4 aboard ship -> Bridge highlighted on Ship Minimap and Deck Directory,
                // and Active Ops tab displays Next Guided Offer (Step 4/7 Jonas at Receiving Counter).
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""S1"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"", ""title"": ""S2"" },
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"", ""title"": ""S3"" }
                    ],
                    ""active"": [],
                    ""available"": []
                }")));

                hud.Minimap.SetQuestShipCompartment("bridge");
                hud.Minimap.SyncMinimap(TacSimActiveStage.ShipInteriorWalk, UnityEngine.Vector3.zero, 0f, false, forceRedraw: true);
                Assert.IsTrue(hud.Minimap.LastShipMinimapHasQuestBeacon);
                StringAssert.Contains("BRIDGE!", hud.Minimap.HeaderText);

                hud.OpsDirectory.UpdateLiveRoom(new MushRoomView { Id = "#201", Name = "Airlock" }, TacSimActiveStage.ShipInteriorWalk);
                hud.OpsDirectory.SetOpen(true);
                Assert.GreaterOrEqual(hud.OpsDirectory.HighlightedQuestDirectoryRowIndex, 0);
                StringAssert.Contains("[QUEST !]", hud.OpsDirectory.GetDirectoryRowLabel(hud.OpsDirectory.HighlightedQuestDirectoryRowIndex));

                hud.OpsDirectory.SelectTab(TacSim.UI.OpsDirectoryTab.ActiveOps);
                StringAssert.Contains("NEXT GUIDED STEP", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("Step 4/7: Return Load", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("Jonas Achterberg-Nkemelu", hud.OpsDirectory.OpsSummarySnapshot);
                StringAssert.Contains("Receiving Counter", hud.OpsDirectory.OpsSummarySnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void SurfaceRoverHud_HighlightsActiveStep6ActionWashFamilyAndPostMineReturnButton()
        {
            var rootGo = new UnityEngine.GameObject("SurfaceRoverHudStep6TestRoot");
            try
            {
                var surfGo = new UnityEngine.GameObject("SurfaceStage");
                surfGo.transform.SetParent(rootGo.transform, false);
                var surface = surfGo.AddComponent<TacSim.Surface.SurfaceMiningController>();

                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.Overlays.BindControllers(surface, null, null);
                surface.EnterSurfaceMode();

                string emitted = string.Empty;
                hud.Overlays.OnCommandRequested += c => emitted = c;

                // 1. Scan pending -> SCAN highlighted, Return button hidden
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""sound_mineral_pocket"", ""title"": ""Sound pocket"", ""instructions"": ""+tv/1 sound go"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));
                Assert.AreEqual("scan", hud.Overlays.HighlightedSurfaceQuestStepForTests);
                Assert.IsFalse(hud.Overlays.IsSurfaceReturnPortButtonVisible);

                // 2. Scan done, Sound pending -> SOUND highlighted
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""sound_mineral_pocket"", ""title"": ""Sound pocket"", ""instructions"": ""+tv/1 sound go"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));
                Assert.AreEqual("sound", hud.Overlays.HighlightedSurfaceQuestStepForTests);

                // 3. Scan+Sound done, Mine pending -> MINE highlighted
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""sound_mineral_pocket"", ""title"": ""Sound pocket"", ""instructions"": ""+tv/1 sound go"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));
                Assert.AreEqual("mine", hud.Overlays.HighlightedSurfaceQuestStepForTests);

                // 4. Mine done -> RECOVER highlighted & 1-click RETURN: CERES button visible (+ship/plan ceres)
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""ready_to_finish"": true,
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""sound_mineral_pocket"", ""title"": ""Sound pocket"", ""instructions"": ""+tv/1 sound go"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": true, ""live_ready"": true }
                        ]
                    }]
                }")));
                Assert.AreEqual("recover", hud.Overlays.HighlightedSurfaceQuestStepForTests);
                Assert.IsTrue(hud.Overlays.IsSurfaceReturnPortButtonVisible);
                Assert.AreEqual("RETURN: CERES", hud.Overlays.SurfaceReturnPortButtonLabel);
                StringAssert.Contains("Ceres Freeport", hud.Overlays.SurfaceReadingsTextForTests);
                Assert.IsTrue(hud.Overlays.TriggerSurfaceReturnPortForTests());
                Assert.AreEqual("+ship/plan ceres", emitted);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestTrackerCard_ReactivelyUpdatesStep2MultiItemHoldAndStep5RefitReadinessOnTradeStateChange()
        {
            var rootGo = new UnityEngine.GameObject("TrackerLiveHoldTestRoot");
            try
            {
                var trade = new TacSimTradeAndShipState();
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.BindTradeState(trade);

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""title"": ""Stock the Locker and Tanks"",
                            ""next_step"": ""Buy 5t propellant at Exchange"",
                            ""checkpoints"": [
                                { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""instructions"": ""Buy 10 provisions."", ""checked"": false, ""live_ready"": false },
                                { ""id"": ""buy_propellant"", ""title"": ""Buy 5t propellant"", ""instructions"": ""Buy 5 tonnes of propellant."", ""checked"": false, ""live_ready"": false }
                            ]
                        },
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"",
                            ""title"": ""Refit the Dray for Surface Mining"",
                            ""next_step"": ""Run +coop/refit prospector"",
                            ""checkpoints"": [
                                { ""id"": ""refit_dray_p"", ""title"": ""Refit Dray into Dray-P"", ""instructions"": ""Empty hold and run +coop/refit prospector."", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ]
                }")));

                // 1. Step 2 with 10d provisions + 2t propellant -> shows PROPELLANT 2/5 t | PROVISIONS 10/10d OK
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 2000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                StringAssert.Contains("[HOLD] PROPELLANT: 2/5 t | PROVISIONS 10/10d OK", hud.QuestJournal.TrackerDetailSnapshot);

                // 2. Top off propellant to 5t -> reactively flips to [HOLD READY] without re-applying quest panel
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 5000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                StringAssert.Contains("[HOLD READY]", hud.QuestJournal.TrackerDetailSnapshot);
                StringAssert.Contains("PROPELLANT 5/5t OK", hud.QuestJournal.TrackerDetailSnapshot);

                // 3. Switch tracked quest to Step 5 refit with stores still loaded -> [REFIT CHECK], then empty -> [READY]
                hud.QuestJournal.TrackQuest("tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector");
                StringAssert.Contains("[REFIT CHECK] Cargo empty | Stores 10d | Tank 5t OK", hud.QuestJournal.TrackerDetailSnapshot);

                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true,
                    ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 5000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 0, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                StringAssert.Contains("[READY] Cargo empty | Stores empty | Tank 5t OK", hud.QuestJournal.TrackerDetailSnapshot);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void TopBarQuestBadgeAndRoomBanner_PrioritizeTurnInOverOfferAndOpenNpcQuestDialogOnClick()
        {
            var rootGo = new UnityEngine.GameObject("TopBarQuestBadgeTestRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                Assert.AreEqual("[J] LOG", hud.QuestTopButtonLabelForTests);
                Assert.IsFalse(hud.IsRoomQuestBannerVisible);

                // 1. Enter room with an available quest offer -> [J] ! and clickable gold room banner
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_markers"": [{
                        ""npc_id"": ""#11"",
                        ""npc_name"": ""Maren Halvorsen"",
                        ""marker"": ""available"",
                        ""symbol"": ""!"",
                        ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""quest_title"": ""First Papers""
                    }],
                    ""available"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""title"": ""First Papers"",
                        ""giver_id"": ""#11"",
                        ""giver_name"": ""Maren Halvorsen"",
                        ""accept_cmd"": ""+quest/accept #11 tok1""
                    }]
                }")));
                Assert.AreEqual("[J] !", hud.QuestTopButtonLabelForTests);
                Assert.IsTrue(hud.IsRoomQuestBannerVisible);
                StringAssert.Contains("[!] Maren Halvorsen", hud.RoomQuestBannerLabelForTests);
                StringAssert.Contains("First Papers", hud.RoomQuestBannerLabelForTests);

                Assert.IsTrue(hud.TriggerRoomQuestBannerForTests());
                Assert.IsTrue(hud.QuestJournal.IsQuestDialogOpen);
                Assert.AreEqual("tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", hud.QuestJournal.DialogQuestKey);
                hud.QuestJournal.CloseQuestDialog();

                // 2. Room with BOTH an offer and a turn-in -> prioritizes turn-in (?) over offer (!)
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_markers"": [
                        {
                            ""npc_id"": ""#11"",
                            ""npc_name"": ""Maren Halvorsen"",
                            ""marker"": ""available"",
                            ""symbol"": ""!"",
                            ""quest_key"": ""q.side"",
                            ""quest_title"": ""Side Job""
                        },
                        {
                            ""npc_id"": ""#18"",
                            ""npc_name"": ""Ilse Oduya-Brandt"",
                            ""marker"": ""turn_in"",
                            ""symbol"": ""?"",
                            ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                            ""quest_title"": ""First Papers""
                        }
                    ],
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""title"": ""First Papers"",
                        ""turn_in_id"": ""#18"",
                        ""turn_in_name"": ""Ilse Oduya-Brandt"",
                        ""can_finish_now"": true,
                        ""finish_cmd"": ""+quest/finish #18 tok2""
                    }],
                    ""available"": [{
                        ""key"": ""q.side"",
                        ""title"": ""Side Job"",
                        ""giver_id"": ""#11"",
                        ""giver_name"": ""Maren Halvorsen"",
                        ""accept_cmd"": ""+quest/accept #11 tok3""
                    }]
                }")));
                Assert.AreEqual("[J] ?", hud.QuestTopButtonLabelForTests);
                Assert.IsTrue(hud.IsRoomQuestBannerVisible);
                StringAssert.Contains("[?] Ilse Oduya-Brandt", hud.RoomQuestBannerLabelForTests);
                StringAssert.Contains("TURN IN", hud.RoomQuestBannerLabelForTests);

                Assert.IsTrue(hud.TriggerRoomQuestBannerForTests());
                Assert.IsTrue(hud.QuestJournal.IsQuestDialogOpen);
                Assert.AreEqual("#18", hud.QuestJournal.DialogNpcId);
                hud.QuestJournal.CloseQuestDialog();

                // 3. Leave room while an active quest remains -> [J] Q(1) and room banner hides
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_markers"": [],
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                        ""title"": ""Stock the Locker and Tanks"",
                        ""can_finish_now"": false
                    }],
                    ""available"": []
                }")));
                Assert.AreEqual("[J] Q(1)", hud.QuestTopButtonLabelForTests);
                Assert.IsFalse(hud.IsRoomQuestBannerVisible);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void GuidedAutoTrackAndCycleTracked_IsolatesShipRoutingAndSyncsAllSurfaces()
        {
            var rootGo = new UnityEngine.GameObject("GuidedAutoTrackIsolationRoot");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [
                        {
                            ""key"": ""q.side_deimos"",
                            ""title"": ""Deliver Water Ice to Deimos"",
                            ""category"": ""optional"",
                            ""next_room_entry"": ""room.d02_concourse"",
                            ""checkpoints"": [{
                                ""id"": ""deliver_water_ice"",
                                ""title"": ""Deliver 3t water ice to Voltaire Anchorage"",
                                ""instructions"": ""Run +ship/plan deimos and +market/sell water_ice."",
                                ""room_entry"": ""room.d02_concourse"",
                                ""checked"": false,
                                ""live_ready"": false
                            }]
                        },
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                            ""title"": ""Scan, Sound, and Mine a Landing Site"",
                            ""category"": ""guided"",
                            ""next_room_entry"": ""room.c07_exchange"",
                            ""checkpoints"": [
                                { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": false, ""live_ready"": false },
                                { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                            ]
                        }
                    ]
                }")));

                // Auto-track prefers Guided Step 6 even though side job is at index 0
                Assert.AreEqual("tacsim.quests.guided_onboarding/quest.guide_06_surface_mining", hud.QuestJournal.TrackedQuestKey);
                Assert.IsTrue(hud.QuestJournal.NeedsSurfaceExpeditionBoarding());
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment(null, out string compStep6, settledPad: true));
                Assert.AreEqual("bay", compStep6);

                // Cycling to side Deimos job isolates ship routing & expedition checks (zero Step 6 leakage!)
                Assert.AreEqual("q.side_deimos", hud.QuestJournal.CycleTrackedQuest());
                Assert.IsFalse(hud.QuestJournal.NeedsSurfaceExpeditionBoarding());
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("ceres", out string compBridge, settledPad: false));
                Assert.AreEqual("bridge", compBridge);
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("deimos", out string compAirlock, settledPad: false));
                Assert.AreEqual("airlock", compAirlock);

                // Cycling again returns cleanly to Step 6
                Assert.AreEqual("tacsim.quests.guided_onboarding/quest.guide_06_surface_mining", hud.QuestJournal.CycleTrackedQuest());
                Assert.IsTrue(hud.QuestJournal.NeedsSurfaceExpeditionBoarding());
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void GuidedOnboarding_FullSevenStepWalkthroughFromMarenToCorridorCaptain()
        {
            var rootGo = new UnityEngine.GameObject("GuidedSevenStepWalkthroughRoot");
            try
            {
                var trade = new TacSimTradeAndShipState();
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.BindTradeState(trade);

                string[] keys =
                {
                    "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll",
                    "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel",
                    "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
                    "tacsim.quests.guided_onboarding/quest.guide_04_return_haul",
                    "tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector",
                    "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
                    "tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore"
                };

                string CompletedJson(int count)
                {
                    var parts = new List<string>();
                    for (int i = 0; i < count; i++)
                        parts.Add($"{{\"key\":\"{keys[i]}\",\"title\":\"Step {i + 1}\"}}");
                    return "[" + string.Join(",", parts) + "]";
                }

                // Step 1: Maren offer at Arrivals -> Enroll Co-op -> Between-steps Step 2 locator
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_markers"": [{ ""npc_id"": ""#11"", ""npc_name"": ""Maren Halvorsen"", ""marker"": ""available"", ""symbol"": ""!"", ""quest_key"": """ + keys[0] + @""", ""quest_title"": ""First Papers"" }],
                    ""available"": [{ ""key"": """ + keys[0] + @""", ""title"": ""First Papers"", ""giver_id"": ""#11"", ""giver_name"": ""Maren Halvorsen"", ""accept_cmd"": ""+quest/accept #11 tok1"" }],
                    ""active"": [], ""completed"": []
                }")));
                Assert.AreEqual("[J] !", hud.QuestTopButtonLabelForTests);
                Assert.IsTrue(hud.IsRoomQuestBannerVisible);

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1, ""active"": [], ""available"": [], ""completed"": " + CompletedJson(1) + @"
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetNextGuidedOffer(out var next2));
                Assert.AreEqual(2, next2.Step);

                // Step 2: Provisions + Propellant dual-item hold readiness
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": """ + keys[1] + @""",
                        ""title"": ""Stock the Locker and Tanks"",
                        ""checkpoints"": [
                            { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""instructions"": ""Buy 10 provisions."", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""buy_propellant"", ""title"": ""Buy 5t propellant"", ""instructions"": ""Buy 5t propellant."", ""checked"": false, ""live_ready"": false }
                        ]
                    }],
                    ""completed"": " + CompletedJson(1) + @"
                }")));
                Assert.AreEqual(2, hud.QuestJournal.GetActiveTradeNeeds().Count);

                // Step 3: Buy 5t metals -> Sail Stickney -> Airlock on arrival
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": """ + keys[2] + @""",
                        ""title"": ""Metals Down the Corridor"",
                        ""next_room_entry"": ""room.p07_receiving"",
                        ""checkpoints"": [{ ""id"": ""deliver_metals"", ""title"": ""Deliver 5t metals to Stickney"", ""instructions"": ""Buy 5t metals, +ship/plan stickney, +market/sell."", ""room_entry"": ""room.p07_receiving"", ""checked"": false, ""live_ready"": false }]
                    }],
                    ""completed"": " + CompletedJson(2) + @"
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveDestinationPort(out var d3));
                Assert.AreEqual("stickney", d3.PortId);
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment("stickney", out string c3, settledPad: false));
                Assert.AreEqual("airlock", c3);

                // Step 4: Return 2t industrial_parts to Ceres
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": """ + keys[3] + @""",
                        ""title"": ""A Return Load for Ceres"",
                        ""next_room_entry"": ""room.c08_chandlery"",
                        ""checkpoints"": [{ ""id"": ""return_industrial_parts"", ""title"": ""Return 2t industrial parts to Ceres"", ""instructions"": ""Buy 2t industrial_parts, +ship/plan ceres, +market/sell."", ""room_entry"": ""room.c08_chandlery"", ""checked"": false, ""live_ready"": false }]
                    }],
                    ""completed"": " + CompletedJson(3) + @"
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveDestinationPort(out var d4));
                Assert.AreEqual("ceres", d4.PortId);

                // Step 5: Refit Dray into Dray-P prospector
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": """ + keys[4] + @""",
                        ""title"": ""Refit the Dray for Surface Mining"",
                        ""checkpoints"": [{ ""id"": ""refit_dray_p"", ""title"": ""Refit Dray-P"", ""instructions"": ""+coop/refit prospector"", ""checked"": false, ""live_ready"": false }]
                    }],
                    ""completed"": " + CompletedJson(4) + @"
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetSuggestedCommand(hud.QuestJournal.GetTrackedActiveQuest(), out _, out string refitCmd));
                Assert.AreEqual("+coop/refit prospector", refitCmd);

                // Step 6: Scan, sound, mine carbon_ore on surface pad
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": """ + keys[5] + @""",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""next_room_entry"": ""room.c07_exchange"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""+tv/1 scan go"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""+tv/1 mine go"", ""checked"": false, ""live_ready"": false }
                        ]
                    }],
                    ""completed"": " + CompletedJson(5) + @"
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveSurfaceMiningStatus(out var m6));
                Assert.AreEqual("carbon", m6.RecommendedFamily);
                Assert.IsTrue(hud.QuestJournal.TryGetActiveShipTargetCompartment(null, out string c6, settledPad: true));
                Assert.AreEqual("bay", c6);

                // Step 7: Sell mined carbon_ore at Stickney -> 7/7 Corridor Captain mastery
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": """ + keys[6] + @""",
                        ""title"": ""Deliver Your Mined Ore to Market"",
                        ""next_room_entry"": ""room.p11_service"",
                        ""checkpoints"": [{ ""id"": ""sell_carbon_ore"", ""title"": ""Sell 1t carbon_ore at Stickney"", ""instructions"": ""+ship/plan stickney and +market/sell carbon_ore."", ""room_entry"": ""room.p11_service"", ""checked"": false, ""live_ready"": false }]
                    }],
                    ""completed"": " + CompletedJson(6) + @"
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetActiveDestinationPort(out var d7));
                Assert.AreEqual("stickney", d7.PortId);

                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1, ""active"": [], ""available"": [], ""completed"": " + CompletedJson(7) + @"
                }")));
                Assert.IsTrue(hud.QuestJournal.TryGetGuidedTourProgress(out int done, out int total, out _, out bool allDone));
                Assert.AreEqual(7, done);
                Assert.AreEqual(7, total);
                Assert.IsTrue(allDone);
                Assert.IsFalse(hud.QuestJournal.TryGetNextGuidedOffer(out _));
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void EconomyPanels_AutoSelectsQuestCommodityInHistoryAndShowsStep5RefitBannerInWorksAndYard()
        {
            var rootGo = new UnityEngine.GameObject("EconomyPanelsQuestGuidanceRoot");
            try
            {
                var trade = new TacSimTradeAndShipState();
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                hud.ShowLoginOverlay(false);
                hud.BindTradeState(trade);

                var emitted = new List<string>();
                hud.OnCommandSubmitted += emitted.Add;

                // 1. Step 3 Metals active -> switching to History auto-selects metals and shows quest banner
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Buy 5 tonnes of metals at Ceres and deliver to Stickney"",
                            ""instructions"": ""Buy 5 metals and +market/sell at Stickney."",
                            ""checked"": false, ""live_ready"": false
                        }]
                    }]
                }")));

                hud.OpenTradeAndSailingModal("EXCHANGE", openSailingTab: false);
                emitted.Clear();
                hud.SwitchTradeTerminalTab(TacSim.UI.TradeTerminalTab.History, autoQuery: true);
                Assert.AreEqual("metals", hud.EconomyPanels.SelectedHistoryGood);
                Assert.IsTrue(hud.EconomyPanels.IsHistoryQuestBannerVisible);
                StringAssert.Contains("24-WEEK TREND: METALS", hud.EconomyPanels.HistoryQuestBannerTextForTests);
                CollectionAssert.Contains(emitted, "+market/history tacsim:metals");

                // 2. Step 7 mined carbon_ore -> History queries exact tacsim:carbon_ore
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore"",
                        ""title"": ""Deliver Your Mined Ore to Market"",
                        ""checkpoints"": [{
                            ""id"": ""sell_carbon_ore"",
                            ""title"": ""Sell 1t carbon_ore at Stickney"",
                            ""instructions"": ""Run +market/sell carbon_ore."",
                            ""checked"": false, ""live_ready"": false
                        }]
                    }]
                }")));
                emitted.Clear();
                hud.SwitchTradeTerminalTab(TacSim.UI.TradeTerminalTab.History, autoQuery: true);
                Assert.AreEqual("carbon_ore", hud.EconomyPanels.SelectedHistoryGood);
                CollectionAssert.Contains(emitted, "+market/history tacsim:carbon_ore");

                // 3. Step 5 Dray-P Refit -> Industry (Works & Yard) shows refit banner + live hold check + 1-click refit
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector"",
                        ""title"": ""Refit the Dray for Surface Mining"",
                        ""checkpoints"": [{
                            ""id"": ""refit_dray_p"",
                            ""title"": ""Refit Dray into Dray-P"",
                            ""instructions"": ""Empty cargo & stores and run +coop/refit prospector."",
                            ""checked"": false, ""live_ready"": false
                        }]
                    }]
                }")));
                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true, ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 5000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 6, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                hud.SwitchTradeTerminalTab(TacSim.UI.TradeTerminalTab.Industry, autoQuery: false);
                Assert.IsTrue(hud.EconomyPanels.IsIndustryQuestBannerVisible);
                StringAssert.Contains("[REFIT CHECK]", hud.EconomyPanels.IndustryQuestBannerTextForTests);

                trade.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true, ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 5000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 0, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                StringAssert.Contains("[READY]", hud.EconomyPanels.IndustryQuestBannerTextForTests);

                emitted.Clear();
                Assert.IsTrue(hud.EconomyPanels.TriggerIndustryQuestActionForTests());
                CollectionAssert.Contains(emitted, "+coop/refit prospector");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestJournal_EmitsCheckpointAndAbandonedMilestonesPostHydration()
        {
            var rootGo = new UnityEngine.GameObject("QuestMilestonesTest");
            try
            {
                var journal = rootGo.AddComponent<TacSim.UI.TacSimQuestJournal>();
                journal.BuildUi(rootGo.transform, null);
                var events = new List<(string kind, string label)>();
                journal.OnQuestMilestoneReached += (kind, label) => events.Add((kind, label));

                // 1. Initial hydration must not emit any milestone
                journal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                        ""title"": ""Stock the Locker and Fuel the Tank"",
                        ""allow_restart"": true,
                        ""ready_to_finish"": false,
                        ""checkpoints"": [
                            { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""buy_propellant"", ""title"": ""Buy 5t propellant"", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));
                Assert.AreEqual(0, events.Count, "Initial hydration must suppress milestone spam");

                // 2. Completing a single checkpoint emits 'checkpoint' without 'ready'
                journal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                        ""title"": ""Stock the Locker and Fuel the Tank"",
                        ""allow_restart"": true,
                        ""ready_to_finish"": false,
                        ""checkpoints"": [
                            { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""buy_propellant"", ""title"": ""Buy 5t propellant"", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));
                Assert.AreEqual(1, events.Count);
                Assert.AreEqual("checkpoint", events[0].kind);
                Assert.AreEqual("Buy 10 provisions", events[0].label);

                // 3. Final checkpoint transitioning quest to ready emits 'ready' only (no duplicate checkpoint)
                events.Clear();
                journal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                        ""title"": ""Stock the Locker and Fuel the Tank"",
                        ""allow_restart"": true,
                        ""ready_to_finish"": true,
                        ""checkpoints"": [
                            { ""id"": ""buy_provisions"", ""title"": ""Buy 10 provisions"", ""checked"": true, ""live_ready"": true },
                            { ""id"": ""buy_propellant"", ""title"": ""Buy 5t propellant"", ""checked"": true, ""live_ready"": true }
                        ]
                    }]
                }")));
                Assert.AreEqual(1, events.Count);
                Assert.AreEqual("ready", events[0].kind);

                // 4. Abandoning a restartable quest emits 'abandoned' with restartable cue
                events.Clear();
                journal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [],
                    ""completed"": []
                }")));
                Assert.AreEqual(1, events.Count);
                Assert.AreEqual("abandoned", events[0].kind);
                StringAssert.Contains("Stock the Locker and Fuel the Tank", events[0].label);
                StringAssert.Contains("restartable", events[0].label);

                // 5. ResetSessionState suppresses re-hydration milestones
                events.Clear();
                journal.ResetSessionState();
                journal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                        ""title"": ""Enroll in the Freeport Co-op"",
                        ""ready_to_finish"": false
                    }]
                }")));
                Assert.AreEqual(0, events.Count, "Post-reset hydration must stay silent");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void QuestJournal_PrioritizesNpcTurnInCyclesDialogAndConfirmsPermanentDecline()
        {
            var rootGo = new UnityEngine.GameObject("QuestDialogPickerDeclineTest");
            try
            {
                var journal = rootGo.AddComponent<TacSim.UI.TacSimQuestJournal>();
                journal.BuildUi(rootGo.transform, null);
                var emitted = new List<string>();
                journal.OnCommandRequested += cmd => emitted.Add(cmd);

                journal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_id"": ""#15"",
                    ""room_name"": ""Registry & Co-op Desk"",
                    ""room_markers"": [
                        {
                            ""npc_id"": ""#18"", ""npc_name"": ""Ilse Oduya-Brandt"",
                            ""marker"": ""available"", ""symbol"": ""!"",
                            ""quest_key"": ""side.one_shot"", ""quest_title"": ""One-Shot Audit""
                        },
                        {
                            ""npc_id"": ""#18"", ""npc_name"": ""Ilse Oduya-Brandt"",
                            ""marker"": ""turn_in"", ""symbol"": ""?"",
                            ""quest_key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""quest_title"": ""First Papers at the Registry""
                        }
                    ],
                    ""active"": [
                        {
                            ""key"": ""side.in_progress"", ""title"": ""In-Progress Favor"",
                            ""giver_id"": ""#18"", ""giver_name"": ""Ilse Oduya-Brandt"",
                            ""turn_in_id"": ""#18"", ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""ready_to_finish"": false, ""can_finish_now"": false,
                            ""abandon_cmd"": ""+quest/abandon side.in_progress""
                        },
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"",
                            ""title"": ""First Papers at the Registry"",
                            ""giver_id"": ""#11"", ""giver_name"": ""Maren Halvorsen"",
                            ""turn_in_id"": ""#18"", ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""ready_to_finish"": true, ""can_finish_now"": true,
                            ""finish_cmd"": ""+quest/finish #18 tok1"",
                            ""abandon_cmd"": ""+quest/abandon q1""
                        }
                    ],
                    ""available"": [
                        {
                            ""key"": ""tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel"",
                            ""title"": ""Stock the Locker and Tanks"",
                            ""giver_id"": ""#18"", ""giver_name"": ""Ilse Oduya-Brandt"",
                            ""turn_in_id"": ""#18"", ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""allow_restart"": true,
                            ""accept_cmd"": ""+quest/accept #18 tok_g2"",
                            ""decline_cmd"": ""+quest/decline #18 tok_g2""
                        },
                        {
                            ""key"": ""side.one_shot"",
                            ""title"": ""One-Shot Audit"",
                            ""giver_id"": ""#18"", ""giver_name"": ""Ilse Oduya-Brandt"",
                            ""turn_in_id"": ""#18"", ""turn_in_name"": ""Ilse Oduya-Brandt"",
                            ""allow_restart"": false,
                            ""accept_cmd"": ""+quest/accept #18 tok_side"",
                            ""decline_cmd"": ""+quest/decline #18 tok_side""
                        }
                    ]
                }")));

                // 1. Opening Ilse's NPC dialog picks turn-in [?] FIRST even though available [!] appeared earlier
                Assert.IsTrue(journal.OpenNpcQuestDialog("#18"));
                Assert.AreEqual("tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", journal.DialogQuestKey);
                Assert.AreEqual("QUEST 1/4", journal.DialogCycleLabelSnapshot);

                // 2. Cycle to restartable guided offer -> permanent decline button stays hidden
                journal.CycleNpcDialogQuest();
                Assert.AreEqual("tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", journal.DialogQuestKey);
                Assert.AreEqual("QUEST 2/4", journal.DialogCycleLabelSnapshot);
                Assert.IsFalse(journal.IsDialogAbandonVisible, "Restartable guided offer must not expose permanent decline");

                // 3. Cycle to non-restartable offer -> 2-click DECLINE -> CONFIRM DECLINE emits +quest/decline
                journal.CycleNpcDialogQuest();
                Assert.AreEqual("side.one_shot", journal.DialogQuestKey);
                Assert.AreEqual("QUEST 3/4", journal.DialogCycleLabelSnapshot);
                Assert.IsTrue(journal.IsDialogAbandonVisible);
                Assert.AreEqual("DECLINE", journal.DialogAbandonLabelSnapshot);

                Assert.IsFalse(journal.AbandonDialogQuest(), "First click arms confirmation without submitting");
                Assert.AreEqual("CONFIRM DECLINE", journal.DialogAbandonLabelSnapshot);
                Assert.AreEqual(0, emitted.Count);

                // Cycling away and back resets confirmation
                journal.CycleNpcDialogQuest();
                Assert.AreEqual("side.in_progress", journal.DialogQuestKey);
                journal.OpenQuestDialogByKey("side.one_shot");
                Assert.AreEqual("DECLINE", journal.DialogAbandonLabelSnapshot);

                Assert.IsFalse(journal.AbandonDialogQuest());
                Assert.IsTrue(journal.AbandonDialogQuest());
                CollectionAssert.Contains(emitted, "+quest/decline #18 tok_side");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void HudController_JobsTabCrossLinksCorridorHaulAndSurfaceSurveyQuests()
        {
            var rootGo = new UnityEngine.GameObject("HudJobsQuestCrossLinkTest");
            try
            {
                var hud = rootGo.AddComponent<TacSim.UI.TacSimHudController>();
                hud.BuildHud();
                var emitted = new List<string>();
                hud.OnCommandSubmitted += cmd => emitted.Add(cmd);

                // Clean state: Jobs quest banner stays hidden and default haul good stays volatiles
                hud.OpenTradeAndSailingModal("EXCHANGE", openSailingTab: false);
                hud.SwitchTradeTerminalTab(TacSim.UI.TradeTerminalTab.Jobs, autoQuery: false);
                Assert.IsFalse(hud.EconomyPanels.IsJobsQuestBannerVisible);
                Assert.AreEqual("volatiles", hud.EconomyPanels.SelectedHaulGood);

                // Step 3 corridor destination -> Jobs tab shows Stickney Yards haulage cross-link
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Deliver 5t metals to Stickney Yards"",
                            ""instructions"": ""Run +ship/plan stickney and sell at Stickney."",
                            ""checked"": false, ""live_ready"": false
                        }]
                    }]
                }")));
                hud.SwitchTradeTerminalTab(TacSim.UI.TradeTerminalTab.Jobs, autoQuery: false);
                Assert.IsTrue(hud.EconomyPanels.IsJobsQuestBannerVisible);
                StringAssert.Contains("Stickney Yards", hud.EconomyPanels.JobsQuestBannerTextForTests);
                emitted.Clear();
                Assert.IsTrue(hud.EconomyPanels.TriggerJobsQuestFilterForTests());
                CollectionAssert.Contains(emitted, "+market/jobs haul");

                // Step 6 surface prospecting -> Jobs tab switches to Survey Commissions cross-link
                hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_06_surface_mining"",
                        ""title"": ""Scan, Sound, and Mine a Landing Site"",
                        ""checkpoints"": [
                            { ""id"": ""scan_landing_site"", ""title"": ""Surface scan"", ""instructions"": ""Run +tv/1 scan go."", ""checked"": false, ""live_ready"": false },
                            { ""id"": ""mine_carbon_ore"", ""title"": ""Mine 1t carbon_ore"", ""instructions"": ""Run +tv/1 mine go."", ""checked"": false, ""live_ready"": false }
                        ]
                    }]
                }")));
                hud.SwitchTradeTerminalTab(TacSim.UI.TradeTerminalTab.Jobs, autoQuery: false);
                Assert.IsTrue(hud.EconomyPanels.IsJobsQuestBannerVisible);
                StringAssert.Contains("Surface Survey", hud.EconomyPanels.JobsQuestBannerTextForTests);
                emitted.Clear();
                Assert.IsTrue(hud.EconomyPanels.TriggerJobsQuestFilterForTests());
                CollectionAssert.Contains(emitted, "+market/jobs survey");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Coordinator_EmitsBetweenStepsNextGuidedArrivalCalloutWhenEnteringGiverRoom()
        {
            Assert.AreEqual(
                "QUEST OBJECTIVE: EXCHANGE HALL",
                TacSim.UI.TacSimFloatingCallouts.FormatQuestRoomArrivalCallout("Exchange Hall", isTurnIn: false));
            Assert.AreEqual(
                "QUEST TURN-IN: REGISTRY & CO-OP DESK",
                TacSim.UI.TacSimFloatingCallouts.FormatQuestRoomArrivalCallout("Registry & Co-op Desk", isTurnIn: true));
            Assert.AreEqual(
                "NEXT GUIDED (2/7) [!]: ILSE ODUYA-BRANDT — REGISTRY & CO-OP DESK",
                TacSim.UI.TacSimFloatingCallouts.FormatQuestRoomArrivalCallout("Registry & Co-op Desk", isTurnIn: false, 2, "Ilse Oduya-Brandt"));

            var rootGo = new UnityEngine.GameObject("BetweenStepsArrivalTest");
            try
            {
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                var hudGo = new UnityEngine.GameObject("Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();
                coord.EnsureSubsystems();

                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                Assert.IsNotNull(onRoom);

                // Between Step 1 (done) and Step 2 (not yet accepted; Ilse at room.c05_registry)
                coord.Hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [],
                    ""available"": [],
                    ""completed"": [
                        { ""key"": ""tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll"", ""title"": ""First Papers at the Registry"" }
                    ]
                }")));

                // 1. Initial room hydration must not fire a room-arrival callout
                onRoom.Invoke(coord, new object[]
                {
                    MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                        ""id"": ""#10"",
                        ""name"": ""Port Watch Arrivals"",
                        ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c03_arrivals"" } }
                    }"))
                });
                Assert.AreEqual(string.Empty, coord.Hud.FloatingCallouts.LastShownText);

                // 2. Stepping into Registry & Co-op Desk emits the Next Guided (2/7) giver arrival callout
                onRoom.Invoke(coord, new object[]
                {
                    MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                        ""id"": ""#15"",
                        ""name"": ""Registry & Co-op Desk"",
                        ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c05_registry"" } }
                    }"))
                });
                Assert.AreEqual(
                    "NEXT GUIDED (2/7) [!]: ILSE ODUYA-BRANDT — REGISTRY & CO-OP DESK",
                    coord.Hud.FloatingCallouts.LastShownText);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void Coordinator_ResolvesLocalStationQuestRoomForMinimapWayfinderAndBoardingCallout()
        {
            var rootGo = new UnityEngine.GameObject("LocalStationQuestResolutionTest");
            try
            {
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                var hudGo = new UnityEngine.GameObject("Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();
                coord.EnsureSubsystems();

                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                Assert.IsNotNull(onRoom);

                // Stand at Registry & Co-op Desk on Ceres with 0/5t metals loaded for Step 3 (Stickney target)
                onRoom.Invoke(coord, new object[]
                {
                    MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                        ""id"": ""#15"",
                        ""name"": ""Registry & Co-op Desk"",
                        ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c05_registry"" } }
                    }"))
                });
                coord.TradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true, ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 10000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                coord.Hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""next_room_entry"": ""room.p07_receiving"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Buy 5 tonnes of metals at Ceres and deliver to Stickney"",
                            ""instructions"": ""Buy 5 metals and +market/sell at Stickney."",
                            ""room_entry"": ""room.p07_receiving"",
                            ""checked"": false, ""live_ready"": false
                        }]
                    }]
                }")));

                // 1. Minimap beacon resolves off-station Stickney target to Ceres Exchange Hall while procuring
                Assert.AreEqual("room.c07_exchange", coord.Hud.Minimap.QuestObjectiveRoomEntry);

                // Clicking tracker Wayfinder plots a real 3D trail to Exchange Hall (not failing on off-station Stickney!)
                Assert.IsTrue(coord.Hud.QuestJournal.TriggerTrackerWayfinder());
                Assert.IsTrue(coord.Wayfinder.HasActiveRoute);
                Assert.AreEqual("room.c07_exchange", coord.Wayfinder.TargetId);

                // Walking into Exchange Hall fires objective arrival callout
                onRoom.Invoke(coord, new object[]
                {
                    MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                        ""id"": ""#17"",
                        ""name"": ""Exchange Hall"",
                        ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c07_exchange"" } }
                    }"))
                });
                Assert.AreEqual("QUEST OBJECTIVE: EXCHANGE HALL", coord.Hud.FloatingCallouts.LastShownText);

                // 2. Load 5/5t metals -> Minimap beacon immediately pivots to Berth Gallery
                coord.TradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true, ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 5000, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 10000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 5 }]
                        }
                    }
                }"));
                Assert.AreEqual("room.c01_berth_gallery", coord.Hud.Minimap.QuestObjectiveRoomEntry);

                // Walking into Visiting Berth Gallery fires Board Ship arrival callout
                onRoom.Invoke(coord, new object[]
                {
                    MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                        ""id"": ""#19"",
                        ""name"": ""Visiting Berth Gallery"",
                        ""ambient"": { ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c01_berth_gallery"" } }
                    }"))
                });
                Assert.AreEqual(
                    "BOARD SHIP FOR QUEST: VISITING BERTH GALLERY (+ship/board)",
                    coord.Hud.FloatingCallouts.LastShownText);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }

        [Test]
        public void StationWorldAndOpsDirectory_PrioritizeTurnInQuestMarkersAndResolveOffStationDirectoryTarget()
        {
            var rootGo = new UnityEngine.GameObject("QuestDirectoryAndMarkerPriorityTest");
            try
            {
                var coord = rootGo.AddComponent<TacSimGameCoordinator>();
                var hudGo = new UnityEngine.GameObject("Hud");
                hudGo.transform.SetParent(rootGo.transform, false);
                coord.Hud = hudGo.AddComponent<TacSim.UI.TacSimHudController>();
                coord.EnsureSubsystems();

                var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance;
                var onRoom = typeof(TacSimGameCoordinator).GetMethod("HandleServerRoomView", flags);
                Assert.IsNotNull(onRoom);

                // Stand at Registry & Co-op Desk with Ilse carrying multiple quest markers (available AFTER turn_in)
                onRoom.Invoke(coord, new object[]
                {
                    MushProtocol.ParseRoomView(JsonNode.Parse(@"{
                        ""room"": { ""id"": ""#15"", ""name"": ""Registry & Co-op Desk"" },
                        ""contents"": [{ ""id"": ""#50"", ""name"": ""Ilse Oduya-Brandt"" }],
                        ""ambient"": {
                            ""atlas"": { ""pack"": ""neighborhood.ceres_freeport"", ""entry"": ""room.c05_registry"" },
                            ""npcs"": [""#50""],
                            ""quest_markers"": [
                                { ""npc_id"": ""#50"", ""npc_name"": ""Ilse Oduya-Brandt"", ""marker"": ""in_progress"", ""symbol"": ""?"", ""quest_key"": ""q.active"", ""quest_title"": ""Active Check"" },
                                { ""npc_id"": ""#50"", ""npc_name"": ""Ilse Oduya-Brandt"", ""marker"": ""turn_in"", ""symbol"": ""?"", ""quest_key"": ""q.turnin"", ""quest_title"": ""First Papers at the Registry"" },
                                { ""npc_id"": ""#50"", ""npc_name"": ""Ilse Oduya-Brandt"", ""marker"": ""available"", ""symbol"": ""!"", ""quest_key"": ""q.next"", ""quest_title"": ""Stock the Hold"" }
                            ]
                        }
                    }"))
                });

                // 1. StationWorldController 3D overhead marker keeps turn_in (?) over later available (!) / in_progress (?)
                var overheadMarker = coord.StationWorld.GetNpcQuestMarker("#50");
                Assert.IsNotNull(overheadMarker);
                Assert.AreEqual("turn_in", overheadMarker.Marker);
                Assert.AreEqual("?", overheadMarker.Symbol);

                // 2. OpsDirectory Presence tab picks turn_in [? QUEST] rather than first-match in_progress
                coord.TradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true, ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 0, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 10000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": []
                        }
                    }
                }"));
                coord.Hud.QuestJournal.ApplyQuestsPanel(MushProtocol.ParseQuestsPanel(JsonNode.Parse(@"{
                    ""schema"": 1,
                    ""room_markers"": [
                        { ""npc_id"": ""#50"", ""npc_name"": ""Ilse Oduya-Brandt"", ""marker"": ""in_progress"", ""symbol"": ""?"", ""quest_key"": ""q.active"", ""quest_title"": ""Active Check"" },
                        { ""npc_id"": ""#50"", ""npc_name"": ""Ilse Oduya-Brandt"", ""marker"": ""available"", ""symbol"": ""!"", ""quest_key"": ""q.next"", ""quest_title"": ""Stock the Hold"" },
                        { ""npc_id"": ""#50"", ""npc_name"": ""Ilse Oduya-Brandt"", ""marker"": ""turn_in"", ""symbol"": ""?"", ""quest_key"": ""q.turnin"", ""quest_title"": ""First Papers at the Registry"" }
                    ],
                    ""active"": [{
                        ""key"": ""tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery"",
                        ""title"": ""Metals Down the Corridor"",
                        ""next_room_entry"": ""room.p07_receiving"",
                        ""checkpoints"": [{
                            ""id"": ""deliver_metals"",
                            ""title"": ""Buy 5 tonnes of metals at Ceres and deliver to Stickney"",
                            ""instructions"": ""Buy 5 metals and +market/sell at Stickney."",
                            ""room_entry"": ""room.p07_receiving"",
                            ""checked"": false, ""live_ready"": false
                        }]
                    }]
                }")));

                coord.Hud.OpsDirectory.SetOpen(true);
                coord.Hud.OpsDirectory.SelectTab(TacSim.UI.OpsDirectoryTab.RoomPresence);
                Assert.AreEqual(1, coord.Hud.OpsDirectory.HighlightedQuestPresenceRowCount);
                StringAssert.Contains("[? QUEST]", coord.Hud.OpsDirectory.FirstPresenceRowLabel);
                StringAssert.Contains("#FFDD44", coord.Hud.OpsDirectory.FirstPresenceRowLabel);

                // 3. OpsDirectory Deck Directory resolves off-station Stickney target -> Ceres Exchange Hall while procuring
                coord.Hud.OpsDirectory.SelectTab(TacSim.UI.OpsDirectoryTab.DeckDirectory);
                Assert.GreaterOrEqual(coord.Hud.OpsDirectory.HighlightedQuestDirectoryRowIndex, 0);
                Assert.AreEqual("room.c07_exchange", coord.Hud.OpsDirectory.HighlightedQuestDirectoryRoomId);

                // 4. Once 5/5t metals are loaded -> OpsDirectory pivots highlight to Visiting Berth Gallery
                coord.TradeState.ApplyPortTradeFeed(JsonNode.Parse(@"{
                    ""enabled"": true, ""venue"": ""ceres"",
                    ""ship"": {
                        ""hull_id"": ""#99"",
                        ""stowage"": {
                            ""cargo_kg"": 5000, ""cargo_limit_kg"": 100000,
                            ""tank_kg"": 10000, ""tank_limit_kg"": 80000,
                            ""stores_days"": 10, ""stores_limit_days"": 120,
                            ""items"": [{ ""good"": ""tacsim:metals"", ""label"": ""Metals"", ""qty"": 5 }]
                        }
                    }
                }"));
                coord.Hud.OpsDirectory.RefreshModalContents();
                Assert.GreaterOrEqual(coord.Hud.OpsDirectory.HighlightedQuestDirectoryRowIndex, 0);
                Assert.AreEqual("room.c01_berth_gallery", coord.Hud.OpsDirectory.HighlightedQuestDirectoryRoomId);
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }
    }
}
