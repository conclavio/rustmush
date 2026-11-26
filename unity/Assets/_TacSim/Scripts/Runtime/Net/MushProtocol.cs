using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using TacSim.Core;

namespace TacSim.Net
{
    public sealed class MushObjRef
    {
        public string Id;
        public string Name;
    }

    public sealed class MushExitRef
    {
        public string Name;
        public List<string> Aliases = new List<string>();
    }

    public sealed class AtlasRoomAmbient
    {
        public string Pack;
        public string Entry;
        public string NeighborhoodPack;
        public string NeighborhoodEntry;
        public string SiteEntry;
        public string Surface;
        public List<string> Details = new List<string>();
    }

    public sealed class TacSimDockedHullRef
    {
        public string Id;
        public string Name;
        public string Mark;
        public string Unit;
    }

    public sealed class TacSimStowageAmbient
    {
        public long CargoKg;
        public long CargoLimitKg;
        public long CargoL;
        public long CargoLimitL;
        public long TankKg;
        public long TankLimitKg;
        public long StoresDays;
        public long StoresLimitDays;
    }

    public sealed class TacSimSimulatorSlot
    {
        public string Team;
        public int Index;
        public string PilotId;
        public string PilotName;
        public string Unit;
        public bool Seated;
    }

    public sealed class TacSimRoomAmbient
    {
        public string Mode; // "interior" | "berth_gallery" | "simulator"
        public string HullId;
        public string HullName;
        public string Mark;
        public string Unit;
        public string ClassWord;
        public string RoomTemplateId; // "airlock" | "passage" | "bridge" | "crew" | "hold" | "engineering" | "bay"
        public bool Docked;
        public bool SettledPad;
        public string Port;
        public string PortEntry;
        public int DockedCount;
        public string OwnHull;
        public string OwnHullName;
        public bool OwnHullDockedHere;
        public TacSimStowageAmbient Stowage;
        public TacSimStowageAmbient OwnStowage;
        public List<TacSimDockedHullRef> DockedHulls = new List<TacSimDockedHullRef>();

        // OOC Simulator Centre structured ambient (`mode == "simulator"`)
        public string SimRoom;       // "lobby" | "red" | "blue"
        public bool SimInPod;
        public string SimMatchState; // "idle" | "selection" | "boarding" | "running" | ...
        public int SimAiFill;
        public string SimMyTeam;     // "red" | "blue" | ""
        public int SimMySlot;
        public string SimMyUnit;
        public List<TacSimSimulatorSlot> SimSlots = new List<TacSimSimulatorSlot>();
    }

    public sealed class IcTimeAmbient
    {
        public long Unix;
        public int Hour;
        public int Minute;
        public string Season;
        public string TimeOfDay;
    }

    public sealed class MushSceneAmbient
    {
        public long Id;
        public string Title = string.Empty;
        public string Kind = string.Empty;
        public string Mode = string.Empty;
        public string Status = string.Empty;
        public int Round;
        public int Turn;
        public MushObjRef CurrentActor;
    }

    public sealed class MushRoomView
    {
        public string Id;
        public string Name;
        public string Desc;
        public string RichDesc;
        public List<MushObjRef> Contents = new List<MushObjRef>();
        public List<MushExitRef> Exits = new List<MushExitRef>();
        public MushObjRef Zone;
        public AtlasRoomAmbient Atlas;
        public TacSimRoomAmbient TacSim;
        public IcTimeAmbient IcTime;
        public MushSceneAmbient Scene;
        public HashSet<string> NpcIds = new HashSet<string>(StringComparer.Ordinal);
        public HashSet<string> DormantNpcIds = new HashSet<string>(StringComparer.Ordinal);
        public HashSet<string> PlayerIds = new HashSet<string>(StringComparer.Ordinal);
        public HashSet<string> ThingIds = new HashSet<string>(StringComparer.Ordinal);
        public string WeatherLine;
        public JsonNode RawAmbient;
    }

    public sealed class MushNarrativeEvent
    {
        public string Kind; // "say", "pose", "semi_pose", "emit", "page", "ooc", "whisper"
        public MushObjRef Actor;
        public string Text;
        public string RichMarkup;
    }

    public sealed class MushChannelEvent
    {
        public string Channel;
        public MushObjRef Actor;
        public string Title;
        public string Kind;
        public string Text;
        public string RichMarkup;
    }

    public sealed class MushComposingEvent
    {
        public MushObjRef Actor;
        public bool Active;
    }

    public sealed class MushWhoEntry
    {
        public string Name = string.Empty;
        public string Alias = string.Empty;
        public int IdleSeconds;
        public int ConnSeconds;
        public string Doing = string.Empty;
    }

    public sealed class MushRequestOption
    {
        public string Key;
        public string Label;
    }

    public sealed class MushServerRequest
    {
        public string Epoch;
        public ulong Id;
        public string Kind;
        public string Prompt;
        public long DeadlineMs;
        public List<MushRequestOption> Options = new List<MushRequestOption>();
    }

    public sealed class MushEnvelope
    {
        public ulong Seq;
        public long TimestampMs;
        public string EventType;
        public JsonNode Data;
        internal bool PongAlreadySent;
    }

