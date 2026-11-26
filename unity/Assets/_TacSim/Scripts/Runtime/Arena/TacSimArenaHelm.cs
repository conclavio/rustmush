using System;
using System.Collections.Generic;
using UnityEngine;

namespace TacSim.Arena
{
    public struct ArenaHelmFrame
    {
        public long? TMs;
        public int HeadingMdeg;
        public string Tempo;
        public int? TurnRateMdegS;
        public bool CanTurn;
    }

    public struct ArenaThrottleFrame
    {
        public int SpeedKph;
        public int? MaxKph;
        public int? ReverseKph;
        public bool Stuck;
    }

    /// <summary>
    /// Exact C# port of Doc 21 (`G.4b`) continuous predictive tank/mech helm (`web/src/lib/helm.ts`
    /// &amp; `mush-net-ssh/src/tui/helm.rs`). Uses integer millidegrees and whole milliseconds so taps
    /// turn 1 degree cleanly, held turns lead by one `OrderGapMs`, and the 3D hull rotates smoothly
    /// at the authoritative `turn_rate_mdeg_s` without snapping on discrete server pulses.
    /// </summary>
    public sealed class ArenaPredictiveHelm
    {
        public const int FullMdeg = 360_000;
        public const int DefaultTurnRateMdegS = 30_000;
        public const int MinTurnRateMdegS = 1_000;
        public const int MaxTurnRateMdegS = 360_000;
        public const int OrderGapMs = 250;
        public const int MinLeadMdeg = 1_000;
        public const int MaxLeadMdeg = 25_000;
        public const int MinLeashMdeg = 20_000;
        public const int MaxLeashMdeg = 150_000;
        public const int ReachedMdeg = 1_000;
        public const int TapMdeg = 1_000;
        public const int TapMs = 200;
        public const int RunCapMs = 600_000;

        private struct SeenFrame
        {
            public long? TMs;
            public int HeadingMdeg;
            public bool CanTurn;
            public long AtMs;
        }

        private struct HeadingAt
        {
            public int Mdeg;
            public long AtMs;
        }

        private int _turning; // -1, 0, +1
        private HeadingAt? _ordered;
        private int? _priorOrderMdeg;
        private SeenFrame? _seen;
        private HeadingAt? _shown;
        private HeadingAt? _turnFrom;
        private bool _live;
        private int _rateMdegS = DefaultTurnRateMdegS;
        private bool _rateRead;
        private int _tempoMs = 1_000;
        private readonly List<string> _orders = new List<string>(8);

        public int Turning => _turning;
        public int? OrderedMdeg => _ordered.HasValue ? _ordered.Value.Mdeg : (int?)null;
        public int TurnRateMdegS => _rateMdegS;
        public bool IsLive => _live;
        public long FrameAtMs => _seen.HasValue ? _seen.Value.AtMs : 0L;
        public int ReckonCapMs => _tempoMs + (_tempoMs / 2);
        public bool IsBusy => _turning != 0 || _ordered.HasValue;

        public void Reset()
        {
            _turning = 0;
            _ordered = null;
            _priorOrderMdeg = null;
            _seen = null;
            _shown = null;
            _turnFrom = null;
            _live = false;
            _rateMdegS = DefaultTurnRateMdegS;
            _rateRead = false;
            _tempoMs = 1_000;
            _orders.Clear();
        }

        public static int Wrap360Mdeg(long mdeg)
        {
            long r = mdeg % FullMdeg;
            return (int)(r < 0 ? r + FullMdeg : r);
        }

        public static int ArcMdeg(int fromMdeg, int toMdeg)
        {
            int d = Wrap360Mdeg((long)toMdeg - fromMdeg);
            return d > FullMdeg / 2 ? d - FullMdeg : d;
        }

        private static int ClampInt(long v, int min, int max)
        {
            if (v < min) return min;
            if (v > max) return max;
            return (int)v;
        }

        private static int MsBetween(long fromMs, long toMs)
        {
            long d = toMs - fromMs;
            if (d <= 0) return 0;
            if (d > RunCapMs) return RunCapMs;
            return (int)d;
        }

        private static int TempoToMs(string tempo)
        {
            if (string.Equals(tempo, "quiet", StringComparison.OrdinalIgnoreCase)) return 5_000;
            if (string.Equals(tempo, "alert", StringComparison.OrdinalIgnoreCase)) return 1_000;
            if (string.Equals(tempo, "engaged", StringComparison.OrdinalIgnoreCase)) return 500;
            return 0;
        }

