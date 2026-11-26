using System;
using System.Collections.Generic;
using TacSim.Character;
using TacSim.Core;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum TacSimDisplayModeOption
    {
        Windowed = 0,
        BorderlessFullscreen = 1,
        ExclusiveFullscreen = 2
    }

    public enum TacSimOptionsTab
    {
        Display = 0,
        Audio = 1,
        CameraAndHud = 2
    }

    public enum TacSimHudDensityPreset
    {
        Standard = 0,
        Compact = 1,
        Cinematic = 2
    }

    public readonly struct TacSimResolutionChoice : IEquatable<TacSimResolutionChoice>
    {
        public readonly int Width;
        public readonly int Height;

        public TacSimResolutionChoice(int width, int height)
        {
            Width = width;
            Height = height;
        }

        public string Label => $"{Width} x {Height}";

        public bool Equals(TacSimResolutionChoice other) => Width == other.Width && Height == other.Height;
        public override bool Equals(object obj) => obj is TacSimResolutionChoice other && Equals(other);
        public override int GetHashCode() => (Width * 397) ^ Height;
    }

    /// <summary>
    /// DPI-aware 3-tab Settings Console (`DISPLAY` | `AUDIO` | `CAMERA &amp; HUD`):
    /// - Display Mode, Resolution, Live UI Scale, and Graphics Quality
    /// - Master / Ambient / SFX Volume Mixer, Mute Toggle &amp; Live SFX Preview Buttons
    /// - Camera Sensitivity, Scroll Zoom Speed, Invert-Y, Field of View &amp; HUD Density (`STANDARD` / `COMPACT` / `CINEMATIC [F9]`)
    /// </summary>
    public sealed class TacSimDisplayOptions : MonoBehaviour
    {
        public const float BaseRefWidth = 1920f;
        public const float BaseRefHeight = 1080f;
        public const float MinUiScale = 0.75f;
        public const float MaxUiScale = 2.00f;

        private const string PrefSaved = "TacSim_DisplayPrefsSaved_v1";
        private const string PrefMode = "TacSim_DisplayMode";
        private const string PrefWidth = "TacSim_ResWidth";
        private const string PrefHeight = "TacSim_ResHeight";
        private const string PrefUiScalePct = "TacSim_UiScalePct";
        private const string PrefQuality = "TacSim_QualityLevel";
        private const string PrefHudDensity = "TacSim_HudDensityPreset";

        private static readonly int[] UiScalePresetsPct = { 85, 100, 125, 150, 175, 200 };
        private static readonly int[] VolumeStepPresets = { 0, 25, 50, 75, 100 };
        private static readonly int[] SensPresets = { 50, 75, 100, 125, 150, 200 };
        private static readonly int[] FovPresets = { 55, 62, 72, 80 };

        public static float CurrentUiScale { get; private set; } = 1.0f;
        public static TacSimHudDensityPreset CurrentHudDensity { get; private set; } = TacSimHudDensityPreset.Standard;

        public event Action<TacSimHudDensityPreset> OnHudDensityChanged;

        public bool IsOpen => _modalOverlay != null && _modalOverlay.activeSelf;
        public TacSimOptionsTab ActiveTab { get; private set; } = TacSimOptionsTab.Display;

        private Font _uiFont;
        private TacSimAudioSynthesizer _audio;
        private TacSimCameraRig _cameraRig;

        private GameObject _modalOverlay;
        private readonly Image[] _tabButtonImages = new Image[3];
        private readonly GameObject[] _tabBodies = new GameObject[3];

        // Tab 0: Display
        private Text _resolutionValueText;
        private Text _qualityValueText;
        private Text _liveStatusText;
        private readonly List<Image> _modeButtonImages = new List<Image>();
        private readonly List<Image> _scaleButtonImages = new List<Image>();
        private List<TacSimResolutionChoice> _resolutionChoices = new List<TacSimResolutionChoice>();

        // Tab 1: Audio Mixer
        private Text _muteBtnLabel;
        private Image _muteBtnImage;
        private Text _masterVolLabel;
        private Text _ambientVolLabel;
        private Text _sfxVolLabel;
        private readonly List<Image> _masterVolPills = new List<Image>();
        private readonly List<Image> _ambientVolPills = new List<Image>();
        private readonly List<Image> _sfxVolPills = new List<Image>();

        // Tab 2: Camera & HUD
        private Text _sensValueLabel;
        private Text _zoomValueLabel;
        private Text _invertYBtnLabel;
        private Image _invertYBtnImage;
        private readonly List<Image> _sensPills = new List<Image>();
        private readonly List<Image> _zoomPills = new List<Image>();
        private readonly List<Image> _fovPills = new List<Image>();
        private readonly List<Image> _hudDensityPills = new List<Image>();

        private TacSimDisplayModeOption _selectedMode = TacSimDisplayModeOption.Windowed;
        private int _selectedResolutionIndex;
        private int _selectedUiScalePct = 100;
        private int _selectedQualityIndex;
        private int _lastWindowedWidth = 1600;
        private int _lastWindowedHeight = 900;
        private TacSimHudDensityPreset _lastNonCinematicDensity = TacSimHudDensityPreset.Standard;

        public static float ComputeWindowsDpiScale(float screenDpi)
        {
            if (float.IsNaN(screenDpi) || float.IsInfinity(screenDpi) || screenDpi <= 48f)
                return 1.0f;
            float raw = screenDpi / 96f;
            float snapped = Mathf.Round(raw * 4f) / 4f; // snap to 25% Windows scaling increments
            return Mathf.Clamp(snapped, 1.0f, 2.5f);
        }

        public static TacSimResolutionChoice ComputeDefaultWindowSize(int monitorWidth, int monitorHeight, float dpiScale)
        {
            int monW = monitorWidth > 0 ? monitorWidth : 1920;
            int monH = monitorHeight > 0 ? monitorHeight : 1080;
            float scale = Mathf.Clamp(dpiScale, 1.0f, 2.5f);

            float targetW = Mathf.Max(1600f, 1440f * scale);
            float targetH = targetW * (9f / 16f);

            float maxW = monW * 0.88f;
            float maxH = monH * 0.86f;
            if (targetW > maxW)
            {
                float ratio = maxW / targetW;
                targetW *= ratio;
                targetH *= ratio;
            }
            if (targetH > maxH)
            {
                float ratio = maxH / targetH;
                targetW *= ratio;
                targetH *= ratio;
            }

            int finalW = Mathf.Clamp(Mathf.RoundToInt(targetW / 16f) * 16, Math.Min(1280, monW), monW);
            int finalH = Mathf.Clamp(Mathf.RoundToInt(targetH / 4f) * 4, Math.Min(720, monH), monH);
            return new TacSimResolutionChoice(finalW, finalH);
        }

        public static int ComputeRecommendedUiScalePct(float dpiScale, int screenWidth, int screenHeight)
        {
            float scale = Mathf.Clamp(dpiScale, 1.0f, 2.5f);
            float effective = scale;
            if (effective <= 1.05f && (screenHeight >= 1800 || screenWidth >= 3200))
                effective = 1.25f;
            else if (effective > 1.5f)
                effective = 1.5f;

            int best = 100;
            float bestDiff = float.MaxValue;
            int targetPct = Mathf.RoundToInt(effective * 100f);
            for (int i = 0; i < UiScalePresetsPct.Length; i++)
            {
                float d = Mathf.Abs(UiScalePresetsPct[i] - targetPct);
                if (d < bestDiff)
                {
                    bestDiff = d;
                    best = UiScalePresetsPct[i];
                }
            }
            return best;
        }

        public static Vector2 ComputeScaledReferenceResolution(float uiScale)
        {
            float clamped = Mathf.Clamp(uiScale, MinUiScale, MaxUiScale);
            return new Vector2(BaseRefWidth / clamped, BaseRefHeight / clamped);
        }

        public static List<TacSimResolutionChoice> BuildResolutionChoices(
            Resolution[] hardwareResolutions,
            int monitorWidth,
            int monitorHeight)
        {
            int maxW = monitorWidth > 0 ? monitorWidth : 3840;
            int maxH = monitorHeight > 0 ? monitorHeight : 2160;
            var seen = new HashSet<TacSimResolutionChoice>();
            var list = new List<TacSimResolutionChoice>();

            void TryAdd(int w, int h)
            {
                if (w < 1024 || h < 600) return;
                if (w > maxW || h > maxH) return;
                var c = new TacSimResolutionChoice(w, h);
                if (seen.Add(c)) list.Add(c);
            }

            if (hardwareResolutions != null)
            {
                for (int i = 0; i < hardwareResolutions.Length; i++)
                    TryAdd(hardwareResolutions[i].width, hardwareResolutions[i].height);
            }

            TryAdd(maxW, maxH);
            TryAdd(3840, 2160);
            TryAdd(3440, 1440);
            TryAdd(2560, 1440);
            TryAdd(2560, 1080);
            TryAdd(2240, 1260);
            TryAdd(1920, 1200);
            TryAdd(1920, 1080);
            TryAdd(1600, 900);
            TryAdd(1280, 720);

            if (list.Count == 0)
                list.Add(new TacSimResolutionChoice(1600, 900));

            list.Sort((a, b) =>
            {
                int cmpW = b.Width.CompareTo(a.Width);
                return cmpW != 0 ? cmpW : b.Height.CompareTo(a.Height);
            });
            return list;
        }

        public static void ApplyUiScaleToAllCanvasScalers(float uiScale)
        {
            CurrentUiScale = Mathf.Clamp(uiScale, MinUiScale, MaxUiScale);
            Vector2 refRes = ComputeScaledReferenceResolution(CurrentUiScale);
            var scalers = FindObjectsByType<CanvasScaler>(FindObjectsSortMode.None);
            if (scalers == null) return;
            for (int i = 0; i < scalers.Length; i++)
            {
                var sc = scalers[i];
                if (sc == null) continue;
                sc.uiScaleMode = CanvasScaler.ScaleMode.ScaleWithScreenSize;
                sc.referenceResolution = refRes;
                sc.matchWidthOrHeight = 0.5f;
            }
        }

        public void BindSubsystems(TacSimAudioSynthesizer audio, TacSimCameraRig cameraRig)
        {
            _audio = audio;
            _cameraRig = cameraRig;
            RefreshVisualState();
        }

        public void BuildModal(Transform canvasRoot, Font uiFont)
        {
            _uiFont = uiFont;
            if (_modalOverlay != null)
            {
                if (Application.isPlaying) Destroy(_modalOverlay);
                else DestroyImmediate(_modalOverlay);
                _modalOverlay = null;
            }
            _modeButtonImages.Clear();
            _scaleButtonImages.Clear();
            _masterVolPills.Clear();
            _ambientVolPills.Clear();
            _sfxVolPills.Clear();
            _sensPills.Clear();
            _zoomPills.Clear();
            _fovPills.Clear();
            _hudDensityPills.Clear();

            RefreshAvailableResolutions();
            LoadStateWithoutApplying();

            _modalOverlay = CreatePanel(
                canvasRoot,
                "DisplayOptionsOverlay",
                Vector2.zero,
                Vector2.one,
                Vector2.zero,
                Vector2.zero,
                new Color(0.01f, 0.03f, 0.06f, 0.84f));

            var card = CreatePanel(
                _modalOverlay.transform,
                "DisplayOptionsCard",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-360f, -278f),
                new Vector2(360f, 278f),
                new Color(0.06f, 0.10f, 0.16f, 0.985f));

            CreatePanel(card.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.32f, 0.88f, 1f, 0.95f));

            var title = CreateText(card.transform, "SETTINGS — DISPLAY, AUDIO & CAMERA", 19, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.90f, 1f));
            SetRect(title.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -46f), new Vector2(-132f, -10f));

            var closeTop = CreateButton(card.transform, "CLOSE [ESC]", new Color(0.45f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeTop.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-124f, -44f), new Vector2(-16f, -12f));

            // 3 Settings Category Tabs
            var tabStrip = CreatePanel(card.transform, "OptionsTabStrip", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -84f), new Vector2(-22f, -52f), Color.clear);
            var tabHlg = tabStrip.AddComponent<HorizontalLayoutGroup>();
            tabHlg.spacing = 8f;
            tabHlg.childControlWidth = true;
            tabHlg.childControlHeight = true;
            tabHlg.childForceExpandWidth = true;

            string[] tabLabels = { "DISPLAY & SCALE", "AUDIO MIXER", "CAMERA & HUD" };
            for (int t = 0; t < 3; t++)
            {
                var capturedTab = (TacSimOptionsTab)t;
                var tb = CreateButton(tabStrip.transform, tabLabels[t], new Color(0.14f, 0.24f, 0.34f), () => SelectTab(capturedTab));
                _tabButtonImages[t] = tb.GetComponent<Image>();
            }

            // Tab bodies area (`y = 66 .. -92`)
            _tabBodies[0] = CreatePanel(card.transform, "DisplayTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(0f, 64f), new Vector2(0f, -90f), Color.clear);
            _tabBodies[1] = CreatePanel(card.transform, "AudioTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(0f, 64f), new Vector2(0f, -90f), Color.clear);
            _tabBodies[2] = CreatePanel(card.transform, "CameraHudTabBody", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(0f, 64f), new Vector2(0f, -90f), Color.clear);

            BuildDisplayTab(_tabBodies[0].transform);
            BuildAudioTab(_tabBodies[1].transform);
            BuildCameraAndHudTab(_tabBodies[2].transform);

            // Shared bottom action row
            var autoBtn = CreateButton(card.transform, "AUTO-DETECT DPI", new Color(0.22f, 0.30f, 0.42f), ResetToAutoDpiDefaults);
            SetRect(autoBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0.48f, 0f), new Vector2(22f, 16f), new Vector2(-6f, 56f));

            var applyBtn = CreateButton(card.transform, "APPLY & SAVE", new Color(0.14f, 0.54f, 0.38f), ApplyAndSaveSelections);
            SetRect(applyBtn.GetComponent<RectTransform>(), new Vector2(0.52f, 0f), new Vector2(1f, 0f), new Vector2(6f, 16f), new Vector2(-22f, 56f));

            SelectTab(TacSimOptionsTab.Display);
            RefreshVisualState();
            _modalOverlay.SetActive(false);
        }

        private void BuildDisplayTab(Transform body)
        {
            // 1. Display Mode
            var modeLbl = CreateText(body, "DISPLAY MODE  (F11 TOGGLES FULLSCREEN)", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(modeLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -24f), new Vector2(-22f, -2f));

            string[] modeLabels = { "Windowed", "Borderless Fullscreen", "Fullscreen" };
            for (int i = 0; i < modeLabels.Length; i++)
            {
                int captured = i;
                float xMin = 22f + i * 228f;
                var btn = CreateButton(body, modeLabels[i], new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    _selectedMode = (TacSimDisplayModeOption)captured;
                    if (_selectedMode != TacSimDisplayModeOption.Windowed)
                        SelectNearestResolution(GetMonitorWidth(), GetMonitorHeight());
                    RefreshVisualState();
                });
                SetRect(btn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xMin, -62f), new Vector2(xMin + 220f, -28f));
                _modeButtonImages.Add(btn.GetComponent<Image>());
            }

            // 2. Resolution selector + quick presets
            var resLbl = CreateText(body, "WINDOW / SCREEN RESOLUTION", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(resLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -92f), new Vector2(-22f, -70f));

            var prevResBtn = CreateButton(body, "<", new Color(0.16f, 0.28f, 0.40f), () => StepResolution(+1));
            SetRect(prevResBtn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(22f, -130f), new Vector2(70f, -96f));

            var resBox = CreatePanel(body, "ResolutionBox", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(78f, -130f), new Vector2(-78f, -96f), new Color(0.09f, 0.14f, 0.22f, 0.96f));
            _resolutionValueText = CreateText(resBox.transform, "1920 x 1080", 14, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(_resolutionValueText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            var nextResBtn = CreateButton(body, ">", new Color(0.16f, 0.28f, 0.40f), () => StepResolution(-1));
            SetRect(nextResBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-70f, -130f), new Vector2(-22f, -96f));

            var presetRow = new (string label, int w, int h)[]
            {
                ("1600x900", 1600, 900),
                ("1920x1080", 1920, 1080),
                ("2560x1440", 2560, 1440),
                ("Native", GetMonitorWidth(), GetMonitorHeight())
            };
            for (int i = 0; i < presetRow.Length; i++)
            {
                var p = presetRow[i];
                float xMin = 22f + i * 170f;
                var pBtn = CreateButton(body, p.label, new Color(0.12f, 0.22f, 0.30f), () =>
                {
                    SelectNearestResolution(p.w, p.h);
                    RefreshVisualState();
                });
                SetRect(pBtn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xMin, -166f), new Vector2(xMin + 162f, -136f));
            }

            // 3. UI Scale (live preview!)
            var scaleLbl = CreateText(body, "UI / HUD SCALE  (LIVE PREVIEW — HELPS ON HIGH-DPI / ULTRAWIDE)", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(scaleLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -196f), new Vector2(-22f, -174f));

            for (int i = 0; i < UiScalePresetsPct.Length; i++)
            {
                int pct = UiScalePresetsPct[i];
                float xMin = 22f + i * 114f;
                var sBtn = CreateButton(body, $"{pct}%", new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    _selectedUiScalePct = pct;
                    ApplyUiScaleToAllCanvasScalers(_selectedUiScalePct / 100f);
                    RefreshVisualState();
                });
                SetRect(sBtn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xMin, -234f), new Vector2(xMin + 106f, -200f));
                _scaleButtonImages.Add(sBtn.GetComponent<Image>());
            }

            // 4. Graphics Quality
            var qualLbl = CreateText(body, "GRAPHICS QUALITY PRESET", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(qualLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -264f), new Vector2(-22f, -242f));

            var prevQ = CreateButton(body, "<", new Color(0.16f, 0.28f, 0.40f), () => StepQuality(-1));
            SetRect(prevQ.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(22f, -302f), new Vector2(70f, -268f));

            var qBox = CreatePanel(body, "QualityBox", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(78f, -302f), new Vector2(-78f, -268f), new Color(0.09f, 0.14f, 0.22f, 0.96f));
            _qualityValueText = CreateText(qBox.transform, "High", 14, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(_qualityValueText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            var nextQ = CreateButton(body, ">", new Color(0.16f, 0.28f, 0.40f), () => StepQuality(+1));
            SetRect(nextQ.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-70f, -302f), new Vector2(-22f, -268f));

            _liveStatusText = CreateText(body, "", 12, FontStyle.Normal, TextAnchor.MiddleCenter, new Color(0.68f, 0.84f, 0.94f));
            SetRect(_liveStatusText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(22f, 10f), new Vector2(-22f, 38f));
        }

        private void BuildAudioTab(Transform body)
        {
            var hdr = CreateText(body, "PROCEDURAL AUDIO MIXER  (CTRL+M QUICK MUTE)", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(0.62f, 1f), new Vector2(22f, -30f), new Vector2(0f, -4f));

            var muteBtn = CreateButton(body, "AUDIO: ACTIVE", new Color(0.14f, 0.46f, 0.36f), () =>
            {
                _audio?.ToggleMuted();
                RefreshVisualState();
            });
            SetRect(muteBtn.GetComponent<RectTransform>(), new Vector2(0.64f, 1f), new Vector2(1f, 1f), new Vector2(0f, -34f), new Vector2(-22f, -2f));
            _muteBtnLabel = muteBtn.GetComponentInChildren<Text>();
            _muteBtnImage = muteBtn.GetComponent<Image>();

            BuildVolumeRow(body, "MASTER VOLUME", -46f, out _masterVolLabel, _masterVolPills,
                pct => { _audio?.SetMasterVolumePct(pct); RefreshVisualState(); },
                delta => { if (_audio != null) _audio.SetMasterVolumePct(_audio.MasterVolumePct + delta); RefreshVisualState(); });

            BuildVolumeRow(body, "AMBIENT SOUNDSCAPE (STATION / SHIP / SPACE)", -124f, out _ambientVolLabel, _ambientVolPills,
                pct => { _audio?.SetAmbientVolumePct(pct); RefreshVisualState(); },
                delta => { if (_audio != null) _audio.SetAmbientVolumePct(_audio.AmbientVolumePct + delta); RefreshVisualState(); });

            BuildVolumeRow(body, "TACTICAL & UI SFX", -202f, out _sfxVolLabel, _sfxVolPills,
                pct => { _audio?.SetSfxVolumePct(pct); RefreshVisualState(); },
                delta => { if (_audio != null) _audio.SetSfxVolumePct(_audio.SfxVolumePct + delta); RefreshVisualState(); });

            var testLbl = CreateText(body, "AUDIO CHANNEL PREVIEW", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(testLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -306f), new Vector2(-22f, -284f));

            var testRow = CreatePanel(body, "AudioTestRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -346f), new Vector2(-22f, -310f), Color.clear);
            var hlg = testRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f; hlg.childControlWidth = true; hlg.childControlHeight = true; hlg.childForceExpandWidth = true;
            CreateButton(testRow.transform, "TEST CHIME", new Color(0.14f, 0.34f, 0.46f), () => _audio?.PlayMarketChime());
            CreateButton(testRow.transform, "TEST LASER", new Color(0.46f, 0.20f, 0.24f), () => _audio?.PlayLaserPulse());
            CreateButton(testRow.transform, "TEST RED ALERT", new Color(0.62f, 0.20f, 0.16f), () => _audio?.PlayRedAlertKlaxon());
            CreateButton(testRow.transform, "RESET MIXER", new Color(0.24f, 0.28f, 0.36f), () =>
            {
                _audio?.ResetMixerDefaults();
                RefreshVisualState();
            });
        }

        private void BuildVolumeRow(
            Transform parent,
            string title,
            float yTop,
            out Text valueLabel,
            List<Image> pillImages,
            Action<int> onPreset,
            Action<int> onStep)
        {
            var lbl = CreateText(parent, title, 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(lbl.rectTransform, new Vector2(0f, 1f), new Vector2(0.76f, 1f), new Vector2(22f, yTop - 22f), new Vector2(0f, yTop));

            valueLabel = CreateText(parent, "100%", 12, FontStyle.Bold, TextAnchor.MiddleRight, new Color(0.50f, 0.96f, 0.82f));
            SetRect(valueLabel.rectTransform, new Vector2(0.76f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(-22f, yTop));

            var row = CreatePanel(parent, "VolRow_" + title, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, yTop - 62f), new Vector2(-22f, yTop - 26f), Color.clear);
            var hlg = row.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 6f; hlg.childControlWidth = true; hlg.childControlHeight = true; hlg.childForceExpandWidth = true;

            CreateButton(row.transform, "-10%", new Color(0.18f, 0.26f, 0.36f), () => onStep(-10));
            for (int i = 0; i < VolumeStepPresets.Length; i++)
            {
                int pct = VolumeStepPresets[i];
                var b = CreateButton(row.transform, $"{pct}%", new Color(0.14f, 0.24f, 0.34f), () => onPreset(pct));
                pillImages.Add(b.GetComponent<Image>());
            }
            CreateButton(row.transform, "+10%", new Color(0.18f, 0.26f, 0.36f), () => onStep(+10));
        }

        private void BuildCameraAndHudTab(Transform body)
        {
            // 1. Orbit Mouse Sensitivity
            var sLbl = CreateText(body, "CAMERA ORBIT SENSITIVITY", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(sLbl.rectTransform, new Vector2(0f, 1f), new Vector2(0.72f, 1f), new Vector2(22f, -24f), new Vector2(0f, -2f));
            _sensValueLabel = CreateText(body, "100%", 12, FontStyle.Bold, TextAnchor.MiddleRight, new Color(0.50f, 0.96f, 0.82f));
            SetRect(_sensValueLabel.rectTransform, new Vector2(0.72f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(-22f, -2f));

            var sensRow = CreatePanel(body, "SensRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -62f), new Vector2(-22f, -28f), Color.clear);
            var sHlg = sensRow.AddComponent<HorizontalLayoutGroup>();
            sHlg.spacing = 6f; sHlg.childControlWidth = true; sHlg.childControlHeight = true; sHlg.childForceExpandWidth = true;
            for (int i = 0; i < SensPresets.Length; i++)
            {
                int p = SensPresets[i];
                var b = CreateButton(sensRow.transform, $"{p}%", new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    if (_cameraRig != null)
                        _cameraRig.ApplyCameraPreferences(p, _cameraRig.ZoomSpeedPct, _cameraRig.InvertY, _cameraRig.FieldOfViewDeg);
                    RefreshVisualState();
                });
                _sensPills.Add(b.GetComponent<Image>());
            }

            // 2. Scroll Zoom Speed
            var zLbl = CreateText(body, "CAMERA SCROLL ZOOM SPEED", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(zLbl.rectTransform, new Vector2(0f, 1f), new Vector2(0.72f, 1f), new Vector2(22f, -94f), new Vector2(0f, -72f));
            _zoomValueLabel = CreateText(body, "100%", 12, FontStyle.Bold, TextAnchor.MiddleRight, new Color(0.50f, 0.96f, 0.82f));
            SetRect(_zoomValueLabel.rectTransform, new Vector2(0.72f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(-22f, -72f));

            var zoomRow = CreatePanel(body, "ZoomRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -132f), new Vector2(-22f, -98f), Color.clear);
            var zHlg = zoomRow.AddComponent<HorizontalLayoutGroup>();
            zHlg.spacing = 6f; zHlg.childControlWidth = true; zHlg.childControlHeight = true; zHlg.childForceExpandWidth = true;
            for (int i = 0; i < SensPresets.Length; i++)
            {
                int p = SensPresets[i];
                var b = CreateButton(zoomRow.transform, $"{p}%", new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    if (_cameraRig != null)
                        _cameraRig.ApplyCameraPreferences(_cameraRig.OrbitSensitivityPct, p, _cameraRig.InvertY, _cameraRig.FieldOfViewDeg);
                    RefreshVisualState();
                });
                _zoomPills.Add(b.GetComponent<Image>());
            }

            // 3. Field of View & Invert-Y
            var fovLbl = CreateText(body, "FIELD OF VIEW (FOV) & INVERT MOUSE Y", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(fovLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -164f), new Vector2(-22f, -142f));

            var fovRow = CreatePanel(body, "FovAndInvertRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -204f), new Vector2(-22f, -168f), Color.clear);
            var fHlg = fovRow.AddComponent<HorizontalLayoutGroup>();
            fHlg.spacing = 6f; fHlg.childControlWidth = true; fHlg.childControlHeight = true; fHlg.childForceExpandWidth = true;
            for (int i = 0; i < FovPresets.Length; i++)
            {
                int fov = FovPresets[i];
                var b = CreateButton(fovRow.transform, $"FOV {fov}°", new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    if (_cameraRig != null)
                        _cameraRig.ApplyCameraPreferences(_cameraRig.OrbitSensitivityPct, _cameraRig.ZoomSpeedPct, _cameraRig.InvertY, fov);
                    RefreshVisualState();
                });
                _fovPills.Add(b.GetComponent<Image>());
            }
            var invBtn = CreateButton(fovRow.transform, "INVERT Y: OFF", new Color(0.16f, 0.26f, 0.38f), () =>
            {
                if (_cameraRig != null)
                    _cameraRig.ApplyCameraPreferences(_cameraRig.OrbitSensitivityPct, _cameraRig.ZoomSpeedPct, !_cameraRig.InvertY, _cameraRig.FieldOfViewDeg);
                RefreshVisualState();
            });
            _invertYBtnLabel = invBtn.GetComponentInChildren<Text>();
            _invertYBtnImage = invBtn.GetComponent<Image>();

            // 4. HUD Layout Density Preset (`F9` Quick Toggle!)
            var hudLbl = CreateText(body, "HUD LAYOUT DENSITY PRESET  (PRESS [F9] ANYTIME FOR CINEMATIC VIEW)", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(hudLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -242f), new Vector2(-22f, -220f));

            var hudRow = CreatePanel(body, "HudDensityRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(22f, -286f), new Vector2(-22f, -246f), Color.clear);
            var dHlg = hudRow.AddComponent<HorizontalLayoutGroup>();
            dHlg.spacing = 8f; dHlg.childControlWidth = true; dHlg.childControlHeight = true; dHlg.childForceExpandWidth = true;
            string[] densityLabels = { "STANDARD (FULL MMORPG)", "COMPACT (WIDER VIEW)", "CINEMATIC MINIMAL [F9]" };
            for (int i = 0; i < densityLabels.Length; i++)
            {
                var preset = (TacSimHudDensityPreset)i;
                var b = CreateButton(hudRow.transform, densityLabels[i], new Color(0.14f, 0.24f, 0.34f), () => SetHudDensityPreset(preset));
                _hudDensityPills.Add(b.GetComponent<Image>());
            }

            var resetCamBtn = CreateButton(body, "RESET CAMERA & HUD DEFAULTS", new Color(0.24f, 0.28f, 0.38f), () =>
            {
                _cameraRig?.ApplyCameraPreferences(100, 100, false, 62);
                SetHudDensityPreset(TacSimHudDensityPreset.Standard);
                RefreshVisualState();
            });
            SetRect(resetCamBtn.GetComponent<RectTransform>(), new Vector2(0.25f, 1f), new Vector2(0.75f, 1f), new Vector2(0f, -340f), new Vector2(0f, -304f));
        }

        public void SelectTab(TacSimOptionsTab tab)
        {
            ActiveTab = tab;
            for (int i = 0; i < 3; i++)
            {
                bool active = i == (int)tab;
                if (_tabBodies[i] != null) _tabBodies[i].SetActive(active);
                if (_tabButtonImages[i] != null)
                {
                    _tabButtonImages[i].color = active
                        ? new Color(0.20f, 0.56f, 0.78f, 1f)
                        : new Color(0.12f, 0.20f, 0.30f, 0.92f);
                }
            }
            RefreshVisualState();
        }

        public void SetHudDensityPreset(TacSimHudDensityPreset preset, bool save = true)
        {
            CurrentHudDensity = preset;
            if (preset != TacSimHudDensityPreset.Cinematic)
                _lastNonCinematicDensity = preset;
            if (save && Application.isPlaying)
            {
                PlayerPrefs.SetInt(PrefHudDensity, (int)preset);
                PlayerPrefs.Save();
            }
            OnHudDensityChanged?.Invoke(preset);
            RefreshVisualState();
        }

        public TacSimHudDensityPreset ToggleCinematicHudMode()
        {
            TacSimHudDensityPreset next = CurrentHudDensity == TacSimHudDensityPreset.Cinematic
                ? _lastNonCinematicDensity
                : TacSimHudDensityPreset.Cinematic;
            SetHudDensityPreset(next, save: false);
            return next;
        }

        public void SetOpen(bool open)
        {
            if (_modalOverlay == null) return;
            if (open)
            {
                RefreshAvailableResolutions();
                RefreshVisualState();
                _modalOverlay.transform.SetAsLastSibling();
            }
            _modalOverlay.SetActive(open);
        }

        public void ApplyStartupPreferences()
        {
            RefreshAvailableResolutions();

            if (!Application.isPlaying)
                return;

            int savedDensity = PlayerPrefs.GetInt(PrefHudDensity, (int)TacSimHudDensityPreset.Standard);
            if (savedDensity >= 0 && savedDensity <= 2)
                SetHudDensityPreset((TacSimHudDensityPreset)savedDensity, save: false);

            bool hasSaved = PlayerPrefs.GetInt(PrefSaved, 0) == 1;
            if (hasSaved)
            {
                LoadStateWithoutApplying();
                ApplyCurrentSettingsToEngine();
                return;
            }

            float dpiScale = ComputeWindowsDpiScale(Screen.dpi);
            int monW = GetMonitorWidth();
            int monH = GetMonitorHeight();
            TacSimResolutionChoice defSize = ComputeDefaultWindowSize(monW, monH, dpiScale);
            _selectedMode = TacSimDisplayModeOption.Windowed;
            _lastWindowedWidth = defSize.Width;
            _lastWindowedHeight = defSize.Height;
            SelectOrInsertResolution(defSize.Width, defSize.Height);
            _selectedUiScalePct = ComputeRecommendedUiScalePct(dpiScale, defSize.Width, defSize.Height);
            _selectedQualityIndex = QualitySettings.GetQualityLevel();

            ApplyCurrentSettingsToEngine();
            RefreshVisualState();
        }

        public void ToggleBorderlessFullscreen()
        {
            if (!Application.isPlaying)
                return;

            if (Screen.fullScreenMode == FullScreenMode.Windowed)
            {
                _lastWindowedWidth = Math.Max(1024, Screen.width);
                _lastWindowedHeight = Math.Max(600, Screen.height);
                _selectedMode = TacSimDisplayModeOption.BorderlessFullscreen;
                int monW = GetMonitorWidth();
                int monH = GetMonitorHeight();
                SelectOrInsertResolution(monW, monH);
                Screen.SetResolution(monW, monH, FullScreenMode.FullScreenWindow);
            }
            else
            {
                _selectedMode = TacSimDisplayModeOption.Windowed;
                int w = _lastWindowedWidth >= 1024 ? _lastWindowedWidth : 1600;
                int h = _lastWindowedHeight >= 600 ? _lastWindowedHeight : 900;
                SelectOrInsertResolution(w, h);
                Screen.SetResolution(w, h, FullScreenMode.Windowed);
            }
            SavePreferences();
            RefreshVisualState();
        }

        public void ApplyAndSaveSelections()
        {
            ApplyCurrentSettingsToEngine();
            SavePreferences();
            RefreshVisualState();
        }

        public void ResetToAutoDpiDefaults()
        {
            float dpiScale = ComputeWindowsDpiScale(Screen.dpi);
            int monW = GetMonitorWidth();
            int monH = GetMonitorHeight();
            TacSimResolutionChoice defSize = ComputeDefaultWindowSize(monW, monH, dpiScale);
            _selectedMode = TacSimDisplayModeOption.Windowed;
            SelectOrInsertResolution(defSize.Width, defSize.Height);
            _selectedUiScalePct = ComputeRecommendedUiScalePct(dpiScale, defSize.Width, defSize.Height);
            ApplyAndSaveSelections();
        }

        private void ApplyCurrentSettingsToEngine()
        {
            ApplyUiScaleToAllCanvasScalers(_selectedUiScalePct / 100f);

            if (!Application.isPlaying)
                return;

            TacSimResolutionChoice res = GetSelectedResolution();
            FullScreenMode mode = _selectedMode == TacSimDisplayModeOption.BorderlessFullscreen
                ? FullScreenMode.FullScreenWindow
                : (_selectedMode == TacSimDisplayModeOption.ExclusiveFullscreen
                    ? FullScreenMode.ExclusiveFullScreen
                    : FullScreenMode.Windowed);

            if (mode == FullScreenMode.Windowed)
            {
                _lastWindowedWidth = res.Width;
                _lastWindowedHeight = res.Height;
            }
            Screen.SetResolution(res.Width, res.Height, mode);

            string[] names = QualitySettings.names;
            if (names != null && names.Length > 0)
            {
                int clampedQ = Mathf.Clamp(_selectedQualityIndex, 0, names.Length - 1);
                if (QualitySettings.GetQualityLevel() != clampedQ)
                    QualitySettings.SetQualityLevel(clampedQ, applyExpensiveChanges: true);
            }
        }

        private void LoadStateWithoutApplying()
        {
            if (Application.isPlaying && PlayerPrefs.GetInt(PrefSaved, 0) == 1)
            {
                int modeInt = PlayerPrefs.GetInt(PrefMode, 0);
                _selectedMode = (modeInt >= 0 && modeInt <= 2) ? (TacSimDisplayModeOption)modeInt : TacSimDisplayModeOption.Windowed;
                int w = PlayerPrefs.GetInt(PrefWidth, Screen.width);
                int h = PlayerPrefs.GetInt(PrefHeight, Screen.height);
                SelectOrInsertResolution(w, h);
                _selectedUiScalePct = Mathf.Clamp(PlayerPrefs.GetInt(PrefUiScalePct, 100), 85, 200);
                _selectedQualityIndex = PlayerPrefs.GetInt(PrefQuality, QualitySettings.GetQualityLevel());
            }
            else
            {
                _selectedMode = Screen.fullScreenMode == FullScreenMode.FullScreenWindow
                    ? TacSimDisplayModeOption.BorderlessFullscreen
                    : (Screen.fullScreenMode == FullScreenMode.ExclusiveFullScreen
                        ? TacSimDisplayModeOption.ExclusiveFullscreen
                        : TacSimDisplayModeOption.Windowed);
                SelectNearestResolution(Screen.width, Screen.height);
                _selectedUiScalePct = Mathf.RoundToInt(CurrentUiScale * 100f);
                _selectedQualityIndex = QualitySettings.GetQualityLevel();
            }
        }

        private void SavePreferences()
        {
            if (!Application.isPlaying) return;
            TacSimResolutionChoice res = GetSelectedResolution();
            PlayerPrefs.SetInt(PrefSaved, 1);
            PlayerPrefs.SetInt(PrefMode, (int)_selectedMode);
            PlayerPrefs.SetInt(PrefWidth, res.Width);
            PlayerPrefs.SetInt(PrefHeight, res.Height);
            PlayerPrefs.SetInt(PrefUiScalePct, _selectedUiScalePct);
            PlayerPrefs.SetInt(PrefQuality, _selectedQualityIndex);
            PlayerPrefs.SetInt(PrefHudDensity, (int)CurrentHudDensity);
            PlayerPrefs.Save();
        }

        private void RefreshAvailableResolutions()
        {
            _resolutionChoices = BuildResolutionChoices(Screen.resolutions, GetMonitorWidth(), GetMonitorHeight());
            if (_selectedResolutionIndex < 0 || _selectedResolutionIndex >= _resolutionChoices.Count)
                _selectedResolutionIndex = 0;
        }

        private void StepResolution(int delta)
        {
            if (_resolutionChoices.Count == 0) return;
            _selectedResolutionIndex = (_selectedResolutionIndex + delta + _resolutionChoices.Count) % _resolutionChoices.Count;
            RefreshVisualState();
        }

        private void StepQuality(int delta)
        {
            string[] names = QualitySettings.names;
            int count = names != null && names.Length > 0 ? names.Length : 1;
            _selectedQualityIndex = (_selectedQualityIndex + delta + count) % count;
            RefreshVisualState();
        }

        private void SelectOrInsertResolution(int width, int height)
        {
            if (width < 1024 || height < 600)
            {
                SelectNearestResolution(1600, 900);
                return;
            }
            var target = new TacSimResolutionChoice(width, height);
            int idx = _resolutionChoices.IndexOf(target);
            if (idx >= 0)
            {
                _selectedResolutionIndex = idx;
                return;
            }
            _resolutionChoices.Add(target);
            _resolutionChoices.Sort((a, b) =>
            {
                int cmpW = b.Width.CompareTo(a.Width);
                return cmpW != 0 ? cmpW : b.Height.CompareTo(a.Height);
            });
            _selectedResolutionIndex = Math.Max(0, _resolutionChoices.IndexOf(target));
        }

        private void SelectNearestResolution(int width, int height)
        {
            if (_resolutionChoices.Count == 0) return;
            int bestIdx = 0;
            long bestScore = long.MaxValue;
            for (int i = 0; i < _resolutionChoices.Count; i++)
            {
                long dw = _resolutionChoices[i].Width - width;
                long dh = _resolutionChoices[i].Height - height;
                long score = dw * dw + dh * dh;
                if (score < bestScore)
                {
                    bestScore = score;
                    bestIdx = i;
                }
            }
            _selectedResolutionIndex = bestIdx;
        }

        private TacSimResolutionChoice GetSelectedResolution()
        {
            if (_resolutionChoices.Count == 0)
                return new TacSimResolutionChoice(1600, 900);
            int idx = Mathf.Clamp(_selectedResolutionIndex, 0, _resolutionChoices.Count - 1);
            return _resolutionChoices[idx];
        }

        private void RefreshVisualState()
        {
            Color activeCol = new Color(0.22f, 0.65f, 0.88f, 1f);
            Color idleCol = new Color(0.14f, 0.22f, 0.30f, 0.92f);

            for (int i = 0; i < _modeButtonImages.Count; i++)
            {
                if (_modeButtonImages[i] != null)
                    _modeButtonImages[i].color = (i == (int)_selectedMode) ? activeCol : idleCol;
            }
            for (int i = 0; i < _scaleButtonImages.Count && i < UiScalePresetsPct.Length; i++)
            {
                if (_scaleButtonImages[i] != null)
                    _scaleButtonImages[i].color = (UiScalePresetsPct[i] == _selectedUiScalePct) ? activeCol : idleCol;
            }
            if (_resolutionValueText != null)
                _resolutionValueText.text = GetSelectedResolution().Label;
            if (_qualityValueText != null)
            {
                string[] names = QualitySettings.names;
                _qualityValueText.text = (names != null && names.Length > 0)
                    ? names[Mathf.Clamp(_selectedQualityIndex, 0, names.Length - 1)]
                    : "Default";
            }
            if (_liveStatusText != null)
            {
                int dpiPct = Mathf.RoundToInt(ComputeWindowsDpiScale(Screen.dpi) * 100f);
                _liveStatusText.text =
                    $"Active: {Screen.width}x{Screen.height} | Monitor: {GetMonitorWidth()}x{GetMonitorHeight()} | OS DPI: {dpiPct}% | UI Scale: {_selectedUiScalePct}%";
            }

            // Audio Mixer UI state
            bool muted = _audio != null && _audio.IsMuted;
            int masterPct = _audio != null ? _audio.MasterVolumePct : 100;
            int ambPct = _audio != null ? _audio.AmbientVolumePct : 80;
            int sfxPct = _audio != null ? _audio.SfxVolumePct : 90;

            if (_muteBtnLabel != null) _muteBtnLabel.text = muted ? "AUDIO: MUTED" : "AUDIO: ACTIVE";
            if (_muteBtnImage != null) _muteBtnImage.color = muted ? new Color(0.60f, 0.20f, 0.20f, 1f) : new Color(0.14f, 0.48f, 0.36f, 1f);
            if (_masterVolLabel != null) _masterVolLabel.text = $"{masterPct}%";
            if (_ambientVolLabel != null) _ambientVolLabel.text = $"{ambPct}%";
            if (_sfxVolLabel != null) _sfxVolLabel.text = $"{sfxPct}%";

            HighlightIntPills(_masterVolPills, VolumeStepPresets, masterPct, activeCol, idleCol);
            HighlightIntPills(_ambientVolPills, VolumeStepPresets, ambPct, activeCol, idleCol);
            HighlightIntPills(_sfxVolPills, VolumeStepPresets, sfxPct, activeCol, idleCol);

            // Camera & HUD UI state
            int sensPct = _cameraRig != null ? _cameraRig.OrbitSensitivityPct : 100;
            int zoomPct = _cameraRig != null ? _cameraRig.ZoomSpeedPct : 100;
            int fovDeg = _cameraRig != null ? _cameraRig.FieldOfViewDeg : 62;
            bool invY = _cameraRig != null && _cameraRig.InvertY;

            if (_sensValueLabel != null) _sensValueLabel.text = $"{sensPct}%";
            if (_zoomValueLabel != null) _zoomValueLabel.text = $"{zoomPct}%";
            if (_invertYBtnLabel != null) _invertYBtnLabel.text = invY ? "INVERT Y: ON" : "INVERT Y: OFF";
            if (_invertYBtnImage != null) _invertYBtnImage.color = invY ? new Color(0.24f, 0.54f, 0.72f, 1f) : idleCol;

            HighlightIntPills(_sensPills, SensPresets, sensPct, activeCol, idleCol);
            HighlightIntPills(_zoomPills, SensPresets, zoomPct, activeCol, idleCol);
            HighlightIntPills(_fovPills, FovPresets, fovDeg, activeCol, idleCol);

            for (int i = 0; i < _hudDensityPills.Count; i++)
            {
                if (_hudDensityPills[i] != null)
                    _hudDensityPills[i].color = (i == (int)CurrentHudDensity) ? activeCol : idleCol;
            }
        }

        private static void HighlightIntPills(List<Image> pills, int[] presets, int currentVal, Color activeCol, Color idleCol)
        {
            for (int i = 0; i < pills.Count && i < presets.Length; i++)
            {
                if (pills[i] != null)
                    pills[i].color = presets[i] == currentVal ? activeCol : idleCol;
            }
        }

        private static int GetMonitorWidth()
        {
            return Display.main != null && Display.main.systemWidth > 0
                ? Display.main.systemWidth
                : Math.Max(1920, Screen.width);
        }

        private static int GetMonitorHeight()
        {
            return Display.main != null && Display.main.systemHeight > 0
                ? Display.main.systemHeight
                : Math.Max(1080, Screen.height);
        }

        private GameObject CreatePanel(Transform parent, string name, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), anchorMin, anchorMax, offsetMin, offsetMax);
            return go;
        }

        private Text CreateText(Transform parent, string content, int size, FontStyle style, TextAnchor align, Color color)
        {
            var go = new GameObject("Text", typeof(RectTransform), typeof(Text));
            go.transform.SetParent(parent, false);
            var t = go.GetComponent<Text>();
            t.font = _uiFont;
            t.text = content;
            t.fontSize = size;
            t.fontStyle = style;
            t.alignment = align;
            t.color = color;
            t.supportRichText = false;
            t.raycastTarget = false;
            return t;
        }

        private Button CreateButton(Transform parent, string label, Color bg, Action onClick)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = Color.white;
            img.raycastTarget = true;

            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.colors = new ColorBlock
            {
                normalColor = bg,
                highlightedColor = new Color(Mathf.Clamp01(bg.r * 1.38f + 0.06f), Mathf.Clamp01(bg.g * 1.38f + 0.06f), Mathf.Clamp01(bg.b * 1.38f + 0.06f), 1f),
                pressedColor = new Color(Mathf.Clamp01(bg.r * 0.65f), Mathf.Clamp01(bg.g * 0.65f), Mathf.Clamp01(bg.b * 0.65f), 1f),
                selectedColor = new Color(Mathf.Clamp01(bg.r * 1.25f + 0.04f), Mathf.Clamp01(bg.g * 1.25f + 0.04f), Mathf.Clamp01(bg.b * 1.25f + 0.04f), 1f),
                disabledColor = new Color(0.2f, 0.22f, 0.25f, 0.6f),
                colorMultiplier = 1f,
                fadeDuration = 0.06f
            };
            btn.onClick.AddListener(() => onClick?.Invoke());

            var txt = CreateText(go.transform, label, 12, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            return btn;
        }

        private static void SetRect(RectTransform rt, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax)
        {
            rt.anchorMin = anchorMin;
            rt.anchorMax = anchorMax;
            rt.offsetMin = offsetMin;
            rt.offsetMax = offsetMax;
        }
    }
}
