using System;
using System.Collections.Generic;
using System.IO;
using TacSim.Core;
using UnityEngine;

namespace TacSim.World
{
    /// <summary>
    /// Exact 1:1 mirror of crates/mush-atlas/src/terrain.rs TerrainClass (codes 0..17).
    /// </summary>
    public enum TerrainCellClass : byte
    {
        Void = 0,
        Paved = 1,
        Ground = 2,
        Grass = 3,
        Water = 4,
        Floor = 5,
        Rubble = 6,
        Road = 7,
        Sand = 8,
        Rock = 9,
        Wall = 10,
        Door = 11,
        Platform = 12,
        WoodsLight = 13,
        WoodsHeavy = 14,
        WaterDeep = 15,
        Ice = 16,
        Regolith = 17
    }

    [Serializable]
    public sealed class ArenaKeySpotDef
    {
        public string Letter;
        public string Name;
        public int CellX;
        public int CellY;
    }

    [Serializable]
    public sealed class ArenaZoneRect
    {
        public int MinX;
        public int MinY;
        public int MaxX;
        public int MaxY;
    }

    [Serializable]
    public sealed class ArenaSpawnPoseDef
    {
        public string SlotKey; // e.g. "red_1", "blue_3"
        public string Party;   // "R" or "B"
        public float Xm;
        public float Zm;
        public float HeadingDeg;
    }

    public sealed class DecodedTerrainGrid
    {
        public string Id;
        public string DisplayName;
        public string Biome; // "desert", "temperate", "arctic", "regolith", "ceres", "vesta"
        public int Width;
        public int Height;
        public float CellSizeM = 10f;
        public int OriginCellX;
        public int OriginCellY;
        public TerrainCellClass[] Classes; // length = Width * Height (row-major, y=0 south -> north)
        public char[] RawGlyphs;           // optional overlay glyphs from site.map / arena.map
        public float[] ElevMeters;         // length = Width * Height

        public ArenaZoneRect RedEntryZone;
        public ArenaZoneRect BlueEntryZone;
        public string HoldSpotLetter = "B";
        public int HoldRadiusCells = 6;
        public List<ArenaKeySpotDef> KeySpots = new List<ArenaKeySpotDef>();
        public List<ArenaSpawnPoseDef> Spawns = new List<ArenaSpawnPoseDef>();

        public float SampleElevationMeters(float localX, float localZ)
        {
            if (ElevMeters == null || ElevMeters.Length == 0 || Width <= 0 || Height <= 0)
                return 0f;

            float gx = Mathf.Clamp(localX / Mathf.Max(0.1f, CellSizeM) - 0.5f, 0f, Width - 1.001f);
            float gy = Mathf.Clamp(localZ / Mathf.Max(0.1f, CellSizeM) - 0.5f, 0f, Height - 1.001f);

            int x0 = Mathf.Clamp(Mathf.FloorToInt(gx), 0, Width - 1);
            int y0 = Mathf.Clamp(Mathf.FloorToInt(gy), 0, Height - 1);
            int x1 = Mathf.Min(Width - 1, x0 + 1);
            int y1 = Mathf.Min(Height - 1, y0 + 1);

            float tx = gx - x0;
            float ty = gy - y0;

            float h00 = ElevMeters[y0 * Width + x0];
            float h10 = ElevMeters[y0 * Width + x1];
            float h01 = ElevMeters[y1 * Width + x0];
            float h11 = ElevMeters[y1 * Width + x1];

            return Mathf.Lerp(Mathf.Lerp(h00, h10, tx), Mathf.Lerp(h01, h11, tx), ty);
        }

        public TerrainCellClass GetCellClass(int cx, int cy)
        {
            if (Classes == null || cx < 0 || cy < 0 || cx >= Width || cy >= Height)
                return TerrainCellClass.Ground;
            return Classes[cy * Width + cx];
        }

        public Vector3 CellToLocalPos(int cx, int cy, float yOffset = 0f)
        {
            float lx = (cx - OriginCellX + 0.5f) * CellSizeM;
            float lz = (cy - OriginCellY + 0.5f) * CellSizeM;
            float ly = SampleElevationMeters(lx, lz) + yOffset;
            return new Vector3(lx, ly, lz);
        }
    }

    /// <summary>
    /// Decodes authoritative terrain &amp; elevation payloads across TacSim:
    /// 1) Live `arena.map` v1 ("zigzag-leb128-delta" base64 elev + RLE glyph rows + spots)
    /// 2) Live `site.map` v1 (28x28 drive window &amp; 32x32 site overview RLE grids)
    /// 3) Authored `crates/tacsim/content/arenas/*.json` + `atlas/*.json` 4-quadrant 256x256 heightfields
    /// 4) Authored `crates/tacsim/content/atlas/region.*.json` planetary prospecting sites (Occator Rim, etc.)
    /// </summary>
    public static class TacSimTerrainDecoder
    {
        public static TerrainCellClass ClassFromCode(byte code)
        {
            return code <= 17 ? (TerrainCellClass)code : TerrainCellClass.Ground;
        }