        public void Observe(ArenaHelmFrame f, long nowMs)
        {
            var prev = _seen;
            int heading = Wrap360Mdeg(f.HeadingMdeg);
            if (prev.HasValue)
            {
                bool same = f.TMs.HasValue
                    ? (prev.Value.TMs.HasValue && f.TMs.Value == prev.Value.TMs.Value)
                    : (!prev.Value.TMs.HasValue && heading == prev.Value.HeadingMdeg && f.CanTurn == prev.Value.CanTurn);
                if (same) return;
            }

            int? stands = PredictedMdeg(nowMs);
            int tempo = TempoToMs(f.Tempo);
            if (tempo > 0) _tempoMs = tempo;
            if (f.TurnRateMdegS.HasValue && f.TurnRateMdegS.Value > 0)
            {
                _rateMdegS = ClampInt(f.TurnRateMdegS.Value, MinTurnRateMdegS, MaxTurnRateMdegS);
                _rateRead = true;
            }

            int? carried = null;
            if (prev.HasValue)
            {
                if (f.TMs.HasValue && prev.Value.TMs.HasValue)
                {
                    long dt = f.TMs.Value - prev.Value.TMs.Value;
                    _live = dt > 0 && dt <= 10_000;
                    if (dt > 0)
                    {
                        carried = stands;
                        Judge(prev.Value, heading, (int)Math.Min(dt, RunCapMs), nowMs);
                    }
                }
                else
                {
                    _live = false;
                    if (!f.TMs.HasValue && !prev.Value.TMs.HasValue)
                        carried = stands;
                }
            }

            _shown = new HeadingAt { Mdeg = carried ?? heading, AtMs = nowMs };
            _seen = new SeenFrame { TMs = f.TMs, HeadingMdeg = heading, CanTurn = f.CanTurn, AtMs = nowMs };
            if (_turning == 0 && _ordered.HasValue && Math.Abs(ArcMdeg(heading, _ordered.Value.Mdeg)) <= ReachedMdeg)
                _ordered = null;
        }

        private void Judge(SeenFrame prev, int heading, int dtMs, long nowMs)
        {
            if (_turning == 0)
            {
                if (_ordered.HasValue)
                {
                    var ord = _ordered.Value;
                    if (nowMs - ord.AtMs > GraceMs() &&
                        Math.Abs(ArcMdeg(heading, ord.Mdeg)) >= Math.Abs(ArcMdeg(prev.HeadingMdeg, ord.Mdeg)))
                    {
                        _ordered = null;
                    }
                }
                return;
            }

            int dir = _turning;
            bool whole = _turnFrom.HasValue && _turnFrom.Value.AtMs <= prev.AtMs;
            bool isShort = _priorOrderMdeg.HasValue && dir * ArcMdeg(heading, _priorOrderMdeg.Value) > ReachedMdeg;
            if (!(whole && isShort)) return;

            int turned = dir * ArcMdeg(prev.HeadingMdeg, heading);
            if (turned <= 0)
            {
                _turning = 0;
                _turnFrom = null;
                _ordered = null;
                _priorOrderMdeg = null;
            }
            else if (!_rateRead && dtMs > 0 && dtMs <= _tempoMs + (_tempoMs / 2))
            {
                _rateMdegS = ClampInt((long)turned * 1_000L / dtMs, MinTurnRateMdegS, MaxTurnRateMdegS);
            }
        }

        public int? PredictedMdeg(long nowMs)
        {
            if (!_seen.HasValue) return null;
            var seen = _seen.Value;
            HeadingAt anchor = _shown ?? new HeadingAt { Mdeg = seen.HeadingMdeg, AtMs = seen.AtMs };
            int ms = MsBetween(anchor.AtMs, nowMs);
            long v;

            if (_turning != 0 && _turnFrom.HasValue)
            {
                int dir = _turning;
                long from = _turnFrom.Value.AtMs;
                int swung = seen.CanTurn
                    ? SwingMdeg(MsBetween(from, nowMs)) - SwingMdeg(MsBetween(from, anchor.AtMs))
                    : 0;
                int room = Math.Max(LeashMdeg() - dir * ArcMdeg(seen.HeadingMdeg, anchor.Mdeg), 0);
                v = anchor.Mdeg + (long)dir * Math.Min(swung, room);
            }
            else if (_ordered.HasValue)
            {
                int run = seen.CanTurn ? (int)((long)_rateMdegS * ms / 1_000L) : 0;
                int d = ArcMdeg(anchor.Mdeg, _ordered.Value.Mdeg);
                int sgn = d > 0 ? 1 : (d < 0 ? -1 : 0);
                v = anchor.Mdeg + (long)sgn * Math.Min(Math.Abs(d), run);
            }
            else
            {
                int d = ArcMdeg(anchor.Mdeg, seen.HeadingMdeg);
                int sgn = d > 0 ? 1 : (d < 0 ? -1 : 0);
                int inPulse = (int)((long)Math.Abs(d) * ms / Math.Max(_tempoMs, 1));
                int atRate = (int)((long)_rateMdegS * ms / 1_000L);
                v = anchor.Mdeg + (long)sgn * Math.Min(Math.Abs(d), Math.Max(atRate, inPulse));
            }
            return Wrap360Mdeg(v);
        }

