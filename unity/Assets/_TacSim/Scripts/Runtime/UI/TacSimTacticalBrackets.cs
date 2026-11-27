using System;
using System.Collections.Generic;
using TacSim.Arena;
using TacSim.Core;
using TacSim.Space;
using TacSim.Surface;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum ArenaHitmarkerKind
    {
        None = 0,
        Hit = 1,
        SectionCrit = 2,
        Kill = 3,
        Miss = 4
    }

    /// <summary>
    /// Pooled 3D Screen-Space Tactical HUD Brackets &amp; Diegetic Combat Schematics:
    /// - Orbital Space (`ShipHelmSeated`): interactive click-to-lock/approach corner brackets projected over
    ///   3D ships &amp; ore asteroids with IFF colors, distance km, `[LOCKED]` reticle, and off-screen edge clamping.
    /// - Planetary Surface (`PlanetarySurface`): projected 3D waypoint &amp; ore vein rings + bottom-center
    ///   rover heading/speed/drill compass strip.
    /// - OOC Simulator Arena (`SimulatorArena`): projected 3D tank/mech target brackets (`[HIT XX%]` + click-to-lock)
    ///   and a live 4-quadrant MechWarrior/Tank directional armor + heat schematic at bottom-center.
    /// Zero per-frame GameObject allocations.
    /// </summary>
    public sealed class TacSimTacticalBrackets : MonoBehaviour
    {
        public event Action<string, bool> OnOrbitalBracketClicked; // (id, isRock)
        public event Action<string> OnArenaBracketClicked;         // (designator)
        public event Action<int> OnReplayBracketClicked;           // (rosterIndex)
        public Func<Vector3, Vector3> ScreenProjectionOverrideForTests;

        private Vector3 ProjectToScreen(Camera cam, Vector3 worldPos) =>
            ScreenProjectionOverrideForTests != null ? ScreenProjectionOverrideForTests(worldPos) : cam.WorldToScreenPoint(worldPos);

        private const int MaxBrackets = 12;

        private sealed class PooledBracket
        {
            public GameObject Root;
            public RectTransform Rect;
            public Button Button;
            public readonly Image[] CornerStrokes = new Image[8];
            public Image CenterFill;
            public Text TagLabel;
            public Text SubLabel;
            public string BoundId;
            public bool BoundIsRock;
            public bool BoundIsArena;
            public int BoundReplayRosterIndex = -1;
        }

        private Font _font;
        private RectTransform _layerRect;
        private readonly List<PooledBracket> _pool = new List<PooledBracket>(MaxBrackets);

        // Arena 4-Quadrant Directional Armor & Heat Schematic (bottom-center, zero overlap)
        private GameObject _arenaArmorWidget;
        private Image _armorFrontImg;
        private Image _armorLeftImg;
        private Image _armorRightImg;
        private Image _armorRearImg;
        private Image _armorCoreImg;
        private Text _armorFrontTxt;
        private Text _armorLeftTxt;
        private Text _armorRightTxt;
        private Text _armorRearTxt;
        private Text _armorCoreTxt;
        private Image _arenaHeatFill;
        private Text _arenaHeatLabel;
        private Text _arenaDriveStrip;
        private Text _arenaGunneryStrip;

        // Arena Top-Center 360-deg Scrolling Compass Tape & Tactical Telemetry Strip
        private const float ArenaCompassWidthPx = 420f;
        private const float ArenaCompassFovDeg = 140f;
        private GameObject _arenaCompassWidget;
        private Text _arenaCompassHeadingTxt;
        private Text _arenaCompassTelemetryTxt;
        private readonly RectTransform[] _compassTickRects = new RectTransform[8];
        private readonly Text[] _compassTickLabels = new Text[8];
        private static readonly float[] CompassTickBearings = { 0f, 45f, 90f, 135f, 180f, 225f, 270f, 315f };
        private static readonly string[] CompassTickNames = { "N", "045", "E", "135", "S", "225", "W", "315" };
        private RectTransform _compassOrderBugPip;
        private RectTransform _compassLockPip;
        private RectTransform _compassObjectivePip;
        private Text _compassObjectivePipLabel;
        private RectTransform _compassWaypointPip;

        // Arena Screen-Center Hitmarker, 4-Quadrant Directional Damage Arcs & Critical Status Banner
        private GameObject _arenaHitmarkerRoot;
        private readonly Image[] _hitmarkerTicks = new Image[4];
        private readonly RectTransform[] _hitmarkerTickRects = new RectTransform[4];
        private ArenaHitmarkerKind _activeHitmarkerKind = ArenaHitmarkerKind.None;
        private float _hitmarkerRemainSec;
        private float _hitmarkerTotalSec = 0.35f;

        private GameObject _arenaDamageArcsRoot;
        private Image _dmgArcFront;
        private Image _dmgArcRear;
        private Image _dmgArcLeft;
        private Image _dmgArcRight;
        private float _dmgAlphaFront;
        private float _dmgAlphaRear;
        private float _dmgAlphaLeft;
        private float _dmgAlphaRight;

        private GameObject _arenaCritBannerRoot;
        private Image _arenaCritBannerBg;
        private Text _arenaCritBannerText;

        // Arena Diegetic Gunner Optics Crosshair & Laser Rangefinder Reticle (active when zoomed or in cupola view)
        private GameObject _arenaOpticsReticleRoot;
        private Text _opticsModeLabel;
        private Text _opticsWeaponStatusLabel;
        private Text _opticsRangefinderLabel;
        private Image _opticsRangefinderBg;
        private readonly List<Image> _opticsFrameStrokes = new List<Image>(16);

        // Surface Rover Compass & Drill Strip (bottom-center above SurfaceRoverActions)
        private GameObject _surfaceCompassWidget;
        private Text _surfaceCompassText;
        private Image _surfaceDrillIndicator;

        // Space Center Flight Reticle (subtle center pip when seated at helm)
        private GameObject _spaceReticleWidget;

        // 6-DOF Bow Docking Alignment Crosshair HUD Reticle (active during IsDockingActive)
        private GameObject _dockingReticleWidget;
        private RectTransform _dockLateralPip;
        private Image _dockLateralPipImg;
        private RectTransform _dockAttitudePip;
        private Image _dockAttitudePipImg;
        private RectTransform _dockRollBar;
        private Image _dockRollBarImg;
        private Image _dockClosureFill;
        private Text _dockStatusText;

        private SpaceFlightController _space;
        private SurfaceMiningController _surface;
        private OocArenaController _arena;

        public int ActiveBracketCount { get; private set; }
        public bool IsArenaArmorSchematicVisible => _arenaArmorWidget != null && _arenaArmorWidget.activeSelf;
        public bool IsArenaCompassVisible => _arenaCompassWidget != null && _arenaCompassWidget.activeSelf;
        public string ArenaCompassHeadingSnapshot => _arenaCompassHeadingTxt != null ? _arenaCompassHeadingTxt.text : string.Empty;
        public string ArenaCompassTelemetrySnapshot => _arenaCompassTelemetryTxt != null ? _arenaCompassTelemetryTxt.text : string.Empty;
        public string ArenaTerrainBadgeSnapshot { get; private set; } = string.Empty;
        public bool ArenaCompassLockPipVisible => _compassLockPip != null && _compassLockPip.gameObject.activeSelf;
        public bool ArenaCompassObjectivePipVisible => _compassObjectivePip != null && _compassObjectivePip.gameObject.activeSelf;
        public bool ArenaCompassWaypointPipVisible => _compassWaypointPip != null && _compassWaypointPip.gameObject.activeSelf;
        public bool ArenaCompassOrderBugVisible => _compassOrderBugPip != null && _compassOrderBugPip.gameObject.activeSelf;
        public bool IsSurfaceCompassVisible => _surfaceCompassWidget != null && _surfaceCompassWidget.activeSelf;
        public bool IsDockingReticleVisible => _dockingReticleWidget != null && _dockingReticleWidget.activeSelf;
        public Vector2 DockingLateralPipPos => _dockLateralPip != null ? _dockLateralPip.anchoredPosition : Vector2.zero;
        public float DockingRollBarAngleDeg { get; private set; }
        public bool DockingReticleAllGreen { get; private set; }
        public string SurfaceCompassSnapshot => _surfaceCompassText != null ? _surfaceCompassText.text : string.Empty;
        public string ArenaArmorSummarySnapshot => _arenaDriveStrip != null ? _arenaDriveStrip.text : string.Empty;
        public string ArenaGunnerySnapshot => _arenaGunneryStrip != null ? _arenaGunneryStrip.text : string.Empty;
        public bool ArenaObjectiveBracketActive { get; private set; }
        public bool ArenaDriveWaypointBracketActive { get; private set; }
        public int OffscreenThreatBracketCount { get; private set; }
        public string GetActiveBracketTagSnapshot(int index) =>
            (index >= 0 && index < ActiveBracketCount && index < _pool.Count && _pool[index].TagLabel != null)
                ? (_pool[index].TagLabel.text ?? string.Empty)
                : string.Empty;
        public bool ClickBracketForTests(int index)
        {
            if (index < 0 || index >= ActiveBracketCount || index >= _pool.Count) return false;
            var b = _pool[index];
            if (b.Button == null || !b.Button.interactable) return false;
            if (b.BoundReplayRosterIndex >= 0)
            {
                OnReplayBracketClicked?.Invoke(b.BoundReplayRosterIndex);
                return true;
            }
            if (string.IsNullOrEmpty(b.BoundId)) return false;
            if (b.BoundIsArena) OnArenaBracketClicked?.Invoke(b.BoundId);
            else OnOrbitalBracketClicked?.Invoke(b.BoundId, b.BoundIsRock);
            return true;
        }
        public bool IsHitmarkerVisible => _arenaHitmarkerRoot != null && _arenaHitmarkerRoot.activeSelf && _hitmarkerRemainSec > 0f;
        public ArenaHitmarkerKind ActiveHitmarkerKind => _activeHitmarkerKind;
        public bool IsCriticalStatusBannerVisible => _arenaCritBannerRoot != null && _arenaCritBannerRoot.activeSelf;
        public string CriticalStatusBannerText => _arenaCritBannerText != null ? _arenaCritBannerText.text : string.Empty;
        public bool IsGunnerOpticsReticleVisible => _arenaOpticsReticleRoot != null && _arenaOpticsReticleRoot.activeSelf;
        public string GunnerOpticsModeSnapshot => _opticsModeLabel != null ? _opticsModeLabel.text : string.Empty;
        public string GunnerOpticsRangefinderSnapshot => _opticsRangefinderLabel != null ? _opticsRangefinderLabel.text : string.Empty;
        public string GunnerOpticsWeaponStatusSnapshot => _opticsWeaponStatusLabel != null ? _opticsWeaponStatusLabel.text : string.Empty;

        public float GetDamageArcAlpha(string facetLabel)
        {
            string up = (facetLabel ?? string.Empty).Trim().ToUpperInvariant();
            switch (up)
            {
                case "FRONT": return _dmgAlphaFront;
                case "REAR": return _dmgAlphaRear;
                case "LEFT": return _dmgAlphaLeft;
                case "RIGHT": return _dmgAlphaRight;
                default: return Mathf.Max(Mathf.Max(_dmgAlphaFront, _dmgAlphaRear), Mathf.Max(_dmgAlphaLeft, _dmgAlphaRight));
            }
        }

        public void TriggerArenaHitmarker(ArenaHitmarkerKind kind)
        {
            if (kind == ArenaHitmarkerKind.None || _arenaHitmarkerRoot == null) return;
            _activeHitmarkerKind = kind;
            _hitmarkerTotalSec = kind == ArenaHitmarkerKind.Kill
                ? 0.65f
                : (kind == ArenaHitmarkerKind.SectionCrit ? 0.45f : (kind == ArenaHitmarkerKind.Miss ? 0.22f : 0.30f));
            _hitmarkerRemainSec = _hitmarkerTotalSec;
            _arenaHitmarkerRoot.SetActive(true);
            ApplyHitmarkerVisual(1f);
        }

        public void TriggerDirectionalDamageArc(string facetLabel, bool internalBreach)
        {
            string up = (facetLabel ?? string.Empty).Trim().ToUpperInvariant();
            if (internalBreach || up == "INTERNALS" || up == "HULL")
            {
                _dmgAlphaFront = Mathf.Max(_dmgAlphaFront, 0.85f);
                _dmgAlphaRear = Mathf.Max(_dmgAlphaRear, 0.85f);
                _dmgAlphaLeft = Mathf.Max(_dmgAlphaLeft, 0.85f);
                _dmgAlphaRight = Mathf.Max(_dmgAlphaRight, 0.85f);
            }
            else if (up == "FRONT") _dmgAlphaFront = 0.92f;
            else if (up == "REAR") _dmgAlphaRear = 0.92f;
            else if (up == "LEFT") _dmgAlphaLeft = 0.92f;
            else if (up == "RIGHT") _dmgAlphaRight = 0.92f;
            else _dmgAlphaFront = Mathf.Max(_dmgAlphaFront, 0.72f);

            if (_arenaDamageArcsRoot != null)
                _arenaDamageArcsRoot.SetActive(true);
            ApplyDamageArcAlphas();
        }

        public void StepCombatFeedbackForTests(float dt) => StepCombatFeedback(Mathf.Max(0f, dt));

        private void Update()
        {
            StepCombatFeedback(Time.unscaledDeltaTime);
        }

        private void StepCombatFeedback(float dt)
        {
            if (dt <= 0f) return;

            if (_hitmarkerRemainSec > 0f)
            {
                _hitmarkerRemainSec = Mathf.Max(0f, _hitmarkerRemainSec - dt);
                float frac = _hitmarkerTotalSec > 0.001f ? Mathf.Clamp01(_hitmarkerRemainSec / _hitmarkerTotalSec) : 0f;
                if (_hitmarkerRemainSec <= 0f)
                {
                    _activeHitmarkerKind = ArenaHitmarkerKind.None;
                    if (_arenaHitmarkerRoot != null) _arenaHitmarkerRoot.SetActive(false);
                }
                else
                {
                    ApplyHitmarkerVisual(frac);
                }
            }

            if (_dmgAlphaFront > 0f || _dmgAlphaRear > 0f || _dmgAlphaLeft > 0f || _dmgAlphaRight > 0f)
            {
                float decay = dt * 0.95f;
                _dmgAlphaFront = Mathf.Max(0f, _dmgAlphaFront - decay);
                _dmgAlphaRear = Mathf.Max(0f, _dmgAlphaRear - decay);
                _dmgAlphaLeft = Mathf.Max(0f, _dmgAlphaLeft - decay);
                _dmgAlphaRight = Mathf.Max(0f, _dmgAlphaRight - decay);
                ApplyDamageArcAlphas();
                bool any = _dmgAlphaFront > 0.005f || _dmgAlphaRear > 0.005f || _dmgAlphaLeft > 0.005f || _dmgAlphaRight > 0.005f;
                if (!any && _arenaDamageArcsRoot != null && _arenaDamageArcsRoot.activeSelf)
                    _arenaDamageArcsRoot.SetActive(false);
            }
        }

        private void ApplyHitmarkerVisual(float lifeFrac)
        {
            Color col;
            float baseSpread;
            float tickLen;
            switch (_activeHitmarkerKind)
            {
                case ArenaHitmarkerKind.Kill:
                    col = new Color(1.0f, 0.24f, 0.20f, Mathf.Clamp01(lifeFrac * 1.4f));
                    baseSpread = 14f + (1f - lifeFrac) * 14f;
                    tickLen = 18f;
                    break;
                case ArenaHitmarkerKind.SectionCrit:
                    col = new Color(1.0f, 0.72f, 0.16f, Mathf.Clamp01(lifeFrac * 1.4f));
                    baseSpread = 12f + (1f - lifeFrac) * 10f;
                    tickLen = 15f;
                    break;
                case ArenaHitmarkerKind.Miss:
                    col = new Color(0.54f, 0.86f, 0.96f, Mathf.Clamp01(lifeFrac * 0.9f));
                    baseSpread = 10f + (1f - lifeFrac) * 6f;
                    tickLen = 9f;
                    break;
                default:
                    col = new Color(1.0f, 0.96f, 0.82f, Mathf.Clamp01(lifeFrac * 1.4f));
                    baseSpread = 10f + (1f - lifeFrac) * 8f;
                    tickLen = 13f;
                    break;
            }

            Vector2[] dirs =
            {
                new Vector2(0.7071f, 0.7071f),
                new Vector2(-0.7071f, 0.7071f),
                new Vector2(-0.7071f, -0.7071f),
                new Vector2(0.7071f, -0.7071f)
            };
            for (int i = 0; i < 4; i++)
            {
                if (_hitmarkerTicks[i] != null) _hitmarkerTicks[i].color = col;
                if (_hitmarkerTickRects[i] != null)
                {
                    _hitmarkerTickRects[i].anchoredPosition = dirs[i] * baseSpread;
                    _hitmarkerTickRects[i].sizeDelta = new Vector2(tickLen, _activeHitmarkerKind == ArenaHitmarkerKind.Kill ? 3.5f : 2.5f);
                }
            }
        }

        private void ApplyDamageArcAlphas()
        {
            if (_dmgArcFront != null) _dmgArcFront.color = new Color(1.0f, 0.18f, 0.14f, _dmgAlphaFront * 0.55f);
            if (_dmgArcRear != null) _dmgArcRear.color = new Color(1.0f, 0.18f, 0.14f, _dmgAlphaRear * 0.55f);
            if (_dmgArcLeft != null) _dmgArcLeft.color = new Color(1.0f, 0.18f, 0.14f, _dmgAlphaLeft * 0.55f);
            if (_dmgArcRight != null) _dmgArcRight.color = new Color(1.0f, 0.18f, 0.14f, _dmgAlphaRight * 0.55f);
        }

        public void BuildLayer(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _pool.Clear();

            var layerGo = new GameObject("TacticalBracketsLayer", typeof(RectTransform));
            layerGo.transform.SetParent(hudRoot, false);
            _layerRect = layerGo.GetComponent<RectTransform>();
            SetRect(_layerRect, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            for (int i = 0; i < MaxBrackets; i++)
                _pool.Add(CreatePooledBracket(_layerRect, i));

            BuildSpaceCenterReticle(_layerRect);
            BuildDockingAlignmentReticle(_layerRect);
            BuildSurfaceCompassStrip(_layerRect);
            BuildArenaArmorSchematic(_layerRect);
            BuildArenaCompassTape(_layerRect);
            BuildArenaCombatFeedbackWidgets(_layerRect);
            BuildArenaGunnerOpticsReticle(_layerRect);
        }

        public void BindControllers(SpaceFlightController space, SurfaceMiningController surface, OocArenaController arena)
        {
            _space = space;
            _surface = surface;
            _arena = arena;
        }

        public void ResetSessionState()
        {
            HideAllBrackets();
            if (_spaceReticleWidget != null) _spaceReticleWidget.SetActive(false);
            if (_dockingReticleWidget != null) _dockingReticleWidget.SetActive(false);
            if (_surfaceCompassWidget != null) _surfaceCompassWidget.SetActive(false);
            if (_arenaArmorWidget != null) _arenaArmorWidget.SetActive(false);
            if (_arenaCompassWidget != null) _arenaCompassWidget.SetActive(false);
            if (_arenaHitmarkerRoot != null) _arenaHitmarkerRoot.SetActive(false);
            if (_arenaDamageArcsRoot != null) _arenaDamageArcsRoot.SetActive(false);
            if (_arenaCritBannerRoot != null) _arenaCritBannerRoot.SetActive(false);
            if (_arenaOpticsReticleRoot != null) _arenaOpticsReticleRoot.SetActive(false);
            _activeHitmarkerKind = ArenaHitmarkerKind.None;
            _hitmarkerRemainSec = 0f;
            _dmgAlphaFront = _dmgAlphaRear = _dmgAlphaLeft = _dmgAlphaRight = 0f;
            DockingRollBarAngleDeg = 0f;
            DockingReticleAllGreen = false;
        }

        /// <summary>
        /// Pure-math signed horizontal pixel offset on a 360-degree scrolling compass tape.
        /// Wraps shortest angular delta cleanly across North (355° -> 005° = +10°).
        /// </summary>
        public static float ComputeCompassTapeOffsetPx(
            float centerHeadingDeg,
            float markerBearingDeg,
            float tapeWidthPx,
            float fovDeg,
            out bool withinTape)
        {
            float delta = Mathf.DeltaAngle(centerHeadingDeg, markerBearingDeg);
            float halfFov = Mathf.Max(20f, fovDeg * 0.5f);
            withinTape = Mathf.Abs(delta) <= halfFov;
            float clamped = Mathf.Clamp(delta, -halfFov, halfFov);
            return (clamped / halfFov) * (tapeWidthPx * 0.5f - 14f);
        }

        public static void ComputeDockingReticleOffsets(
            long dxMm,
            long dyMm,
            long yawMdeg,
            long pitchMdeg,
            long rollMdeg,
            long vzMmps,
            out Vector2 lateralPx,
            out Vector2 attitudePx,
            out float rollDeg,
            out float closureNorm)
        {
            // ±150mm lateral gate maps cleanly inside the ±26px center capture box; clamp at ±108px
            float lx = Mathf.Clamp((dxMm / 150f) * 26f, -108f, 108f);
            float ly = Mathf.Clamp((dyMm / 150f) * 26f, -108f, 108f);
            lateralPx = new Vector2(lx, ly);

            // ±3 deg (3000 mdeg) attitude gate maps inside ±24px; clamp at ±104px
            float ax = Mathf.Clamp((yawMdeg / 3000f) * 24f, -104f, 104f);
            float ay = Mathf.Clamp((-pitchMdeg / 3000f) * 24f, -104f, 104f);
            attitudePx = new Vector2(ax, ay);

            rollDeg = Mathf.Clamp(-rollMdeg / 1000f, -65f, 65f);
            closureNorm = Mathf.Clamp01(vzMmps / 1000f);
        }

        /// <summary>
        /// Pure-math viewport projection into normalized Canvas coordinates with smooth edge clamping
        /// when a tracked target lies off-screen or behind the camera.
        /// </summary>
        public static void ComputeCanvasPositionFromScreenPoint(
            Vector3 screenPos,
            float screenWidth,
            float screenHeight,
            Vector2 canvasSize,
            float edgeMarginPx,
            out Vector2 anchoredPos,
            out bool isBehind,
            out bool isClamped)
        {
            float sw = Mathf.Max(1f, screenWidth);
            float sh = Mathf.Max(1f, screenHeight);
            float cw = Mathf.Max(100f, canvasSize.x);
            float ch = Mathf.Max(100f, canvasSize.y);

            isBehind = screenPos.z < 0f;
            float nx = screenPos.x / sw;
            float ny = screenPos.y / sh;

            if (isBehind)
            {
                // Mirror behind-camera vector to perimeter edge so the pilot knows which way to turn
                nx = 1f - nx;
                ny = 1f - ny;
                float dx = nx - 0.5f;
                float dy = ny - 0.5f;
                if (Mathf.Abs(dx) < 0.001f && Mathf.Abs(dy) < 0.001f) dy = -1f;
                float scale = 0.48f / Mathf.Max(Mathf.Abs(dx), Mathf.Abs(dy));
                nx = 0.5f + dx * scale;
                ny = 0.5f + dy * scale;
            }

            float rawX = (nx - 0.5f) * cw;
            float rawY = (ny - 0.5f) * ch;

            float halfW = cw * 0.5f - edgeMarginPx;
            float halfH = ch * 0.5f - edgeMarginPx;
            float topLimit = ch * 0.5f - Mathf.Max(edgeMarginPx, 86f);
            float botLimit = -ch * 0.5f + Mathf.Max(edgeMarginPx, 130f);

            float clampedX = Mathf.Clamp(rawX, -halfW, halfW);
            float clampedY = Mathf.Clamp(rawY, botLimit, topLimit);

            isClamped = isBehind || Mathf.Abs(clampedX - rawX) > 0.5f || Mathf.Abs(clampedY - rawY) > 0.5f;
            anchoredPos = new Vector2(clampedX, clampedY);
        }

        public void SyncBrackets(TacSimActiveStage stage, Camera cam, bool hideForOverlay = false, int arenaOpticsZoomLevel = 0, bool arenaCupolaView = false)
        {
            if (_layerRect == null) return;

            if (hideForOverlay || stage == TacSimActiveStage.StationWalk || stage == TacSimActiveStage.ShipInteriorWalk)
            {
                HideAllBrackets();
                if (_spaceReticleWidget != null) _spaceReticleWidget.SetActive(false);
                if (_dockingReticleWidget != null) _dockingReticleWidget.SetActive(false);
                if (_surfaceCompassWidget != null) _surfaceCompassWidget.SetActive(false);
                if (_arenaArmorWidget != null) _arenaArmorWidget.SetActive(false);
                if (_arenaCompassWidget != null) _arenaCompassWidget.SetActive(false);
                if (_arenaCritBannerRoot != null) _arenaCritBannerRoot.SetActive(false);
                if (_arenaOpticsReticleRoot != null) _arenaOpticsReticleRoot.SetActive(false);
                return;
            }

            Vector2 canvasSize = _layerRect.rect.width > 10f && _layerRect.rect.height > 10f
                ? new Vector2(_layerRect.rect.width, _layerRect.rect.height)
                : new Vector2(1920f, 1080f);
            float sw = Mathf.Max(1f, Screen.width > 0 ? Screen.width : 1920f);
            float sh = Mathf.Max(1f, Screen.height > 0 ? Screen.height : 1080f);

            bool dockingActive = stage == TacSimActiveStage.ShipHelmSeated && _space != null && _space.IsDockingActive && _space.ActiveDockingSession != null;
            bool spectatorArena = stage == TacSimActiveStage.SimulatorArena && _arena != null && _arena.IsSpectatorTheaterActive;
            bool liveArena = stage == TacSimActiveStage.SimulatorArena && !spectatorArena;

            if (_spaceReticleWidget != null)
                _spaceReticleWidget.SetActive(stage == TacSimActiveStage.ShipHelmSeated && !dockingActive);
            if (_dockingReticleWidget != null)
                _dockingReticleWidget.SetActive(dockingActive);
            if (_surfaceCompassWidget != null)
                _surfaceCompassWidget.SetActive(stage == TacSimActiveStage.PlanetarySurface);
            if (_arenaArmorWidget != null)
                _arenaArmorWidget.SetActive(liveArena);
            if (_arenaCompassWidget != null)
                _arenaCompassWidget.SetActive(liveArena);
            if (_arenaCritBannerRoot != null && !liveArena)
                _arenaCritBannerRoot.SetActive(false);
            if (_arenaOpticsReticleRoot != null && !liveArena)
                _arenaOpticsReticleRoot.SetActive(false);

            if (dockingActive)
            {
                HideAllBrackets();
                SyncDockingAlignmentReticle(_space.ActiveDockingSession);
                return;
            }

            switch (stage)
            {
                case TacSimActiveStage.ShipHelmSeated:
                    SyncSpaceBrackets(cam, sw, sh, canvasSize);
                    break;
                case TacSimActiveStage.PlanetarySurface:
                    SyncSurfaceBracketsAndCompass(cam, sw, sh, canvasSize);
                    break;
                case TacSimActiveStage.SimulatorArena:
                    if (spectatorArena)
                        SyncReplaySpectatorBrackets(cam, sw, sh, canvasSize);
                    else
                        SyncArenaBracketsAndArmor(cam, sw, sh, canvasSize, arenaOpticsZoomLevel, arenaCupolaView);
                    break;
                default:
                    HideAllBrackets();
                    break;
            }
        }

        private void SyncSpaceBrackets(Camera cam, float sw, float sh, Vector2 canvasSize)
        {
            if (_space == null || cam == null || _space.IsDockingActive)
            {
                HideAllBrackets();
                return;
            }

            var blips = _space.RadarBlips;
            int used = 0;
            for (int i = 0; i < blips.Count && used < MaxBrackets; i++)
            {
                var b = blips[i];
                if (b.IsTorpedo || string.IsNullOrEmpty(b.Id)) continue;
                if (!_space.TryGetTargetPosition(b.Id, out Vector3 worldPos)) continue;

                Vector3 sp = cam.WorldToScreenPoint(worldPos);
                ComputeCanvasPositionFromScreenPoint(sp, sw, sh, canvasSize, 52f, out Vector2 pos, out _, out bool clamped);

                Color col = b.IsLocked
                    ? new Color(1.0f, 0.34f, 0.24f, 0.96f)
                    : (b.IsRock
                        ? new Color(0.95f, 0.78f, 0.28f, 0.92f)
                        : (b.IsHostile
                            ? new Color(1.0f, 0.42f, 0.36f, 0.92f)
                            : new Color(0.32f, 0.90f, 1.0f, 0.90f)));

                string prefix = clamped ? "» " : string.Empty;
                string tag = b.IsLocked
                    ? $"{prefix}[LOCK] {(string.IsNullOrEmpty(b.Label) ? b.Id : b.Label)}"
                    : (b.IsRock ? $"{prefix}ROCK {b.Id}" : $"{prefix}{(string.IsNullOrEmpty(b.Label) ? b.Id : b.Label)}");
                string sub = b.RangeKm >= 1.0 ? $"{b.RangeKm:0.1} km" : $"{b.RangeKm * 1000.0:0} m";

                ConfigureBracket(_pool[used], pos, b.IsLocked ? 72f : 56f, col, tag, sub, b.Id, b.IsRock, isArena: false, interactive: true);
                used++;
            }

            HideUnusedBracketsFrom(used);
        }

        private void SyncSurfaceBracketsAndCompass(Camera cam, float sw, float sh, Vector2 canvasSize)
        {
            if (_surface == null)
            {
                HideAllBrackets();
                return;
            }

            if (_surfaceCompassText != null)
            {
                float hdg = (_surface.RoverHeadingDeg % 360f + 360f) % 360f;
                string drillStr = _surface.IsDrillingActive ? "DRILLING" : "DRILL READY";
                _surfaceCompassText.text =
                    $"HDG {hdg:000}°  |  {_surface.RoverSpeedKph:0} kph  |  CELL ({_surface.AuthoritativeSiteCellX},{_surface.AuthoritativeSiteCellY})  |  {drillStr}";
            }
            if (_surfaceDrillIndicator != null)
            {
                _surfaceDrillIndicator.color = _surface.IsDrillingActive
                    ? new Color(1.0f, 0.72f, 0.18f, 1f)
                    : new Color(0.22f, 0.80f, 0.56f, 0.9f);
            }

            if (cam == null)
            {
                HideAllBrackets();
                return;
            }

            int used = 0;
            if (_surface.TryGetActiveWaypointWorldPos(out Vector3 wpPos, out string wpLabel) && used < MaxBrackets)
            {
                Vector3 sp = cam.WorldToScreenPoint(wpPos);
                ComputeCanvasPositionFromScreenPoint(sp, sw, sh, canvasSize, 52f, out Vector2 pos, out _, out bool clamped);
                float distM = _surface.RoverTransform != null ? Vector3.Distance(_surface.RoverTransform.position, wpPos) : 0f;
                ConfigureBracket(
                    _pool[used],
                    pos,
                    58f,
                    new Color(0.32f, 0.96f, 1.0f, 0.95f),
                    clamped ? $"» {wpLabel}" : wpLabel,
                    $"{distM:0} m",
                    string.Empty,
                    isRock: false,
                    isArena: false,
                    interactive: false);
                used++;
            }

            var beacons = _surface.OreBeacons;
            for (int i = 0; i < beacons.Count && used < MaxBrackets; i++)
            {
                var bc = beacons[i];
                Vector3 sp = cam.WorldToScreenPoint(bc.WorldPos);
                if (sp.z < 0f) continue; // Keep off-screen ore beacons clean so only active waypoint clamps
                ComputeCanvasPositionFromScreenPoint(sp, sw, sh, canvasSize, 48f, out Vector2 pos, out _, out bool clamped);
                if (clamped) continue;
                float distM = _surface.RoverTransform != null ? Vector3.Distance(_surface.RoverTransform.position, bc.WorldPos) : 0f;
                Color col = bc.IsRich ? new Color(0.24f, 1.0f, 0.58f, 0.94f) : new Color(0.95f, 0.78f, 0.28f, 0.88f);
                ConfigureBracket(_pool[used], pos, 46f, col, bc.IsRich ? "RICH ORE" : "ORE VEIN", $"{distM:0} m", string.Empty, false, false, interactive: false);
                used++;
            }

            HideUnusedBracketsFrom(used);
        }

        private void SyncArenaBracketsAndArmor(Camera cam, float sw, float sh, Vector2 canvasSize, int opticsZoomLevel = 0, bool cupolaView = false)
        {
            if (_arena == null || _arena.State == null)
            {
                if (_arenaOpticsReticleRoot != null) _arenaOpticsReticleRoot.SetActive(false);
                HideAllBrackets();
                return;
            }

            var st = _arena.State;
            SetArmorPlate(_armorFrontImg, _armorFrontTxt, "F", st.ArmorFrontPct);
            SetArmorPlate(_armorLeftImg, _armorLeftTxt, "L", st.ArmorLeftPct);
            SetArmorPlate(_armorRightImg, _armorRightTxt, "R", st.ArmorRightPct);
            SetArmorPlate(_armorRearImg, _armorRearTxt, "B", st.ArmorRearPct);
            SetArmorPlate(_armorCoreImg, _armorCoreTxt, "INT", st.InternalsPct);

            float heatFrac = st.HeatMax > 0 ? Mathf.Clamp01(st.Heat / (float)st.HeatMax) : 0f;
            if (_arenaHeatFill != null)
            {
                _arenaHeatFill.rectTransform.anchorMax = new Vector2(Mathf.Max(0.02f, heatFrac), 1f);
                _arenaHeatFill.color = st.Overheated || heatFrac > 0.78f
                    ? new Color(1.0f, 0.24f, 0.20f, 0.98f)
                    : (heatFrac > 0.45f ? new Color(0.96f, 0.64f, 0.16f, 0.96f) : new Color(0.24f, 0.76f, 0.95f, 0.94f));
            }
            if (_arenaHeatLabel != null)
                _arenaHeatLabel.text = st.Overheated ? $"OVERHEAT {st.Heat}/{st.HeatMax}" : $"HEAT {st.Heat}/{st.HeatMax}";

            if (_arenaDriveStrip != null)
            {
                float hdg = (_arena.PredictedHeadingDeg % 360f + 360f) % 360f;
                string lockTag = !string.IsNullOrEmpty(st.LockedDesignator) ? $" | LOCK {st.LockedDesignator} ({st.LockHitPercent}%)" : " | TAB: LOCK";
                _arenaDriveStrip.text = $"HDG {hdg:000}° | {st.SpeedKph:0} kph{lockTag}";
            }

            if (_arenaGunneryStrip != null)
            {
                string gunTxt;
                if (_arena.TryGetPrimaryWeaponGunnery(out var primary) && primary.RangeBandTotal > 0)
                {
                    gunTxt = $"BAND {primary.RangeBandIndex}/{primary.RangeBandTotal} · {primary.MaxRangeM:0}m · {primary.DispersionMrad}mr";
                }
                else if (_arena.TryGetPrimaryWeaponGunnery(out var primaryRange) && primaryRange.MaxRangeM > 1f)
                {
                    gunTxt = $"REACH {primaryRange.MaxRangeM:0}m";
                }
                else
                {
                    gunTxt = "GUNNERY READY";
                }
                int shots = st.MatchHits + st.MatchMisses;
                if (shots > 0 || st.MatchKills > 0)
                    gunTxt += $" · {st.MatchKills}K {st.MatchHits}/{shots}H";
                if (st.MovementBlocked)
                    gunTxt += " · [BLOCKED]";
                _arenaGunneryStrip.text = gunTxt;
                _arenaGunneryStrip.color = st.MovementBlocked
                    ? new Color(1.0f, 0.56f, 0.24f)
                    : new Color(0.68f, 0.90f, 0.96f);
            }

            SyncArenaCompassTape();
            SyncArenaCriticalBanner();
            SyncArenaGunnerOpticsReticle(opticsZoomLevel, cupolaView);

            ArenaObjectiveBracketActive = false;
            ArenaDriveWaypointBracketActive = false;
            OffscreenThreatBracketCount = 0;
            if (cam == null)
            {
                HideAllBrackets();
                return;
            }

            int used = 0;

            // 1. Project 3D Hold Objective / Dynamic Objective world bracket (faction-tinted, edge-clamped so always visible)
            if (_arena.TryGetActiveObjectiveWorldPos(out Vector3 objWorldPos, out string objLetter) && used < MaxBrackets)
            {
                Vector3 osp = ProjectToScreen(cam, objWorldPos);
                ComputeCanvasPositionFromScreenPoint(osp, sw, sh, canvasSize, 52f, out Vector2 objPos, out _, out bool objClamped);
                float objDistM = _arena.PlayerVehicleTransform != null
                    ? Vector3.Distance(_arena.PlayerVehicleTransform.position, objWorldPos)
                    : 0f;
                string owner = _arena.ActiveHoldZoneVisualOwner ?? "NEUTRAL";
                Color objCol = owner == "RED"
                    ? new Color(1.0f, 0.34f, 0.28f, 0.96f)
                    : (owner == "BLUE"
                        ? new Color(0.32f, 0.72f, 1.0f, 0.96f)
                        : new Color(1.0f, 0.84f, 0.24f, 0.96f));
                string objPrefix = objClamped ? "» " : string.Empty;
                string objOwnerTag = owner != "NEUTRAL" ? $" · {owner}" : string.Empty;
                string objTag = $"{objPrefix}[{objLetter}] HOLD ZONE{objOwnerTag}";
                ConfigureBracket(
                    _pool[used],
                    objPos,
                    58f,
                    objCol,
                    objTag,
                    $"{objDistM:0} m",
                    string.Empty,
                    isRock: false,
                    isArena: true,
                    interactive: false);
                ArenaObjectiveBracketActive = true;
                used++;
            }

            // 2. Project active Drive Waypoint cyan world bracket (edge-clamped so pilot can always steer toward destination)
            if (_arena.TryGetDriveWaypointWorldPos(out Vector3 wpWorldPos, out string wpLabel) && used < MaxBrackets)
            {
                Vector3 wsp = ProjectToScreen(cam, wpWorldPos);
                ComputeCanvasPositionFromScreenPoint(wsp, sw, sh, canvasSize, 52f, out Vector2 wpPos, out _, out bool wpClamped);
                float wpDistM = _arena.PlayerVehicleTransform != null
                    ? Vector3.Distance(_arena.PlayerVehicleTransform.position, wpWorldPos)
                    : 0f;
                string wpPrefix = wpClamped ? "» " : string.Empty;
                ConfigureBracket(
                    _pool[used],
                    wpPos,
                    52f,
                    new Color(0.28f, 0.94f, 1.0f, 0.95f),
                    $"{wpPrefix}[WP] {wpLabel}",
                    $"{wpDistM:0} m",
                    string.Empty,
                    isRock: false,
                    isArena: true,
                    interactive: false);
                ArenaDriveWaypointBracketActive = true;
                used++;
            }

            // 3. Project locked target (always clamped) + on-screen contacts, while queuing closest off-screen hostile threats
            var offscreenHostiles = new List<(ArenaContactState contact, Vector2 pos, bool isLeftEdge)>(4);

            for (int i = 0; i < st.Contacts.Count && used < MaxBrackets; i++)
            {
                var ct = st.Contacts[i];
                if (ct.IsWreck || string.IsNullOrEmpty(ct.Designator)) continue;
                if (!_arena.TryGetContactWorldPosition(ct.Designator, out Vector3 worldPos)) continue;

                bool isLocked = string.Equals(ct.Designator, st.LockedDesignator, StringComparison.OrdinalIgnoreCase);
                Vector3 sp = ProjectToScreen(cam, worldPos);
                ComputeCanvasPositionFromScreenPoint(sp, sw, sh, canvasSize, 52f, out Vector2 pos, out bool isBehind, out bool clamped);
                bool isOffscreen = isBehind || clamped;

                if (!isLocked && isOffscreen)
                {
                    if (!ct.Friendly)
                        offscreenHostiles.Add((ct, pos, pos.x < 0f));
                    continue;
                }

                Color col = isLocked
                    ? new Color(1.0f, 0.30f, 0.22f, 0.98f)
                    : (ct.Friendly
                        ? new Color(0.28f, 0.88f, 0.66f, 0.90f)
                        : new Color(1.0f, 0.56f, 0.24f, 0.92f));

                string prefix = clamped ? "» " : string.Empty;
                string aspectSuffix = !string.IsNullOrEmpty(ct.Aspect) && !string.Equals(ct.Aspect, "front", StringComparison.OrdinalIgnoreCase)
                    ? $" · {ct.Aspect.ToUpperInvariant()}"
                    : string.Empty;
                string tag = isLocked
                    ? $"{prefix}[{ct.Designator}] HIT {st.LockHitPercent}%{aspectSuffix}"
                    : (!ct.Friendly && ct.HitOddsPct > 0
                        ? $"{prefix}[{ct.Designator}] {ct.HitOddsPct}%{aspectSuffix}"
                        : $"{prefix}[{ct.Designator}] {(string.IsNullOrEmpty(ct.ClassLabel) ? ct.Party : ct.ClassLabel)}");
                string bandPart = !string.IsNullOrEmpty(ct.Band) ? $" {ct.Band.ToUpperInvariant()}" : string.Empty;
                string movePart = ct.Moving ? " · MOV" : string.Empty;
                string sub = $"{ct.RangeM:0}m{bandPart}{movePart}";

                ConfigureBracket(_pool[used], pos, isLocked ? 74f : 54f, col, tag, sub, ct.Designator, isRock: false, isArena: true, interactive: !ct.Friendly);
                used++;
            }

            // 4. Clamp up to 2 closest off-screen non-locked hostile contacts as clickable peripheral threat edge chevrons
            if (offscreenHostiles.Count > 0 && used < MaxBrackets)
            {
                offscreenHostiles.Sort((a, b) => a.contact.RangeM.CompareTo(b.contact.RangeM));
                int maxThreats = Mathf.Min(2, offscreenHostiles.Count);
                for (int t = 0; t < maxThreats && used < MaxBrackets; t++)
                {
                    var item = offscreenHostiles[t];
                    var ct = item.contact;
                    string tag = item.isLeftEdge
                        ? $"« [{ct.Designator}] THREAT"
                        : $"[{ct.Designator}] THREAT »";
                    string sub = ct.HitOddsPct > 0 ? $"{ct.RangeM:0}m · {ct.HitOddsPct}%" : $"{ct.RangeM:0}m";
                    ConfigureBracket(
                        _pool[used],
                        item.pos,
                        42f,
                        new Color(1.0f, 0.36f, 0.22f, 0.94f),
                        tag,
                        sub,
                        ct.Designator,
                        isRock: false,
                        isArena: true,
                        interactive: true);
                    used++;
                    OffscreenThreatBracketCount++;
                }
            }

            HideUnusedBracketsFrom(used);
        }

        private void SyncReplaySpectatorBrackets(Camera cam, float sw, float sh, Vector2 canvasSize)
        {
            ArenaObjectiveBracketActive = false;
            ArenaDriveWaypointBracketActive = false;
            OffscreenThreatBracketCount = 0;
            if (_arena == null || cam == null || _arena.ActiveReplayDoc == null)
            {
                HideAllBrackets();
                return;
            }

            var doc = _arena.ActiveReplayDoc;
            var frame = _arena.LatestReplayFrame;
            int used = 0;

            for (int i = 0; i < doc.Roster.Count && used < MaxBrackets; i++)
            {
                if (!_arena.TryGetReplayRigWorldPosition(i, out Vector3 worldPos)) continue;
                var entry = doc.Roster[i];
                bool focused = i == _arena.ReplayFocusUnitIndex;
                bool wrecked = frame != null && i < frame.IsWrecked.Length && frame.IsWrecked[i];

                Vector3 sp = ProjectToScreen(cam, worldPos);
                ComputeCanvasPositionFromScreenPoint(sp, sw, sh, canvasSize, 52f, out Vector2 pos, out bool isBehind, out bool clamped);
                if (!focused && (isBehind || clamped)) continue;

                Color col = focused
                    ? new Color(1.0f, 0.86f, 0.24f, 0.98f)
                    : (wrecked
                        ? new Color(0.56f, 0.52f, 0.54f, 0.72f)
                        : (entry.IsBlue ? new Color(0.32f, 0.72f, 1.0f, 0.92f) : new Color(1.0f, 0.36f, 0.30f, 0.92f)));

                string prefix = clamped ? "» " : string.Empty;
                string tag = focused
                    ? $"{prefix}[CAM] {entry.Callsign}"
                    : $"{prefix}[{entry.Callsign}]";
                string sub = wrecked
                    ? "WRECK"
                    : (entry.IsBlue ? "BLUE · CLICK CAM" : "RED · CLICK CAM");

                ConfigureBracket(
                    _pool[used],
                    pos,
                    focused ? 68f : 48f,
                    col,
                    tag,
                    sub,
                    entry.Callsign,
                    isRock: false,
                    isArena: true,
                    interactive: true,
                    replayRosterIndex: i);
                used++;
            }

            HideUnusedBracketsFrom(used);
        }

        private void ConfigureBracket(
            PooledBracket b,
            Vector2 anchoredPos,
            float sizePx,
            Color color,
            string topTag,
            string subTag,
            string boundId,
            bool isRock,
            bool isArena,
            bool interactive,
            int replayRosterIndex = -1)
        {
            b.BoundId = boundId;
            b.BoundIsRock = isRock;
            b.BoundIsArena = isArena;
            b.BoundReplayRosterIndex = replayRosterIndex;
            b.Rect.anchoredPosition = anchoredPos;
            b.Rect.sizeDelta = new Vector2(sizePx, sizePx);
            b.Button.interactable = interactive && (replayRosterIndex >= 0 || !string.IsNullOrEmpty(boundId));
            b.CenterFill.raycastTarget = b.Button.interactable;
            b.CenterFill.color = new Color(color.r, color.g, color.b, interactive ? 0.05f : 0.01f);

            for (int i = 0; i < b.CornerStrokes.Length; i++)
                b.CornerStrokes[i].color = color;

            b.TagLabel.text = topTag;
            b.TagLabel.color = color;
            b.SubLabel.text = subTag;
            b.SubLabel.color = new Color(0.86f, 0.94f, 1f, 0.92f);

            if (!b.Root.activeSelf)
                b.Root.SetActive(true);
        }

        private void HideAllBrackets() => HideUnusedBracketsFrom(0);

        private void HideUnusedBracketsFrom(int startIdx)
        {
            ActiveBracketCount = startIdx;
            for (int i = startIdx; i < _pool.Count; i++)
            {
                if (_pool[i].Root != null && _pool[i].Root.activeSelf)
                    _pool[i].Root.SetActive(false);
            }
        }

        private PooledBracket CreatePooledBracket(Transform parent, int index)
        {
            var b = new PooledBracket();
            var go = new GameObject($"TacBracket_{index}", typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            b.Root = go;
            b.Rect = go.GetComponent<RectTransform>();
            b.Rect.anchorMin = new Vector2(0.5f, 0.5f);
            b.Rect.anchorMax = new Vector2(0.5f, 0.5f);
            b.Rect.pivot = new Vector2(0.5f, 0.5f);
            b.Rect.sizeDelta = new Vector2(56f, 56f);

            b.CenterFill = go.GetComponent<Image>();
            b.CenterFill.color = new Color(0f, 0f, 0f, 0.01f);
            b.CenterFill.raycastTarget = true;

            b.Button = go.GetComponent<Button>();
            b.Button.targetGraphic = b.CenterFill;
            b.Button.onClick.AddListener(() =>
            {
                if (b.BoundReplayRosterIndex >= 0)
                {
                    OnReplayBracketClicked?.Invoke(b.BoundReplayRosterIndex);
                    return;
                }
                if (string.IsNullOrEmpty(b.BoundId)) return;
                if (b.BoundIsArena) OnArenaBracketClicked?.Invoke(b.BoundId);
                else OnOrbitalBracketClicked?.Invoke(b.BoundId, b.BoundIsRock);
            });

            // 4 crisp sci-fi L-corners (8 thin strokes)
            const float L = 12f;
            const float T = 2.5f;
            b.CornerStrokes[0] = CreateStroke(go.transform, new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(0f, -T), new Vector2(L, 0f));
            b.CornerStrokes[1] = CreateStroke(go.transform, new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(0f, -L), new Vector2(T, 0f));
            b.CornerStrokes[2] = CreateStroke(go.transform, new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-L, -T), new Vector2(0f, 0f));
            b.CornerStrokes[3] = CreateStroke(go.transform, new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-T, -L), new Vector2(0f, 0f));
            b.CornerStrokes[4] = CreateStroke(go.transform, new Vector2(0f, 0f), new Vector2(0f, 0f), new Vector2(0f, 0f), new Vector2(L, T));
            b.CornerStrokes[5] = CreateStroke(go.transform, new Vector2(0f, 0f), new Vector2(0f, 0f), new Vector2(0f, 0f), new Vector2(T, L));
            b.CornerStrokes[6] = CreateStroke(go.transform, new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-L, 0f), new Vector2(0f, T));
            b.CornerStrokes[7] = CreateStroke(go.transform, new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-T, 0f), new Vector2(0f, L));

            b.TagLabel = CreateText(go.transform, string.Empty, 11, FontStyle.Bold, TextAnchor.LowerCenter, Color.cyan);
            SetRect(b.TagLabel.rectTransform, new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-90f, 3f), new Vector2(90f, 20f));

            b.SubLabel = CreateText(go.transform, string.Empty, 10, FontStyle.Normal, TextAnchor.UpperCenter, Color.white);
            SetRect(b.SubLabel.rectTransform, new Vector2(0.5f, 0f), new Vector2(0.5f, 0f), new Vector2(-70f, -17f), new Vector2(70f, -2f));

            go.SetActive(false);
            return b;
        }

        private void BuildSpaceCenterReticle(Transform parent)
        {
            _spaceReticleWidget = new GameObject("SpaceFlightCenterReticle", typeof(RectTransform));
            _spaceReticleWidget.transform.SetParent(parent, false);
            var rt = _spaceReticleWidget.GetComponent<RectTransform>();
            SetRect(rt, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-24f, -24f), new Vector2(24f, 24f));

            Color pip = new Color(0.42f, 0.88f, 1.0f, 0.45f);
            CreateStroke(_spaceReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-20f, -1f), new Vector2(-7f, 1f), pip);
            CreateStroke(_spaceReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(7f, -1f), new Vector2(20f, 1f), pip);
            CreateStroke(_spaceReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-1f, 7f), new Vector2(1f, 20f), pip);
            CreateStroke(_spaceReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-1f, -20f), new Vector2(1f, -7f), pip);
            _spaceReticleWidget.SetActive(false);
        }

        private void BuildDockingAlignmentReticle(Transform parent)
        {
            _dockingReticleWidget = new GameObject("DockingAlignmentReticle", typeof(RectTransform));
            _dockingReticleWidget.transform.SetParent(parent, false);
            var rt = _dockingReticleWidget.GetComponent<RectTransform>();
            SetRect(rt, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-130f, -130f), new Vector2(130f, 130f));

            // Center collar tolerance box (±26px = ±150 mm)
            Color boxCol = new Color(0.34f, 0.88f, 1.0f, 0.58f);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-26f, 25f), new Vector2(26f, 27f), boxCol);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-26f, -27f), new Vector2(26f, -25f), boxCol);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-27f, -26f), new Vector2(-25f, 26f), boxCol);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(25f, -26f), new Vector2(27f, 26f), boxCol);

            // Outer reference axes
            Color axisCol = new Color(0.32f, 0.74f, 0.88f, 0.30f);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-115f, -1f), new Vector2(-32f, 1f), axisCol);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(32f, -1f), new Vector2(115f, 1f), axisCol);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-1f, 32f), new Vector2(1f, 115f), axisCol);
            CreateStroke(_dockingReticleWidget.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-1f, -115f), new Vector2(1f, -32f), axisCol);

            // Live rotating Roll Horizon Wing Bar
            var rollGo = new GameObject("RollHorizonBar", typeof(RectTransform), typeof(Image));
            rollGo.transform.SetParent(_dockingReticleWidget.transform, false);
            _dockRollBar = rollGo.GetComponent<RectTransform>();
            SetRect(_dockRollBar, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-72f, -2f), new Vector2(72f, 2f));
            _dockRollBarImg = rollGo.GetComponent<Image>();
            _dockRollBarImg.color = new Color(1.0f, 0.72f, 0.22f, 0.85f);
            _dockRollBarImg.raycastTarget = false;

            // Live Lateral Offset Pip (Dx, Dy)
            var latGo = new GameObject("LateralOffsetPip", typeof(RectTransform), typeof(Image));
            latGo.transform.SetParent(_dockingReticleWidget.transform, false);
            _dockLateralPip = latGo.GetComponent<RectTransform>();
            SetRect(_dockLateralPip, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-7f, -7f), new Vector2(7f, 7f));
            _dockLateralPip.localRotation = Quaternion.Euler(0f, 0f, 45f);
            _dockLateralPipImg = latGo.GetComponent<Image>();
            _dockLateralPipImg.color = new Color(1.0f, 0.64f, 0.20f, 0.95f);
            _dockLateralPipImg.raycastTarget = false;

            // Live Pitch/Yaw Attitude Vector Pip
            var attGo = new GameObject("AttitudeVectorPip", typeof(RectTransform), typeof(Image));
            attGo.transform.SetParent(_dockingReticleWidget.transform, false);
            _dockAttitudePip = attGo.GetComponent<RectTransform>();
            SetRect(_dockAttitudePip, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-4f, -4f), new Vector2(4f, 4f));
            _dockAttitudePipImg = attGo.GetComponent<Image>();
            _dockAttitudePipImg.color = new Color(0.38f, 0.92f, 1.0f, 0.95f);
            _dockAttitudePipImg.raycastTarget = false;

            // Vertical Closure Velocity Gauge on right edge of reticle (100..800 mm/s green zone)
            var clBg = CreatePanel(_dockingReticleWidget.transform, "ClosureBarBg", new Vector2(1f, 0.5f), new Vector2(1f, 0.5f), new Vector2(10f, -70f), new Vector2(20f, 70f), new Color(0.06f, 0.11f, 0.17f, 0.88f));
            clBg.GetComponent<Image>().raycastTarget = false;
            var clFill = CreatePanel(clBg.transform, "ClosureBarFill", new Vector2(0f, 0f), new Vector2(1f, 0.35f), Vector2.zero, Vector2.zero, new Color(0.24f, 0.92f, 0.56f, 0.95f));
            _dockClosureFill = clFill.GetComponent<Image>();
            _dockClosureFill.raycastTarget = false;

            _dockStatusText = CreateText(_dockingReticleWidget.transform, "6-DOF COLLAR ALIGNMENT  |  0/5 GATES", 11, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.75f, 0.95f, 1f));
            SetRect(_dockStatusText.rectTransform, new Vector2(0.5f, 0f), new Vector2(0.5f, 0f), new Vector2(-170f, -24f), new Vector2(170f, -4f));

            _dockingReticleWidget.SetActive(false);
        }

        private void SyncDockingAlignmentReticle(DockingClientSession session)
        {
            if (session == null) return;
            var gates = session.EvaluateGates();
            DockingReticleAllGreen = gates.AllGreen;

            ComputeDockingReticleOffsets(
                session.DxMm,
                session.DyMm,
                session.YawMdeg,
                session.PitchMdeg,
                session.RollMdeg,
                session.VzMmps,
                out Vector2 latPx,
                out Vector2 attPx,
                out float rollDeg,
                out float closureNorm);

            DockingRollBarAngleDeg = rollDeg;

            Color green = new Color(0.22f, 1.0f, 0.56f, 0.96f);
            Color amber = new Color(1.0f, 0.62f, 0.18f, 0.96f);
            Color cyan = new Color(0.38f, 0.90f, 1.0f, 0.95f);

            if (_dockLateralPip != null) _dockLateralPip.anchoredPosition = latPx;
            if (_dockLateralPipImg != null) _dockLateralPipImg.color = gates.LateralOk ? green : amber;

            if (_dockAttitudePip != null) _dockAttitudePip.anchoredPosition = attPx;
            if (_dockAttitudePipImg != null) _dockAttitudePipImg.color = gates.AttitudeOk ? green : cyan;

            if (_dockRollBar != null) _dockRollBar.localRotation = Quaternion.Euler(0f, 0f, rollDeg);
            if (_dockRollBarImg != null) _dockRollBarImg.color = gates.RollOk ? green : amber;

            if (_dockClosureFill != null)
            {
                _dockClosureFill.rectTransform.anchorMax = new Vector2(1f, Mathf.Max(0.03f, closureNorm));
                _dockClosureFill.color = gates.ClosureOk ? green : amber;
            }

            if (_dockStatusText != null)
            {
                _dockStatusText.text = $"GATES {gates.GreenCount}/5  |  Z {session.DzMm / 1000f:0.00} m  |  Vz {session.VzMmps} mm/s  |  ROLL {session.RollMdeg / 1000f:0.0}°";
                _dockStatusText.color = gates.AllGreen ? green : new Color(0.84f, 0.95f, 1f);
            }
        }

        private void BuildSurfaceCompassStrip(Transform parent)
        {
            _surfaceCompassWidget = CreatePanel(
                parent,
                "SurfaceRoverCompassStrip",
                new Vector2(0.5f, 0f),
                new Vector2(0.5f, 0f),
                new Vector2(-215f, 72f),
                new Vector2(215f, 102f),
                new Color(0.03f, 0.07f, 0.11f, 0.88f));

            var dotGo = CreatePanel(_surfaceCompassWidget.transform, "DrillLed", new Vector2(0f, 0.5f), new Vector2(0f, 0.5f), new Vector2(10f, -5f), new Vector2(20f, 5f), new Color(0.24f, 0.84f, 0.58f, 1f));
            _surfaceDrillIndicator = dotGo.GetComponent<Image>();

            _surfaceCompassText = CreateText(_surfaceCompassWidget.transform, "HDG 180°  |  0 kph  |  CELL (128,124)  |  DRILL READY", 12, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.84f, 0.95f, 1f));
            SetRect(_surfaceCompassText.rectTransform, Vector2.zero, Vector2.one, new Vector2(26f, 0f), new Vector2(-10f, 0f));

            _surfaceCompassWidget.SetActive(false);
        }

        private void BuildArenaArmorSchematic(Transform parent)
        {
            // Sits cleanly at bottom-center between CommsDrawer (x=14..436) and OwnPlatformCard (x=-430..-16)
            _arenaArmorWidget = CreatePanel(
                parent,
                "ArenaArmorSchematic",
                new Vector2(0.5f, 0f),
                new Vector2(0.5f, 0f),
                new Vector2(-128f, 18f),
                new Vector2(128f, 190f),
                new Color(0.03f, 0.06f, 0.11f, 0.92f));

            var hdr = CreateText(_arenaArmorWidget.transform, "ARMOR INTEGRITY & HEAT", 10, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.62f, 0.88f, 1f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(6f, -18f), new Vector2(-6f, -2f));

            // 4-quadrant directional armor plates + internals core
            BuildArmorPlate(_arenaArmorWidget.transform, "FrontPlate", new Vector2(0.5f, 1f), new Vector2(-42f, -44f), new Vector2(42f, -22f), out _armorFrontImg, out _armorFrontTxt);
            BuildArmorPlate(_arenaArmorWidget.transform, "LeftPlate", new Vector2(0.5f, 1f), new Vector2(-92f, -84f), new Vector2(-44f, -48f), out _armorLeftImg, out _armorLeftTxt);
            BuildArmorPlate(_arenaArmorWidget.transform, "CorePlate", new Vector2(0.5f, 1f), new Vector2(-40f, -84f), new Vector2(40f, -48f), out _armorCoreImg, out _armorCoreTxt);
            BuildArmorPlate(_arenaArmorWidget.transform, "RightPlate", new Vector2(0.5f, 1f), new Vector2(44f, -84f), new Vector2(92f, -48f), out _armorRightImg, out _armorRightTxt);
            BuildArmorPlate(_arenaArmorWidget.transform, "RearPlate", new Vector2(0.5f, 1f), new Vector2(-42f, -110f), new Vector2(42f, -88f), out _armorRearImg, out _armorRearTxt);

            // Heat bar
            var heatBg = CreatePanel(_arenaArmorWidget.transform, "HeatBarBg", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(12f, 40f), new Vector2(-12f, 56f), new Color(0.10f, 0.14f, 0.20f, 1f));
            var heatFill = CreatePanel(heatBg.transform, "HeatBarFill", Vector2.zero, new Vector2(0.05f, 1f), Vector2.zero, Vector2.zero, new Color(0.24f, 0.76f, 0.95f, 0.95f));
            _arenaHeatFill = heatFill.GetComponent<Image>();
            _arenaHeatLabel = CreateText(heatBg.transform, "HEAT 0/1000", 10, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(_arenaHeatLabel.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            _arenaDriveStrip = CreateText(_arenaArmorWidget.transform, "HDG 090° | 0 kph | TAB: LOCK", 10, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.84f, 0.94f, 1f));
            SetRect(_arenaDriveStrip.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(6f, 20f), new Vector2(-6f, 37f));

            _arenaGunneryStrip = CreateText(_arenaArmorWidget.transform, "GUNNERY READY", 9, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.68f, 0.90f, 0.96f));
            SetRect(_arenaGunneryStrip.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(6f, 3f), new Vector2(-6f, 19f));

            _arenaArmorWidget.SetActive(false);
        }

        private void BuildArenaCompassTape(Transform parent)
        {
            // Sits cleanly below RedVsBlueScoreboard (y = -118..-68) and above ArenaBanner (-206..-166)
            _arenaCompassWidget = CreatePanel(
                parent,
                "ArenaCompassTape",
                new Vector2(0.5f, 1f),
                new Vector2(0.5f, 1f),
                new Vector2(-ArenaCompassWidthPx * 0.5f, -160f),
                new Vector2(ArenaCompassWidthPx * 0.5f, -122f),
                new Color(0.03f, 0.06f, 0.11f, 0.90f));
            _arenaCompassWidget.GetComponent<Image>().raycastTarget = false;

            // Subtle top & bottom horizon hairlines + center bearing notch
            CreateStroke(_arenaCompassWidget.transform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(4f, -2f), new Vector2(-4f, 0f), new Color(0.28f, 0.72f, 0.90f, 0.48f));
            CreateStroke(_arenaCompassWidget.transform, new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-1f, -14f), new Vector2(1f, 0f), new Color(0.45f, 0.95f, 1f, 0.92f));

            // 8 pooled cardinal/intercardinal labels scrolling smoothly along top track
            for (int i = 0; i < CompassTickNames.Length; i++)
            {
                bool cardinal = i % 2 == 0;
                var lbl = CreateText(
                    _arenaCompassWidget.transform,
                    CompassTickNames[i],
                    cardinal ? 11 : 10,
                    cardinal ? FontStyle.Bold : FontStyle.Normal,
                    TextAnchor.MiddleCenter,
                    cardinal ? new Color(0.84f, 0.96f, 1f, 0.95f) : new Color(0.54f, 0.72f, 0.84f, 0.80f));
                SetRect(lbl.rectTransform, new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-18f, -20f), new Vector2(18f, -4f));
                _compassTickRects[i] = lbl.rectTransform;
                _compassTickLabels[i] = lbl;
            }

            // Ordered Helm Bug chevron (cyan)
            var bugGo = CreatePanel(_arenaCompassWidget.transform, "CompassHelmBug", new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-4f, -9f), new Vector2(4f, -1f), new Color(0.28f, 0.96f, 1f, 0.95f));
            bugGo.GetComponent<Image>().raycastTarget = false;
            _compassOrderBugPip = bugGo.GetComponent<RectTransform>();
            _compassOrderBugPip.localRotation = Quaternion.Euler(0f, 0f, 45f);
            bugGo.SetActive(false);

            // Locked Hostile Target diamond (crimson)
            var lockGo = CreatePanel(_arenaCompassWidget.transform, "CompassLockPip", new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-5f, -19f), new Vector2(5f, -9f), new Color(1.0f, 0.28f, 0.20f, 0.98f));
            lockGo.GetComponent<Image>().raycastTarget = false;
            _compassLockPip = lockGo.GetComponent<RectTransform>();
            _compassLockPip.localRotation = Quaternion.Euler(0f, 0f, 45f);
            lockGo.SetActive(false);

            // Active Objective / Hold Zone marker (gold)
            var objGo = CreatePanel(_arenaCompassWidget.transform, "CompassObjectivePip", new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-9f, -20f), new Vector2(9f, -4f), new Color(0.95f, 0.76f, 0.16f, 0.94f));
            objGo.GetComponent<Image>().raycastTarget = false;
            _compassObjectivePip = objGo.GetComponent<RectTransform>();
            _compassObjectivePipLabel = CreateText(objGo.transform, "B", 10, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.06f, 0.08f, 0.10f, 1f));
            SetRect(_compassObjectivePipLabel.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
            objGo.SetActive(false);

            // Active Drive Waypoint marker (cyan)
            var wpGo = CreatePanel(_arenaCompassWidget.transform, "CompassWaypointPip", new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-8f, -20f), new Vector2(8f, -5f), new Color(0.24f, 0.92f, 1.0f, 0.96f));
            wpGo.GetComponent<Image>().raycastTarget = false;
            _compassWaypointPip = wpGo.GetComponent<RectTransform>();
            var wpLbl = CreateText(wpGo.transform, "W", 9, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.03f, 0.08f, 0.12f, 1f));
            SetRect(wpLbl.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
            wpGo.SetActive(false);

            // Center digital heading readout pill + telemetry line below
            var hdgBg = CreatePanel(_arenaCompassWidget.transform, "CompassCenterHeadingBg", new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-26f, -21f), new Vector2(26f, -3f), new Color(0.06f, 0.14f, 0.22f, 0.96f));
            hdgBg.GetComponent<Image>().raycastTarget = false;
            _arenaCompassHeadingTxt = CreateText(hdgBg.transform, "090°", 11, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.42f, 0.96f, 1f));
            SetRect(_arenaCompassHeadingTxt.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            _arenaCompassTelemetryTxt = CreateText(_arenaCompassWidget.transform, "SCOPE CLEAR  |  TEMPO ALERT  |  SIGHT 120m  |  RCVR 100%", 10, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.74f, 0.90f, 0.98f));
            SetRect(_arenaCompassTelemetryTxt.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(6f, 2f), new Vector2(-6f, 16f));

            _arenaCompassWidget.SetActive(false);
        }

        private void SyncArenaCompassTape()
        {
            if (_arenaCompassWidget == null || _arena == null || _arena.State == null) return;
            var st = _arena.State;
            float centerHdg = ((_arena.PredictedHeadingDeg % 360f) + 360f) % 360f;

            if (_arenaCompassHeadingTxt != null)
                _arenaCompassHeadingTxt.text = $"{Mathf.RoundToInt(centerHdg) % 360:000}°";

            for (int i = 0; i < _compassTickRects.Length; i++)
            {
                var rt = _compassTickRects[i];
                if (rt == null) continue;
                float xOff = ComputeCompassTapeOffsetPx(centerHdg, CompassTickBearings[i], ArenaCompassWidthPx, ArenaCompassFovDeg, out bool visible);
                if (rt.gameObject.activeSelf != visible) rt.gameObject.SetActive(visible);
                if (visible)
                    rt.anchoredPosition = new Vector2(xOff, -12f);
            }

            // Ordered Helm Bug (shown whenever an ordered helm heading differs from current heading)
            if (_compassOrderBugPip != null)
            {
                bool showBug = _arena.Helm != null && _arena.Helm.OrderedMdeg.HasValue;
                float bugDeg = showBug ? (_arena.Helm.OrderedMdeg.Value * 0.001f) : centerHdg;
                if (showBug && Mathf.Abs(Mathf.DeltaAngle(centerHdg, bugDeg)) < 1.5f)
                    showBug = false;
                if (_compassOrderBugPip.gameObject.activeSelf != showBug)
                    _compassOrderBugPip.gameObject.SetActive(showBug);
                if (showBug)
                {
                    float xOff = ComputeCompassTapeOffsetPx(centerHdg, bugDeg, ArenaCompassWidthPx, ArenaCompassFovDeg, out _);
                    _compassOrderBugPip.anchoredPosition = new Vector2(xOff, -5f);
                }
            }

            // Locked Hostile Contact Pip
            if (_compassLockPip != null)
            {
                bool hasLock = _arena.TryGetLockedContactBearingDeg(out float lockBearing);
                if (_compassLockPip.gameObject.activeSelf != hasLock)
                    _compassLockPip.gameObject.SetActive(hasLock);
                if (hasLock)
                {
                    float xOff = ComputeCompassTapeOffsetPx(centerHdg, lockBearing, ArenaCompassWidthPx, ArenaCompassFovDeg, out _);
                    _compassLockPip.anchoredPosition = new Vector2(xOff, -14f);
                }
            }

            // Active Dynamic Objective / Hold Spot Pip
            if (_compassObjectivePip != null)
            {
                bool hasObj = _arena.TryGetActiveObjectiveBearingDeg(out float objBearing, out _, out string objLabel);
                if (_compassObjectivePip.gameObject.activeSelf != hasObj)
                    _compassObjectivePip.gameObject.SetActive(hasObj);
                if (hasObj)
                {
                    float xOff = ComputeCompassTapeOffsetPx(centerHdg, objBearing, ArenaCompassWidthPx, ArenaCompassFovDeg, out _);
                    _compassObjectivePip.anchoredPosition = new Vector2(xOff, -12f);
                    if (_compassObjectivePipLabel != null)
                        _compassObjectivePipLabel.text = string.IsNullOrEmpty(objLabel) ? "O" : objLabel;
                }
            }

            // Active Drive Waypoint Pip (cyan "W")
            if (_compassWaypointPip != null)
            {
                bool hasWp = _arena.TryGetDriveWaypointBearingDeg(out float wpBearing, out _, out _);
                if (_compassWaypointPip.gameObject.activeSelf != hasWp)
                    _compassWaypointPip.gameObject.SetActive(hasWp);
                if (hasWp)
                {
                    float xOff = ComputeCompassTapeOffsetPx(centerHdg, wpBearing, ArenaCompassWidthPx, ArenaCompassFovDeg, out _);
                    _compassWaypointPip.anchoredPosition = new Vector2(xOff, -12f);
                }
            }

            if (_arenaCompassTelemetryTxt != null)
            {
                string scopeUp = (st.Scope ?? "clear").ToUpperInvariant();
                string tempoUp = (st.Tempo ?? "alert").ToUpperInvariant();
                string postureUp = (st.Posture ?? "return").ToUpperInvariant();
                string rcvrTag = st.HasReceiver ? $" | RCVR {st.ReceiverReachPercent}%" : string.Empty;

                var ti = _arena.CurrentTerrainIntel;
                string coverPart = !string.IsNullOrEmpty(ti.CoverLabel) ? ti.CoverLabel : "OPEN";
                string elevPart = string.Empty;
                if (ti.HasTargetSample)
                {
                    if (ti.ElevationDeltaM >= 3.0f) elevPart = $" HIGH +{Mathf.RoundToInt(ti.ElevationDeltaM)}m";
                    else if (ti.ElevationDeltaM <= -3.0f) elevPart = $" LOW {Mathf.RoundToInt(ti.ElevationDeltaM)}m";
                }
                string losPart = (ti.HasTargetSample && ti.IsLineOfSightObstructed && !string.IsNullOrEmpty(ti.LineOfSightStatus))
                    ? $" · {ti.LineOfSightStatus} LOS"
                    : string.Empty;
                ArenaTerrainBadgeSnapshot = $"{coverPart}{elevPart}{losPart}";

                _arenaCompassTelemetryTxt.text = $"SCOPE {scopeUp} | SIGHT {st.SightMeters:0}m | {postureUp}{rcvrTag} | {ArenaTerrainBadgeSnapshot}";
                _arenaCompassTelemetryTxt.color = string.Equals(st.Scope, "hostile", StringComparison.OrdinalIgnoreCase)
                    ? new Color(1.0f, 0.44f, 0.36f)
                    : (string.Equals(st.Scope, "unidentified", StringComparison.OrdinalIgnoreCase) || (st.HasReceiver && st.ReceiverReachPercent < 85)
                        ? new Color(1.0f, 0.80f, 0.34f)
                        : new Color(0.68f, 0.92f, 1.0f));
            }
        }

        private void BuildArenaCombatFeedbackWidgets(Transform parent)
        {
            // 1. Dynamic 4-stroke screen-center hitmarker
            _arenaHitmarkerRoot = new GameObject("ArenaCenterHitmarker", typeof(RectTransform));
            _arenaHitmarkerRoot.transform.SetParent(parent, false);
            var hrt = _arenaHitmarkerRoot.GetComponent<RectTransform>();
            SetRect(hrt, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-44f, -44f), new Vector2(44f, 44f));

            float[] angles = { 45f, 135f, 225f, 315f };
            for (int i = 0; i < 4; i++)
            {
                var tickGo = new GameObject($"HitTick_{i}", typeof(RectTransform), typeof(Image));
                tickGo.transform.SetParent(_arenaHitmarkerRoot.transform, false);
                var trt = tickGo.GetComponent<RectTransform>();
                trt.anchorMin = new Vector2(0.5f, 0.5f);
                trt.anchorMax = new Vector2(0.5f, 0.5f);
                trt.pivot = new Vector2(0.5f, 0.5f);
                trt.sizeDelta = new Vector2(13f, 2.5f);
                trt.localRotation = Quaternion.Euler(0f, 0f, angles[i]);
                var img = tickGo.GetComponent<Image>();
                img.color = Color.white;
                img.raycastTarget = false;
                _hitmarkerTicks[i] = img;
                _hitmarkerTickRects[i] = trt;
            }
            _arenaHitmarkerRoot.SetActive(false);

            // 2. 4-Quadrant Directional Incoming-Hit Damage Arcs (non-blocking perimeter flash strips)
            _arenaDamageArcsRoot = new GameObject("ArenaDirectionalDamageArcs", typeof(RectTransform));
            _arenaDamageArcsRoot.transform.SetParent(parent, false);
            SetRect(_arenaDamageArcsRoot.GetComponent<RectTransform>(), Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            _dmgArcFront = CreateStroke(_arenaDamageArcsRoot.transform, new Vector2(0.22f, 1f), new Vector2(0.78f, 1f), new Vector2(0f, -14f), Vector2.zero, Color.clear);
            _dmgArcRear = CreateStroke(_arenaDamageArcsRoot.transform, new Vector2(0.22f, 0f), new Vector2(0.78f, 0f), Vector2.zero, new Vector2(0f, 14f), Color.clear);
            _dmgArcLeft = CreateStroke(_arenaDamageArcsRoot.transform, new Vector2(0f, 0.22f), new Vector2(0f, 0.78f), Vector2.zero, new Vector2(14f, 0f), Color.clear);
            _dmgArcRight = CreateStroke(_arenaDamageArcsRoot.transform, new Vector2(1f, 0.22f), new Vector2(1f, 0.78f), new Vector2(-14f, 0f), Vector2.zero, Color.clear);
            _arenaDamageArcsRoot.SetActive(false);

            // 3. Thermal Overheat / Critical Hull Warning Bar (sits cleanly below compass tape at y=-192..-164)
            _arenaCritBannerRoot = CreatePanel(
                parent,
                "ArenaCriticalStatusBanner",
                new Vector2(0.5f, 1f),
                new Vector2(0.5f, 1f),
                new Vector2(-210f, -192f),
                new Vector2(210f, -164f),
                new Color(0.58f, 0.12f, 0.10f, 0.90f));
            _arenaCritBannerBg = _arenaCritBannerRoot.GetComponent<Image>();
            _arenaCritBannerBg.raycastTarget = false;
            _arenaCritBannerText = CreateText(_arenaCritBannerRoot.transform, string.Empty, 11, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(1f, 0.92f, 0.78f));
            SetRect(_arenaCritBannerText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-8f, 0f));
            _arenaCritBannerRoot.SetActive(false);
        }

        private void SyncArenaCriticalBanner()
        {
            if (_arenaCritBannerRoot == null || _arena?.State == null) return;
            var st = _arena.State;
            string msg = string.Empty;
            Color bg = new Color(0.58f, 0.12f, 0.10f, 0.90f);

            if (st.IsWreck)
            {
                msg = "VEHICLE DESTROYED — EXIT POD OR WATCH MATCH";
                bg = new Color(0.52f, 0.08f, 0.10f, 0.94f);
            }
            else if (st.Overheated)
            {
                msg = $"THERMAL OVERHEAT ({st.Heat}/{st.HeatMax}) — VENTING HEAT";
                bg = new Color(0.68f, 0.22f, 0.08f, 0.92f);
            }
            else if (st.Immobile)
            {
                msg = "DRIVETRAIN DISABLED — HULL IMMOBILE";
                bg = new Color(0.62f, 0.30f, 0.08f, 0.92f);
            }
            else if (st.InternalsPct < 40)
            {
                msg = $"CRITICAL INTERNAL DAMAGE ({st.InternalsPct}% STRUCTURE)";
                bg = new Color(0.62f, 0.10f, 0.10f, 0.92f);
            }

            bool show = !string.IsNullOrEmpty(msg);
            if (_arenaCritBannerRoot.activeSelf != show)
                _arenaCritBannerRoot.SetActive(show);
            if (show)
            {
                float pulse = 0.80f + 0.20f * Mathf.Sin(Time.unscaledTime * 6.5f);
                if (_arenaCritBannerBg != null) _arenaCritBannerBg.color = new Color(bg.r, bg.g, bg.b, bg.a * pulse);
                if (_arenaCritBannerText != null) _arenaCritBannerText.text = msg;
            }
        }

        private void BuildArenaGunnerOpticsReticle(Transform parent)
        {
            _opticsFrameStrokes.Clear();
            _arenaOpticsReticleRoot = new GameObject("ArenaGunnerOpticsReticle", typeof(RectTransform));
            _arenaOpticsReticleRoot.transform.SetParent(parent, false);
            var rt = _arenaOpticsReticleRoot.GetComponent<RectTransform>();
            SetRect(rt, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-190f, -134f), new Vector2(190f, 134f));

            Color strokeCol = new Color(0.32f, 0.92f, 1.0f, 0.55f);

            // Outer optical visor corner L-brackets (non-blocking)
            const float cLen = 28f;
            const float cThick = 2f;
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(0f, -cThick), new Vector2(cLen, 0f), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(0f, -cLen), new Vector2(cThick, 0f), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-cLen, -cThick), Vector2.zero, strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-cThick, -cLen), Vector2.zero, strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0f, 0f), new Vector2(0f, 0f), Vector2.zero, new Vector2(cLen, cThick), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0f, 0f), new Vector2(0f, 0f), Vector2.zero, new Vector2(cThick, cLen), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-cLen, 0f), new Vector2(0f, cThick), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-cThick, 0f), new Vector2(0f, cLen), strokeCol));

            // Center open-aperture mil-dot crosshairs + ballistic drop stadia ticks
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-92f, -1f), new Vector2(-14f, 1f), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(14f, -1f), new Vector2(92f, 1f), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-1f, 14f), new Vector2(1f, 54f), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-1f, -68f), new Vector2(1f, -14f), strokeCol));

            // Stadia range drop ticks along bottom vertical hair
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-18f, -28f), new Vector2(18f, -26f), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-12f, -44f), new Vector2(12f, -42f), strokeCol));
            _opticsFrameStrokes.Add(CreateStroke(_arenaOpticsReticleRoot.transform, new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-7f, -60f), new Vector2(7f, -58f), strokeCol));

            // Top-left magnification badge & top-right weapon readiness readout
            _opticsModeLabel = CreateText(_arenaOpticsReticleRoot.transform, "OPTICS 2.5x · GUNNER", 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.96f, 1f));
            SetRect(_opticsModeLabel.rectTransform, new Vector2(0f, 1f), new Vector2(0.56f, 1f), new Vector2(8f, -22f), new Vector2(0f, -4f));

            _opticsWeaponStatusLabel = CreateText(_arenaOpticsReticleRoot.transform, "RDY · MAIN GUN", 11, FontStyle.Bold, TextAnchor.MiddleRight, new Color(0.38f, 0.98f, 0.64f));
            SetRect(_opticsWeaponStatusLabel.rectTransform, new Vector2(0.44f, 1f), new Vector2(1f, 1f), new Vector2(0f, -22f), new Vector2(-8f, -4f));

            // Bottom-center live laser rangefinder & firing solution bar
            var rfBar = CreatePanel(_arenaOpticsReticleRoot.transform, "OpticsRangefinderBar", new Vector2(0.5f, 0f), new Vector2(0.5f, 0f), new Vector2(-186f, 6f), new Vector2(186f, 26f), new Color(0.02f, 0.06f, 0.10f, 0.78f));
            _opticsRangefinderBg = rfBar.GetComponent<Image>();
            _opticsRangefinderBg.raycastTarget = false;
            _opticsRangefinderLabel = CreateText(rfBar.transform, "NO LOCK · TAB TO ACQUIRE", 10, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.74f, 0.94f, 1f));
            SetRect(_opticsRangefinderLabel.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 0f), new Vector2(-6f, 0f));

            _arenaOpticsReticleRoot.SetActive(false);
        }

        private void SyncArenaGunnerOpticsReticle(int opticsZoomLevel, bool cupolaView)
        {
            if (_arenaOpticsReticleRoot == null) return;
            bool show = _arena != null && _arena.State != null && (opticsZoomLevel > 0 || cupolaView);
            if (_arenaOpticsReticleRoot.activeSelf != show)
                _arenaOpticsReticleRoot.SetActive(show);
            if (!show) return;

            var st = _arena.State;
            var ti = _arena.CurrentTerrainIntel;

            if (_opticsModeLabel != null)
            {
                if (opticsZoomLevel >= 2)
                    _opticsModeLabel.text = cupolaView ? "OPTICS 4.0x · CUPOLA" : "OPTICS 4.0x · SNIPER";
                else if (opticsZoomLevel == 1)
                    _opticsModeLabel.text = cupolaView ? "OPTICS 2.5x · CUPOLA" : "OPTICS 2.5x · GUNNER";
                else
                    _opticsModeLabel.text = "CUPOLA VISOR 1.0x";
            }

            if (_opticsWeaponStatusLabel != null)
            {
                if (st.IsWreck)
                {
                    _opticsWeaponStatusLabel.text = "HULL WRECKED";
                    _opticsWeaponStatusLabel.color = new Color(1.0f, 0.30f, 0.24f);
                }
                else if (st.Overheated)
                {
                    _opticsWeaponStatusLabel.text = $"OVERHEAT {st.Heat}/{st.HeatMax}";
                    _opticsWeaponStatusLabel.color = new Color(1.0f, 0.34f, 0.22f);
                }
                else if (!st.WeaponsFree)
                {
                    _opticsWeaponStatusLabel.text = "HOLD FIRE";
                    _opticsWeaponStatusLabel.color = new Color(1.0f, 0.76f, 0.26f);
                }
                else if (_arena.TryGetPrimaryWeaponGunnery(out var primary))
                {
                    string wpn = (primary.Id ?? "MAIN").Replace("_", " ").ToUpperInvariant();
                    if (primary.Ready)
                    {
                        _opticsWeaponStatusLabel.text = $"RDY · {wpn}";
                        _opticsWeaponStatusLabel.color = new Color(0.38f, 0.98f, 0.64f);
                    }
                    else
                    {
                        _opticsWeaponStatusLabel.text = $"CYCLE {primary.ReadyInMs / 1000f:0.0}s · {wpn}";
                        _opticsWeaponStatusLabel.color = new Color(1.0f, 0.74f, 0.24f);
                    }
                }
                else
                {
                    _opticsWeaponStatusLabel.text = "WEAPONS READY";
                    _opticsWeaponStatusLabel.color = new Color(0.38f, 0.98f, 0.64f);
                }
            }

            bool obstructed = ti.HasTargetSample && ti.IsLineOfSightObstructed;
            if (_opticsRangefinderLabel != null)
            {
                if (!string.IsNullOrEmpty(st.LockedDesignator))
                {
                    ArenaContactState locked = null;
                    for (int i = 0; i < st.Contacts.Count; i++)
                    {
                        if (string.Equals(st.Contacts[i].Designator, st.LockedDesignator, StringComparison.OrdinalIgnoreCase))
                        {
                            locked = st.Contacts[i];
                            break;
                        }
                    }

                    float rangeM = st.LockRangeM > 0.5f ? st.LockRangeM : (locked != null ? locked.RangeM : 0f);
                    string aspectTag = locked != null && !string.IsNullOrEmpty(locked.Aspect)
                        ? $" · {locked.Aspect.ToUpperInvariant()}"
                        : string.Empty;
                    string elevTag = string.Empty;
                    if (ti.HasTargetSample && Mathf.Abs(ti.ElevationDeltaM) >= 3.0f)
                        elevTag = ti.ElevationDeltaM > 0f ? $" · +{Mathf.RoundToInt(ti.ElevationDeltaM)}m" : $" · {Mathf.RoundToInt(ti.ElevationDeltaM)}m";
                    string losTag = obstructed && !string.IsNullOrEmpty(ti.LineOfSightStatus)
                        ? $" · {ti.LineOfSightStatus} LOS"
                        : " · CLEAR LOS";

                    _opticsRangefinderLabel.text = $"LOCK {st.LockedDesignator} · {rangeM:0}m · HIT {st.LockHitPercent}%{aspectTag}{elevTag}{losTag}";
                    _opticsRangefinderLabel.color = obstructed
                        ? new Color(1.0f, 0.74f, 0.24f)
                        : (st.LockHitPercent >= 65 ? new Color(0.45f, 1.0f, 0.68f) : new Color(0.84f, 0.96f, 1.0f));
                }
                else
                {
                    string cover = !string.IsNullOrEmpty(ti.CoverLabel) ? ti.CoverLabel : "OPEN";
                    _opticsRangefinderLabel.text = $"NO LOCK · TAB TO ACQUIRE · SIGHT {st.SightMeters:0}m · {cover}";
                    _opticsRangefinderLabel.color = new Color(0.68f, 0.88f, 0.96f);
                }
            }

            float frameAlpha = opticsZoomLevel >= 2 ? 0.72f : (opticsZoomLevel == 1 ? 0.56f : 0.34f);
            Color frameColor = obstructed
                ? new Color(1.0f, 0.68f, 0.22f, frameAlpha)
                : new Color(0.32f, 0.92f, 1.0f, frameAlpha);
            for (int i = 0; i < _opticsFrameStrokes.Count; i++)
            {
                if (_opticsFrameStrokes[i] != null)
                    _opticsFrameStrokes[i].color = frameColor;
            }
        }

        private void BuildArmorPlate(Transform parent, string name, Vector2 anchor, Vector2 minOff, Vector2 maxOff, out Image bg, out Text txt)
        {
            var go = CreatePanel(parent, name, anchor, anchor, minOff, maxOff, new Color(0.16f, 0.68f, 0.44f, 0.90f));
            bg = go.GetComponent<Image>();
            bg.raycastTarget = false;
            txt = CreateText(go.transform, "100%", 10, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);
        }

        private static void SetArmorPlate(Image bg, Text txt, string code, int pct)
        {
            int clamped = Mathf.Clamp(pct, 0, 100);
            if (bg != null)
            {
                if (clamped >= 70) bg.color = new Color(0.14f, 0.64f, 0.42f, 0.92f);
                else if (clamped >= 35) bg.color = new Color(0.84f, 0.56f, 0.14f, 0.94f);
                else bg.color = new Color(0.82f, 0.18f, 0.18f, 0.96f);
            }
            if (txt != null)
                txt.text = $"{code} {clamped}%";
        }

        private Image CreateStroke(Transform parent, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax, Color? col = null)
        {
            var go = new GameObject("Stroke", typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = col ?? Color.cyan;
            img.raycastTarget = false;
            SetRect(go.GetComponent<RectTransform>(), aMin, aMax, oMin, oMax);
            return img;
        }

        private GameObject CreatePanel(Transform parent, string name, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), aMin, aMax, oMin, oMax);
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
