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
    /// - Tactile terminal button chime
    /// - Pneumatic airlock pressure-cycle hiss (+ship/board &amp; +ship/leave)
    /// - 6-DOF docking collar hard-clamp lock chord
    /// </summary>
    public sealed class TacSimAudioSynthesizer : MonoBehaviour
    {
        private AudioSource _ambientLoop;
        private AudioSource _sfxSource;

        private AudioClip _stationHumClip;
        private AudioClip _shipHumClip;
        private AudioClip _spaceHumClip;
        private AudioClip _uiClickClip;
        private AudioClip _airlockCycleClip;
        private AudioClip _dockClampClip;

        private const int SampleRate = 22050;

        private void Awake()
        {
            _ambientLoop = gameObject.AddComponent<AudioSource>();
            _ambientLoop.loop = true;
            _ambientLoop.spatialBlend = 0f;
            _ambientLoop.volume = 0.085f;
            _ambientLoop.playOnAwake = false;

            _sfxSource = gameObject.AddComponent<AudioSource>();
            _sfxSource.loop = false;
            _sfxSource.spatialBlend = 0f;
            _sfxSource.volume = 0.22f;
            _sfxSource.playOnAwake = false;

            _stationHumClip = BuildDroneLoop("StationHum", baseHz: 55f, fifthHz: 82.5f, airNoise: 0.018f);
            _shipHumClip = BuildDroneLoop("ShipReactorHum", baseHz: 64f, fifthHz: 128f, airNoise: 0.026f);
            _spaceHumClip = BuildDroneLoop("SpaceCanopyHum", baseHz: 42f, fifthHz: 63f, airNoise: 0.008f);
            _uiClickClip = BuildUiChimeClip();
            _airlockCycleClip = BuildAirlockHissClip();
            _dockClampClip = BuildDockClampClip();

            SetSoundscape(AmbientSoundscape.StationRing);
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
            if (desired != null)
                _ambientLoop.Play();
        }

        public void PlayUiClick()
        {
            if (_sfxSource != null && _uiClickClip != null)
                _sfxSource.PlayOneShot(_uiClickClip, 0.35f);
        }

        public void PlayAirlockCycle()
        {
            if (_sfxSource != null && _airlockCycleClip != null)
                _sfxSource.PlayOneShot(_airlockCycleClip, 0.75f);
        }

        public void PlayDockHardClamp()
        {
            if (_sfxSource != null && _dockClampClip != null)
                _sfxSource.PlayOneShot(_dockClampClip, 0.9f);
        }

        private static AudioClip BuildDroneLoop(string name, float baseHz, float fifthHz, float airNoise)
        {
            int samples = SampleRate * 4; // seamless 4-second integer-cycle loop
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

            var clip = AudioClip.Create(name, samples, 1, SampleRate, false);
            clip.SetData(buf, 0);
            return clip;
        }

        private static AudioClip BuildUiChimeClip()
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
            var clip = AudioClip.Create("UiChime", samples, 1, SampleRate, false);
            clip.SetData(buf, 0);
            return clip;
        }

        private static AudioClip BuildAirlockHissClip()
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

                buf[i] = clunk + lp * attack * decay * 0.55f;
            }
            var clip = AudioClip.Create("AirlockCycle", samples, 1, SampleRate, false);
            clip.SetData(buf, 0);
            return clip;
        }

        private static AudioClip BuildDockClampClip()
        {
            int samples = (int)(SampleRate * 0.95f);
            float[] buf = new float[samples];
            for (int i = 0; i < samples; i++)
            {
                float t = i / (float)SampleRate;
                float thud = Mathf.Exp(-t * 18f) * Mathf.Sin(t * 68f * Mathf.PI * 2f) * 0.7f;
                float chimeEnv = t > 0.10f ? Mathf.Exp(-(t - 0.10f) * 4.5f) : 0f;
                float chord = (Mathf.Sin(t * 523.25f * Mathf.PI * 2f) + Mathf.Sin(t * 659.25f * Mathf.PI * 2f) + Mathf.Sin(t * 783.99f * Mathf.PI * 2f)) * 0.18f * chimeEnv;
                buf[i] = thud + chord;
            }
            var clip = AudioClip.Create("DockHardClamp", samples, 1, SampleRate, false);
            clip.SetData(buf, 0);
            return clip;
        }
    }
}
