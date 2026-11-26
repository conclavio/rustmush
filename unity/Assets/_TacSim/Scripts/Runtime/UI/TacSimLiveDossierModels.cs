using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;

namespace TacSim.UI
{
    public sealed class LiveSheetFieldEntry
    {
        public string Id = string.Empty;
        public string Label = string.Empty;
        public string Kind = "text";
        public string FormattedValue = "—";
        public bool StaffOnly;
    }

    public sealed class LiveSheetSectionModel
    {
        public string Id = string.Empty;
        public string Label = string.Empty;
        public string Layout = "list";
        public readonly List<LiveSheetFieldEntry> Fields = new List<LiveSheetFieldEntry>();
    }

    /// <summary>
    /// Ruleset-agnostic schema-driven character sheet model matching `web/src/lib/sheet.ts`
    /// (`Query::Sheet` -> `<ruleset>.sheet` panel keyframe). Zero hardcoded ruleset or field names.
    /// </summary>
    public sealed class LiveSheetDocument
    {
        public string Target = string.Empty;
        public string Ruleset = string.Empty;
        public int Version;
        public string ApprovalState = "unknown";
        public bool Approved;
        public readonly List<LiveSheetSectionModel> Sections = new List<LiveSheetSectionModel>();

        public static bool TryDecode(JsonNode raw, out LiveSheetDocument doc)
        {
            doc = null;
            if (raw == null || !raw.IsObject) return false;
            var schema = raw["schema"];
            if (schema == null || !schema.IsObject) return false;
            if (schema["ruleset"].Kind != JsonKind.String) return false;
            if (schema["version"].Kind != JsonKind.Int && schema["version"].Kind != JsonKind.Float) return false;

            var parsed = new LiveSheetDocument
            {
                Target = raw["target"].Kind == JsonKind.String ? raw.Str("target", string.Empty) : string.Empty,
                Ruleset = schema.Str("ruleset", string.Empty),
                Version = schema.Int("version", 1)
            };

            var appr = raw["approved"];
            if (appr != null && appr.IsObject)
            {
                parsed.ApprovalState = appr["state"].Kind == JsonKind.String ? appr.Str("state", "unknown") : "unknown";
                parsed.Approved = appr.Bool("approved", false);
            }

            var dataObj = raw["data"];
            var sectionsArr = schema["sections"].AsArray();
            if (sectionsArr != null)
            {
                int secCap = Math.Min(sectionsArr.Count, 32);
                for (int s = 0; s < secCap; s++)
                {
                    var secRaw = sectionsArr[s];
                    if (secRaw == null || !secRaw.IsObject) continue;
                    string sid = secRaw.Str("id", string.Empty);
                    string slabel = secRaw.Str("label", sid);
                    if (string.IsNullOrEmpty(sid)) continue;

                    string layout = "list";
                    var layoutNode = secRaw["layout"];
                    if (layoutNode.Kind == JsonKind.String)
                        layout = layoutNode.AsString();
                    else if (layoutNode.IsObject && layoutNode["grid"].IsObject)
                        layout = "grid";

                    var secModel = new LiveSheetSectionModel
                    {
                        Id = sid,
                        Label = slabel,
                        Layout = layout
                    };

                    var fieldsArr = secRaw["fields"].AsArray();
                    if (fieldsArr != null)
                    {
                        int fCap = Math.Min(fieldsArr.Count, 64);
                        for (int f = 0; f < fCap; f++)
                        {
                            var fRaw = fieldsArr[f];
                            if (fRaw == null || !fRaw.IsObject) continue;
                            string fid = fRaw.Str("id", string.Empty);
                            string flabel = fRaw.Str("label", fid);
                            var ty = fRaw["ty"];
                            if (string.IsNullOrEmpty(fid) || ty == null || !ty.IsObject) continue;
                            string kind = ty.Str("kind", string.Empty);
                            if (string.IsNullOrEmpty(kind)) continue;

                            bool staffOnly = fRaw.Bool("staff_only", false);
                            bool presentInData = dataObj != null && dataObj.IsObject && dataObj.ContainsKey(fid);
                            if (staffOnly && !presentInData)
                                continue;

                            JsonNode valNode = presentInData ? dataObj[fid] : fRaw["default"];
                            if (kind == "map")
                            {
                                AppendMapFieldEntries(secModel.Fields, fid, flabel, ty["value"], valNode, staffOnly);
                            }
                            else
                            {
                                secModel.Fields.Add(new LiveSheetFieldEntry
                                {
                                    Id = fid,
                                    Label = flabel,
                                    Kind = kind,
                                    StaffOnly = staffOnly,
                                    FormattedValue = FormatTypedValue(ty, valNode)
                                });
                            }
                        }
                    }

                    if (secModel.Fields.Count > 0)
                        parsed.Sections.Add(secModel);
                }
            }

            doc = parsed;
            return true;
        }