        public static TerrainCellClass ClassFromGlyph(char g)
        {
            switch (g)
            {
                case ' ': return TerrainCellClass.Void;
                case '.': return TerrainCellClass.Paved;
                case ',': return TerrainCellClass.Ground;
                case '"': return TerrainCellClass.Grass;
                case '~': return TerrainCellClass.Water;
                case '_': return TerrainCellClass.Floor;
                case 'x': return TerrainCellClass.Rubble;
                case '=': return TerrainCellClass.Road;
                case ':': return TerrainCellClass.Sand;
                case '^': return TerrainCellClass.Rock;
                case '#': return TerrainCellClass.Wall;
                case '+': return TerrainCellClass.Door;
                case 'D': return TerrainCellClass.Platform;
                case 't': return TerrainCellClass.WoodsLight;
                case 'T': return TerrainCellClass.WoodsHeavy;
                case 'W': return TerrainCellClass.WaterDeep;
                case 'i': return TerrainCellClass.Ice;
                case 'r': return TerrainCellClass.Regolith;
                default:  return TerrainCellClass.Regolith;
            }
        }

        /// <summary>
        /// Decodes `arena.map` v1 `elev.data` ("zigzag-leb128-delta" centimetres) into int[] elev_cm.
        /// Mirrors crates/tacsim/src/match_/cockpit/frames.rs exact zigzag + LEB128 accumulator.
        /// </summary>
        public static int[] DecodeZigzagLeb128DeltaElevCm(string base64Data, int expectedCells)
        {
            int count = Mathf.Max(0, expectedCells);
            var outCm = new int[count];
            if (string.IsNullOrWhiteSpace(base64Data) || count == 0)
                return outCm;

            byte[] bytes;
            try
            {
                bytes = Convert.FromBase64String(base64Data.Trim());
            }
            catch
            {
                return outCm;
            }

            int cursor = 0;
            int accumCm = 0;
            for (int i = 0; i < count && cursor < bytes.Length; i++)
            {
                uint zig = 0;
                int shift = 0;
                while (cursor < bytes.Length)
                {
                    byte b = bytes[cursor++];
                    zig |= (uint)(b & 0x7F) << shift;
                    if ((b & 0x80) == 0)
                        break;
                    shift += 7;
                    if (shift >= 35)
                        break;
                }
                int delta = (int)(zig >> 1) ^ -(int)(zig & 1);
                accumCm += delta;
                outCm[i] = accumCm;
            }
            return outCm;
        }

        /// <summary>
        /// Encodes int[] elev_cm into "zigzag-leb128-delta" base64 (used by tests &amp; synthetic fixtures).
        /// </summary>
        public static string EncodeZigzagLeb128DeltaElevCm(int[] elevCm)
        {
            if (elevCm == null || elevCm.Length == 0) return string.Empty;
            var bytes = new List<byte>(elevCm.Length * 2);
            int prev = 0;
            for (int i = 0; i < elevCm.Length; i++)
            {
                int delta = elevCm[i] - prev;
                prev = elevCm[i];
                uint zig = (uint)((delta << 1) ^ (delta >> 31));
                while (zig >= 0x80u)
                {
                    bytes.Add((byte)((zig & 0x7Fu) | 0x80u));
                    zig >>= 7;
                }
                bytes.Add((byte)zig);
            }
            return Convert.ToBase64String(bytes.ToArray());
        }

        /// <summary>
        /// Decodes RLE `rows` (`[[glyph_str, count], ...]`) shared by `arena.map` and `site.map`.
        /// </summary>
        public static char[] DecodeRleGlyphRows(JsonNode rowsNode, int width, int height)
        {
            var grid = new char[Mathf.Max(1, width * height)];
            for (int i = 0; i < grid.Length; i++) grid[i] = ',';

            var rows = rowsNode != null ? rowsNode.AsArray() : null;
            if (rows == null) return grid;

            int rowCount = Mathf.Min(height, rows.Count);
            for (int y = 0; y < rowCount; y++)
            {
                var runs = rows[y].AsArray();
                if (runs == null) continue;
                int x = 0;
                for (int r = 0; r < runs.Count && x < width; r++)
                {
                    var pair = runs[r].AsArray();
                    if (pair == null || pair.Count < 2) continue;
                    string gStr = pair[0].AsString(",");
                    char ch = !string.IsNullOrEmpty(gStr) ? gStr[0] : ',';
                    int runLen = Mathf.Max(0, pair[1].AsInt(1));
                    for (int k = 0; k < runLen && x < width; k++)
                    {
                        grid[y * width + x] = ch;
                        x++;
                    }
                }
            }
            return grid;
        }

