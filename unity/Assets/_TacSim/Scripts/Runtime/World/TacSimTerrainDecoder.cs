using System;
using System.Collections.Generic;
using System.IO;
using TacSim.Core;
using TacSim.Net;
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
        public int SpanCells = 1;
        public bool IsFromLiveServerFeed;
        public bool HasAuthoritativeHullCell;
        public int HullFrameX;
        public int HullFrameY;
        public TerrainCellClass[] Classes; // length = Width * Height (row-major, y=0 south -> north)
        public char[] RawGlyphs;           // optional overlay glyphs from site.map / arena.map
        public float[] ElevMeters;         // length = Width * Height

        public bool IsSiteCellInWindow(int siteX, int siteY)
        {
            int span = Math.Max(1, SpanCells);
            return Width > 0 && Height > 0 &&
                   siteX >= OriginCellX && siteY >= OriginCellY &&
                   siteX < OriginCellX + Width * span &&
                   siteY < OriginCellY + Height * span;
        }

        public int LocalToSiteCellX(int localCx) => OriginCellX + Mathf.Clamp(localCx, 0, Math.Max(0, Width - 1)) * Math.Max(1, SpanCells);
        public int LocalToSiteCellY(int localCy) => OriginCellY + Mathf.Clamp(localCy, 0, Math.Max(0, Height - 1)) * Math.Max(1, SpanCells);
        public int SiteToLocalCellX(int siteX) => Mathf.Clamp((siteX - OriginCellX) / Math.Max(1, SpanCells), 0, Math.Max(0, Width - 1));
        public int SiteToLocalCellY(int siteY) => Mathf.Clamp((siteY - OriginCellY) / Math.Max(1, SpanCells), 0, Math.Max(0, Height - 1));

        public int LocalPosToSiteCellX(float localX)
        {
            int span = Math.Max(1, SpanCells);
            float siteCellM = Mathf.Max(0.1f, CellSizeM / span);
            int maxOffset = Math.Max(0, Width * span - 1);
            int offset = Mathf.Clamp(Mathf.FloorToInt(localX / siteCellM), 0, maxOffset);
            return OriginCellX + offset;
        }

        public int LocalPosToSiteCellY(float localZ)
        {
            int span = Math.Max(1, SpanCells);
            float siteCellM = Mathf.Max(0.1f, CellSizeM / span);
            int maxOffset = Math.Max(0, Height * span - 1);
            int offset = Mathf.Clamp(Mathf.FloorToInt(localZ / siteCellM), 0, maxOffset);
            return OriginCellY + offset;
        }

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

            // Match BuildSingleChunk vertex placement: vertex (gx, gy) sits at (gx * CellSizeM, gy * CellSizeM).
            float gx = Mathf.Clamp(localX / Mathf.Max(0.1f, CellSizeM), 0f, Width - 1.001f);
            float gy = Mathf.Clamp(localZ / Mathf.Max(0.1f, CellSizeM), 0f, Height - 1.001f);

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

        /// <summary>
        /// Converts a window-local cell index <c>(0 .. Width-1, 0 .. Height-1)</c> into local mesh metres.
        /// </summary>
        public Vector3 CellToLocalPos(int localCx, int localCy, float yOffset = 0f)
        {
            float lx = (localCx + 0.5f) * CellSizeM;
            float lz = (localCy + 0.5f) * CellSizeM;
            float ly = SampleElevationMeters(lx, lz) + yOffset;
            return new Vector3(lx, ly, lz);
        }

        /// <summary>
        /// Converts an authoritative site/arena cell <c>(siteX, siteY)</c> into local mesh metres,
        /// accounting for <see cref="OriginCellX"/>, <see cref="OriginCellY"/>, and <see cref="SpanCells"/> once.
        /// </summary>
        public Vector3 SiteCellToLocalPos(int siteX, int siteY, float yOffset = 0f)
        {
            float siteCellMeters = CellSizeM / Math.Max(1, SpanCells);
            float maxCoordX = Math.Max(0f, Width * CellSizeM);
            float maxCoordZ = Math.Max(0f, Height * CellSizeM);
            float lx = Mathf.Clamp((siteX - OriginCellX + 0.5f) * siteCellMeters, 0f, maxCoordX);
            float lz = Mathf.Clamp((siteY - OriginCellY + 0.5f) * siteCellMeters, 0f, maxCoordZ);
            float ly = SampleElevationMeters(lx, lz) + yOffset;
            return new Vector3(lx, ly, lz);
        }

        /// <summary>
        /// Converts authoritative site centimetres <c>(x_cm, y_cm)</c> (e.g. from <c>LegWaypoint</c>) into local mesh metres.
        /// </summary>
        public Vector3 SiteCmToLocalPos(double xCm, double yCm, float yOffset = 0f)
        {
            float siteCellMeters = CellSizeM / Math.Max(1, SpanCells);
            double originXCm = OriginCellX * (siteCellMeters * 100.0);
            double originYCm = OriginCellY * (siteCellMeters * 100.0);
            float maxCoordX = Math.Max(0f, Width * CellSizeM);
            float maxCoordZ = Math.Max(0f, Height * CellSizeM);
            float lx = Mathf.Clamp((float)((xCm - originXCm) * 0.01), 0f, maxCoordX);
            float lz = Mathf.Clamp((float)((yCm - originYCm) * 0.01), 0f, maxCoordZ);
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

        public const int MaxGridDimension = 512;
        public const int MaxTileBase64Chars = 4 * 1024 * 1024;

        /// <summary>
        /// Decodes `arena.map` v1 `elev.data` ("zigzag-leb128-delta" centimetres) into int[] elev_cm.
        /// Mirrors crates/tacsim/src/match_/cockpit/frames.rs exact zigzag + LEB128 accumulator.
        /// </summary>
        public static int[] DecodeZigzagLeb128DeltaElevCm(string base64Data, int expectedCells)
        {
            int count = Mathf.Clamp(expectedCells, 0, MaxGridDimension * MaxGridDimension);
            var outCm = new int[count];
            if (string.IsNullOrWhiteSpace(base64Data) || count == 0 || base64Data.Length > MaxTileBase64Chars)
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
            int w = Mathf.Clamp(width, 1, MaxGridDimension);
            int h = Mathf.Clamp(height, 1, MaxGridDimension);
            var grid = new char[w * h];
            for (int i = 0; i < grid.Length; i++) grid[i] = ',';

            var rows = rowsNode != null ? rowsNode.AsArray() : null;
            if (rows == null) return grid;

            int rowCount = Mathf.Min(h, rows.Count);
            for (int y = 0; y < rowCount; y++)
            {
                var runs = rows[y].AsArray();
                if (runs == null) continue;
                int x = 0;
                for (int r = 0; r < runs.Count && x < w; r++)
                {
                    var pair = runs[r].AsArray();
                    if (pair == null || pair.Count < 2) continue;
                    string gStr = pair[0].AsString(",");
                    char ch = !string.IsNullOrEmpty(gStr) ? gStr[0] : ',';
                    int runLen = Mathf.Clamp(pair[1].AsInt(1), 0, w - x);
                    for (int k = 0; k < runLen && x < w; k++)
                    {
                        grid[y * w + x] = ch;
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
            int w = Mathf.Clamp(mapFrame.Int("width", 256), 1, MaxGridDimension);
            int h = Mathf.Clamp(mapFrame.Int("height", 256), 1, MaxGridDimension);
            int cellCm = Mathf.Clamp(mapFrame.Int("cell_cm", 1000), 100, 100000);
            float cellM = cellCm * 0.01f;

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
            int unitCm = Mathf.Clamp(elevObj.IsObject ? elevObj.Int("unit_cm", 1) : 1, 1, 10000);
            float unitScaleM = unitCm * 0.01f;
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
                int spotCap = Math.Min(spotsArr.Count, 32);
                for (int i = 0; i < spotCap; i++)
                {
                    var sp = spotsArr[i];
                    var cellArr = sp["cell"].AsArray();
                    if (cellArr == null || cellArr.Count < 2) continue;
                    string fallbackLetter = ((char)('A' + (i % 26))).ToString();
                    string rawLetter = sp.Str("letter", fallbackLetter);
                    string safeLetter = MushProtocol.TryValidateSafeCommandToken(rawLetter, out string validLetter, maxLen: 16)
                        ? validLetter
                        : fallbackLetter;
                    grid.KeySpots.Add(new ArenaKeySpotDef
                    {
                        Letter = safeLetter,
                        Name = sp.Str("name", $"Point {fallbackLetter}"),
                        CellX = cellArr[0].AsInt(128),
                        CellY = cellArr[1].AsInt(128)
                    });
                }
            }
            return grid;
        }

        /// <summary>
        /// Decodes an authoritative `site.map` panel keyframe (`rows`, `width`, `height`, `site.grid`)
        /// into a 3D `DecodedTerrainGrid` with exact `OriginCellX`, `OriginCellY`, and `SpanCells`
        /// across both 256-cell and 512-cell planetary extraction sites.
        /// </summary>
        public static DecodedTerrainGrid DecodeSiteMapFrame(JsonNode siteMapFrame)
        {
            if (siteMapFrame == null || !siteMapFrame.IsObject || siteMapFrame.Bool("refused", false))
                return null;

            int rawW = siteMapFrame.Int("width", 0);
            int rawH = siteMapFrame.Int("height", 0);
            var rowsArr = siteMapFrame["rows"].AsArray();
            // Reject refusal / reason frames (`width: 0, height: 0, rows: []`) instead of inventing a 4x4 terrain patch!
            if (rawW <= 0 || rawH <= 0 || rowsArr == null || rowsArr.Count == 0)
                return null;

            int w = Mathf.Clamp(rawW, 1, 512);
            int h = Mathf.Clamp(rawH, 1, 512);

            int ox = 0;
            int oy = 0;
            var orig = siteMapFrame["origin_cell"].AsArray();
            if (orig != null && orig.Count >= 2)
            {
                ox = orig[0].AsInt(0);
                oy = orig[1].AsInt(0);
            }

            int span = 1;
            int baseCellCm = Mathf.Clamp(siteMapFrame.Int("cell_cm", 1000), 100, 100000);
            bool hasHullCell = false;
            int hullFx = 0;
            int hullFy = 0;
            var siteObj = siteMapFrame["site"];
            string siteTitle = siteMapFrame.Str("title", "Planetary Site");
            if (siteObj.IsObject)
            {
                siteTitle = siteObj.Str("site", siteTitle);
                var gridObj = siteObj["grid"];
                if (gridObj.IsObject)
                {
                    ox = gridObj.Int("x0", ox);
                    oy = gridObj.Int("y0", oy);
                    span = Mathf.Clamp(gridObj.Int("span", 1), 1, 64);
                    // `site.grid.cell_cm` is a single site cell's edge (1000 cm); multiply by `span` below!
                    baseCellCm = Mathf.Clamp(gridObj.Int("cell_cm", 1000), 100, 100000);
                }
                else if (span > 1)
                {
                    baseCellCm = Mathf.Clamp(baseCellCm / span, 100, 100000);
                }

                var hullArr = siteObj["marks"]?["hull"].AsArray();
                if (hullArr != null && hullArr.Count >= 2)
                {
                    int candidateFx = hullArr[0].AsInt(-1);
                    int candidateFy = hullArr[1].AsInt(-1);
                    if (candidateFx >= 0 && candidateFx < w && candidateFy >= 0 && candidateFy < h)
                    {
                        hasHullCell = true;
                        hullFx = candidateFx;
                        hullFy = candidateFy;
                    }
                }
            }

            float cellSizeM = Mathf.Max(1f, baseCellCm * 0.01f * span);
            char[] glyphs = DecodeRleGlyphRows(siteMapFrame["rows"], w, h);
            var classes = new TerrainCellClass[w * h];
            var elevM = new float[w * h];

            for (int y = 0; y < h; y++)
            {
                for (int x = 0; x < w; x++)
                {
                    int i = y * w + x;
                    char g = glyphs[i];
                    // site.map uses '=' for planetary surface Ice (site_map.rs:967,2318), whereas arena.map uses '=' for Road.
                    TerrainCellClass cls = g == '=' ? TerrainCellClass.Ice : ClassFromGlyph(g);
                    if (hasHullCell && x == hullFx && y == hullFy)
                        cls = TerrainCellClass.Platform;
                    else if (g == ',' || g == '.' || g == ' ' || g == '+')
                        cls = TerrainCellClass.Regolith;
                    classes[i] = cls;

                    int gx = ox + x * span;
                    int gy = oy + y * span;
                    float micro = (SmoothValueNoise2D(gx * 0.09f, gy * 0.09f) - 0.5f) * 1.4f;
                    if (cls == TerrainCellClass.Platform) elevM[i] = 0.15f;
                    else if (g == '^' || g == 'O') elevM[i] = 2.2f + micro * 0.5f;
                    else if (g == 'v' || g == 'V') elevM[i] = -1.3f;
                    else if (cls == TerrainCellClass.Ice) elevM[i] = -0.45f + micro * 0.3f;
                    else elevM[i] = micro;
                }
            }

            string biome = siteTitle.IndexOf("rheasilvia", StringComparison.OrdinalIgnoreCase) >= 0 ||
                           siteTitle.IndexOf("vesta", StringComparison.OrdinalIgnoreCase) >= 0
                ? "vesta"
                : "ceres";

            return new DecodedTerrainGrid
            {
                Id = $"sitemap_{ox}_{oy}_{w}x{h}_s{span}",
                DisplayName = siteTitle,
                Biome = biome,
                Width = w,
                Height = h,
                CellSizeM = cellSizeM,
                OriginCellX = ox,
                OriginCellY = oy,
                SpanCells = span,
                IsFromLiveServerFeed = true,
                HasAuthoritativeHullCell = hasHullCell,
                HullFrameX = hullFx,
                HullFrameY = hullFy,
                Classes = classes,
                RawGlyphs = glyphs,
                ElevMeters = elevM
            };
        }

        /// <summary>
        /// Loads a static 256x256 arena definition (`proving_ground`, `mars_canyon`, `winter_valley`)
        /// from `crates/tacsim/content/arenas/<slug>.json` + `atlas/<slug>.json` 4-quadrant tiles.
        /// </summary>
        public static DecodedTerrainGrid LoadStaticArena(string arenaSlug)
        {
            string slug = string.IsNullOrWhiteSpace(arenaSlug) ? "mars_canyon" : arenaSlug.Trim().ToLowerInvariant();
            bool safeSlug = TacSimContentLoader.IsSafeContentSlug(slug);
            string safeId = safeSlug ? slug : "mars_canyon";
            string arenaPath = safeSlug
                ? TacSimContentLoader.ResolveRepoContentPath(Path.Combine("arenas", $"{slug}.json"))
                : string.Empty;
            string atlasPath = safeSlug
                ? TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", $"{slug}.json"))
                : string.Empty;

            int side = 256;
            float cellM = 10f;
            string biome = safeId.Contains("mars") ? "desert" : safeId.Contains("winter") ? "arctic" : "temperate";

            var grid = new DecodedTerrainGrid
            {
                Id = safeId,
                DisplayName = FormatTitle(safeId),
                Biome = biome,
                Width = side,
                Height = side,
                CellSizeM = cellM,
                Classes = new TerrainCellClass[side * side],
                ElevMeters = new float[side * side]
            };

            string arenaJsonText = null;
            if (safeSlug && TacSimContentLoader.TryGetSyncedOrResourceJson($"arena.{slug}", out string syncedArenaJson))
                arenaJsonText = syncedArenaJson;
            else if (safeSlug && !string.IsNullOrEmpty(arenaPath) && File.Exists(arenaPath))
                arenaJsonText = File.ReadAllText(arenaPath);

            if (!string.IsNullOrEmpty(arenaJsonText))
            {
                JsonNode docRoot = null;
                try
                {
                    docRoot = JsonNode.Parse(arenaJsonText);
                }
                catch (FormatException)
                {
                    docRoot = null;
                }
                if (docRoot != null && docRoot.IsObject)
                {
                JsonNode aDef = docRoot;
                var entriesObj = docRoot["entries"].AsObject();
                if (entriesObj != null)
                {
                    if (entriesObj.TryGetValue($"tacsim.arena.{slug}", out var exactEntry) && exactEntry["def"].IsObject)
                    {
                        aDef = exactEntry["def"];
                    }
                    else
                    {
                        foreach (var kv in entriesObj)
                        {
                            if (kv.Value.Str("kind") == "arena" && kv.Value["def"].IsObject)
                            {
                                aDef = kv.Value["def"];
                                break;
                            }
                        }
                    }
                }

                grid.DisplayName = aDef.Str("label", grid.DisplayName);
                biome = aDef.Str("biome", biome);
                grid.Biome = biome;
                side = Mathf.Clamp(aDef.Int("cols", aDef.Int("width_cells", 256)), 64, MaxGridDimension);
                int hSide = Mathf.Clamp(aDef.Int("rows", aDef.Int("height_cells", side)), 64, MaxGridDimension);
                grid.Width = side;
                grid.Height = hSide;
                grid.CellSizeM = Mathf.Clamp(aDef.Int("cell_cm", 1000), 100, 100000) * 0.01f;
                grid.Classes = new TerrainCellClass[side * hSide];
                grid.ElevMeters = new float[side * hSide];

                var ezNode = aDef["entry_zones"];
                if (ezNode.IsArray)
                {
                    var ezArr = ezNode.AsArray();
                    for (int i = 0; i < ezArr.Count; i++)
                    {
                        var z = ezArr[i];
                        if (!z.IsObject) continue;
                        var rect = new ArenaZoneRect
                        {
                            MinX = z.Int("x0", 0),
                            MinY = z.Int("y0", 0),
                            MaxX = z.Int("x1", 15),
                            MaxY = z.Int("y1", hSide - 1)
                        };
                        if (string.Equals(z.Str("team", string.Empty), "blue", StringComparison.OrdinalIgnoreCase))
                            grid.BlueEntryZone = rect;
                        else
                            grid.RedEntryZone = rect;
                    }
                }
                else if (ezNode.IsObject)
                {
                    grid.RedEntryZone = ParseZoneRect(ezNode["red"]);
                    grid.BlueEntryZone = ParseZoneRect(ezNode["blue"]);
                }

                string holdSpotTarget = null;
                var modesNode = aDef["modes"];
                if (modesNode.IsArray)
                {
                    var mArr = modesNode.AsArray();
                    for (int i = 0; i < mArr.Count; i++)
                    {
                        var m = mArr[i];
                        if (m.IsObject && string.Equals(m.Str("mode", string.Empty), "hold", StringComparison.OrdinalIgnoreCase))
                        {
                            holdSpotTarget = m.Str("spot", "B");
                            grid.HoldRadiusCells = m.Int("radius_cells", 6);
                            break;
                        }
                    }
                }
                else if (modesNode.IsObject && modesNode["hold"].IsObject)
                {
                    holdSpotTarget = modesNode["hold"].Str("spot", "B");
                    grid.HoldRadiusCells = modesNode["hold"].Int("radius_cells", 6);
                }

                var ksArr = aDef["key_spots"].AsArray();
                if (ksArr != null)
                {
                    for (int i = 0; i < ksArr.Count; i++)
                    {
                        var ks = ksArr[i];
                        if (!ks.IsObject) continue;
                        int cx, cy;
                        var c = ks["cell"].AsArray();
                        if (c != null && c.Count >= 2)
                        {
                            cx = c[0].AsInt(side / 2);
                            cy = c[1].AsInt(hSide / 2);
                        }
                        else if (ks.ContainsKey("x") && ks.ContainsKey("y"))
                        {
                            cx = ks.Int("x", side / 2);
                            cy = ks.Int("y", hSide / 2);
                        }
                        else
                        {
                            continue;
                        }

                        string letter = ks.Str("letter", ((char)('A' + i)).ToString());
                        string name = ks.Str("name", $"Objective {letter}");
                        grid.KeySpots.Add(new ArenaKeySpotDef
                        {
                            Letter = letter,
                            Name = name,
                            CellX = cx,
                            CellY = cy
                        });
                        if (!string.IsNullOrEmpty(holdSpotTarget) &&
                            (string.Equals(holdSpotTarget, name, StringComparison.OrdinalIgnoreCase) ||
                             string.Equals(holdSpotTarget, letter, StringComparison.OrdinalIgnoreCase)))
                        {
                            grid.HoldSpotLetter = letter;
                        }
                    }
                }

                var tanksNode = aDef["tanks"];
                if (tanksNode.IsArray)
                {
                    var tArr = tanksNode.AsArray();
                    for (int i = 0; i < tArr.Count; i++)
                    {
                        var tNode = tArr[i];
                        if (!tNode.IsObject) continue;
                        string id = tNode.Str("id", $"slot_{i + 1}");
                        string team = tNode.Str("team", id.StartsWith("blue", StringComparison.OrdinalIgnoreCase) ? "blue" : "red");
                        float xm = tNode.ContainsKey("x_cm")
                            ? tNode.Int("x_cm", 20000) * 0.01f
                            : (tNode.Int("x", 20) + 0.5f) * grid.CellSizeM;
                        float zm = tNode.ContainsKey("y_cm")
                            ? tNode.Int("y_cm", 128000) * 0.01f
                            : (tNode.Int("y", 128) + 0.5f) * grid.CellSizeM;
                        grid.Spawns.Add(new ArenaSpawnPoseDef
                        {
                            SlotKey = id,
                            Party = string.Equals(team, "blue", StringComparison.OrdinalIgnoreCase) ? "B" : "R",
                            Xm = xm,
                            Zm = zm,
                            HeadingDeg = tNode.Int("heading_mdeg", 90000) * 0.001f
                        });
                    }
                }
                else if (tanksNode.IsObject)
                {
                    foreach (var kv in tanksNode.AsObject())
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
            }

            bool loadedTiles = false;
            if (safeSlug && TacSimContentLoader.TryGetSyncedOrResourceJson($"arena_atlas.{slug}", out string syncedAtlasJson))
            {
                loadedTiles = StitchAtlasQuadrantTiles(syncedAtlasJson, grid);
            }
            else if (safeSlug && !string.IsNullOrEmpty(atlasPath) && File.Exists(atlasPath))
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

        public static bool TryStitchAtlasQuadrantTiles(string atlasJson, DecodedTerrainGrid grid)
        {
            return StitchAtlasQuadrantTiles(atlasJson, grid);
        }

        private static bool StitchAtlasQuadrantTiles(string atlasJson, DecodedTerrainGrid grid)
        {
            if (string.IsNullOrWhiteSpace(atlasJson) || grid == null || grid.Classes == null || grid.ElevMeters == null)
                return false;

            JsonNode root;
            try
            {
                root = JsonNode.Parse(atlasJson);
            }
            catch (FormatException)
            {
                return false;
            }
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
                if (classesB64.Length > MaxTileBase64Chars || dataB64.Length > MaxTileBase64Chars) continue;

                byte[] classBytes;
                byte[] elevBytes;
                try
                {
                    classBytes = Convert.FromBase64String(classesB64);
                    elevBytes = Convert.FromBase64String(dataB64);
                }
                catch (FormatException)
                {
                    continue;
                }
                if (classBytes.Length < 4) continue;

                int tw = classBytes[0] | (classBytes[1] << 8);
                int th = classBytes[2] | (classBytes[3] << 8);
                if (tw <= 0 || th <= 0 || tw > MaxGridDimension || th > MaxGridDimension)
                    continue;
                long totalLong = (long)tw * th;
                if (totalLong <= 0 || totalLong > (long)MaxGridDimension * MaxGridDimension)
                    continue;
                int total = (int)totalLong;

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
                    if (gy < 0 || gy >= grid.Height) continue;
                    for (int tx = 0; tx < tw; tx++)
                    {
                        int gx = ox + tx;
                        if (gx < 0 || gx >= grid.Width) continue;
                        long ti = (long)ty * tw + tx;
                        long gi = (long)gy * grid.Width + gx;
                        if (ti < 0 || ti >= tileClasses.Length || gi < 0 || gi >= grid.Classes.Length)
                            continue;
                        grid.Classes[gi] = tileClasses[ti];

                        long elevOff = ti * 2L;
                        if (elevOff + 1L < elevBytes.Length && gi < grid.ElevMeters.Length)
                        {
                            short elevCm = (short)(elevBytes[elevOff] | (elevBytes[elevOff + 1L] << 8));
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
            side = Mathf.Clamp(side, 8, MaxGridDimension);
            string clean = string.IsNullOrWhiteSpace(siteSlug) ? "occator_rim" : siteSlug.Replace("region.", "").Trim().ToLowerInvariant();
            bool safeSlug = TacSimContentLoader.IsSafeContentSlug(clean);
            string safeId = safeSlug ? clean : "occator_rim";
            string path = safeSlug
                ? TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", $"region.{clean}.json"))
                : string.Empty;

            string displayName = FormatTitle(safeId);
            string body = safeId.Contains("rheasilvia") ? "vesta" : "ceres";
            int seed = 2605;
            float reliefM = 3.8f;

            string regionJsonText = null;
            if (safeSlug && TacSimContentLoader.TryGetSyncedOrResourceJson($"region.{clean}", out string syncedRegionJson))
            {
                regionJsonText = syncedRegionJson;
            }
            else if (safeSlug && !string.IsNullOrEmpty(path) && File.Exists(path))
            {
                regionJsonText = File.ReadAllText(path);
            }

            if (!string.IsNullOrEmpty(regionJsonText))
            {
                try
                {
                    var root = JsonNode.Parse(regionJsonText);
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
                catch (FormatException)
                {
                    // Ignore malformed regional JSON and proceed with safe procedural fallback.
                }
            }

            var grid = new DecodedTerrainGrid
            {
                Id = safeId,
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
            tex.Apply(updateMipmaps: true, makeNoLongerReadable: true);
            return UrpMaterialFactory.CreateTexturedLit(grid.Id, tex, smoothness: 0.15f, metallic: 0.05f);
        }

        /// <summary>
        /// Regenerates the terrain splatmap Texture2D/Material in-place on an existing chunk hierarchy
        /// (for same-window `site.map` updates such as radar/seismic scans, ore-family toggles, and claims)
        /// while destroying the superseded GPU Material and Texture2D cleanly.
        /// </summary>
        public static void RefreshTerrainSplatMaterial(DecodedTerrainGrid grid, Transform terrainRoot, bool showTacticalGrid = true)
        {
            if (grid == null || terrainRoot == null) return;
            var renderers = terrainRoot.GetComponentsInChildren<MeshRenderer>(includeInactive: true);
            if (renderers == null || renderers.Length == 0) return;

            var oldMats = new HashSet<Material>();
            var oldTexs = new HashSet<Texture>();
            Material newMat = CreateTerrainSplatMaterial(grid, showTacticalGrid);

            for (int i = 0; i < renderers.Length; i++)
            {
                var mr = renderers[i];
                if (mr == null) continue;
                var prev = mr.sharedMaterial;
                if (prev != null && prev != newMat)
                {
                    oldMats.Add(prev);
                    if (prev.mainTexture != null && prev.mainTexture.name.StartsWith("Splat_", StringComparison.Ordinal))
                        oldTexs.Add(prev.mainTexture);
                }
                mr.sharedMaterial = newMat;
            }

            foreach (var mat in oldMats)
            {
                if (mat == null) continue;
                if (Application.isPlaying) UnityEngine.Object.Destroy(mat);
                else UnityEngine.Object.DestroyImmediate(mat);
            }
            foreach (var tex in oldTexs)
            {
                if (tex == null) continue;
                if (Application.isPlaying) UnityEngine.Object.Destroy(tex);
                else UnityEngine.Object.DestroyImmediate(tex);
            }
        }

        /// <summary>
        /// Explicitly destroys runtime Mesh, Material, and Texture2D assets allocated under <paramref name="terrainRoot"/>
        /// so terrain rebuilds never leak GPU/native memory.
        /// </summary>
        public static void DestroyTerrainChunkResources(Transform terrainRoot)
        {
            if (terrainRoot == null) return;
            var meshes = new HashSet<Mesh>();
            var mats = new HashSet<Material>();
            var texs = new HashSet<Texture>();

            foreach (var mf in terrainRoot.GetComponentsInChildren<MeshFilter>(includeInactive: true))
            {
                if (mf != null && mf.sharedMesh != null)
                {
                    meshes.Add(mf.sharedMesh);
                    mf.sharedMesh = null;
                }
            }
            foreach (var mc in terrainRoot.GetComponentsInChildren<MeshCollider>(includeInactive: true))
            {
                if (mc != null && mc.sharedMesh != null)
                {
                    meshes.Add(mc.sharedMesh);
                    mc.sharedMesh = null;
                }
            }
            foreach (var mr in terrainRoot.GetComponentsInChildren<MeshRenderer>(includeInactive: true))
            {
                if (mr != null && mr.sharedMaterial != null)
                {
                    var m = mr.sharedMaterial;
                    mats.Add(m);
                    if (m.mainTexture != null && m.mainTexture.name.StartsWith("Splat_", StringComparison.Ordinal))
                        texs.Add(m.mainTexture);
                    mr.sharedMaterial = null;
                }
            }

            foreach (var mesh in meshes)
            {
                if (mesh == null) continue;
                if (Application.isPlaying) UnityEngine.Object.Destroy(mesh);
                else UnityEngine.Object.DestroyImmediate(mesh);
            }
            foreach (var mat in mats)
            {
                if (mat == null) continue;
                if (Application.isPlaying) UnityEngine.Object.Destroy(mat);
                else UnityEngine.Object.DestroyImmediate(mat);
            }
            foreach (var tex in texs)
            {
                if (tex == null) continue;
                if (Application.isPlaying) UnityEngine.Object.Destroy(tex);
                else UnityEngine.Object.DestroyImmediate(tex);
            }
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
