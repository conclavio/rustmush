using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.Net;
using UnityEngine;

namespace TacSim.Arena
{
    public sealed class ArenaReplayRosterEntry
    {
        public string Callsign = "?";
        public string Team = "red"; // "red" | "blue"
        public bool IsBlue => string.Equals(Team, "blue", StringComparison.OrdinalIgnoreCase);
        public string PartyCode => IsBlue ? "B" : "R";
    }

    public struct ArenaReplayUnitPose
    {
        public float CellX;
        public float CellY;
        public float HeadingDeg;
    }

    public sealed class ArenaReplayKeyframe
    {
        public long TimeMs;
        public ArenaReplayUnitPose?[] Units;
    }

    public sealed class ArenaReplayShot
    {
        public long TimeMs;
        public int ShooterIndex;
        public int TargetIndex;
        public string Outcome; // "flying" | "hit" | "miss"
    }

    public readonly struct ArenaReplayWreck
    {
        public readonly long TimeMs;
        public readonly int RosterIndex;

        public ArenaReplayWreck(long timeMs, int rosterIndex)
        {
            TimeMs = timeMs;
            RosterIndex = rosterIndex;
        }
    }

    public sealed class ArenaReplayDocument
    {
        public ulong MatchId;
        public string ArenaName = string.Empty;
        public int CellCm = 100;
        public int Width;
        public int Height;
        public int SummaryMs;
        public long EndMs;
        public string Outcome = string.Empty;
        public string Reason = string.Empty;
        public string Mode = string.Empty;
        public bool Truncated;
        public readonly List<ArenaReplayRosterEntry> Roster = new List<ArenaReplayRosterEntry>();
        public readonly List<ArenaReplayKeyframe> Keyframes = new List<ArenaReplayKeyframe>();
        public readonly List<ArenaReplayShot> Shots = new List<ArenaReplayShot>();
        public readonly List<ArenaReplayWreck> Destroyed = new List<ArenaReplayWreck>();
    }

    public sealed class ArenaObserverUnit
    {
        public string Callsign = "?";
        public string Team = "red";
        public bool HasCell;
        public int CellX;
        public int CellY;
        public int HeadingDeg;
        public long ObjectId;
        public string Flags = string.Empty;

        public bool IsBlue => string.Equals(Team, "blue", StringComparison.OrdinalIgnoreCase);
        public bool IsWreck => !string.IsNullOrEmpty(Flags) && Flags.IndexOf('w') >= 0;
        public bool IsMachine => !string.IsNullOrEmpty(Flags) && Flags.IndexOf('m') >= 0;
        public bool IsHeld => !string.IsNullOrEmpty(Flags) && Flags.IndexOf('h') >= 0;
        public bool IsStarting => !string.IsNullOrEmpty(Flags) && Flags.IndexOf('s') >= 0;
    }

    public sealed class ArenaObserverPicture
    {
        public ulong MatchId;
        public long NowMs;
        public string Tempo = "?";
        public int? FloorMs;
        public int CellCm = 100;
        public int Width;
        public int Height;
        public int InFlight;
        public readonly List<ArenaObserverUnit> Units = new List<ArenaObserverUnit>();
    }

    public sealed class ArenaReplaySampledFrame
    {
        public float TimeMs;
        public ArenaReplayUnitPose?[] Units = Array.Empty<ArenaReplayUnitPose?>();
        public bool[] IsWrecked = Array.Empty<bool>();
        public bool BetweenKeyframes;
        public bool Held;
        public int BeforeKeyframeIndex;
        public int AfterKeyframeIndex;
        public int RedAliveCount;
        public int BlueAliveCount;
    }

    /// <summary>
    /// Wire decoder and interpolator for `mushd` `arena.replay` (finished match tapes)
    /// and `arena.observer` (live staff match snapshots), matching `web/src/lib/replay.ts`.
    /// </summary>
    public static class TacSimArenaReplayModel
    {
        private static readonly Dictionary<string, string> OutcomeWordsMap = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
        {
            { "red/elimination", "red won (blue eliminated)" },
            { "blue/elimination", "blue won (red eliminated)" },
            { "red/conceded", "red won (blue conceded)" },
            { "blue/conceded", "blue won (red conceded)" },
            { "red/hold", "red won (red held the key spot)" },
            { "blue/hold", "blue won (blue held the key spot)" },
            { "draw/spent", "draw (both sides out of ammunition)" },
            { "draw/elimination", "draw (both sides eliminated)" },
            { "timeout/unattended", "timed out (every pilot had left)" },
            { "timeout/timeout", "timed out (the clock ran out)" },
            { "abort/abort", "aborted" },
            { "abort/restart", "aborted (the server restarted)" },
            { "void/void", "void" },
            { "void/ledger_overflow", "void (the tape overflowed)" }
        };