        /// <summary>
        /// Decodes a live `arena.map` v1 panel keyframe from mushd into a full 3D DecodedTerrainGrid.
        /// </summary>
        public static DecodedTerrainGrid DecodeArenaMapFrame(JsonNode mapFrame)
        {
            if (mapFrame == null || !mapFrame.IsObject) return null;
            int w = Mathf.Clamp(mapFrame.Int("width", 256), 1, 512);
            int h = Mathf.Clamp(mapFrame.Int("height", 256), 1, 512);
            float cellM = mapFrame.Int("cell_cm", 1000) * 0.01f;

            int ox = 0, oy = 0;
            var orig = mapFrame["origin_cell"].AsArray();
            if (orig != null && orig.Count >= 2)
            {
                ox = orig[0].AsInt(0);
                oy = orig[1].AsInt(0);
            }

            char[] glyphs = DecodeRleGlyphRows(mapFrame["rows"], w, h);
            var classes = new TerrainCellClass[w * h];
            for (int i = 0; i < glyphs.Length; i++)
                classes[i] = ClassFromGlyph(glyphs[i]);

            var elevObj = mapFrame["elev"];
            string elevB64 = elevObj.IsObject ? elevObj.Str("data", string.Empty) : string.Empty;
            float unitScaleM = (elevObj.IsObject ? elevObj.Int("unit_cm", 1) : 1) * 0.01f;
            int[] rawElev = DecodeZigzagLeb128DeltaElevCm(elevB64, w * h);
            var elevM = new float[w * h];
            for (int i = 0; i < elevM.Length; i++)
                elevM[i] = rawElev[i] * unitScaleM;

            var grid = new DecodedTerrainGrid
            {
                Id = mapFrame.Str("surface", $"match_{mapFrame.Int("match", 1)}"),
                DisplayName = mapFrame.Str("surface", "Simulator Arena"),
                Biome = mapFrame.Str("biome", "desert"),
                Width = w,
                Height = h,
                CellSizeM = Mathf.Max(1f, cellM),
                OriginCellX = ox,
                OriginCellY = oy,
                Classes = classes,
                RawGlyphs = glyphs,
                ElevMeters = elevM
            };

            var spotsArr = mapFrame["spots"].AsArray();
            if (spotsArr != null)
            {
                for (int i = 0; i < spotsArr.Count; i++)
                {
                    var sp = spotsArr[i];
                    var cellArr = sp["cell"].AsArray();
                    if (cellArr == null || cellArr.Count < 2) continue;
                    grid.KeySpots.Add(new ArenaKeySpotDef
                    {
                        Letter = sp.Str("letter", ((char)('A' + i)).ToString()),
                        Name = sp.Str("name", $"Point {(char)('A' + i)}"),
                        CellX = cellArr[0].AsInt(128),
                        CellY = cellArr[1].AsInt(128)
                    });
                }
            }
            return grid;
        }

