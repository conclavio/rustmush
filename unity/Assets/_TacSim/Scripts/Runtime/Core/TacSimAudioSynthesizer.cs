using System;
using System.Collections.Generic;
using System.Diagnostics;
using UnityEngine;

namespace TacSim.Core
{
    public enum AmbientSoundscape
    {
        StationRing = 0,
        ShipInterior = 1,
        OrbitalSpace = 2
    }

    /// <summary>
    /// Zero-dependency procedural sci-fi soundscape &amp; diegetic SFX synthesizer:
    /// - Warm low-frequency station spin-ring / ship fusion / orbital canopy hum
    /// - Tactile terminal button chime &amp; two-tone market quote/fill chord
    /// - Pneumatic airlock pressure-cycle hiss (+ship/board &amp; +ship/leave)
    /// - 6-DOF docking collar hard-clamp lock chord
    /// - Tactical sensor ping, dual-mode laser pulse, railgun crack &amp; torpedo tube launch
    /// </summary>
    public sealed class TacSimAudioSynthesizer : MonoBehaviour
    {
        public const string MutePlayerPrefKey = "TacSim.AudioMuted";
        public const string MasterVolPrefKey = "TacSim.AudioMasterPct";
        public const string AmbientVolPrefKey = "TacSim.AudioAmbientPct";
        public const string SfxVolPrefKey = "TacSim.AudioSfxPct";
        public const int SampleRate = 22050;
        private const float BaseAmbientGain = 0.085f;
        private const float BaseSfxGain = 0.22f;
        private const double MinPerClipIntervalSec = 0.045;
        private const double MinGlobalBurstIntervalSec = 0.015;

        public bool IsMuted { get; private set; }
        public int MasterVolumePct { get; private set; } = 100;
        public int AmbientVolumePct { get; private set; } = 80;
        public int SfxVolumePct { get; private set; } = 90;
        public float EffectiveAmbientVolume => IsMuted ? 0f : BaseAmbientGain * (MasterVolumePct / 100f) * (AmbientVolumePct / 100f);
        public float EffectiveSfxVolume => IsMuted ? 0f : BaseSfxGain * (MasterVolumePct / 100f) * (SfxVolumePct / 100f);
        public float CurrentAmbientPitch => _ambientLoop != null ? _ambientLoop.pitch : 1f;
        public int AllocatedClipCount => CountAllocatedClips();

        private AudioSource _ambientLoop;
        private AudioSource _sfxSource;
        private readonly Dictionary<AudioClip, double> _lastPlayByClip = new Dictionary<AudioClip, double>();
        private double _lastGlobalOneShotSec = -10.0;

        private AudioClip _stationHumClip;
        private AudioClip _shipHumClip;
        private AudioClip _spaceHumClip;
        private AudioClip _uiClickClip;
        private AudioClip _marketChimeClip;
        private AudioClip _airlockCycleClip;
        private AudioClip _dockClampClip;
        private AudioClip _radarPingClip;
        private AudioClip _laserPulseClip;
        private AudioClip _railgunCrackClip;
        private AudioClip _torpedoLaunchClip;
        private AudioClip _redAlertKlaxonClip;
        private AudioClip _targetLockToneClip;
        private AudioClip _killConfirmChordClip;
        private AudioClip _hitConfirmTickClip;
        private AudioClip _armorImpactThudClip;

        private static double MonotonicSeconds() =>
            Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency;

        private void Awake()
        {
            InitializeAudioGraph();
        }

        private void OnDestroy()
        {
            DisposeClips();
        }

        public void DisposeClips()
        {
            if (_ambientLoop != null)
            {
                _ambientLoop.Stop();
                _ambientLoop.clip = null;
            }
            if (_sfxSource != null)
                _sfxSource.Stop();

            _lastPlayByClip.Clear();
            DestroyClip(ref _stationHumClip);
            DestroyClip(ref _shipHumClip);
            DestroyClip(ref _spaceHumClip);
            DestroyClip(ref _uiClickClip);
            DestroyClip(ref _marketChimeClip);
            DestroyClip(ref _airlockCycleClip);
            DestroyClip(ref _dockClampClip);
            DestroyClip(ref _radarPingClip);
            DestroyClip(ref _laserPulseClip);
            DestroyClip(ref _railgunCrackClip);
            DestroyClip(ref _torpedoLaunchClip);
            DestroyClip(ref _redAlertKlaxonClip);
            DestroyClip(ref _targetLockToneClip);
            DestroyClip(ref _killConfirmChordClip);
            DestroyClip(ref _hitConfirmTickClip);
            DestroyClip(ref _armorImpactThudClip);
        }