        public static bool IsReplayUnavailable(JsonNode data)
        {
            return data != null && data.IsObject && data.ContainsKey("available") && !data.Bool("available", true);
        }

        public static bool IsObserverCleared(JsonNode data)
        {
            return data != null && data.IsObject && data.ContainsKey("watching") && !data.Bool("watching", true);
        }

        /// <summary>
        /// Detects the pushed `grid` trigger emitted by `SIM REPLAY &lt;match&gt;` on `arena.replay`.
        /// </summary>
        public static bool TryDecodeReplayGridTrigger(JsonNode data, out ulong matchId, out int keyframeIndex)
        {
            matchId = 0;
            keyframeIndex = 0;
            if (data == null || !data.IsObject) return false;
            var schema = data["schema"];
            if (!schema.IsObject || !string.Equals(schema.Str("shape", string.Empty), "grid", StringComparison.Ordinal))
                return false;
            long rawMatch = data.Long("match", -1);
            if (rawMatch < 0) return false;
            matchId = (ulong)rawMatch;
            var kfObj = data["keyframe"];
            if (kfObj.IsObject)
                keyframeIndex = Math.Max(0, kfObj.Int("index", 0));
            return true;
        }

        /// <summary>
        /// Decodes a full `arena.replay` document (`schema.shape == "replay"` or `record == "tacsim.replay"`).
        /// </summary>
        public static bool TryDecodeReplayDocument(JsonNode data, out ArenaReplayDocument doc)
        {
            doc = null;
            if (data == null || !data.IsObject || IsReplayUnavailable(data))
                return false;

            var schema = data["schema"];
            string shape = schema.IsObject ? schema.Str("shape", string.Empty) : null;
            bool validShape = string.Equals(shape, "replay", StringComparison.Ordinal) ||
                              (shape == null && string.Equals(data.Str("record", string.Empty), "tacsim.replay", StringComparison.Ordinal));
            if (!validShape) return false;

            long rawMatch = data.Long("match", -1);
            if (rawMatch < 0) return false;

            doc = new ArenaReplayDocument
            {
                MatchId = (ulong)rawMatch,
                ArenaName = CleanWord(data.Str("arena", string.Empty), string.Empty, 40),
                CellCm = Math.Max(10, data.Int("cell_cm", 100)),
                Width = Math.Max(0, data.Int("width", 0)),
                Height = Math.Max(0, data.Int("height", 0)),
                SummaryMs = Math.Max(0, data.Int("summary_ms", 0)),
                EndMs = Math.Max(0L, data.Long("end_ms", 0)),
                Outcome = CleanWord(data.Str("outcome", string.Empty), string.Empty, 16),
                Reason = CleanWord(data.Str("reason", string.Empty), string.Empty, 24),
                Mode = CleanWord(data.Str("mode", string.Empty), string.Empty, 16),
                Truncated = data.Bool("truncated", false)
            };

            var rosterArr = data["roster"].AsArray();
            if (rosterArr != null)
            {
                int rCap = Math.Min(rosterArr.Count, 64);
                for (int i = 0; i < rCap; i++)
                {
                    var r = rosterArr[i];
                    string cs = r.IsObject ? CleanWord(r.Str("callsign", "?"), "?", 8) : "?";
                    string team = r.IsObject && string.Equals(r.Str("team", "red"), "blue", StringComparison.OrdinalIgnoreCase) ? "blue" : "red";
                    doc.Roster.Add(new ArenaReplayRosterEntry { Callsign = cs, Team = team });
                }
            }

            int n = doc.Roster.Count;
            var kfArr = data["keyframes"].AsArray();
            if (kfArr != null)
            {
                int kCap = Math.Min(kfArr.Count, 4096);
                for (int i = 0; i < kCap; i++)
                {
                    var kf = kfArr[i].AsArray();
                    if (kf == null || kf.Count < 2) continue;
                    long tMs = kf[0].AsLong(-1);
                    if (tMs < 0) continue;

                    var slots = kf[1].AsArray();
                    var units = new ArenaReplayUnitPose?[n];
                    for (int u = 0; u < n; u++)
                    {
                        if (slots == null || u >= slots.Count) continue;
                        var slot = slots[u].AsArray();
                        if (slot == null || slot.Count < 3) continue;
                        int cx = slot[0].AsInt(-1);
                        int cy = slot[1].AsInt(-1);
                        int hdg = slot[2].AsInt(-1);
                        if (cx < 0 || cy < 0 || hdg < 0) continue;
                        units[u] = new ArenaReplayUnitPose
                        {
                            CellX = cx,
                            CellY = cy,
                            HeadingDeg = ((hdg % 360) + 360) % 360
                        };
                    }

                    doc.Keyframes.Add(new ArenaReplayKeyframe { TimeMs = tMs, Units = units });
                }
            }

            if (doc.EndMs == 0 && doc.Keyframes.Count > 0)
                doc.EndMs = doc.Keyframes[doc.Keyframes.Count - 1].TimeMs;

            var shotArr = data["shots"].AsArray();
            if (shotArr != null)
            {
                int sCap = Math.Min(shotArr.Count, 4096);
                for (int i = 0; i < sCap; i++)
                {
                    var s = shotArr[i].AsArray();
                    if (s == null || s.Count < 4) continue;
                    long tMs = s[0].AsLong(-1);
                    int shooter = s[1].AsInt(-1);
                    int target = s[2].AsInt(-1);
                    string outWord = s[3].AsString(string.Empty);
                    if (tMs < 0 || shooter < 0 || target < 0 || shooter >= n || target >= n) continue;
                    if (outWord != "flying" && outWord != "hit" && outWord != "miss") continue;
                    doc.Shots.Add(new ArenaReplayShot
                    {
                        TimeMs = tMs,
                        ShooterIndex = shooter,
                        TargetIndex = target,
                        Outcome = outWord
                    });
                }
            }

            var destArr = data["destroyed"].AsArray();
            if (destArr != null)
            {
                int dCap = Math.Min(destArr.Count, 128);
                for (int i = 0; i < dCap; i++)
                {
                    var d = destArr[i].AsArray();
                    if (d == null || d.Count < 2) continue;
                    long tMs = d[0].AsLong(-1);
                    int idx = d[1].AsInt(-1);
                    if (tMs < 0 || idx < 0 || idx >= n) continue;
                    doc.Destroyed.Add(new ArenaReplayWreck(tMs, idx));
                }
            }

            return true;
        }

