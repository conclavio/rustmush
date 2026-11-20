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
        public const int MaxDepth = 64;
        public static readonly JsonNode Null = new JsonNode(JsonKind.Null);

        public JsonKind Kind { get; private set; }
        private bool _boolVal;
        private long _intVal;
        private ulong _ulongVal;
        private bool _hasUlongVal;
        private double _floatVal;
        private string _strVal;
        private List<JsonNode> _arrVal;
        private Dictionary<string, JsonNode> _objVal;

        private JsonNode(JsonKind kind)
        {
            Kind = kind;
        }

        private static bool IsFiniteDouble(double v) => !double.IsNaN(v) && !double.IsInfinity(v);

        private static bool TryDoubleToLong(double v, out long result)
        {
            if (!IsFiniteDouble(v))
            {
                result = 0;
                return false;
            }
            double rounded = Math.Round(v);
            if (rounded < -9223372036854775808.0 || rounded >= 9223372036854775808.0)
            {
                result = 0;
                return false;
            }
            result = (long)rounded;
            return true;
        }

        public static JsonNode FromBool(bool v) => new JsonNode(JsonKind.Bool) { _boolVal = v };
        public static JsonNode FromLong(long v) => new JsonNode(JsonKind.Int) { _intVal = v, _ulongVal = v >= 0 ? (ulong)v : 0UL, _floatVal = v };
        public static JsonNode FromULong(ulong v)
        {
            if (v <= (ulong)long.MaxValue)
                return FromLong((long)v);
            return new JsonNode(JsonKind.Int) { _intVal = long.MaxValue, _ulongVal = v, _hasUlongVal = true, _floatVal = v };
        }
        public static JsonNode FromDouble(double v)
        {
            if (!IsFiniteDouble(v))
                return Null;
            long clampedInt = TryDoubleToLong(v, out long lv)
                ? lv
                : (v < 0.0 ? long.MinValue : long.MaxValue);
            return new JsonNode(JsonKind.Float) { _floatVal = v, _intVal = clampedInt };
        }
        public static JsonNode FromString(string v) => v == null ? Null : new JsonNode(JsonKind.String) { _strVal = v };

        public static JsonNode NewArray() => new JsonNode(JsonKind.Array) { _arrVal = new List<JsonNode>() };
        public static JsonNode NewObject() => new JsonNode(JsonKind.Object) { _objVal = new Dictionary<string, JsonNode>(StringComparer.Ordinal) };

        public bool IsNull => Kind == JsonKind.Null;
        public bool IsObject => Kind == JsonKind.Object;
        public bool IsArray => Kind == JsonKind.Array;

        public bool AsBool(bool fallback = false)
        {
            if (Kind == JsonKind.Bool) return _boolVal;
            if (Kind == JsonKind.Int) return _hasUlongVal ? _ulongVal != 0 : _intVal != 0;
            return fallback;
        }

        public long AsLong(long fallback = 0)
        {
            if (Kind == JsonKind.Int) return _hasUlongVal ? long.MaxValue : _intVal;
            if (Kind == JsonKind.Float)
                return TryDoubleToLong(_floatVal, out long lv) ? lv : fallback;
            if (Kind == JsonKind.String && long.TryParse(_strVal, NumberStyles.Integer, CultureInfo.InvariantCulture, out long p))
                return p;
            return fallback;
        }

        public ulong AsULong(ulong fallback = 0)
        {
            if (Kind == JsonKind.Int)
            {
                if (_hasUlongVal) return _ulongVal;
                return _intVal >= 0 ? (ulong)_intVal : fallback;
            }
            if (Kind == JsonKind.Float)
            {
                if (!IsFiniteDouble(_floatVal) || _floatVal < 0.0)
                    return fallback;
                double rounded = Math.Round(_floatVal);
                if (rounded >= 18446744073709551616.0) return ulong.MaxValue;
                return (ulong)rounded;
            }
            if (Kind == JsonKind.String && ulong.TryParse(_strVal, NumberStyles.Integer, CultureInfo.InvariantCulture, out ulong u))
                return u;
            return fallback;
        }

        public int AsInt(int fallback = 0)
        {
            long v = AsLong(fallback);
            if (v < int.MinValue || v > int.MaxValue)
                return fallback;
            return (int)v;
        }

        public double AsDouble(double fallback = 0.0)
        {
            if (Kind == JsonKind.Float)
                return IsFiniteDouble(_floatVal) ? _floatVal : fallback;
            if (Kind == JsonKind.Int)
                return _hasUlongVal ? (double)_ulongVal : _intVal;
            if (Kind == JsonKind.String &&
                double.TryParse(_strVal, NumberStyles.Float, CultureInfo.InvariantCulture, out double p) &&
                IsFiniteDouble(p))
                return p;
            return fallback;
        }

        public float AsFloat(float fallback = 0f) => (float)AsDouble(fallback);

        public string AsString(string fallback = null)
        {
            if (Kind == JsonKind.String) return _strVal;
            if (Kind == JsonKind.Int) return _hasUlongVal ? _ulongVal.ToString(CultureInfo.InvariantCulture) : _intVal.ToString(CultureInfo.InvariantCulture);
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
        public ulong ULong(string key, ulong fallback = 0) => this[key].AsULong(fallback);
        public int Int(string key, int fallback = 0) => this[key].AsInt(fallback);
        public double Double(string key, double fallback = 0.0) => this[key].AsDouble(fallback);
        public float Float(string key, float fallback = 0f) => this[key].AsFloat(fallback);
        public bool Bool(string key, bool fallback = false) => this[key].AsBool(fallback);

        public JsonNode Clone() => CloneInternal(0);

        private JsonNode CloneInternal(int depth)
        {
            if (depth > MaxDepth)
                throw new InvalidOperationException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
            switch (Kind)
            {
                case JsonKind.Null: return Null;
                case JsonKind.Bool: return FromBool(_boolVal);
                case JsonKind.Int: return _hasUlongVal ? FromULong(_ulongVal) : FromLong(_intVal);
                case JsonKind.Float: return FromDouble(_floatVal);
                case JsonKind.String: return FromString(_strVal);
                case JsonKind.Array:
                {
                    if (depth >= MaxDepth)
                        throw new InvalidOperationException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
                    var copy = NewArray();
                    foreach (var el in _arrVal) copy.Add((el ?? Null).CloneInternal(depth + 1));
                    return copy;
                }
                case JsonKind.Object:
                {
                    if (depth >= MaxDepth)
                        throw new InvalidOperationException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
                    var copy = NewObject();
                    foreach (var kv in _objVal) copy[kv.Key] = (kv.Value ?? Null).CloneInternal(depth + 1);
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
            WriteJson(sb, 0);
            return sb.ToString();
        }

        private void WriteJson(StringBuilder sb, int depth)
        {
            if (depth > MaxDepth)
                throw new InvalidOperationException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
            switch (Kind)
            {
                case JsonKind.Null:
                    sb.Append("null");
                    break;
                case JsonKind.Bool:
                    sb.Append(_boolVal ? "true" : "false");
                    break;
                case JsonKind.Int:
                    sb.Append(_hasUlongVal
                        ? _ulongVal.ToString(CultureInfo.InvariantCulture)
                        : _intVal.ToString(CultureInfo.InvariantCulture));
                    break;
                case JsonKind.Float:
                    if (!IsFiniteDouble(_floatVal))
                        sb.Append("0");
                    else
                        sb.Append(_floatVal.ToString("R", CultureInfo.InvariantCulture));
                    break;
                case JsonKind.String:
                    WriteEscapedString(sb, _strVal ?? string.Empty);
                    break;
                case JsonKind.Array:
                    if (depth >= MaxDepth)
                        throw new InvalidOperationException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
                    sb.Append('[');
                    for (int i = 0; i < _arrVal.Count; i++)
                    {
                        if (i > 0) sb.Append(',');
                        (_arrVal[i] ?? Null).WriteJson(sb, depth + 1);
                    }
                    sb.Append(']');
                    break;
                case JsonKind.Object:
                    if (depth >= MaxDepth)
                        throw new InvalidOperationException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
                    sb.Append('{');
                    bool first = true;
                    foreach (var kv in _objVal)
                    {
                        if (!first) sb.Append(',');
                        first = false;
                        WriteEscapedString(sb, kv.Key);
                        sb.Append(':');
                        (kv.Value ?? Null).WriteJson(sb, depth + 1);
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
            var node = ParseValue(json, ref idx, 0);
            SkipWs(json, ref idx);
            if (idx < json.Length)
                throw new FormatException($"Unexpected trailing character '{json[idx]}' at {idx}");
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

        private static JsonNode ParseValue(string s, ref int i, int depth)
        {
            if (depth > MaxDepth)
                throw new FormatException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
            SkipWs(s, ref i);
            if (i >= s.Length)
                throw new FormatException($"Unexpected end of JSON input at {i}");
            char c = s[i];
            switch (c)
            {
                case '{': return ParseObject(s, ref i, depth + 1);
                case '[': return ParseArray(s, ref i, depth + 1);
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

        private static JsonNode ParseObject(string s, ref int i, int depth)
        {
            if (depth > MaxDepth)
                throw new FormatException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
            var obj = NewObject();
            i++; // '{'
            SkipWs(s, ref i);
            if (i >= s.Length)
                throw new FormatException($"Unterminated JSON object at {i}");
            if (s[i] == '}') { i++; return obj; }
            while (true)
            {
                SkipWs(s, ref i);
                if (i >= s.Length || s[i] != '"')
                    throw new FormatException($"Expected string key at {i}");
                string key = ParseString(s, ref i);
                SkipWs(s, ref i);
                if (i >= s.Length || s[i] != ':')
                    throw new FormatException($"Expected ':' at {i}");
                i++;
                obj[key] = ParseValue(s, ref i, depth);
                SkipWs(s, ref i);
                if (i >= s.Length)
                    throw new FormatException($"Unterminated JSON object at {i}");
                if (s[i] == ',') { i++; continue; }
                if (s[i] == '}') { i++; return obj; }
                throw new FormatException($"Expected ',' or '}}' at {i}");
            }
        }

        private static JsonNode ParseArray(string s, ref int i, int depth)
        {
            if (depth > MaxDepth)
                throw new FormatException($"JSON nesting depth exceeded maximum ({MaxDepth}).");
            var arr = NewArray();
            i++; // '['
            SkipWs(s, ref i);
            if (i >= s.Length)
                throw new FormatException($"Unterminated JSON array at {i}");
            if (s[i] == ']') { i++; return arr; }
            while (true)
            {
                arr.Add(ParseValue(s, ref i, depth));
                SkipWs(s, ref i);
                if (i >= s.Length)
                    throw new FormatException($"Unterminated JSON array at {i}");
                if (s[i] == ',') { i++; continue; }
                if (s[i] == ']') { i++; return arr; }
                throw new FormatException($"Expected ',' or ']' at {i}");
            }
        }

        private static bool TryParseStrictHex4(string s, int offset, out int value)
        {
            value = 0;
            if (offset + 4 > s.Length) return false;
            int acc = 0;
            for (int k = 0; k < 4; k++)
            {
                char ch = s[offset + k];
                int d;
                if (ch >= '0' && ch <= '9') d = ch - '0';
                else if (ch >= 'a' && ch <= 'f') d = 10 + (ch - 'a');
                else if (ch >= 'A' && ch <= 'F') d = 10 + (ch - 'A');
                else return false;
                acc = (acc << 4) | d;
            }
            value = acc;
            return true;
        }

        private static string ParseString(string s, ref int i)
        {
            i++; // '"'
            var sb = new StringBuilder();
            while (i < s.Length)
            {
                char c = s[i++];
                if (c == '"') return sb.ToString();
                if (c < 0x20)
                    throw new FormatException($"Unescaped control character 0x{(int)c:x2} in JSON string at {i - 1}");
                if (c == '\\')
                {
                    if (i >= s.Length)
                        throw new FormatException($"Unterminated escape sequence in JSON string at {i}");
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
                            if (!TryParseStrictHex4(s, i, out int code))
                                throw new FormatException($"Invalid \\uXXXX escape in JSON string at {i}");
                            sb.Append((char)code);
                            i += 4;
                            break;
                        default:
                            throw new FormatException($"Invalid escape sequence '\\{esc}' in JSON string at {i - 1}");
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
            bool negative = false;
            if (s[i] == '-')
            {
                negative = true;
                i++;
            }
            if (i >= s.Length || s[i] < '0' || s[i] > '9')
                throw new FormatException($"Invalid JSON number at {start}");
            if (s[i] == '0')
            {
                i++;
                if (i < s.Length && s[i] >= '0' && s[i] <= '9')
                    throw new FormatException($"Leading zero in JSON number at {start}");
            }
            else
            {
                while (i < s.Length && s[i] >= '0' && s[i] <= '9') i++;
            }
            if (i < s.Length && s[i] == '.')
            {
                isFloat = true;
                i++;
                int fracStart = i;
                while (i < s.Length && s[i] >= '0' && s[i] <= '9') i++;
                if (i == fracStart)
                    throw new FormatException($"Missing fractional digits in JSON number at {start}");
            }
            if (i < s.Length && (s[i] == 'e' || s[i] == 'E'))
            {
                isFloat = true;
                i++;
                if (i < s.Length && (s[i] == '+' || s[i] == '-')) i++;
                int expStart = i;
                while (i < s.Length && s[i] >= '0' && s[i] <= '9') i++;
                if (i == expStart)
                    throw new FormatException($"Missing exponent digits in JSON number at {start}");
            }
            string slice = s.Substring(start, i - start);
            if (!isFloat)
            {
                if (long.TryParse(slice, NumberStyles.Integer, CultureInfo.InvariantCulture, out long lval))
                    return FromLong(lval);
                if (!negative && ulong.TryParse(slice, NumberStyles.Integer, CultureInfo.InvariantCulture, out ulong uval))
                    return FromULong(uval);
            }
            if (double.TryParse(slice, NumberStyles.Float, CultureInfo.InvariantCulture, out double dval) &&
                IsFiniteDouble(dval))
            {
                return FromDouble(dval);
            }
            throw new FormatException($"Out-of-range or invalid JSON number '{slice}' at {start}");
        }
    }
}
