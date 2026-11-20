using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using TacSim.Core;

namespace TacSim.Space
{
    /// <summary>
    /// Per-frame 6-DOF control input matching mush-pixel/src/docking.rs and tacsim/src/space/docking.rs (D-35-14).
    /// </summary>
    public struct DockingControlInput : IEquatable<DockingControlInput>
    {
        public sbyte TransX;       // -1 port, 0 coast, +1 starboard
        public sbyte TransY;       // -1 down, 0 coast, +1 up
        public sbyte ClosurePulse; // -1 braking/aft, 0 coast, +1 forward closure
        public sbyte PitchTrim;    // -1 nose down, 0 hold, +1 nose up
        public sbyte YawTrim;      // -1 nose left, 0 hold, +1 nose right
        public sbyte RollTrim;     // -1 CCW, 0 hold, +1 CW
        public bool ToggleDampeners;
        public bool Abort;

        private static ushort EncodeAxis(sbyte val)
        {
            if (val > 0) return 0b01;
            if (val < 0) return 0b10;
            return 0b00;
        }

        private static sbyte DecodeAxis(ushort bits)
        {
            switch (bits & 0b11)
            {
                case 0b01: return 1;
                case 0b10: return -1;
                default: return 0;
            }
        }

        public ushort ToBitmask()
        {
            ushort mask = 0;
            mask |= EncodeAxis(TransX);
            mask |= (ushort)(EncodeAxis(TransY) << 2);
            mask |= (ushort)(EncodeAxis(ClosurePulse) << 4);
            mask |= (ushort)(EncodeAxis(PitchTrim) << 6);
            mask |= (ushort)(EncodeAxis(YawTrim) << 8);
            mask |= (ushort)(EncodeAxis(RollTrim) << 10);
            if (ToggleDampeners) mask |= 1 << 12;
            if (Abort) mask |= 1 << 13;
            return mask;
        }

        public static DockingControlInput FromBitmask(ushort mask)
        {
            return new DockingControlInput
            {
                TransX = DecodeAxis(mask),
                TransY = DecodeAxis((ushort)(mask >> 2)),
                ClosurePulse = DecodeAxis((ushort)(mask >> 4)),
                PitchTrim = DecodeAxis((ushort)(mask >> 6)),
                YawTrim = DecodeAxis((ushort)(mask >> 8)),
                RollTrim = DecodeAxis((ushort)(mask >> 10)),
                ToggleDampeners = (mask & (1 << 12)) != 0,
                Abort = (mask & (1 << 13)) != 0
            };
        }

        public uint ActiveAxisPulses()
        {
            uint c = 0;
            if (TransX != 0) c++;
            if (TransY != 0) c++;
            if (ClosurePulse != 0) c++;
            if (PitchTrim != 0) c++;
            if (YawTrim != 0) c++;
            if (RollTrim != 0) c++;
            return c;
        }

        public bool Equals(DockingControlInput other) => ToBitmask() == other.ToBitmask();
        public override bool Equals(object obj) => obj is DockingControlInput o && Equals(o);
        public override int GetHashCode() => ToBitmask();
    }

    public struct DockingRcsAuthority
    {
        public long TransDvMmpsPerFrame;
        public long ClosureDvMmpsPerFrame;
        public int AttitudeRateMdegSPerFrame;
        public int RollRateMdegSPerFrame;
        public long DampTransMmpsPerFrame;
        public int DampAttitudeMdegSPerFrame;
        public int DampRollMdegSPerFrame;

        public static DockingRcsAuthority Default => new DockingRcsAuthority
        {
            TransDvMmpsPerFrame = 30,
            ClosureDvMmpsPerFrame = 25,
            AttitudeRateMdegSPerFrame = 180,
            RollRateMdegSPerFrame = 240,
            DampTransMmpsPerFrame = 15,
            DampAttitudeMdegSPerFrame = 90,
            DampRollMdegSPerFrame = 120
        };
    }

    public struct DockingGateStatus
    {
        public bool LateralOk;
        public bool AttitudeOk;
        public bool RollOk;
        public bool ClosureOk;
        public bool RangeOk;

        public int GreenCount
        {
            get
            {
                int c = 0;
                if (LateralOk) c++;
                if (AttitudeOk) c++;
                if (RollOk) c++;
                if (ClosureOk) c++;
                if (RangeOk) c++;
                return c;
            }
        }