        /// <summary>
        /// Decodes a live `arena.observer` snapshot pushed to staff watching a running match.
        /// </summary>
        public static bool TryDecodeObserverPicture(JsonNode data, out ArenaObserverPicture pic)
        {
            pic = null;
            if (data == null || !data.IsObject || IsObserverCleared(data))
                return false;
            long rawMatch = data.Long("match", -1);
            if (rawMatch < 0) return false;

            pic = new ArenaObserverPicture
            {
                MatchId = (ulong)rawMatch,
                NowMs = Math.Max(0L, data.Long("now_ms", 0)),
                Tempo = CleanWord(data.Str("tempo", "?"), "?", 8),
                FloorMs = data.ContainsKey("floor_ms") && !data["floor_ms"].IsNull ? (int?)Math.Max(0, data.Int("floor_ms", 0)) : null,
                CellCm = Math.Max(10, data.Int("cell_cm", 100)),
                Width = Math.Max(0, data.Int("width", 0)),
                Height = Math.Max(0, data.Int("height", 0)),
                InFlight = Math.Max(0, data.Int("in_flight", 0))
            };

            var uArr = data["units"].AsArray();
            if (uArr != null)
            {
                int cap = Math.Min(uArr.Count, 64);
                for (int i = 0; i < cap; i++)
                {
                    var row = uArr[i].AsArray();
                    if (row == null || row.Count < 6) continue;
                    bool hasX = !row[2].IsNull && row[2].AsInt(-1) >= 0;
                    bool hasY = !row[3].IsNull && row[3].AsInt(-1) >= 0;
                    pic.Units.Add(new ArenaObserverUnit
                    {
                        Callsign = CleanWord(row[0].AsString("?"), "?", 8),
                        Team = string.Equals(row[1].AsString("R"), "B", StringComparison.OrdinalIgnoreCase) ? "blue" : "red",
                        HasCell = hasX && hasY,
                        CellX = hasX ? row[2].AsInt(0) : 0,
                        CellY = hasY ? row[3].AsInt(0) : 0,
                        HeadingDeg = ((row[4].AsInt(0) % 360) + 360) % 360,
                        ObjectId = Math.Max(0L, row[5].AsLong(0)),
                        Flags = row.Count > 6 ? CleanWord(row[6].AsString(string.Empty), string.Empty, 8) : string.Empty
                    });
                }
            }

            return true;
        }

