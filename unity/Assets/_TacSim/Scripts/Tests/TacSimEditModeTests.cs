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

            // 1-click buy Quote -> Accept auto-chain
            string quoteCmd = state.BeginOneClickBuy(4UL, 15, currentTimeS: 10f);
            Assert.AreEqual("+market/quote #4=15", quoteCmd);
            Assert.IsTrue(state.IngestServerLine("Quote #27: 15 × 100.00cr = 1500.00cr — good until 2026-11-17T03:00:00Z (real time). It reserves nothing; take it up with +market/accept #27.", 11f, out var followUps));
            Assert.IsNotNull(followUps);
            Assert.AreEqual("+market/accept #27", followUps[0]);

            // Parse canonical crates/tacsim/src/port/gauges.rs +ship & +credit lines
            Assert.IsTrue(state.IngestServerLine("Hold: cargo 29/100 t · 11.6/120 m³ · tank 42/80 t · locker 20/120 crew-days.", 12f, out _));
            Assert.AreEqual(29, state.HoldMassTonnes);
            Assert.AreEqual(11.6f, state.HoldVolumeM3, 0.01f);
            Assert.AreEqual(42, state.TankTonnes);
            Assert.AreEqual(20, state.StoresCrewDays);

            Assert.IsTrue(state.IngestServerLine("Cash: 23500.00cr available; the buyout is 90000.00cr.", 12f, out _));
            Assert.AreEqual("23500.00cr", state.CreditsText);
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

            // Delta before snapshot must trigger resubscribe
            var orphanDelta = JsonNode.Parse(@"{""kind"":""delta"",""generation"":""g1"",""base_sequence"":1,""sequence"":2}");
            Assert.AreEqual(ViewCursorDisposition.NeedsResubscribe, MushProtocol.EvaluateViewCursor(cursor, orphanDelta));

            // Snapshot establishes baseline
            var snap = JsonNode.Parse(@"{""kind"":""snapshot"",""generation"":""g1"",""sequence"":5}");
            Assert.AreEqual(ViewCursorDisposition.ApplySnapshot, MushProtocol.EvaluateViewCursor(cursor, snap));
            Assert.AreEqual(5UL, cursor.LastSequence);

            // Contiguous delta advances cursor
            var delta6 = JsonNode.Parse(@"{""kind"":""delta"",""generation"":""g1"",""base_sequence"":5,""sequence"":6}");
            Assert.AreEqual(ViewCursorDisposition.ApplyDelta, MushProtocol.EvaluateViewCursor(cursor, delta6));
            Assert.AreEqual(6UL, cursor.LastSequence);

            // Duplicate sequence is ignored
            Assert.AreEqual(ViewCursorDisposition.IgnoreDuplicateOrStale, MushProtocol.EvaluateViewCursor(cursor, delta6));

            // Gap in base_sequence triggers clean resubscribe
            var gappedDelta = JsonNode.Parse(@"{""kind"":""delta"",""generation"":""g1"",""base_sequence"":8,""sequence"":9}");
            Assert.AreEqual(ViewCursorDisposition.NeedsResubscribe, MushProtocol.EvaluateViewCursor(cursor, gappedDelta));

            // Generation rotation on delta triggers resubscribe
            var wrongGen = JsonNode.Parse(@"{""kind"":""delta"",""generation"":""g2"",""base_sequence"":6,""sequence"":7}");
            Assert.AreEqual(ViewCursorDisposition.NeedsResubscribe, MushProtocol.EvaluateViewCursor(cursor, wrongGen));

            // WS Close Code & Query invariants
            Assert.AreEqual(WsCloseDisposition.FatalSessionEnded, MushWsClient.ClassifyCloseCode(4401));
            Assert.AreEqual(WsCloseDisposition.Cooloff60s, MushWsClient.ClassifyCloseCode(4429));
            Assert.AreEqual(WsCloseDisposition.ReconnectBackoff, MushWsClient.ClassifyCloseCode(1012));
            Assert.AreEqual("char=42", MushWsClient.BuildWsQuery("42", 0UL));
            Assert.AreEqual("char=42&resume=108", MushWsClient.BuildWsQuery("42", 108UL));
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
        }
    }
}