        private static void DestroyClip(ref AudioClip clip)
        {
            if (clip == null) return;
            if (Application.isPlaying) Destroy(clip);
            else DestroyImmediate(clip);
            clip = null;
        }

        private int CountAllocatedClips()
        {
            int c = 0;
            if (_stationHumClip != null) c++;
            if (_shipHumClip != null) c++;
            if (_spaceHumClip != null) c++;
            if (_uiClickClip != null) c++;
            if (_marketChimeClip != null) c++;
            if (_airlockCycleClip != null) c++;
            if (_dockClampClip != null) c++;
            if (_radarPingClip != null) c++;
            if (_laserPulseClip != null) c++;
            if (_railgunCrackClip != null) c++;
            if (_torpedoLaunchClip != null) c++;
            if (_redAlertKlaxonClip != null) c++;
            if (_targetLockToneClip != null) c++;
            if (_killConfirmChordClip != null) c++;
            if (_hitConfirmTickClip != null) c++;
            if (_armorImpactThudClip != null) c++;
            return c;
        }

        public void InitializeAudioGraph()
        {
            if (_ambientLoop != null && _stationHumClip != null) return;

            IsMuted = PlayerPrefs.GetInt(MutePlayerPrefKey, 0) == 1;
            MasterVolumePct = Mathf.Clamp(PlayerPrefs.GetInt(MasterVolPrefKey, 100), 0, 100);
            AmbientVolumePct = Mathf.Clamp(PlayerPrefs.GetInt(AmbientVolPrefKey, 80), 0, 100);
            SfxVolumePct = Mathf.Clamp(PlayerPrefs.GetInt(SfxVolPrefKey, 90), 0, 100);

            if (_ambientLoop == null)
            {
                _ambientLoop = gameObject.AddComponent<AudioSource>();
                _ambientLoop.loop = true;
                _ambientLoop.spatialBlend = 0f;
                _ambientLoop.volume = EffectiveAmbientVolume;
                _ambientLoop.playOnAwake = false;
            }

            if (_sfxSource == null)
            {
                _sfxSource = gameObject.AddComponent<AudioSource>();
                _sfxSource.loop = false;
                _sfxSource.spatialBlend = 0f;
                _sfxSource.volume = EffectiveSfxVolume;
                _sfxSource.playOnAwake = false;
            }

            if (_stationHumClip == null) _stationHumClip = BuildDroneLoop("StationHum", baseHz: 55f, fifthHz: 82.5f, airNoise: 0.018f);
            if (_shipHumClip == null) _shipHumClip = BuildDroneLoop("ShipReactorHum", baseHz: 64f, fifthHz: 128f, airNoise: 0.026f);
            if (_spaceHumClip == null) _spaceHumClip = BuildDroneLoop("SpaceCanopyHum", baseHz: 42f, fifthHz: 63f, airNoise: 0.008f);
            if (_uiClickClip == null) _uiClickClip = BuildClipFromSamples("UiChime", GenerateUiChimeSamples());
            if (_marketChimeClip == null) _marketChimeClip = BuildClipFromSamples("MarketChime", GenerateMarketChimeSamples());
            if (_airlockCycleClip == null) _airlockCycleClip = BuildClipFromSamples("AirlockCycle", GenerateAirlockHissSamples());
            if (_dockClampClip == null) _dockClampClip = BuildClipFromSamples("DockHardClamp", GenerateDockClampSamples());
            if (_radarPingClip == null) _radarPingClip = BuildClipFromSamples("RadarScanPing", GenerateRadarPingSamples());
            if (_laserPulseClip == null) _laserPulseClip = BuildClipFromSamples("LaserPulse", GenerateLaserPulseSamples());
            if (_railgunCrackClip == null) _railgunCrackClip = BuildClipFromSamples("RailgunCrack", GenerateRailgunCrackSamples());
            if (_torpedoLaunchClip == null) _torpedoLaunchClip = BuildClipFromSamples("TorpedoLaunch", GenerateTorpedoLaunchSamples());

            SetSoundscape(AmbientSoundscape.StationRing);
        }

        public void SetMuted(bool muted)
        {
            IsMuted = muted;
            PlayerPrefs.SetInt(MutePlayerPrefKey, muted ? 1 : 0);
            PlayerPrefs.Save();
            ApplyMixerVolumes();
        }