        private static void AppendMapFieldEntries(
            List<LiveSheetFieldEntry> dest,
            string fid,
            string flabel,
            JsonNode valueType,
            JsonNode mapNode,
            bool staffOnly)
        {
            var dict = mapNode != null && mapNode.IsObject ? mapNode.AsObject() : null;
            if (dict == null || dict.Count == 0)
            {
                dest.Add(new LiveSheetFieldEntry
                {
                    Id = fid,
                    Label = flabel,
                    Kind = "map",
                    StaffOnly = staffOnly,
                    FormattedValue = "None"
                });
                return;
            }

            var keys = new List<string>(dict.Keys);
            keys.Sort(StringComparer.OrdinalIgnoreCase);
            int kCap = Math.Min(keys.Count, 32);
            for (int i = 0; i < kCap; i++)
            {
                string k = keys[i];
                dest.Add(new LiveSheetFieldEntry
                {
                    Id = $"{fid}.{k}",
                    Label = k,
                    Kind = valueType != null && valueType.IsObject ? valueType.Str("kind", "text") : "text",
                    StaffOnly = staffOnly,
                    FormattedValue = FormatTypedValue(valueType, dict[k])
                });
            }
        }

        public static string FormatTypedValue(JsonNode ty, JsonNode val)
        {
            if (ty == null || !ty.IsObject) return FormatScalarFallback(val);
            string kind = ty.Str("kind", string.Empty);

            switch (kind)
            {
                case "dots":
                {
                    int max = Mathf.Clamp(ty.Int("max", 5), 1, 12);
                    int v = Mathf.Clamp(val != null ? val.AsInt(0) : 0, 0, max);
                    return FormatPips(v, max);
                }
                case "number":
                {
                    int min = ty.Int("min", 0);
                    int max = ty.Int("max", 10);
                    int v = val != null ? val.AsInt(min) : min;
                    if (min == 0 && max >= 1 && max <= 10)
                        return FormatPips(Mathf.Clamp(v, 0, max), max);
                    int span = Math.Max(1, max - min);
                    int pct = Mathf.Clamp(Mathf.RoundToInt(((v - min) * 100f) / span), 0, 100);
                    return $"{v}/{max} ({pct}%)";
                }
                case "text":
                {
                    string txt = val != null && val.Kind == JsonKind.String ? val.AsString() : string.Empty;
                    return string.IsNullOrWhiteSpace(txt) ? "—" : txt.Trim();
                }
                case "enum":
                {
                    string txt = val != null && val.Kind == JsonKind.String ? val.AsString().Trim() : string.Empty;
                    if (string.IsNullOrEmpty(txt)) return "—";
                    return char.ToUpperInvariant(txt[0]) + (txt.Length > 1 ? txt.Substring(1) : string.Empty);
                }
                case "list":
                {
                    var arr = val != null ? val.AsArray() : null;
                    if (arr == null || arr.Count == 0) return "—";
                    var items = new List<string>(Math.Min(arr.Count, 12));
                    for (int i = 0; i < arr.Count && i < 12; i++)
                    {
                        string s = FormatScalarFallback(arr[i]);
                        if (!string.IsNullOrEmpty(s)) items.Add(s);
                    }
                    return items.Count > 0 ? string.Join(", ", items) : "—";
                }
                case "track":
                    return FormatTrack(val, ty["levels"].AsArray());
                default:
                    return FormatScalarFallback(val);
            }
        }

