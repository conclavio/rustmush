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
    }
}
