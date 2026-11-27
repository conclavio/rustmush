using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Arena;
using TacSim.Net;
using TacSim.Space;
using TacSim.Surface;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// Modular uGUI overlays for:
    /// - Vertical Slice 2: Zero-G Asteroid Mining &amp; Space Tactical Fire Control Bar (seated at Helm)
    /// - Vertical Slice 3: Planetary Surface Rover &amp; Quarry `site.map` HUD (when excursion active)
    /// - OOC Simulator Arena: Diegetic Simulator Selector Console GUI (`sim team`, `sim unit`, `sim ai`,
    ///   `sim mode`, `sim start`, `board`, `sim status`, `sim leave`, `+ic`) &amp; Pooled 3D Cockpit HUD.
    /// Zero standalone/offline bypass buttons.
    /// </summary>
    public sealed class TacSimMissionOverlays : MonoBehaviour
    {
        public event Action<string> OnCommandRequested;
        public event Action OnExitSurfaceExcursionRequested;
        public event Action OnOpenGroundChartRequested;
        public event Action<string> OnArenaCockpitVerbRequested;
        public event Action<string> OnPrefillChatRequested;
        public event Action OnToggleVehicleCameraViewRequested;
        public event Action OnCycleArenaOpticsRequested;

        private Font _font;
        private SurfaceMiningController _surface;
        private OocArenaController _arena;
        private SpaceFlightController _space;
        private bool _seatedAtHelm;

        // Slice 2: Orbital Ops Bar (Mining + Space Tactical) & Doc 49 (`CC.9`) Fire-Control Card
        private GameObject _orbitalOpsBar;
        private Text _orbitalTargetLabel;
        private GameObject _spaceFireCard;
        private Text _spaceLockSummaryText;

        private sealed class PooledSpaceMountRow
        {
            public GameObject Root;
            public Image Background;
            public Text Label;
        }

        private const int MaxPooledSpaceMountRows = 8;
        private readonly List<PooledSpaceMountRow> _spaceMountRowPool = new List<PooledSpaceMountRow>(MaxPooledSpaceMountRows);

        // Slice 3: Surface Rover HUD
        private GameObject _surfaceHudRoot;
        private Text _surfaceHeaderText;
        private Text _surfaceGaugesText;
        private Text _surfaceReadingsText;
        private GameObject _surfaceRigRow;
        private readonly List<Image> _surfaceRigBtnImages = new List<Image>();
        private readonly List<Text> _surfaceRigBtnLabels = new List<Text>();
        private readonly List<int> _surfaceRigBtnSlots = new List<int>();
        private readonly List<Image> _familyBtnImages = new List<Image>();

        // Diegetic OOC Simulator Selector Console Modal (`room.simulator_lobby`)
        private GameObject _arenaLobbyModal;
        private Text _arenaLobbySummaryText;
        private Text _arenaLobbyChassisSpecText;
        private Text _arenaLobbyLiveRosterText;
        private Text _arenaLobbyReplayHintText;
        private readonly List<Image> _teamBtnImages = new List<Image>();
        private readonly List<Image> _unitBtnImages = new List<Image>();
        private readonly List<Image> _aiFillBtnImages = new List<Image>();
        private readonly List<Image> _modeBtnImages = new List<Image>();
        private const int MaxRecentReplayPills = 4;

        private sealed class PooledReplayPill
        {
            public GameObject Root;
            public Button Button;
            public Text Label;
            public ulong MatchId;
        }

        private readonly List<PooledReplayPill> _recentReplayPills = new List<PooledReplayPill>(MaxRecentReplayPills);
        private readonly List<(ulong matchId, string badge)> _parsedHistoryMatches = new List<(ulong, string)>(MaxRecentReplayPills);

        // OOC Arena Active 3D Battlefield Cockpit HUD (with Pooled Contact Rows!)
        private GameObject _arenaBattleHudRoot;
        private Text _arenaScoreboardText;
        private Text _arenaBannerText;
        private Text _arenaOwnArmorText;
        private Text _arenaWeaponText;
        private RawImage _arenaArmorDollImg;
        private Texture2D _arenaArmorDollTex;
        private Color32[] _arenaArmorDollPixels;
        private string _lastArmorDollSignature = string.Empty;
        private Text _fireBtnLabel;
        private Button _emconBtn;
        private Text _emconBtnLabel;
        private Image _opticsBtnBg;
        private Text _opticsBtnLabel;
        private Button _advanceBtn;
        private Text _advanceBtnLabel;
        private string _selectedAdvanceSpot;
        private Transform _arenaContactsListRoot;
        private Transform _arenaKeySpotsRowRoot;
        private string _lastKeySpotsSignature = string.Empty;

        // OOC Arena Detachment Squad Command & Incoming Order Banner (`arena.cockpit.detachment`)
        private GameObject _detachmentBar;
        private GameObject _detachmentOrderRow;
        private Text _detachmentOrderText;
        private Button _detachmentWilcoBtn;
        private Button _detachmentNegativeBtn;
        private Text _detachmentInvalidText;

        private sealed class PooledDetachmentSlotButton
        {
            public GameObject Root;
            public Image Background;
            public Button Button;
            public Text Label;
            public string BoundPrefill;
        }

        private const int MaxPooledDetachmentButtons = 7; // [ALL] + slots 1..6
        private readonly List<PooledDetachmentSlotButton> _detachmentSlotPool = new List<PooledDetachmentSlotButton>(MaxPooledDetachmentButtons);

        private const string DefaultLobbyRosterHint =
            "Walk north into <b>Red Room</b> or south into <b>Blue Room</b> after <b>SIM START</b>, then press <b>[E]</b> on your pod (or click <b>BOARD POD</b>)!";

        private sealed class PooledContactRow
        {
            public GameObject Root;
            public Image Background;
            public Button Button;
            public Text Label;
            public string BoundDesignator;
            public bool CanLock;
        }

        private const int MaxPooledContactRows = 8;
        private readonly List<PooledContactRow> _contactRowPool = new List<PooledContactRow>(MaxPooledContactRows);

        // OOC Arena Standing Auto-Lock / Auto-Fire Rules & Pooled Per-Weapon Fire Deck
        private Button _autoLockRuleBtn;
        private Image _autoLockRuleBg;
        private Text _autoLockRuleLabel;
        private Button _autoFireRuleBtn;
        private Image _autoFireRuleBg;
        private Text _autoFireRuleLabel;
        private Button _autoFireOddsBtn;
        private Text _autoFireOddsLabel;
        private int _autoFireOddsPct = 60;
        private static readonly int[] AutoFireOddsPresets = { 40, 60, 80 };

        private sealed class PooledWeaponSlotButton
        {
            public GameObject Root;
            public Button Button;
            public Image Background;
            public Text Label;
            public string BoundWeaponId;
        }

        private const int MaxPooledWeaponSlotButtons = 4;
        private readonly List<PooledWeaponSlotButton> _weaponSlotPool = new List<PooledWeaponSlotButton>(MaxPooledWeaponSlotButtons);

        // OOC Arena Locked Target Inspection Paper-Doll Card (`contacts[i].condition` + `scan_tables`)
        private GameObject _targetInspectCard;
        private Text _targetInspectText;
        private Button _targetScanBtn;
        private RawImage _targetArmorDollImg;
        private Texture2D _targetArmorDollTex;
        private Color32[] _targetArmorDollPixels;
        private string _lastTargetDollSignature = string.Empty;

        // OOC Arena Sealed Mind Profile & Hull Posture Deck (`arena.cockpit.mind` + `posture`)
        private GameObject _mindStrip;
        private RectTransform _mindStripRect;
        private Text _mindHeaderText;
        private readonly List<PooledMindProfileButton> _mindProfilePool = new List<PooledMindProfileButton>(MaxPooledMindProfiles);
        private readonly Dictionary<string, (Button btn, Image bg)> _postureButtons = new Dictionary<string, (Button, Image)>(StringComparer.OrdinalIgnoreCase);

        private sealed class PooledMindProfileButton
        {
            public GameObject Root;
            public Button Button;
            public Image Background;
            public Text Label;
            public string BoundProfileName;
        }

        private const int MaxPooledMindProfiles = 4;

        // OOC Arena Live Combat & Kill Feed Ticker (docked mid-left cleanly below TacticalMinimap)
        private GameObject _combatFeedPanel;
        private readonly List<Text> _combatFeedLines = new List<Text>(MaxCombatFeedLines);
        private const int MaxCombatFeedLines = 5;

        // OOC Arena Wrecked / Match Debrief Card (appears cleanly in center-screen when hull wrecked)
        private GameObject _arenaDebriefCard;
        private Text _arenaDebriefText;
        private ulong _dismissedDebriefMatchId;

        // OOC Arena Collapsible Standing Rules & Sealed Mind Inspector Drawer (`rules` + `mind.sealed_rules` + `order`)
        private GameObject _rulesDrawer;
        private RectTransform _rulesDrawerRect;
        private Text _rulesDrawerBodyText;
        private Button _rulesToggleBtn;
        private Text _rulesToggleBtnLabel;
        private bool _rulesDrawerOpen;
        private const int MaxPooledStandingRuleChips = 6;
        private readonly List<PooledStandingRuleChip> _ruleChipPool = new List<PooledStandingRuleChip>(MaxPooledStandingRuleChips);

        private sealed class PooledStandingRuleChip
        {
            public GameObject Root;
            public Button Button;
            public Text Label;
            public int BoundSlot;
        }

        public bool IsLobbyModalOpen => _arenaLobbyModal != null && _arenaLobbyModal.activeSelf;
        public bool IsSpaceFireCardVisible => _spaceFireCard != null && _spaceFireCard.activeSelf;
        public bool IsDetachmentBarVisible => _detachmentBar != null && _detachmentBar.activeSelf;
        public bool IsTargetInspectCardVisible => _targetInspectCard != null && _targetInspectCard.activeSelf;
        public string TargetInspectSummarySnapshot => _targetInspectText != null ? _targetInspectText.text : string.Empty;
        public Texture2D TargetArmorDollTexture => _targetArmorDollTex;
        public bool IsMindStripVisible => _mindStrip != null && _mindStrip.activeSelf;
        public string MindStripHeaderSnapshot => _mindHeaderText != null ? _mindHeaderText.text : string.Empty;
        public bool IsCombatFeedVisible => _combatFeedPanel != null && _combatFeedPanel.activeSelf;
        public bool IsDebriefCardVisible => _arenaDebriefCard != null && _arenaDebriefCard.activeSelf;
        public string DebriefSummarySnapshot => _arenaDebriefText != null ? _arenaDebriefText.text : string.Empty;
        public bool IsRulesDrawerOpen => _rulesDrawer != null && _rulesDrawer.activeSelf;
        public string RulesDrawerSnapshot => _rulesDrawerBodyText != null ? _rulesDrawerBodyText.text : string.Empty;
        public int VisibleStandingRulePillCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _ruleChipPool.Count; i++)
                    if (_ruleChipPool[i].Root != null && _ruleChipPool[i].Root.activeSelf) n++;
                return n;
            }
        }
        public int VisibleCombatFeedEntryCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _combatFeedLines.Count; i++)
                    if (_combatFeedLines[i] != null && _combatFeedLines[i].gameObject.activeSelf && !string.IsNullOrEmpty(_combatFeedLines[i].text)) n++;
                return n;
            }
        }
        public string CombatFeedSnapshot
        {
            get
            {
                var sb = new StringBuilder(160);
                for (int i = 0; i < _combatFeedLines.Count; i++)
                {
                    if (_combatFeedLines[i] == null || !_combatFeedLines[i].gameObject.activeSelf) continue;
                    if (sb.Length > 0) sb.Append('\n');
                    sb.Append(_combatFeedLines[i].text);
                }
                return sb.ToString();
            }
        }
        public int VisibleMindProfileButtonCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _mindProfilePool.Count; i++)
                    if (_mindProfilePool[i].Root != null && _mindProfilePool[i].Root.activeSelf) n++;
                return n;
            }
        }
        public bool IsAutoLockArmedUi => _arena != null && _arena.HasAutoLockRule;
        public bool IsAutoFireArmedUi => _arena != null && _arena.HasAutoFireRule;
        public int AutoFireOddsThresholdPct => _autoFireOddsPct;
        public int ActiveWeaponSlotButtonCount
        {
            get
            {
                int count = 0;
                for (int i = 0; i < _weaponSlotPool.Count; i++)
                    if (_weaponSlotPool[i].Root != null && _weaponSlotPool[i].Root.activeSelf) count++;
                return count;
            }
        }
        public bool IsDetachmentOrderBannerVisible => _detachmentOrderRow != null && _detachmentOrderRow.activeSelf;
        public string DetachmentOrderText => _detachmentOrderText != null ? _detachmentOrderText.text : string.Empty;
        public bool IsArmorDollVisible => _arenaArmorDollImg != null && _arenaArmorDollImg.gameObject.activeSelf && _arenaArmorDollTex != null;
        public string ActiveArmorDollUnitType { get; private set; } = string.Empty;
        public Texture2D ArmorDollTexture => _arenaArmorDollTex;
        public int VisibleDetachmentSlotButtonCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _detachmentSlotPool.Count; i++)
                {
                    if (_detachmentSlotPool[i].Root != null && _detachmentSlotPool[i].Root.activeSelf)
                        n++;
                }
                return n;
            }
        }
        public string SpaceLockSummaryText => _spaceLockSummaryText != null ? _spaceLockSummaryText.text : string.Empty;
        public int VisibleSpaceMountRowCount
        {
            get
            {
                int n = 0;
                for (int i = 0; i < _spaceMountRowPool.Count; i++)
                {
                    if (_spaceMountRowPool[i].Root != null && _spaceMountRowPool[i].Root.activeSelf)
                        n++;
                }
                return n;
            }
        }

        public string GetSpaceMountRowText(int index)
        {
            if (index < 0 || index >= _spaceMountRowPool.Count || _spaceMountRowPool[index].Label == null)
                return string.Empty;
            return _spaceMountRowPool[index].Label.text ?? string.Empty;
        }

        public void BuildOverlays(Transform gameplayHudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _familyBtnImages.Clear();
            _teamBtnImages.Clear();
            _unitBtnImages.Clear();
            _aiFillBtnImages.Clear();
            _modeBtnImages.Clear();
            _contactRowPool.Clear();
            _spaceMountRowPool.Clear();
            _detachmentSlotPool.Clear();
            _weaponSlotPool.Clear();
            _mindProfilePool.Clear();
            _postureButtons.Clear();
            _combatFeedLines.Clear();
            _ruleChipPool.Clear();

            if (gameplayHudRoot == null) return;

            // 1. Slice 2 Orbital Mining, Space Tactical Bar & Doc 49 (`CC.9`) Fire-Control Card
            BuildSlice2OrbitalOpsBar(gameplayHudRoot);
            BuildSpaceFireControlCard(gameplayHudRoot);

            // 2. Slice 3 Planetary Surface & Quarry HUD (shown when live rover excursion is active)
            BuildSlice3SurfaceHud(gameplayHudRoot);

            // 3. Diegetic OOC Simulator Selector Console GUI & Pooled 3D Cockpit HUD
            BuildArenaLobbyModal(gameplayHudRoot);
            BuildArenaBattleCockpitHud(gameplayHudRoot);
        }

        public void BindControllers(
            SurfaceMiningController surface,
            OocArenaController arena,
            SpaceFlightController space = null)
        {
            if (_surface != null) _surface.OnSurfaceTelemetryChanged -= RefreshSurfaceHud;
            if (_arena != null) _arena.OnArenaStateUpdated -= RefreshArenaHud;
            if (_space != null) _space.OnSpaceTelemetryChanged -= RefreshSpaceFireCard;

            _surface = surface;
            _arena = arena;
            _space = space;

            if (_surface != null) _surface.OnSurfaceTelemetryChanged += RefreshSurfaceHud;
            if (_arena != null) _arena.OnArenaStateUpdated += RefreshArenaHud;
            if (_space != null) _space.OnSpaceTelemetryChanged += RefreshSpaceFireCard;

            RefreshLobbySelectionHighlights();
            RefreshSurfaceHud();
            RefreshArenaHud();
            RefreshSpaceFireCard();
        }

        private void OnDestroy()
        {
            if (_surface != null) _surface.OnSurfaceTelemetryChanged -= RefreshSurfaceHud;
            if (_arena != null) _arena.OnArenaStateUpdated -= RefreshArenaHud;
            if (_space != null) _space.OnSpaceTelemetryChanged -= RefreshSpaceFireCard;
            _surface = null;
            _arena = null;
            _space = null;
            if (_arenaArmorDollTex != null)
            {
                if (Application.isPlaying) Destroy(_arenaArmorDollTex);
                else DestroyImmediate(_arenaArmorDollTex);
                _arenaArmorDollTex = null;
            }
            if (_targetArmorDollTex != null)
            {
                if (Application.isPlaying) Destroy(_targetArmorDollTex);
                else DestroyImmediate(_targetArmorDollTex);
                _targetArmorDollTex = null;
            }
        }

        public void ResetSessionState()
        {
            _seatedAtHelm = false;
            _selectedAdvanceSpot = null;
            _lastKeySpotsSignature = string.Empty;
            _lastArmorDollSignature = string.Empty;
            _lastTargetDollSignature = string.Empty;
            ActiveArmorDollUnitType = string.Empty;
            _dismissedDebriefMatchId = 0;
            if (_orbitalOpsBar != null) _orbitalOpsBar.SetActive(false);
            if (_spaceFireCard != null) _spaceFireCard.SetActive(false);
            if (_surfaceHudRoot != null) _surfaceHudRoot.SetActive(false);
            if (_arenaLobbyModal != null) _arenaLobbyModal.SetActive(false);
            if (_arenaBattleHudRoot != null) _arenaBattleHudRoot.SetActive(false);
            if (_detachmentBar != null) _detachmentBar.SetActive(false);
            if (_targetInspectCard != null) _targetInspectCard.SetActive(false);
            if (_mindStrip != null) _mindStrip.SetActive(false);
            if (_combatFeedPanel != null) _combatFeedPanel.SetActive(false);
            if (_arenaDebriefCard != null) _arenaDebriefCard.SetActive(false);
            if (_rulesDrawer != null) _rulesDrawer.SetActive(false);
            _rulesDrawerOpen = false;
            _parsedHistoryMatches.Clear();
            RefreshRecentReplayPills();
            if (_arenaLobbyLiveRosterText != null) _arenaLobbyLiveRosterText.text = DefaultLobbyRosterHint;
        }

        public string SelectedAdvanceSpot => ResolveAdvanceSpot();

        public void SelectAdvanceSpot(string spotLetterOrName)
        {
            _selectedAdvanceSpot = string.IsNullOrWhiteSpace(spotLetterOrName) ? null : spotLetterOrName.Trim();
            UpdateAdvanceButtonVisual();
        }

        public string ResolveAdvanceSpot()
        {
            var spots = _arena?.TerrainGrid?.KeySpots;
            if (spots != null && spots.Count > 0)
            {
                if (!string.IsNullOrEmpty(_selectedAdvanceSpot))
                {
                    for (int i = 0; i < spots.Count; i++)
                    {
                        if ((string.Equals(spots[i].Letter, _selectedAdvanceSpot, StringComparison.OrdinalIgnoreCase) ||
                             string.Equals(spots[i].Name, _selectedAdvanceSpot, StringComparison.OrdinalIgnoreCase)) &&
                            MushProtocol.TryValidateSafeCommandToken(spots[i].Letter, out string matchedSpot, maxLen: 16))
                        {
                            return matchedSpot;
                        }
                    }
                }
                for (int i = 0; i < spots.Count; i++)
                {
                    if (MushProtocol.TryValidateSafeCommandToken(spots[i].Letter, out string safeSpot, maxLen: 16))
                        return safeSpot;
                }
                return null;
            }
            return MushProtocol.TryValidateSafeCommandToken(_selectedAdvanceSpot, out string fallbackSpot, maxLen: 16)
                ? fallbackSpot
                : null;
        }

        private string ResolveSquadAllOrderPrefix()
        {
            var det = _arena?.State?.Detachment;
            if (det != null && !string.IsNullOrEmpty(det.AllOrderPrefill))
                return det.AllOrderPrefill;
            return "order all ";
        }

        public string ResolveSquadAdvanceCommand()
        {
            string spot = ResolveAdvanceSpot();
            return MushProtocol.TryValidateSafeCommandToken(spot, out string safeSpot, maxLen: 16)
                ? $"{ResolveSquadAllOrderPrefix()}advance {safeSpot}"
                : null;
        }

        public void DispatchDetachmentWilco()
        {
            string cmd = _arena?.State?.Detachment?.WilcoCommand;
            if (!string.IsNullOrEmpty(cmd))
                OnArenaCockpitVerbRequested?.Invoke(cmd);
        }

        public void DispatchDetachmentNegative()
        {
            string cmd = _arena?.State?.Detachment?.NegativeCommand;
            if (!string.IsNullOrEmpty(cmd))
                OnArenaCockpitVerbRequested?.Invoke(cmd);
        }

        public void PrefillDetachmentSlot(int buttonIndex)
        {
            if (buttonIndex < 0 || buttonIndex >= _detachmentSlotPool.Count) return;
            var btn = _detachmentSlotPool[buttonIndex];
            if (btn.Root != null && btn.Root.activeSelf && !string.IsNullOrEmpty(btn.BoundPrefill))
                OnPrefillChatRequested?.Invoke(btn.BoundPrefill);
        }

        private void UpdateAdvanceButtonVisual()
        {
            string spot = ResolveAdvanceSpot();
            if (_advanceBtn != null)
                _advanceBtn.interactable = !string.IsNullOrEmpty(spot);
            if (_advanceBtnLabel != null)
                _advanceBtnLabel.text = !string.IsNullOrEmpty(spot) ? $"SQUAD: ADV [{spot}]" : "SQUAD: ADVANCE";
        }

        public void SetHelmSeatedVisible(bool seatedAtHelm)
        {
            _seatedAtHelm = seatedAtHelm;
            bool showHelm = seatedAtHelm &&
                            (_surface == null || !_surface.IsExcursionActive) &&
                            (_arena == null || !_arena.IsArenaActive);
            if (_orbitalOpsBar != null)
                _orbitalOpsBar.SetActive(showHelm);
            RefreshSpaceFireCard();
        }

        public void UpdateOrbitalTargetReadout(
            string rockId,
            string contactId,
            string matchedRockId = null,
            string approachingRockId = null,
            string lockFacet = null)
        {
            if (_orbitalTargetLabel != null)
            {
                string r = string.IsNullOrEmpty(rockId) ? "NONE" : MushProtocol.EscapeUnityRichText(rockId);
                string rState = string.Empty;
                if (!string.IsNullOrEmpty(rockId))
                {
                    if (string.Equals(rockId, matchedRockId, StringComparison.OrdinalIgnoreCase))
                        rState = " <color=#55FF99>[MATCHED]</color>";
                    else if (string.Equals(rockId, approachingRockId, StringComparison.OrdinalIgnoreCase))
                        rState = " <color=#FFCC55>[APPROACH]</color>";
                    else if (matchedRockId != null || approachingRockId != null)
                        rState = " <color=#FF8866>[UNMATCHED]</color>";
                }
                string c = string.IsNullOrEmpty(contactId) ? "NONE" : MushProtocol.EscapeUnityRichText(contactId);
                string bandTag = !string.IsNullOrEmpty(lockFacet)
                    ? $" <color=#FFBB55>[{MushProtocol.EscapeUnityRichText(lockFacet.ToUpperInvariant())} BAND]</color>"
                    : string.Empty;
                _orbitalTargetLabel.text = $"ROCK: <b>{r}</b>{rState} | LOCK: <b>{c}</b>{bandTag}";
            }
            RefreshSpaceFireCard();
        }

        // ---- SLICE 2: ORBITAL MINING, NAVAL TACTICAL BAR & DOC 49 FIRE-CONTROL CARD ----

        private void BuildSlice2OrbitalOpsBar(Transform parent)
        {
            // Anchored strictly right of CommsDrawer (`x = 452..-16`) so zero overlap at 1920x1080 or narrow viewports.
            _orbitalOpsBar = CreatePanel(parent, "OrbitalMiningAndTacBar", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 124f), new Vector2(-16f, 204f), new Color(0.03f, 0.08f, 0.14f, 0.92f));

            var rowMine = CreatePanel(_orbitalOpsBar.transform, "ZeroGMiningRow", new Vector2(0f, 0.5f), new Vector2(1f, 1f), new Vector2(8f, 2f), new Vector2(-8f, -4f), Color.clear);
            var mHlg = rowMine.AddComponent<HorizontalLayoutGroup>();
            mHlg.spacing = 5f; mHlg.childControlWidth = true; mHlg.childControlHeight = true; mHlg.childForceExpandWidth = true;

            _orbitalTargetLabel = CreateTxt(rowMine.transform, "ROCK: <b>NONE</b> | LOCK: <b>NONE</b>", 11, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.55f, 0.95f, 0.85f));
            _orbitalTargetLabel.supportRichText = true;

            CreateBtn(rowMine.transform, "PING ROCKS", new Color(0.12f, 0.38f, 0.44f), () => OnCommandRequested?.Invoke("space:mine_ping"));
            CreateBtn(rowMine.transform, "ROCK A", new Color(0.16f, 0.34f, 0.30f), () => OnCommandRequested?.Invoke("space:rock:A"));
            CreateBtn(rowMine.transform, "ROCK B", new Color(0.16f, 0.34f, 0.30f), () => OnCommandRequested?.Invoke("space:rock:B"));
            CreateBtn(rowMine.transform, "ROCK C", new Color(0.16f, 0.34f, 0.30f), () => OnCommandRequested?.Invoke("space:rock:C"));
            CreateBtn(rowMine.transform, "MATCH VEL", new Color(0.18f, 0.46f, 0.56f), () => OnCommandRequested?.Invoke("space:helm_match"));
            CreateBtn(rowMine.transform, "MINING LASER", new Color(0.12f, 0.50f, 0.44f), () => OnCommandRequested?.Invoke("space:mine_laser"));
            CreateBtn(rowMine.transform, "DRONE SORTIE", new Color(0.46f, 0.36f, 0.12f), () => OnCommandRequested?.Invoke("space:mine_drones"));
            CreateBtn(rowMine.transform, "STAKE CLAIM", new Color(0.24f, 0.42f, 0.26f), () => OnCommandRequested?.Invoke("space:mine_stake"));

            var rowTac = CreatePanel(_orbitalOpsBar.transform, "NavalTacticalRow", new Vector2(0f, 0f), new Vector2(1f, 0.5f), new Vector2(8f, 4f), new Vector2(-8f, -2f), Color.clear);
            var tHlg = rowTac.AddComponent<HorizontalLayoutGroup>();
            tHlg.spacing = 4f; tHlg.childControlWidth = true; tHlg.childControlHeight = true; tHlg.childForceExpandWidth = true;

            CreateBtn(rowTac.transform, "SCAN", new Color(0.16f, 0.32f, 0.48f), () => OnCommandRequested?.Invoke("+tac/scan"));
            CreateBtn(rowTac.transform, "TAC STATUS", new Color(0.22f, 0.30f, 0.44f), () => OnCommandRequested?.Invoke("+tac/status"));
            CreateBtn(rowTac.transform, "FIRE BEAM", new Color(0.55f, 0.18f, 0.24f), () => OnCommandRequested?.Invoke("space:tac_beam"));
            CreateBtn(rowTac.transform, "RAILGUN", new Color(0.20f, 0.40f, 0.62f), () => OnCommandRequested?.Invoke("space:tac_railgun"));
            CreateBtn(rowTac.transform, "TORPEDO", new Color(0.58f, 0.30f, 0.12f), () => OnCommandRequested?.Invoke("space:tac_torp"));
            CreateBtn(rowTac.transform, "LASER OFFENSE", new Color(0.52f, 0.24f, 0.18f), () => OnCommandRequested?.Invoke("+tac/pdc offense"));
            CreateBtn(rowTac.transform, "LASER DEFENSE", new Color(0.18f, 0.38f, 0.50f), () => OnCommandRequested?.Invoke("+tac/pdc point"));
            CreateBtn(rowTac.transform, "PDC AUTO", new Color(0.22f, 0.34f, 0.42f), () => OnCommandRequested?.Invoke("+tac/pdc auto"));
            CreateBtn(rowTac.transform, "CUSTOMS SUBMIT", new Color(0.20f, 0.42f, 0.36f), () => OnCommandRequested?.Invoke("+helm/submit"));

            _orbitalOpsBar.SetActive(false);
        }

        private void BuildSpaceFireControlCard(Transform parent)
        {
            _spaceFireCard = CreatePanel(parent, "SpaceFireControlCard", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-430f, -346f), new Vector2(-16f, -74f), new Color(0.03f, 0.07f, 0.13f, 0.91f));

            var hdr = CreateTxt(_spaceFireCard.transform, "FIRE CONTROL — MOUNTS, MAGAZINES & 5-BAND LOCK", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.58f, 0.90f, 1.0f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -26f), new Vector2(-10f, -4f));

            _spaceLockSummaryText = CreateTxt(_spaceFireCard.transform, "NO ACTIVE LOCK — Click 3D contact or press <b>SCAN</b>", 11, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.92f, 0.96f, 1.0f));
            _spaceLockSummaryText.supportRichText = true;
            SetRect(_spaceLockSummaryText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -52f), new Vector2(-10f, -28f));

            var mountsArea = CreatePanel(_spaceFireCard.transform, "SpaceMountsArea", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(8f, 8f), new Vector2(-8f, -56f), Color.clear);
            var vlg = mountsArea.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 3f;
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxPooledSpaceMountRows; i++)
            {
                var rowGo = CreatePanel(mountsArea.transform, $"MountRow_{i}", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.08f, 0.15f, 0.22f, 0.90f));
                rowGo.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 24f);
                var lbl = CreateTxt(rowGo.transform, string.Empty, 11, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
                lbl.supportRichText = true;
                SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 1f), new Vector2(-8f, -1f));
                rowGo.SetActive(false);
                _spaceMountRowPool.Add(new PooledSpaceMountRow
                {
                    Root = rowGo,
                    Background = rowGo.GetComponent<Image>(),
                    Label = lbl,
                });
            }

            _spaceFireCard.SetActive(false);
        }

        public static string FormatSpaceMountRowSummary(SpaceMountTelemetry m)
        {
            if (m == null) return string.Empty;
            string famLabel = m.Family switch
            {
                "tubes" => "TUBES",
                "spinal" => "SPINAL",
                "point_defence" => "POINT DEF",
                _ => "LASER"
            };

            string magOrMode = string.Empty;
            if (!string.IsNullOrEmpty(m.Mode))
            {
                magOrMode = $" · {MushProtocol.EscapeUnityRichText(m.Mode.ToUpperInvariant())}";
            }
            else if (m.Rounds >= 0 && m.RoundsMax > 0)
            {
                string col = m.Rounds == 0 ? "#FF6666" : (m.Rounds < m.RoundsMax ? "#FFCC66" : "#BBEEFF");
                magOrMode = $" · <color={col}>{m.Rounds}/{m.RoundsMax}</color>";
            }
            else if (m.Rounds >= 0)
            {
                magOrMode = $" · {m.Rounds} LEFT";
            }

            string stateWord = m.State switch
            {
                "cycling" => $"<color=#FFCC55>CYCLING {m.ReadyInMs / 1000f:F1}s</color>",
                "empty" => "<color=#FF5555>EMPTY</color>",
                "too_hot" => "<color=#FF7744>TOO HOT</color>",
                "past_reach" => "<color=#CCBB77>PAST REACH</color>",
                "down" => "<color=#AA6666>DOWN</color>",
                _ => m.Mode == "defense"
                    ? "<color=#66CCFF>DEFENSE SCREEN</color>"
                    : "<color=#55FF99>READY</color>"
            };

            string atLock = string.Empty;
            if (m.DamageAtLock >= 0)
                atLock = $" · <b>{m.DamageAtLock} DMG</b>";
            else if (m.FlightMsAtLock >= 0)
                atLock = $" · <b>{m.FlightMsAtLock / 1000f:F1}s FLIGHT</b>";

            string safeId = MushProtocol.EscapeUnityRichText((m.Id ?? string.Empty).ToUpperInvariant());
            return $"<b>{safeId}</b> [{famLabel}{magOrMode}]  {stateWord}{atLock}";
        }

        public void RefreshSpaceFireCard()
        {
            if (_spaceFireCard == null) return;
            bool show = _seatedAtHelm &&
                        _space != null &&
                        !_space.IsDockingActive &&
                        (_surface == null || !_surface.IsExcursionActive) &&
                        (_arena == null || !_arena.IsArenaActive) &&
                        (_space.FireMounts.Count > 0 || _space.ActiveLock != null || !string.IsNullOrEmpty(_space.LockedContactId));
            _spaceFireCard.SetActive(show);
            if (!show) return;

            if (_spaceLockSummaryText != null)
            {
                if (_space.ActiveLock != null)
                {
                    var lk = _space.ActiveLock;
                    string rangeTxt = lk.RangeM >= 1000 ? $"{lk.RangeM / 1000f:0.0} km" : $"{lk.RangeM} m";
                    string lbl = string.IsNullOrEmpty(lk.Label) ? lk.ShipId : lk.Label;
                    string safeLbl = MushProtocol.EscapeUnityRichText((lbl ?? string.Empty).ToUpperInvariant());
                    string safeShipId = MushProtocol.EscapeUnityRichText(lk.ShipId ?? string.Empty);
                    string safeFacet = MushProtocol.EscapeUnityRichText((lk.Facet ?? string.Empty).ToUpperInvariant());
                    _spaceLockSummaryText.text =
                        $"LOCK: <b>{safeLbl}</b> ({safeShipId})  |  <b>{rangeTxt}</b>  |  BAND: <color=#FFBB55><b>[{safeFacet}]</b></color>";
                }
                else if (!string.IsNullOrEmpty(_space.LockedContactId))
                {
                    _spaceLockSummaryText.text = $"LOCK DESIGNATED: <b>{MushProtocol.EscapeUnityRichText(_space.LockedContactId)}</b> (awaiting sensor solution)";
                }
                else
                {
                    _spaceLockSummaryText.text = "NO ACTIVE LOCK — Click 3D contact or press <b>SCAN</b>";
                }
            }

            var mounts = _space.FireMounts;
            int shown = Mathf.Min(mounts.Count, _spaceMountRowPool.Count);
            for (int i = 0; i < shown; i++)
            {
                var m = mounts[i];
                var slot = _spaceMountRowPool[i];
                slot.Label.text = FormatSpaceMountRowSummary(m);
                slot.Background.color = m.State == "down" || m.State == "empty"
                    ? new Color(0.26f, 0.12f, 0.14f, 0.90f)
                    : (m.State == "ready" && m.Mode != "defense"
                        ? new Color(0.08f, 0.22f, 0.18f, 0.90f)
                        : new Color(0.09f, 0.16f, 0.24f, 0.90f));
                if (!slot.Root.activeSelf) slot.Root.SetActive(true);
            }
            for (int i = shown; i < _spaceMountRowPool.Count; i++)
            {
                if (_spaceMountRowPool[i].Root.activeSelf)
                    _spaceMountRowPool[i].Root.SetActive(false);
            }
        }

        // ---- SLICE 3: PLANETARY SURFACE ROVER & QUARRY HUD ----

        private void BuildSlice3SurfaceHud(Transform parent)
        {
            _surfaceHudRoot = CreatePanel(parent, "SurfaceMiningHud", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, Color.clear);

            var card = CreatePanel(_surfaceHudRoot.transform, "QuarryCard", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-470f, -376f), new Vector2(-16f, -74f), new Color(0.04f, 0.08f, 0.12f, 0.92f));
            _surfaceHeaderText = CreateTxt(card.transform, "OCCATOR RIM — SURFACE PROSPECTOR RIG 1", 15, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.84f, 0.42f));
            SetRect(_surfaceHeaderText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(12f, -32f), new Vector2(-12f, -6f));

            _surfaceGaugesText = CreateTxt(card.transform, "HOPPER: 0 / 6 t  |  BATTERY: 100%  |  HOLD ROOM: 86 t", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.55f, 0.95f, 0.78f));
            SetRect(_surfaceGaugesText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(12f, -56f), new Vector2(-12f, -34f));

            _surfaceRigRow = CreatePanel(card.transform, "MultiRigSwitcherRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -84f), new Vector2(-10f, -58f), Color.clear);
            var rHlg = _surfaceRigRow.AddComponent<HorizontalLayoutGroup>();
            rHlg.spacing = 5f; rHlg.childControlWidth = true; rHlg.childControlHeight = true; rHlg.childForceExpandWidth = true;
            _surfaceRigBtnImages.Clear();
            _surfaceRigBtnLabels.Clear();
            _surfaceRigBtnSlots.Clear();
            for (int i = 0; i < 4; i++)
            {
                int btnIdx = i;
                var rb = CreateBtn(_surfaceRigRow.transform, $"RIG {i + 1}", new Color(0.14f, 0.24f, 0.34f), () =>
                {
                    if (btnIdx < _surfaceRigBtnSlots.Count && _surfaceRigBtnSlots[btnIdx] > 0)
                        _surface?.SelectRigSlot(_surfaceRigBtnSlots[btnIdx]);
                });
                _surfaceRigBtnImages.Add(rb.GetComponent<Image>());
                _surfaceRigBtnLabels.Add(rb.GetComponentInChildren<Text>());
                _surfaceRigBtnSlots.Add(i + 1);
                rb.gameObject.SetActive(false);
            }
            _surfaceRigRow.SetActive(false);

            var famRow = CreatePanel(card.transform, "OreFamilyRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -116f), new Vector2(-10f, -88f), Color.clear);
            var fHlg = famRow.AddComponent<HorizontalLayoutGroup>();
            fHlg.spacing = 6f; fHlg.childControlWidth = true; fHlg.childControlHeight = true; fHlg.childForceExpandWidth = true;

            string[] families = { "volatiles", "metals", "strategic", "carbon" };
            for (int i = 0; i < families.Length; i++)
            {
                int idx = i;
                string fam = families[i];
                var b = CreateBtn(famRow.transform, fam.ToUpperInvariant(), new Color(0.14f, 0.28f, 0.38f), () =>
                {
                    _surface?.SetOreFamilyFilter(fam);
                    HighlightPillGroup(_familyBtnImages, idx, new Color(0.22f, 0.65f, 0.85f));
                    OnCommandRequested?.Invoke($"surface:family:{fam}");
                });
                _familyBtnImages.Add(b.GetComponent<Image>());
            }
            HighlightPillGroup(_familyBtnImages, 0, new Color(0.22f, 0.65f, 0.85f));

            _surfaceReadingsText = CreateTxt(card.transform, "WASD or Left-Click terrain to drive Rig 1.\n1) Click SCAN 9 FIELDS  2) Drive near beacon & SOUND  3) CORE / DRILL!", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.84f, 0.92f, 0.98f));
            _surfaceReadingsText.supportRichText = true;
            SetRect(_surfaceReadingsText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(12f, 10f), new Vector2(-12f, -122f));

            var actionBar = CreatePanel(_surfaceHudRoot.transform, "SurfaceRoverActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 16f), new Vector2(-16f, 66f), new Color(0.05f, 0.09f, 0.13f, 0.94f));
            var aHlg = actionBar.AddComponent<HorizontalLayoutGroup>();
            aHlg.spacing = 5f; aHlg.padding = new RectOffset(8, 8, 6, 6);
            aHlg.childControlWidth = true; aHlg.childControlHeight = true; aHlg.childForceExpandWidth = true;

            CreateBtn(actionBar.transform, "SCAN 9 FIELDS", new Color(0.14f, 0.44f, 0.54f), () => OnCommandRequested?.Invoke("surface:scan"));
            CreateBtn(actionBar.transform, "SOUND POCKETS", new Color(0.52f, 0.36f, 0.12f), () => OnCommandRequested?.Invoke("surface:sound"));
            CreateBtn(actionBar.transform, "CORE / DRILL", new Color(0.14f, 0.52f, 0.34f), () => OnCommandRequested?.Invoke("surface:mine"));
            CreateBtn(actionBar.transform, "STAKE CLAIM", new Color(0.32f, 0.42f, 0.18f), () => OnCommandRequested?.Invoke("surface:claim_stake"));
            CreateBtn(actionBar.transform, "UNLOAD HOPPER", new Color(0.22f, 0.40f, 0.48f), () => OnCommandRequested?.Invoke("surface:unload"));
            CreateBtn(actionBar.transform, "RECOVER RAMP", new Color(0.28f, 0.28f, 0.44f), () => OnCommandRequested?.Invoke("surface:recover"));
            CreateBtn(actionBar.transform, "PLANET CHART [G]", new Color(0.14f, 0.38f, 0.44f), () => OnOpenGroundChartRequested?.Invoke());
            CreateBtn(actionBar.transform, "CAB / CHASE CAM", new Color(0.24f, 0.32f, 0.42f), () => OnToggleVehicleCameraViewRequested?.Invoke());
            CreateBtn(actionBar.transform, "RETURN TO BAY", new Color(0.48f, 0.18f, 0.20f), () =>
            {
                OnExitSurfaceExcursionRequested?.Invoke();
            });

            _surfaceHudRoot.SetActive(false);
        }

        public void RefreshSurfaceHud()
        {
            if (_surfaceHudRoot == null) return;
            bool show = _surface != null && _surface.IsExcursionActive && (_arena == null || !_arena.IsArenaActive);
            _surfaceHudRoot.SetActive(show);
            if (!show) return;

            _surfaceHeaderText.text = $"{_surface.ActiveSiteTitle.ToUpperInvariant()} — RIG {_surface.AddressedRigSlot} [{_surface.ActiveOreFamily.ToUpperInvariant()}]";
            _surfaceGaugesText.text = $"HOPPER: {_surface.HopperLoadedTonnes} / {_surface.HopperCapacityTonnes} t   |   BATTERY: {_surface.BatteryPct}%   |   HOLD FREE: {_surface.HoldUnreservedTonnes} t";

            if (_surfaceRigRow != null)
            {
                var rigs = _surface.OwnedRigs;
                bool multiRig = rigs != null && rigs.Count >= 2;
                _surfaceRigRow.SetActive(multiRig);
                if (multiRig)
                {
                    for (int i = 0; i < _surfaceRigBtnImages.Count; i++)
                    {
                        if (i < rigs.Count)
                        {
                            var r = rigs[i];
                            _surfaceRigBtnSlots[i] = r.Slot;
                            bool sel = r.Slot == _surface.AddressedRigSlot;
                            _surfaceRigBtnImages[i].color = sel
                                ? new Color(0.20f, 0.60f, 0.78f, 1f)
                                : new Color(0.12f, 0.22f, 0.32f, 0.94f);
                            if (_surfaceRigBtnLabels[i] != null)
                                _surfaceRigBtnLabels[i].text = $"RIG {r.Slot} ({r.ChargePct}%{(r.OnGround ? "" : " BAY")})";
                            _surfaceRigBtnImages[i].gameObject.SetActive(true);
                        }
                        else
                        {
                            _surfaceRigBtnSlots[i] = 0;
                            _surfaceRigBtnImages[i].gameObject.SetActive(false);
                        }
                    }
                }
            }

            var sb = new StringBuilder(320);
            sb.AppendLine($"<color=#FFEE77>{MushProtocol.EscapeUnityRichText(_surface.StatusSummaryLine)}</color>");
            if (!string.IsNullOrEmpty(_surface.ActiveRunSummary))
                sb.AppendLine($"<color=#66FFAA>[RUN] {MushProtocol.EscapeUnityRichText(_surface.ActiveRunSummary)}</color>");
            if (!string.IsNullOrEmpty(_surface.ActivePlanSummary))
                sb.AppendLine($"<color=#77DDFF>[PLAN] {MushProtocol.EscapeUnityRichText(_surface.ActivePlanSummary)}</color>");
            if (!string.IsNullOrEmpty(_surface.ActiveQueuedSummary))
                sb.AppendLine($"<color=#FFCC66>[QUEUED] {MushProtocol.EscapeUnityRichText(_surface.ActiveQueuedSummary)}</color>");
            if (!string.IsNullOrEmpty(_surface.PresenceLine))
                sb.AppendLine($"<color=#FFAA88>{MushProtocol.EscapeUnityRichText(_surface.PresenceLine)}</color>");
            var lines = _surface.RecentSiteLines;
            for (int i = Mathf.Max(0, lines.Count - 3); i < lines.Count; i++)
                sb.AppendLine("• " + MushProtocol.EscapeUnityRichText(lines[i]));
            sb.Append("<color=#88CCFF>[WASD / LMB Terrain] Drive   [RMB + Scroll] Orbit   [G] Ground Chart</color>");
            _surfaceReadingsText.text = sb.ToString();
        }

        // ---- DIEGETIC OOC SIMULATOR SELECTOR CONSOLE MODAL (PURE LIVE MUSHD VERBS) ----

        private void BuildArenaLobbyModal(Transform parent)
        {
            _recentReplayPills.Clear();
            _arenaLobbyModal = CreatePanel(parent, "OocArenaLobbyModal", new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-460f, -312f), new Vector2(460f, 312f), new Color(0.04f, 0.07f, 0.12f, 0.99f));

            var hdr = CreateTxt(_arenaLobbyModal.transform, "OOC SIMULATOR SELECTOR CONSOLE — RED vs BLUE", 20, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.58f, 0.48f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -46f), new Vector2(-130f, -8f));

            var closeBtn = CreateBtn(_arenaLobbyModal.transform, "CLOSE [ESC]", new Color(0.42f, 0.16f, 0.18f), () => _arenaLobbyModal.SetActive(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-120f, -44f), new Vector2(-16f, -10f));

            // Row 1: Claim Team Slot (`SIM TEAM red|blue`)
            var teamRow = CreateRow(_arenaLobbyModal.transform, "TeamRow", -98f, -54f);
            var redBtn = CreateBtn(teamRow, "CLAIM RED TEAM SLOT (SIM TEAM RED)", new Color(0.68f, 0.18f, 0.18f), () =>
            {
                if (_arena != null) _arena.SelectedLobbyTeam = "red";
                RefreshLobbySelectionHighlights();
                OnCommandRequested?.Invoke("sim team red");
            });
            var blueBtn = CreateBtn(teamRow, "CLAIM BLUE TEAM SLOT (SIM TEAM BLUE)", new Color(0.16f, 0.42f, 0.78f), () =>
            {
                if (_arena != null) _arena.SelectedLobbyTeam = "blue";
                RefreshLobbySelectionHighlights();
                OnCommandRequested?.Invoke("sim team blue");
            });
            _teamBtnImages.Add(redBtn.GetComponent<Image>());
            _teamBtnImages.Add(blueBtn.GetComponent<Image>());

            // Row 2: Select Unit Chassis (`SIM UNIT <type>`)
            var unitRow = CreateRow(_arenaLobbyModal.transform, "UnitRow", -148f, -104f);
            (string id, string label)[] units =
            {
                ("line_tank", "Line Tank"),
                ("line_tank_laser", "Laser Tank"),
                ("heavy_mk1", "Heavy Mk1"),
                ("biped_mk1", "Biped Mk1"),
                ("biped_mk2", "Biped Mk2"),
                ("scout_mk1", "Scout Mk1")
            };
            for (int i = 0; i < units.Length; i++)
            {
                string uId = units[i].id;
                var ub = CreateBtn(unitRow, units[i].label, new Color(0.14f, 0.25f, 0.36f), () =>
                {
                    if (_arena != null) _arena.SelectedLobbyUnit = uId;
                    RefreshLobbySelectionHighlights();
                    OnCommandRequested?.Invoke($"sim unit {uId}");
                });
                _unitBtnImages.Add(ub.GetComponent<Image>());
            }

            // Chassis Tactical Dossier Strip (authentic specs from crates/tacsim/content/units/*.json)
            var specCard = CreatePanel(_arenaLobbyModal.transform, "LobbyChassisSpecCard", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -196f), new Vector2(-20f, -154f), new Color(0.05f, 0.10f, 0.16f, 0.96f));
            CreatePanel(specCard.transform, "SpecAccent", new Vector2(0f, 0f), new Vector2(0f, 1f), Vector2.zero, new Vector2(3f, 0f), new Color(0.32f, 0.88f, 1.0f, 0.95f));
            _arenaLobbyChassisSpecText = CreateTxt(specCard.transform, ResolveChassisTacticalSpecs("line_tank"), 12, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.84f, 0.94f, 1f));
            _arenaLobbyChassisSpecText.supportRichText = true;
            SetRect(_arenaLobbyChassisSpecText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 2f), new Vector2(-10f, -2f));

            // Row 3: AI Wingman Fill (`SIM AI <n>`)
            var aiRow = CreateRow(_arenaLobbyModal.transform, "AiFillRow", -248f, -204f);
            (int count, string label)[] aiFills =
            {
                (1, "SIM AI 1 (1v1)"),
                (2, "SIM AI 2 (2v2)"),
                (4, "SIM AI 4 (4v4)"),
                (6, "SIM AI 6 (6v6 FULL)")
            };
            for (int i = 0; i < aiFills.Length; i++)
            {
                int n = aiFills[i].count;
                var ab = CreateBtn(aiRow, aiFills[i].label, new Color(0.14f, 0.25f, 0.36f), () =>
                {
                    if (_arena != null) _arena.SelectedLobbyAiCount = n;
                    RefreshLobbySelectionHighlights();
                    OnCommandRequested?.Invoke($"sim ai {n}");
                });
                _aiFillBtnImages.Add(ab.GetComponent<Image>());
            }

            // Row 4: Match Rule (`SIM MODE skirmish|hold`)
            var modeRow = CreateRow(_arenaLobbyModal.transform, "ModeRow", -298f, -254f);
            (string mode, string label)[] modes =
            {
                ("skirmish", "SIM MODE SKIRMISH (ELIMINATION)"),
                ("hold", "SIM MODE HOLD (OBJECTIVE ZONE)")
            };
            for (int i = 0; i < modes.Length; i++)
            {
                string md = modes[i].mode;
                var mdb = CreateBtn(modeRow, modes[i].label, new Color(0.14f, 0.25f, 0.36f), () =>
                {
                    if (_arena != null) _arena.SelectedLobbyMode = md;
                    RefreshLobbySelectionHighlights();
                    OnCommandRequested?.Invoke($"sim mode {md}");
                });
                _modeBtnImages.Add(mdb.GetComponent<Image>());
            }

            // Live Centre Status & Claimed Slots Readout
            _arenaLobbySummaryText = CreateTxt(_arenaLobbyModal.transform, "", 13, FontStyle.Normal, TextAnchor.MiddleCenter, new Color(0.86f, 0.94f, 1f));
            _arenaLobbySummaryText.supportRichText = true;
            SetRect(_arenaLobbySummaryText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -342f), new Vector2(-20f, -304f));

            _arenaLobbyLiveRosterText = CreateTxt(_arenaLobbyModal.transform, DefaultLobbyRosterHint, 13, FontStyle.Normal, TextAnchor.MiddleCenter, new Color(0.68f, 0.90f, 0.84f));
            _arenaLobbyLiveRosterText.supportRichText = true;
            SetRect(_arenaLobbyLiveRosterText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -392f), new Vector2(-20f, -344f));

            // Row 5: Start Boarding & Enter Pod Bays
            var startRow = CreateRow(_arenaLobbyModal.transform, "SimStartAndBayRow", -446f, -398f);
            CreateBtn(startRow, "OPEN PODS (SIM START)", new Color(0.16f, 0.54f, 0.36f), () =>
                OnCommandRequested?.Invoke("sim start"));
            CreateBtn(startRow, "WALK TO RED BAY", new Color(0.56f, 0.18f, 0.20f), () =>
            {
                _arenaLobbyModal.SetActive(false);
                OnCommandRequested?.Invoke("red");
            });
            CreateBtn(startRow, "WALK TO BLUE BAY", new Color(0.16f, 0.38f, 0.68f), () =>
            {
                _arenaLobbyModal.SetActive(false);
                OnCommandRequested?.Invoke("blue");
            });
            CreateBtn(startRow, "BOARD POD (BOARD)", new Color(0.52f, 0.34f, 0.12f), () =>
            {
                _arenaLobbyModal.SetActive(false);
                OnCommandRequested?.Invoke("board");
            });

            // Row 6: Centre Management, Match History & Return IC
            var mgmtRow = CreateRow(_arenaLobbyModal.transform, "SimMgmtRow", -498f, -454f);
            CreateBtn(mgmtRow, "SIM STATUS", new Color(0.16f, 0.28f, 0.40f), () => OnCommandRequested?.Invoke("sim status"));
            CreateBtn(mgmtRow, "MATCH HISTORY", new Color(0.24f, 0.36f, 0.48f), () => OnCommandRequested?.Invoke("sim history"));
            CreateBtn(mgmtRow, "SIM LEAVE", new Color(0.38f, 0.24f, 0.20f), () => OnCommandRequested?.Invoke("sim leave"));
            CreateBtn(mgmtRow, "SIM ABORT", new Color(0.46f, 0.18f, 0.20f), () => OnCommandRequested?.Invoke("sim abort"));
            CreateBtn(mgmtRow, "RETURN TO IC PORT (+ic)", new Color(0.16f, 0.40f, 0.36f), () =>
            {
                _arenaLobbyModal.SetActive(false);
                OnCommandRequested?.Invoke("+ic");
            });

            // Row 7: Pooled Quick-Launch 3D Match Replay Pills (populated from authoritative `SIM HISTORY` lines)
            var replayBox = CreatePanel(_arenaLobbyModal.transform, "LobbyRecentReplaysBox", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -558f), new Vector2(-20f, -506f), new Color(0.05f, 0.08f, 0.13f, 0.94f));
            var rLbl = CreateTxt(replayBox.transform, "RECENT 3D REPLAYS:", 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1.0f, 0.80f, 0.36f));
            SetRect(rLbl.rectTransform, new Vector2(0f, 0f), new Vector2(0f, 1f), new Vector2(10f, 4f), new Vector2(148f, -4f));

            _arenaLobbyReplayHintText = CreateTxt(replayBox.transform, "Click MATCH HISTORY above to load clickable 3D Replay Theater buttons.", 11, FontStyle.Italic, TextAnchor.MiddleLeft, new Color(0.64f, 0.78f, 0.88f));
            SetRect(_arenaLobbyReplayHintText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(154f, 4f), new Vector2(-10f, -4f));

            var replayBtnsRow = CreatePanel(replayBox.transform, "RecentReplayPillsRow", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(154f, 7f), new Vector2(-10f, -7f), Color.clear);
            var rpHlg = replayBtnsRow.AddComponent<HorizontalLayoutGroup>();
            rpHlg.spacing = 6f;
            rpHlg.childControlWidth = true;
            rpHlg.childControlHeight = true;
            rpHlg.childForceExpandWidth = true;

            for (int r = 0; r < MaxRecentReplayPills; r++)
            {
                int capturedIdx = r;
                var pill = new PooledReplayPill();
                var btn = CreateBtn(replayBtnsRow.transform, string.Empty, new Color(0.24f, 0.34f, 0.48f), () => HandleRecentReplayPillClick(capturedIdx));
                pill.Root = btn.gameObject;
                pill.Button = btn;
                pill.Label = btn.GetComponentInChildren<Text>();
                if (pill.Label != null) pill.Label.fontSize = 11;
                pill.Root.SetActive(false);
                _recentReplayPills.Add(pill);
            }

            _arenaLobbyModal.SetActive(false);
        }

        public string LobbyChassisSpecSnapshot => _arenaLobbyChassisSpecText != null ? _arenaLobbyChassisSpecText.text : string.Empty;
        public int RecentReplayPillCount => _parsedHistoryMatches.Count;

        public bool ClickRecentReplayPillForTests(int index) => HandleRecentReplayPillClick(index);

        private bool HandleRecentReplayPillClick(int index)
        {
            if (index < 0 || index >= _recentReplayPills.Count) return false;
            var pill = _recentReplayPills[index];
            if (pill == null || pill.MatchId == 0 || !pill.Root.activeSelf) return false;
            if (_arenaLobbyModal != null) _arenaLobbyModal.SetActive(false);
            OnCommandRequested?.Invoke($"sim replay {pill.MatchId}");
            return true;
        }

        public static string ResolveChassisTacticalSpecs(string unitId)
        {
            string key = (unitId ?? "line_tank").Trim().ToLowerInvariant().Replace("tacsim.unit.", string.Empty);
            switch (key)
            {
                case "line_tank_laser":
                    return "<color=#66EEFF><b>LASER TANK</b></color> — Front 400 / Turret 350 · Weapons: <b>105mm Cannon + Mk1 Laser + Coax MG</b> · Visual Mk1 · Thermal-managed energy/ballistic hunter.";
                case "heavy_mk1":
                    return "<color=#FFAA55><b>HEAVY MK1</b></color> — Front 700 / Turret 550 · Weapons: <b>105mm Cannon + Long-Range Missiles + Coax MG</b> · Visual Mk1 · Heavy breakthrough juggernaut.";
                case "biped_mk1":
                    return "<color=#77FFAA><b>BIPED MK1</b></color> — Torso 450 / Legs 260 · Weapons: <b>Arm 105mm Cannon + Coax MG</b> · Visual Mk1 · Agile all-terrain combat walker.";
                case "biped_mk2":
                    return "<color=#FF88EE><b>BIPED MK2</b></color> — Torso 520 / Legs 300 · Weapons: <b>105mm Cannon + Mk1 Laser + Coax MG</b> · <b>Thermal Mk1</b> · Heavy thermal-optics assault mech.";
                case "scout_mk1":
                    return "<color=#FFEE66><b>SCOUT MK1</b></color> — Front 200 / Turret 150 · Weapons: <b>30mm Autocannon + Coax MG</b> · Visual Mk1 · High-speed flanker & recon car.";
                default:
                    return "<color=#66EEFF><b>LINE TANK</b></color> — Front 400 / Turret 350 · Weapons: <b>105mm Cannon + Coax MG</b> · Visual Mk1 · Balanced main battle tank.";
            }
        }

        /// <summary>
        /// Parses authoritative `SIM HISTORY` rows (`  #42  red wins [hold]  3m12s  4 v 4 ...`)
        /// into clickable `sim replay <id>` quick-launch pills inside the Simulator Selector Console.
        /// </summary>
        public void IngestSimHistoryOutput(string plainText)
        {
            if (string.IsNullOrWhiteSpace(plainText)) return;
            string[] lines = plainText.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries);
            bool changed = false;

            for (int i = 0; i < lines.Length; i++)
            {
                string trimmed = lines[i].Trim();
                if (trimmed.Length < 8 || trimmed[0] != '#') continue;

                int idx = 1;
                while (idx < trimmed.Length && char.IsDigit(trimmed[idx])) idx++;
                if (idx <= 1 || idx >= trimmed.Length || !char.IsWhiteSpace(trimmed[idx])) continue;

                string numStr = trimmed.Substring(1, idx - 1);
                if (!ulong.TryParse(numStr, out ulong matchId) || matchId == 0) continue;

                string rest = trimmed.Substring(idx).Trim();
                bool looksLikeHistory =
                    rest.IndexOf(" v ", StringComparison.OrdinalIgnoreCase) >= 0 ||
                    rest.IndexOf("wins", StringComparison.OrdinalIgnoreCase) >= 0 ||
                    rest.IndexOf("draw", StringComparison.OrdinalIgnoreCase) >= 0 ||
                    rest.IndexOf("aborted", StringComparison.OrdinalIgnoreCase) >= 0 ||
                    rest.IndexOf("void", StringComparison.OrdinalIgnoreCase) >= 0;
                if (!looksLikeHistory) continue;

                string badge = rest.IndexOf("[hold]", StringComparison.OrdinalIgnoreCase) >= 0 ? "HOLD"
                    : (rest.IndexOf("red", StringComparison.OrdinalIgnoreCase) >= 0 ? "RED"
                        : (rest.IndexOf("blue", StringComparison.OrdinalIgnoreCase) >= 0 ? "BLUE" : "MATCH"));

                bool exists = false;
                for (int e = 0; e < _parsedHistoryMatches.Count; e++)
                {
                    if (_parsedHistoryMatches[e].matchId == matchId)
                    {
                        exists = true;
                        break;
                    }
                }
                if (exists) continue;

                if (_parsedHistoryMatches.Count >= MaxRecentReplayPills)
                    _parsedHistoryMatches.RemoveAt(_parsedHistoryMatches.Count - 1);
                _parsedHistoryMatches.Add((matchId, badge));
                changed = true;
            }

            if (changed)
                RefreshRecentReplayPills();
        }

        private void RefreshRecentReplayPills()
        {
            int count = Mathf.Min(_parsedHistoryMatches.Count, _recentReplayPills.Count);
            if (_arenaLobbyReplayHintText != null)
                _arenaLobbyReplayHintText.gameObject.SetActive(count == 0);

            for (int i = 0; i < _recentReplayPills.Count; i++)
            {
                var pill = _recentReplayPills[i];
                if (pill == null || pill.Root == null) continue;
                if (i < count)
                {
                    var item = _parsedHistoryMatches[i];
                    pill.MatchId = item.matchId;
                    if (pill.Label != null)
                        pill.Label.text = $"REPLAY #{item.matchId} ({item.badge})";
                    pill.Root.SetActive(true);
                }
                else
                {
                    pill.MatchId = 0;
                    pill.Root.SetActive(false);
                }
            }
        }

        public void ShowArenaLobbyModal(bool visible)
        {
            if (_arenaLobbyModal == null) return;
            _arenaLobbyModal.SetActive(visible);
            if (visible) RefreshLobbySelectionHighlights();
        }

        public void RefreshLobbySelectionHighlights()
        {
            if (_arena == null) return;
            int teamIdx = _arena.SelectedLobbyTeam.StartsWith("b", StringComparison.OrdinalIgnoreCase) ? 1 : 0;
            if (_teamBtnImages.Count >= 2)
            {
                _teamBtnImages[0].color = teamIdx == 0 ? new Color(0.92f, 0.24f, 0.22f) : new Color(0.32f, 0.14f, 0.15f);
                _teamBtnImages[1].color = teamIdx == 1 ? new Color(0.22f, 0.56f, 0.96f) : new Color(0.14f, 0.22f, 0.36f);
            }

            string[] uIds = { "line_tank", "line_tank_laser", "heavy_mk1", "biped_mk1", "biped_mk2", "scout_mk1" };
            int uIdx = Mathf.Max(0, Array.IndexOf(uIds, _arena.SelectedLobbyUnit));
            HighlightPillGroup(_unitBtnImages, uIdx, new Color(0.22f, 0.64f, 0.86f));

            if (_arenaLobbyChassisSpecText != null)
                _arenaLobbyChassisSpecText.text = ResolveChassisTacticalSpecs(_arena.SelectedLobbyUnit);

            int[] aiCounts = { 1, 2, 4, 6 };
            int aiIdx = Mathf.Max(0, Array.IndexOf(aiCounts, _arena.SelectedLobbyAiCount));
            HighlightPillGroup(_aiFillBtnImages, aiIdx, new Color(0.22f, 0.64f, 0.86f));

            int mdIdx = _arena.SelectedLobbyMode == "hold" ? 1 : 0;
            HighlightPillGroup(_modeBtnImages, mdIdx, new Color(0.22f, 0.64f, 0.86f));

            if (_arenaLobbySummaryText != null)
            {
                string tColor = teamIdx == 0 ? "#FF6655" : "#55AAFF";
                string mState = string.IsNullOrEmpty(_arena.LiveLobbyMatchState) ? "IDLE" : _arena.LiveLobbyMatchState.ToUpperInvariant();
                _arenaLobbySummaryText.text =
                    $"CENTRE STATE: <b>{MushProtocol.EscapeUnityRichText(mState)}</b>   |   TEAM: <color={tColor}><b>{MushProtocol.EscapeUnityRichText(_arena.SelectedLobbyTeam.ToUpperInvariant())}</b></color>   |   UNIT: <b>{MushProtocol.EscapeUnityRichText(_arena.SelectedLobbyUnit.ToUpperInvariant())}</b>   |   AI SIDE SIZE: <b>{_arena.SelectedLobbyAiCount}</b>   |   RULE: <b>{MushProtocol.EscapeUnityRichText(_arena.SelectedLobbyMode.ToUpperInvariant())}</b>";
            }

            if (_arenaLobbyLiveRosterText != null)
            {
                _arenaLobbyLiveRosterText.text = !string.IsNullOrEmpty(_arena.LiveLobbySlotSummary)
                    ? _arena.LiveLobbySlotSummary
                    : DefaultLobbyRosterHint;
            }

            RefreshRecentReplayPills();
        }

        // ---- OOC SIMULATOR ARENA: ACTIVE 3D TANK/MECH COCKPIT HUD (POOLED CONTACT ROWS) ----

        private void BuildArenaBattleCockpitHud(Transform parent)
        {
            _arenaBattleHudRoot = CreatePanel(parent, "ArenaBattleCockpitHud", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, Color.clear);

            var scorePanel = CreatePanel(_arenaBattleHudRoot.transform, "RedVsBlueScoreboard", new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-340f, -118f), new Vector2(340f, -68f), new Color(0.03f, 0.06f, 0.10f, 0.93f));
            _arenaScoreboardText = CreateTxt(scorePanel.transform, "", 16, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            _arenaScoreboardText.supportRichText = true;
            SetRect(_arenaScoreboardText.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 0f), new Vector2(-10f, 0f));

            _arenaBannerText = CreateTxt(_arenaBattleHudRoot.transform, "", 24, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(1f, 0.88f, 0.35f));
            _arenaBannerText.supportRichText = false;
            SetRect(_arenaBannerText.rectTransform, new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-420f, -206f), new Vector2(420f, -166f));

            var tacCard = CreatePanel(_arenaBattleHudRoot.transform, "ArenaContactsPanel", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-390f, -422f), new Vector2(-16f, -74f), new Color(0.03f, 0.07f, 0.12f, 0.90f));
            var cHdr = CreateTxt(tacCard.transform, "SENSORS & CLICK-TO-LOCK CONTACTS", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.55f, 0.90f, 1f));
            SetRect(cHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -28f), new Vector2(-10f, -4f));

            var spotsBar = CreatePanel(tacCard.transform, "KeySpotsDriveBar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(8f, -62f), new Vector2(-8f, -32f), Color.clear);
            var spHlg = spotsBar.AddComponent<HorizontalLayoutGroup>();
            spHlg.spacing = 5f; spHlg.childControlWidth = true; spHlg.childControlHeight = true; spHlg.childForceExpandWidth = true;
            _arenaKeySpotsRowRoot = spotsBar.transform;

            var listArea = CreatePanel(tacCard.transform, "ContactsButtonsArea", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(8f, 8f), new Vector2(-8f, -68f), Color.clear);
            var cVlg = listArea.AddComponent<VerticalLayoutGroup>();
            cVlg.spacing = 4f;
            cVlg.childControlWidth = true;
            cVlg.childControlHeight = false;
            cVlg.childForceExpandWidth = true;
            cVlg.childForceExpandHeight = false;
            _arenaContactsListRoot = listArea.transform;

            // Pre-allocate pooled contact row buttons (zero GC / zero Destroy churn during 2 Hz cockpit pulses!)
            for (int r = 0; r < MaxPooledContactRows; r++)
            {
                var pooled = new PooledContactRow();
                var btn = CreateBtn(_arenaContactsListRoot, string.Empty, new Color(0.18f, 0.16f, 0.16f, 0.85f), () =>
                {
                    if (pooled.CanLock && MushProtocol.TryValidateSafeCommandToken(pooled.BoundDesignator, out string safeDesig))
                        OnArenaCockpitVerbRequested?.Invoke($"lock {safeDesig}");
                });
                btn.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 28f);
                pooled.Root = btn.gameObject;
                pooled.Button = btn;
                pooled.Background = btn.GetComponent<Image>();
                pooled.Label = btn.GetComponentInChildren<Text>();
                pooled.Root.SetActive(false);
                _contactRowPool.Add(pooled);
            }

            // Locked Target Inspection Paper-Doll Card (docked right edge cleanly below ArenaContactsPanel `-422` and above OwnPlatformCard)
            _targetInspectCard = CreatePanel(_arenaBattleHudRoot.transform, "ArenaTargetInspectCard", new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-390f, -586f), new Vector2(-16f, -430f), new Color(0.04f, 0.07f, 0.12f, 0.93f));
            _targetInspectText = CreateTxt(_targetInspectCard.transform, string.Empty, 11, FontStyle.Normal, TextAnchor.UpperLeft, Color.white);
            _targetInspectText.supportRichText = true;
            SetRect(_targetInspectText.rectTransform, new Vector2(0f, 0.24f), new Vector2(0.64f, 1f), new Vector2(10f, 2f), new Vector2(-4f, -8f));

            _targetScanBtn = CreateBtn(_targetInspectCard.transform, "SCAN TARGET", new Color(0.14f, 0.42f, 0.56f), HandleScanLockedTargetClick);
            SetRect(_targetScanBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0.64f, 0.24f), new Vector2(10f, 8f), new Vector2(-4f, -2f));
            var scanBtnLbl = _targetScanBtn.GetComponentInChildren<Text>();
            if (scanBtnLbl != null) scanBtnLbl.fontSize = 11;

            var tgtDollFrame = CreatePanel(_targetInspectCard.transform, "TargetArmorDollFrame", new Vector2(0.65f, 0f), new Vector2(1f, 1f), new Vector2(2f, 8f), new Vector2(-10f, -8f), new Color(0.02f, 0.04f, 0.08f, 0.96f));
            var tgtDollGo = new GameObject("TargetArmorDollImg", typeof(RectTransform), typeof(RawImage));
            tgtDollGo.transform.SetParent(tgtDollFrame.transform, false);
            _targetArmorDollImg = tgtDollGo.GetComponent<RawImage>();
            _targetArmorDollImg.color = Color.white;
            _targetArmorDollImg.raycastTarget = false;
            SetRect(_targetArmorDollImg.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 6f), new Vector2(-6f, -6f));
            tgtDollGo.SetActive(false);
            _targetInspectCard.SetActive(false);

            var hullCard = CreatePanel(_arenaBattleHudRoot.transform, "OwnPlatformCard", new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-430f, 18f), new Vector2(-16f, 326f), new Color(0.03f, 0.07f, 0.12f, 0.93f));
            _arenaOwnArmorText = CreateTxt(hullCard.transform, "", 11, FontStyle.Normal, TextAnchor.UpperLeft, Color.white);
            _arenaOwnArmorText.supportRichText = true;
            SetRect(_arenaOwnArmorText.rectTransform, new Vector2(0f, 0.62f), new Vector2(0.68f, 1f), new Vector2(10f, 0f), new Vector2(-4f, -8f));

            var dollFrame = CreatePanel(hullCard.transform, "ArmorPaperDollFrame", new Vector2(0.69f, 0.62f), new Vector2(1f, 1f), new Vector2(0f, 4f), new Vector2(-10f, -8f), new Color(0.02f, 0.04f, 0.08f, 0.96f));
            var dollGo = new GameObject("ArmorPaperDollImg", typeof(RectTransform), typeof(RawImage));
            dollGo.transform.SetParent(dollFrame.transform, false);
            _arenaArmorDollImg = dollGo.GetComponent<RawImage>();
            _arenaArmorDollImg.color = Color.white;
            _arenaArmorDollImg.raycastTarget = false;
            SetRect(_arenaArmorDollImg.rectTransform, Vector2.zero, Vector2.one, new Vector2(6f, 6f), new Vector2(-6f, -6f));
            dollGo.SetActive(false);

            // Doc 21 Standing Rules Row: AUTO-LOCK, AUTO-FIRE, ODDS threshold stepper
            var autoRulesRow = CreatePanel(hullCard.transform, "ArenaStandingRulesRow", new Vector2(0f, 0.50f), new Vector2(1f, 0.61f), new Vector2(10f, 0f), new Vector2(-10f, 0f), Color.clear);
            var arHlg = autoRulesRow.AddComponent<HorizontalLayoutGroup>();
            arHlg.spacing = 4f; arHlg.childControlWidth = true; arHlg.childControlHeight = true; arHlg.childForceExpandWidth = true;

            _autoLockRuleBtn = CreateBtn(autoRulesRow.transform, "AUTO-LOCK: OFF [L]", new Color(0.14f, 0.24f, 0.34f), HandleAutoLockRuleClick);
            _autoLockRuleBg = _autoLockRuleBtn.GetComponent<Image>();
            _autoLockRuleLabel = _autoLockRuleBtn.GetComponentInChildren<Text>();
            if (_autoLockRuleLabel != null) _autoLockRuleLabel.fontSize = 10;

            _autoFireRuleBtn = CreateBtn(autoRulesRow.transform, "AUTO-FIRE: OFF", new Color(0.14f, 0.24f, 0.34f), HandleAutoFireRuleClick);
            _autoFireRuleBg = _autoFireRuleBtn.GetComponent<Image>();
            _autoFireRuleLabel = _autoFireRuleBtn.GetComponentInChildren<Text>();
            if (_autoFireRuleLabel != null) _autoFireRuleLabel.fontSize = 10;

            _autoFireOddsBtn = CreateBtn(autoRulesRow.transform, "ODDS >= 60%", new Color(0.20f, 0.28f, 0.38f), CycleAutoFireOddsPreset);
            _autoFireOddsLabel = _autoFireOddsBtn.GetComponentInChildren<Text>();
            if (_autoFireOddsLabel != null) _autoFireOddsLabel.fontSize = 10;

            // Pooled clickable per-weapon fire buttons ([1]..[4])
            var wpnSlotsRow = CreatePanel(hullCard.transform, "ArenaWeaponSlotsRow", new Vector2(0f, 0.36f), new Vector2(1f, 0.49f), new Vector2(10f, 0f), new Vector2(-10f, 0f), Color.clear);
            var wsHlg = wpnSlotsRow.AddComponent<HorizontalLayoutGroup>();
            wsHlg.spacing = 4f; wsHlg.childControlWidth = true; wsHlg.childControlHeight = true; wsHlg.childForceExpandWidth = true;
            for (int wIdx = 0; wIdx < MaxPooledWeaponSlotButtons; wIdx++)
            {
                int capturedIdx = wIdx;
                var wSlot = new PooledWeaponSlotButton();
                var wBtn = CreateBtn(wpnSlotsRow.transform, string.Empty, new Color(0.18f, 0.32f, 0.26f), () => FireWeaponSlotAt(capturedIdx));
                wSlot.Root = wBtn.gameObject;
                wSlot.Button = wBtn;
                wSlot.Background = wBtn.GetComponent<Image>();
                wSlot.Label = wBtn.GetComponentInChildren<Text>();
                if (wSlot.Label != null) wSlot.Label.fontSize = 10;
                wSlot.Root.SetActive(false);
                _weaponSlotPool.Add(wSlot);
            }

            _arenaWeaponText = CreateTxt(hullCard.transform, "", 10, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.65f, 0.95f, 0.80f));
            _arenaWeaponText.supportRichText = true;
            SetRect(_arenaWeaponText.rectTransform, new Vector2(0f, 0.17f), new Vector2(1f, 0.35f), new Vector2(10f, 0f), new Vector2(-10f, 0f));

            var fireBtn = CreateBtn(hullCard.transform, "FIRE MAIN WEAPON [SPACE]", new Color(0.72f, 0.18f, 0.16f), () => OnArenaCockpitVerbRequested?.Invoke("fire"));
            SetRect(fireBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0.64f, 0.17f), new Vector2(10f, 8f), new Vector2(-4f, -2f));
            _fireBtnLabel = fireBtn.GetComponentInChildren<Text>();

            var volleyBtn = CreateBtn(hullCard.transform, "VOLLEY [G]", new Color(0.54f, 0.26f, 0.14f), () => OnArenaCockpitVerbRequested?.Invoke("fire all"));
            SetRect(volleyBtn.GetComponent<RectTransform>(), new Vector2(0.64f, 0f), new Vector2(1f, 0.17f), new Vector2(4f, 8f), new Vector2(-10f, -2f));

            // Anchored strictly between CommsDrawer (x=14..436) and OwnPlatformCard (x=-430..-16) so zero overlap on any aspect ratio.
            var squadBar = CreatePanel(_arenaBattleHudRoot.transform, "ArenaSquadOrderBar", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 16f), new Vector2(-446f, 108f), new Color(0.04f, 0.08f, 0.14f, 0.94f));

            var helmRow = CreatePanel(squadBar.transform, "ArenaHelmRow", new Vector2(0f, 0.5f), new Vector2(1f, 1f), new Vector2(8f, 2f), new Vector2(-8f, -6f), Color.clear);
            var hHlg = helmRow.AddComponent<HorizontalLayoutGroup>();
            hHlg.spacing = 5f; hHlg.childControlWidth = true; hHlg.childControlHeight = true; hHlg.childForceExpandWidth = true;

            CreateBtn(helmRow.transform, "STARTUP", new Color(0.16f, 0.42f, 0.34f), () => OnArenaCockpitVerbRequested?.Invoke("startup"));
            CreateBtn(helmRow.transform, "ALL STOP [X]", new Color(0.36f, 0.26f, 0.16f), () => OnArenaCockpitVerbRequested?.Invoke("speed 0"));
            CreateBtn(helmRow.transform, "SCAN", new Color(0.14f, 0.38f, 0.48f), () => OnArenaCockpitVerbRequested?.Invoke("scan"));
            _emconBtn = CreateBtn(helmRow.transform, "EMCON: ON", new Color(0.22f, 0.28f, 0.44f), () =>
            {
                bool silent = _arena != null && _arena.State.EmconSilent;
                OnArenaCockpitVerbRequested?.Invoke(silent ? "emcon on" : "emcon off");
            });
            _emconBtnLabel = _emconBtn.GetComponentInChildren<Text>();
            CreateBtn(helmRow.transform, "HOLD/FREE", new Color(0.32f, 0.26f, 0.42f), () =>
            {
                bool free = _arena != null && _arena.State.WeaponsFree;
                OnArenaCockpitVerbRequested?.Invoke(free ? "fire hold" : "fire free");
            });

            var squadRow = CreatePanel(squadBar.transform, "ArenaSquadRow", new Vector2(0f, 0f), new Vector2(1f, 0.5f), new Vector2(8f, 6f), new Vector2(-8f, -2f), Color.clear);
            var sHlg = squadRow.AddComponent<HorizontalLayoutGroup>();
            sHlg.spacing = 5f; sHlg.childControlWidth = true; sHlg.childControlHeight = true; sHlg.childForceExpandWidth = true;

            CreateBtn(squadRow.transform, "SQUAD: ENGAGE", new Color(0.52f, 0.24f, 0.16f), () => OnArenaCockpitVerbRequested?.Invoke($"{ResolveSquadAllOrderPrefix()}engage"));
            _advanceBtn = CreateBtn(squadRow.transform, "SQUAD: ADVANCE", new Color(0.46f, 0.34f, 0.14f), () =>
            {
                string advCmd = ResolveSquadAdvanceCommand();
                if (!string.IsNullOrEmpty(advCmd))
                    OnArenaCockpitVerbRequested?.Invoke(advCmd);
            });
            _advanceBtnLabel = _advanceBtn.GetComponentInChildren<Text>();
            CreateBtn(squadRow.transform, "SQUAD: HOLD", new Color(0.20f, 0.36f, 0.46f), () => OnArenaCockpitVerbRequested?.Invoke($"{ResolveSquadAllOrderPrefix()}hold"));
            CreateBtn(squadRow.transform, "CAM VIEW [V]", new Color(0.28f, 0.28f, 0.46f), () => OnToggleVehicleCameraViewRequested?.Invoke());
            var opticsBtn = CreateBtn(squadRow.transform, "OPTICS 1x [Q]", new Color(0.20f, 0.30f, 0.44f), () => OnCycleArenaOpticsRequested?.Invoke());
            _opticsBtnBg = opticsBtn.GetComponent<Image>();
            _opticsBtnLabel = opticsBtn.GetComponentInChildren<Text>();
            CreateBtn(squadRow.transform, "EXIT POD", new Color(0.48f, 0.16f, 0.18f), () => OnArenaCockpitVerbRequested?.Invoke("exit"));

            // Detachment Command Deck (stacked cleanly above ArenaSquadOrderBar, visible only when server projects `detachment`)
            _detachmentBar = CreatePanel(_arenaBattleHudRoot.transform, "ArenaDetachmentBar", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 114f), new Vector2(-446f, 184f), new Color(0.03f, 0.07f, 0.13f, 0.94f));

            _detachmentOrderRow = CreatePanel(_detachmentBar.transform, "DetachmentOrderRow", new Vector2(0f, 0.50f), new Vector2(1f, 1f), new Vector2(8f, 2f), new Vector2(-8f, -4f), new Color(0.08f, 0.12f, 0.18f, 0.92f));
            _detachmentOrderText = CreateTxt(_detachmentOrderRow.transform, "", 12, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
            _detachmentOrderText.supportRichText = true;
            SetRect(_detachmentOrderText.rectTransform, new Vector2(0f, 0f), new Vector2(0.72f, 1f), new Vector2(8f, 0f), new Vector2(-4f, 0f));

            _detachmentWilcoBtn = CreateBtn(_detachmentOrderRow.transform, "WILCO", new Color(0.14f, 0.48f, 0.30f), DispatchDetachmentWilco);
            SetRect(_detachmentWilcoBtn.GetComponent<RectTransform>(), new Vector2(0.73f, 0f), new Vector2(0.86f, 1f), new Vector2(2f, 2f), new Vector2(-2f, -2f));

            _detachmentNegativeBtn = CreateBtn(_detachmentOrderRow.transform, "NEGATIVE", new Color(0.54f, 0.18f, 0.18f), DispatchDetachmentNegative);
            SetRect(_detachmentNegativeBtn.GetComponent<RectTransform>(), new Vector2(0.86f, 0f), new Vector2(1f, 1f), new Vector2(2f, 2f), new Vector2(-2f, -2f));
            _detachmentOrderRow.SetActive(false);

            _detachmentInvalidText = CreateTxt(_detachmentBar.transform, "<color=#FF7766>DETACHMENT PROJECTION INVALID — COMMANDS DISABLED</color>", 12, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(1f, 0.52f, 0.46f));
            _detachmentInvalidText.supportRichText = true;
            SetRect(_detachmentInvalidText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 4f), new Vector2(-10f, -4f));
            _detachmentInvalidText.gameObject.SetActive(false);

            var detSlotsRow = CreatePanel(_detachmentBar.transform, "DetachmentSlotsRow", new Vector2(0f, 0f), new Vector2(1f, 0.50f), new Vector2(8f, 4f), new Vector2(-8f, -2f), Color.clear);
            var detHlg = detSlotsRow.AddComponent<HorizontalLayoutGroup>();
            detHlg.spacing = 4f; detHlg.childControlWidth = true; detHlg.childControlHeight = true; detHlg.childForceExpandWidth = true;

            for (int b = 0; b < MaxPooledDetachmentButtons; b++)
            {
                int capturedIdx = b;
                var slotBtn = new PooledDetachmentSlotButton();
                var btn = CreateBtn(detSlotsRow.transform, string.Empty, new Color(0.14f, 0.28f, 0.40f), () => PrefillDetachmentSlot(capturedIdx));
                slotBtn.Root = btn.gameObject;
                slotBtn.Button = btn;
                slotBtn.Background = btn.GetComponent<Image>();
                slotBtn.Label = btn.GetComponentInChildren<Text>();
                if (slotBtn.Label != null) slotBtn.Label.fontSize = 11;
                slotBtn.Root.SetActive(false);
                _detachmentSlotPool.Add(slotBtn);
            }
            _detachmentBar.SetActive(false);

            // Sealed Mind Profile Switcher & Hull Posture Deck (dynamically stacks above SquadOrderBar or DetachmentBar)
            _mindStrip = CreatePanel(_arenaBattleHudRoot.transform, "ArenaMindStrip", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 114f), new Vector2(-446f, 150f), new Color(0.03f, 0.08f, 0.13f, 0.94f));
            _mindStripRect = _mindStrip.GetComponent<RectTransform>();

            _mindHeaderText = CreateTxt(_mindStrip.transform, "MIND", 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.56f, 0.92f, 1f));
            _mindHeaderText.supportRichText = true;
            SetRect(_mindHeaderText.rectTransform, new Vector2(0f, 0f), new Vector2(0.32f, 1f), new Vector2(10f, 2f), new Vector2(-4f, -2f));

            var mindProfRow = CreatePanel(_mindStrip.transform, "MindProfilesRow", new Vector2(0.32f, 0f), new Vector2(0.72f, 1f), new Vector2(4f, 4f), new Vector2(-4f, -4f), Color.clear);
            var mpHlg = mindProfRow.AddComponent<HorizontalLayoutGroup>();
            mpHlg.spacing = 4f; mpHlg.childControlWidth = true; mpHlg.childControlHeight = true; mpHlg.childForceExpandWidth = true;
            for (int p = 0; p < MaxPooledMindProfiles; p++)
            {
                int capturedP = p;
                var mpBtn = new PooledMindProfileButton();
                var btn = CreateBtn(mindProfRow.transform, string.Empty, new Color(0.14f, 0.30f, 0.42f), () => HandleMindProfileClick(capturedP));
                mpBtn.Root = btn.gameObject;
                mpBtn.Button = btn;
                mpBtn.Background = btn.GetComponent<Image>();
                mpBtn.Label = btn.GetComponentInChildren<Text>();
                if (mpBtn.Label != null) mpBtn.Label.fontSize = 10;
                mpBtn.Root.SetActive(false);
                _mindProfilePool.Add(mpBtn);
            }

            var postureRow = CreatePanel(_mindStrip.transform, "MindPostureRow", new Vector2(0.73f, 0f), new Vector2(0.90f, 1f), new Vector2(4f, 4f), new Vector2(-4f, -4f), Color.clear);
            var posHlg = postureRow.AddComponent<HorizontalLayoutGroup>();
            posHlg.spacing = 4f; posHlg.childControlWidth = true; posHlg.childControlHeight = true; posHlg.childForceExpandWidth = true;
            string[] postureWords = { "return", "guard", "hold" };
            for (int i = 0; i < postureWords.Length; i++)
            {
                string pw = postureWords[i];
                var pBtn = CreateBtn(postureRow.transform, pw.ToUpperInvariant(), new Color(0.16f, 0.24f, 0.34f), () => HandleMindPostureClick(pw));
                var lbl = pBtn.GetComponentInChildren<Text>();
                if (lbl != null) lbl.fontSize = 10;
                _postureButtons[pw] = (pBtn, pBtn.GetComponent<Image>());
            }

            _rulesToggleBtn = CreateBtn(_mindStrip.transform, "RULES", new Color(0.22f, 0.34f, 0.46f), ToggleRulesDrawer);
            SetRect(_rulesToggleBtn.GetComponent<RectTransform>(), new Vector2(0.905f, 0f), new Vector2(1f, 1f), new Vector2(2f, 4f), new Vector2(-6f, -4f));
            _rulesToggleBtnLabel = _rulesToggleBtn.GetComponentInChildren<Text>();
            if (_rulesToggleBtnLabel != null) _rulesToggleBtnLabel.fontSize = 10;
            _mindStrip.SetActive(false);

            // Collapsible Standing Rules & Sealed Mind Inspector Drawer (docks cleanly directly above ArenaMindStrip)
            _rulesDrawer = CreatePanel(_arenaBattleHudRoot.transform, "ArenaRulesDrawer", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, 154f), new Vector2(-446f, 276f), new Color(0.03f, 0.06f, 0.11f, 0.96f));
            _rulesDrawerRect = _rulesDrawer.GetComponent<RectTransform>();
            var rdHdr = CreateTxt(_rulesDrawer.transform, "STANDING RULES (CLICK TO CLEAR) & SEALED MIND INSPECTOR", 10, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.54f, 0.90f, 1f));
            SetRect(rdHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -20f), new Vector2(-10f, -2f));

            var chipsRow = CreatePanel(_rulesDrawer.transform, "StandingRuleChipsRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(8f, -46f), new Vector2(-8f, -22f), Color.clear);
            var rcHlg = chipsRow.AddComponent<HorizontalLayoutGroup>();
            rcHlg.spacing = 4f; rcHlg.childControlWidth = true; rcHlg.childControlHeight = true; rcHlg.childForceExpandWidth = true;
            for (int c = 0; c < MaxPooledStandingRuleChips; c++)
            {
                int capturedIdx = c;
                var chip = new PooledStandingRuleChip();
                var cBtn = CreateBtn(chipsRow.transform, string.Empty, new Color(0.36f, 0.22f, 0.16f), () => HandleRemoveStandingRuleClick(capturedIdx));
                chip.Root = cBtn.gameObject;
                chip.Button = cBtn;
                chip.Label = cBtn.GetComponentInChildren<Text>();
                if (chip.Label != null) chip.Label.fontSize = 10;
                chip.Root.SetActive(false);
                _ruleChipPool.Add(chip);
            }

            _rulesDrawerBodyText = CreateTxt(_rulesDrawer.transform, string.Empty, 10, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.84f, 0.92f, 0.98f));
            _rulesDrawerBodyText.supportRichText = true;
            SetRect(_rulesDrawerBodyText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 6f), new Vector2(-10f, -50f));
            _rulesDrawer.SetActive(false);

            // Live Arena Combat & Kill Feed Ticker (docked cleanly at Mid-Left below TacticalMinimap y=-412, well above CommsDrawer)
            _combatFeedPanel = CreatePanel(_arenaBattleHudRoot.transform, "ArenaCombatFeedPanel", new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(16f, -542f), new Vector2(346f, -424f), new Color(0.03f, 0.06f, 0.10f, 0.86f));
            _combatFeedPanel.GetComponent<Image>().raycastTarget = false;
            var cfHdr = CreateTxt(_combatFeedPanel.transform, "COMBAT FEED", 10, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.54f, 0.88f, 1f));
            SetRect(cfHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(8f, -18f), new Vector2(-8f, -2f));
            var cfList = CreatePanel(_combatFeedPanel.transform, "CombatFeedList", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(8f, 4f), new Vector2(-8f, -20f), Color.clear);
            var cfVlg = cfList.AddComponent<VerticalLayoutGroup>();
            cfVlg.spacing = 2f; cfVlg.childControlWidth = true; cfVlg.childControlHeight = true; cfVlg.childForceExpandWidth = true; cfVlg.childForceExpandHeight = true;
            for (int f = 0; f < MaxCombatFeedLines; f++)
            {
                var line = CreateTxt(cfList.transform, string.Empty, 10, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
                line.supportRichText = true;
                line.gameObject.SetActive(false);
                _combatFeedLines.Add(line);
            }
            _combatFeedPanel.SetActive(false);

            // Wrecked / Post-Match Debrief Card (appears cleanly center-screen when hull wrecked)
            _arenaDebriefCard = CreatePanel(_arenaBattleHudRoot.transform, "ArenaDebriefCard", new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-240f, -72f), new Vector2(240f, 92f), new Color(0.05f, 0.06f, 0.11f, 0.96f));
            CreatePanel(_arenaDebriefCard.transform, "DebriefAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.95f, 0.26f, 0.22f, 0.95f));
            _arenaDebriefText = CreateTxt(_arenaDebriefCard.transform, string.Empty, 12, FontStyle.Normal, TextAnchor.UpperCenter, Color.white);
            _arenaDebriefText.supportRichText = true;
            SetRect(_arenaDebriefText.rectTransform, new Vector2(0f, 0.34f), new Vector2(1f, 1f), new Vector2(14f, 4f), new Vector2(-14f, -12f));

            var debriefBtns = CreatePanel(_arenaDebriefCard.transform, "DebriefActionsRow", new Vector2(0f, 0f), new Vector2(1f, 0.34f), new Vector2(12f, 10f), new Vector2(-12f, -4f), Color.clear);
            var dbHlg = debriefBtns.AddComponent<HorizontalLayoutGroup>();
            dbHlg.spacing = 8f; dbHlg.childControlWidth = true; dbHlg.childControlHeight = true; dbHlg.childForceExpandWidth = true;
            CreateBtn(debriefBtns.transform, "EXIT POD (exit)", new Color(0.62f, 0.18f, 0.18f), HandleDebriefExitClick);
            CreateBtn(debriefBtns.transform, "MATCH HISTORY", new Color(0.18f, 0.34f, 0.48f), () => OnCommandRequested?.Invoke("sim history"));
            CreateBtn(debriefBtns.transform, "KEEP WATCHING", new Color(0.22f, 0.26f, 0.34f), HandleDebriefDismissClick);
            _arenaDebriefCard.SetActive(false);

            _arenaBattleHudRoot.SetActive(false);
        }

        public string ArenaOpticsButtonSnapshot => _opticsBtnLabel != null ? _opticsBtnLabel.text : string.Empty;
        public string ArenaScoreboardSnapshot => _arenaScoreboardText != null ? _arenaScoreboardText.text : string.Empty;

        public void UpdateArenaOpticsButton(int zoomLevel)
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

        public void RefreshArenaHud()
        {
            if (_arenaBattleHudRoot == null) return;
            bool active = _arena != null && _arena.IsArenaActive;
            _arenaBattleHudRoot.SetActive(active);

            if (!active) return;

            var s = _arena.State;
            string clockTag = s.ArenaClockMs > 0
                ? $" · {TacSimArenaReplayModel.FormatClockText(s.ArenaClockMs)}"
                : string.Empty;
            string holdStr = s.Mode == "hold"
                ? $"  |  HOLD ZONE: {(s.HoldControlScore >= 0 ? $"<color=#FF6655>RED +{s.HoldControlScore}%</color>" : $"<color=#55AAFF>BLUE {Mathf.Abs(s.HoldControlScore)}%</color>")}{clockTag}"
                : $"  |  MATCH #{s.MatchId}{clockTag}";

            _arenaScoreboardText.text =
                $"<color=#FF5544>RED ON SENSORS: {s.RedAliveCount}</color>    vs    <color=#55AAFF>BLUE ON SENSORS: {s.BlueAliveCount}</color>{holdStr}";
            _arenaBannerText.text = s.MatchBanner ?? string.Empty;

            string pCol = s.Party == "R" ? "#FF6655" : "#55AAFF";
            string pwrCol = s.Power == "online" ? "#55FF99" : "#FFAA55";
            string gateTag = s.WeaponsFree
                ? "<color=#55FF99>[WEAPONS FREE]</color>"
                : "<color=#FFAA44>[HOLD FIRE]</color>";
            float setKph = _arena != null ? _arena.TargetCruiseSpeedKph : 0f;
            string safeCallsign = MushProtocol.EscapeUnityRichText(s.Callsign ?? string.Empty);
            string safeUnit = MushProtocol.EscapeUnityRichText((s.UnitType ?? string.Empty).Replace("tacsim.unit.", ""));
            string safePower = MushProtocol.EscapeUnityRichText((s.Power ?? string.Empty).ToUpperInvariant());
            _arenaOwnArmorText.text =
                $"<color={pCol}><b>{safeCallsign}</b></color> [{safeUnit}]   PWR: <color={pwrCol}><b>{safePower}</b></color>  {gateTag}\n" +
                $"SPD: <b>{s.SpeedKph:0} kph</b> <color=#99CCEE>(SET {setKph:0})</color>   HDG: <b>{s.HeadingDeg:000}°</b>   HEAT: <b>{s.Heat}/{s.HeatMax}</b>\n" +
                $"ARMOR — F: <b>{s.ArmorFrontPct}%</b> | L: <b>{s.ArmorLeftPct}%</b> | R: <b>{s.ArmorRightPct}%</b> | AFT: <b>{s.ArmorRearPct}%</b>\n" +
                $"INTERNAL STRUCTURE: <b>{s.InternalsPct}%</b>{(s.IsWreck ? "  <color=#FF4444>[WRECKED]</color>" : "")}";

            if (s.Weapons.Count > 0)
            {
                var wsb = new StringBuilder(220);
                int showCount = Mathf.Min(s.Weapons.Count, 4);
                for (int i = 0; i < showCount; i++)
                {
                    if (i > 0) wsb.Append('\n');
                    var w = s.Weapons[i];
                    string rdy = w.Ready ? "<color=#55FF99>RDY</color>" : $"<color=#FFAA44>{w.ReadyInMs / 1000f:F1}s</color>";
                    string ammoTxt = w.RoundsMax > 0 ? $"{w.Rounds}/{w.RoundsMax}" : $"{w.Rounds}";
                    string armTxt = w.ArmedSlot > 0
                        ? $" <color=#77DDFF>[AUTO #{w.ArmedSlot} >={w.ArmedMinPercent}%{(w.ArmedKeeps > 0 ? $" KEEP {w.ArmedKeeps}" : "")}]</color>"
                        : string.Empty;
                    string safeWpnId = MushProtocol.EscapeUnityRichText((w.Id ?? string.Empty).ToUpperInvariant());
                    wsb.Append($"<b>{safeWpnId}</b> [{rdy}] AMMO {ammoTxt}{armTxt}");
                }
                _arenaWeaponText.text = wsb.ToString();
            }
            else
            {
                _arenaWeaponText.text = "WPN: <b>UNARMED SCOUT</b>";
            }

            if (_fireBtnLabel != null)
            {
                string lockTxt = !string.IsNullOrEmpty(s.LockedDesignator)
                    ? $"FIRE AT {s.LockedDesignator} ({s.LockHitPercent}% HIT) [SPACE]"
                    : "FIRE MAIN WEAPON [SPACE]";
                _fireBtnLabel.text = lockTxt;
            }

            if (_emconBtnLabel != null)
                _emconBtnLabel.text = s.EmconSilent ? "EMCON: OFF" : "EMCON: ON";

            RebuildKeySpotButtonsIfChanged();
            UpdateAdvanceButtonVisual();
            UpdateDetachmentCommandDeck(s);
            UpdateMindStrip(s);
            UpdateStandingRulesAndWeaponDeck(s);
            UpdateArmorPaperDoll(s);
            UpdateTargetInspectionCard(s);
            UpdateCombatFeed(s);
            UpdateDebriefCard(s);
            UpdatePooledContactRows(s);
        }

        public void ClickDebriefExitForTests() => HandleDebriefExitClick();
        public void ClickDebriefDismissForTests() => HandleDebriefDismissClick();

        private void HandleDebriefExitClick()
        {
            if (_arenaDebriefCard != null) _arenaDebriefCard.SetActive(false);
            OnArenaCockpitVerbRequested?.Invoke("exit");
        }

        private void HandleDebriefDismissClick()
        {
            if (_arena?.State != null)
                _dismissedDebriefMatchId = _arena.State.MatchId;
            if (_arenaDebriefCard != null) _arenaDebriefCard.SetActive(false);
        }

        private void UpdateDebriefCard(ArenaCockpitState s)
        {
            if (_arenaDebriefCard == null) return;
            bool shouldShow = s != null && s.IsWreck && (s.MatchId == 0 || _dismissedDebriefMatchId != s.MatchId);
            if (_arenaDebriefCard.activeSelf != shouldShow)
                _arenaDebriefCard.SetActive(shouldShow);
            if (!shouldShow || _arenaDebriefText == null) return;

            int shots = s.MatchHits + s.MatchMisses;
            int accPct = shots > 0 ? Mathf.RoundToInt((s.MatchHits * 100f) / shots) : 0;
            string accStr = shots > 0 ? $"{s.MatchHits}/{shots} ({accPct}%)" : "0/0";
            _arenaDebriefText.text =
                $"<color=#FF6655><b>VEHICLE DESTROYED — MATCH #{s.MatchId} DEBRIEF</b></color>\n" +
                $"KILLS: <b>{s.MatchKills}</b>   |   HITS: <b>{accStr}</b>   |   MODE: <b>{MushProtocol.EscapeUnityRichText((s.Mode ?? "skirmish").ToUpperInvariant())}</b>\n" +
                $"<color=#FF7766>RED ALIVE: {s.RedAliveCount}</color>   vs   <color=#66BBFF>BLUE ALIVE: {s.BlueAliveCount}</color>   |   HEAT: {s.Heat}/{s.HeatMax}";
        }

        private void UpdateCombatFeed(ArenaCockpitState s)
        {
            if (_combatFeedPanel == null) return;
            var list = s?.RecentNotices;
            if (list == null || list.Count == 0)
            {
                if (_combatFeedPanel.activeSelf) _combatFeedPanel.SetActive(false);
                return;
            }

            if (!_combatFeedPanel.activeSelf) _combatFeedPanel.SetActive(true);
            int total = list.Count;
            int shown = Mathf.Min(total, _combatFeedLines.Count);
            int startIdx = total - shown;
            for (int i = 0; i < shown; i++)
            {
                var line = _combatFeedLines[i];
                if (line == null) continue;
                line.text = FormatCombatFeedBadge(list[startIdx + i]);
                if (!line.gameObject.activeSelf) line.gameObject.SetActive(true);
            }
            for (int i = shown; i < _combatFeedLines.Count; i++)
            {
                if (_combatFeedLines[i] != null && _combatFeedLines[i].gameObject.activeSelf)
                    _combatFeedLines[i].gameObject.SetActive(false);
            }
        }

        private static string FormatCombatFeedBadge(ArenaCombatNotice n)
        {
            if (n == null) return string.Empty;
            string desig = MushProtocol.EscapeUnityRichText(n.Designator ?? string.Empty);
            string sec = MushProtocol.EscapeUnityRichText((n.Section ?? string.Empty).Replace("_", " ").ToUpperInvariant());
            string wpn = MushProtocol.EscapeUnityRichText((n.Weapon ?? string.Empty).Replace("_", " ").ToUpperInvariant());
            switch (n.Kind)
            {
                case "kill":
                    return $"<color=#FF4444>[KILL]</color> <b>{desig}</b> DESTROYED!";
                case "target_section_lost":
                    return $"<color=#FFAA33>[CRIT]</color> <b>{desig}</b> {sec} DESTROYED";
                case "hit":
                    return $"<color=#77FFAA>[HIT]</color> <b>{desig}</b> {(string.IsNullOrEmpty(sec) ? wpn : sec)}{(n.Spent ? " (SPENT)" : "")}";
                case "miss":
                    return $"<color=#88BBDD>[MISS]</color> {(string.IsNullOrEmpty(wpn) ? "SHOT" : wpn)} -> {desig}";
                case "slot_fired":
                    return $"<color=#66DDFF>[AUTO #{n.Slot}]</color> {wpn} -> <b>{desig}</b> ({n.HitPercent}%)";
                case "struck":
                case "struck_spent":
                    return $"<color=#FF8844>[STRUCK]</color> OWN {(string.IsNullOrEmpty(sec) ? "HULL" : sec)} HIT";
                case "section_lost":
                    return $"<color=#FF3333>[BREACH]</color> LOST {(string.IsNullOrEmpty(sec) ? "SECTION" : sec)}!";
                case "destroyed":
                    return "<color=#FF2222>[WRECK]</color> VEHICLE DESTROYED!";
                case "disabled":
                    return "<color=#FF6633>[IMMOB]</color> MOBILITY DISABLED";
                case "refused":
                case "blocked":
                    string why = MushProtocol.EscapeUnityRichText(!string.IsNullOrEmpty(n.Refusal) ? n.Refusal : n.Cause);
                    string wpnTag = !string.IsNullOrEmpty(wpn) ? $"{wpn} " : string.Empty;
                    string extra = n.DetailSecs > 0.05f
                        ? $" ({n.DetailSecs:0.#}s)"
                        : (n.DetailFloorPercent > 0 ? $" (FLOOR {n.DetailFloorPercent}%)" : string.Empty);
                    return $"<color=#FFAA44>[WARN]</color> {wpnTag}{(string.IsNullOrEmpty(why) ? n.Kind : why).ToUpperInvariant()}{extra}";
                case "emcon_on":
                    return n.DetailRadiusM > 1f
                        ? $"<color=#CC88FF>[ECM]</color> POD ACTIVE ({n.DetailRadiusM:0}m RADIUS)"
                        : "<color=#CC88FF>[ECM]</color> POD ACTIVE";
                case "emcon_off":
                    return "<color=#88CCEE>[ECM]</color> EMCON SILENT";
                case "identified":
                    string wasTag = !string.IsNullOrEmpty(n.Was) ? $"{MushProtocol.EscapeUnityRichText(n.Was)} -> " : string.Empty;
                    return $"<color=#FFEE66>[ID]</color> {wasTag}<b>{desig}</b>";
                case "unidentified_again":
                    return $"<color=#CCAA66>[ID]</color> LOST ID ON {desig}";
                case "receiver_degraded":
                    return "<color=#FFCC44>[ECM]</color> SENSOR REACH DEGRADED";
                case "receiver_clear":
                    return "<color=#66EEAA>[ECM]</color> RECEIVER CLEAR";
                case "drive_arrived":
                    return "<color=#66EEFF>[NAV]</color> WAYPOINT ARRIVED";
                case "drive_no_route":
                    return "<color=#FF9955>[NAV]</color> NO ROUTE TO WAYPOINT";
                case "locked":
                    return $"<color=#FFEE66>[LOCK]</color> ACQUIRED <b>{desig}</b>";
                case "lock_lost":
                    return $"<color=#AA9988>[LOCK]</color> LOST {desig}";
                default:
                    string safeText = MushProtocol.EscapeUnityRichText(n.Text ?? n.Kind ?? string.Empty);
                    if (safeText.Length > 42) safeText = safeText.Substring(0, 42) + "...";
                    return $"<color=#AACCDE>[TAC]</color> {safeText}";
            }
        }

        public void ClickAutoLockToggleForTests() => HandleAutoLockRuleClick();
        public void ClickAutoFireToggleForTests() => HandleAutoFireRuleClick();
        public void CycleAutoFireOddsForTests() => CycleAutoFireOddsPreset();
        public void ClickWeaponSlotButtonForTests(int index) => FireWeaponSlotAt(index);
        public void ClickScanLockedTargetForTests() => HandleScanLockedTargetClick();
        public void ClickMindProfileForTests(int index) => HandleMindProfileClick(index);
        public void ClickMindPostureForTests(string postureWord) => HandleMindPostureClick(postureWord);
        public void ToggleRulesDrawerForTests() => ToggleRulesDrawer();
        public void ClickRemoveStandingRulePillForTests(int index) => HandleRemoveStandingRuleClick(index);

        private void ToggleRulesDrawer()
        {
            _rulesDrawerOpen = !_rulesDrawerOpen;
            if (_arena?.State != null)
                UpdateRulesDrawer(_arena.State);
            else if (_rulesDrawer != null)
                _rulesDrawer.SetActive(_rulesDrawerOpen);
        }

        private void HandleRemoveStandingRuleClick(int index)
        {
            if (_arena == null || index < 0 || index >= _ruleChipPool.Count) return;
            int slot = _ruleChipPool[index].BoundSlot;
            if (slot > 0)
                _arena.ClearStandingRuleSlot(slot);
        }

        private void HandleMindProfileClick(int index)
        {
            if (_arena == null || index < 0 || index >= _mindProfilePool.Count) return;
            string prof = _mindProfilePool[index].BoundProfileName;
            if (!string.IsNullOrEmpty(prof))
                _arena.SwitchMindProfile(prof);
        }

        private void HandleMindPostureClick(string postureWord)
        {
            _arena?.SetHullPosture(postureWord);
        }

        private void UpdateMindStrip(ArenaCockpitState s)
        {
            if (_mindStrip == null) return;
            var mind = s?.Mind;
            bool hasRulesOrOrder = s != null && (s.Rules.Count > 0 || s.StandingOrder != null);
            if (mind == null && !hasRulesOrOrder)
            {
                if (_mindStrip.activeSelf) _mindStrip.SetActive(false);
                if (_rulesDrawer != null && _rulesDrawer.activeSelf) _rulesDrawer.SetActive(false);
                return;
            }

            // Dynamically stack cleanly above DetachmentBar when present, else directly above SquadOrderBar
            float yBot = (_detachmentBar != null && _detachmentBar.activeSelf) ? 190f : 114f;
            if (_mindStripRect != null)
                SetRect(_mindStripRect, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, yBot), new Vector2(-446f, yBot + 36f));

            if (!_mindStrip.activeSelf) _mindStrip.SetActive(true);

            if (_mindHeaderText != null)
            {
                if (mind != null)
                {
                    string safeName = string.IsNullOrEmpty(mind.Name)
                        ? "OWN MIND"
                        : MushProtocol.EscapeUnityRichText(mind.Name.ToUpperInvariant());
                    string classBadge = !string.IsNullOrEmpty(mind.ClassWord)
                        ? $" [{MushProtocol.EscapeUnityRichText(mind.ClassWord)}/{MushProtocol.EscapeUnityRichText(string.IsNullOrEmpty(mind.CoreWord) ? "-" : mind.CoreWord)}]"
                        : string.Empty;
                    string stateTag = mind.IsOff ? " <color=#FF9966>[ROM]</color>" : string.Empty;
                    _mindHeaderText.text = $"MIND: <b>{safeName}</b>{classBadge}{stateTag}";
                }
                else
                {
                    _mindHeaderText.text = $"STANDING RULES ({s.Rules.Count})";
                }
            }

            int shown = mind != null ? Mathf.Min(mind.Profiles.Count, _mindProfilePool.Count) : 0;
            for (int i = 0; i < shown; i++)
            {
                var slot = _mindProfilePool[i];
                string pName = mind.Profiles[i];
                bool isActive = i == mind.ActiveIndex;
                slot.BoundProfileName = pName;
                if (slot.Label != null)
                    slot.Label.text = isActive ? $"● {pName.ToUpperInvariant()}" : pName.ToUpperInvariant();
                if (slot.Background != null)
                    slot.Background.color = isActive ? new Color(0.14f, 0.52f, 0.34f) : new Color(0.14f, 0.28f, 0.42f);
                if (slot.Button != null)
                    slot.Button.interactable = !isActive;
                if (slot.Root != null && !slot.Root.activeSelf)
                    slot.Root.SetActive(true);
            }
            for (int i = shown; i < _mindProfilePool.Count; i++)
            {
                _mindProfilePool[i].BoundProfileName = string.Empty;
                if (_mindProfilePool[i].Root != null && _mindProfilePool[i].Root.activeSelf)
                    _mindProfilePool[i].Root.SetActive(false);
            }

            string curPosture = (s.Posture ?? "return").Trim().ToLowerInvariant();
            foreach (var kv in _postureButtons)
            {
                bool isCur = string.Equals(kv.Key, curPosture, StringComparison.OrdinalIgnoreCase);
                if (kv.Value.bg != null)
                    kv.Value.bg.color = isCur ? new Color(0.48f, 0.36f, 0.14f) : new Color(0.14f, 0.22f, 0.32f);
                if (kv.Value.btn != null)
                    kv.Value.btn.interactable = !isCur;
            }

            if (_rulesToggleBtnLabel != null)
                _rulesToggleBtnLabel.text = s.Rules.Count > 0 ? $"RULES ({s.Rules.Count})" : "RULES";

            UpdateRulesDrawer(s, yBot + 40f);
        }

        private void UpdateRulesDrawer(ArenaCockpitState s, float? forcedBottomY = null)
        {
            if (_rulesDrawer == null) return;
            bool show = _rulesDrawerOpen && s != null;
            if (_rulesDrawer.activeSelf != show)
                _rulesDrawer.SetActive(show);
            if (!show) return;

            if (_rulesDrawerRect != null)
            {
                float yBot = forcedBottomY ?? ((_detachmentBar != null && _detachmentBar.activeSelf) ? 230f : 154f);
                SetRect(_rulesDrawerRect, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(452f, yBot), new Vector2(-446f, yBot + 122f));
            }

            int ruleCount = Mathf.Min(s.Rules.Count, _ruleChipPool.Count);
            for (int i = 0; i < ruleCount; i++)
            {
                var r = s.Rules[i];
                var chip = _ruleChipPool[i];
                chip.BoundSlot = r.Slot;
                string extra = !string.IsNullOrEmpty(r.Weapon)
                    ? $" {r.Weapon.ToUpperInvariant()}"
                    : (r.MinPercent > 0 ? $" >={r.MinPercent}%" : string.Empty);
                if (chip.Label != null)
                    chip.Label.text = $"S{r.Slot} {r.Action.ToUpperInvariant()}{extra} ×";
                if (chip.Root != null && !chip.Root.activeSelf)
                    chip.Root.SetActive(true);
            }
            for (int i = ruleCount; i < _ruleChipPool.Count; i++)
            {
                _ruleChipPool[i].BoundSlot = 0;
                if (_ruleChipPool[i].Root != null && _ruleChipPool[i].Root.activeSelf)
                    _ruleChipPool[i].Root.SetActive(false);
            }

            if (_rulesDrawerBodyText != null)
            {
                var sb = new StringBuilder(240);
                if (s.StandingOrder != null)
                {
                    var ord = s.StandingOrder;
                    string objUp = MushProtocol.EscapeUnityRichText((ord.Objective ?? string.Empty).ToUpperInvariant());
                    string dest = ord.HasToCell
                        ? $" -> ({ord.ToCellX},{ord.ToCellY})"
                        : (!string.IsNullOrEmpty(ord.Target) ? $" -> {MushProtocol.EscapeUnityRichText(ord.Target)}" : string.Empty);
                    string hf = ord.HoldFire ? " <color=#FFAA44>[HOLD FIRE]</color>" : string.Empty;
                    sb.Append($"<color=#FFEE66>ORDER:</color> <b>{objUp}</b>{dest}{hf}\n");
                }
                if (s.Mind != null)
                {
                    string schema = !string.IsNullOrEmpty(s.Mind.CardSchema) ? $" ({MushProtocol.EscapeUnityRichText(s.Mind.CardSchema)})" : string.Empty;
                    if (s.Mind.SealedRules.Count > 0)
                    {
                        sb.Append($"<color=#77DDFF>SEALED MIND{schema}:</color> ");
                        int cap = Mathf.Min(s.Mind.SealedRules.Count, 4);
                        for (int i = 0; i < cap; i++)
                        {
                            if (i > 0) sb.Append("  |  ");
                            sb.Append(MushProtocol.EscapeUnityRichText(s.Mind.SealedRules[i]));
                        }
                    }
                    else
                    {
                        sb.Append($"<color=#77DDFF>SEALED MIND{schema}:</color> No sealed rules listed.");
                    }
                }
                else if (s.Rules.Count == 0)
                {
                    sb.Append("No active standing rules. Toggle AUTO-LOCK or AUTO-FIRE to arm.");
                }
                _rulesDrawerBodyText.text = sb.ToString();
            }
        }

        private void HandleScanLockedTargetClick()
        {
            if (_arena == null || _arena.State == null) return;
            string desig = _arena.State.LockedDesignator;
            if (!MushProtocol.TryValidateSafeCommandToken(desig, out string safeDesig))
                return;
            OnArenaCockpitVerbRequested?.Invoke($"scan {safeDesig}");
        }

        private void UpdateTargetInspectionCard(ArenaCockpitState s)
        {
            if (_targetInspectCard == null) return;
            if (_arena == null || s == null || !_arena.TryGetLockedContact(out var ct) || ct == null)
            {
                if (_targetInspectCard.activeSelf) _targetInspectCard.SetActive(false);
                return;
            }

            if (!_targetInspectCard.activeSelf) _targetInspectCard.SetActive(true);

            string safeDesig = MushProtocol.EscapeUnityRichText(ct.Designator ?? string.Empty);
            string safeClass = MushProtocol.EscapeUnityRichText((ct.ClassLabel ?? "unit").ToUpperInvariant());
            string aspectUpper = (ct.Aspect ?? "front").ToUpperInvariant();
            string aspectCol = aspectUpper == "REAR"
                ? "#66FF99"
                : (aspectUpper == "LEFT" || aspectUpper == "RIGHT") ? "#FFCC55" : "#99CCEE";
            string aspectBadge = aspectUpper == "REAR"
                ? $"<color={aspectCol}><b>REAR EXPOSED</b></color>"
                : $"<color={aspectCol}><b>{aspectUpper}</b></color>";
            string bandTag = !string.IsNullOrEmpty(ct.Band) ? $" ({MushProtocol.EscapeUnityRichText(ct.Band.ToUpperInvariant())})" : string.Empty;
            string moveTag = ct.IsWreck
                ? "<color=#FF5555>[WRECK]</color>"
                : (ct.Moving ? "<color=#66EEFF>[MOVING]</color>" : "<color=#8899AA>[STATIC]</color>");
            string ageTag = ct.AgeMs >= 1000 ? $"{ct.AgeMs / 1000f:F1}s" : $"{ct.AgeMs}ms";

            var sb = new StringBuilder(240);
            sb.Append($"<color=#FFEE66><b>LOCK {safeDesig}</b></color> [{safeClass}] {moveTag}\n");
            sb.Append($"RNG: <b>{(int)ct.RangeM}m</b>{bandTag}  |  ASPECT: {aspectBadge}  <color=#88A0B8>({ageTag})</color>\n");

            var scanSections = _arena.TryGetContactScanSections(ct);
            bool hasScan = scanSections != null && scanSections.Count > 0 && !string.IsNullOrEmpty(ct.ConditionBands);
            if (hasScan)
            {
                sb.Append("SCAN: ");
                int lim = Mathf.Min(scanSections.Count, ct.ConditionBands.Length);
                for (int i = 0; i < lim; i++)
                {
                    if (i > 0) sb.Append(' ');
                    char b = char.ToLowerInvariant(ct.ConditionBands[i]);
                    string bc = b == 'g' ? "#66EE88" : b == 'y' ? "#FFD255" : b == 'r' ? "#FF6655" : "#888894";
                    string sLabel = MushProtocol.EscapeUnityRichText(scanSections[i].Replace("_", " ").ToUpperInvariant());
                    if (sLabel.Length > 7) sLabel = sLabel.Substring(0, 7);
                    sb.Append($"<color={bc}>{sLabel}:{char.ToUpperInvariant(b)}</color>");
                }
                sb.Append('\n');
            }
            else
            {
                sb.Append("<color=#88A8C0>NO HULL SCAN — CLICK SCAN TARGET</color>\n");
            }

            if (ct.FatalSections != null && ct.FatalSections.Count > 0)
            {
                sb.Append("<color=#FF8866>FATAL:</color> ");
                for (int i = 0; i < ct.FatalSections.Count; i++)
                {
                    if (i > 0) sb.Append(", ");
                    sb.Append(MushProtocol.EscapeUnityRichText(ct.FatalSections[i].ToUpperInvariant()));
                }
            }
            else
            {
                sb.Append($"HIT ODDS: <b>{s.LockHitPercent}%</b>  |  HEALTH: <b>{MushProtocol.EscapeUnityRichText((ct.HealthWord ?? "UNKNOWN").ToUpperInvariant())}</b>");
            }

            if (_targetInspectText != null)
                _targetInspectText.text = sb.ToString();

            if (_targetScanBtn != null)
                _targetScanBtn.interactable = !ct.IsWreck;

            if (_targetArmorDollImg != null)
            {
                string unitKey = !string.IsNullOrEmpty(ct.UnitType) ? ct.UnitType : ct.ClassLabel;
                var doll = TacSimArmorPaperDoll.FindForUnitType(unitKey);
                if (doll != null)
                {
                    string sig = $"{unitKey}|{(ct.IsWreck ? 'W' : 'A')}|{ct.ConditionTable}|{ct.ConditionBands}";
                    if (!string.Equals(sig, _lastTargetDollSignature, StringComparison.Ordinal) || _targetArmorDollTex == null)
                    {
                        _lastTargetDollSignature = sig;
                        _targetArmorDollTex = TacSimArmorPaperDoll.RenderContactScanToTexture(
                            doll, scanSections, ct.ConditionBands, ct.IsWreck, _targetArmorDollTex, ref _targetArmorDollPixels);
                        _targetArmorDollImg.texture = _targetArmorDollTex;
                    }
                    if (!_targetArmorDollImg.gameObject.activeSelf)
                        _targetArmorDollImg.gameObject.SetActive(true);
                }
                else if (_targetArmorDollImg.gameObject.activeSelf)
                {
                    _targetArmorDollImg.gameObject.SetActive(false);
                }
            }
        }

        private void HandleAutoLockRuleClick()
        {
            _arena?.ToggleAutoLockRule();
        }

        private void HandleAutoFireRuleClick()
        {
            _arena?.ToggleAutoFireRule(_autoFireOddsPct, "ALL");
        }

        private void CycleAutoFireOddsPreset()
        {
            int idx = Array.IndexOf(AutoFireOddsPresets, _autoFireOddsPct);
            _autoFireOddsPct = AutoFireOddsPresets[(idx + 1) % AutoFireOddsPresets.Length];
            if (_autoFireOddsLabel != null)
                _autoFireOddsLabel.text = $"ODDS >= {_autoFireOddsPct}%";
            if (_arena != null && _arena.HasAutoFireRule)
                _arena.SetAutoFireRule(_autoFireOddsPct, enable: true, weaponId: "ALL");
        }

        private void FireWeaponSlotAt(int index)
        {
            if (index < 0 || index >= _weaponSlotPool.Count) return;
            string wpnId = _weaponSlotPool[index].BoundWeaponId;
            if (!MushProtocol.TryValidateSafeCommandToken(wpnId, out string safeWpn, maxLen: 32))
                return;
            OnArenaCockpitVerbRequested?.Invoke($"fire {safeWpn}");
        }

        private void UpdateStandingRulesAndWeaponDeck(ArenaCockpitState s)
        {
            if (s == null) return;

            bool autoLockOn = _arena != null && _arena.HasAutoLockRule;
            if (_autoLockRuleLabel != null)
                _autoLockRuleLabel.text = autoLockOn ? "AUTO-LOCK: ON [L]" : "AUTO-LOCK: OFF [L]";
            if (_autoLockRuleBg != null)
                _autoLockRuleBg.color = autoLockOn ? new Color(0.14f, 0.52f, 0.36f) : new Color(0.14f, 0.24f, 0.34f);

            bool autoFireOn = _arena != null && _arena.HasAutoFireRule;
            if (_autoFireRuleLabel != null)
                _autoFireRuleLabel.text = autoFireOn ? "AUTO-FIRE: ON" : "AUTO-FIRE: OFF";
            if (_autoFireRuleBg != null)
                _autoFireRuleBg.color = autoFireOn ? new Color(0.64f, 0.30f, 0.14f) : new Color(0.14f, 0.24f, 0.34f);

            if (_autoFireOddsLabel != null)
                _autoFireOddsLabel.text = $"ODDS >= {_autoFireOddsPct}%";

            int shown = Mathf.Min(s.Weapons.Count, _weaponSlotPool.Count);
            for (int i = 0; i < shown; i++)
            {
                var w = s.Weapons[i];
                var slot = _weaponSlotPool[i];
                slot.BoundWeaponId = w.Id ?? string.Empty;
                string shortName = (w.Id ?? $"W{i + 1}").ToUpperInvariant();
                if (shortName.Length > 9) shortName = shortName.Substring(0, 9);
                string rdyTxt = w.Ready ? "RDY" : $"{w.ReadyInMs / 1000f:F1}s";
                if (slot.Label != null)
                    slot.Label.text = $"[{i + 1}] {shortName} {rdyTxt}";
                if (slot.Background != null)
                {
                    slot.Background.color = w.ArmedSlot > 0
                        ? new Color(0.16f, 0.38f, 0.54f)
                        : (w.Ready ? new Color(0.18f, 0.42f, 0.28f) : new Color(0.28f, 0.22f, 0.16f));
                }
                if (slot.Root != null && !slot.Root.activeSelf)
                    slot.Root.SetActive(true);
            }
            for (int i = shown; i < _weaponSlotPool.Count; i++)
            {
                _weaponSlotPool[i].BoundWeaponId = string.Empty;
                if (_weaponSlotPool[i].Root != null && _weaponSlotPool[i].Root.activeSelf)
                    _weaponSlotPool[i].Root.SetActive(false);
            }
        }

        private void UpdateArmorPaperDoll(ArenaCockpitState s)
        {
            if (_arenaArmorDollImg == null || s == null) return;
            var doll = TacSimArmorPaperDoll.FindForUnitType(s.UnitType);
            if (doll == null)
            {
                if (_arenaArmorDollImg.gameObject.activeSelf) _arenaArmorDollImg.gameObject.SetActive(false);
                return;
            }

            var sigSb = new StringBuilder(160);
            sigSb.Append(s.UnitType ?? string.Empty).Append('|').Append(s.IsWreck ? 'W' : 'A').Append('|');
            if (s.Sections.Count > 0)
            {
                for (int i = 0; i < s.Sections.Count; i++)
                {
                    var sec = s.Sections[i];
                    sigSb.Append(sec.Id).Append(':').Append(sec.Condition).Append(':').Append(sec.Internals).Append('/').Append(sec.InternalsMax);
                    for (int f = 0; f < sec.Facets.Count; f++)
                        sigSb.Append(',').Append(sec.Facets[f].Id).Append('=').Append(sec.Facets[f].Armor).Append('/').Append(sec.Facets[f].ArmorMax);
                    sigSb.Append(';');
                }
            }
            else
            {
                sigSb.Append(s.ArmorFrontPct).Append(',').Append(s.ArmorLeftPct).Append(',').Append(s.ArmorRightPct).Append(',').Append(s.ArmorRearPct).Append(',').Append(s.InternalsPct);
            }

            string sig = sigSb.ToString();
            if (!string.Equals(sig, _lastArmorDollSignature, StringComparison.Ordinal) || _arenaArmorDollTex == null)
            {
                _lastArmorDollSignature = sig;
                ActiveArmorDollUnitType = s.UnitType ?? string.Empty;
                _arenaArmorDollTex = TacSimArmorPaperDoll.RenderToTexture(doll, s.Sections, s, _arenaArmorDollTex, ref _arenaArmorDollPixels);
                _arenaArmorDollImg.texture = _arenaArmorDollTex;
            }
            if (!_arenaArmorDollImg.gameObject.activeSelf)
                _arenaArmorDollImg.gameObject.SetActive(true);
        }

        private void UpdateDetachmentCommandDeck(ArenaCockpitState s)
        {
            if (_detachmentBar == null) return;
            var det = s?.Detachment;
            bool invalid = s != null && s.DetachmentProjectionInvalid;

            if (det == null && !invalid)
            {
                if (_detachmentBar.activeSelf) _detachmentBar.SetActive(false);
                return;
            }

            if (!_detachmentBar.activeSelf) _detachmentBar.SetActive(true);

            if (invalid || det == null)
            {
                if (_detachmentInvalidText != null) _detachmentInvalidText.gameObject.SetActive(true);
                if (_detachmentOrderRow != null) _detachmentOrderRow.SetActive(false);
                for (int i = 0; i < _detachmentSlotPool.Count; i++)
                {
                    _detachmentSlotPool[i].BoundPrefill = null;
                    if (_detachmentSlotPool[i].Root != null && _detachmentSlotPool[i].Root.activeSelf)
                        _detachmentSlotPool[i].Root.SetActive(false);
                }
                return;
            }

            if (_detachmentInvalidText != null) _detachmentInvalidText.gameObject.SetActive(false);

            // 1. Incoming Order Banner + WILCO / NEGATIVE controls
            if (_detachmentOrderRow != null)
            {
                bool hasOrder = det.MyOrder != null;
                _detachmentOrderRow.SetActive(hasOrder);
                if (hasOrder)
                {
                    string safeFrom = MushProtocol.EscapeUnityRichText((det.MyOrder.From ?? string.Empty).ToUpperInvariant());
                    string safeObj = MushProtocol.EscapeUnityRichText(det.MyOrder.Objective ?? string.Empty);
                    string badge = det.MyOrder.Response == "wilco"
                        ? "<color=#55FF99>[WILCO]</color>"
                        : det.MyOrder.Response == "negative"
                            ? "<color=#FF7766>[NEGATIVE]</color>"
                            : "<color=#FFDD55>[AWAITING ACK]</color>";
                    if (_detachmentOrderText != null)
                        _detachmentOrderText.text = $"<color=#77CCFF>ORDER {safeFrom}:</color> {safeObj}  {badge}";
                    if (_detachmentWilcoBtn != null)
                        _detachmentWilcoBtn.interactable = !string.IsNullOrEmpty(det.WilcoCommand) && det.MyOrder.Response != "wilco";
                    if (_detachmentNegativeBtn != null)
                        _detachmentNegativeBtn.interactable = !string.IsNullOrEmpty(det.NegativeCommand) && det.MyOrder.Response != "negative";
                }
            }

            // 2. Pooled Slot Prefill Buttons: index 0 = [ALL], index 1..N = detachment slots
            int shown = 0;
            if (_detachmentSlotPool.Count > 0)
            {
                var allBtn = _detachmentSlotPool[0];
                allBtn.BoundPrefill = det.AllOrderPrefill;
                if (allBtn.Label != null) allBtn.Label.text = $"ALL [DT:{det.Id}]";
                if (allBtn.Background != null) allBtn.Background.color = new Color(0.24f, 0.36f, 0.52f);
                if (allBtn.Root != null && !allBtn.Root.activeSelf) allBtn.Root.SetActive(true);
                shown = 1;
            }

            for (int i = 0; i < det.Slots.Count && shown < _detachmentSlotPool.Count; i++)
            {
                var sl = det.Slots[i];
                var btn = _detachmentSlotPool[shown];
                bool isLeader = det.LeaderSlot.HasValue && det.LeaderSlot.Value == sl.Slot;
                string ctrlTag = sl.Controller == "player" ? "P" : sl.Controller == "npc" ? "NPC" : "AI";
                btn.BoundPrefill = sl.OrderPrefill;
                if (btn.Label != null)
                    btn.Label.text = $"{sl.Slot} {sl.Callsign}{(isLeader ? "*" : "")} [{ctrlTag}]";
                if (btn.Background != null)
                {
                    btn.Background.color = isLeader
                        ? new Color(0.46f, 0.34f, 0.14f)
                        : sl.Controller == "player"
                            ? new Color(0.14f, 0.38f, 0.48f)
                            : new Color(0.16f, 0.22f, 0.30f);
                }
                if (btn.Root != null && !btn.Root.activeSelf) btn.Root.SetActive(true);
                shown++;
            }

            for (int r = shown; r < _detachmentSlotPool.Count; r++)
            {
                _detachmentSlotPool[r].BoundPrefill = null;
                if (_detachmentSlotPool[r].Root != null && _detachmentSlotPool[r].Root.activeSelf)
                    _detachmentSlotPool[r].Root.SetActive(false);
            }
        }

        private void RebuildKeySpotButtonsIfChanged()
        {
            if (_arenaKeySpotsRowRoot == null || _arena?.TerrainGrid == null) return;
            var spots = _arena.TerrainGrid.KeySpots;
            string sig = $"{_arena.TerrainGrid.Id}:{spots.Count}";
            if (sig == _lastKeySpotsSignature) return;
            _lastKeySpotsSignature = sig;

            for (int i = _arenaKeySpotsRowRoot.childCount - 1; i >= 0; i--)
            {
                var ch = _arenaKeySpotsRowRoot.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }

            for (int i = 0; i < Mathf.Min(4, spots.Count); i++)
            {
                string letter = spots[i].Letter;
                if (!MushProtocol.TryValidateSafeCommandToken(letter, out string safeLetter, maxLen: 16))
                    continue;
                CreateBtn(_arenaKeySpotsRowRoot, $"DRIVE [{safeLetter}]", new Color(0.14f, 0.34f, 0.44f), () =>
                {
                    SelectAdvanceSpot(safeLetter);
                    OnArenaCockpitVerbRequested?.Invoke($"drive {safeLetter}");
                });
            }
            UpdateAdvanceButtonVisual();
        }

        private void UpdatePooledContactRows(ArenaCockpitState s)
        {
            if (_contactRowPool.Count == 0) return;

            int shown = 0;
            for (int pass = 0; pass < 2 && shown < _contactRowPool.Count; pass++)
            {
                for (int i = 0; i < s.Contacts.Count && shown < _contactRowPool.Count; i++)
                {
                    var c = s.Contacts[i];
                    if (pass == 0 && (c.Friendly || c.IsWreck)) continue;
                    if (pass == 1 && (!c.Friendly && !c.IsWreck)) continue;

                    bool isLocked = string.Equals(s.LockedDesignator, c.Designator, StringComparison.OrdinalIgnoreCase);
                    Color rowBg = c.IsWreck ? new Color(0.18f, 0.16f, 0.16f, 0.85f)
                                : isLocked ? new Color(0.62f, 0.38f, 0.10f, 0.96f)
                                : c.Friendly ? new Color(0.10f, 0.24f, 0.36f, 0.90f)
                                : new Color(0.42f, 0.14f, 0.16f, 0.92f);

                    string odds = (!c.Friendly && !c.IsWreck) ? $" | {c.HitOddsPct}%" : string.Empty;
                    string label = $"{(isLocked ? ">> " : "")}{c.Designator} [{c.ClassLabel}] {(int)c.RangeM}m {c.HealthWord.ToUpperInvariant()}{odds}";

                    var slot = _contactRowPool[shown];
                    slot.BoundDesignator = c.Designator;
                    slot.CanLock = !c.Friendly && !c.IsWreck;
                    if (slot.Background != null) slot.Background.color = rowBg;
                    if (slot.Label != null) slot.Label.text = label;
                    if (slot.Root != null && !slot.Root.activeSelf) slot.Root.SetActive(true);
                    shown++;
                }
            }

            for (int r = shown; r < _contactRowPool.Count; r++)
            {
                if (_contactRowPool[r].Root != null && _contactRowPool[r].Root.activeSelf)
                    _contactRowPool[r].Root.SetActive(false);
            }
        }

        // ---- Small uGUI Helpers ----

        private Transform CreateRow(Transform parent, string name, float yMin, float yMax)
        {
            var p = CreatePanel(parent, name, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, yMin), new Vector2(-20f, yMax), Color.clear);
            var hlg = p.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;
            return p.transform;
        }

        private static void HighlightPillGroup(List<Image> imgs, int selectedIdx, Color activeCol)
        {
            for (int i = 0; i < imgs.Count; i++)
            {
                if (imgs[i] == null) continue;
                imgs[i].color = i == selectedIdx ? activeCol : new Color(0.13f, 0.23f, 0.33f, 0.92f);
            }
        }

        private static GameObject CreatePanel(Transform parent, string name, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), anchorMin, anchorMax, offsetMin, offsetMax);
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

            var txt = CreateTxt(go.transform, label, 12, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
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