        public bool AllGreen => LateralOk && AttitudeOk && RollOk && ClosureOk && RangeOk;
    }

    /// <summary>
    /// Bit-exact 60 Hz modulo-60 integer 6-DOF docking kinematics &amp; RLE tape recorder
    /// ported 1:1 from crates/mush-pixel/src/docking.rs and crates/tacsim/src/space/docking.rs.
    /// Guarantees server replay_docking_tape verification matches client frame-for-frame.
    /// </summary>
    public sealed class DockingClientSession
    {
        public const int MaxTapeFrames = 3600;
        public const long GateLateralMm = 250;
        public const long GateMaxLatVelMmps = 300;
        public const int GateAttitudeMdeg = 2000;
        public const int GateRollMdeg = 3000;
        public const int GateMaxAngRateMdegS = 500;
        public const long GateClosureMinMmps = 100;
        public const long GateClosureMaxMmps = 800;
        public const long GateRangeZMm = 500;

        public string SessionId = "dock.0.1";
        public string TargetLabel = "Ceres Freeport Berth";

        public long DxMm;
        public long DyMm;
        public long DzMm;
        public long PitchMdeg;
        public long YawMdeg;
        public long RollMdeg;
        public long VxMmps;
        public long VyMmps;
        public long VzMmps;
        public int PitchRateMdegS;
        public int YawRateMdegS;
        public int RollRateMdegS;

        public int[] TargetTumbleMdegS = new int[3];
        public DockingRcsAuthority RcsAuthority = DockingRcsAuthority.Default;
        public bool DampenersOn = true;
        public int MaxFrames = MaxTapeFrames;

        private long _remX;
        private long _remY;
        private long _remZ;
        private int _remPitch;
        private int _remYaw;
        private int _remRoll;

        private readonly List<(ushort mask, ushort count)> _spans = new List<(ushort, ushort)>();
        public int FramesStepped { get; private set; }
        public uint TotalRcsPulses { get; private set; }

        /// <summary>
        /// Maximum safe `+helm/dock tape &lt;session_id&gt; &lt;rle&gt;` command length (gateway cap is 8,192 chars).
        /// </summary>
        public const int MaxSafeCommandChars = 7600;

        public int EffectiveMaxFrames => MaxFrames > 0 ? Math.Min(MaxFrames, MaxTapeFrames) : MaxTapeFrames;

        public int EstimatedCommandLength => 24 + (SessionId?.Length ?? 8) + _spans.Count * 10;

        public bool IsTapeBudgetExhausted => FramesStepped >= EffectiveMaxFrames || EstimatedCommandLength >= MaxSafeCommandChars;

        public static DockingClientSession CreateStandardPortPractice(string sessionId = "dock.demo.1", string label = "Ceres Freeport Cradle 04")
        {
            return new DockingClientSession
            {
                SessionId = sessionId,
                TargetLabel = label,
                DxMm = 600,
                DyMm = -420,
                DzMm = 2400,
                PitchMdeg = -2400,
                YawMdeg = 2400,
                RollMdeg = -3600,
                VxMmps = 0,
                VyMmps = 0,
                VzMmps = 0,
                RcsAuthority = DockingRcsAuthority.Default,
                DampenersOn = true
            };
        }