        public static ArenaReplayDocument FromObserverPicture(ArenaObserverPicture pic)
        {
            if (pic == null) return null;
            var doc = new ArenaReplayDocument
            {
                MatchId = pic.MatchId,
                CellCm = pic.CellCm,
                Width = pic.Width,
                Height = pic.Height,
                EndMs = pic.NowMs
            };
            var units = new ArenaReplayUnitPose?[pic.Units.Count];
            for (int i = 0; i < pic.Units.Count; i++)
            {
                var u = pic.Units[i];
                doc.Roster.Add(new ArenaReplayRosterEntry { Callsign = u.Callsign, Team = u.Team });
                if (u.HasCell)
                {
                    units[i] = new ArenaReplayUnitPose
                    {
                        CellX = u.CellX,
                        CellY = u.CellY,
                        HeadingDeg = u.HeadingDeg
                    };
                }
                if (u.IsWreck)
                    doc.Destroyed.Add(new ArenaReplayWreck(pic.NowMs, i));
            }
            doc.Keyframes.Add(new ArenaReplayKeyframe { TimeMs = pic.NowMs, Units = units });
            return doc;
        }

        public static float TurnHeadingDeg(float fromDeg, float toDeg, float f)
        {
            float delta = (((toDeg - fromDeg) % 360f) + 540f) % 360f - 180f;
            float res = (fromDeg + delta * Mathf.Clamp01(f)) % 360f;
            return res < 0f ? res + 360f : res;
        }

        public static bool IsWreckedAt(ArenaReplayDocument doc, int rosterIndex, float tMs)
        {
            if (doc == null) return false;
            for (int i = 0; i < doc.Destroyed.Count; i++)
            {
                if (doc.Destroyed[i].RosterIndex == rosterIndex && doc.Destroyed[i].TimeMs <= tMs)
                    return true;
            }
            return false;
        }

        /// <summary>
        /// Samples smoothly interpolated 3D poses at arbitrary arena time `tMs` using linear cell motion
        /// and shortest-arc heading turns, while flagging `BetweenKeyframes` / `Held` for honest UI labeling.
        /// </summary>
        public static ArenaReplaySampledFrame SampleInterpolatedFrame(ArenaReplayDocument doc, float tMs)
        {
            var res = new ArenaReplaySampledFrame { TimeMs = Math.Max(0f, tMs) };
            if (doc == null) return res;

            int n = doc.Roster.Count;
            res.Units = new ArenaReplayUnitPose?[n];
            res.IsWrecked = new bool[n];

            for (int i = 0; i < n; i++)
            {
                bool w = IsWreckedAt(doc, i, res.TimeMs);
                res.IsWrecked[i] = w;
                if (!w)
                {
                    if (doc.Roster[i].IsBlue) res.BlueAliveCount++;
                    else res.RedAliveCount++;
                }
            }

            int kCount = doc.Keyframes.Count;
            if (kCount == 0) return res;

            var first = doc.Keyframes[0];
            var last = doc.Keyframes[kCount - 1];
            if (res.TimeMs <= first.TimeMs)
            {
                Array.Copy(first.Units, res.Units, Math.Min(n, first.Units.Length));
                res.Held = res.TimeMs < first.TimeMs;
                res.BetweenKeyframes = res.Held;
                res.BeforeKeyframeIndex = 0;
                res.AfterKeyframeIndex = 0;
                return res;
            }
            if (res.TimeMs >= last.TimeMs)
            {
                Array.Copy(last.Units, res.Units, Math.Min(n, last.Units.Length));
                res.Held = res.TimeMs > last.TimeMs;
                res.BetweenKeyframes = res.Held;
                res.BeforeKeyframeIndex = kCount - 1;
                res.AfterKeyframeIndex = kCount - 1;
                return res;
            }

            int after = kCount - 1;
            for (int i = 0; i < kCount; i++)
            {
                if (doc.Keyframes[i].TimeMs >= res.TimeMs)
                {
                    after = i;
                    break;
                }
            }

            var kb = doc.Keyframes[after];
            if (Mathf.Abs(kb.TimeMs - res.TimeMs) < 0.5f)
            {
                Array.Copy(kb.Units, res.Units, Math.Min(n, kb.Units.Length));
                res.BeforeKeyframeIndex = after;
                res.AfterKeyframeIndex = after;
                return res;
            }

            int before = Math.Max(0, after - 1);
            var ka = doc.Keyframes[before];
            float span = Math.Max(1f, kb.TimeMs - ka.TimeMs);
            float f = Mathf.Clamp01((res.TimeMs - ka.TimeMs) / span);

            for (int i = 0; i < n; i++)
            {
                ArenaReplayUnitPose? aPose = i < ka.Units.Length ? ka.Units[i] : null;
                ArenaReplayUnitPose? bPose = i < kb.Units.Length ? kb.Units[i] : null;
                if (!aPose.HasValue)
                {
                    res.Units[i] = bPose;
                }
                else if (!bPose.HasValue)
                {
                    res.Units[i] = aPose;
                }
                else
                {
                    res.Units[i] = new ArenaReplayUnitPose
                    {
                        CellX = Mathf.Lerp(aPose.Value.CellX, bPose.Value.CellX, f),
                        CellY = Mathf.Lerp(aPose.Value.CellY, bPose.Value.CellY, f),
                        HeadingDeg = TurnHeadingDeg(aPose.Value.HeadingDeg, bPose.Value.HeadingDeg, f)
                    };
                }
            }

            res.BetweenKeyframes = true;
            res.Held = false;
            res.BeforeKeyframeIndex = before;
            res.AfterKeyframeIndex = after;
            return res;
        }

