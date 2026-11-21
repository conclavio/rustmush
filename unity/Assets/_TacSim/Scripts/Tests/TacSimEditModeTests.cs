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
                string chunk1MiB = "\"" + new string('a', (1024 * 1024) - 2) + "\"";
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

                // 3xx redirect status must be rejected cleanly.
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
                Assert.IsTrue(req.aborted, "Oversized Content-Length must abort UnityWebRequest immediately");
                Assert.IsFalse(handler.FeedChunkForTest(new byte[] { (byte)'x' }, 1));

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
            }

            // 3. Chunked response without Content-Length that exceeds 64 KiB aborts request and rejects.
            using (var req = MushAuthClient.CreateAuthPostRequest(httpsUri, System.Text.Encoding.UTF8.GetBytes("{}"), out BoundedDownloadHandler handler))
            {
                byte[] chunk = new byte[16 * 1024];
                for (int c = 0; c < 4; c++)
                    Assert.IsTrue(handler.FeedChunkForTest(chunk, chunk.Length));
                Assert.IsFalse(req.aborted);
                Assert.IsFalse(handler.FeedChunkForTest(new byte[] { (byte)'z' }, 1));
                Assert.IsTrue(handler.Overflowed);
                Assert.IsTrue(req.aborted, "Chunked overflow without Content-Length must abort UnityWebRequest");
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

                UnityEngine.Time.time = 100f;
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#101", "Visiting Berth Gallery", "room.c01_berth_gallery") });

                // Walk 2 hops north (c01_berth_gallery -> c02_pressure -> c03_arrivals) and call Flush twice before RoomView.
                coord.Player.TeleportTo(coord.StationWorld.GetRoomSpawnPosition("room.c03_arrivals"), 0f);
                UnityEngine.Time.time = 101f;
                onFlush.Invoke(coord, null);
                onFlush.Invoke(coord, null);
                Assert.AreEqual(1, dispatched.Count, "Multi-hop flush must send ONLY ONE hop before RoomView confirmation");
                Assert.AreEqual("goto n", dispatched[0]);

                // Authoritative RoomView confirming c02_pressure advances ONLY hop 2 (`goto n`).
                UnityEngine.Time.time = 101.5f;
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#102", "Pressure Lobby", "room.c02_pressure") });
                Assert.AreEqual(2, dispatched.Count);
                Assert.AreEqual("goto n", dispatched[1]);

                // Authoritative RoomView confirming c03_arrivals completes with no extra commands.
                UnityEngine.Time.time = 102f;
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#103", "Port Watch Arrivals", "room.c03_arrivals") });
                Assert.AreEqual(2, dispatched.Count);

                // Mismatch cancellation: walk 2 hops back south, flush hop 1, then receive RoomView STILL in c03_arrivals -> hop 2 canceled!
                dispatched.Clear();
                coord.Player.TeleportTo(coord.StationWorld.GetRoomSpawnPosition("room.c01_berth_gallery"), 180f);
                UnityEngine.Time.time = 105f;
                onFlush.Invoke(coord, null);
                Assert.AreEqual(1, dispatched.Count);
                Assert.AreEqual("goto s", dispatched[0]);
                UnityEngine.Time.time = 105.5f;
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#103", "Port Watch Arrivals", "room.c03_arrivals") });
                Assert.AreEqual(1, dispatched.Count, "Mismatched RoomView must cancel remaining queued hops");

                // Unrelated explicit command cancels remaining queued hops!
                dispatched.Clear();
                coord.Player.TeleportTo(coord.StationWorld.GetRoomSpawnPosition("room.c01_berth_gallery"), 180f);
                UnityEngine.Time.time = 110f;
                onFlush.Invoke(coord, null);
                Assert.AreEqual(1, dispatched.Count);
                coord.ExecutePlayerCommand("look");
                dispatched.Clear();
                UnityEngine.Time.time = 110.5f;
                onRoom.Invoke(coord, new object[] { MakeStationRoom("#102", "Pressure Lobby", "room.c02_pressure") });
                Assert.AreEqual(0, dispatched.Count, "Explicit command must cancel queued multi-hop navigation");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(rootGo);
            }
        }
    }
}