        /// <summary>
        /// Loads a static 256x256 arena definition (`proving_ground`, `mars_canyon`, `winter_valley`)
        /// from `crates/tacsim/content/arenas/<slug>.json` + `atlas/<slug>.json` 4-quadrant tiles.
        /// </summary>
        public static DecodedTerrainGrid LoadStaticArena(string arenaSlug)
        {
            string slug = string.IsNullOrWhiteSpace(arenaSlug) ? "mars_canyon" : arenaSlug.Trim().ToLowerInvariant();
            string arenaPath = TacSimContentLoader.ResolveRepoContentPath(Path.Combine("arenas", $"{slug}.json"));
            string atlasPath = TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", $"{slug}.json"));

            int side = 256;
            float cellM = 10f;
            string biome = slug.Contains("mars") ? "desert" : slug.Contains("winter") ? "arctic" : "temperate";

            var grid = new DecodedTerrainGrid
            {
                Id = slug,
                DisplayName = FormatTitle(slug),
                Biome = biome,
                Width = side,
                Height = side,
                CellSizeM = cellM,
                Classes = new TerrainCellClass[side * side],
                ElevMeters = new float[side * side]
            };

            string arenaJsonText = null;
            if (TacSimContentLoader.TryGetSyncedOrResourceJson($"arena.{slug}", out string syncedArenaJson))
                arenaJsonText = syncedArenaJson;
            else if (File.Exists(arenaPath))
                arenaJsonText = File.ReadAllText(arenaPath);

            if (!string.IsNullOrEmpty(arenaJsonText))
            {
                var aRoot = JsonNode.Parse(arenaJsonText);
                biome = aRoot.Str("biome", biome);
                grid.Biome = biome;
                side = Mathf.Clamp(aRoot.Int("width_cells", 256), 64, 512);
                int hSide = Mathf.Clamp(aRoot.Int("height_cells", side), 64, 512);
                grid.Width = side;
                grid.Height = hSide;
                grid.CellSizeM = Mathf.Max(1f, aRoot.Int("cell_cm", 1000) * 0.01f);
                grid.Classes = new TerrainCellClass[side * hSide];
                grid.ElevMeters = new float[side * hSide];

                grid.RedEntryZone = ParseZoneRect(aRoot["entry_zones"]["red"]);
                grid.BlueEntryZone = ParseZoneRect(aRoot["entry_zones"]["blue"]);

                var holdObj = aRoot["modes"]["hold"];
                if (holdObj.IsObject)
                {
                    grid.HoldSpotLetter = holdObj.Str("spot", "B");
                    grid.HoldRadiusCells = holdObj.Int("radius_cells", 6);
                }

                var ksArr = aRoot["key_spots"].AsArray();
                if (ksArr != null)
                {
                    for (int i = 0; i < ksArr.Count; i++)
                    {
                        var ks = ksArr[i];
                        var c = ks["cell"].AsArray();
                        if (c == null || c.Count < 2) continue;
                        grid.KeySpots.Add(new ArenaKeySpotDef
                        {
                            Letter = ks.Str("letter", ((char)('A' + i)).ToString()),
                            Name = ks.Str("name", $"Objective {(char)('A' + i)}"),
                            CellX = c[0].AsInt(side / 2),
                            CellY = c[1].AsInt(hSide / 2)
                        });
                    }
                }

                var tanksObj = aRoot["tanks"].AsObject();
                if (tanksObj != null)
                {
                    foreach (var kv in tanksObj)
                    {
                        var tNode = kv.Value;
                        grid.Spawns.Add(new ArenaSpawnPoseDef
                        {
                            SlotKey = kv.Key,
                            Party = kv.Key.StartsWith("blue", StringComparison.OrdinalIgnoreCase) ? "B" : "R",
                            Xm = tNode.Int("x_cm", 20000) * 0.01f,
                            Zm = tNode.Int("y_cm", 128000) * 0.01f,
                            HeadingDeg = tNode.Int("heading_mdeg", 90000) * 0.001f
                        });
                    }
                }
            }

            bool loadedTiles = false;
            if (TacSimContentLoader.TryGetSyncedOrResourceJson($"arena_atlas.{slug}", out string syncedAtlasJson))
            {
                loadedTiles = StitchAtlasQuadrantTiles(syncedAtlasJson, grid);
            }
            else if (File.Exists(atlasPath))
            {
                loadedTiles = StitchAtlasQuadrantTiles(File.ReadAllText(atlasPath), grid);
            }

            if (!loadedTiles)
            {
                SynthesizeFallbackArenaRelief(grid);
            }

            if (grid.KeySpots.Count == 0)
            {
                grid.KeySpots.Add(new ArenaKeySpotDef { Letter = "A", Name = "West Approach", CellX = 68, CellY = 128 });
                grid.KeySpots.Add(new ArenaKeySpotDef { Letter = "B", Name = "Central Crossroads", CellX = 128, CellY = 128 });
                grid.KeySpots.Add(new ArenaKeySpotDef { Letter = "C", Name = "East Ridge", CellX = 188, CellY = 128 });
            }

            if (grid.Spawns.Count == 0)
            {
                for (int s = 1; s <= 6; s++)
                {
                    float zOff = 1040f + (s - 1) * 95f;
                    grid.Spawns.Add(new ArenaSpawnPoseDef { SlotKey = $"red_{s}", Party = "R", Xm = 220f, Zm = zOff, HeadingDeg = 90f });
                    grid.Spawns.Add(new ArenaSpawnPoseDef { SlotKey = $"blue_{s}", Party = "B", Xm = 2340f, Zm = zOff, HeadingDeg = 270f });
                }
            }

            return grid;
        }

        private static bool StitchAtlasQuadrantTiles(string atlasJson, DecodedTerrainGrid grid)
        {
            var root = JsonNode.Parse(atlasJson);
            var entries = root["entries"].AsObject();
            if (entries == null) return false;

            bool any = false;
            string[] quads = { "field.q0_0", "field.q1_0", "field.q0_1", "field.q1_1" };
            int[,] offsets = { { 0, 0 }, { 128, 0 }, { 0, 128 }, { 128, 128 } };

            for (int q = 0; q < quads.Length; q++)
            {
                if (!entries.TryGetValue(quads[q], out JsonNode tileEntry)) continue;
                var def = tileEntry["def"];
                string classesB64 = def.Str("classes", string.Empty);
                string dataB64 = def.Str("data", string.Empty);
                if (string.IsNullOrEmpty(classesB64) || string.IsNullOrEmpty(dataB64)) continue;

                byte[] classBytes = Convert.FromBase64String(classesB64);
                byte[] elevBytes = Convert.FromBase64String(dataB64);
                if (classBytes.Length < 4) continue;

                int tw = classBytes[0] | (classBytes[1] << 8);
                int th = classBytes[2] | (classBytes[3] << 8);
                int total = tw * th;

                var tileClasses = new TerrainCellClass[total];
                int cur = 4;
                int idx = 0;
                while (cur + 2 < classBytes.Length && idx < total)
                {
                    int run = classBytes[cur] | (classBytes[cur + 1] << 8);
                    byte code = classBytes[cur + 2];
                    cur += 3;
                    TerrainCellClass cls = ClassFromCode(code);
                    for (int k = 0; k < run && idx < total; k++)
                        tileClasses[idx++] = cls;
                }

                int ox = offsets[q, 0];
                int oy = offsets[q, 1];
                for (int ty = 0; ty < th; ty++)
                {
                    int gy = oy + ty;
                    if (gy >= grid.Height) continue;
                    for (int tx = 0; tx < tw; tx++)
                    {
                        int gx = ox + tx;
                        if (gx >= grid.Width) continue;
                        int ti = ty * tw + tx;
                        int gi = gy * grid.Width + gx;
                        grid.Classes[gi] = tileClasses[ti];

                        if (ti * 2 + 1 < elevBytes.Length)
                        {
                            short elevCm = (short)(elevBytes[ti * 2] | (elevBytes[ti * 2 + 1] << 8));
                            grid.ElevMeters[gi] = elevCm * 0.01f;
                        }
                    }
                }
                any = true;
            }
            return any;
        }

