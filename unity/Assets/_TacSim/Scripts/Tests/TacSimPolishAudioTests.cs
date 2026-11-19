using System;
using NUnit.Framework;
using TacSim.Core;
using UnityEngine;

namespace TacSim.Tests
{
    public sealed class TacSimPolishAudioTests
    {
        private static void AssertValidPcmBuffer(float[] samples, string label)
        {
            Assert.IsNotNull(samples, label);
            Assert.Greater(samples.Length, 1000, label);
            float peak = 0f;
            for (int i = 0; i < samples.Length; i++)
            {
                float s = samples[i];
                Assert.IsFalse(float.IsNaN(s) || float.IsInfinity(s), $"{label} sample #{i} must be finite");
                Assert.GreaterOrEqual(s, -1.0f, label);
                Assert.LessOrEqual(s, 1.0f, label);
                float a = Math.Abs(s);
                if (a > peak) peak = a;
            }
            Assert.Greater(peak, 0.15f, $"{label} must not be silent");
        }

        [Test]
        public void ProceduralAudio_GeneratesBoundedWaveforms_AndSupportsMuteAndCommandRouting()
        {
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateUiChimeSamples(), "UiChime");
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateMarketChimeSamples(), "MarketChime");
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateAirlockHissSamples(), "AirlockHiss");
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateDockClampSamples(), "DockClamp");
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateRadarPingSamples(), "RadarPing");
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateLaserPulseSamples(), "LaserPulse");
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateRailgunCrackSamples(), "RailgunCrack");
            AssertValidPcmBuffer(TacSimAudioSynthesizer.GenerateTorpedoLaunchSamples(), "TorpedoLaunch");

            var go = new GameObject("TestAudioSynth");
            try
            {
                var synth = go.AddComponent<TacSimAudioSynthesizer>();
                synth.InitializeAudioGraph();
                Assert.IsFalse(synth.IsMuted);

                synth.PlayForCommandLine("+tac/beam");
                synth.PlayForCommandLine("+tac/railgun");
                synth.PlayForCommandLine("+tac/torp");
                synth.PlayForCommandLine("+tac/scan");
                synth.PlayForCommandLine("+market/accept 42");

                Assert.IsTrue(synth.ToggleMuted());
                Assert.IsTrue(synth.IsMuted);
                synth.PlayForCommandLine("+tac/beam");
                Assert.IsFalse(synth.ToggleMuted());
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(go);
            }
        }

        [Test]
        public void Remediation_UH10_UM05_AudioDisposesClips_StopsLoopOnMute_AndHasSmoothWaveforms()
        {
            var go = new GameObject("TestAudioLifecycle");
            try
            {
                var synth = go.AddComponent<TacSimAudioSynthesizer>();
                synth.InitializeAudioGraph();
                Assert.AreEqual(11, synth.AllocatedClipCount, "InitializeAudioGraph allocates all 11 procedural AudioClips");

                var sources = go.GetComponents<AudioSource>();
                Assert.GreaterOrEqual(sources.Length, 2);
                AudioSource ambientLoop = sources[0];
                Assert.IsTrue(ambientLoop.isPlaying, "Unmuted ambient loop must be playing");

                synth.SetMuted(true);
                Assert.IsFalse(ambientLoop.isPlaying, "Muting must stop the ambient AudioSource instead of burning CPU at volume 0");
                synth.SetMuted(false);
                Assert.IsTrue(ambientLoop.isPlaying, "Unmuting must resume ambient AudioSource");

                synth.DisposeClips();
                Assert.AreEqual(0, synth.AllocatedClipCount, "DisposeClips/OnDestroy must release all 11 procedural AudioClips");

                // Verify smooth phase-continuous waveforms with no hard jump clicks
                AssertMaxSampleDelta(TacSimAudioSynthesizer.GenerateUiChimeSamples(), 0.28f, "UiChime");
                AssertMaxSampleDelta(TacSimAudioSynthesizer.GenerateMarketChimeSamples(), 0.28f, "MarketChime");
                AssertMaxSampleDelta(TacSimAudioSynthesizer.GenerateRadarPingSamples(), 0.35f, "RadarPing");
            }
            finally
            {
                UnityEngine.Object.DestroyImmediate(go);
            }
        }

        private static void AssertMaxSampleDelta(float[] samples, float maxJump, string label)
        {
            float worst = 0f;
            for (int i = 1; i < samples.Length; i++)
            {
                float d = Math.Abs(samples[i] - samples[i - 1]);
                if (d > worst) worst = d;
            }
            Assert.LessOrEqual(worst, maxJump, $"{label} has phase discontinuity jump {worst:F3} > {maxJump:F3}");
        }
    }
}