        public static DockingClientSession FromFeed(JsonNode feed)
        {
            if (feed == null || !feed.IsObject)
                return CreateStandardPortPractice();

            // `replay_docking_tape` on `mushd` ALWAYS replays submitted tapes from `challenge.start_pose`
            // with `dampeners_on = true` and enforces `challenge.max_frames`. Prefer `feed["start_pose"]`
            // whenever present so reconnects or prior text steps never diverge from server verification.
            var startPose = feed["start_pose"];
            bool hasStartPose = startPose.IsObject;
            JsonNode poseSrc = hasStartPose ? startPose : feed;

            int feedMaxFrames = feed.Int("max_frames", MaxTapeFrames);
            int clampedMaxFrames = feedMaxFrames > 0 ? Math.Min(feedMaxFrames, MaxTapeFrames) : MaxTapeFrames;

            var s = new DockingClientSession
            {
                SessionId = feed.Str("session_id", "dock.0.1"),
                TargetLabel = feed.Str("target_label", feed.Str("target_name", "Docking Collar")),
                DxMm = poseSrc.Long("dx_mm", poseSrc.Long("lateral_x_mm", 600)),
                DyMm = poseSrc.Long("dy_mm", poseSrc.Long("lateral_y_mm", -420)),
                DzMm = poseSrc.Long("dz_mm", poseSrc.Long("range_z_mm", 2400)),
                PitchMdeg = poseSrc.Long("pitch_mdeg", poseSrc.Long("pitch_err_mdeg", -2400)),
                YawMdeg = poseSrc.Long("yaw_mdeg", poseSrc.Long("yaw_err_mdeg", 2400)),
                RollMdeg = poseSrc.Long("roll_mdeg", poseSrc.Long("roll_err_mdeg", -3600)),
                VxMmps = poseSrc.Long("vx_mmps", poseSrc.Long("vel_x_mmps", 0)),
                VyMmps = poseSrc.Long("vy_mmps", poseSrc.Long("vel_y_mmps", 0)),
                VzMmps = poseSrc.Long("vz_mmps", poseSrc.Long("closure_mmps", 0)),
                PitchRateMdegS = poseSrc.Int("pitch_rate_mdeg_s", 0),
                YawRateMdegS = poseSrc.Int("yaw_rate_mdeg_s", 0),
                RollRateMdegS = poseSrc.Int("roll_rate_mdeg_s", 0),
                DampenersOn = hasStartPose ? true : (feed.ContainsKey("dampeners") ? feed.Bool("dampeners", true) : true),
                MaxFrames = clampedMaxFrames
            };

            var tumble = feed["target_tumble_mdeg_s"].AsArray();
            if (tumble != null && tumble.Count >= 3)
            {
                s.TargetTumbleMdegS[0] = tumble[0].AsInt(0);
                s.TargetTumbleMdegS[1] = tumble[1].AsInt(0);
                s.TargetTumbleMdegS[2] = tumble[2].AsInt(0);
            }

            var auth = feed["rcs_authority"];
            if (auth.IsObject)
            {
                s.RcsAuthority = new DockingRcsAuthority
                {
                    TransDvMmpsPerFrame = auth.Long("trans_dv_mmps_per_frame", 30),
                    ClosureDvMmpsPerFrame = auth.Long("closure_dv_mmps_per_frame", 25),
                    AttitudeRateMdegSPerFrame = auth.Int("attitude_rate_mdeg_s_per_frame", 180),
                    RollRateMdegSPerFrame = auth.Int("roll_rate_mdeg_s_per_frame", 240),
                    DampTransMmpsPerFrame = auth.Long("damp_trans_mmps_per_frame", 15),
                    DampAttitudeMdegSPerFrame = auth.Int("damp_attitude_mdeg_s_per_frame", 90),
                    DampRollMdegSPerFrame = auth.Int("damp_roll_mdeg_s_per_frame", 120)
                };
            }
            return s;
        }

        private static long BleedI64(long val, long step)
        {
            if (val > step) return val - step;
            if (val < -step) return val + step;
            return 0;
        }

        private static int BleedI32(int val, int step)
        {
            if (val > step) return val - step;
            if (val < -step) return val + step;
            return 0;
        }

        private static long ClampI8(sbyte v) => v > 1 ? 1 : (v < -1 ? -1 : v);