    public enum ViewCursorDisposition
    {
        ApplySnapshot,
        ApplyDelta,
        IgnoreDuplicateOrStale,
        NeedsResubscribe
    }

    public sealed class ViewStreamCursor
    {
        public string Generation = string.Empty;
        public ulong LastSequence;
        public bool HasSnapshot;
        public int SchemaVersion;
        public bool HasSchemaVersion;
    }

    public static class MushProtocol
    {
        public static MushEnvelope ParseEnvelope(string rawJson)
        {
            var root = JsonNode.Parse(rawJson);
            if (!root.IsObject) return null;

            string eventType = root.Str("t", string.Empty);
            JsonNode eventData = root["d"];
            if (string.IsNullOrEmpty(eventType))
            {
                var ev = root["e"];
                if (!ev.IsObject) return null;
                eventType = ev.Str("t", string.Empty);
                eventData = ev["d"];
                if (string.IsNullOrEmpty(eventType)) return null;
            }

            return new MushEnvelope
            {
                Seq = root.ULong("seq", 0),
                TimestampMs = root.Long("ts", 0),
                EventType = eventType,
                Data = eventData
            };
        }

        public static string FlattenRichText(JsonNode node)
        {
            if (node == null || node.IsNull) return string.Empty;
            if (node.Kind == JsonKind.String) return node.AsString(string.Empty);

            JsonNode spansNode = node.IsArray ? node : node["spans"];
            var arr = spansNode.AsArray();
            if (arr == null || arr.Count == 0)
                return node.Str("text", string.Empty);

            var sb = new StringBuilder(64);
            for (int i = 0; i < arr.Count; i++)
            {
                var sp = arr[i];
                if (sp.Kind == JsonKind.String)
                    sb.Append(sp.AsString(string.Empty));
                else if (sp.IsObject)
                    sb.Append(sp.Str("text", string.Empty));
            }
            return sb.ToString();
        }

        /// <summary>
        /// Renders mush-proto RichText spans (Span.tag, Style.fg/bold/italic, and CommandLink)
        /// into safe Unity uGUI/TMP rich-text markup while escaping literal angle brackets.
        /// </summary>
        public static string RenderRichTextToUnityMarkup(JsonNode node)
        {
            if (node == null || node.IsNull) return string.Empty;
            if (node.Kind == JsonKind.String) return EscapeUnityRichText(node.AsString(string.Empty));

            JsonNode spansNode = node.IsArray ? node : node["spans"];
            var arr = spansNode.AsArray();
            if (arr == null || arr.Count == 0)
                return EscapeUnityRichText(node.Str("text", string.Empty));

            var sb = new StringBuilder(128);
            for (int i = 0; i < arr.Count; i++)
            {
                var sp = arr[i];
                if (sp.Kind == JsonKind.String)
                {
                    sb.Append(EscapeUnityRichText(sp.AsString(string.Empty)));
                    continue;
                }
                if (!sp.IsObject) continue;

                string rawText = sp.Str("text", string.Empty);
                if (string.IsNullOrEmpty(rawText)) continue;
                string escaped = EscapeUnityRichText(rawText);

                string tag = sp.Str("tag", string.Empty);
                var style = sp["style"];
                var link = sp["link"];

                bool bold = style.IsObject && style.Bool("bold", false);
                bool italic = style.IsObject && (style.Bool("italic", false) || style.Bool("underline", false));
                string hexColor = ResolveSpanHexColor(tag, style, link);

                if (link.IsObject)
                {
                    bold = true;
                    escaped = $"[{escaped}]";
                }

                // Emit only tags supported by both legacy uGUI UnityEngine.UI.Text and TextMeshPro (<b>, <i>, <color>).
                if (!string.IsNullOrEmpty(hexColor)) sb.Append("<color=").Append(hexColor).Append('>');
                if (bold) sb.Append("<b>");
                if (italic) sb.Append("<i>");

                sb.Append(escaped);

                if (italic) sb.Append("</i>");
                if (bold) sb.Append("</b>");
                if (!string.IsNullOrEmpty(hexColor)) sb.Append("</color>");
            }
            return sb.ToString();
        }

        public static string EscapeUnityRichText(string raw)
        {
            if (string.IsNullOrEmpty(raw)) return string.Empty;
            if (raw.IndexOf('<') < 0 && raw.IndexOf('>') < 0) return raw;
            return raw.Replace("<", "‹").Replace(">", "›");
        }

        private static string ResolveSpanHexColor(string semanticTag, JsonNode style, JsonNode link)
        {
            if (style != null && style.IsObject)
            {
                var fg = style["fg"];
                string explicitHex = ParseProtoColorHex(fg);
                if (!string.IsNullOrEmpty(explicitHex))
                    return explicitHex;
            }

            if (link != null && link.IsObject)
                return "#66EECC";

            if (!string.IsNullOrEmpty(semanticTag))
            {
                switch (semanticTag.ToLowerInvariant())
                {
                    case "actor":
                    case "player":
                    case "speaker":
                        return "#88DDFF";
                    case "exit":
                        return "#66EEBB";
                    case "object":
                    case "item":
                        return "#FFDD88";
                    case "alert":
                    case "error":
                    case "danger":
                        return "#FF7766";
                    case "muted":
                    case "dim":
                        return "#8899AA";
                    case "system":
                    case "info":
                        return "#99CCEE";
                }
            }
            return null;
        }