        /// <summary>
        /// Builds a deterministic planetary prospecting surface (`occator_rim`, `ahuna_flank`, `rheasilvia_commons`)
        /// from `crates/tacsim/content/atlas/region.<slug>.json` + optional live `site.map` window overlay.
        /// </summary>
        public static DecodedTerrainGrid BuildPlanetarySiteGrid(string siteSlug, int side = 64)
        {
            string clean = string.IsNullOrWhiteSpace(siteSlug) ? "occator_rim" : siteSlug.Replace("region.", "").Trim().ToLowerInvariant();
            string path = TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", $"region.{clean}.json"));

            string displayName = FormatTitle(clean);
            string body = clean.Contains("rheasilvia") ? "vesta" : "ceres";
            int seed = 2605;
            float reliefM = 3.8f;

            if (File.Exists(path))
            {
                var root = JsonNode.Parse(File.ReadAllText(path));
                var entries = root["entries"].AsObject();
                if (entries != null)
                {
                    foreach (var kv in entries)
                    {
                        if (kv.Value.Str("kind") == "region")
                        {
                            var def = kv.Value["def"];
                            displayName = def.Str("name", displayName);
                            body = def.Str("body", body);
                            seed = def["background"].Int("seed", seed);
                            reliefM = Mathf.Max(1.8f, def["background"].Int("relief_cm", 220) * 0.018f);
                            break;
                        }
                    }
                }
            }

            var grid = new DecodedTerrainGrid
            {
                Id = clean,
                DisplayName = displayName,
                Biome = body,
                Width = side,
                Height = side,
                CellSizeM = 10f,
                Classes = new TerrainCellClass[side * side],
                RawGlyphs = new char[side * side],
                ElevMeters = new float[side * side]
            };

            int padCx = side / 2;
            int padCy = side / 2;

            for (int y = 0; y < side; y++)
            {
                for (int x = 0; x < side; x++)
                {
                    int i = y * side + x;
                    float nx = (x + seed * 0.13f) * 0.09f;
                    float ny = (y + seed * 0.29f) * 0.09f;
                    float baseWave = (SmoothValueNoise2D(nx, ny) - 0.45f) * reliefM * 2.2f
                                   + (SmoothValueNoise2D(nx * 2.7f, ny * 2.7f) - 0.5f) * reliefM * 0.85f;

                    // Sculpt two rim craters & flatten central landing pad
                    float dPad = Vector2.Distance(new Vector2(x, y), new Vector2(padCx, padCy));
                    float dCraterA = Vector2.Distance(new Vector2(x, y), new Vector2(side * 0.27f, side * 0.68f));
                    float dCraterB = Vector2.Distance(new Vector2(x, y), new Vector2(side * 0.74f, side * 0.32f));

                    TerrainCellClass cls = TerrainCellClass.Regolith;
                    char glyph = ',';

                    if (dPad < 3.6f)
                    {
                        baseWave = Mathf.Lerp(0.15f, baseWave, Mathf.Clamp01((dPad - 1.8f) / 1.8f));
                        cls = TerrainCellClass.Platform;
                    }
                    else if (dCraterA < 6.5f)
                    {
                        float rim = dCraterA > 4.8f ? 1.8f : -2.6f * (1f - dCraterA / 5.0f);
                        baseWave += rim;
                        cls = dCraterA < 3.2f ? TerrainCellClass.Ice : TerrainCellClass.Rock;
                        glyph = dCraterA < 3.2f ? '=' : 'O';
                    }
                    else if (dCraterB < 5.2f)
                    {
                        float rim = dCraterB > 3.8f ? 1.4f : -2.1f * (1f - dCraterB / 4.0f);
                        baseWave += rim;
                        cls = TerrainCellClass.Rock;
                        glyph = 'O';
                    }
                    else
                    {
                        float patch = SmoothValueNoise2D(nx * 1.9f + 11.3f, ny * 1.9f + 4.7f);
                        if (patch > 0.70f) { cls = TerrainCellClass.Rock; glyph = '^'; baseWave += 1.1f; }
                        else if (patch < 0.26f) { cls = TerrainCellClass.Ice; glyph = '='; }
                    }

                    grid.Classes[i] = cls;
                    grid.RawGlyphs[i] = glyph;
                    grid.ElevMeters[i] = baseWave;
                }
            }
            grid.RawGlyphs[padCy * side + padCx] = 'H';
            return grid;
        }

