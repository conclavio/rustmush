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
            Assert.AreEqual("n", toCoop[0].command);

            var coopToBerth = ceres.FindExitPath("room.c05_registry", "room.c01_berth_gallery");
            Assert.AreEqual(3, coopToBerth.Count);
            Assert.AreEqual("s", coopToBerth[0].command);
            Assert.AreEqual("s", coopToBerth[1].command);
            Assert.AreEqual("s", coopToBerth[2].command);

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
                Assert.IsTrue(sync.StoreVerifiedPack("goods", shaA, serverAGoods));
                // Persist manifest on server A by simulating pack panel arrival
                using (var ws = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(ws, JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""goods"",""sha256"":""{shaA}"",""json"":{JsonNode.FromString(serverAGoods).ToJson()}}}"), 1f);
                }
                Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _));
                Assert.IsTrue(tradeState.GoodsCatalog.Exists(g => g.Label == "Server A Metal"));

                // Switching to a COLD Server B (empty cache) MUST clear Server A's dynamic packs AND fire OnPacksUpdated to invalidate loaded models!
                int beforeColdSwitch = updateEvents;
                sync.InitializeDiskCacheForServer("https://server-b.example.com", tempDir);
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Switching servers must clear previous server's dynamic packs");
                Assert.Greater(updateEvents, beforeColdSwitch, "Cold-server switch must still fire OnPacksUpdated so loaded models invalidate");
                Assert.IsFalse(tradeState.GoodsCatalog.Exists(g => g.Label == "Server A Metal"), "Cold-server switch must evict Server A models from loaded catalog");

                // Switching back to Server A with warm disk cache MUST hydrate packs AND fire OnPacksUpdated!
                int beforeWarm = updateEvents;
                sync.InitializeDiskCacheForServer("https://server-a.example.com", tempDir);
                Assert.IsTrue(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out string warmJson));
                Assert.AreEqual(serverAGoods, warmJson);
                Assert.Greater(updateEvents, beforeWarm, "Warm-cache activation must raise OnPacksUpdated so already-constructed models refresh");

                // Pruning check: delivering an authoritative empty manifest MUST delete removed packs from memory AND saved manifest!
                var emptyManifest = JsonNode.Parse(@"{""op"":""manifest"",""packs"":[]}");
                using (var wsPrune = new MushWsClient())
                {
                    sync.HandleWebSocketContentPanel(wsPrune, emptyManifest, nowRealtime: 5f);
                }
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Pruned pack must be evicted from memory");
                // Re-hydrating Server A's disk cache proves `goods` was also removed from persisted manifest.json
                sync.InitializeDiskCacheForServer("https://server-a.example.com", tempDir);
                Assert.IsFalse(TacSimContentLoader.TryGetSyncedOrResourceJson("goods", out _), "Pruned pack must not reappear from disk manifest");

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
                    for (int i = 0; i < 4; i++)
                    {
                        var (pid, psha, pjson) = packPayloads[i];
                        var reply = JsonNode.Parse($@"{{""op"":""pack"",""found"":true,""id"":""{pid}"",""sha256"":""{psha}"",""json"":{JsonNode.FromString(pjson).ToJson()}}}");
                        sync.HandleWebSocketContentPanel(ws, reply, nowRealtime: 10.0f);
                    }
                    // Completing a batch at t=10.0f MUST NOT emit another batch before MinBatchIntervalSec (0.45s)!
                    sync.PumpPackQueue(ws, nowRealtime: 10.10f);
                    Assert.AreEqual(4, ws.OutboundQueueCount, "Pacing must hold between batches before 0.45s has elapsed");

                    // Advancing past 0.45s dispatches exactly the next batch of 4 (total 8):
                    sync.PumpPackQueue(ws, nowRealtime: 10.50f);
                    Assert.AreEqual(8, ws.OutboundQueueCount, "Next batch of 4 dispatches once 0.45s interval elapses");
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

            // 4. Decode 512-site `site.map` window centered around [256, 256] (`x0=224, y0=224, span=2`)
            var siteFrame = JsonNode.Parse(@"{
                ""width"": 32,
                ""height"": 32,
                ""cell_cm"": 1000,
                ""rows"": [[["","", 32]]],
                ""you"": [16, 16],
                ""origin_cell"": [224, 224],
                ""site"": {
                    ""site"": ""Urvara Reach"",
                    ""addressed"": 1,
                    ""grid"": { ""x0"": 224, ""y0"": 224, ""span"": 2, ""cell_cm"": 1000 },
                    ""rigs"": [
                        { ""slot"": 1, ""cell"": [256, 256], ""charge_pct"": 17, ""hopper"": ""3 of 6 t"" }
                    ]
                }
            }");
            var grid = TacSimTerrainDecoder.DecodeSiteMapFrame(siteFrame);
            Assert.IsNotNull(grid);
            Assert.IsTrue(grid.IsFromLiveServerFeed);
            Assert.AreEqual(224, grid.OriginCellX);
            Assert.AreEqual(224, grid.OriginCellY);
            Assert.AreEqual(2, grid.SpanCells);
            Assert.AreEqual(16, grid.SiteToLocalCellX(256));
            Assert.AreEqual(16, grid.SiteToLocalCellY(256));
            Assert.AreEqual(256, grid.LocalToSiteCellX(16));
            Assert.AreEqual(258, grid.LocalToSiteCellX(17), "Nearby click at local cell 17 on span=2 window maps to site cell 258, never clamped near 157");

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

            // 8. Cross-language Rust live-reconciled neighborhood schema (`footprint.rect` + `surface.floor`) & synthesized room
            try
            {
                const string ReconciledStationJson = @"{
                    ""manifest"": { ""id"": ""neighborhood.test_station"", ""version"": 1, ""kind"": ""neighborhood"", ""projected"": true },
                    ""entries"": {
                        ""structure.hub"": {
                            ""kind"": ""structure"",
                            ""def"": {
                                ""floors"": [
                                    { ""index"": 1, ""name"": ""Deck 1"", ""z_offset_m"": 0.0, ""height_m"": 4.5 },
                                    { ""index"": 2, ""name"": ""Deck 2"", ""z_offset_m"": 5.0, ""height_m"": 4.5 },
                                    { ""index"": 3, ""name"": ""Deck 3"", ""z_offset_m"": 10.0, ""height_m"": 4.5 }
                                ]
                            }
                        },
                        ""room.existing"": {
                            ""kind"": ""room"",
                            ""def"": {
                                ""name"": ""Live Moved Room"",
                                ""kind"": ""indoor"",
                                ""footprint"": { ""rect"": [[100.0, 200.0], [120.0, 220.0]] },
                                ""surface"": { ""floor"": [""structure.hub"", 3] },
                                ""access"": ""public""
                            }
                        },
                        ""room.synth"": {
                            ""kind"": ""room"",
                            ""def"": {
                                ""name"": ""Synthesized Room"",
                                ""kind"": ""circulation"",
                                ""footprint"": { ""rect"": [[40.0, 50.0], [65.0, 80.0]] },
                                ""surface"": { ""floor"": [""structure.hub"", 2] },
                                ""access"": ""public""
                            }
                        }
                    }
                }";
                TacSimContentLoader.RegisterDynamicPack("neighborhood.test_station", ReconciledStationJson);
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
        }
    }
}