        private static readonly string[] Ansi16Palette =
        {
            "#556677", // 0: black (legible on dark terminal)
            "#CC4444", // 1: red
            "#44BB77", // 2: green
            "#DDBB44", // 3: yellow
            "#4488DD", // 4: blue
            "#CC66CC", // 5: magenta
            "#44BBBB", // 6: cyan
            "#CCCCCC", // 7: white
            "#8899AA", // 8: bright black
            "#FF6655", // 9: bright red
            "#66EE99", // 10: bright green
            "#FFDD66", // 11: bright yellow
            "#66AAFF", // 12: bright blue
            "#EE88FF", // 13: bright magenta
            "#66EEFF", // 14: bright cyan
            "#F2F6FA"  // 15: bright white
        };

        private static bool IsValidHexColor(string s)
        {
            if (string.IsNullOrEmpty(s) || s[0] != '#' || (s.Length != 7 && s.Length != 9))
                return false;
            for (int i = 1; i < s.Length; i++)
            {
                char c = s[i];
                bool isHex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
                if (!isHex) return false;
            }
            return true;
        }

        private static string Index256ToHex(int idx)
        {
            int c = Math.Clamp(idx, 0, 255);
            if (c < 16)
                return Ansi16Palette[c];
            if (c < 232)
            {
                int n = c - 16;
                int[] levels = { 0, 95, 135, 175, 215, 255 };
                int r = levels[(n / 36) % 6];
                int g = levels[(n / 6) % 6];
                int b = levels[n % 6];
                return string.Format(CultureInfo.InvariantCulture, "#{0:X2}{1:X2}{2:X2}", r, g, b);
            }
            int gray = 8 + (c - 232) * 10;
            return string.Format(CultureInfo.InvariantCulture, "#{0:X2}{1:X2}{2:X2}", gray, gray, gray);
        }

        private static string ParseProtoColorHex(JsonNode fg)
        {
            if (fg == null || fg.IsNull) return null;
            if (fg.Kind == JsonKind.String)
            {
                string s = fg.AsString(string.Empty).Trim();
                if (IsValidHexColor(s))
                    return s;
                switch (s.ToLowerInvariant())
                {
                    case "red": case "bright_red": return "#FF6655";
                    case "green": case "bright_green": return "#66EE99";
                    case "yellow": case "bright_yellow": return "#FFDD66";
                    case "blue": case "bright_blue": return "#66AAFF";
                    case "magenta": case "bright_magenta": return "#EE88FF";
                    case "cyan": case "bright_cyan": return "#66EEFF";
                    case "white": case "bright_white": return "#F2F6FA";
                    case "black": case "bright_black": return "#8899AA";
                }
            }
            else if (fg.IsObject)
            {
                if (fg.ContainsKey("ansi") && !fg["ansi"].IsNull)
                {
                    int idx = Math.Clamp(fg.Int("ansi", 7), 0, 15);
                    return Ansi16Palette[idx];
                }
                if (fg.ContainsKey("idx256") && !fg["idx256"].IsNull)
                {
                    return Index256ToHex(fg.Int("idx256", 7));
                }
                var rgb = fg["rgb"].AsArray();
                if (rgb != null && rgb.Count >= 3)
                {
                    int r = Math.Clamp(rgb[0].AsInt(255), 0, 255);
                    int g = Math.Clamp(rgb[1].AsInt(255), 0, 255);
                    int b = Math.Clamp(rgb[2].AsInt(255), 0, 255);
                    return string.Format(CultureInfo.InvariantCulture, "#{0:X2}{1:X2}{2:X2}", r, g, b);
                }
                if (fg.ContainsKey("r") && fg.ContainsKey("g") && fg.ContainsKey("b"))
                {
                    int r = Math.Clamp(fg.Int("r", 255), 0, 255);
                    int g = Math.Clamp(fg.Int("g", 255), 0, 255);
                    int b = Math.Clamp(fg.Int("b", 255), 0, 255);
                    return string.Format(CultureInfo.InvariantCulture, "#{0:X2}{1:X2}{2:X2}", r, g, b);
                }
            }
            return null;
        }

