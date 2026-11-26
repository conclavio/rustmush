using System;
using System.Collections.Generic;
using UnityEngine;

namespace TacSim.Arena
{
    public sealed class ArenaFacetState
    {
        public string Id = string.Empty;
        public int Armor;
        public int ArmorMax;
    }

    public sealed class ArenaSectionState
    {
        public string Id = string.Empty;
        public string Condition = "intact";
        public int Internals;
        public int InternalsMax;
        public readonly List<ArenaFacetState> Facets = new List<ArenaFacetState>(4);
    }

    public sealed class ArenaDollPartSpec
    {
        public string Section = string.Empty;
        public string Facet;
    }

    public sealed class ArenaDollGrid
    {
        public string Label = string.Empty;
        public string View = "top"; // "top" | "upright"
        public int Width;
        public int Height;
        public ArenaDollPartSpec[] Parts = Array.Empty<ArenaDollPartSpec>();
        public int[] Cells = Array.Empty<int>(); // -1 = empty, >= 0 = index into Parts
        public int[] Tones = Array.Empty<int>(); // 0 or 1 greedy 2-tone neighbor shading
    }

    /// <summary>
    /// Doc 22 (`U.4`) Tactical Armor Paper-Doll Schematic matching `crates/mush-pixel/looks/*.json`
    /// and `web/src/lib/look.ts` + `DollView.svelte`. Renders crisp pixel-bordered 2D vehicle/mech
    /// armor &amp; internal-structure schematics into a reusable `Texture2D`.
    /// </summary>
    public static class TacSimArmorPaperDoll
    {
        public const int CellScalePx = 6;
        private static readonly Color32 Transparent = new Color32(0, 0, 0, 0);
        private static readonly Color32 SeamInk = new Color32(14, 18, 22, 255); // #0e1216

        private static readonly Dictionary<string, ArenaDollGrid> ShippedByUnit = BuildShippedRegistry();

        public static ArenaDollGrid FindForUnitType(string unitType)
        {
            if (string.IsNullOrWhiteSpace(unitType)) return ShippedByUnit["tacsim.unit.line_tank"];
            string key = unitType.Trim();
            if (ShippedByUnit.TryGetValue(key, out var exact)) return exact;
            if (!key.StartsWith("tacsim.unit.", StringComparison.OrdinalIgnoreCase) &&
                ShippedByUnit.TryGetValue("tacsim.unit." + key, out var prefixed))
                return prefixed;
            string low = key.ToLowerInvariant();
            if (low.Contains("biped")) return ShippedByUnit["tacsim.unit.biped_mk1"];
            if (low.Contains("scout")) return ShippedByUnit["tacsim.unit.scout_mk1"];
            if (low.Contains("drone")) return ShippedByUnit["tacsim.unit.drone_mk1"];
            if (low.Contains("wreck")) return ShippedByUnit["tacsim.unit.tv_wrecker"];
            if (low.Contains("heavy")) return ShippedByUnit["tacsim.unit.heavy_mk1"];
            return ShippedByUnit["tacsim.unit.line_tank"];
        }

        /// <summary>
        /// Exact port of `web/src/lib/look.ts` `dollColour(left, max, tone)`.
        /// </summary>
        public static Color32 ConditionColor(int left, int max, int tone)
        {
            bool light = tone == 1;
            if (max <= 0 || left <= 0)
                return light ? new Color32(0x6a, 0x6a, 0x72, 255) : new Color32(0x4a, 0x4a, 0x52, 255);
            if ((long)left * 3L >= (long)max * 2L)
                return light ? new Color32(0x7a, 0xd8, 0x8a, 255) : new Color32(0x46, 0xbe, 0x5a, 255);
            if ((long)left * 3L >= max)
                return light ? new Color32(0xf5, 0xd2, 0x6e, 255) : new Color32(0xe6, 0xb4, 0x32, 255);
            return light ? new Color32(0xf0, 0x78, 0x6a, 255) : new Color32(0xdc, 0x3c, 0x32, 255);
        }