        public void SetMasterVolumePct(int pct)
        {
            MasterVolumePct = Mathf.Clamp(pct, 0, 100);
            PlayerPrefs.SetInt(MasterVolPrefKey, MasterVolumePct);
            PlayerPrefs.Save();
            ApplyMixerVolumes();
        }

        public void SetAmbientVolumePct(int pct)
        {
            AmbientVolumePct = Mathf.Clamp(pct, 0, 100);
            PlayerPrefs.SetInt(AmbientVolPrefKey, AmbientVolumePct);
            PlayerPrefs.Save();
            ApplyMixerVolumes();
        }

        public void SetSfxVolumePct(int pct)
        {
            SfxVolumePct = Mathf.Clamp(pct, 0, 100);
            PlayerPrefs.SetInt(SfxVolPrefKey, SfxVolumePct);
            PlayerPrefs.Save();
            ApplyMixerVolumes();
        }

        public void ResetMixerDefaults()
        {
            IsMuted = false;
            MasterVolumePct = 100;
            AmbientVolumePct = 80;
            SfxVolumePct = 90;
            PlayerPrefs.SetInt(MutePlayerPrefKey, 0);
            PlayerPrefs.SetInt(MasterVolPrefKey, MasterVolumePct);
            PlayerPrefs.SetInt(AmbientVolPrefKey, AmbientVolumePct);
            PlayerPrefs.SetInt(SfxVolPrefKey, SfxVolumePct);
            PlayerPrefs.Save();
            ApplyMixerVolumes();
        }

        private void ApplyMixerVolumes()
        {
            float ambVol = EffectiveAmbientVolume;
            if (_ambientLoop != null)
            {
                _ambientLoop.volume = ambVol;
                if (IsMuted || ambVol <= 0.0001f)
                {
                    if (_ambientLoop.isPlaying)
                        _ambientLoop.Stop();
                }
                else if (_ambientLoop.clip != null && !_ambientLoop.isPlaying)
                {
                    _ambientLoop.Play();
                }
            }
            if (_sfxSource != null)
            {
                _sfxSource.volume = EffectiveSfxVolume;
            }
        }

        public bool ToggleMuted()
        {
            SetMuted(!IsMuted);
            return IsMuted;
        }

        public void SetSoundscape(AmbientSoundscape mode)
        {
            if (_ambientLoop == null) return;
            _ambientLoop.pitch = 1.0f;
            AudioClip desired = mode == AmbientSoundscape.ShipInterior ? _shipHumClip
                              : mode == AmbientSoundscape.OrbitalSpace ? _spaceHumClip
                              : _stationHumClip;
            bool shouldSilence = IsMuted || EffectiveAmbientVolume <= 0.0001f;
            if (_ambientLoop.clip == desired && (_ambientLoop.isPlaying || shouldSilence))
                return;

            _ambientLoop.clip = desired;
            _ambientLoop.volume = EffectiveAmbientVolume;
            if (desired != null && !shouldSilence)
                _ambientLoop.Play();
            else if (shouldSilence && _ambientLoop.isPlaying)
                _ambientLoop.Stop();
        }

        public void SetArenaEngineThrottle(float speedFrac)
        {
            if (_ambientLoop == null) return;
            _ambientLoop.pitch = Mathf.Lerp(0.95f, 1.22f, Mathf.Clamp01(speedFrac));
        }

        public void ResetAmbientPitch()
        {
            if (_ambientLoop != null)
                _ambientLoop.pitch = 1.0f;
        }

        public void PlayUiClick() => PlayOneShotIfUnmuted(_uiClickClip, 0.35f);
        public void PlayMarketChime() => PlayOneShotIfUnmuted(_marketChimeClip, 0.50f);
        public void PlayAirlockCycle() => PlayOneShotIfUnmuted(_airlockCycleClip, 0.75f);
        public void PlayDockHardClamp() => PlayOneShotIfUnmuted(_dockClampClip, 0.90f);
        public void PlayRadarScanPing() => PlayOneShotIfUnmuted(_radarPingClip, 0.55f);
        public void PlayLaserPulse() => PlayOneShotIfUnmuted(_laserPulseClip, 0.55f);
        public void PlayRailgunCrack() => PlayOneShotIfUnmuted(_railgunCrackClip, 0.70f);
        public void PlayTorpedoLaunch() => PlayOneShotIfUnmuted(_torpedoLaunchClip, 0.65f);
        public void PlayRedAlertKlaxon()
        {
            if (_redAlertKlaxonClip == null)
                _redAlertKlaxonClip = BuildClipFromSamples("RedAlertKlaxon", GenerateRedAlertKlaxonSamples());
            PlayOneShotIfUnmuted(_redAlertKlaxonClip, 0.78f);
        }