        public static ViewCursorDisposition EvaluateViewCursor(ViewStreamCursor cursor, JsonNode viewFrame)
        {
            if (cursor == null || viewFrame == null || !viewFrame.IsObject)
                return ViewCursorDisposition.IgnoreDuplicateOrStale;

            string gen = viewFrame.Str("generation", string.Empty);
            ulong seq = viewFrame.ContainsKey("sequence") && !viewFrame["sequence"].IsNull
                ? viewFrame.ULong("sequence", 0)
                : viewFrame.ULong("seq", 0);
            bool hasBaseSeq = (viewFrame.ContainsKey("base_sequence") && !viewFrame["base_sequence"].IsNull) ||
                              (viewFrame.ContainsKey("base") && !viewFrame["base"].IsNull);
            ulong baseSeq = viewFrame.ContainsKey("base_sequence") && !viewFrame["base_sequence"].IsNull
                ? viewFrame.ULong("base_sequence", 0)
                : viewFrame.ULong("base", 0);
            bool hasSchemaVer = viewFrame.ContainsKey("schema_version") && !viewFrame["schema_version"].IsNull;
            int schemaVer = hasSchemaVer ? viewFrame.Int("schema_version", 0) : 0;

            // Canonical Rust ViewEnvelope distinguishes deltas by base_sequence presence (no "kind" field).
            string explicitKind = viewFrame.Str("kind", string.Empty);
            bool isDelta = hasBaseSeq || string.Equals(explicitKind, "delta", StringComparison.OrdinalIgnoreCase);
            bool isSnapshot = !isDelta;

            if (isSnapshot)
            {
                bool schemaChanged = hasSchemaVer && cursor.HasSchemaVersion && schemaVer != cursor.SchemaVersion;
                if (!schemaChanged && cursor.HasSnapshot && !string.IsNullOrEmpty(gen) && gen == cursor.Generation && seq > 0 && seq <= cursor.LastSequence)
                    return ViewCursorDisposition.IgnoreDuplicateOrStale;

                cursor.Generation = gen;
                cursor.LastSequence = seq;
                cursor.HasSnapshot = true;
                if (hasSchemaVer)
                {
                    cursor.SchemaVersion = schemaVer;
                    cursor.HasSchemaVersion = true;
                }
                return ViewCursorDisposition.ApplySnapshot;
            }

            // Delta frame: requires matching generation, schema_version, and contiguous base_sequence == LastSequence
            if (!cursor.HasSnapshot)
                return ViewCursorDisposition.NeedsResubscribe;
            if (!string.IsNullOrEmpty(gen) && !string.IsNullOrEmpty(cursor.Generation) && gen != cursor.Generation)
                return ViewCursorDisposition.NeedsResubscribe;
            if (hasSchemaVer && cursor.HasSchemaVersion && schemaVer != cursor.SchemaVersion)
                return ViewCursorDisposition.NeedsResubscribe;
            if (seq > 0 && seq <= cursor.LastSequence)
                return ViewCursorDisposition.IgnoreDuplicateOrStale;
            if (hasBaseSeq && baseSeq != cursor.LastSequence)
                return ViewCursorDisposition.NeedsResubscribe;

            cursor.LastSequence = seq > 0 ? seq : cursor.LastSequence + 1;
            return ViewCursorDisposition.ApplyDelta;
        }

        public static MushObjRef ParseObjRef(JsonNode node)
        {
            if (node == null || !node.IsObject) return null;
            string id = node.Str("id", null);
            string name = node.Str("name", null);
            if (string.IsNullOrEmpty(id) && string.IsNullOrEmpty(name)) return null;
            return new MushObjRef { Id = id ?? string.Empty, Name = name ?? string.Empty };
        }

        private static TacSimStowageAmbient ParseStowageAmbient(JsonNode s)
        {
            if (s == null || !s.IsObject) return null;
            return new TacSimStowageAmbient
            {
                CargoKg = s.Long("cargo_kg", 0),
                CargoLimitKg = s.Long("cargo_limit_kg", 100000),
                CargoL = s.Long("cargo_l", 0),
                CargoLimitL = s.Long("cargo_limit_l", 120000),
                TankKg = s.Long("tank_kg", 0),
                TankLimitKg = s.Long("tank_limit_kg", 80000),
                StoresDays = s.Long("stores_days", 0),
                StoresLimitDays = s.Long("stores_limit_days", 120)
            };
        }