        public float? PredictedHeadingDeg(long nowMs)
        {
            int? mdeg = PredictedMdeg(nowMs);
            return mdeg.HasValue ? mdeg.Value * 0.001f : (float?)null;
        }

        private int GraceMs() => _tempoMs + 1_000;

        private int SwingMdeg(int ms)
        {
            int slow = Math.Min(Math.Max(ms, 0), TapMs);
            int fast = Math.Max(ms - slow, 0);
            int slowArc = Math.Min((int)((long)TapMdeg * slow / TapMs), (int)((long)_rateMdegS * slow / 1_000L));
            int fastArc = (int)((long)_rateMdegS * fast / 1_000L);
            return slowArc + fastArc;
        }

        private int LeadMdeg(int heldMs)
        {
            return ClampInt(SwingMdeg(heldMs + OrderGapMs) - SwingMdeg(heldMs), MinLeadMdeg, MaxLeadMdeg);
        }

        private int LeashMdeg()
        {
            return ClampInt((long)_rateMdegS * (_tempoMs + (_tempoMs / 2)) / 1_000L, MinLeashMdeg, MaxLeashMdeg);
        }

        private void OrderHeading(long headingMdeg, long nowMs, bool always = false)
        {
            int deg = (Wrap360Mdeg(headingMdeg) + 500) / 1_000 % 360;
            if (!always && OrderedMdeg == deg * 1_000) return;
            _orders.Add($"HEADING {deg} QUIET");
            _ordered = new HeadingAt { Mdeg = deg * 1_000, AtMs = nowMs };
        }

        public void Start(int dir, long nowMs)
        {
            if (dir == 0) return;
            dir = dir < 0 ? -1 : 1;
            int? p = PredictedMdeg(nowMs);
            if (!p.HasValue) return;
            _shown = new HeadingAt { Mdeg = p.Value, AtMs = nowMs };
            _turnFrom = new HeadingAt { Mdeg = p.Value, AtMs = nowMs };
            _turning = dir;
            OrderHeading(p.Value + dir * TapMdeg, nowMs);
            _priorOrderMdeg = null;
        }

        public void Stop(long nowMs)
        {
            if (_turning == 0) return;
            int dir = _turning;
            int? stands = PredictedMdeg(nowMs);
            _turning = 0;
            _priorOrderMdeg = null;
            var turnFrom = _turnFrom;
            _turnFrom = null;
            if (!stands.HasValue) return;
            _shown = new HeadingAt { Mdeg = stands.Value, AtMs = nowMs };
            int from = turnFrom.HasValue ? turnFrom.Value.Mdeg : stands.Value;
            long target = dir * ArcMdeg(from, stands.Value) < TapMdeg ? from + dir * TapMdeg : stands.Value;
            OrderHeading(target, nowMs);
        }

        public void Steer(int headingMdeg, long nowMs)
        {
            int? stands = PredictedMdeg(nowMs);
            _turning = 0;
            _turnFrom = null;
            _priorOrderMdeg = null;
            if (stands.HasValue) _shown = new HeadingAt { Mdeg = stands.Value, AtMs = nowMs };
            OrderHeading(headingMdeg, nowMs, always: true);
        }

