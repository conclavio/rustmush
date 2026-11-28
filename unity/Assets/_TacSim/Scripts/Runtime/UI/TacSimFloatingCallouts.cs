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

        // Priority upper-center Milestone / Kill-Streak Banner
        private GameObject _milestoneRoot;
        private Image _milestoneBg;
        private Image _milestoneAccent;
        private Text _milestoneText;
        private Color _milestoneAccentBase = new Color(1.0f, 0.34f, 0.22f, 0.96f);
        private float _milestoneRemainSec;
        private float _milestoneTotalSec = 2.8f;

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
        public bool IsMilestoneBannerVisible => _milestoneRoot != null && _milestoneRoot.activeSelf && _milestoneRemainSec > 0f;
        public string ActiveMilestoneBannerText => _milestoneText != null ? _milestoneText.text : string.Empty;

        public static string FormatKillStreakTitle(int matchKills, string targetDesignator)
        {
            string tgt = string.IsNullOrWhiteSpace(targetDesignator) ? "TARGET" : targetDesignator.Trim().ToUpperInvariant();
            if (matchKills <= 1) return $"FIRST BLOOD — {tgt} DESTROYED!";
            if (matchKills == 2) return $"DOUBLE KILL — {tgt} DESTROYED!";
            if (matchKills == 3) return $"TRIPLE KILL — {tgt} DESTROYED!";
            if (matchKills == 4) return $"QUAD KILL — {tgt} DESTROYED!";
            return $"UNSTOPPABLE ({matchKills} KILLS) — {tgt}!";
        }

        public static string FormatQuestCompletionBanner(
            string questTitle,
            int tourDone,
            int tourTotal,
            bool tourAllDone,
            string nextStepHint = null,
            bool includeJobsHint = false)
        {
            if (tourAllDone && tourTotal > 0)
            {
                string core = $"CORRIDOR CAPTAIN — GUIDED TOUR COMPLETE ({tourDone}/{tourTotal})";
                return includeJobsHint ? $"{core} · TRY +market/jobs" : core;
            }
            string title = string.IsNullOrWhiteSpace(questTitle) ? "QUEST" : questTitle.Trim().ToUpperInvariant();
            if (!string.IsNullOrWhiteSpace(nextStepHint))
                return $"QUEST COMPLETED: {title} · {nextStepHint.Trim().ToUpperInvariant()}";
            return $"QUEST COMPLETED: {title}";
        }

        public static string FormatQuestRoomArrivalCallout(
            string roomName,
            bool isTurnIn,
            int nextGuidedStep = 0,
            string nextGuidedGiver = null,
            bool isBoarding = false)
        {
            string roomUpper = string.IsNullOrWhiteSpace(roomName) ? "OBJECTIVE ROOM" : roomName.Trim().ToUpperInvariant();
            if (nextGuidedStep > 0 && !string.IsNullOrWhiteSpace(nextGuidedGiver))
                return $"NEXT GUIDED ({nextGuidedStep}/7) [!]: {nextGuidedGiver.Trim().ToUpperInvariant()} — {roomUpper}";
            if (isBoarding)
                return $"BOARD SHIP FOR QUEST: {roomUpper} (+ship/board)";
            return isTurnIn ? $"QUEST TURN-IN: {roomUpper}" : $"QUEST OBJECTIVE: {roomUpper}";
        }

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

            // Priority Milestone Banner (sits cleanly above drifting callouts at y = +205)
            _milestoneRoot = new GameObject("ArenaMilestoneBanner", typeof(RectTransform), typeof(Image));
            _milestoneRoot.transform.SetParent(_container, false);
            var mrt = _milestoneRoot.GetComponent<RectTransform>();
            mrt.anchorMin = new Vector2(0.5f, 0.5f);
            mrt.anchorMax = new Vector2(0.5f, 0.5f);
            mrt.pivot = new Vector2(0.5f, 0.5f);
            mrt.anchoredPosition = new Vector2(0f, 205f);
            mrt.sizeDelta = new Vector2(460f, 36f);
            _milestoneBg = _milestoneRoot.GetComponent<Image>();
            _milestoneBg.color = new Color(0.04f, 0.06f, 0.10f, 0.90f);
            _milestoneBg.raycastTarget = false;

            var accentGo = new GameObject("Accent", typeof(RectTransform), typeof(Image));
            accentGo.transform.SetParent(_milestoneRoot.transform, false);
            var art = accentGo.GetComponent<RectTransform>();
            art.anchorMin = new Vector2(0f, 1f);
            art.anchorMax = new Vector2(1f, 1f);
            art.offsetMin = new Vector2(0f, -3f);
            art.offsetMax = Vector2.zero;
            _milestoneAccent = accentGo.GetComponent<Image>();
            _milestoneAccent.color = _milestoneAccentBase;
            _milestoneAccent.raycastTarget = false;

            var mtGo = new GameObject("Text", typeof(RectTransform), typeof(Text));
            mtGo.transform.SetParent(_milestoneRoot.transform, false);
            var mtrt = mtGo.GetComponent<RectTransform>();
            mtrt.anchorMin = Vector2.zero;
            mtrt.anchorMax = Vector2.one;
            mtrt.offsetMin = new Vector2(12f, 2f);
            mtrt.offsetMax = new Vector2(-12f, -3f);
            _milestoneText = mtGo.GetComponent<Text>();
            _milestoneText.font = _font;
            _milestoneText.fontSize = 16;
            _milestoneText.fontStyle = FontStyle.Bold;
            _milestoneText.alignment = TextAnchor.MiddleCenter;
            _milestoneText.color = Color.white;
            _milestoneText.supportRichText = false;
            _milestoneText.raycastTarget = false;

            _milestoneRoot.SetActive(false);
        }

        public void ShowMilestoneBanner(string title, Color accentColor, float durationSec = 2.8f)
        {
            if (string.IsNullOrWhiteSpace(title) || _milestoneRoot == null) return;
            string trimmed = title.Trim();
            LastShownText = trimmed;
            _milestoneTotalSec = Mathf.Max(0.6f, durationSec);
            _milestoneRemainSec = _milestoneTotalSec;
            _milestoneAccentBase = accentColor;
            if (_milestoneText != null)
            {
                _milestoneText.text = trimmed;
                _milestoneText.color = Color.white;
            }
            if (_milestoneAccent != null) _milestoneAccent.color = accentColor;
            if (_milestoneBg != null) _milestoneBg.color = new Color(0.04f, 0.06f, 0.10f, 0.90f);
            _milestoneRoot.SetActive(true);
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
            _milestoneRemainSec = 0f;
            if (_milestoneRoot != null) _milestoneRoot.SetActive(false);
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

            if (_milestoneRemainSec > 0f)
            {
                _milestoneRemainSec = Mathf.Max(0f, _milestoneRemainSec - safeDt);
                if (_milestoneRemainSec <= 0f)
                {
                    if (_milestoneRoot != null) _milestoneRoot.SetActive(false);
                }
                else
                {
                    float frac = Mathf.Clamp01(_milestoneRemainSec / Mathf.Max(0.01f, _milestoneTotalSec));
                    float alpha = frac < 0.25f ? (frac / 0.25f) : 1f;
                    if (_milestoneBg != null) _milestoneBg.color = new Color(0.04f, 0.06f, 0.10f, 0.90f * alpha);
                    if (_milestoneAccent != null) _milestoneAccent.color = new Color(_milestoneAccentBase.r, _milestoneAccentBase.g, _milestoneAccentBase.b, _milestoneAccentBase.a * alpha);
                    if (_milestoneText != null) _milestoneText.color = new Color(1f, 1f, 1f, alpha);
                }
            }

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
