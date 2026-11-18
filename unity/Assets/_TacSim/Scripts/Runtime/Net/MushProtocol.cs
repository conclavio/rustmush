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
        public List<MushRequestOption> Options = new List<MushRequestOption>();
    }

    public sealed class MushEnvelope
    {
        public ulong Seq;
        public long TimestampMs;
        public string EventType;
        public JsonNode Data;
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
                Seq = (ulong)Math.Max(0L, root.Long("seq", 0)),
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
                bool italic = style.IsObject && style.Bool("italic", false);
                string hexColor = ResolveSpanHexColor(tag, style, link);

                if (link.IsObject)
                {
                    bold = true;
                    escaped = $"[{escaped}]";
                }

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

        private static string ParseProtoColorHex(JsonNode fg)
        {
            if (fg == null || fg.IsNull) return null;
            if (fg.Kind == JsonKind.String)
            {
                string s = fg.AsString(string.Empty).Trim();
                if (s.StartsWith("#", StringComparison.Ordinal) && (s.Length == 7 || s.Length == 9))
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
            ulong seq = (ulong)Math.Max(0L, viewFrame.Long("sequence", (long)viewFrame.Long("seq", 0)));
            bool hasBaseSeq = (viewFrame.ContainsKey("base_sequence") && !viewFrame["base_sequence"].IsNull) ||
                              (viewFrame.ContainsKey("base") && !viewFrame["base"].IsNull);
            ulong baseSeq = (ulong)Math.Max(0L, viewFrame.Long("base_sequence", (long)viewFrame.Long("base", 0)));

            // Canonical Rust ViewEnvelope distinguishes deltas by base_sequence presence (no "kind" field).
            string explicitKind = viewFrame.Str("kind", string.Empty);
            bool isDelta = hasBaseSeq || string.Equals(explicitKind, "delta", StringComparison.OrdinalIgnoreCase);
            bool isSnapshot = !isDelta;

            if (isSnapshot)
            {
                if (cursor.HasSnapshot && !string.IsNullOrEmpty(gen) && gen == cursor.Generation && seq > 0 && seq < cursor.LastSequence)
                    return ViewCursorDisposition.IgnoreDuplicateOrStale;

                cursor.Generation = gen;
                cursor.LastSequence = seq;
                cursor.HasSnapshot = true;
                return ViewCursorDisposition.ApplySnapshot;
            }

            // Delta frame: requires matching generation and contiguous base_sequence == LastSequence
            if (!cursor.HasSnapshot)
                return ViewCursorDisposition.NeedsResubscribe;
            if (!string.IsNullOrEmpty(gen) && !string.IsNullOrEmpty(cursor.Generation) && gen != cursor.Generation)
                return ViewCursorDisposition.NeedsResubscribe;
            if (seq > 0 && seq <= cursor.LastSequence)
                return ViewCursorDisposition.IgnoreDuplicateOrStale;
            if (baseSeq > 0 && baseSeq != cursor.LastSequence)
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
                Id = (ulong)Math.Max(0L, d.Long("id", 0)),
                Kind = d["kind"].IsObject ? d["kind"].Str("t", "choice") : d.Str("kind", "choice"),
                Prompt = RenderRichTextToUnityMarkup(d["prompt"])
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
                req.Options.Add(new MushRequestOption { Key = "false", Label = "Cancel" });
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

        public static string BuildLineCommand(string commandLine)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("line");
            obj["d"] = JsonNode.FromString(commandLine ?? string.Empty);
            return obj.ToJson();
        }

        public static string BuildPongCommand(ulong nonce)
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("pong");
            var d = JsonNode.NewObject();
            d["nonce"] = JsonNode.FromLong((long)nonce);
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
            d["id"] = JsonNode.FromLong((long)id);

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
        /// Builds canonical ClientCommand::Respond:
        /// {"t":"respond","d":{"epoch":"...","id":1,"choice":{"t":"choose","d":"opt_id"}}}
        /// </summary>
        public static string BuildChoiceResponse(ulong requestId, string choiceKey, string epoch = "", string requestKind = "choice")
        {
            var obj = JsonNode.NewObject();
            obj["t"] = JsonNode.FromString("respond");
            var d = JsonNode.NewObject();
            d["epoch"] = JsonNode.FromString(epoch ?? string.Empty);
            d["id"] = JsonNode.FromLong((long)requestId);

            var choice = JsonNode.NewObject();
            if (requestKind == "confirm")
            {
                choice["t"] = JsonNode.FromString("confirm");
                choice["d"] = JsonNode.FromBool(string.Equals(choiceKey, "true", StringComparison.OrdinalIgnoreCase));
            }
            else if (choiceKey == "__cancel__")
            {
                choice["t"] = JsonNode.FromString("cancel");
            }
            else
            {
                choice["t"] = JsonNode.FromString(requestKind == "text" || requestKind == "target" ? "text" : "choose");
                choice["d"] = JsonNode.FromString(choiceKey ?? string.Empty);
            }

            d["choice"] = choice;
            obj["d"] = d;
            return obj.ToJson();
        }
    }
}