        public static MushRoomView ParseRoomView(JsonNode d)
        {
            if (d == null || !d.IsObject) return null;
            var rv = new MushRoomView
            {
                Id = d.Str("id", string.Empty),
                Name = d.Str("name", string.Empty),
                Desc = FlattenRichText(d["desc"]),
                RichDesc = RenderRichTextToUnityMarkup(d["desc"]),
                Zone = ParseObjRef(d["zone"]),
                RawAmbient = d["ambient"]
            };

            var contentsArr = d["contents"].AsArray();
            if (contentsArr != null)
            {
                for (int i = 0; i < contentsArr.Count; i++)
                {
                    var o = ParseObjRef(contentsArr[i]);
                    if (o != null) rv.Contents.Add(o);
                }
            }

            var exitsArr = d["exits"].AsArray();
            if (exitsArr != null)
            {
                for (int i = 0; i < exitsArr.Count; i++)
                {
                    var ex = exitsArr[i];
                    if (!ex.IsObject) continue;
                    var exitRef = new MushExitRef { Name = ex.Str("name", string.Empty) };
                    var aliases = ex["aliases"].AsArray();
                    if (aliases != null)
                    {
                        for (int j = 0; j < aliases.Count; j++)
                        {
                            string al = aliases[j].AsString();
                            if (!string.IsNullOrEmpty(al)) exitRef.Aliases.Add(al);
                        }
                    }
                    rv.Exits.Add(exitRef);
                }
            }

            var amb = d["ambient"];
            if (amb.IsObject)
            {
                var atlas = amb["atlas"];
                if (atlas.IsObject)
                {
                    rv.Atlas = new AtlasRoomAmbient
                    {
                        Pack = atlas.Str("pack", string.Empty),
                        Entry = atlas.Str("entry", string.Empty),
                        NeighborhoodPack = atlas.Str("neighborhood_pack", string.Empty),
                        NeighborhoodEntry = atlas.Str("neighborhood_entry", string.Empty),
                        SiteEntry = atlas.Str("site_entry", string.Empty),
                        Surface = atlas.Str("surface", string.Empty)
                    };
                    var detArr = atlas["details"].AsArray();
                    if (detArr != null)
                    {
                        for (int i = 0; i < detArr.Count; i++)
                        {
                            string kw = detArr[i].AsString();
                            if (!string.IsNullOrEmpty(kw)) rv.Atlas.Details.Add(kw);
                        }
                    }
                }

                var tac = amb["tacsim"];
                if (tac.IsObject)
                {
                    string mode = tac.Str("mode", string.Empty);
                    var parsedTac = new TacSimRoomAmbient
                    {
                        Mode = mode,
                        HullId = tac.Str("hull_id", string.Empty),
                        HullName = tac.Str("hull_name", string.Empty),
                        Mark = tac.Str("mark", string.Empty),
                        Unit = tac.Str("unit", string.Empty),
                        ClassWord = tac.Str("class", string.Empty),
                        RoomTemplateId = mode == "simulator" ? string.Empty : tac.Str("room", string.Empty),
                        Docked = tac.Bool("docked", false),
                        SettledPad = tac.Bool("settled_pad", false),
                        Port = tac.Str("port", string.Empty),
                        PortEntry = tac.Str("port_entry", string.Empty),
                        DockedCount = tac.Int("docked_count", 0),
                        OwnHull = tac.Str("own_hull", string.Empty),
                        OwnHullName = tac.Str("own_hull_name", string.Empty),
                        OwnHullDockedHere = tac.Bool("own_hull_docked_here", false),
                        Stowage = ParseStowageAmbient(tac["stowage"]),
                        OwnStowage = ParseStowageAmbient(tac["own_stowage"]),
                        SimRoom = mode == "simulator" ? tac.Str("room", string.Empty) : string.Empty,
                        SimInPod = tac.Bool("in_pod", false),
                        SimMatchState = tac.Str("match_state", string.Empty),
                        SimAiFill = tac.Int("ai_fill", 0),
                        SimMyTeam = tac.Str("my_team", string.Empty),
                        SimMySlot = tac.Int("my_slot", 0),
                        SimMyUnit = tac.Str("my_unit", string.Empty)
                    };

                    var hullsArr = tac["docked_hulls"].AsArray();
                    if (hullsArr != null)
                    {
                        for (int i = 0; i < hullsArr.Count; i++)
                        {
                            var h = hullsArr[i];
                            if (!h.IsObject) continue;
                            parsedTac.DockedHulls.Add(new TacSimDockedHullRef
                            {
                                Id = h.Str("id", string.Empty),
                                Name = h.Str("name", string.Empty),
                                Mark = h.Str("mark", string.Empty),
                                Unit = h.Str("unit", string.Empty)
                            });
                        }
                    }

                    var slotsArr = tac["slots"].AsArray();
                    if (slotsArr != null)
                    {
                        for (int i = 0; i < slotsArr.Count; i++)
                        {
                            var sl = slotsArr[i];
                            if (!sl.IsObject) continue;
                            parsedTac.SimSlots.Add(new TacSimSimulatorSlot
                            {
                                Team = sl.Str("team", "red"),
                                Index = sl.Int("index", 1),
                                PilotId = sl.Str("pilot", string.Empty),
                                PilotName = sl.Str("pilot_name", string.Empty),
                                Unit = sl.Str("unit", "line_tank"),
                                Seated = sl.Bool("seated", false)
                            });
                        }
                    }

                    rv.TacSim = parsedTac;
                }

                var ic = amb["ic_time"];
                if (ic.IsObject)
                {
                    rv.IcTime = new IcTimeAmbient
                    {
                        Unix = ic.Long("unix", 0),
                        Hour = ic.Int("hour", 0),
                        Minute = ic.Int("minute", 0),
                        Season = ic.Str("season", string.Empty),
                        TimeOfDay = ic.Str("time_of_day", string.Empty)
                    };
                }

                var occ = amb["occupants"];
                if (occ.IsObject)
                {
                    ReadIdSet(occ["npcs"], rv.NpcIds);
                    ReadIdSet(occ["dormant"], rv.DormantNpcIds);
                    ReadIdSet(occ["players"], rv.PlayerIds);
                    ReadIdSet(occ["things"], rv.ThingIds);
                }
                ReadIdSet(amb["npcs"], rv.NpcIds);
                ReadIdSet(amb["dormant_npcs"], rv.DormantNpcIds);
                ReadIdSet(amb["players"], rv.PlayerIds);
                ReadIdSet(amb["things"], rv.ThingIds);

                var wx = amb["weather"];
                if (wx.IsObject)
                {
                    rv.WeatherLine = wx.Str("line", string.Empty);
                }

                var sc = amb["scene"];
                if (sc.IsObject && sc.ContainsKey("id"))
                {
                    rv.Scene = new MushSceneAmbient
                    {
                        Id = sc.Long("id", 0),
                        Title = sc.Str("title", string.Empty),
                        Kind = sc.Str("kind", string.Empty),
                        Mode = sc.Str("mode", string.Empty),
                        Status = sc.Str("status", "active"),
                        Round = sc.Int("round", 1),
                        Turn = sc.Int("turn", 0),
                        CurrentActor = ParseObjRef(sc["current_actor"])
                    };
                }
            }

            return rv;
        }

