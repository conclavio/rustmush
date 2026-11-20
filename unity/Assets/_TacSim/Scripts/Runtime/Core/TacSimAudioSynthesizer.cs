using System;
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
        public const int SampleRate = 22050;

        public bool IsMuted { get; private set; }

        private AudioSource _ambientLoop;
        private AudioSource _sfxSource;

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

        private void Awake()
        {
            InitializeAudioGraph();
        }

        public void InitializeAudioGraph()
        {
            if (_ambientLoop != null) return;

            IsMuted = PlayerPrefs.GetInt(MutePlayerPrefKey, 0) == 1;

            _ambientLoop = gameObject.AddComponent<AudioSource>();
            _ambientLoop.loop = true;
            _ambientLoop.spatialBlend = 0f;
            _ambientLoop.volume = IsMuted ? 0f : 0.085f;
            _ambientLoop.playOnAwake = false;

            _sfxSource = gameObject.AddComponent<AudioSource>();
            _sfxSource.loop = false;
            _sfxSource.spatialBlend = 0f;
            _sfxSource.volume = 0.22f;
            _sfxSource.playOnAwake = false;

            _stationHumClip = BuildDroneLoop("StationHum", baseHz: 55f, fifthHz: 82.5f, airNoise: 0.018f);
            _shipHumClip = BuildDroneLoop("ShipReactorHum", baseHz: 64f, fifthHz: 128f, airNoise: 0.026f);
            _spaceHumClip = BuildDroneLoop("SpaceCanopyHum", baseHz: 42f, fifthHz: 63f, airNoise: 0.008f);
            _uiClickClip = BuildClipFromSamples("UiChime", GenerateUiChimeSamples());
            _marketChimeClip = BuildClipFromSamples("MarketChime", GenerateMarketChimeSamples());
            _airlockCycleClip = BuildClipFromSamples("AirlockCycle", GenerateAirlockHissSamples());
            _dockClampClip = BuildClipFromSamples("DockHardClamp", GenerateDockClampSamples());
            _radarPingClip = BuildClipFromSamples("RadarScanPing", GenerateRadarPingSamples());
            _laserPulseClip = BuildClipFromSamples("LaserPulse", GenerateLaserPulseSamples());
            _railgunCrackClip = BuildClipFromSamples("RailgunCrack", GenerateRailgunCrackSamples());
            _torpedoLaunchClip = BuildClipFromSamples("TorpedoLaunch", GenerateTorpedoLaunchSamples());

            SetSoundscape(AmbientSoundscape.StationRing);
        }

        public void SetMuted(bool muted)
        {
            IsMuted = muted;
            PlayerPrefs.SetInt(MutePlayerPrefKey, muted ? 1 : 0);
            PlayerPrefs.Save();
            if (_ambientLoop != null)
                _ambientLoop.volume = muted ? 0f : 0.085f;
        }

        public bool ToggleMuted()
        {
            SetMuted(!IsMuted);
            return IsMuted;
        }

        public void SetSoundscape(AmbientSoundscape mode)
        {
            if (_ambientLoop == null) return;
            AudioClip desired = mode == AmbientSoundscape.ShipInterior ? _shipHumClip
                              : mode == AmbientSoundscape.OrbitalSpace ? _spaceHumClip
                              : _stationHumClip;
            if (_ambientLoop.clip == desired && _ambientLoop.isPlaying)
                return;

            _ambientLoop.clip = desired;
            _ambientLoop.volume = IsMuted ? 0f : 0.085f;
            if (desired != null)
                _ambientLoop.Play();
        }

        public void PlayUiClick() => PlayOneShotIfUnmuted(_uiClickClip, 0.35f);
        public void PlayMarketChime() => PlayOneShotIfUnmuted(_marketChimeClip, 0.50f);
        public void PlayAirlockCycle() => PlayOneShotIfUnmuted(_airlockCycleClip, 0.75f);
        public void PlayDockHardClamp() => PlayOneShotIfUnmuted(_dockClampClip, 0.90f);
        public void PlayRadarScanPing() => PlayOneShotIfUnmuted(_radarPingClip, 0.55f);
        public void PlayLaserPulse() => PlayOneShotIfUnmuted(_laserPulseClip, 0.55f);
        public void PlayRailgunCrack() => PlayOneShotIfUnmuted(_railgunCrackClip, 0.70f);
        public void PlayTorpedoLaunch() => PlayOneShotIfUnmuted(_torpedoLaunchClip, 0.65f);

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
            else if (low.StartsWith("+tac/scan", StringComparison.Ordinal) || low == "scan")
                PlayRadarScanPing();
            else if (low.StartsWith("+market/accept", StringComparison.Ordinal) || low.StartsWith("+market/quote", StringComparison.Ordinal))
                PlayMarketChime();
            else if (low.StartsWith("fire ", StringComparison.Ordinal))
                PlayRailgunCrack();
        }

        private void PlayOneShotIfUnmuted(AudioClip clip, float scale)
        {
            if (!IsMuted && _sfxSource != null && clip != null)
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
            float[] buf = new float[samples];
            uint rng = 0x9E3779B9u;
            float lp = 0f;

            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float s1 = Mathf.Sin(t * baseHz * Mathf.PI * 2f) * 0.45f;
                float s2 = Mathf.Sin(t * fifthHz * Mathf.PI * 2f) * 0.22f;
                float lfo = 0.85f + 0.15f * Mathf.Sin(t * 0.25f * Mathf.PI * 2f);

                rng = rng * 1664525u + 1013904223u;
                float white = ((rng & 0xFFFFu) / 32768f) - 1f;
                lp = lp * 0.94f + white * 0.06f;

                buf[i] = (s1 + s2) * lfo + lp * airNoise;
            }

            return BuildClipFromSamples(name, buf);
        }

        public static float[] GenerateUiChimeSamples()
        {
            int samples = (int)(SampleRate * 0.085f);
            float[] buf = new float[samples];
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float env = Mathf.Exp(-t * 42f);
                float freq = i < samples / 2 ? 880f : 1174.66f;
                buf[i] = Mathf.Sin(t * freq * Mathf.PI * 2f) * env * 0.45f;
            }
            return buf;
        }

        public static float[] GenerateMarketChimeSamples()
        {
            int samples = (int)(SampleRate * 0.24f);
            float[] buf = new float[samples];
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                bool second = t >= 0.09f;
                float localT = second ? t - 0.09f : t;
                float env = Mathf.Exp(-localT * 22f);
                float f1 = second ? 783.99f : 587.33f;
                float f2 = second ? 1174.66f : 880.0f;
                buf[i] = (Mathf.Sin(t * f1 * Mathf.PI * 2f) * 0.32f + Mathf.Sin(t * f2 * Mathf.PI * 2f) * 0.18f) * env;
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
                float chimeEnv = t > 0.10f ? Mathf.Exp(-(t - 0.10f) * 4.5f) : 0f;
                float chord = (Mathf.Sin(t * 523.25f * Mathf.PI * 2f) + Mathf.Sin(t * 659.25f * Mathf.PI * 2f) + Mathf.Sin(t * 783.99f * Mathf.PI * 2f)) * 0.18f * chimeEnv;
                buf[i] = Mathf.Clamp(thud + chord, -1f, 1f);
            }
            return buf;
        }

        public static float[] GenerateRadarPingSamples()
        {
            int samples = (int)(SampleRate * 0.34f);
            float[] buf = new float[samples];
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float env = Mathf.Exp(-t * 11.5f);
                float sweepHz = 1320f + 220f * Mathf.Exp(-t * 18f);
                buf[i] = Mathf.Sin(t * sweepHz * Mathf.PI * 2f) * env * 0.38f;
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
    }
}