        public void PlayTargetLockTone()
        {
            if (_targetLockToneClip == null)
                _targetLockToneClip = BuildClipFromSamples("TargetLockTone", GenerateTargetLockToneSamples());
            PlayOneShotIfUnmuted(_targetLockToneClip, 0.52f);
        }

        public void PlayKillConfirmChord()
        {
            if (_killConfirmChordClip == null)
                _killConfirmChordClip = BuildClipFromSamples("KillConfirmChord", GenerateKillConfirmChordSamples());
            PlayOneShotIfUnmuted(_killConfirmChordClip, 0.66f);
        }

        public void PlayHitConfirmTick(bool isCrit = false)
        {
            if (_hitConfirmTickClip == null)
                _hitConfirmTickClip = BuildClipFromSamples("HitConfirmTick", GenerateHitConfirmTickSamples());
            PlayOneShotIfUnmuted(_hitConfirmTickClip, isCrit ? 0.64f : 0.44f);
        }

        public void PlayArmorImpactThud()
        {
            if (_armorImpactThudClip == null)
                _armorImpactThudClip = BuildClipFromSamples("ArmorImpactThud", GenerateArmorImpactThudSamples());
            PlayOneShotIfUnmuted(_armorImpactThudClip, 0.72f);
        }

        /// <summary>
        /// Triggers matching tactical / market SFX for an outbound MUSH command line.
        /// </summary>
        public void PlayForCommandLine(string commandLine)
        {
            if (IsMuted || string.IsNullOrWhiteSpace(commandLine)) return;
            string low = commandLine.Trim().ToLowerInvariant();
            if (low.StartsWith("+tac/beam", StringComparison.Ordinal) || low.StartsWith("fire 1", StringComparison.Ordinal))
                PlayLaserPulse();
            else if (low.StartsWith("+tac/railgun", StringComparison.Ordinal))
                PlayRailgunCrack();
            else if (low.StartsWith("+tac/torp", StringComparison.Ordinal))
                PlayTorpedoLaunch();
            else if (low.StartsWith("lock ", StringComparison.Ordinal))
                PlayTargetLockTone();
            else if (low.StartsWith("+tac/scan", StringComparison.Ordinal) ||
                     low.StartsWith("+tac/lock", StringComparison.Ordinal) ||
                     low.StartsWith("+tv/scan", StringComparison.Ordinal) ||
                     low == "scan" || low.StartsWith("scan ", StringComparison.Ordinal) ||
                     low == "emcon" || low.StartsWith("emcon ", StringComparison.Ordinal))
                PlayRadarScanPing();
            else if (low == "mind" || low.StartsWith("mind ", StringComparison.Ordinal))
                PlayUiClick();
            else if (low.StartsWith("+market/accept", StringComparison.Ordinal) ||
                     low.StartsWith("+market/quote", StringComparison.Ordinal) ||
                     low.StartsWith("+market/sell", StringComparison.Ordinal) ||
                     low.StartsWith("+market/post", StringComparison.Ordinal) ||
                     low.StartsWith("+market/bid", StringComparison.Ordinal))
                PlayMarketChime();
            else if (low.StartsWith("fire ", StringComparison.Ordinal) || low == "fire")
                PlayRailgunCrack();
        }

        private void PlayOneShotIfUnmuted(AudioClip clip, float scale)
        {
            if (IsMuted || _sfxSource == null || clip == null) return;
            double now = MonotonicSeconds();
            if (now - _lastGlobalOneShotSec < MinGlobalBurstIntervalSec)
                return;
            if (_lastPlayByClip.TryGetValue(clip, out double prev) && now - prev < MinPerClipIntervalSec)
                return;
            _lastPlayByClip[clip] = now;
            _lastGlobalOneShotSec = now;
            _sfxSource.PlayOneShot(clip, scale);
        }

        private static AudioClip BuildClipFromSamples(string name, float[] samples)
        {
            var clip = AudioClip.Create(name, samples.Length, 1, SampleRate, false);
            clip.SetData(samples, 0);
            return clip;
        }