        public void Step60Fps(DockingControlInput input)
        {
            if (FramesStepped >= EffectiveMaxFrames)
                return;

            ushort mask = input.ToBitmask();
            if (_spans.Count > 0 && _spans[_spans.Count - 1].mask == mask && _spans[_spans.Count - 1].count < ushort.MaxValue)
            {
                var last = _spans[_spans.Count - 1];
                _spans[_spans.Count - 1] = (last.mask, (ushort)(last.count + 1));
            }
            else
            {
                _spans.Add((mask, 1));
            }

            if (input.ToggleDampeners)
                DampenersOn = !DampenersOn;

            long tx = ClampI8(input.TransX);
            long ty = ClampI8(input.TransY);
            long tz = ClampI8(input.ClosurePulse);
            int dpitch = (int)ClampI8(input.PitchTrim);
            int dyaw = (int)ClampI8(input.YawTrim);
            int droll = (int)ClampI8(input.RollTrim);

            VxMmps += tx * RcsAuthority.TransDvMmpsPerFrame;
            VyMmps += ty * RcsAuthority.TransDvMmpsPerFrame;
            VzMmps += tz * RcsAuthority.ClosureDvMmpsPerFrame;
            PitchRateMdegS += dpitch * RcsAuthority.AttitudeRateMdegSPerFrame;
            YawRateMdegS += dyaw * RcsAuthority.AttitudeRateMdegSPerFrame;
            RollRateMdegS += droll * RcsAuthority.RollRateMdegSPerFrame;

            uint framePulses = input.ActiveAxisPulses();

            if (DampenersOn)
            {
                if (tx == 0 && VxMmps != 0)
                {
                    VxMmps = BleedI64(VxMmps, RcsAuthority.DampTransMmpsPerFrame);
                    framePulses++;
                }
                if (ty == 0 && VyMmps != 0)
                {
                    VyMmps = BleedI64(VyMmps, RcsAuthority.DampTransMmpsPerFrame);
                    framePulses++;
                }
                if (dpitch == 0 && PitchRateMdegS != 0)
                {
                    PitchRateMdegS = BleedI32(PitchRateMdegS, RcsAuthority.DampAttitudeMdegSPerFrame);
                    framePulses++;
                }
                if (dyaw == 0 && YawRateMdegS != 0)
                {
                    YawRateMdegS = BleedI32(YawRateMdegS, RcsAuthority.DampAttitudeMdegSPerFrame);
                    framePulses++;
                }
                if (droll == 0 && RollRateMdegS != 0)
                {
                    RollRateMdegS = BleedI32(RollRateMdegS, RcsAuthority.DampRollMdegSPerFrame);
                    framePulses++;
                }
            }

            // Note: Rust `/` and `%` on signed integers truncate toward zero — identical to C# `/` and `%`!
            long totX = VxMmps + _remX;
            DxMm += totX / 60;
            _remX = totX % 60;

            long totY = VyMmps + _remY;
            DyMm += totY / 60;
            _remY = totY % 60;

            long totZ = VzMmps + _remZ;
            DzMm = Math.Max(0L, DzMm - (totZ / 60));
            _remZ = totZ % 60;

            int netPitch = PitchRateMdegS - TargetTumbleMdegS[0];
            int totPitch = netPitch + _remPitch;
            PitchMdeg += totPitch / 60;
            _remPitch = totPitch % 60;

            int netYaw = YawRateMdegS - TargetTumbleMdegS[1];
            int totYaw = netYaw + _remYaw;
            YawMdeg += totYaw / 60;
            _remYaw = totYaw % 60;

            int netRoll = RollRateMdegS - TargetTumbleMdegS[2];
            int totRoll = netRoll + _remRoll;
            RollMdeg += totRoll / 60;
            _remRoll = totRoll % 60;

            PitchMdeg = Math.Clamp(PitchMdeg, int.MinValue, int.MaxValue);
            YawMdeg = Math.Clamp(YawMdeg, int.MinValue, int.MaxValue);
            RollMdeg = Math.Clamp(RollMdeg, int.MinValue, int.MaxValue);

            TotalRcsPulses += framePulses;
            FramesStepped++;
        }

        public DockingGateStatus EvaluateGates()
        {
            return new DockingGateStatus
            {
                LateralOk = Math.Abs(DxMm) <= GateLateralMm
                            && Math.Abs(DyMm) <= GateLateralMm
                            && Math.Abs(VxMmps) <= GateMaxLatVelMmps
                            && Math.Abs(VyMmps) <= GateMaxLatVelMmps,
                AttitudeOk = Math.Abs(PitchMdeg) <= GateAttitudeMdeg
                             && Math.Abs(YawMdeg) <= GateAttitudeMdeg
                             && Math.Abs(PitchRateMdegS) <= GateMaxAngRateMdegS
                             && Math.Abs(YawRateMdegS) <= GateMaxAngRateMdegS,
                RollOk = Math.Abs(RollMdeg) <= GateRollMdeg
                         && Math.Abs(RollRateMdegS) <= GateMaxAngRateMdegS,
                ClosureOk = VzMmps >= GateClosureMinMmps && VzMmps <= GateClosureMaxMmps,
                RangeOk = DzMm >= 0 && DzMm <= GateRangeZMm
            };
        }

        public bool IsClamped => EvaluateGates().AllGreen;

        public bool IsCollarScrapeBounce => DzMm == 0 && VzMmps > 0 && !IsClamped;