        public static bool TryResolvePartCondition(
            ArenaDollPartSpec part,
            IReadOnlyList<ArenaSectionState> sections,
            ArenaCockpitState fallbackState,
            out int left,
            out int max)
        {
            left = 0;
            max = 0;
            if (part == null) return false;

            if (sections != null && sections.Count > 0)
            {
                for (int i = 0; i < sections.Count; i++)
                {
                    var sec = sections[i];
                    if (!string.Equals(sec.Id, part.Section, StringComparison.OrdinalIgnoreCase)) continue;
                    if (!string.IsNullOrEmpty(part.Facet))
                    {
                        for (int f = 0; f < sec.Facets.Count; f++)
                        {
                            if (string.Equals(sec.Facets[f].Id, part.Facet, StringComparison.OrdinalIgnoreCase))
                            {
                                left = sec.Facets[f].Armor;
                                max = sec.Facets[f].ArmorMax;
                                return true;
                            }
                        }
                    }
                    left = string.Equals(sec.Condition, "destroyed", StringComparison.OrdinalIgnoreCase) ? 0 : sec.Internals;
                    max = sec.InternalsMax;
                    return true;
                }
                return false;
            }

            if (fallbackState == null) return false;
            if (fallbackState.IsWreck)
            {
                left = 0;
                max = 100;
                return true;
            }
            if (!string.IsNullOrEmpty(part.Facet))
            {
                max = 100;
                if (part.Facet == "front") { left = fallbackState.ArmorFrontPct; return true; }
                if (part.Facet == "left") { left = fallbackState.ArmorLeftPct; return true; }
                if (part.Facet == "right") { left = fallbackState.ArmorRightPct; return true; }
                if (part.Facet == "rear") { left = fallbackState.ArmorRearPct; return true; }
            }
            left = fallbackState.InternalsPct;
            max = 100;
            return true;
        }

        /// <summary>
        /// Renders the paper doll into a reusable `Texture2D` (creating or resizing only when dimensions change),
        /// drawing crisp 1px dark seam lines around every part boundary matching `DollView.svelte` / `mush-pixel`.
        /// </summary>
        public static Texture2D RenderToTexture(
            ArenaDollGrid doll,
            IReadOnlyList<ArenaSectionState> sections,
            ArenaCockpitState fallbackState,
            Texture2D existingTex,
            ref Color32[] pixelBuffer)
        {
            if (doll == null || doll.Width <= 0 || doll.Height <= 0) return existingTex;

            int texW = doll.Width * CellScalePx + 2;
            int texH = doll.Height * CellScalePx + 2;
            Texture2D tex = existingTex;
            if (tex == null || tex.width != texW || tex.height != texH)
            {
                if (tex != null)
                {
                    if (Application.isPlaying) UnityEngine.Object.Destroy(tex);
                    else UnityEngine.Object.DestroyImmediate(tex);
                }
                tex = new Texture2D(texW, texH, TextureFormat.RGBA32, mipChain: false)
                {
                    filterMode = FilterMode.Point,
                    wrapMode = TextureWrapMode.Clamp,
                    name = $"ArmorDoll_{doll.Label}"
                };
            }

            int totalPx = texW * texH;
            if (pixelBuffer == null || pixelBuffer.Length != totalPx)
                pixelBuffer = new Color32[totalPx];

            for (int i = 0; i < totalPx; i++)
                pixelBuffer[i] = Transparent;

            var partColors = new Color32[doll.Parts.Length];
            for (int p = 0; p < doll.Parts.Length; p++)
            {
                int tone = p < doll.Tones.Length ? doll.Tones[p] : 0;
                if (TryResolvePartCondition(doll.Parts[p], sections, fallbackState, out int left, out int max))
                    partColors[p] = ConditionColor(left, max, tone);
                else
                    partColors[p] = ConditionColor(0, 0, tone);
            }

            int CellAt(int cx, int cy)
            {
                if (cx < 0 || cy < 0 || cx >= doll.Width || cy >= doll.Height) return -1;
                return doll.Cells[cy * doll.Width + cx];
            }

            for (int cy = 0; cy < doll.Height; cy++)
            {
                for (int cx = 0; cx < doll.Width; cx++)
                {
                    int partIdx = CellAt(cx, cy);
                    if (partIdx < 0) continue;

                    Color32 fill = partColors[partIdx];
                    bool seamLeft = CellAt(cx - 1, cy) != partIdx;
                    bool seamRight = CellAt(cx + 1, cy) != partIdx;
                    bool seamTop = CellAt(cx, cy - 1) != partIdx;
                    bool seamBot = CellAt(cx, cy + 1) != partIdx;

                    int baseX = 1 + cx * CellScalePx;
                    // Flip vertically: grid row 0 (front/head) renders at TOP of Texture2D!
                    int baseY = 1 + (doll.Height - 1 - cy) * CellScalePx;

                    for (int dy = 0; dy < CellScalePx; dy++)
                    {
                        int py = baseY + dy;
                        int rowOff = py * texW;
                        bool edgeBot = dy == 0 && seamBot;
                        bool edgeTop = dy == CellScalePx - 1 && seamTop;

                        for (int dx = 0; dx < CellScalePx; dx++)
                        {
                            bool edgeL = dx == 0 && seamLeft;
                            bool edgeR = dx == CellScalePx - 1 && seamRight;
                            pixelBuffer[rowOff + baseX + dx] = (edgeL || edgeR || edgeTop || edgeBot) ? SeamInk : fill;
                        }
                    }
                }
            }

            tex.SetPixels32(pixelBuffer);
            tex.Apply(updateMipmaps: false, makeNoLongerReadable: false);
            return tex;
        }

