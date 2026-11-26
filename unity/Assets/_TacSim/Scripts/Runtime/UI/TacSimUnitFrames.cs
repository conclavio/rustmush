using System;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// WoW / EverQuest-style top-left Player/Ship Unit Frame and Target Frame.
    /// Automatically adapts resource bars and target actions across all 5 TacSim stages
    /// (Station Walk, Ship Interior, Orbital Space, Planetary Surface, OOC Arena).
    /// </summary>
    public sealed class TacSimUnitFrames : MonoBehaviour
    {
        public bool IsTargetVisible => _targetRoot != null && _targetRoot.activeSelf;
        public string PlayerTitleText => _playerTitle != null ? _playerTitle.text : string.Empty;
        public string TargetTitleText => _targetTitle != null ? _targetTitle.text : string.Empty;

        private Font _font;
        private GameObject _root;

        // Player / Ship Unit Frame
        private Image _playerCrestBg;
        private Text _playerCrestText;
        private Text _playerTitle;
        private Text _playerSubtitle;
        private Image _bar1Fill;
        private Text _bar1Text;
        private Image _bar2Fill;
        private Text _bar2Text;
        private Text _playerFooter;

        // Target Frame
        private GameObject _targetRoot;
        private Image _targetCrestBg;
        private Text _targetCrestText;
        private Text _targetTitle;
        private Text _targetSubtitle;
        private Image _targetBarFill;
        private Text _targetBarText;
        private Button _targetAct1Btn;
        private Text _targetAct1Label;
        private Action _targetAct1Callback;
        private Button _targetAct2Btn;
        private Text _targetAct2Label;
        private Action _targetAct2Callback;

        public void BuildFrames(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            if (_root != null)
            {
                if (Application.isPlaying) Destroy(_root);
                else DestroyImmediate(_root);
                _root = null;
            }

            _root = CreatePanel(hudRoot, "MmorpgUnitFrames", new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(16f, -158f), new Vector2(526f, -72f), Color.clear);

            // 1. Player / Hull Unit Frame (250 x 86)
            var pCard = CreatePanel(_root.transform, "PlayerUnitFrame", new Vector2(0f, 0f), new Vector2(0f, 1f), Vector2.zero, new Vector2(250f, 0f), new Color(0.03f, 0.06f, 0.11f, 0.90f));

            var pCrest = CreatePanel(pCard.transform, "StageCrest", new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(6f, -34f), new Vector2(52f, -6f), new Color(0.14f, 0.42f, 0.56f, 0.96f));
            _playerCrestBg = pCrest.GetComponent<Image>();
            _playerCrestText = CreateText(pCrest.transform, "STN", 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(_playerCrestText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            _playerTitle = CreateText(pCard.transform, "PILOT", 13, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.90f, 0.96f, 1f));
            SetRect(_playerTitle.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(58f, -22f), new Vector2(-8f, -4f));

            _playerSubtitle = CreateText(pCard.transform, "Ceres Freeport", 10, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.62f, 0.80f, 0.92f));
            SetRect(_playerSubtitle.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(58f, -36f), new Vector2(-8f, -21f));

            BuildResourceBar(pCard.transform, "Bar1", -52f, -38f, new Color(0.18f, 0.66f, 0.88f, 0.95f), out _bar1Fill, out _bar1Text);
            BuildResourceBar(pCard.transform, "Bar2", -68f, -54f, new Color(0.86f, 0.54f, 0.16f, 0.95f), out _bar2Fill, out _bar2Text);

            _playerFooter = CreateText(pCard.transform, "CREDITS: —", 10, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.96f, 0.72f));
            SetRect(_playerFooter.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(8f, 2f), new Vector2(-8f, 17f));

            // 2. Target Unit Frame (252 x 86, docked cleanly to the right of Player Frame)
            _targetRoot = CreatePanel(_root.transform, "TargetUnitFrame", new Vector2(0f, 0f), new Vector2(0f, 1f), new Vector2(258f, 0f), new Vector2(510f, 0f), new Color(0.05f, 0.06f, 0.10f, 0.92f));

            var tCrest = CreatePanel(_targetRoot.transform, "TargetCrest", new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(6f, -34f), new Vector2(56f, -6f), new Color(0.54f, 0.24f, 0.16f, 0.96f));
            _targetCrestBg = tCrest.GetComponent<Image>();
            _targetCrestText = CreateText(tCrest.transform, "TGT", 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(_targetCrestText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            _targetTitle = CreateText(_targetRoot.transform, "NO TARGET", 13, FontStyle.Bold, TextAnchor.UpperLeft, new Color(1f, 0.92f, 0.78f));
            SetRect(_targetTitle.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(62f, -22f), new Vector2(-8f, -4f));

            _targetSubtitle = CreateText(_targetRoot.transform, "", 10, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.76f, 0.86f, 0.94f));
            SetRect(_targetSubtitle.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(62f, -36f), new Vector2(-8f, -21f));

            BuildResourceBar(_targetRoot.transform, "TargetBar", -54f, -39f, new Color(0.84f, 0.34f, 0.26f, 0.95f), out _targetBarFill, out _targetBarText);

            _targetAct1Btn = CreateActionPill(_targetRoot.transform, "TgtAct1", new Vector2(0f, 0f), new Vector2(0.5f, 0f), new Vector2(6f, 4f), new Vector2(-3f, 26f), new Color(0.14f, 0.44f, 0.56f), out _targetAct1Label, () => _targetAct1Callback?.Invoke());
            _targetAct2Btn = CreateActionPill(_targetRoot.transform, "TgtAct2", new Vector2(0.5f, 0f), new Vector2(1f, 0f), new Vector2(3f, 4f), new Vector2(-6f, 26f), new Color(0.46f, 0.22f, 0.18f), out _targetAct2Label, () => _targetAct2Callback?.Invoke());

            _targetRoot.SetActive(false);
        }

        public void ResetSessionState()
        {
            ClearTarget();
        }

        public void UpdatePlayerFrame(
            string crestTag,
            Color crestColor,
            string title,
            string subtitle,
            float bar1Frac,
            string bar1Label,
            Color bar1Color,
            float bar2Frac,
            string bar2Label,
            Color bar2Color,
            string footerText)
        {
            if (_playerCrestText != null) _playerCrestText.text = crestTag ?? "STN";
            if (_playerCrestBg != null) _playerCrestBg.color = crestColor;
            if (_playerTitle != null) _playerTitle.text = title ?? string.Empty;
            if (_playerSubtitle != null) _playerSubtitle.text = subtitle ?? string.Empty;
            SetBar(_bar1Fill, _bar1Text, bar1Frac, bar1Label, bar1Color);
            SetBar(_bar2Fill, _bar2Text, bar2Frac, bar2Label, bar2Color);
            if (_playerFooter != null) _playerFooter.text = footerText ?? string.Empty;
        }

        public void SetTarget(
            string crestTag,
            Color crestColor,
            string title,
            string subtitle,
            float barFrac,
            string barLabel,
            Color barColor,
            string action1Label = null,
            Action action1 = null,
            string action2Label = null,
            Action action2 = null)
        {
            if (_targetRoot == null) return;
            _targetRoot.SetActive(true);
            if (_targetCrestText != null) _targetCrestText.text = crestTag ?? "TGT";
            if (_targetCrestBg != null) _targetCrestBg.color = crestColor;
            if (_targetTitle != null) _targetTitle.text = title ?? string.Empty;
            if (_targetSubtitle != null) _targetSubtitle.text = subtitle ?? string.Empty;
            SetBar(_targetBarFill, _targetBarText, barFrac, barLabel, barColor);

            _targetAct1Callback = action1;
            if (_targetAct1Btn != null)
            {
                bool show1 = !string.IsNullOrEmpty(action1Label) && action1 != null;
                _targetAct1Btn.gameObject.SetActive(show1);
                if (show1 && _targetAct1Label != null) _targetAct1Label.text = action1Label;
            }

            _targetAct2Callback = action2;
            if (_targetAct2Btn != null)
            {
                bool show2 = !string.IsNullOrEmpty(action2Label) && action2 != null;
                _targetAct2Btn.gameObject.SetActive(show2);
                if (show2 && _targetAct2Label != null) _targetAct2Label.text = action2Label;
            }
        }

        public void ClearTarget()
        {
            _targetAct1Callback = null;
            _targetAct2Callback = null;
            if (_targetRoot != null && _targetRoot.activeSelf)
                _targetRoot.SetActive(false);
        }

        private static void SetBar(Image fill, Text label, float frac, string text, Color col)
        {
            if (fill != null)
            {
                float c = Mathf.Clamp01(frac);
                fill.rectTransform.anchorMax = new Vector2(c, 1f);
                fill.color = col;
            }
            if (label != null)
                label.text = text ?? string.Empty;
        }

        private void BuildResourceBar(Transform parent, string name, float yMin, float yMax, Color fillCol, out Image fillImg, out Text barText)
        {
            var track = CreatePanel(parent, name, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(8f, yMin), new Vector2(-8f, yMax), new Color(0.08f, 0.12f, 0.18f, 0.96f));
            var fillGo = CreatePanel(track.transform, "Fill", Vector2.zero, new Vector2(0.5f, 1f), Vector2.zero, Vector2.zero, fillCol);
            fillImg = fillGo.GetComponent<Image>();
            barText = CreateText(track.transform, "", 10, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(barText.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 0f), new Vector2(-4f, 0f));
        }

        private Button CreateActionPill(Transform parent, string name, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax, Color bg, out Text labelOut, Action onClick)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = Color.white;
            img.raycastTarget = true;
            SetRect(go.GetComponent<RectTransform>(), aMin, aMax, oMin, oMax);

            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.colors = new ColorBlock
            {
                normalColor = bg,
                highlightedColor = new Color(Mathf.Clamp01(bg.r * 1.35f + 0.06f), Mathf.Clamp01(bg.g * 1.35f + 0.06f), Mathf.Clamp01(bg.b * 1.35f + 0.06f), 1f),
                pressedColor = new Color(Mathf.Clamp01(bg.r * 0.68f), Mathf.Clamp01(bg.g * 0.68f), Mathf.Clamp01(bg.b * 0.68f), 1f),
                selectedColor = bg,
                disabledColor = new Color(0.2f, 0.22f, 0.25f, 0.6f),
                colorMultiplier = 1f,
                fadeDuration = 0.05f
            };
            btn.onClick.AddListener(() => onClick?.Invoke());

            labelOut = CreateText(go.transform, "", 10, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(labelOut.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 0f), new Vector2(-4f, 0f));
            return btn;
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
            t.font = _font;
            t.text = content;
            t.fontSize = size;
            t.fontStyle = style;
            t.alignment = align;
            t.color = color;
            t.supportRichText = false;
            t.raycastTarget = false;
            return t;
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
