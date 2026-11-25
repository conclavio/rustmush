using System;
using System.Collections.Generic;
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
    /// DPI-aware display, resolution, UI scale, and graphics quality controller + diegetic Options modal.
    /// Respects Windows DPI scaling on high-DPI / ultrawide monitors on first launch and persists user
    /// choices in PlayerPrefs across sessions.
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

        private static readonly int[] UiScalePresetsPct = { 85, 100, 125, 150, 175, 200 };

        public static float CurrentUiScale { get; private set; } = 1.0f;

        public bool IsOpen => _modalOverlay != null && _modalOverlay.activeSelf;

        private Font _uiFont;
        private GameObject _modalOverlay;
        private Text _resolutionValueText;
        private Text _qualityValueText;
        private Text _liveStatusText;

        private readonly List<Image> _modeButtonImages = new List<Image>();
        private readonly List<Image> _scaleButtonImages = new List<Image>();
        private List<TacSimResolutionChoice> _resolutionChoices = new List<TacSimResolutionChoice>();

        private TacSimDisplayModeOption _selectedMode = TacSimDisplayModeOption.Windowed;
        private int _selectedResolutionIndex;
        private int _selectedUiScalePct = 100;
        private int _selectedQualityIndex;
        private int _lastWindowedWidth = 1600;
        private int _lastWindowedHeight = 900;

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

            // Start from a comfortable 1440x810 logical size multiplied by Windows DPI scaling,
            // and never exceed ~88% of either monitor dimension (preserving a clean 16:9 frame).
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
            // On high-resolution displays where Unity reports 96 DPI (common when DPI-unaware or custom scaling),
            // bump UI scale gently when height >= 1440 so text and chat remain crisp.
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

            // Always include helpful windowed/ultrawide presets that fit inside the current monitor.
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
                new Vector2(-340f, -245f),
                new Vector2(340f, 245f),
                new Color(0.06f, 0.10f, 0.16f, 0.98f));

            var title = CreateText(card.transform, "DISPLAY & GRAPHICS OPTIONS", 20, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.90f, 1f));
            SetRect(title.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(24f, -48f), new Vector2(-130f, -12f));

            var closeTop = CreateButton(card.transform, "CLOSE [ESC]", new Color(0.45f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeTop.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-124f, -46f), new Vector2(-18f, -14f));

            // 1. Display Mode
            var modeLbl = CreateText(card.transform, "DISPLAY MODE  (F11 TOGGLES FULLSCREEN)", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(modeLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(24f, -84f), new Vector2(-24f, -60f));

            string[] modeLabels = { "Windowed", "Borderless Fullscreen", "Fullscreen" };
            for (int i = 0; i < modeLabels.Length; i++)
            {
                int captured = i;
                float xMin = 24f + i * 214f;
                var btn = CreateButton(card.transform, modeLabels[i], new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    _selectedMode = (TacSimDisplayModeOption)captured;
                    if (_selectedMode != TacSimDisplayModeOption.Windowed)
                        SelectNearestResolution(GetMonitorWidth(), GetMonitorHeight());
                    RefreshVisualState();
                });
                SetRect(btn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xMin, -124f), new Vector2(xMin + 204f, -88f));
                _modeButtonImages.Add(btn.GetComponent<Image>());
            }

            // 2. Resolution selector + quick presets
            var resLbl = CreateText(card.transform, "WINDOW / SCREEN RESOLUTION", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(resLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(24f, -158f), new Vector2(-24f, -134f));

            var prevResBtn = CreateButton(card.transform, "<", new Color(0.16f, 0.28f, 0.40f), () => StepResolution(+1));
            SetRect(prevResBtn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(24f, -198f), new Vector2(72f, -162f));

            var resBox = CreatePanel(card.transform, "ResolutionBox", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(80f, -198f), new Vector2(-80f, -162f), new Color(0.09f, 0.14f, 0.22f, 0.96f));
            _resolutionValueText = CreateText(resBox.transform, "1920 x 1080", 15, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(_resolutionValueText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            var nextResBtn = CreateButton(card.transform, ">", new Color(0.16f, 0.28f, 0.40f), () => StepResolution(-1));
            SetRect(nextResBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-72f, -198f), new Vector2(-24f, -162f));

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
                float xMin = 24f + i * 160f;
                var pBtn = CreateButton(card.transform, p.label, new Color(0.12f, 0.22f, 0.30f), () =>
                {
                    SelectNearestResolution(p.w, p.h);
                    RefreshVisualState();
                });
                SetRect(pBtn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xMin, -234f), new Vector2(xMin + 150f, -204f));
            }

            // 3. UI Scale (live preview!)
            var scaleLbl = CreateText(card.transform, "UI / HUD SCALE  (LIVE PREVIEW — HELPS ON HIGH-DPI / ULTRAWIDE)", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(scaleLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(24f, -268f), new Vector2(-24f, -244f));

            for (int i = 0; i < UiScalePresetsPct.Length; i++)
            {
                int pct = UiScalePresetsPct[i];
                float xMin = 24f + i * 106f;
                var sBtn = CreateButton(card.transform, $"{pct}%", new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    _selectedUiScalePct = pct;
                    ApplyUiScaleToAllCanvasScalers(_selectedUiScalePct / 100f);
                    RefreshVisualState();
                });
                SetRect(sBtn.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(xMin, -308f), new Vector2(xMin + 98f, -272f));
                _scaleButtonImages.Add(sBtn.GetComponent<Image>());
            }

            // 4. Graphics Quality
            var qualLbl = CreateText(card.transform, "GRAPHICS QUALITY PRESET", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.72f, 0.86f, 0.96f));
            SetRect(qualLbl.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(24f, -342f), new Vector2(-24f, -318f));

            var prevQ = CreateButton(card.transform, "<", new Color(0.16f, 0.28f, 0.40f), () => StepQuality(-1));
            SetRect(prevQ.GetComponent<RectTransform>(), new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(24f, -382f), new Vector2(72f, -346f));

            var qBox = CreatePanel(card.transform, "QualityBox", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(80f, -382f), new Vector2(-80f, -346f), new Color(0.09f, 0.14f, 0.22f, 0.96f));
            _qualityValueText = CreateText(qBox.transform, "High", 15, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(_qualityValueText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            var nextQ = CreateButton(card.transform, ">", new Color(0.16f, 0.28f, 0.40f), () => StepQuality(+1));
            SetRect(nextQ.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-72f, -382f), new Vector2(-24f, -346f));

            // 5. Live diagnostics & action buttons
            _liveStatusText = CreateText(card.transform, "", 12, FontStyle.Normal, TextAnchor.MiddleCenter, new Color(0.68f, 0.84f, 0.94f));
            SetRect(_liveStatusText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(24f, 68f), new Vector2(-24f, 96f));

            var autoBtn = CreateButton(card.transform, "AUTO-DETECT DPI", new Color(0.22f, 0.30f, 0.42f), ResetToAutoDpiDefaults);
            SetRect(autoBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0.48f, 0f), new Vector2(24f, 18f), new Vector2(-6f, 60f));

            var applyBtn = CreateButton(card.transform, "APPLY & SAVE", new Color(0.14f, 0.54f, 0.38f), ApplyAndSaveSelections);
            SetRect(applyBtn.GetComponent<RectTransform>(), new Vector2(0.52f, 0f), new Vector2(1f, 0f), new Vector2(6f, 18f), new Vector2(-24f, 60f));

            RefreshVisualState();
            _modalOverlay.SetActive(false);
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

            bool hasSaved = PlayerPrefs.GetInt(PrefSaved, 0) == 1;
            if (hasSaved)
            {
                LoadStateWithoutApplying();
                ApplyCurrentSettingsToEngine();
                return;
            }

            // First launch: auto-scale window & UI for Windows DPI and monitor dimensions!
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
            for (int i = 0; i < _modeButtonImages.Count; i++)
            {
                if (_modeButtonImages[i] == null) continue;
                _modeButtonImages[i].color = (i == (int)_selectedMode)
                    ? new Color(0.22f, 0.65f, 0.88f, 1f)
                    : new Color(0.14f, 0.22f, 0.30f, 0.92f);
            }

            for (int i = 0; i < _scaleButtonImages.Count && i < UiScalePresetsPct.Length; i++)
            {
                if (_scaleButtonImages[i] == null) continue;
                _scaleButtonImages[i].color = (UiScalePresetsPct[i] == _selectedUiScalePct)
                    ? new Color(0.22f, 0.65f, 0.88f, 1f)
                    : new Color(0.14f, 0.22f, 0.30f, 0.92f);
            }

            if (_resolutionValueText != null)
                _resolutionValueText.text = GetSelectedResolution().Label;

            if (_qualityValueText != null)
            {
                string[] names = QualitySettings.names;
                if (names != null && names.Length > 0)
                    _qualityValueText.text = names[Mathf.Clamp(_selectedQualityIndex, 0, names.Length - 1)];
                else
                    _qualityValueText.text = "Default";
            }

            if (_liveStatusText != null)
            {
                int dpiPct = Mathf.RoundToInt(ComputeWindowsDpiScale(Screen.dpi) * 100f);
                _liveStatusText.text =
                    $"Active: {Screen.width}x{Screen.height} | Monitor: {GetMonitorWidth()}x{GetMonitorHeight()} | OS DPI: {dpiPct}% | UI Scale: {_selectedUiScalePct}%";
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

            var txt = CreateText(go.transform, label, 13, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 2f), new Vector2(-6f, -2f));
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