        private static ArenaDollGrid CompileGrid(
            string label,
            string view,
            string[] rows,
            Dictionary<char, (string section, string facet)> legend)
        {
            var keys = new List<char>(legend.Keys);
            keys.Sort();

            var parts = new ArenaDollPartSpec[keys.Count];
            var keyToIndex = new Dictionary<char, int>(keys.Count);
            for (int i = 0; i < keys.Count; i++)
            {
                char k = keys[i];
                keyToIndex[k] = i;
                var entry = legend[k];
                parts[i] = new ArenaDollPartSpec { Section = entry.section, Facet = entry.facet };
            }

            int h = rows.Length;
            int w = rows[0].Length;
            var cells = new int[w * h];
            for (int y = 0; y < h; y++)
            {
                string row = rows[y];
                for (int x = 0; x < w; x++)
                {
                    char c = row[x];
                    cells[y * w + x] = (c == '.' || c == ' ' || !keyToIndex.TryGetValue(c, out int idx)) ? -1 : idx;
                }
            }

            // Greedy 2-tone neighbor shading (`Doll::tones` / `dollTones`)
            int n = parts.Length;
            var touches = new HashSet<int>[n];
            for (int i = 0; i < n; i++) touches[i] = new HashSet<int>();
            for (int y = 0; y < h; y++)
            {
                for (int x = 0; x < w; x++)
                {
                    int a = cells[y * w + x];
                    if (a < 0) continue;
                    if (x + 1 < w)
                    {
                        int b = cells[y * w + x + 1];
                        if (b >= 0 && b != a) { touches[a].Add(b); touches[b].Add(a); }
                    }
                    if (y + 1 < h)
                    {
                        int b = cells[(y + 1) * w + x];
                        if (b >= 0 && b != a) { touches[a].Add(b); touches[b].Add(a); }
                    }
                }
            }
            var tones = new int[n];
            var assigned = new bool[n];
            for (int p = 0; p < n; p++)
            {
                bool has0 = false, has1 = false;
                foreach (int nb in touches[p])
                {
                    if (!assigned[nb]) continue;
                    if (tones[nb] == 0) has0 = true;
                    else if (tones[nb] == 1) has1 = true;
                }
                tones[p] = (has0 && !has1) ? 1 : 0;
                assigned[p] = true;
            }

            return new ArenaDollGrid
            {
                Label = label,
                View = view,
                Width = w,
                Height = h,
                Parts = parts,
                Cells = cells,
                Tones = tones
            };
        }