        private static string FormatPips(int filled, int max)
        {
            var sb = new StringBuilder(max + 6);
            for (int i = 0; i < max; i++)
                sb.Append(i < filled ? '●' : '○');
            sb.Append(' ').Append(filled).Append('/').Append(max);
            return sb.ToString();
        }

        public static string FormatTrackBoxes(List<string> boxes, string levelLabel = null, int levelPenalty = 0)
        {
            var sb = new StringBuilder(48);
            if (boxes != null)
            {
                for (int i = 0; i < boxes.Count && i < 16; i++)
                {
                    sb.Append('[').Append(TrackBoxGlyph(boxes[i])).Append(']');
                }
            }
            if (!string.IsNullOrEmpty(levelLabel))
            {
                sb.Append(' ').Append(levelLabel);
                if (levelPenalty < 0)
                    sb.Append(" (").Append(levelPenalty).Append(')');
            }
            return sb.Length > 0 ? sb.ToString() : "—";
        }

        public static string TrackBoxGlyph(string mark)
        {
            if (string.IsNullOrEmpty(mark)) return " ";
            switch (mark)
            {
                case "bashing": return "/";
                case "lethal": return "X";
                case "aggravated": return "*";
                default: return char.ToUpperInvariant(mark[0]).ToString(CultureInfo.InvariantCulture);
            }
        }

        private static string FormatTrack(JsonNode val, List<JsonNode> levels)
        {
            var boxes = new List<string>();
            var arr = val != null ? val.AsArray() : null;
            int filledCount = 0;
            if (arr != null)
            {
                for (int i = 0; i < arr.Count && i < 16; i++)
                {
                    string mk = arr[i].Kind == JsonKind.String ? arr[i].AsString() : null;
                    boxes.Add(mk);
                    if (!string.IsNullOrEmpty(mk)) filledCount++;
                }
            }

            string levelLabel = null;
            int penalty = 0;
            if (filledCount > 0 && levels != null && levels.Count > 0)
            {
                int idx = Mathf.Clamp(filledCount - 1, 0, levels.Count - 1);
                var lv = levels[idx];
                if (lv != null && lv.IsObject)
                {
                    levelLabel = lv.Str("label", lv.Str("id", string.Empty));
                    penalty = lv.Int("penalty", 0);
                }
            }
            return FormatTrackBoxes(boxes, levelLabel, penalty);
        }

        private static string FormatScalarFallback(JsonNode val)
        {
            if (val == null || val.IsNull) return "—";
            switch (val.Kind)
            {
                case JsonKind.String: return val.AsString();
                case JsonKind.Bool: return val.AsBool() ? "Yes" : "No";
                case JsonKind.Int: return val.AsLong().ToString(CultureInfo.InvariantCulture);
                case JsonKind.Float: return val.AsDouble().ToString("0.##", CultureInfo.InvariantCulture);
                default: return "—";
            }
        }

        public string FormatRichSummary()
        {
            var sb = new StringBuilder(512);
            string apprCol = Approved ? "#66FF99" : (ApprovalState == "pending" ? "#FFCC55" : "#88AAC8");
            sb.AppendFormat("<color=#66EEFF><b>RULESET SHEET</b></color> [{0} v{1}]  <color={2}><b>[{3}]</b></color>\n",
                MushProtocol.EscapeUnityRichText(Ruleset.ToUpperInvariant()),
                Version,
                apprCol,
                MushProtocol.EscapeUnityRichText(ApprovalState.ToUpperInvariant()));

            int shownLines = 0;
            for (int s = 0; s < Sections.Count && shownLines < 28; s++)
            {
                var sec = Sections[s];
                sb.AppendFormat("\n<color=#FFCC66><b>{0}</b></color>\n", MushProtocol.EscapeUnityRichText(sec.Label.ToUpperInvariant()));
                shownLines++;
                for (int f = 0; f < sec.Fields.Count && shownLines < 28; f++)
                {
                    var fld = sec.Fields[f];
                    string star = fld.StaffOnly ? " <color=#FFCC55>*</color>" : string.Empty;
                    sb.AppendFormat("• {0}{1}: <b>{2}</b>\n",
                        MushProtocol.EscapeUnityRichText(fld.Label),
                        star,
                        MushProtocol.EscapeUnityRichText(fld.FormattedValue));
                    shownLines++;
                }
            }
            return sb.ToString();
        }
    }