        private static void ReadIdSet(JsonNode node, HashSet<string> dest)
        {
            var arr = node.AsArray();
            if (arr == null) return;
            for (int i = 0; i < arr.Count; i++)
            {
                string id = arr[i].AsString();
                if (!string.IsNullOrEmpty(id))
                    dest.Add(id);
            }
        }

        public static MushNarrativeEvent ParseNarrative(JsonNode d)
        {
            if (d == null || !d.IsObject) return null;
            return new MushNarrativeEvent
            {
                Kind = d.Str("kind", "say"),
                Actor = ParseObjRef(d["actor"]),
                Text = FlattenRichText(d["text"]),
                RichMarkup = RenderRichTextToUnityMarkup(d["text"])
            };
        }

        public static MushChannelEvent ParseChannel(JsonNode d)
        {
            if (d == null || !d.IsObject) return null;
            return new MushChannelEvent
            {
                Channel = d.Str("channel", string.Empty),
                Actor = ParseObjRef(d["actor"]),
                Title = d.Str("title", string.Empty),
                Kind = d.Str("kind", "say"),
                Text = FlattenRichText(d["text"]),
                RichMarkup = RenderRichTextToUnityMarkup(d["text"])
            };
        }

        public static MushComposingEvent ParseComposing(JsonNode d)
        {
            if (d == null || !d.IsObject) return null;
            var actor = ParseObjRef(d["actor"]);
            if (actor == null) return null;
            return new MushComposingEvent
            {
                Actor = actor,
                Active = d.Bool("active", false)
            };
        }

        public static MushServerRequest ParseRequest(JsonNode d)
        {
            if (d == null || !d.IsObject) return null;
            var req = new MushServerRequest
            {
                Epoch = d.Str("epoch", string.Empty),
                Id = d.ULong("id", 0),
                Kind = d["kind"].IsObject ? d["kind"].Str("t", "choice") : d.Str("kind", "choice"),
                Prompt = RenderRichTextToUnityMarkup(d["prompt"]),
                DeadlineMs = d.Long("deadline_ms", 0)
            };
            var opts = d["options"].AsArray();
            if (opts != null)
            {
                for (int i = 0; i < opts.Count; i++)
                {
                    var o = opts[i];
                    if (!o.IsObject) continue;
                    req.Options.Add(new MushRequestOption
                    {
                        Key = o.Str("id", o.Str("key", i.ToString())),
                        Label = FlattenRichText(o["label"].IsNull ? o["text"] : o["label"])
                    });
                }
            }
            else if (req.Kind == "confirm")
            {
                req.Options.Add(new MushRequestOption { Key = "true", Label = "Confirm" });
                req.Options.Add(new MushRequestOption { Key = "false", Label = "Decline" });
            }
            return req;
        }

        public static string BuildCapsCommand(int width = 120, int height = 40)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("caps");
            var d = JsonNode.NewObject();
            d["kind"] = JsonNode.FromString("web");
            d["color"] = JsonNode.FromString("true_color");
            d["width"] = JsonNode.FromLong(Math.Max(40, width));
            d["height"] = JsonNode.FromLong(Math.Max(20, height));
            d["utf8"] = JsonNode.FromBool(true);
            d["structured"] = JsonNode.FromBool(true);
            obj["d"] = d;
            return obj.ToJson();
        }