        private static Dictionary<string, ArenaDollGrid> BuildShippedRegistry()
        {
            var reg = new Dictionary<string, ArenaDollGrid>(StringComparer.OrdinalIgnoreCase);

            var tankLegend = new Dictionary<char, (string, string)>
            {
                ['F'] = ("hull", "front"),
                ['B'] = ("hull", "rear"),
                ['L'] = ("hull", "left"),
                ['R'] = ("hull", "right"),
                ['H'] = ("hull", null),
                ['T'] = ("turret", null),
                ['E'] = ("engine", null),
                ['l'] = ("left_track", null),
                ['r'] = ("right_track", null)
            };
            string[] tankRows =
            {
                "lll.FFFFFFFTTFFFFFFF.rrr",
                "lll.FFFFFFFTTFFFFFFF.rrr",
                "lll.LHHHHHHTTHHHHHHR.rrr",
                "lll.LHHHHHHTTHHHHHHR.rrr",
                "lll.LHHHTTTTTTTTHHHR.rrr",
                "lll.LHHHTTTTTTTTHHHR.rrr",
                "lll.LHHHTTTTTTTTHHHR.rrr",
                "lll.LHHHTTTTTTTTHHHR.rrr",
                "lll.LHHHTTTTTTTTHHHR.rrr",
                "lll.LHHHHHHHHHHHHHHR.rrr",
                "lll.LHHHHEEEEEEHHHHR.rrr",
                "lll.LHHHHEEEEEEHHHHR.rrr",
                "lll.LHHHHEEEEEEHHHHR.rrr",
                "lll.LHHHHHHHHHHHHHHR.rrr",
                "lll.BBBBBBBBBBBBBBBB.rrr",
                "lll.BBBBBBBBBBBBBBBB.rrr"
            };
            var tankDoll = CompileGrid("Tracked tank", "top", tankRows, tankLegend);
            foreach (string u in new[]
            {
                "tacsim.unit.line_tank", "tacsim.unit.line_tank_laser", "tacsim.unit.line_tank_thermal",
                "tacsim.unit.line_tank_ecm", "tacsim.unit.line_tank_eccm", "tacsim.unit.target_mk1",
                "tacsim.unit.tv_escort", "tacsim.unit.heavy_mk1"
            })
            {
                reg[u] = tankDoll;
            }

            var bipedDoll = CompileGrid(
                "Biped",
                "upright",
                new[]
                {
                    "..........hhhh..........",
                    "..........hhhh..........",
                    "...aaaa.tttttttt.bbbb...",
                    "...aaaa.tttttttt.bbbb...",
                    "...aaaa.tttttttt.bbbb...",
                    "...aaaa.tttttttt.bbbb...",
                    "...aaa..tttttttt..bbb...",
                    "...aaa..tttttttt..bbb...",
                    "........llll..rrrr......",
                    "........llll..rrrr......",
                    "........llll..rrrr......",
                    "........llll..rrrr......",
                    "........llll..rrrr......",
                    "........llll..rrrr......",
                    ".......lllll..rrrrr.....",
                    ".......lllll..rrrrr....."
                },
                new Dictionary<char, (string, string)>
                {
                    ['h'] = ("head", null),
                    ['t'] = ("torso", null),
                    ['a'] = ("left_arm", null),
                    ['b'] = ("right_arm", null),
                    ['l'] = ("left_leg", null),
                    ['r'] = ("right_leg", null)
                });
            reg["tacsim.unit.biped_mk1"] = bipedDoll;
            reg["tacsim.unit.biped_mk2"] = bipedDoll;

            var scoutDoll = CompileGrid(
                "Scout car",
                "top",
                new[]
                {
                    "....FFFFFFFFFFFFFFFF....",
                    "ll..FFFFFFFTTFFFFFFF..rr",
                    "ll..LHHHHHHTTHHHHHHR..rr",
                    "ll..LHHHTTTTTTTTHHHR..rr",
                    "....LHHHTTTTTTTTHHHR....",
                    "....LHHHTTTTTTTTHHHR....",
                    "....LHHHHHHHHHHHHHHR....",
                    "....LHHHHEEEEEEHHHHR....",
                    "ll..LHHHHEEEEEEHHHHR..rr",
                    "ll..LHHHHHHHHHHHHHHR..rr",
                    "ll..BBBBBBBBBBBBBBBB..rr",
                    "....BBBBBBBBBBBBBBBB...."
                },
                new Dictionary<char, (string, string)>
                {
                    ['F'] = ("hull", "front"),
                    ['B'] = ("hull", "rear"),
                    ['L'] = ("hull", "left"),
                    ['R'] = ("hull", "right"),
                    ['H'] = ("hull", null),
                    ['T'] = ("turret", null),
                    ['E'] = ("engine", null),
                    ['l'] = ("left_wheels", null),
                    ['r'] = ("right_wheels", null)
                });
            reg["tacsim.unit.scout_mk1"] = scoutDoll;

            var droneDoll = CompileGrid(
                "Ground drone",
                "top",
                new[]
                {
                    "..FFFFFF..",
                    ".LbbbbbbR.",
                    ".LbbbbbbR.",
                    ".LbbbbbbR.",
                    "..BBBBBB.."
                },
                new Dictionary<char, (string, string)>
                {
                    ['F'] = ("body", "front"),
                    ['B'] = ("body", "rear"),
                    ['L'] = ("body", "left"),
                    ['R'] = ("body", "right"),
                    ['b'] = ("body", null)
                });
            reg["tacsim.unit.drone_mk1"] = droneDoll;

            var wreckerDoll = CompileGrid(
                "Wrecker",
                "top",
                new[]
                {
                    "FFFFFFFFFFFFFFFF",
                    "LHHHHHHHHHHHHHHR",
                    "LHHHHHHHHHHHHHHR",
                    "LssHHHHHHHHHHSSR",
                    "LssHHHHttHHHHSSR",
                    "LssHHHHttHHHHSSR",
                    "LssHHHHHHHHHHSSR",
                    "LHHHHHHHHHHHHHHR",
                    "LHHHMMMMMMMMHHHR",
                    "LHHHMMMMMMMMHHHR",
                    "LHHHMMMMMMMMHHHR",
                    "BBBBBBBBBBBBBBBB"
                },
                new Dictionary<char, (string, string)>
                {
                    ['F'] = ("chassis", "front"),
                    ['B'] = ("chassis", "rear"),
                    ['L'] = ("chassis", "left"),
                    ['R'] = ("chassis", "right"),
                    ['H'] = ("chassis", "top"),
                    ['s'] = ("stores", "left"),
                    ['S'] = ("stores", "right"),
                    ['t'] = ("stores", "top"),
                    ['M'] = ("boom", null)
                });
            reg["tacsim.unit.tv_wrecker"] = wreckerDoll;

            return reg;
        }
    }
}