    public sealed class LiveVitalItem
    {
        public string Id = string.Empty;
        public string Label = string.Empty;
        public string Role = "readout"; // dominant | track | pool | readout
        public string State = "ok";     // ok | warn | crit | down
        public string ValueKind = "text"; // bounded | track | text
        public double Current;
        public double Min;
        public double Max;
        public readonly List<string> Boxes = new List<string>();
        public string Level;
        public string Text = string.Empty;
        public string Detail;

        public int RoleRank
        {
            get
            {
                switch (Role)
                {
                    case "dominant": return 0;
                    case "track": return 1;
                    case "pool": return 2;
                    default: return 3;
                }
            }
        }
    }

    public sealed class LiveVitalsFrame
    {
        public string Feed = string.Empty;
        public string SubjectId = string.Empty;
        public string SubjectName = string.Empty;
        public long Revision;
        public readonly List<LiveVitalItem> Vitals = new List<LiveVitalItem>();

        public static bool TryDecode(string feed, JsonNode raw, out LiveVitalsFrame frame)
        {
            frame = null;
            if (raw == null || !raw.IsObject) return false;
            var subj = raw["subject"];
            if (subj == null || !subj.IsObject) return false;
            if (subj["id"].Kind != JsonKind.String || subj["name"].Kind != JsonKind.String) return false;
            if (raw["revision"].Kind != JsonKind.Int && raw["revision"].Kind != JsonKind.Float) return false;
            var arr = raw["vitals"].AsArray();
            if (arr == null) return false;

            var parsed = new LiveVitalsFrame
            {
                Feed = feed ?? string.Empty,
                SubjectId = subj.Str("id", string.Empty),
                SubjectName = subj.Str("name", string.Empty),
                Revision = raw.Long("revision", 0)
            };

            int cap = Math.Min(arr.Count, 16);
            for (int i = 0; i < cap; i++)
            {
                var v = arr[i];
                if (v == null || !v.IsObject) continue;
                string id = v["id"].Kind == JsonKind.String ? v.Str("id", string.Empty) : string.Empty;
                string label = v["label"].Kind == JsonKind.String ? v.Str("label", string.Empty) : string.Empty;
                string role = v["role"].Kind == JsonKind.String ? v.Str("role", string.Empty) : string.Empty;
                string state = v["state"].Kind == JsonKind.String ? v.Str("state", string.Empty) : string.Empty;
                if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(label)) continue;
                if (role != "dominant" && role != "track" && role != "pool" && role != "readout") continue;
                if (state != "ok" && state != "warn" && state != "crit" && state != "down") continue;

                var val = v["value"];
                if (val == null || !val.IsObject || val["kind"].Kind != JsonKind.String) continue;
                string vk = val.Str("kind", string.Empty);

                var item = new LiveVitalItem
                {
                    Id = id,
                    Label = label,
                    Role = role,
                    State = state,
                    ValueKind = vk,
                    Detail = v["detail"].Kind == JsonKind.String ? v.Str("detail", null) : null
                };

                if (vk == "bounded")
                {
                    if ((val["current"].Kind != JsonKind.Int && val["current"].Kind != JsonKind.Float) ||
                        (val["min"].Kind != JsonKind.Int && val["min"].Kind != JsonKind.Float) ||
                        (val["max"].Kind != JsonKind.Int && val["max"].Kind != JsonKind.Float))
                        continue;
                    item.Current = val.Double("current", 0.0);
                    item.Min = val.Double("min", 0.0);
                    item.Max = val.Double("max", 1.0);
                }
                else if (vk == "track")
                {
                    var boxes = val["boxes"].AsArray();
                    if (boxes == null) continue;
                    for (int b = 0; b < boxes.Count && b < 16; b++)
                        item.Boxes.Add(boxes[b].Kind == JsonKind.String ? boxes[b].AsString() : null);
                    item.Level = val["level"].Kind == JsonKind.String ? val.Str("level", null) : null;
                }
                else if (vk == "text")
                {
                    if (val["text"].Kind != JsonKind.String) continue;
                    item.Text = val.Str("text", string.Empty);
                }
                else
                {
                    continue;
                }

                parsed.Vitals.Add(item);
            }

