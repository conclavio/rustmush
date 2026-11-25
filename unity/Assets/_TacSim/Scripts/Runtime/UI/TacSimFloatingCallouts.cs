using System;
using System.Collections.Generic;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// Pooled WoW / EVE-style floating combat, mining, and navigation callout popups
    /// (`LOCK ACQUIRED`, `LASER SALVO`, `ORE DRILLED`, `OVERHEAT WARNING`, `ARRIVED`).
    /// Zero per-callout GameObject allocations; drifts smoothly upward and fades cleanly.
    /// </summary>
    public sealed class TacSimFloatingCallouts : MonoBehaviour
    {
        private const int PoolSize = 10;
        private const float DefaultDurationSec = 2.15f;
        private const float DriftSpeedPxPerSec = 42f;

        private sealed class PooledPopup
        {
            public GameObject Root;
            public RectTransform Rect;
            public Text Label;
            public float RemainingSec;
            public float TotalDurationSec;
            public Vector2 Position;
            public Color BaseColor;
            public bool Active;
        }

        private Font _font;
        private Transform _container;
        private readonly List<PooledPopup> _pool = new List<PooledPopup>(PoolSize);
        private int _nextSlot;

        public int ActivePopupCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _pool.Count; i++)
                    if (_pool[i].Active) n++;
                return n;
            }
        }

        public string LastShownText { get; private set; } = string.Empty;

        public void BuildLayer(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _pool.Clear();
            _nextSlot = 0;

            var layerGo = new GameObject("FloatingCombatCallouts", typeof(RectTransform));
            layerGo.transform.SetParent(hudRoot, false);
            var rt = layerGo.GetComponent<RectTransform>();
            rt.anchorMin = Vector2.zero;
            rt.anchorMax = Vector2.one;
            rt.offsetMin = Vector2.zero;
            rt.offsetMax = Vector2.zero;
            _container = layerGo.transform;

            for (int i = 0; i < PoolSize; i++)
            {
                var pGo = new GameObject($"Callout_{i}", typeof(RectTransform), typeof(Text));
                pGo.transform.SetParent(_container, false);
                var prt = pGo.GetComponent<RectTransform>();
                prt.anchorMin = new Vector2(0.5f, 0.5f);
                prt.anchorMax = new Vector2(0.5f, 0.5f);
                prt.pivot = new Vector2(0.5f, 0.5f);
                prt.sizeDelta = new Vector2(460f, 30f);

                var txt = pGo.GetComponent<Text>();
                txt.font = _font;
                txt.fontSize = 17;
                txt.fontStyle = FontStyle.Bold;
                txt.alignment = TextAnchor.MiddleCenter;
                txt.supportRichText = false;
                txt.raycastTarget = false;

                pGo.SetActive(false);
                _pool.Add(new PooledPopup
                {
                    Root = pGo,
                    Rect = prt,
                    Label = txt,
                    Active = false
                });
            }
        }

        public void ShowCallout(string message, Color color, float durationSec = DefaultDurationSec, float xOffsetPx = 0f, float yStartPx = 128f)
        {
            if (string.IsNullOrWhiteSpace(message) || _pool.Count == 0) return;
            string trimmed = message.Trim();
            LastShownText = trimmed;

            // Stack active popups slightly so rapid bursts never overlap on the exact same line
            float stackOffset = 0f;
            for (int i = 0; i < _pool.Count; i++)
            {
                if (_pool[i].Active && Mathf.Abs(_pool[i].Position.y - (yStartPx + stackOffset)) < 24f)
                    stackOffset -= 26f;
            }

            var slot = _pool[_nextSlot];
            _nextSlot = (_nextSlot + 1) % _pool.Count;

            slot.Active = true;
            slot.TotalDurationSec = Mathf.Max(0.4f, durationSec);
            slot.RemainingSec = slot.TotalDurationSec;
            slot.BaseColor = color;
            slot.Position = new Vector2(xOffsetPx, yStartPx + stackOffset);
            slot.Label.text = trimmed;
            slot.Label.color = color;
            slot.Rect.anchoredPosition = slot.Position;
            slot.Root.SetActive(true);
        }

        public void ResetSessionState()
        {
            LastShownText = string.Empty;
            for (int i = 0; i < _pool.Count; i++)
            {
                _pool[i].Active = false;
                _pool[i].RemainingSec = 0f;
                if (_pool[i].Root != null) _pool[i].Root.SetActive(false);
            }
        }

        private void Update()
        {
            StepAnimation(Time.unscaledDeltaTime);
        }

        public void StepAnimation(float dt)
        {
            float safeDt = Mathf.Max(0f, dt);
            for (int i = 0; i < _pool.Count; i++)
            {
                var p = _pool[i];
                if (!p.Active) continue;

                p.RemainingSec -= safeDt;
                if (p.RemainingSec <= 0f)
                {
                    p.Active = false;
                    if (p.Root != null) p.Root.SetActive(false);
                    continue;
                }

                p.Position = new Vector2(p.Position.x, p.Position.y + DriftSpeedPxPerSec * safeDt);
                p.Rect.anchoredPosition = p.Position;

                float lifeFrac = Mathf.Clamp01(p.RemainingSec / p.TotalDurationSec);
                float alpha = lifeFrac < 0.35f ? (lifeFrac / 0.35f) : 1f;
                p.Label.color = new Color(p.BaseColor.r, p.BaseColor.g, p.BaseColor.b, p.BaseColor.a * alpha);
            }
        }
    }
}
