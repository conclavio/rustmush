using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Arena;
using TacSim.Net;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// Dedicated 3D Arena Match Replay Theater &amp; Live Staff Observer HUD.
    /// Renders cleanly without overlapping CommsDrawer (`x = 14..436`) or TopStrip (`y = -64..-10`):
    /// - Top-center scoreboard &amp; provenance bar (`Keyframe X of Y` vs `interpolated` vs `LIVE 1 Hz`)
    /// - Right-edge clickable Red/Blue unit focus roster &amp; live shots/wrecks combat feed
    /// - Bottom-right transport bar with scrubber, Play/Pause, `0.5x/1x/2x/4x` speed, keyframe stepping, camera cycle, and Exit
    /// </summary>
    public sealed class TacSimReplayControlsHud : MonoBehaviour
    {
        public event Action OnCloseRequested;
        public event Action OnStopLiveWatchRequested;
        public event Action OnToggleCameraModeRequested;
        public event Action OnCycleOpticsRequested;
        public event Action OnFocusUnitChanged;

        private const int MaxRosterRows = 12;
        private const int MaxTimelineMarkers = 32;

        private sealed class PooledUnitRow
        {
            public GameObject Root;
            public Image Background;
            public Button Button;
            public Text Label;
            public int RosterIndex;
        }

        private Font _font;
        private OocArenaController _arena;
        private GameObject _root;
        private Text _headerBannerText;
        private Text _provenanceLabelText;
        private Text _feedTitleText;
        private Text _eventFeedText;
        private Text _clockReadoutText;
        private Image _scrubberFill;
        private Button _playPauseBtn;
        private Text _playPauseLabel;
        private Text _speedBtnLabel;
        private Button _prevKfBtn;
        private Button _nextKfBtn;
        private Button _prevEventBtn;
        private Button _nextEventBtn;
        private Button _back10Btn;
        private Button _fwd10Btn;
        private Button _speedBtn;
        private Image _opticsBtnBg;
        private Text _opticsBtnLabel;
        private Button _closeBtn;
        private Text _closeBtnLabel;
        private readonly List<PooledUnitRow> _rosterPool = new List<PooledUnitRow>(MaxRosterRows);
        private readonly List<(RectTransform rect, Image img)> _timelineMarkers = new List<(RectTransform, Image)>(MaxTimelineMarkers);
        private ulong _lastMarkerDocMatchId;
        private int _lastMarkerEventCount = -1;

        private float _nextUiRefreshAt;
        private static readonly float[] SpeedPresets = { 0.5f, 1f, 2f, 4f };

        public bool IsVisible => _root != null && _root.activeSelf;
        public string HeaderBannerText => _headerBannerText != null ? _headerBannerText.text : string.Empty;
        public string ProvenanceText => _provenanceLabelText != null ? _provenanceLabelText.text : string.Empty;
        public string ClockText => _clockReadoutText != null ? _clockReadoutText.text : string.Empty;
        public string ReplayStatsSnapshot => _feedTitleText != null ? _feedTitleText.text : string.Empty;
        public string ReplayOpticsButtonSnapshot => _opticsBtnLabel != null ? _opticsBtnLabel.text : string.Empty;
        public int VisibleTimelineMarkerCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _timelineMarkers.Count; i++)
                    if (_timelineMarkers[i].rect != null && _timelineMarkers[i].rect.gameObject.activeSelf) n++;
                return n;
            }
        }
        public void ClickPrevEventForTests()
        {
            _arena?.StepReplayToEvent(-1);
            RefreshView();
        }
        public void ClickNextEventForTests()
        {
            _arena?.StepReplayToEvent(+1);
            RefreshView();
        }
        public void UpdateOpticsButton(int zoomLevel)
        {
            int lvl = Mathf.Clamp(zoomLevel, 0, 2);
            if (_opticsBtnLabel != null)
                _opticsBtnLabel.text = lvl == 2 ? "OPTICS 4x [Q]" : (lvl == 1 ? "OPTICS 2.5x [Q]" : "OPTICS 1x [Q]");
            if (_opticsBtnBg != null)
            {
                _opticsBtnBg.color = lvl == 2
                    ? new Color(0.14f, 0.56f, 0.68f, 0.98f)
                    : (lvl == 1 ? new Color(0.16f, 0.44f, 0.58f, 0.96f) : new Color(0.20f, 0.30f, 0.44f, 0.95f));
            }
        }
        public int VisibleRosterRowCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _rosterPool.Count; i++)
                {
                    if (_rosterPool[i].Root != null && _rosterPool[i].Root.activeSelf) n++;
                }
                return n;
            }
        }

        public void BuildHud(Transform gameplayHudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _rosterPool.Clear();
            _timelineMarkers.Clear();
            _lastMarkerDocMatchId = 0;
            _lastMarkerEventCount = -1;
            if (gameplayHudRoot == null) return;

            _root = CreatePanel(gameplayHudRoot, "ArenaReplayTheaterHud", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, Color.clear);

            // 1. Top-Center Theater Header & Provenance Banner
            var topCard = CreatePanel(_root.transform, "ReplayTopBanner", new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-390f, -132f), new Vector2(390f, -68f), new Color(0.03f, 0.06f, 0.11f, 0.94f));
            CreatePanel(topCard.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.95f, 0.68f, 0.22f, 0.95f));

            _headerBannerText = CreateTxt(topCard.transform, "3D ARENA REPLAY THEATER", 15, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            _headerBannerText.supportRichText = true;
            SetRect(_headerBannerText.rectTransform, new Vector2(0f, 0.48f), new Vector2(1f, 1f), new Vector2(12f, 0f), new Vector2(-12f, -4f));

            _provenanceLabelText = CreateTxt(topCard.transform, "Waiting for replay document...", 11, FontStyle.Italic, TextAnchor.MiddleCenter, new Color(0.76f, 0.88f, 0.96f));
            _provenanceLabelText.supportRichText = false;
            SetRect(_provenanceLabelText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0.48f), new Vector2(12f, 4f), new Vector2(-12f, 0f));

            // 2. Right-Edge Spectator Roster & Shot/Wreck Feed
            var sideCard = CreatePanel(_root.transform, "ReplayRosterAndFeed", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-366f, -540f), new Vector2(-16f, -74f), new Color(0.03f, 0.06f, 0.11f, 0.92f));
            var rHdr = CreateTxt(sideCard.transform, "UNIT CAMERAS — CLICK TO FOLLOW [TAB]", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.92f, 1f));
            SetRect(rHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -26f), new Vector2(-10f, -4f));

            var rosterArea = CreatePanel(sideCard.transform, "ReplayRosterRows", new Vector2(0f, 0.36f), new Vector2(1f, 1f), new Vector2(8f, 6f), new Vector2(-8f, -30f), Color.clear);
            var vlg = rosterArea.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 3f;
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxRosterRows; i++)
            {
                var row = new PooledUnitRow { RosterIndex = i };
                var btn = CreateBtn(rosterArea.transform, string.Empty, new Color(0.12f, 0.18f, 0.26f, 0.92f), () =>
                {
                    if (_arena != null)
                    {
                        _arena.SelectReplayFocusUnit(row.RosterIndex);
                        OnFocusUnitChanged?.Invoke();
                        RefreshView();
                    }
                });
                btn.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 22f);
                var lbl = btn.GetComponentInChildren<Text>();
                if (lbl != null)
                {
                    lbl.fontSize = 11;
                    lbl.alignment = TextAnchor.MiddleLeft;
                    lbl.supportRichText = true;
                    SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-8f, 0f));
                }
                row.Root = btn.gameObject;
                row.Button = btn;
                row.Background = btn.GetComponent<Image>();
                row.Label = lbl;
                row.Root.SetActive(false);
                _rosterPool.Add(row);
            }

            _feedTitleText = CreateTxt(sideCard.transform, "SHOTS & WRECKS TIMELINE", 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.78f, 0.38f));
            SetRect(_feedTitleText.rectTransform, new Vector2(0f, 0.31f), new Vector2(1f, 0.36f), new Vector2(10f, 0f), new Vector2(-10f, 0f));

            var feedBg = CreatePanel(sideCard.transform, "ReplayEventFeedBox", new Vector2(0f, 0f), new Vector2(1f, 0.31f), new Vector2(8f, 8f), new Vector2(-8f, -2f), new Color(0.02f, 0.04f, 0.08f, 0.90f));
            _eventFeedText = CreateTxt(feedBg.transform, "No shots yet.", 11, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.84f, 0.90f, 0.96f));
            _eventFeedText.supportRichText = true;
            _eventFeedText.horizontalOverflow = HorizontalWrapMode.Wrap;
            SetRect(_eventFeedText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 6f), new Vector2(-8f, -6f));

            // 3. Bottom-Right Transport Bar (anchored strictly right of CommsDrawer x = 452..-16)
            var bar = CreatePanel(_root.transform, "ReplayTransportBar", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 16f), new Vector2(-16f, 114f), new Color(0.03f, 0.07f, 0.12f, 0.94f));

            // Top row inside transport bar: Timeline progress track + clock
            var trackRow = CreatePanel(bar.transform, "TimelineRow", new Vector2(0f, 0.56f), new Vector2(1f, 1f), new Vector2(10f, 2f), new Vector2(-10f, -6f), Color.clear);
            _clockReadoutText = CreateTxt(trackRow.transform, "0:00 / 0:00", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.65f, 0.94f, 1f));
            SetRect(_clockReadoutText.rectTransform, new Vector2(0f, 0f), new Vector2(0f, 1f), Vector2.zero, new Vector2(126f, 0f));

            // Clickable segmented seek scrubber (10 quick-jump bins across the track for reliable click-seek in both Unity & headless)
            var trackBg = CreatePanel(trackRow.transform, "ScrubberTrack", new Vector2(0f, 0.16f), new Vector2(1f, 0.84f), new Vector2(132f, 0f), new Vector2(-4f, 0f), new Color(0.08f, 0.14f, 0.22f, 0.95f));
            var fillGo = CreatePanel(trackBg.transform, "ScrubberFill", new Vector2(0f, 0f), new Vector2(0f, 1f), Vector2.zero, Vector2.zero, new Color(0.24f, 0.78f, 0.96f, 0.95f));
            _scrubberFill = fillGo.GetComponent<Image>();
            _scrubberFill.raycastTarget = false;

            // Pooled timeline highlight markers (crimson wrecks + gold hits plotted across scrubber track)
            for (int m = 0; m < MaxTimelineMarkers; m++)
            {
                var mGo = CreatePanel(trackBg.transform, $"TimelineMarker_{m}", new Vector2(0f, 0f), new Vector2(0f, 1f), new Vector2(-1.5f, -2f), new Vector2(1.5f, 2f), new Color(1f, 0.78f, 0.24f, 0.95f));
                var mImg = mGo.GetComponent<Image>();
                mImg.raycastTarget = false;
                mGo.SetActive(false);
                _timelineMarkers.Add((mGo.GetComponent<RectTransform>(), mImg));
            }

            const int SeekBins = 20;
            for (int b = 0; b < SeekBins; b++)
            {
                float frac = (b + 0.5f) / SeekBins;
                var binGo = CreatePanel(trackBg.transform, $"SeekBin_{b}", new Vector2(b / (float)SeekBins, 0f), new Vector2((b + 1) / (float)SeekBins, 1f), Vector2.zero, Vector2.zero, new Color(1f, 1f, 1f, 0.001f));
                var binBtn = binGo.AddComponent<Button>();
                binBtn.targetGraphic = binGo.GetComponent<Image>();
                binBtn.onClick.AddListener(() =>
                {
                    if (_arena != null && _arena.IsReplayModeActive && _arena.ActiveReplayDoc != null)
                    {
                        _arena.SeekReplayMs(_arena.ActiveReplayDoc.EndMs * frac);
                        RefreshView();
                    }
                });
            }

            // Bottom row inside transport bar: Playback & Camera Controls
            var ctrlRow = CreatePanel(bar.transform, "TransportControls", new Vector2(0f, 0f), new Vector2(1f, 0.54f), new Vector2(10f, 6f), new Vector2(-10f, -2f), Color.clear);
            var hlg = ctrlRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 5f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            _prevKfBtn = CreateBtn(ctrlRow.transform, "<< KF", new Color(0.14f, 0.28f, 0.40f), () =>
            {
                _arena?.StepReplayKeyframe(-1);
                RefreshView();
            });
            _prevEventBtn = CreateBtn(ctrlRow.transform, "<< EV", new Color(0.42f, 0.24f, 0.14f), () =>
            {
                _arena?.StepReplayToEvent(-1);
                RefreshView();
            });
            _back10Btn = CreateBtn(ctrlRow.transform, "-10s", new Color(0.14f, 0.28f, 0.40f), () =>
            {
                if (_arena != null) _arena.SeekReplayMs(_arena.ReplayCursorMs - 10000f);
                RefreshView();
            });
            _playPauseBtn = CreateBtn(ctrlRow.transform, "PAUSE [SPC]", new Color(0.16f, 0.48f, 0.36f), () =>
            {
                _arena?.ToggleReplayPlaying();
                RefreshView();
            });
            _playPauseLabel = _playPauseBtn.GetComponentInChildren<Text>();

            _fwd10Btn = CreateBtn(ctrlRow.transform, "+10s", new Color(0.14f, 0.28f, 0.40f), () =>
            {
                if (_arena != null) _arena.SeekReplayMs(_arena.ReplayCursorMs + 10000f);
                RefreshView();
            });
            _nextEventBtn = CreateBtn(ctrlRow.transform, "EV >>", new Color(0.46f, 0.26f, 0.14f), () =>
            {
                _arena?.StepReplayToEvent(+1);
                RefreshView();
            });
            _nextKfBtn = CreateBtn(ctrlRow.transform, "KF >>", new Color(0.14f, 0.28f, 0.40f), () =>
            {
                _arena?.StepReplayKeyframe(+1);
                RefreshView();
            });
            _speedBtn = CreateBtn(ctrlRow.transform, "1x", new Color(0.32f, 0.28f, 0.16f), CyclePlaybackSpeed);
            _speedBtnLabel = _speedBtn.GetComponentInChildren<Text>();

            CreateBtn(ctrlRow.transform, "UNIT [TAB]", new Color(0.18f, 0.34f, 0.48f), () =>
            {
                _arena?.CycleReplayFocusUnit(+1);
                OnFocusUnitChanged?.Invoke();
                RefreshView();
            });
            CreateBtn(ctrlRow.transform, "CAM [V]", new Color(0.24f, 0.28f, 0.46f), () => OnToggleCameraModeRequested?.Invoke());
            var optBtn = CreateBtn(ctrlRow.transform, "OPTICS 1x [Q]", new Color(0.20f, 0.30f, 0.44f), () => OnCycleOpticsRequested?.Invoke());
            _opticsBtnBg = optBtn.GetComponent<Image>();
            _opticsBtnLabel = optBtn.GetComponentInChildren<Text>();

            _closeBtn = CreateBtn(ctrlRow.transform, "CLOSE [ESC]", new Color(0.50f, 0.18f, 0.20f), () =>
            {
                if (_arena != null && _arena.IsLiveObserverActive)
                    OnStopLiveWatchRequested?.Invoke();
                OnCloseRequested?.Invoke();
            });
            _closeBtnLabel = _closeBtn.GetComponentInChildren<Text>();

            _root.SetActive(false);
        }

        public void BindArena(OocArenaController arena)
        {
            if (_arena != null) _arena.OnReplayStateChanged -= RefreshView;
            _arena = arena;
            if (_arena != null) _arena.OnReplayStateChanged += RefreshView;
            RefreshView();
        }

        private void OnDestroy()
        {
            if (_arena != null) _arena.OnReplayStateChanged -= RefreshView;
            _arena = null;
        }

        public void ResetSessionState()
        {
            if (_root != null) _root.SetActive(false);
        }

        public void CyclePlaybackSpeed()
        {
            if (_arena == null || !_arena.IsReplayModeActive) return;
            int idx = 1;
            for (int i = 0; i < SpeedPresets.Length; i++)
            {
                if (Mathf.Abs(_arena.ReplayPlaybackSpeed - SpeedPresets[i]) < 0.05f)
                {
                    idx = (i + 1) % SpeedPresets.Length;
                    break;
                }
            }
            _arena.SetReplaySpeed(SpeedPresets[idx]);
            RefreshView();
        }

        private void Update()
        {
            if (_arena == null || _root == null) return;
            bool show = _arena.IsReplayModeActive || _arena.IsLiveObserverActive;
            if (_root.activeSelf != show)
            {
                _root.SetActive(show);
                if (show) RefreshView();
            }
            if (!show) return;

            if (Time.unscaledTime >= _nextUiRefreshAt)
            {
                _nextUiRefreshAt = Time.unscaledTime + 0.12f;
                RefreshView();
            }
        }

        public void RefreshView()
        {
            if (_root == null) return;
            bool show = _arena != null && (_arena.IsReplayModeActive || _arena.IsLiveObserverActive) && !_arena.IsArenaActive;
            if (_root.activeSelf != show) _root.SetActive(show);
            if (!show) return;

            bool isLive = _arena.IsLiveObserverActive;
            var doc = _arena.ActiveReplayDoc;
            var frame = _arena.LatestReplayFrame;
            var obs = _arena.ActiveObserverPicture;

            if (isLive && obs != null)
            {
                string floorTxt = obs.FloorMs.HasValue ? $" · FLOOR {obs.FloorMs.Value}ms" : string.Empty;
                int redAlive = frame != null ? frame.RedAliveCount : 0;
                int blueAlive = frame != null ? frame.BlueAliveCount : 0;
                _headerBannerText.text =
                    $"<color=#66FFAA>[LIVE OBSERVER]</color> MATCH #{obs.MatchId} [{MushProtocol.EscapeUnityRichText(obs.Tempo.ToUpperInvariant())}{floorTxt}]   |   " +
                    $"<color=#FF6655>RED {redAlive}</color> vs <color=#55AAFF>BLUE {blueAlive}</color>   |   IN FLIGHT: {obs.InFlight}";
                _provenanceLabelText.text = "Live: the arena's own positions, at most once an arena second.";
                _clockReadoutText.text = $"{TacSimArenaReplayModel.FormatClockText(obs.NowMs)} (LIVE)";
                if (_scrubberFill != null)
                    _scrubberFill.rectTransform.anchorMax = new Vector2(1f, 1f);
            }
            else if (doc != null)
            {
                string outcomeTxt = MushProtocol.EscapeUnityRichText(TacSimArenaReplayModel.FormatOutcomeWords(doc.Outcome, doc.Reason));
                string modeTag = !string.IsNullOrEmpty(doc.Mode) && !string.Equals(doc.Mode, "skirmish", StringComparison.OrdinalIgnoreCase)
                    ? $" [{MushProtocol.EscapeUnityRichText(doc.Mode.ToUpperInvariant())}]"
                    : string.Empty;
                int redAlive = frame != null ? frame.RedAliveCount : 0;
                int blueAlive = frame != null ? frame.BlueAliveCount : 0;
                _headerBannerText.text =
                    $"<color=#FFCC55>[REPLAY]</color> MATCH #{doc.MatchId}{modeTag} — {outcomeTxt}   |   " +
                    $"<color=#FF6655>RED {redAlive}</color> vs <color=#55AAFF>BLUE {blueAlive}</color>";
                _provenanceLabelText.text = TacSimArenaReplayModel.FormatFrameSourceLabel(doc, frame);
                long curMs = (long)Mathf.Round(_arena.ReplayCursorMs);
                _clockReadoutText.text = $"{TacSimArenaReplayModel.FormatClockText(curMs)} / {TacSimArenaReplayModel.FormatClockText(doc.EndMs)}";
                float frac = doc.EndMs > 0 ? Mathf.Clamp01(_arena.ReplayCursorMs / doc.EndMs) : 0f;
                if (_scrubberFill != null)
                    _scrubberFill.rectTransform.anchorMax = new Vector2(frac, 1f);
            }
            else
            {
                _headerBannerText.text = $"<color=#FFCC55>[REPLAY]</color> MATCH #{_arena.ActiveSpectatorMatchId} — LOADING TAPE...";
                _provenanceLabelText.text = "Waiting for the replay document from mushd...";
                _clockReadoutText.text = "0:00 / 0:00";
                if (_scrubberFill != null)
                    _scrubberFill.rectTransform.anchorMax = new Vector2(0f, 1f);
            }

            if (_prevKfBtn != null) _prevKfBtn.interactable = !isLive && doc != null;
            if (_nextKfBtn != null) _nextKfBtn.interactable = !isLive && doc != null;
            if (_prevEventBtn != null) _prevEventBtn.interactable = !isLive && doc != null && (doc.Destroyed.Count > 0 || doc.Shots.Count > 0);
            if (_nextEventBtn != null) _nextEventBtn.interactable = !isLive && doc != null && (doc.Destroyed.Count > 0 || doc.Shots.Count > 0);
            if (_back10Btn != null) _back10Btn.interactable = !isLive && doc != null;
            if (_fwd10Btn != null) _fwd10Btn.interactable = !isLive && doc != null;
            if (_playPauseBtn != null) _playPauseBtn.interactable = !isLive && doc != null;
            if (_speedBtn != null) _speedBtn.interactable = !isLive && doc != null;

            if (_playPauseLabel != null)
                _playPauseLabel.text = isLive ? "LIVE 1Hz" : (_arena.ReplayPlaying ? "PAUSE [SPC]" : "PLAY [SPC]");
            if (_speedBtnLabel != null)
                _speedBtnLabel.text = isLive ? "LIVE" : $"{_arena.ReplayPlaybackSpeed:0.#}x";
            if (_closeBtnLabel != null)
                _closeBtnLabel.text = isLive ? "STOP WATCH [ESC]" : "CLOSE [ESC]";

            RefreshTimelineMarkers(doc, isLive);
            RefreshRosterRows(doc, frame, obs, isLive);
            RefreshEventFeed(doc, obs, isLive);
        }

        private void RefreshTimelineMarkers(ArenaReplayDocument doc, bool isLive)
        {
            if (_timelineMarkers.Count == 0) return;
            if (isLive || doc == null || doc.EndMs <= 0)
            {
                _lastMarkerDocMatchId = 0;
                _lastMarkerEventCount = -1;
                for (int i = 0; i < _timelineMarkers.Count; i++)
                    if (_timelineMarkers[i].rect != null && _timelineMarkers[i].rect.gameObject.activeSelf)
                        _timelineMarkers[i].rect.gameObject.SetActive(false);
                return;
            }

            int eventSig = doc.Destroyed.Count * 10000 + doc.Shots.Count;
            if (_lastMarkerDocMatchId == doc.MatchId && _lastMarkerEventCount == eventSig)
                return;
            _lastMarkerDocMatchId = doc.MatchId;
            _lastMarkerEventCount = eventSig;

            int used = 0;
            // Prioritize wreck markers (crimson, taller), then hit shot markers (gold)
            for (int i = 0; i < doc.Destroyed.Count && used < _timelineMarkers.Count; i++)
            {
                float frac = Mathf.Clamp01(doc.Destroyed[i].TimeMs / (float)doc.EndMs);
                var m = _timelineMarkers[used++];
                m.rect.anchorMin = new Vector2(frac, 0f);
                m.rect.anchorMax = new Vector2(frac, 1f);
                m.rect.offsetMin = new Vector2(-1.8f, -3f);
                m.rect.offsetMax = new Vector2(1.8f, 3f);
                m.img.color = new Color(1.0f, 0.28f, 0.24f, 0.98f);
                m.rect.gameObject.SetActive(true);
            }
            for (int i = 0; i < doc.Shots.Count && used < _timelineMarkers.Count; i++)
            {
                if (!string.Equals(doc.Shots[i].Outcome, "hit", StringComparison.OrdinalIgnoreCase)) continue;
                float frac = Mathf.Clamp01(doc.Shots[i].TimeMs / (float)doc.EndMs);
                var m = _timelineMarkers[used++];
                m.rect.anchorMin = new Vector2(frac, 0.15f);
                m.rect.anchorMax = new Vector2(frac, 0.85f);
                m.rect.offsetMin = new Vector2(-1.2f, 0f);
                m.rect.offsetMax = new Vector2(1.2f, 0f);
                m.img.color = new Color(1.0f, 0.82f, 0.26f, 0.92f);
                m.rect.gameObject.SetActive(true);
            }
            for (int i = used; i < _timelineMarkers.Count; i++)
            {
                if (_timelineMarkers[i].rect != null && _timelineMarkers[i].rect.gameObject.activeSelf)
                    _timelineMarkers[i].rect.gameObject.SetActive(false);
            }
        }

        private void RefreshRosterRows(ArenaReplayDocument doc, ArenaReplaySampledFrame frame, ArenaObserverPicture obs, bool isLive)
        {
            int count = doc != null ? Mathf.Min(doc.Roster.Count, _rosterPool.Count) : 0;
            for (int i = 0; i < count; i++)
            {
                var r = doc.Roster[i];
                var slot = _rosterPool[i];
                bool focused = i == _arena.ReplayFocusUnitIndex;
                bool wrecked = frame != null && i < frame.IsWrecked.Length && frame.IsWrecked[i];
                ArenaReplayUnitPose? pose = frame != null && i < frame.Units.Length ? frame.Units[i] : null;

                string sideCol = r.IsBlue ? "#55AAFF" : "#FF6655";
                string where = wrecked
                    ? "<color=#AA6666>WRECK</color>"
                    : (pose.HasValue ? $"({Mathf.RoundToInt(pose.Value.CellX)},{Mathf.RoundToInt(pose.Value.CellY)}) {Mathf.RoundToInt(pose.Value.HeadingDeg):000}°" : "off-map");
                string staffSuffix = string.Empty;
                if (isLive && obs != null && i < obs.Units.Count)
                {
                    var ou = obs.Units[i];
                    staffSuffix = $" #{ou.ObjectId} {(ou.IsMachine ? "AI" : "PILOT")}";
                }

                slot.RosterIndex = i;
                slot.Label.text = $"{(focused ? "<color=#FFEE55>>> </color>" : "")}<color={sideCol}><b>{MushProtocol.EscapeUnityRichText(r.Callsign)}</b></color>  {where}{staffSuffix}";
                slot.Background.color = focused
                    ? new Color(0.28f, 0.36f, 0.16f, 0.96f)
                    : (wrecked
                        ? new Color(0.14f, 0.13f, 0.14f, 0.85f)
                        : (r.IsBlue ? new Color(0.08f, 0.16f, 0.28f, 0.90f) : new Color(0.26f, 0.10f, 0.12f, 0.90f)));
                if (!slot.Root.activeSelf) slot.Root.SetActive(true);
            }
            for (int i = count; i < _rosterPool.Count; i++)
            {
                if (_rosterPool[i].Root.activeSelf) _rosterPool[i].Root.SetActive(false);
            }
        }

        private void RefreshEventFeed(ArenaReplayDocument doc, ArenaObserverPicture obs, bool isLive)
        {
            if (_eventFeedText == null) return;
            if (isLive && obs != null)
            {
                if (_feedTitleText != null) _feedTitleText.text = "LIVE OBSERVER STATUS";
                _eventFeedText.text =
                    $"Match #{obs.MatchId}  |  Tempo: <b>{MushProtocol.EscapeUnityRichText(obs.Tempo)}</b>\n" +
                    $"Ordnance in flight: <b>{obs.InFlight}</b>\n" +
                    $"Arena grid: <b>{obs.Width}x{obs.Height}</b>\n" +
                    "Use <b>[RMB + Scroll]</b> to orbit 3D terrain.\n" +
                    "Press <b>[ESC]</b> or click <b>STOP WATCH</b> to exit.";
                return;
            }

            if (doc == null)
            {
                if (_feedTitleText != null) _feedTitleText.text = "SHOTS & WRECKS TIMELINE";
                _eventFeedText.text = "Waiting for match tape...";
                return;
            }

            float curMs = _arena.ReplayCursorMs;
            int shotsUpToNow = 0, hitsUpToNow = 0, wrecksUpToNow = 0;
            for (int i = 0; i < doc.Shots.Count; i++)
            {
                if (doc.Shots[i].TimeMs > curMs) continue;
                shotsUpToNow++;
                if (string.Equals(doc.Shots[i].Outcome, "hit", StringComparison.OrdinalIgnoreCase)) hitsUpToNow++;
            }
            for (int i = 0; i < doc.Destroyed.Count; i++)
            {
                if (doc.Destroyed[i].TimeMs <= curMs) wrecksUpToNow++;
            }
            if (_feedTitleText != null)
            {
                int acc = shotsUpToNow > 0 ? Mathf.RoundToInt((hitsUpToNow * 100f) / shotsUpToNow) : 0;
                _feedTitleText.text = $"{shotsUpToNow} SHOTS ({hitsUpToNow} HITS {acc}%) · {wrecksUpToNow} WRECK{(wrecksUpToNow == 1 ? "" : "S")}";
            }

            var sb = new StringBuilder(256);
            int lines = 0;

            for (int i = doc.Destroyed.Count - 1; i >= 0 && lines < 3; i--)
            {
                var w = doc.Destroyed[i];
                if (w.TimeMs > curMs) continue;
                string cs = w.RosterIndex >= 0 && w.RosterIndex < doc.Roster.Count ? doc.Roster[w.RosterIndex].Callsign : "?";
                sb.Append($"<color=#FF7766>{TacSimArenaReplayModel.FormatClockText(w.TimeMs)}  <b>{MushProtocol.EscapeUnityRichText(cs)}</b> WRECKED</color>\n");
                lines++;
            }

            for (int i = doc.Shots.Count - 1; i >= 0 && lines < 7; i--)
            {
                var s = doc.Shots[i];
                if (s.TimeMs > curMs) continue;
                string sh = s.ShooterIndex >= 0 && s.ShooterIndex < doc.Roster.Count ? doc.Roster[s.ShooterIndex].Callsign : "?";
                string tg = s.TargetIndex >= 0 && s.TargetIndex < doc.Roster.Count ? doc.Roster[s.TargetIndex].Callsign : "?";
                string col = s.Outcome == "hit" ? "#FFCC55" : (s.Outcome == "flying" ? "#88DDFF" : "#99AABB");
                sb.Append($"<color={col}>{TacSimArenaReplayModel.FormatClockText(s.TimeMs)}  {MushProtocol.EscapeUnityRichText(sh)} -> {MushProtocol.EscapeUnityRichText(tg)}: {s.Outcome}</color>\n");
                lines++;
            }

            _eventFeedText.text = lines > 0 ? sb.ToString().TrimEnd('\n') : "No shots or wrecks up to current timestamp.";
        }

        private static GameObject CreatePanel(Transform parent, string name, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), aMin, aMax, oMin, oMax);
            return go;
        }

        private Text CreateTxt(Transform parent, string content, int size, FontStyle style, TextAnchor align, Color color)
        {
            var go = new GameObject("Txt", typeof(RectTransform), typeof(Text));
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

        private Button CreateBtn(Transform parent, string label, Color bg, Action onClick)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.onClick.AddListener(() => onClick?.Invoke());

            var txt = CreateTxt(go.transform, label, 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(3f, 2f), new Vector2(-3f, -2f));
            return btn;
        }

        private static void SetRect(RectTransform rt, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax)
        {
            rt.anchorMin = aMin;
            rt.anchorMax = aMax;
            rt.offsetMin = oMin;
            rt.offsetMax = oMax;
        }
    }
}