            parsed.Vitals.Sort((a, b) => a.RoleRank.CompareTo(b.RoleRank));
            frame = parsed;
            return true;
        }
    }

    /// <summary>
    /// Monotonic-revision store for all `<ruleset>.vitals` feeds matching `web/src/lib/stores/vitals.svelte.ts`.
    /// </summary>
    public sealed class LiveVitalsStore
    {
        private readonly Dictionary<string, LiveVitalsFrame> _groups = new Dictionary<string, LiveVitalsFrame>(StringComparer.Ordinal);
        private readonly HashSet<string> _acceptNext = new HashSet<string>(StringComparer.Ordinal);

        public int GroupCount => _groups.Count;

        public bool Apply(string feed, JsonNode raw)
        {
            if (string.IsNullOrEmpty(feed)) return false;
            if (raw == null || raw.IsNull)
            {
                _acceptNext.Remove(feed);
                _groups.Remove(feed);
                return true;
            }
            if (!LiveVitalsFrame.TryDecode(feed, raw, out var frame))
                return false;

            bool acceptNext = _acceptNext.Remove(feed);
            if (!acceptNext && _groups.TryGetValue(feed, out var existing) &&
                string.Equals(existing.SubjectId, frame.SubjectId, StringComparison.Ordinal) &&
                frame.Revision != 0 &&
                existing.Revision > frame.Revision)
            {
                return false;
            }

            _groups[feed] = frame;
            return true;
        }

        public void ResetWatermarks()
        {
            _acceptNext.Clear();
            foreach (string feed in _groups.Keys)
                _acceptNext.Add(feed);
        }

        public void Clear()
        {
            _groups.Clear();
            _acceptNext.Clear();
        }

        public List<LiveVitalsFrame> GetOrderedGroups()
        {
            var list = new List<LiveVitalsFrame>(_groups.Values);
            list.Sort((a, b) => string.Compare(a.Feed, b.Feed, StringComparison.Ordinal));
            return list;
        }

        public string FormatRichSummary(bool compact = false)
        {
            var groups = GetOrderedGroups();
            if (groups.Count == 0) return string.Empty;

            var sb = new StringBuilder(384);
            for (int g = 0; g < groups.Count; g++)
            {
                var grp = groups[g];
                if (!compact)
                {
                    sb.AppendFormat("<color=#66EEFF><b>VITALS: {0}</b></color> <color=#88AAC8>(rev {1})</color>\n",
                        MushProtocol.EscapeUnityRichText(grp.SubjectName),
                        grp.Revision);
                }
                int limit = compact ? Math.Min(grp.Vitals.Count, 4) : grp.Vitals.Count;
                for (int i = 0; i < limit; i++)
                {
                    var v = grp.Vitals[i];
                    string col = GaugeStateColorHex(v.State);
                    string valStr;
                    if (v.ValueKind == "bounded")
                    {
                        double span = Math.Max(1e-6, v.Max - v.Min);
                        int pct = Mathf.Clamp(Mathf.RoundToInt((float)(((v.Current - v.Min) / span) * 100.0)), 0, 100);
                        valStr = $"{v.Current:0.##}/{v.Max:0.##} ({pct}%)";
                    }
                    else if (v.ValueKind == "track")
                    {
                        valStr = LiveSheetDocument.FormatTrackBoxes(v.Boxes, v.Level);
                    }
                    else
                    {
                        valStr = v.Text;
                    }
                    string det = !string.IsNullOrEmpty(v.Detail) ? $" — {MushProtocol.EscapeUnityRichText(v.Detail)}" : string.Empty;
                    sb.AppendFormat("• {0}: <color={1}><b>{2}</b></color>{3}\n",
                        MushProtocol.EscapeUnityRichText(v.Label),
                        col,
                        MushProtocol.EscapeUnityRichText(valStr),
                        det);
                }
            }
            return sb.ToString().TrimEnd();
        }

        public static string GaugeStateColorHex(string state)
        {
            switch (state)
            {
                case "ok": return "#66FF99";
                case "warn": return "#FFCC55";
                case "crit": return "#FF7744";
                case "down": return "#FF4455";
                default: return "#DDEEFF";
            }
        }
    }

    public sealed class LiveUnitCellModel
    {
        public string Addr = string.Empty;
        public string Label = string.Empty;
        public double Current;
        public double Min;
        public double Max;
        public string State = string.Empty;
    }

    public sealed class LiveUnitFacetModel
    {
        public string Id = string.Empty;
        public readonly List<string> CellAddrs = new List<string>();
    }

    public sealed class LiveUnitSectionModel
    {
        public string Id = string.Empty;
        public string Condition = string.Empty;
        public string State = string.Empty;
        public readonly List<LiveUnitFacetModel> Facets = new List<LiveUnitFacetModel>();
    }

    public sealed class LiveUnitComponentModel
    {
        public string Id = string.Empty;
        public string Definition = string.Empty;
        public string Install = string.Empty;
        public string Condition = string.Empty;
        public string State = string.Empty;
    }

    public sealed class LiveUnitCapabilityModel
    {
        public string Id = string.Empty;
        public string Source;
    }

    /// <summary>
    /// Module-generic `*.unit` (`tactics.unit`) keyframe model matching `web/src/lib/stores/unit.svelte.ts`.
    /// Prints every ruleset condition word verbatim and interprets ONLY `ok|warn|crit|down` gauge states.
    /// </summary>
    public sealed class LiveUnitChassisView
    {
        public string ObjectId = string.Empty;
        public string ObjectName = string.Empty;
        public long Revision;
        public string State = string.Empty;
        public string Record;
        public string MobilityMode;
        public string SwitchingTo;
        public bool Truncated;

        public readonly List<LiveUnitSectionModel> Sections = new List<LiveUnitSectionModel>();
        public readonly List<LiveUnitCellModel> Cells = new List<LiveUnitCellModel>();
        public readonly Dictionary<string, LiveUnitCellModel> CellByAddr = new Dictionary<string, LiveUnitCellModel>(StringComparer.Ordinal);
        public readonly List<LiveUnitComponentModel> Components = new List<LiveUnitComponentModel>();
        public readonly List<LiveUnitCapabilityModel> Capabilities = new List<LiveUnitCapabilityModel>();

        private static string NormalizeGaugeState(JsonNode node)
        {
            if (node == null || node.Kind != JsonKind.String) return string.Empty;
            string s = node.AsString();
            return (s == "ok" || s == "warn" || s == "crit" || s == "down") ? s : string.Empty;
        }

        public static bool TryDecode(JsonNode raw, out LiveUnitChassisView view)
        {
            view = null;
            if (raw == null || !raw.IsObject) return false;
            var obj = raw["object"];
            if (obj == null || !obj.IsObject || obj["id"].Kind != JsonKind.String) return false;
            if (raw["revision"].Kind != JsonKind.Int && raw["revision"].Kind != JsonKind.Float) return false;

            var parsed = new LiveUnitChassisView
            {
                ObjectId = obj.Str("id", string.Empty),
                ObjectName = obj["name"].Kind == JsonKind.String ? obj.Str("name", string.Empty) : obj.Str("id", string.Empty),
                Revision = raw.Long("revision", 0),
                State = NormalizeGaugeState(raw["state"]),
                Record = raw["record"].Kind == JsonKind.String ? raw.Str("record", null) : null,
                Truncated = raw.Bool("truncated", false)
            };

            var mob = raw["mobility"];
            if (mob != null && mob.IsObject)
            {
                parsed.MobilityMode = mob["mode"].Kind == JsonKind.String ? mob.Str("mode", null) : null;
                var sw = mob["switching"];
                if (sw != null && sw.IsObject && sw["to"].Kind == JsonKind.String)
                    parsed.SwitchingTo = sw.Str("to", null);
            }

            var cellsArr = raw["cells"].AsArray();
            if (cellsArr != null)
            {
                int cap = Math.Min(cellsArr.Count, 64);
                for (int i = 0; i < cap; i++)
                {
                    var c = cellsArr[i];
                    if (c == null || !c.IsObject || c["addr"].Kind != JsonKind.String) continue;
                    string addr = c.Str("addr", string.Empty);
                    var cm = new LiveUnitCellModel
                    {
                        Addr = addr,
                        Label = c["label"].Kind == JsonKind.String ? c.Str("label", addr) : addr,
                        Current = c.Double("current", 0.0),
                        Min = c.Double("min", 0.0),
                        Max = c.Double("max", 1.0),
                        State = NormalizeGaugeState(c["state"])
                    };
                    parsed.Cells.Add(cm);
                    parsed.CellByAddr[addr] = cm;
                }
            }

            var secArr = raw["sections"].AsArray();
            if (secArr != null)
            {
                int cap = Math.Min(secArr.Count, 24);
                for (int i = 0; i < cap; i++)
                {
                    var s = secArr[i];
                    if (s == null || !s.IsObject || s["id"].Kind != JsonKind.String) continue;
                    var sm = new LiveUnitSectionModel
                    {
                        Id = s.Str("id", string.Empty),
                        Condition = s["condition"].Kind == JsonKind.String ? s.Str("condition", string.Empty) : string.Empty,
                        State = NormalizeGaugeState(s["state"])
                    };
                    var fArr = s["facets"].AsArray();
                    if (fArr != null)
                    {
                        for (int f = 0; f < fArr.Count && f < 16; f++)
                        {
                            var fr = fArr[f];
                            if (fr == null || !fr.IsObject || fr["id"].Kind != JsonKind.String) continue;
                            var fm = new LiveUnitFacetModel { Id = fr.Str("id", string.Empty) };
                            var cAddrs = fr["cells"].AsArray();
                            if (cAddrs != null)
                            {
                                for (int k = 0; k < cAddrs.Count && k < 8; k++)
                                {
                                    if (cAddrs[k].Kind == JsonKind.String)
                                        fm.CellAddrs.Add(cAddrs[k].AsString());
                                }
                            }
                            sm.Facets.Add(fm);
                        }
                    }
                    parsed.Sections.Add(sm);
                }
            }

            var compArr = raw["components"].AsArray();
            if (compArr != null)
            {
                int cap = Math.Min(compArr.Count, 32);
                for (int i = 0; i < cap; i++)
                {
                    var c = compArr[i];
                    if (c == null || !c.IsObject || c["id"].Kind != JsonKind.String) continue;
                    parsed.Components.Add(new LiveUnitComponentModel
                    {
                        Id = c.Str("id", string.Empty),
                        Definition = c.Str("definition", string.Empty),
                        Install = c.Str("install", string.Empty),
                        Condition = c.Str("condition", string.Empty),
                        State = NormalizeGaugeState(c["state"])
                    });
                }
            }

            var capArr = raw["capabilities"].AsArray();
            if (capArr != null)
            {
                int cap = Math.Min(capArr.Count, 24);
                for (int i = 0; i < cap; i++)
                {
                    var c = capArr[i];
                    if (c == null || !c.IsObject || c["id"].Kind != JsonKind.String) continue;
                    parsed.Capabilities.Add(new LiveUnitCapabilityModel
                    {
                        Id = c.Str("id", string.Empty),
                        Source = c["source"].Kind == JsonKind.String ? c.Str("source", null) : null
                    });
                }
            }

            view = parsed;
            return true;
        }

        public string FormatRichSummary()
        {
            var sb = new StringBuilder(640);
            string stCol = LiveVitalsStore.GaugeStateColorHex(State);
            string rec = !string.IsNullOrEmpty(Record) ? $" <color=#FFCC55>[{MushProtocol.EscapeUnityRichText(Record)}]</color>" : string.Empty;
            sb.AppendFormat("<color=#FFCC66><b>UNIT CHASSIS: {0}</b></color> ({1})  <color={2}><b>[{3}]</b></color> rev {4}{5}\n",
                MushProtocol.EscapeUnityRichText(ObjectName),
                MushProtocol.EscapeUnityRichText(ObjectId),
                stCol,
                MushProtocol.EscapeUnityRichText(string.IsNullOrEmpty(State) ? "STATUS" : State.ToUpperInvariant()),
                Revision,
                rec);

            if (!string.IsNullOrEmpty(MobilityMode) || !string.IsNullOrEmpty(SwitchingTo))
            {
                string mobStr = !string.IsNullOrEmpty(SwitchingTo)
                    ? $"{MobilityMode ?? "—"} -> {SwitchingTo}"
                    : MobilityMode;
                sb.AppendFormat("Mobility: <b>{0}</b>\n", MushProtocol.EscapeUnityRichText(mobStr));
            }

            // Unit-scoped cells (addr starting with "unit/")
            bool printedPlant = false;
            for (int i = 0; i < Cells.Count; i++)
            {
                var c = Cells[i];
                if (!c.Addr.StartsWith("unit/", StringComparison.Ordinal)) continue;
                if (!printedPlant)
                {
                    sb.Append("<color=#88DDFF>Plant:</color> ");
                    printedPlant = true;
                }
                else
                {
                    sb.Append("  |  ");
                }
                sb.AppendFormat("<color={0}>{1} {2:0.##}/{3:0.##}</color>",
                    LiveVitalsStore.GaugeStateColorHex(c.State),
                    MushProtocol.EscapeUnityRichText(c.Label),
                    c.Current,
                    c.Max);
            }
            if (printedPlant) sb.AppendLine();

            if (Sections.Count > 0)
            {
                sb.AppendLine("\n<color=#66DDFF><b>SECTIONS & FACETS</b></color>");
                for (int i = 0; i < Sections.Count && i < 10; i++)
                {
                    var sec = Sections[i];
                    string sCol = LiveVitalsStore.GaugeStateColorHex(sec.State);
                    sb.AppendFormat("• <color={0}><b>{1}</b></color> [{2}]",
                        sCol,
                        MushProtocol.EscapeUnityRichText(sec.Id),
                        MushProtocol.EscapeUnityRichText(string.IsNullOrEmpty(sec.Condition) ? "—" : sec.Condition));

                    for (int f = 0; f < sec.Facets.Count && f < 4; f++)
                    {
                        var fac = sec.Facets[f];
                        if (fac.CellAddrs.Count > 0 && CellByAddr.TryGetValue(fac.CellAddrs[0], out var fc))
                        {
                            sb.AppendFormat("  {0}:{1:0}/{2:0}",
                                MushProtocol.EscapeUnityRichText(fac.Id),
                                fc.Current,
                                fc.Max);
                        }
                    }
                    sb.AppendLine();
                }
            }

            if (Components.Count > 0)
            {
                sb.AppendLine("\n<color=#88CCEE><b>COMPONENTS</b></color>");
                for (int i = 0; i < Components.Count && i < 8; i++)
                {
                    var c = Components[i];
                    sb.AppendFormat("• {0} ({1}): <color={2}>{3}</color>\n",
                        MushProtocol.EscapeUnityRichText(c.Id),
                        MushProtocol.EscapeUnityRichText(c.Install),
                        LiveVitalsStore.GaugeStateColorHex(c.State),
                        MushProtocol.EscapeUnityRichText(string.IsNullOrEmpty(c.Condition) ? c.State : c.Condition));
                }
            }

            if (Capabilities.Count > 0)
            {
                sb.Append("\n<color=#66FFAA><b>CAPABILITIES:</b></color> ");
                for (int i = 0; i < Capabilities.Count && i < 8; i++)
                {
                    if (i > 0) sb.Append(", ");
                    sb.Append(MushProtocol.EscapeUnityRichText(Capabilities[i].Id));
                }
                sb.AppendLine();
            }

            return sb.ToString().TrimEnd();
        }
    }
}