        private static void SynthesizeFallbackArenaRelief(DecodedTerrainGrid grid)
        {
            for (int y = 0; y < grid.Height; y++)
            {
                for (int x = 0; x < grid.Width; x++)
                {
                    int i = y * grid.Width + x;
                    float nx = x * 0.035f;
                    float ny = y * 0.035f;
                    float canyon = Mathf.Abs(y - grid.Height * 0.5f) / (grid.Height * 0.5f);
                    float elev = (SmoothValueNoise2D(nx + 3.1f, ny + 7.4f) - 0.4f) * 8f + canyon * canyon * 14f;
                    TerrainCellClass cls = grid.Biome == "desert" ? TerrainCellClass.Sand : TerrainCellClass.Ground;
                    if (Mathf.Abs(y - grid.Height * 0.5f) <= 2 || Mathf.Abs(x - grid.Width * 0.5f) <= 1)
                    {
                        cls = TerrainCellClass.Road;
                        elev *= 0.35f;
                    }
                    else if (elev > 11f)
                    {
                        cls = TerrainCellClass.Rock;
                    }
                    grid.Classes[i] = cls;
                    grid.ElevMeters[i] = elev;
                }
            }
        }

        private static float SmoothValueNoise2D(float x, float y)
        {
            int x0 = Mathf.FloorToInt(x);
            int y0 = Mathf.FloorToInt(y);
            float tx = x - x0;
            float ty = y - y0;
            tx = tx * tx * (3f - 2f * tx);
            ty = ty * ty * (3f - 2f * ty);

            float v00 = Hash2D(x0, y0);
            float v10 = Hash2D(x0 + 1, y0);
            float v01 = Hash2D(x0, y0 + 1);
            float v11 = Hash2D(x0 + 1, y0 + 1);
            return Mathf.Lerp(Mathf.Lerp(v00, v10, tx), Mathf.Lerp(v01, v11, tx), ty);
        }

        private static float Hash2D(int x, int y)
        {
            uint h = (uint)(x * 374761393 + y * 668265263);
            h = (h ^ (h >> 13)) * 1274126177u;
            return ((h ^ (h >> 16)) & 0xFFFFu) / 65535f;
        }

        private static ArenaZoneRect ParseZoneRect(JsonNode node)
        {
            var arr = node != null ? node.AsArray() : null;
            if (arr == null || arr.Count < 2 || arr[0].Count < 2 || arr[1].Count < 2)
                return null;
            return new ArenaZoneRect
            {
                MinX = arr[0][0].AsInt(0),
                MinY = arr[0][1].AsInt(0),
                MaxX = arr[1][0].AsInt(32),
                MaxY = arr[1][1].AsInt(255)
            };
        }