        public void Tick(long nowMs)
        {
            if (!_seen.HasValue) return;
            var seen = _seen.Value;
            int grace = GraceMs();
            bool silent = (nowMs - seen.AtMs) > grace;
            if (_turning == 0)
            {
                if (silent && _ordered.HasValue && (nowMs - _ordered.Value.AtMs) > grace)
                {
                    int? p0 = PredictedMdeg(nowMs);
                    if (p0.HasValue) _shown = new HeadingAt { Mdeg = p0.Value, AtMs = nowMs };
                    _ordered = null;
                }
                return;
            }

            int? p = PredictedMdeg(nowMs);
            if (!p.HasValue || !_turnFrom.HasValue) return;
            if (silent && (nowMs - _turnFrom.Value.AtMs) > grace)
            {
                _shown = new HeadingAt { Mdeg = p.Value, AtMs = nowMs };
                _turning = 0;
                _turnFrom = null;
                _ordered = null;
                _priorOrderMdeg = null;
                return;
            }

            if (!_ordered.HasValue || (nowMs - _ordered.Value.AtMs) >= OrderGapMs)
            {
                int lead = LeadMdeg(MsBetween(_turnFrom.Value.AtMs, nowMs));
                int? was = OrderedMdeg;
                OrderHeading(p.Value + _turning * lead, nowMs);
                if (OrderedMdeg != was) _priorOrderMdeg = was;
            }
        }

        public List<string> TakeOrders()
        {
            var copy = new List<string>(_orders);
            _orders.Clear();
            return copy;
        }
    }

    /// <summary>
    /// Exact C# port of Doc 21 (`G.12b`) coalesced throttle (`web/src/lib/helm.ts` `Throttle`).
    /// Steps speed by 10% of max (minimum 5 kph) through 0 into reverse, emitting `SPEED <kph> QUIET`
    /// with 250ms order coalescing and immediate `SPEED 0 QUIET` on all-stop.
    /// </summary>
    public sealed class ArenaCoalescedThrottle
    {
        public const int DefaultMaxKph = 60;
        public const int DefaultReverseKph = 20;
        public const int CoalesceMs = 250;
        public const int ReachedKph = 1;

        private int? _ordered;
        private int? _pending;
        private long? _sentAtMs;
        private int? _seen;
        private int _maxKph = DefaultMaxKph;
        private int _reverseKph = DefaultReverseKph;
        private readonly List<string> _orders = new List<string>(8);

        public int? OrderedKph => _pending ?? _ordered;
        public int MaxKph => _maxKph;
        public int ReverseKph => _reverseKph;

        public void Reset()
        {
            _ordered = null;
            _pending = null;
            _sentAtMs = null;
            _seen = null;
            _maxKph = DefaultMaxKph;
            _reverseKph = DefaultReverseKph;
            _orders.Clear();
        }

        public static int SpeedStep(int maxKph)
        {
            return Math.Max(5, (int)Math.Round(maxKph / 10.0, MidpointRounding.AwayFromZero));
        }

        public static int SteppedSpeed(int fromKph, int dir, int maxKph, int reverseKph)
        {
            int step = SpeedStep(maxKph);
            int next = fromKph + (dir < 0 ? -step : step);
            return Mathf.Clamp(next, -reverseKph, maxKph);
        }

        public void Observe(ArenaThrottleFrame f)
        {
            if (f.MaxKph.HasValue) _maxKph = Math.Max(0, f.MaxKph.Value);
            if (f.ReverseKph.HasValue) _reverseKph = Math.Abs(f.ReverseKph.Value);
            _seen = f.SpeedKph;
            if (f.Stuck || (_ordered.HasValue && Math.Abs(_ordered.Value - f.SpeedKph) <= ReachedKph))
                _ordered = null;
        }

        public int PredictedKph()
        {
            return OrderedKph ?? _seen ?? 0;
        }

        public void Bump(int dir, long nowMs)
        {
            _pending = SteppedSpeed(PredictedKph(), dir, _maxKph, _reverseKph);
            Tick(nowMs);
        }

        public void Set(int kph, long nowMs)
        {
            _pending = Mathf.Clamp(kph, -_reverseKph, _maxKph);
            Tick(nowMs);
        }

        public void Stop(long nowMs)
        {
            _pending = 0;
            Flush(nowMs);
        }

        public void SyncExternalSpeedOrder(int kph)
        {
            _pending = null;
            _ordered = Mathf.Clamp(kph, -_reverseKph, _maxKph);
        }

        public void Flush(long nowMs)
        {
            _sentAtMs = null;
            Tick(nowMs);
        }

        public void Tick(long nowMs)
        {
            if (!_pending.HasValue) return;
            if (_sentAtMs.HasValue && (nowMs - _sentAtMs.Value) < CoalesceMs) return;
            int want = _pending.Value;
            _pending = null;
            _sentAtMs = nowMs;
            _ordered = want;
            _orders.Add($"SPEED {want} QUIET");
        }

        public List<string> TakeOrders()
        {
            var copy = new List<string>(_orders);
            _orders.Clear();
            return copy;
        }
    }
}
