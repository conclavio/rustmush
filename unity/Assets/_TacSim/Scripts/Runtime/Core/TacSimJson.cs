using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace TacSim.Core
{
    public enum JsonKind
    {
        Null,
        Bool,
        Int,
        Float,
        String,
        Array,
        Object
    }

    /// <summary>
    /// Lightweight, zero-dependency RFC-8259 JSON DOM for parsing polymorphic rustmush
    /// WebSocket envelopes, panel feed deltas, and TacSim content packs without external NuGet packages.
    /// </summary>
    public sealed class JsonNode
    {
        public static readonly JsonNode Null = new JsonNode(JsonKind.Null);

        public JsonKind Kind { get; private set; }
        private bool _boolVal;
        private long _intVal;
        private double _floatVal;
        private string _strVal;
        private List<JsonNode> _arrVal;
        private Dictionary<string, JsonNode> _objVal;

        private JsonNode(JsonKind kind)
        {
            Kind = kind;
        }

        public static JsonNode FromBool(bool v) => new JsonNode(JsonKind.Bool) { _boolVal = v };
        public static JsonNode FromLong(long v) => new JsonNode(JsonKind.Int) { _intVal = v, _floatVal = v };
        public static JsonNode FromDouble(double v) => new JsonNode(JsonKind.Float) { _floatVal = v, _intVal = (long)Math.Round(v) };
        public static JsonNode FromString(string v) => v == null ? Null : new JsonNode(JsonKind.String) { _strVal = v };

        public static JsonNode NewArray() => new JsonNode(JsonKind.Array) { _arrVal = new List<JsonNode>() };
        public static JsonNode NewObject() => new JsonNode(JsonKind.Object) { _objVal = new Dictionary<string, JsonNode>(StringComparer.Ordinal) };

        public bool IsNull => Kind == JsonKind.Null;
        public bool IsObject => Kind == JsonKind.Object;
        public bool IsArray => Kind == JsonKind.Array;

        public bool AsBool(bool fallback = false)
        {
            if (Kind == JsonKind.Bool) return _boolVal;
            if (Kind == JsonKind.Int) return _intVal != 0;
            return fallback;
        }

        public long AsLong(long fallback = 0)
        {
            if (Kind == JsonKind.Int) return _intVal;
            if (Kind == JsonKind.Float) return (long)Math.Round(_floatVal);
            if (Kind == JsonKind.String && long.TryParse(_strVal, NumberStyles.Integer, CultureInfo.InvariantCulture, out long p))
                return p;
            return fallback;
        }

        public int AsInt(int fallback = 0) => (int)AsLong(fallback);

        public double AsDouble(double fallback = 0.0)
        {
            if (Kind == JsonKind.Float) return _floatVal;
            if (Kind == JsonKind.Int) return _intVal;
            if (Kind == JsonKind.String && double.TryParse(_strVal, NumberStyles.Float, CultureInfo.InvariantCulture, out double p))
                return p;
            return fallback;
        }

        public float AsFloat(float fallback = 0f) => (float)AsDouble(fallback);

        public string AsString(string fallback = null)
        {
            if (Kind == JsonKind.String) return _strVal;
            if (Kind == JsonKind.Int) return _intVal.ToString(CultureInfo.InvariantCulture);
            if (Kind == JsonKind.Float) return _floatVal.ToString(CultureInfo.InvariantCulture);
            if (Kind == JsonKind.Bool) return _boolVal ? "true" : "false";
            return fallback;
        }

        public List<JsonNode> AsArray() => Kind == JsonKind.Array ? _arrVal : null;
        public Dictionary<string, JsonNode> AsObject() => Kind == JsonKind.Object ? _objVal : null;

        public int Count
        {
            get
            {
                if (Kind == JsonKind.Array) return _arrVal.Count;
                if (Kind == JsonKind.Object) return _objVal.Count;
                return 0;
            }
        }

        public JsonNode this[string key]
        {
            get
            {
                if (Kind == JsonKind.Object && key != null && _objVal.TryGetValue(key, out JsonNode v))
                    return v;
                return Null;
            }
            set
            {
                if (Kind != JsonKind.Object)
                    throw new InvalidOperationException("JsonNode is not an Object.");
                _objVal[key] = value ?? Null;
            }
        }

        public JsonNode this[int index]
        {
            get
            {
                if (Kind == JsonKind.Array && index >= 0 && index < _arrVal.Count)
                    return _arrVal[index];
                return Null;
            }
        }

        public void Add(JsonNode item)
        {
            if (Kind != JsonKind.Array)
                throw new InvalidOperationException("JsonNode is not an Array.");
            _arrVal.Add(item ?? Null);
        }

        public bool ContainsKey(string key) => Kind == JsonKind.Object && key != null && _objVal.ContainsKey(key);

        public string Str(string key, string fallback = null) => this[key].AsString(fallback);
        public long Long(string key, long fallback = 0) => this[key].AsLong(fallback);
        public int Int(string key, int fallback = 0) => this[key].AsInt(fallback);
        public double Double(string key, double fallback = 0.0) => this[key].AsDouble(fallback);
        public float Float(string key, float fallback = 0f) => this[key].AsFloat(fallback);
        public bool Bool(string key, bool fallback = false) => this[key].AsBool(fallback);

        public JsonNode Clone()
        {
            switch (Kind)
            {
                case JsonKind.Null: return Null;
                case JsonKind.Bool: return FromBool(_boolVal);
                case JsonKind.Int: return FromLong(_intVal);
                case JsonKind.Float: return FromDouble(_floatVal);
                case JsonKind.String: return FromString(_strVal);
                case JsonKind.Array:
                {
                    var copy = NewArray();
                    foreach (var el in _arrVal) copy.Add(el.Clone());
                    return copy;
                }
                case JsonKind.Object:
                {
                    var copy = NewObject();
                    foreach (var kv in _objVal) copy[kv.Key] = kv.Value.Clone();
                    return copy;
                }
                default: return Null;
            }
        }

        /// <summary>
        /// Merge top-level fields of <paramref name="delta"/> into this JSON object (for Panel/View delta frames).
        /// Per mush-proto delta semantics, a null field value deletes that top-level key.
        /// </summary>
        public void MergeObjectDelta(JsonNode delta)
        {
            if (Kind != JsonKind.Object || delta == null || delta.Kind != JsonKind.Object)
                return;
            foreach (var kv in delta._objVal)
            {
                if (kv.Value == null || kv.Value.IsNull)
                    _objVal.Remove(kv.Key);
                else
                    _objVal[kv.Key] = kv.Value.Clone();
            }
        }

        public string ToJson()
        {
            var sb = new StringBuilder(128);
            WriteJson(sb);
            return sb.ToString();
        }

        private void WriteJson(StringBuilder sb)
        {
            switch (Kind)
            {
                case JsonKind.Null:
                    sb.Append("null");
                    break;
                case JsonKind.Bool:
                    sb.Append(_boolVal ? "true" : "false");
                    break;
                case JsonKind.Int:
                    sb.Append(_intVal.ToString(CultureInfo.InvariantCulture));
                    break;
                case JsonKind.Float:
                    if (double.IsNaN(_floatVal) || double.IsInfinity(_floatVal))
                        sb.Append("0");
                    else
                        sb.Append(_floatVal.ToString("R", CultureInfo.InvariantCulture));
                    break;
                case JsonKind.String:
                    WriteEscapedString(sb, _strVal ?? string.Empty);
                    break;
                case JsonKind.Array:
                    sb.Append('[');
                    for (int i = 0; i < _arrVal.Count; i++)
                    {
                        if (i > 0) sb.Append(',');
                        _arrVal[i].WriteJson(sb);
                    }
                    sb.Append(']');
                    break;
                case JsonKind.Object:
                    sb.Append('{');
                    bool first = true;
                    foreach (var kv in _objVal)
                    {
                        if (!first) sb.Append(',');
                        first = false;
                        WriteEscapedString(sb, kv.Key);
                        sb.Append(':');
                        (kv.Value ?? Null).WriteJson(sb);
                    }
                    sb.Append('}');
                    break;
            }
        }

        private static void WriteEscapedString(StringBuilder sb, string s)
        {
            sb.Append('"');
            for (int i = 0; i < s.Length; i++)
            {
                char c = s[i];
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\b': sb.Append("\\b"); break;
                    case '\f': sb.Append("\\f"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (c < ' ')
                            sb.AppendFormat(CultureInfo.InvariantCulture, "\\u{0:x4}", (int)c);
                        else
                            sb.Append(c);
                        break;
                }
            }
            sb.Append('"');
        }

        public static JsonNode Parse(string json)
        {
            if (string.IsNullOrWhiteSpace(json))
                return Null;
            int idx = 0;
            var node = ParseValue(json, ref idx);
            return node ?? Null;
        }

        private static void SkipWs(string s, ref int i)
        {
            while (i < s.Length)
            {
                char c = s[i];
                if (c == ' ' || c == '\t' || c == '\r' || c == '\n') i++;
                else break;
            }
        }

        private static JsonNode ParseValue(string s, ref int i)
        {
            SkipWs(s, ref i);
            if (i >= s.Length) return Null;
            char c = s[i];
            switch (c)
            {
                case '{': return ParseObject(s, ref i);
                case '[': return ParseArray(s, ref i);
                case '"': return FromString(ParseString(s, ref i));
                case 't':
                    if (i + 3 < s.Length && s.Substring(i, 4) == "true") { i += 4; return FromBool(true); }
                    throw new FormatException($"Unexpected token at {i}");
                case 'f':
                    if (i + 4 < s.Length && s.Substring(i, 5) == "false") { i += 5; return FromBool(false); }
                    throw new FormatException($"Unexpected token at {i}");
                case 'n':
                    if (i + 3 < s.Length && s.Substring(i, 4) == "null") { i += 4; return Null; }
                    throw new FormatException($"Unexpected token at {i}");
                default:
                    if (c == '-' || (c >= '0' && c <= '9'))
                        return ParseNumber(s, ref i);
                    throw new FormatException($"Unexpected char '{c}' at {i}");
            }
        }

        private static JsonNode ParseObject(string s, ref int i)
        {
            var obj = NewObject();
            i++; // '{'
            SkipWs(s, ref i);
            if (i < s.Length && s[i] == '}') { i++; return obj; }
            while (i < s.Length)
            {
                SkipWs(s, ref i);
                if (i >= s.Length || s[i] != '"')
                    throw new FormatException($"Expected string key at {i}");
                string key = ParseString(s, ref i);
                SkipWs(s, ref i);
                if (i >= s.Length || s[i] != ':')
                    throw new FormatException($"Expected ':' at {i}");
                i++;
                obj[key] = ParseValue(s, ref i);
                SkipWs(s, ref i);
                if (i < s.Length && s[i] == ',') { i++; continue; }
                if (i < s.Length && s[i] == '}') { i++; break; }
                throw new FormatException($"Expected ',' or '}}' at {i}");
            }
            return obj;
        }

        private static JsonNode ParseArray(string s, ref int i)
        {
            var arr = NewArray();
            i++; // '['
            SkipWs(s, ref i);
            if (i < s.Length && s[i] == ']') { i++; return arr; }
            while (i < s.Length)
            {
                arr.Add(ParseValue(s, ref i));
                SkipWs(s, ref i);
                if (i < s.Length && s[i] == ',') { i++; continue; }
                if (i < s.Length && s[i] == ']') { i++; break; }
                throw new FormatException($"Expected ',' or ']' at {i}");
            }
            return arr;
        }

        private static string ParseString(string s, ref int i)
        {
            i++; // '"'
            var sb = new StringBuilder();
            while (i < s.Length)
            {
                char c = s[i++];
                if (c == '"') return sb.ToString();
                if (c == '\\' && i < s.Length)
                {
                    char esc = s[i++];
                    switch (esc)
                    {
                        case '"': sb.Append('"'); break;
                        case '\\': sb.Append('\\'); break;
                        case '/': sb.Append('/'); break;
                        case 'b': sb.Append('\b'); break;
                        case 'f': sb.Append('\f'); break;
                        case 'n': sb.Append('\n'); break;
                        case 'r': sb.Append('\r'); break;
                        case 't': sb.Append('\t'); break;
                        case 'u':
                            if (i + 4 <= s.Length && int.TryParse(s.Substring(i, 4), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out int code))
                            {
                                sb.Append((char)code);
                                i += 4;
                            }
                            break;
                        default:
                            sb.Append(esc);
                            break;
                    }
                }
                else
                {
                    sb.Append(c);
                }
            }
            throw new FormatException("Unterminated JSON string");
        }

        private static JsonNode ParseNumber(string s, ref int i)
        {
            int start = i;
            bool isFloat = false;
            if (s[i] == '-') i++;
            while (i < s.Length && s[i] >= '0' && s[i] <= '9') i++;
            if (i < s.Length && s[i] == '.')
            {
                isFloat = true;
                i++;
                while (i < s.Length && s[i] >= '0' && s[i] <= '9') i++;
            }
            if (i < s.Length && (s[i] == 'e' || s[i] == 'E'))
            {
                isFloat = true;
                i++;
                if (i < s.Length && (s[i] == '+' || s[i] == '-')) i++;
                while (i < s.Length && s[i] >= '0' && s[i] <= '9') i++;
            }
            string slice = s.Substring(start, i - start);
            if (!isFloat && long.TryParse(slice, NumberStyles.Integer, CultureInfo.InvariantCulture, out long lval))
                return FromLong(lval);
            if (double.TryParse(slice, NumberStyles.Float, CultureInfo.InvariantCulture, out double dval))
                return FromDouble(dval);
            return FromLong(0);
        }
    }
}