        public static Color ColorForTerrain(TerrainCellClass cls, string biome, float elevM, char overlayGlyph = '\0')
        {
            if (overlayGlyph == '3') return new Color(0.20f, 0.92f, 0.55f); // Rich ore reading
            if (overlayGlyph == '2') return new Color(0.32f, 0.78f, 0.88f); // Fair ore reading
            if (overlayGlyph == '1') return new Color(0.45f, 0.62f, 0.85f); // Lean ore reading
            if (overlayGlyph == 'v' || overlayGlyph == 'V') return new Color(0.48f, 0.30f, 0.18f); // Worked pit
            if (overlayGlyph == '#') return new Color(0.82f, 0.65f, 0.22f); // Claim boundary

            bool mars = !string.IsNullOrEmpty(biome) && (biome.Contains("desert") || biome.Contains("mars"));
            bool arctic = !string.IsNullOrEmpty(biome) && (biome.Contains("arctic") || biome.Contains("winter"));
            bool airless = !string.IsNullOrEmpty(biome) && (biome.Contains("ceres") || biome.Contains("vesta") || biome.Contains("regolith"));

            float shade = Mathf.Clamp(0.88f + elevM * 0.008f, 0.68f, 1.18f);
            Color c;
            switch (cls)
            {
                case TerrainCellClass.Road:
                case TerrainCellClass.Paved:
                case TerrainCellClass.Platform:
                    c = mars ? new Color(0.44f, 0.34f, 0.28f) : new Color(0.34f, 0.37f, 0.41f);
                    break;
                case TerrainCellClass.Water:
                case TerrainCellClass.WaterDeep:
                    c = new Color(0.12f, 0.34f, 0.56f);
                    break;
                case TerrainCellClass.Ice:
                    c = airless ? new Color(0.72f, 0.84f, 0.94f) : new Color(0.76f, 0.88f, 0.96f);
                    break;
                case TerrainCellClass.Grass:
                    c = arctic ? new Color(0.62f, 0.68f, 0.66f) : new Color(0.26f, 0.42f, 0.24f);
                    break;
                case TerrainCellClass.WoodsLight:
                case TerrainCellClass.WoodsHeavy:
                    c = new Color(0.18f, 0.33f, 0.19f);
                    break;
                case TerrainCellClass.Sand:
                    c = mars ? new Color(0.68f, 0.38f, 0.23f) : new Color(0.64f, 0.56f, 0.38f);
                    break;
                case TerrainCellClass.Rock:
                case TerrainCellClass.Rubble:
                case TerrainCellClass.Wall:
                    c = mars ? new Color(0.46f, 0.26f, 0.19f)
                      : airless ? new Color(0.36f, 0.35f, 0.38f)
                      : new Color(0.40f, 0.41f, 0.43f);
                    break;
                case TerrainCellClass.Regolith:
                    c = mars ? new Color(0.58f, 0.32f, 0.20f) : new Color(0.29f, 0.30f, 0.33f);
                    break;
                default:
                    c = mars ? new Color(0.62f, 0.34f, 0.21f)
                      : arctic ? new Color(0.78f, 0.83f, 0.88f)
                      : airless ? new Color(0.31f, 0.32f, 0.35f)
                      : new Color(0.35f, 0.38f, 0.29f);
                    break;
            }
            return new Color(Mathf.Clamp01(c.r * shade), Mathf.Clamp01(c.g * shade), Mathf.Clamp01(c.b * shade), 1f);
        }

        /// <summary>
        /// Bakes a 2x-supersampled URP Lit albedo splatmap with subtle tactical grid contours,
        /// Red/Blue entry-zone ground markings, and ore reading highlights.
        /// </summary>
        public static Material CreateTerrainSplatMaterial(DecodedTerrainGrid grid, bool showTacticalGrid = true)
        {
            if (grid == null || grid.Width <= 0 || grid.Height <= 0)
                return UrpMaterialFactory.GetOpaque("FallbackTerrain", new Color(0.32f, 0.34f, 0.36f), 0.14f, 0.05f);

            int sub = grid.Width <= 128 ? 4 : 2;
            int texW = grid.Width * sub;
            int texH = grid.Height * sub;
            var tex = new Texture2D(texW, texH, TextureFormat.RGBA32, mipChain: true)
            {
                name = $"Splat_{grid.Id}",
                wrapMode = TextureWrapMode.Clamp,
                filterMode = FilterMode.Bilinear
            };

            var pixels = new Color32[texW * texH];
            for (int cy = 0; cy < grid.Height; cy++)
            {
                for (int cx = 0; cx < grid.Width; cx++)
                {
                    int gi = cy * grid.Width + cx;
                    char ov = (grid.RawGlyphs != null && gi < grid.RawGlyphs.Length) ? grid.RawGlyphs[gi] : '\0';
                    Color baseCol = ColorForTerrain(grid.Classes[gi], grid.Biome, grid.ElevMeters[gi], ov);

                    // Subtle Red / Blue entry zone tint on arena maps
                    if (grid.RedEntryZone != null &&
                        cx >= grid.RedEntryZone.MinX && cx <= grid.RedEntryZone.MaxX &&
                        cy >= grid.RedEntryZone.MinY && cy <= grid.RedEntryZone.MaxY)
                    {
                        bool border = cx == grid.RedEntryZone.MaxX || cx == grid.RedEntryZone.MinX;
                        baseCol = Color.Lerp(baseCol, new Color(0.85f, 0.22f, 0.18f), border ? 0.52f : 0.10f);
                    }
                    else if (grid.BlueEntryZone != null &&
                             cx >= grid.BlueEntryZone.MinX && cx <= grid.BlueEntryZone.MaxX &&
                             cy >= grid.BlueEntryZone.MinY && cy <= grid.BlueEntryZone.MaxY)
                    {
                        bool border = cx == grid.BlueEntryZone.MinX || cx == grid.BlueEntryZone.MaxX;
                        baseCol = Color.Lerp(baseCol, new Color(0.18f, 0.50f, 0.92f), border ? 0.52f : 0.10f);
                    }

                    for (int sy = 0; sy < sub; sy++)
                    {
                        for (int sx = 0; sx < sub; sx++)
                        {
                            Color pxCol = baseCol;
                            if (showTacticalGrid)
                            {
                                bool majorSector = (cx % 16 == 0 && sx == 0) || (cy % 16 == 0 && sy == 0);
                                bool cellEdge = sx == 0 || sy == 0;
                                if (majorSector)
                                    pxCol = Color.Lerp(pxCol, new Color(0.65f, 0.82f, 0.95f), 0.16f);
                                else if (cellEdge)
                                    pxCol *= 0.94f;
                            }
                            pixels[(cy * sub + sy) * texW + (cx * sub + sx)] = pxCol;
                        }
                    }
                }
            }

            tex.SetPixels32(pixels);
            tex.Apply(updateMipmaps: true, makeNoLongerReadable: false);
            return UrpMaterialFactory.CreateTexturedLit(grid.Id, tex, smoothness: 0.15f, metallic: 0.05f);
        }