        private static AudioClip BuildDroneLoop(string name, float baseHz, float fifthHz, float airNoise)
        {
            int samples = SampleRate * 4;
            int xfade = SampleRate / 10; // 100 ms seamless loop crossfade for filtered noise
            float[] noiseBuf = new float[samples + xfade];
            uint rng = 0x9E3779B9u;
            float lp = 0f;
            for (int i = 0; i < noiseBuf.Length; i++)
            {
                rng = rng * 1664525u + 1013904223u;
                float white = ((rng & 0xFFFFu) / 32768f) - 1f;
                lp = lp * 0.94f + white * 0.06f;
                noiseBuf[i] = lp;
            }

            float[] buf = new float[samples];
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float s1 = Mathf.Sin(t * baseHz * Mathf.PI * 2f) * 0.45f;
                float s2 = Mathf.Sin(t * fifthHz * Mathf.PI * 2f) * 0.22f;
                float lfo = 0.85f + 0.15f * Mathf.Sin(t * 0.25f * Mathf.PI * 2f);

                float n = noiseBuf[i];
                if (i < xfade)
                {
                    float w = i / (float)xfade;
                    n = noiseBuf[samples + i] * (1f - w) + noiseBuf[i] * w;
                }

                buf[i] = Mathf.Clamp((s1 + s2) * lfo + n * airNoise, -1f, 1f);
            }

            return BuildClipFromSamples(name, buf);
        }

