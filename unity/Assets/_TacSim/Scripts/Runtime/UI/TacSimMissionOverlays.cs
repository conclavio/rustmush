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
        public event Action OnToggleVehicleCameraViewRequested;

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
        private Text _arenaLobbyLiveRosterText;
        private readonly List<Image> _teamBtnImages = new List<Image>();
        private readonly List<Image> _unitBtnImages = new List<Image>();
        private readonly List<Image> _aiFillBtnImages = new List<Image>();
        private readonly List<Image> _modeBtnImages = new List<Image>();

        // OOC Arena Active 3D Battlefield Cockpit HUD (with Pooled Contact Rows!)
        private GameObject _arenaBattleHudRoot;
        private Text _arenaScoreboardText;
        private Text _arenaBannerText;
        private Text _arenaOwnArmorText;
        private Text _arenaWeaponText;
        private Text _fireBtnLabel;
        private Button _emconBtn;
        private Text _emconBtnLabel;
        private Button _advanceBtn;
        private Text _advanceBtnLabel;
        private string _selectedAdvanceSpot;
        private Transform _arenaContactsListRoot;
        private Transform _arenaKeySpotsRowRoot;
        private string _lastKeySpotsSignature = string.Empty;

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

        public bool IsLobbyModalOpen => _arenaLobbyModal != null && _arenaLobbyModal.activeSelf;
        public bool IsSpaceFireCardVisible => _spaceFireCard != null && _spaceFireCard.activeSelf;
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
        }

        public void ResetSessionState()
        {
            _seatedAtHelm = false;
            _selectedAdvanceSpot = null;
            _lastKeySpotsSignature = string.Empty;
            if (_orbitalOpsBar != null) _orbitalOpsBar.SetActive(false);
            if (_spaceFireCard != null) _spaceFireCard.SetActive(false);
            if (_surfaceHudRoot != null) _surfaceHudRoot.SetActive(false);
            if (_arenaLobbyModal != null) _arenaLobbyModal.SetActive(false);
            if (_arenaBattleHudRoot != null) _arenaBattleHudRoot.SetActive(false);
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

        public string ResolveSquadAdvanceCommand()
        {
            string spot = ResolveAdvanceSpot();
            return MushProtocol.TryValidateSafeCommandToken(spot, out string safeSpot, maxLen: 16)
                ? $"order all advance {safeSpot}"
                : null;
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
            _arenaLobbyModal = CreatePanel(parent, "OocArenaLobbyModal", new Vector2(0.5f, 0.5f), new Vector2(0.5f, 0.5f), new Vector2(-450f, -290f), new Vector2(450f, 290f), new Color(0.04f, 0.07f, 0.12f, 0.99f));

            var hdr = CreateTxt(_arenaLobbyModal.transform, "OOC SIMULATOR SELECTOR CONSOLE — RED vs BLUE", 20, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.58f, 0.48f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -46f), new Vector2(-130f, -8f));

            var closeBtn = CreateBtn(_arenaLobbyModal.transform, "CLOSE [ESC]", new Color(0.42f, 0.16f, 0.18f), () => _arenaLobbyModal.SetActive(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-120f, -44f), new Vector2(-16f, -10f));

            // Row 1: Claim Team Slot (`SIM TEAM red|blue`)
            var teamRow = CreateRow(_arenaLobbyModal.transform, "TeamRow", -102f, -56f);
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
            var unitRow = CreateRow(_arenaLobbyModal.transform, "UnitRow", -154f, -110f);
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

            // Row 3: AI Wingman Fill (`SIM AI <n>`)
            var aiRow = CreateRow(_arenaLobbyModal.transform, "AiFillRow", -206f, -162f);
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
            var modeRow = CreateRow(_arenaLobbyModal.transform, "ModeRow", -258f, -214f);
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
            SetRect(_arenaLobbySummaryText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -310f), new Vector2(-20f, -264f));

            _arenaLobbyLiveRosterText = CreateTxt(_arenaLobbyModal.transform, "Walk north into <b>Red Room</b> or south into <b>Blue Room</b> after <b>SIM START</b>, then press <b>[E]</b> on your pod (or click <b>BOARD POD</b>)!", 13, FontStyle.Normal, TextAnchor.MiddleCenter, new Color(0.68f, 0.90f, 0.84f));
            _arenaLobbyLiveRosterText.supportRichText = true;
            SetRect(_arenaLobbyLiveRosterText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(20f, -366f), new Vector2(-20f, -312f));

            // Row 5: Start Boarding & Enter Pod Bays
            var startRow = CreateRow(_arenaLobbyModal.transform, "SimStartAndBayRow", -424f, -374f);
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
            var mgmtRow = CreateRow(_arenaLobbyModal.transform, "SimMgmtRow", -478f, -432f);
            CreateBtn(mgmtRow, "SIM STATUS", new Color(0.16f, 0.28f, 0.40f), () => OnCommandRequested?.Invoke("sim status"));
            CreateBtn(mgmtRow, "MATCH HISTORY", new Color(0.24f, 0.36f, 0.48f), () => OnCommandRequested?.Invoke("sim history"));
            CreateBtn(mgmtRow, "SIM LEAVE", new Color(0.38f, 0.24f, 0.20f), () => OnCommandRequested?.Invoke("sim leave"));
            CreateBtn(mgmtRow, "SIM ABORT", new Color(0.46f, 0.18f, 0.20f), () => OnCommandRequested?.Invoke("sim abort"));
            CreateBtn(mgmtRow, "RETURN TO IC PORT (+ic)", new Color(0.16f, 0.40f, 0.36f), () =>
            {
                _arenaLobbyModal.SetActive(false);
                OnCommandRequested?.Invoke("+ic");
            });

            _arenaLobbyModal.SetActive(false);
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
            SetRect(_arenaBannerText.rectTransform, new Vector2(0.5f, 1f), new Vector2(0.5f, 1f), new Vector2(-420f, -166f), new Vector2(420f, -124f));

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

            var hullCard = CreatePanel(_arenaBattleHudRoot.transform, "OwnPlatformCard", new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-430f, 18f), new Vector2(-16f, 276f), new Color(0.03f, 0.07f, 0.12f, 0.93f));
            _arenaOwnArmorText = CreateTxt(hullCard.transform, "", 12, FontStyle.Normal, TextAnchor.UpperLeft, Color.white);
            _arenaOwnArmorText.supportRichText = true;
            SetRect(_arenaOwnArmorText.rectTransform, new Vector2(0f, 0.56f), new Vector2(1f, 1f), new Vector2(12f, 0f), new Vector2(-12f, -8f));

            _arenaWeaponText = CreateTxt(hullCard.transform, "", 11, FontStyle.Bold, TextAnchor.UpperLeft, new Color(0.65f, 0.95f, 0.80f));
            _arenaWeaponText.supportRichText = true;
            SetRect(_arenaWeaponText.rectTransform, new Vector2(0f, 0.23f), new Vector2(1f, 0.56f), new Vector2(12f, 0f), new Vector2(-12f, 0f));

            var fireBtn = CreateBtn(hullCard.transform, "FIRE MAIN WEAPON [SPACE]", new Color(0.72f, 0.18f, 0.16f), () => OnArenaCockpitVerbRequested?.Invoke("fire"));
            SetRect(fireBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0.64f, 0.22f), new Vector2(12f, 8f), new Vector2(-4f, -2f));
            _fireBtnLabel = fireBtn.GetComponentInChildren<Text>();

            var volleyBtn = CreateBtn(hullCard.transform, "VOLLEY ALL", new Color(0.54f, 0.26f, 0.14f), () => OnArenaCockpitVerbRequested?.Invoke("fire all"));
            SetRect(volleyBtn.GetComponent<RectTransform>(), new Vector2(0.64f, 0f), new Vector2(1f, 0.22f), new Vector2(4f, 8f), new Vector2(-12f, -2f));

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

            CreateBtn(squadRow.transform, "SQUAD: ENGAGE", new Color(0.52f, 0.24f, 0.16f), () => OnArenaCockpitVerbRequested?.Invoke("order all engage"));
            _advanceBtn = CreateBtn(squadRow.transform, "SQUAD: ADVANCE", new Color(0.46f, 0.34f, 0.14f), () =>
            {
                string advCmd = ResolveSquadAdvanceCommand();
                if (!string.IsNullOrEmpty(advCmd))
                    OnArenaCockpitVerbRequested?.Invoke(advCmd);
            });
            _advanceBtnLabel = _advanceBtn.GetComponentInChildren<Text>();
            CreateBtn(squadRow.transform, "SQUAD: HOLD", new Color(0.20f, 0.36f, 0.46f), () => OnArenaCockpitVerbRequested?.Invoke("order all hold"));
            CreateBtn(squadRow.transform, "CUPOLA [V]", new Color(0.28f, 0.28f, 0.46f), () => OnToggleVehicleCameraViewRequested?.Invoke());
            CreateBtn(squadRow.transform, "EXIT POD", new Color(0.48f, 0.16f, 0.18f), () => OnArenaCockpitVerbRequested?.Invoke("exit"));

            _arenaBattleHudRoot.SetActive(false);
        }

        public void RefreshArenaHud()
        {
            if (_arenaBattleHudRoot == null) return;
            bool active = _arena != null && _arena.IsArenaActive;
            _arenaBattleHudRoot.SetActive(active);

            if (!active) return;

            var s = _arena.State;
            string holdStr = s.Mode == "hold"
                ? $"  |  HOLD ZONE: {(s.HoldControlScore >= 0 ? $"<color=#FF6655>RED +{s.HoldControlScore}%</color>" : $"<color=#55AAFF>BLUE {Mathf.Abs(s.HoldControlScore)}%</color>")}"
                : $"  |  MATCH #{s.MatchId}";

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
            UpdatePooledContactRows(s);
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