        /// <summary>
        /// Builds chunked URP Lit 3D heightfield Mesh + MeshCollider tiles under <paramref name="parent"/>.
        /// </summary>
        public static void BuildChunkedTerrainMeshes(DecodedTerrainGrid grid, Transform parent, Material terrainMat = null, int chunkCells = 64)
        {
            if (grid == null || parent == null) return;
            if (terrainMat == null)
                terrainMat = CreateTerrainSplatMaterial(grid);
            int step = Mathf.Clamp(chunkCells, 16, 64);

            for (int cy0 = 0; cy0 < grid.Height; cy0 += step)
            {
                int ch = Mathf.Min(step, grid.Height - cy0);
                for (int cx0 = 0; cx0 < grid.Width; cx0 += step)
                {
                    int cw = Mathf.Min(step, grid.Width - cx0);
                    BuildSingleChunk(grid, parent, terrainMat, cx0, cy0, cw, ch);
                }
            }
        }

        private static void BuildSingleChunk(DecodedTerrainGrid grid, Transform parent, Material mat, int cx0, int cy0, int cw, int ch)
        {
            int vxCount = cw + 1;
            int vzCount = ch + 1;
            var verts = new Vector3[vxCount * vzCount];
            var colors = new Color[vxCount * vzCount];
            var uvs = new Vector2[vxCount * vzCount];
            var tris = new int[cw * ch * 6];

            for (int lz = 0; lz <= ch; lz++)
            {
                int gy = Mathf.Clamp(cy0 + lz, 0, grid.Height - 1);
                for (int lx = 0; lx <= cw; lx++)
                {
                    int gx = Mathf.Clamp(cx0 + lx, 0, grid.Width - 1);
                    int vi = lz * vxCount + lx;
                    int gi = gy * grid.Width + gx;

                    float xPos = (cx0 + lx) * grid.CellSizeM;
                    float zPos = (cy0 + lz) * grid.CellSizeM;
                    float yPos = grid.ElevMeters[gi];

                    verts[vi] = new Vector3(xPos, yPos, zPos);
                    uvs[vi] = new Vector2((cx0 + lx) / (float)grid.Width, (cy0 + lz) / (float)grid.Height);
                    char ov = (grid.RawGlyphs != null && gi < grid.RawGlyphs.Length) ? grid.RawGlyphs[gi] : '\0';
                    colors[vi] = ColorForTerrain(grid.Classes[gi], grid.Biome, yPos, ov);
                }
            }

            int ti = 0;
            for (int lz = 0; lz < ch; lz++)
            {
                for (int lx = 0; lx < cw; lx++)
                {
                    int v00 = lz * vxCount + lx;
                    int v10 = v00 + 1;
                    int v01 = (lz + 1) * vxCount + lx;
                    int v11 = v01 + 1;

                    tris[ti++] = v00;
                    tris[ti++] = v01;
                    tris[ti++] = v10;

                    tris[ti++] = v10;
                    tris[ti++] = v01;
                    tris[ti++] = v11;
                }
            }

            var mesh = new Mesh
            {
                name = $"TerrainChunk_{cx0}_{cy0}"
            };
            mesh.vertices = verts;
            mesh.colors = colors;
            mesh.uv = uvs;
            mesh.triangles = tris;
            mesh.RecalculateNormals();
            mesh.RecalculateBounds();

            var chunkGo = new GameObject($"Terrain_{cx0}_{cy0}");
            chunkGo.transform.SetParent(parent, false);
            chunkGo.transform.localPosition = Vector3.zero;
            var mf = chunkGo.AddComponent<MeshFilter>();
            mf.sharedMesh = mesh;
            var mr = chunkGo.AddComponent<MeshRenderer>();
            mr.sharedMaterial = mat;
            var mc = chunkGo.AddComponent<MeshCollider>();
            mc.sharedMesh = mesh;
        }

        private static string FormatTitle(string slug)
        {
            if (string.IsNullOrEmpty(slug)) return "Surface";
            string[] parts = slug.Split('_');
            for (int i = 0; i < parts.Length; i++)
            {
                if (parts[i].Length > 0)
                    parts[i] = char.ToUpperInvariant(parts[i][0]) + parts[i].Substring(1);
            }
            return string.Join(" ", parts);
        }
    }
}