        public static void CollectShotsInWindow(ArenaReplayDocument doc, float fromExclusiveMs, float toInclusiveMs, List<ArenaReplayShot> results)
        {
            results.Clear();
            if (doc == null || toInclusiveMs <= fromExclusiveMs) return;
            for (int i = 0; i < doc.Shots.Count; i++)
            {
                var s = doc.Shots[i];
                if (s.TimeMs > fromExclusiveMs && s.TimeMs <= toInclusiveMs)
                    results.Add(s);
            }
        }

        public static string FormatClockText(long ms)
        {
            long s = Math.Max(0L, ms) / 1000L;
            return $"{s / 60L}:{s % 60L:00}";
        }

        public static string FormatOutcomeWords(string outcome, string reason)
        {
            string key = $"{outcome}/{reason}";
            if (OutcomeWordsMap.TryGetValue(key, out string words))
                return words;
            if (!string.IsNullOrEmpty(reason))
                return $"{outcome} ({reason})";
            return outcome ?? string.Empty;
        }

        public static string FormatFrameSourceLabel(ArenaReplayDocument doc, ArenaReplaySampledFrame frame)
        {
            if (doc == null || frame == null || doc.Keyframes.Count == 0)
                return "Waiting for replay document...";

            long kfMs = frame.BeforeKeyframeIndex >= 0 && frame.BeforeKeyframeIndex < doc.Keyframes.Count
                ? doc.Keyframes[frame.BeforeKeyframeIndex].TimeMs
                : 0;
            string kfClock = FormatClockText(kfMs);
            if (frame.Held)
            {
                return frame.TimeMs < doc.Keyframes[0].TimeMs
                    ? $"Before keyframe 1 ({kfClock}): first recorded positions, held."
                    : $"After keyframe {frame.BeforeKeyframeIndex + 1} ({kfClock}): last recorded positions, held.";
            }
            if (frame.BetweenKeyframes)
                return $"Between keyframes {frame.BeforeKeyframeIndex + 1} and {frame.AfterKeyframeIndex + 1}: interpolated smoothly (drawn, not recorded).";
            return $"Keyframe {frame.BeforeKeyframeIndex + 1} of {doc.Keyframes.Count} ({kfClock}): authoritative arena positions.";
        }

        private static string CleanWord(string raw, string fallback, int maxLen)
        {
            if (string.IsNullOrWhiteSpace(raw)) return fallback;
            string stripped = MushProtocol.StripControlChars(raw).Trim();
            if (string.IsNullOrEmpty(stripped)) return fallback;
            return stripped.Length > maxLen ? stripped.Substring(0, maxLen) : stripped;
        }
    }
}