        public static float[] GenerateUiChimeSamples()
        {
            int samples = (int)(SampleRate * 0.085f);
            float[] buf = new float[samples];
            float phase = 0f;
            int split = samples / 2;
            int ramp = Math.Max(1, (int)(SampleRate * 0.002f));
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float env = Mathf.Exp(-t * 42f);
                if (i < ramp) env *= i / (float)ramp;
                if (i >= samples - ramp) env *= (samples - 1 - i) / (float)ramp;
                float freq = i < split ? 880f : 1174.66f;
                phase += (freq / SampleRate) * Mathf.PI * 2f;
                buf[i] = Mathf.Sin(phase) * env * 0.45f;
            }
            return buf;
        }

        public static float[] GenerateMarketChimeSamples()
        {
            int samples = (int)(SampleRate * 0.24f);
            float[] buf = new float[samples];
            float p1 = 0f;
            float p2 = 0f;
            int splitIdx = (int)(SampleRate * 0.09f);
            int ramp = Math.Max(1, (int)(SampleRate * 0.003f));
            for (int i = 0; i < samples; i++)
            {
                bool second = i >= splitIdx;
                int localIdx = second ? (i - splitIdx) : i;
                int noteLen = second ? (samples - splitIdx) : splitIdx;
                float localT = localIdx / (float)SampleRate;
                float env = Mathf.Exp(-localT * 22f);
                if (localIdx < ramp) env *= localIdx / (float)ramp;
                if (localIdx >= noteLen - ramp) env *= Math.Max(0, noteLen - 1 - localIdx) / (float)ramp;

                float f1 = second ? 783.99f : 587.33f;
                float f2 = second ? 1174.66f : 880.0f;
                p1 += (f1 / SampleRate) * Mathf.PI * 2f;
                p2 += (f2 / SampleRate) * Mathf.PI * 2f;
                buf[i] = Mathf.Clamp((Mathf.Sin(p1) * 0.32f + Mathf.Sin(p2) * 0.18f) * env, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateAirlockHissSamples()
        {
            int samples = (int)(SampleRate * 1.15f);
            float[] buf = new float[samples];
            uint rng = 0x12345678u;
            float lp = 0f;
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float attack = Mathf.Clamp01(t / 0.08f);
                float decay = Mathf.Exp(-Mathf.Max(0f, t - 0.12f) * 3.2f);
                float clunk = Mathf.Exp(-t * 28f) * Mathf.Sin(t * 92f * Mathf.PI * 2f) * 0.65f;

                rng = rng * 1664525u + 1013904223u;
                float white = ((rng & 0xFFFFu) / 32768f) - 1f;
                lp = lp * 0.78f + white * 0.22f;

                buf[i] = Mathf.Clamp(clunk + lp * attack * decay * 0.55f, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateDockClampSamples()
        {
            int samples = (int)(SampleRate * 0.95f);
            float[] buf = new float[samples];
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float thud = Mathf.Exp(-t * 18f) * Mathf.Sin(t * 68f * Mathf.PI * 2f) * 0.7f;
                float chimeEnv = t > 0.10f ? Mathf.Clamp01((t - 0.10f) / 0.005f) * Mathf.Exp(-(t - 0.10f) * 4.5f) : 0f;
                float chord = (Mathf.Sin(t * 523.25f * Mathf.PI * 2f) + Mathf.Sin(t * 659.25f * Mathf.PI * 2f) + Mathf.Sin(t * 783.99f * Mathf.PI * 2f)) * 0.18f * chimeEnv;
                buf[i] = Mathf.Clamp(thud + chord, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateRadarPingSamples()
        {
            int samples = (int)(SampleRate * 0.34f);
            float[] buf = new float[samples];
            float phase = 0f;
            int ramp = Math.Max(1, (int)(SampleRate * 0.002f));
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float env = Mathf.Exp(-t * 11.5f);
                if (i < ramp) env *= i / (float)ramp;
                float sweepHz = 1320f + 220f * Mathf.Exp(-t * 18f);
                phase += (sweepHz / SampleRate) * Mathf.PI * 2f;
                buf[i] = Mathf.Sin(phase) * env * 0.38f;
            }
            return buf;
        }

        public static float[] GenerateLaserPulseSamples()
        {
            int samples = (int)(SampleRate * 0.19f);
            float[] buf = new float[samples];
            float phase = 0f;
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float freq = 1480f * Mathf.Exp(-t * 7.5f) + 220f;
                phase += (freq / SampleRate) * Mathf.PI * 2f;
                float env = Mathf.Exp(-t * 14f);
                buf[i] = Mathf.Sin(phase) * env * 0.42f;
            }
            return buf;
        }

        public static float[] GenerateRailgunCrackSamples()
        {
            int samples = (int)(SampleRate * 0.28f);
            float[] buf = new float[samples];
            uint rng = 0x89ABCDEFu;
            float bp = 0f;
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                rng = rng * 1664525u + 1013904223u;
                float white = ((rng & 0xFFFFu) / 32768f) - 1f;
                bp = bp * 0.62f + white * 0.38f;
                float crack = bp * Mathf.Exp(-t * 34f) * 0.72f;
                float sub = Mathf.Sin(t * 96f * Mathf.PI * 2f) * Mathf.Exp(-t * 16f) * 0.48f;
                buf[i] = Mathf.Clamp(crack + sub, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateTorpedoLaunchSamples()
        {
            int samples = (int)(SampleRate * 0.46f);
            float[] buf = new float[samples];
            uint rng = 0x5150CAFEu;
            float lp = 0f;
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                rng = rng * 1664525u + 1013904223u;
                float white = ((rng & 0xFFFFu) / 32768f) - 1f;
                lp = lp * 0.85f + white * 0.15f;
                float thump = Mathf.Sin(t * 74f * Mathf.PI * 2f) * Mathf.Exp(-t * 15f) * 0.55f;
                float booster = lp * Mathf.Clamp01(t / 0.06f) * Mathf.Exp(-Mathf.Max(0f, t - 0.08f) * 5.2f) * 0.45f;
                buf[i] = Mathf.Clamp(thump + booster, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateRedAlertKlaxonSamples()
        {
            int samples = (int)(SampleRate * 0.62f);
            float[] buf = new float[samples];
            float phase = 0f;
            int split = samples / 2;
            int ramp = Math.Max(1, (int)(SampleRate * 0.008f));
            for (int i = 0; i < samples; i++)
            {
                bool second = i >= split;
                int local = second ? i - split : i;
                int len = second ? samples - split : split;
                float u = local / (float)Math.Max(1, len);
                float freq = Mathf.Lerp(540f, 820f, u);
                phase += (freq / SampleRate) * Mathf.PI * 2f;
                float env = 1f;
                if (local < ramp) env *= local / (float)ramp;
                if (local >= len - ramp) env *= Math.Max(0, len - 1 - local) / (float)ramp;
                float tone = Mathf.Sin(phase) * 0.42f + Mathf.Sin(phase * 2f) * 0.14f;
                buf[i] = Mathf.Clamp(tone * env, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateTargetLockToneSamples()
        {
            int samples = (int)(SampleRate * 0.18f);
            float[] buf = new float[samples];
            float phase = 0f;
            int split = (int)(SampleRate * 0.075f);
            int ramp = Math.Max(1, (int)(SampleRate * 0.003f));
            for (int i = 0; i < samples; i++)
            {
                bool second = i >= split;
                int local = second ? i - split : i;
                int len = second ? samples - split : split;
                float localT = local / (float)SampleRate;
                float freq = second ? 1318.51f : 880.0f;
                phase += (freq / SampleRate) * Mathf.PI * 2f;
                float env = Mathf.Exp(-localT * (second ? 18f : 26f));
                if (local < ramp) env *= local / (float)ramp;
                if (local >= len - ramp) env *= Math.Max(0, len - 1 - local) / (float)ramp;
                buf[i] = Mathf.Clamp(Mathf.Sin(phase) * env * 0.44f, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateKillConfirmChordSamples()
        {
            int samples = (int)(SampleRate * 0.34f);
            float[] buf = new float[samples];
            float p1 = 0f, p2 = 0f, p3 = 0f;
            int split = (int)(SampleRate * 0.08f);
            int ramp = Math.Max(1, (int)(SampleRate * 0.004f));
            for (int i = 0; i < samples; i++)
            {
                bool tail = i >= split;
                int local = tail ? i - split : i;
                int len = tail ? samples - split : split;
                float localT = local / (float)SampleRate;
                float f1 = tail ? 523.25f : 392.00f;
                float f2 = tail ? 659.25f : 523.25f;
                float f3 = tail ? 1046.50f : 783.99f;
                p1 += (f1 / SampleRate) * Mathf.PI * 2f;
                p2 += (f2 / SampleRate) * Mathf.PI * 2f;
                p3 += (f3 / SampleRate) * Mathf.PI * 2f;
                float env = Mathf.Exp(-localT * (tail ? 9.5f : 20f));
                if (local < ramp) env *= local / (float)ramp;
                if (local >= len - ramp) env *= Math.Max(0, len - 1 - local) / (float)ramp;
                float sig = (Mathf.Sin(p1) * 0.28f + Mathf.Sin(p2) * 0.24f + Mathf.Sin(p3) * 0.16f) * env;
                buf[i] = Mathf.Clamp(sig, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateHitConfirmTickSamples()
        {
            int samples = (int)(SampleRate * 0.11f);
            float[] buf = new float[samples];
            float p1 = 0f, p2 = 0f;
            int ramp = Math.Max(1, (int)(SampleRate * 0.002f));
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float f1 = Mathf.Lerp(1568.0f, 1760.0f, Mathf.Clamp01(t / 0.05f));
                float f2 = f1 * 2.0f;
                p1 += (f1 / SampleRate) * Mathf.PI * 2f;
                p2 += (f2 / SampleRate) * Mathf.PI * 2f;
                float env = Mathf.Exp(-t * 34f);
                if (i < ramp) env *= i / (float)ramp;
                float snap = i < (int)(SampleRate * 0.006f) ? ((i % 3 == 0 ? 0.16f : -0.14f) * (1f - i / (SampleRate * 0.006f))) : 0f;
                float sig = (Mathf.Sin(p1) * 0.42f + Mathf.Sin(p2) * 0.18f) * env + snap;
                buf[i] = Mathf.Clamp(sig, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateArmorImpactThudSamples()
        {
            int samples = (int)(SampleRate * 0.22f);
            float[] buf = new float[samples];
            float phase = 0f;
            uint rng = 0x5EED1234u;
            int ramp = Math.Max(1, (int)(SampleRate * 0.003f));
            float lpNoise = 0f;
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float u = i / (float)Math.Max(1, samples - 1);
                float freq = Mathf.Lerp(92f, 38f, u);
                phase += (freq / SampleRate) * Mathf.PI * 2f;
                rng = rng * 1664525u + 1013904223u;
                float white = ((rng >> 9) / (float)(1 << 23)) * 2f - 1f;
                lpNoise = lpNoise * 0.78f + white * 0.22f;
                float bodyEnv = Mathf.Exp(-t * 14f);
                float crunchEnv = Mathf.Exp(-t * 28f);
                if (i < ramp)
                {
                    float r = i / (float)ramp;
                    bodyEnv *= r;
                    crunchEnv *= r;
                }
                float sig = Mathf.Sin(phase) * bodyEnv * 0.62f + lpNoise * crunchEnv * 0.38f;
                buf[i] = Mathf.Clamp(sig, -1f, 1f);
            }
            return buf;
        }
    }
}