        /// <summary>
        /// Closed-loop guidance helper (matching mush-pixel's next_guidance_input) for optional pilot flight-assist [G].
        /// </summary>
        public DockingControlInput ComputeGuidanceAssistInput()
        {
            var input = new DockingControlInput();
            long dvTrans = Math.Max(5L, RcsAuthority.TransDvMmpsPerFrame);
            long dvClose = Math.Max(5L, RcsAuthority.ClosureDvMmpsPerFrame);
            int dAtt = Math.Max(30, RcsAuthority.AttitudeRateMdegSPerFrame);
            int dRoll = Math.Max(30, RcsAuthority.RollRateMdegSPerFrame);

            long fastVx = Math.Min(240L, dvTrans * 6);
            long slowVx = Math.Min(120L, dvTrans * 2);
            long deadVx = Math.Max(5L, dvTrans / 2);

            long targetVx = DxMm > 120 ? -fastVx : (DxMm < -120 ? fastVx : (DxMm > 20 ? -slowVx : (DxMm < -20 ? slowVx : 0)));
            if (VxMmps < targetVx - deadVx) input.TransX = 1;
            else if (VxMmps > targetVx + deadVx) input.TransX = -1;

            long targetVy = DyMm > 120 ? -fastVx : (DyMm < -120 ? fastVx : (DyMm > 20 ? -slowVx : (DyMm < -20 ? slowVx : 0)));
            if (VyMmps < targetVy - deadVx) input.TransY = 1;
            else if (VyMmps > targetVy + deadVx) input.TransY = -1;

            int fastAtt = Math.Min(420, dAtt * 2);
            int slowAtt = Math.Min(240, dAtt);
            int deadAtt = Math.Max(15, dAtt / 2);

            int targetPitchRate = PitchMdeg > 600 ? -fastAtt : (PitchMdeg < -600 ? fastAtt : (PitchMdeg > 90 ? -slowAtt : (PitchMdeg < -90 ? slowAtt : TargetTumbleMdegS[0])));
            if (PitchRateMdegS < targetPitchRate - deadAtt) input.PitchTrim = 1;
            else if (PitchRateMdegS > targetPitchRate + deadAtt) input.PitchTrim = -1;

            int targetYawRate = YawMdeg > 600 ? -fastAtt : (YawMdeg < -600 ? fastAtt : (YawMdeg > 90 ? -slowAtt : (YawMdeg < -90 ? slowAtt : TargetTumbleMdegS[1])));
            if (YawRateMdegS < targetYawRate - deadAtt) input.YawTrim = 1;
            else if (YawRateMdegS > targetYawRate + deadAtt) input.YawTrim = -1;

            int fastRoll = Math.Min(480, dRoll * 2);
            int slowRoll = Math.Min(240, dRoll);
            int deadRoll = Math.Max(15, dRoll / 2);

            int targetRollRate = RollMdeg > 900 ? -fastRoll : (RollMdeg < -900 ? fastRoll : (RollMdeg > 120 ? -slowRoll : (RollMdeg < -120 ? slowRoll : TargetTumbleMdegS[2])));
            if (RollRateMdegS < targetRollRate - deadRoll) input.RollTrim = 1;
            else if (RollRateMdegS > targetRollRate + deadRoll) input.RollTrim = -1;

            var gates = EvaluateGates();
            long targetClosure = (gates.LateralOk && gates.AttitudeOk && gates.RollOk) ? 350L : 0L;
            long deadClose = Math.Clamp(dvClose, 5L, 20L);
            if (VzMmps < targetClosure - deadClose) input.ClosurePulse = 1;
            else if (VzMmps > targetClosure + deadClose) input.ClosurePulse = -1;

            return input;
        }

        public string EncodeRle()
        {
            var sb = new StringBuilder(_spans.Count * 9);
            for (int i = 0; i < _spans.Count; i++)
            {
                if (i > 0) sb.Append(',');
                sb.Append(_spans[i].mask.ToString("x4", CultureInfo.InvariantCulture));
                sb.Append('*');
                sb.Append(_spans[i].count.ToString(CultureInfo.InvariantCulture));
            }
            return sb.ToString();
        }

        public string FormatTapeCommand()
        {
            return $"+helm/dock tape {SessionId} {EncodeRle()}";
        }
    }
}