        public static string BuildComposingCommand(bool active)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("composing");
            var d = JsonNode.NewObject();
            d["active"] = JsonNode.FromBool(active);
            obj["d"] = d;
            return obj.ToJson();
        }

        public static bool ContainsControlChars(string s)
        {
            if (string.IsNullOrEmpty(s)) return false;
            for (int i = 0; i < s.Length; i++)
            {
                char ch = s[i];
                if (ch < 0x20 || (ch >= 0x7f && ch <= 0x9f))
                    return true;
            }
            return false;
        }

        public static string StripControlChars(string s)
        {
            if (string.IsNullOrEmpty(s)) return string.Empty;
            if (!ContainsControlChars(s)) return s;
            var sb = new StringBuilder(s.Length);
            for (int i = 0; i < s.Length; i++)
            {
                char ch = s[i];
                if (ch >= 0x20 && !(ch >= 0x7f && ch <= 0x9f))
                    sb.Append(ch);
            }
            return sb.ToString();
        }

        /// <summary>
        /// Strict single-token allowlist (<c>^[A-Za-z0-9._-]+$</c>, rejecting leading switches/dots or traversal)
        /// for telemetry IDs interpolated into commands (<c>rock</c>, <c>designator</c>, <c>letter</c>, <c>session_id</c>).
        /// </summary>
        public static bool TryValidateSafeCommandToken(string raw, out string token, int maxLen = 64)
        {
            token = null;
            if (string.IsNullOrWhiteSpace(raw)) return false;
            if (ContainsControlChars(raw)) return false;
            string s = raw.Trim();
            if (s.Length == 0 || s.Length > maxLen) return false;
            if (s[0] == '-' || s[0] == '.' || s.Contains("..", StringComparison.Ordinal)) return false;
            for (int i = 0; i < s.Length; i++)
            {
                char c = s[i];
                bool ok = (c >= 'A' && c <= 'Z') ||
                          (c >= 'a' && c <= 'z') ||
                          (c >= '0' && c <= '9') ||
                          c == '.' || c == '_' || c == '-';
                if (!ok) return false;
            }
            token = s;
            return true;
        }

        /// <summary>
        /// Validates a room detail keyword or actor display name before interpolating it into
        /// <c>look &lt;keyword&gt;</c> or <c>say &lt;name&gt;, ...</c>, rejecting control chars and leading switches/prefixes.
        /// </summary>
        public static bool TryValidateSafeKeywordOrName(string raw, out string cleaned, int maxLen = 80)
        {
            cleaned = null;
            if (string.IsNullOrWhiteSpace(raw)) return false;
            if (ContainsControlChars(raw)) return false;
            string s = raw.Trim();
            if (s.Length == 0 || s.Length > maxLen) return false;
            char first = s[0];
            if (first == '-' || first == '/' || first == '@' || first == '+' ||
                first == '"' || first == ':' || first == ';' || first == '\\')
                return false;
            cleaned = s;
            return true;
        }

        public static string BuildLineCommand(string commandLine)
        {
            string clean = StripControlChars(commandLine ?? string.Empty);
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("line");
            obj["d"] = JsonNode.FromString(clean);
            return obj.ToJson();
        }

        public static string NormalizeListingCode(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw))
                return null;
            string s = raw.Trim().ToLowerInvariant();
            if (s.Length != 8)
                return null;
            for (int i = 0; i < 8; i++)
            {
                char c = s[i];
                if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')))
                    return null;
            }
            return s;
        }

        public static string BuildPongCommand(ulong nonce)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("pong");
            var d = JsonNode.NewObject();
            d["nonce"] = JsonNode.FromULong(nonce);
            obj["d"] = d;
            return obj.ToJson();
        }

        public static List<MushWhoEntry> ParseWhoEntries(JsonNode d)
        {
            var result = new List<MushWhoEntry>();
            if (d == null || !d.IsObject) return result;
            var entries = d["entries"].AsArray() ?? d["players"].AsArray();
            if (entries == null) return result;
            for (int i = 0; i < entries.Count; i++)
            {
                var e = entries[i];
                if (e == null || e.IsNull) continue;
                if (e.Kind == JsonKind.String)
                {
                    string s = e.AsString(string.Empty).Trim();
                    if (!string.IsNullOrEmpty(s))
                        result.Add(new MushWhoEntry { Name = s });
                    continue;
                }
                if (!e.IsObject) continue;
                string nm = e.Str("name", e.Str("character", string.Empty)).Trim();
                if (string.IsNullOrEmpty(nm)) continue;
                result.Add(new MushWhoEntry
                {
                    Name = nm,
                    Alias = e.Str("alias", string.Empty).Trim(),
                    IdleSeconds = Math.Max(0, e.Int("idle_s", e.Int("idle", 0))),
                    ConnSeconds = Math.Max(0, e.Int("conn_s", e.Int("conn", 0))),
                    Doing = e.Str("doing", string.Empty).Trim()
                });
            }
            return result;
        }

        public static string BuildWhoQueryCommand(ulong id)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);
            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("who");
            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        public static string BuildMailListQueryCommand(ulong id, string folder = null)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);
            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("mail_list");
            var qd = JsonNode.NewObject();
            qd["folder"] = !string.IsNullOrWhiteSpace(folder)
                ? JsonNode.FromString(folder.Trim().ToLowerInvariant())
                : JsonNode.Null;
            q["d"] = qd;
            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Req with Query::Commands (`commands.list` panel keyframe):
        /// {"t":"req","d":{"id":1,"q":{"t":"commands"}}}
        /// </summary>
        public static string BuildCommandsQueryCommand(ulong id)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);
            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("commands");
            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Req with Query::Sheet (`<ruleset>.sheet` keyframe):
        /// {"t":"req","d":{"id":1,"q":{"t":"sheet"}}}
        /// </summary>
        public static string BuildSheetQueryCommand(ulong id)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);
            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("sheet");
            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Req with Query::ChannelBacklog (`comsys` panel delta with `backlog`):
        /// {"t":"req","d":{"id":1,"q":{"t":"channel_backlog","d":{"channel":"Public","before_seq":null,"limit":40}}}}
        /// </summary>
        public static string BuildChannelBacklogQueryCommand(ulong id, string channel, ulong? beforeSeq = null, int limit = 40)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);
            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("channel_backlog");
            var qd = JsonNode.NewObject();
            qd["channel"] = JsonNode.FromString((channel ?? string.Empty).Trim());
            if (beforeSeq.HasValue)
                qd["before_seq"] = JsonNode.FromULong(beforeSeq.Value);
            qd["limit"] = JsonNode.FromLong(Math.Max(1, Math.Min(200, limit)));
            q["d"] = qd;
            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Req with Query::NarrativeBacklog (`narrative` panel delta with `backlog`):
        /// {"t":"req","d":{"id":1,"q":{"t":"narrative_backlog","d":{"before_seq":42,"limit":30}}}}
        /// </summary>
        public static string BuildNarrativeBacklogQueryCommand(ulong id, ulong? beforeSeq = null, int limit = 30)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);
            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("narrative_backlog");
            var qd = JsonNode.NewObject();
            if (beforeSeq.HasValue)
                qd["before_seq"] = JsonNode.FromULong(beforeSeq.Value);
            qd["limit"] = JsonNode.FromLong(Math.Max(1, Math.Min(200, limit)));
            q["d"] = qd;
            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Req with Query::PanelSubscribe:
        /// {"t":"req","d":{"id":1,"q":{"t":"panel_subscribe","d":{"feed":"space.cockpit"}}}}
        /// </summary>
        public static string BuildPanelSubscribeCommand(ulong id, string feed, JsonNode queryParams = null)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);

            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("panel_subscribe");
            var qd = JsonNode.NewObject();
            qd["feed"] = JsonNode.FromString(feed ?? string.Empty);
            if (queryParams != null && queryParams.IsObject)
                qd["params"] = queryParams.Clone();
            q["d"] = qd;

            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Req with Query::MindMarket (`kind == "listings"` or `"holdings"`):
        /// {"t":"req","d":{"id":1,"q":{"t":"mind_market","d":{"kind":"listings","after":null}}}}
        /// </summary>
        public static string BuildMindMarketQueryCommand(
            ulong id,
            string kind,
            string section = null,
            string afterListingCode = null,
            long? afterHoldingNumber = null)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("req");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);

            var q = JsonNode.NewObject();
            q["t"] = JsonNode.FromString("mind_market");
            var qd = JsonNode.NewObject();
            if (string.Equals(kind, "holdings", StringComparison.OrdinalIgnoreCase))
            {
                qd["kind"] = JsonNode.FromString("holdings");
                qd["section"] = JsonNode.FromString(
                    string.Equals(section, "owner", StringComparison.OrdinalIgnoreCase) ? "owner" : "paid");
                qd["after"] = afterHoldingNumber.HasValue
                    ? JsonNode.FromLong(afterHoldingNumber.Value)
                    : JsonNode.Null;
            }
            else
            {
                qd["kind"] = JsonNode.FromString("listings");
                qd["after"] = !string.IsNullOrWhiteSpace(afterListingCode)
                    ? JsonNode.FromString(afterListingCode.Trim())
                    : JsonNode.Null;
            }
            q["d"] = qd;
            d["q"] = q;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Invoke for `InvokeRequest::Mind`:
        /// {"t":"invoke","d":{"id":2,"request":{"service":"mind","request":{...}}}}
        /// </summary>
        public static string BuildMindInvokeCommand(ulong id, JsonNode mindRequest)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("invoke");
            var d = JsonNode.NewObject();
            d["id"] = JsonNode.FromULong(id);

            var reqWrap = JsonNode.NewObject();
            reqWrap["service"] = JsonNode.FromString("mind");
            reqWrap["request"] = mindRequest != null ? mindRequest.Clone() : JsonNode.NewObject();
            d["request"] = reqWrap;
            obj["d"] = d;
            return obj.ToJson();
        }

        /// <summary>
        /// Builds canonical ClientCommand::Respond:
        /// {"t":"respond","d":{"epoch":"...","id":1,"choice":{"t":"choose","d":"opt_id"}}}
        /// </summary>
        public static string BuildChoiceResponse(ulong requestId, string choiceKey, string epoch = "", string requestKind = "choice")
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("respond");
            var d = JsonNode.NewObject();
            d["epoch"] = JsonNode.FromString(epoch ?? string.Empty);
            d["id"] = JsonNode.FromULong(requestId);

            var choice = JsonNode.NewObject();
            if (string.Equals(choiceKey, "__cancel__", StringComparison.Ordinal))
            {
                choice["t"] = JsonNode.FromString("cancel");
            }
            else if (string.Equals(requestKind, "confirm", StringComparison.OrdinalIgnoreCase))
            {
                choice["t"] = JsonNode.FromString("confirm");
                choice["d"] = JsonNode.FromBool(string.Equals(choiceKey, "true", StringComparison.OrdinalIgnoreCase));
            }
            else
            {
                bool isText = string.Equals(requestKind, "text", StringComparison.OrdinalIgnoreCase) ||
                              string.Equals(requestKind, "target", StringComparison.OrdinalIgnoreCase);
                choice["t"] = JsonNode.FromString(isText ? "text" : "choose");
                choice["d"] = JsonNode.FromString(choiceKey ?? string.Empty);
            }

            d["choice"] = choice;
            obj["d"] = d;
            return obj.ToJson();
        }
    }
}
