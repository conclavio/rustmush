using System;
using System.Collections.Generic;
using TacSim.Arena;
using TacSim.Core;
using TacSim.Net;
using TacSim.World;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public readonly struct PaletteEntry
    {
        public readonly string Category;
        public readonly string Title;
        public readonly string Subtitle;
        public readonly string SearchBlob;
        public readonly Action Execute;

        public PaletteEntry(string category, string title, string subtitle, Action execute, string extraKeywords = null)
        {
            Category = category ?? string.Empty;
            Title = title ?? string.Empty;
            Subtitle = subtitle ?? string.Empty;
            SearchBlob = string.IsNullOrEmpty(extraKeywords)
                ? $"{Category} {Title} {Subtitle}".ToLowerInvariant()
                : $"{Category} {Title} {Subtitle} {extraKeywords}".ToLowerInvariant();
            Execute = execute;
        }
    }

    /// <summary>
    /// Spotlight / VSCode / EVE-style `Ctrl+K` Fuzzy Command &amp; Fast-Travel Palette.
    /// Searches live 3D Station rooms &amp; Ship compartments (lighting up 3D Wayfinder trails),
    /// Solar System ports, live role-authorized `commands.list` server verbs, and MMORPG UI modals with full keyboard control.
    /// </summary>
    public sealed class TacSimCommandPalette : MonoBehaviour
    {
        public event Action<string> OnCommandRequested;
        public event Action<string> OnPrefillCommandRequested;
        public event Action<string> OnPlotStationRoomRequested;
        public event Action<string> OnPlotShipCompartmentRequested;
        public event Action<string> OnInspectSolarPortRequested;
        public event Action<string> OnUiActionRequested;
        public event Action<string> OnArenaActionRequested;
        public event Action<int> OnReplayFollowUnitRequested;

        private const int MaxVisibleRows = 10;

        private readonly struct ServerCommandInfo
        {
            public readonly string Name;
            public readonly string Usage;
            public readonly string Summary;
            public readonly string Module;
            public readonly string ExtraKeywords;

            public ServerCommandInfo(string name, string usage, string summary, string module, string extraKeywords)
            {
                Name = name;
                Usage = usage;
                Summary = summary;
                Module = module;
                ExtraKeywords = extraKeywords;
            }
        }

        private sealed class PooledRow
        {
            public GameObject Root;
            public Image Background;
            public Button Button;
            public Text Label;
            public int MatchedIndex;
        }

        private Font _font;
        private GameObject _modalRoot;
        private InputField _searchInput;
        private Text _footerText;
        private readonly List<PooledRow> _rowPool = new List<PooledRow>(MaxVisibleRows);

        private readonly List<PaletteEntry> _allEntries = new List<PaletteEntry>(256);
        private readonly List<ServerCommandInfo> _serverCommands = new List<ServerCommandInfo>(160);
        private readonly List<int> _filteredIndices = new List<int>(256);
        private int _selectedCursor;

        private StationWorldController _station;
        private ShipInteriorController _ship;
        private OocArenaController _arena;
        private TacSimQuestJournal _quests;
        private TacSimTradeAndShipState _trade;
        private TacSimActiveStage _activeStage = TacSimActiveStage.StationWalk;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public bool IsSearchFocused => IsOpen && _searchInput != null && _searchInput.isFocused;
        public int FilteredMatchCount => _filteredIndices.Count;
        public int ServerCommandCount => _serverCommands.Count;
        public int SelectedIndex => _selectedCursor;
        public string SelectedEntryTitle =>
            (_selectedCursor >= 0 && _selectedCursor < _filteredIndices.Count)
                ? _allEntries[_filteredIndices[_selectedCursor]].Title
                : string.Empty;

        public List<string> GetFilteredTitlesSnapshot()
        {
            var list = new List<string>(_filteredIndices.Count);
            for (int i = 0; i < _filteredIndices.Count; i++)
                list.Add(_allEntries[_filteredIndices[i]].Title);
            return list;
        }

        public List<string> GetFilteredSubtitlesSnapshot()
        {
            var list = new List<string>(_filteredIndices.Count);
            for (int i = 0; i < _filteredIndices.Count; i++)
                list.Add(_allEntries[_filteredIndices[i]].Subtitle);
            return list;
        }

        private string FormatQuestReadinessTag(MushActiveQuest q)
        {
            if (q == null || _quests == null || q.CanFinishNow) return string.Empty;
            string holdSummary = string.Empty;
            bool hasHold = _quests.TryGetQuestHoldReadinessSummary(q, out holdSummary, out _);
            if (hasHold)
            {
                if (holdSummary.StartsWith("[RECOVER RIG FIRST]", StringComparison.Ordinal))
                    return "[RECOVER RIG FIRST]";
                if (holdSummary.StartsWith("[ORE HOLD READY]", StringComparison.Ordinal))
                    return "[ORE HOLD READY]";
            }
            if (_quests.TryGetQuestPreflightReadinessSummary(q, out _, out bool pfReady))
                return pfReady ? "[PRE-FLIGHT OK]" : "[CHECK BEFORE UNDOCK]";
            if (hasHold && !string.IsNullOrEmpty(holdSummary))
            {
                if (holdSummary.StartsWith("[HOLD READY]", StringComparison.Ordinal)) return "[HOLD READY]";
                if (holdSummary.StartsWith("[HOLD]", StringComparison.Ordinal)) return "[HOLD]";
                if (holdSummary.StartsWith("[READY]", StringComparison.Ordinal)) return "[READY]";
                if (holdSummary.StartsWith("[REFIT CHECK]", StringComparison.Ordinal)) return "[REFIT CHECK]";
                if (holdSummary.StartsWith("[ORE HOLD]", StringComparison.Ordinal)) return "[ORE HOLD]";
            }
            return string.Empty;
        }

        private static string FormatQuestStepSuffix(string key)
        {
            int step = TacSimQuestJournal.GuidedStepNumber(key);
            return step > 0 ? $" [Step {step}/7]" : string.Empty;
        }

        private static bool IsGuidedQuest(string key, string category)
        {
            return string.Equals(category, "guided", StringComparison.OrdinalIgnoreCase) ||
                TacSimQuestJournal.GuidedStepNumber(key) > 0;
        }

        private static string FormatQuestRouteTitle(string rCmd, string rRoom, int rHops)
        {
            if (string.Equals(rCmd, "+ship/board", StringComparison.OrdinalIgnoreCase))
                return "Board Ship";
            if (string.Equals(rCmd, "+ship/leave", StringComparison.OrdinalIgnoreCase))
                return "Cycle Ashore";
            if (string.Equals(rCmd, "+tv/bay in", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(rCmd, "+tv/bay", StringComparison.OrdinalIgnoreCase))
                return "Enter Vehicle Bay";
            return $"Walk -> {rRoom} ({rHops}h)";
        }

        /// <summary>
        /// Returns true when a server command's usage line shows required or optional arguments
        /// (e.g. `&lt;target&gt;`, `[args]`, `=`), indicating the palette should pre-fill chat input instead of firing bare.
        /// </summary>
        public static bool CommandTakesArguments(string usage, string primaryVerb)
        {
            if (string.IsNullOrWhiteSpace(usage)) return false;
            string trimmed = usage.Trim();
            if (trimmed.IndexOf('<') >= 0 || trimmed.IndexOf('[') >= 0 || trimmed.IndexOf('=') >= 0)
                return true;
            if (!string.IsNullOrWhiteSpace(primaryVerb) && trimmed.Length > primaryVerb.Trim().Length + 1)
            {
                int sp = trimmed.IndexOf(' ');
                if (sp > 0 && sp < trimmed.Length - 1)
                    return true;
            }
            return false;
        }

        private static readonly (string slug, string label)[] CorridorPorts =
        {
            ("ceres", "Ceres Freeport"),
            ("stickney", "Stickney Yards (Phobos)"),
            ("luna", "Aitken Highport (Luna)"),
            ("deimos", "Voltaire Anchorage (Deimos)"),
            ("titan", "Ligeia Terminal (Titan)"),
            ("commons", "Freeport Commons (Ceres Surface)"),
            ("rheasilvia", "Rheasilvia Deep (Vesta)"),
            ("occator", "Occator Rim (Ceres Surface)"),
            ("kerwan", "Kerwan Basin"),
            ("ahuna", "Ahuna Mons"),
            ("juling", "Juling Crater"),
            ("urvara", "Urvara Basin")
        };

        /// <summary>
        /// Case-insensitive multi-token fuzzy score: returns &gt;= 0 when every whitespace-separated
        /// query token matches as a substring or ordered subsequence (higher = stronger match), or -1 if unmatched.
        /// </summary>
        public static int ScoreFuzzyMatch(string haystackLower, string rawQuery)
        {
            if (string.IsNullOrWhiteSpace(rawQuery)) return 100;
            if (string.IsNullOrEmpty(haystackLower)) return -1;

            string[] tokens = rawQuery.Trim().ToLowerInvariant().Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
            int totalScore = 0;
            for (int t = 0; t < tokens.Length; t++)
            {
                string tok = tokens[t];
                int idx = haystackLower.IndexOf(tok, StringComparison.Ordinal);
                if (idx >= 0)
                {
                    totalScore += idx == 0 ? 120 : (80 - Mathf.Min(60, idx));
                    continue;
                }

                // Ordered subsequence fallback (e.g. "shor" -> "ship orders")
                int hi = 0;
                int matched = 0;
                for (int ci = 0; ci < tok.Length; ci++)
                {
                    char c = tok[ci];
                    while (hi < haystackLower.Length && haystackLower[hi] != c)
                        hi++;
                    if (hi >= haystackLower.Length)
                        return -1;
                    matched++;
                    hi++;
                }
                if (matched == tok.Length)
                    totalScore += 25;
                else
                    return -1;
            }
            return totalScore;
        }

        public void BuildModal(Transform parent, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _rowPool.Clear();

            _modalRoot = CreatePanel(
                parent,
                "CommandPaletteModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-360f, -245f),
                new Vector2(360f, 245f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            CreatePanel(_modalRoot.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.32f, 0.90f, 1.0f, 0.95f));

            var hdr = CreateText(_modalRoot.transform, "QUICK COMMAND & FAST-TRAVEL PALETTE [CTRL+K]", 14, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.92f, 1f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -34f), new Vector2(-106f, -6f));

            var closeBtn = CreateButton(_modalRoot.transform, "ESC", new Color(0.42f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-96f, -32f), new Vector2(-12f, -8f));

            _searchInput = CreateInputField(
                _modalRoot.transform,
                "Type room, port, command, or panel (e.g. exchange, stickney, +quest, options)...",
                new Vector2(0f, 1f),
                new Vector2(1f, 1f),
                new Vector2(14f, -74f),
                new Vector2(-14f, -38f));
            _searchInput.onValueChanged.AddListener(SetQuery);
            _searchInput.onSubmit.AddListener(_ => ExecuteSelected());

            var listArea = CreatePanel(_modalRoot.transform, "PaletteRows", new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(14f, 34f), new Vector2(-14f, -82f), new Color(0.02f, 0.05f, 0.09f, 0.88f));
            var vlg = listArea.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxVisibleRows; i++)
            {
                int slotIdx = i;
                var go = CreatePanel(listArea.transform, $"PaletteRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 32f), new Color(0.08f, 0.13f, 0.20f, 0.92f));
                go.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 32f);
                var img = go.GetComponent<Image>();
                img.raycastTarget = true;
                var btn = go.AddComponent<Button>();
                btn.targetGraphic = img;

                var txt = CreateText(go.transform, string.Empty, 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.90f, 0.96f, 1f));
                txt.supportRichText = true;
                SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 0f), new Vector2(-10f, 0f));

                var row = new PooledRow
                {
                    Root = go,
                    Background = img,
                    Button = btn,
                    Label = txt,
                    MatchedIndex = -1
                };
                btn.onClick.AddListener(() =>
                {
                    _selectedCursor = slotIdx;
                    ExecuteSelected();
                });
                go.SetActive(false);
                _rowPool.Add(row);
            }

            _footerText = CreateText(_modalRoot.transform, "[Up/Down] Select   [Enter] Run   [Esc] Close", 11, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.64f, 0.78f, 0.88f));
            SetRect(_footerText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(16f, 8f), new Vector2(-16f, 28f));

            _modalRoot.SetActive(false);
        }

        public void BindWorldSources(StationWorldController station, ShipInteriorController ship, OocArenaController arena = null)
        {
            _station = station;
            _ship = ship;
            _arena = arena;
        }

        public void BindQuestJournal(TacSimQuestJournal quests)
        {
            _quests = quests;
        }

        public void BindTradeState(TacSimTradeAndShipState trade)
        {
            _trade = trade;
        }

        public void SetActiveStage(TacSimActiveStage stage)
        {
            _activeStage = stage;
        }

        public void SetOpen(bool open)
        {
            if (_modalRoot == null) return;
            _modalRoot.SetActive(open);
            if (open)
            {
                RebuildCatalog();
                if (_searchInput != null)
                {
                    _searchInput.SetTextWithoutNotify(string.Empty);
                    _searchInput.Select();
                    _searchInput.ActivateInputField();
                }
                SetQuery(string.Empty);
            }
            else if (_searchInput != null)
            {
                _searchInput.DeactivateInputField();
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void ResetSessionState()
        {
            _serverCommands.Clear();
            SetOpen(false);
        }

        /// <summary>
        /// Hydrates the palette with the live role-authorized command table from `mushd` (`commands.list`).
        /// </summary>
        public void ApplyServerCommandsList(JsonNode data)
        {
            if (data == null || !data.IsObject) return;
            var arr = data["commands"].AsArray();
            if (arr == null) return;

            _serverCommands.Clear();
            int cap = Mathf.Min(arr.Count, 512);
            for (int i = 0; i < cap; i++)
            {
                var c = arr[i];
                if (!c.IsObject) continue;
                string name = MushProtocol.StripControlChars(c.Str("name", string.Empty)).Trim();
                if (string.IsNullOrEmpty(name)) continue;

                string usage = MushProtocol.StripControlChars(c.Str("usage", string.Empty)).Trim();
                string summary = MushProtocol.StripControlChars(c.Str("summary", string.Empty)).Trim();
                string module = MushProtocol.StripControlChars(c.Str("module", string.Empty)).Trim();

                var kwParts = new List<string>(8);
                if (!string.IsNullOrEmpty(module)) kwParts.Add(module);
                if (!string.IsNullOrEmpty(usage)) kwParts.Add(usage);

                var aliases = c["aliases"].AsArray();
                if (aliases != null)
                {
                    for (int j = 0; j < aliases.Count && j < 8; j++)
                    {
                        string al = MushProtocol.StripControlChars(aliases[j].AsString()).Trim();
                        if (!string.IsNullOrEmpty(al)) kwParts.Add(al);
                    }
                }

                var switches = c["switches"].AsArray();
                if (switches != null)
                {
                    for (int j = 0; j < switches.Count && j < 12; j++)
                    {
                        string sw = MushProtocol.StripControlChars(switches[j].AsString()).Trim();
                        if (!string.IsNullOrEmpty(sw)) kwParts.Add($"{name}/{sw}");
                    }
                }

                _serverCommands.Add(new ServerCommandInfo(
                    name,
                    usage,
                    summary,
                    module,
                    kwParts.Count > 0 ? string.Join(" ", kwParts) : string.Empty));
            }

            if (IsOpen)
            {
                RebuildCatalog();
                SetQuery(_searchInput != null ? _searchInput.text : string.Empty);
            }
        }

        public void RebuildCatalog()
        {
            _allEntries.Clear();
            var curatedVerbs = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            // 0. Context-First Live OOC Simulator Arena & 3D Replay Theater Actions
            if (_activeStage == TacSimActiveStage.SimulatorArena && _arena != null)
            {
                if (_arena.IsArenaActive && _arena.State != null)
                {
                    var st = _arena.State;
                    for (int i = 0; i < st.Contacts.Count; i++)
                    {
                        var ct = st.Contacts[i];
                        if (ct.IsWreck || ct.Friendly) continue;
                        if (!MushProtocol.TryValidateSafeCommandToken(ct.Designator, out string safeDesig)) continue;
                        string cls = string.IsNullOrEmpty(ct.ClassLabel) ? "unit" : ct.ClassLabel;
                        _allEntries.Add(new PaletteEntry(
                            "ARENA LOCK",
                            $"Lock {safeDesig} [{cls} · {(int)ct.RangeM}m · {ct.HitOddsPct}%]",
                            $"lock {safeDesig}",
                            () => OnArenaActionRequested?.Invoke($"lock {safeDesig}"),
                            $"contact target enemy {safeDesig} {cls}"));
                        _allEntries.Add(new PaletteEntry(
                            "ARENA SCAN",
                            $"Scan {safeDesig} [{cls}]",
                            $"scan {safeDesig}",
                            () => OnArenaActionRequested?.Invoke($"scan {safeDesig}"),
                            $"inspect hull damage {safeDesig} {cls}"));
                    }

                    if (_arena.TerrainGrid?.KeySpots != null)
                    {
                        for (int i = 0; i < _arena.TerrainGrid.KeySpots.Count; i++)
                        {
                            var sp = _arena.TerrainGrid.KeySpots[i];
                            if (!MushProtocol.TryValidateSafeCommandToken(sp.Letter, out string safeLtr, maxLen: 16)) continue;
                            string sName = string.IsNullOrEmpty(sp.Name) ? safeLtr : sp.Name;
                            _allEntries.Add(new PaletteEntry(
                                "ARENA DRIVE",
                                $"Drive to [{safeLtr}] {sName}",
                                $"drive {safeLtr}",
                                () => OnArenaActionRequested?.Invoke($"drive {safeLtr}"),
                                $"waypoint autopilot move spot {safeLtr} {sName}"));
                            _allEntries.Add(new PaletteEntry(
                                "ARENA SQUAD",
                                $"Squad Advance to [{safeLtr}] {sName}",
                                $"order all advance {safeLtr}",
                                () => OnArenaActionRequested?.Invoke($"order all advance {safeLtr}"),
                                $"squad order advance keyspot {safeLtr} {sName}"));
                        }
                    }

                    if (st.Mind != null && st.Mind.Profiles != null)
                    {
                        for (int i = 0; i < st.Mind.Profiles.Count; i++)
                        {
                            if (i == st.Mind.ActiveIndex) continue;
                            string rawProf = st.Mind.Profiles[i];
                            if (!MushProtocol.TryValidateSafeCommandToken(rawProf, out string safeProf, maxLen: 32)) continue;
                            _allEntries.Add(new PaletteEntry(
                                "ARENA MIND",
                                $"Switch Mind Profile: {safeProf}",
                                $"MIND PROFILE {safeProf}",
                                () => OnArenaActionRequested?.Invoke($"MIND PROFILE {safeProf}"),
                                $"mind profile preset doctrine {safeProf}"));
                        }
                    }

                    string[] postures = { "return", "guard", "hold" };
                    for (int p = 0; p < postures.Length; p++)
                    {
                        string pw = postures[p];
                        if (string.Equals(st.Posture, pw, StringComparison.OrdinalIgnoreCase)) continue;
                        string pwUp = pw.ToUpperInvariant();
                        _allEntries.Add(new PaletteEntry(
                            "ARENA POSTURE",
                            $"Set Hull Posture: {pwUp}",
                            $"MIND POSTURE {pw}",
                            () => OnArenaActionRequested?.Invoke($"MIND POSTURE {pw}"),
                            $"posture mind roe {pw}"));
                    }

                    _allEntries.Add(new PaletteEntry("ARENA SQUAD", "Squad Engage All", "order all engage", () => OnArenaActionRequested?.Invoke("order all engage"), "squad attack engage all"));
                    _allEntries.Add(new PaletteEntry("ARENA SQUAD", "Squad Hold Position", "order all hold", () => OnArenaActionRequested?.Invoke("order all hold"), "squad defend hold position"));
                    _allEntries.Add(new PaletteEntry("ARENA", "Cycle Gunner Optics (1x / 2.5x / 4x) [Q]", "Zoom gunner sights", () => OnUiActionRequested?.Invoke("ui:cycle_optics"), "optics zoom sniper reticle sight"));
                    _allEntries.Add(new PaletteEntry("ARENA", "Toggle Auto-Lock Nearest [L]", _arena.HasAutoLockRule ? "Disable standing auto-lock rule" : "LOCK nearest WHEN nolock", () => _arena.ToggleAutoLockRule(), "auto lock rule nearest"));
                    _allEntries.Add(new PaletteEntry("ARENA", "Fire Main Weapon [Space]", "fire", () => OnArenaActionRequested?.Invoke("fire"), "shoot gun cannon"));
                    _allEntries.Add(new PaletteEntry("ARENA", "Volley All Weapons [G]", "fire all", () => OnArenaActionRequested?.Invoke("fire all"), "salvo alpha strike volley"));
                    string fireGateVerb = st.WeaponsFree ? "fire hold" : "fire free";
                    _allEntries.Add(new PaletteEntry("ARENA", st.WeaponsFree ? "Set Weapons Hold [F]" : "Set Weapons Free [F]", fireGateVerb, () => OnArenaActionRequested?.Invoke(fireGateVerb), "hold free weapons gate"));
                    _allEntries.Add(new PaletteEntry("ARENA", "Toggle EMCON Jammer [M]", "emcon", () => OnArenaActionRequested?.Invoke("emcon"), "ecm radar silent"));
                    _allEntries.Add(new PaletteEntry("ARENA", "Active Sensor Sweep [N]", "scan", () => OnArenaActionRequested?.Invoke("scan"), "radar ping scan"));
                    _allEntries.Add(new PaletteEntry("ARENA", "All Stop [X / 0]", "speed 0", () => OnArenaActionRequested?.Invoke("speed 0"), "halt brake stop"));
                    _allEntries.Add(new PaletteEntry("ARENA", "Exit Simulator Pod", "exit", () => OnArenaActionRequested?.Invoke("exit"), "leave disembark pod"));
                }
                else if (_arena.IsSpectatorTheaterActive && _arena.ActiveReplayDoc != null)
                {
                    var doc = _arena.ActiveReplayDoc;
                    for (int i = 0; i < doc.Roster.Count; i++)
                    {
                        int capturedIdx = i;
                        var r = doc.Roster[i];
                        string teamTag = r.IsBlue ? "BLUE" : "RED";
                        _allEntries.Add(new PaletteEntry(
                            "REPLAY CAM",
                            $"Follow Unit {r.Callsign} [{teamTag}]",
                            "Bind 3D spectator camera to unit",
                            () => OnReplayFollowUnitRequested?.Invoke(capturedIdx),
                            $"camera spectate watch {r.Callsign} {teamTag}"));
                    }
                    if (_arena.IsReplayModeActive)
                    {
                        _allEntries.Add(new PaletteEntry("REPLAY", "Restart Replay from 0:00 [R]", "Rewind tape to start and play", () => _arena.RestartReplayFromStart(), "restart rewind beginning 0:00"));
                        _allEntries.Add(new PaletteEntry("REPLAY", "Jump to Next Combat Highlight []]", "Next hit or wreck timestamp", () => _arena.StepReplayToEvent(+1), "event kill shot next"));
                        _allEntries.Add(new PaletteEntry("REPLAY", "Jump to Previous Combat Highlight [[]", "Previous hit or wreck timestamp", () => _arena.StepReplayToEvent(-1), "event kill shot previous back"));
                        _allEntries.Add(new PaletteEntry("REPLAY", "Toggle Replay Play / Pause [Space]", "Pause or resume match tape", () => _arena.ToggleReplayPlaying(), "play pause"));
                    }
                    _allEntries.Add(new PaletteEntry("REPLAY", "Cycle Spectator Optics Zoom [Q]", "Zoom 1x / 2.5x / 4x camera", () => OnUiActionRequested?.Invoke("ui:cycle_optics"), "optics zoom camera"));
                }
            }

            // 0b. Live Active & Available Quest Quick Actions
            if (_quests != null && _quests.LatestPanel != null)
            {
                var qp = _quests.LatestPanel;
                var tracked = _quests.GetTrackedActiveQuest();

                // Order active quests by urgency (can finish -> ready -> can verify -> in-progress; tracked first among equals)
                var orderedActive = new List<MushActiveQuest>(qp.Active);
                orderedActive.Sort((a, b) =>
                {
                    int ua = a.CanFinishNow ? 0 : (a.ReadyToFinish ? 1 : (a.CanCheckNow ? 2 : 3));
                    int ub = b.CanFinishNow ? 0 : (b.ReadyToFinish ? 1 : (b.CanCheckNow ? 2 : 3));
                    if (ua != ub) return ua.CompareTo(ub);
                    int ta = (tracked != null && a.Key == tracked.Key) ? 0 : 1;
                    int tb = (tracked != null && b.Key == tracked.Key) ? 0 : 1;
                    return ta.CompareTo(tb);
                });

                // 1. Surface immediate 1-click Finish / Verify actions across ALL active quests
                for (int i = 0; i < orderedActive.Count; i++)
                {
                    var q = orderedActive[i];
                    string stepSfx = FormatQuestStepSuffix(q.Key);
                    if (q.CanFinishNow && !string.IsNullOrEmpty(q.FinishCmd))
                    {
                        string finishCmd = q.FinishCmd;
                        _allEntries.Add(new PaletteEntry(
                            "QUEST STEP",
                            $"Finish Now: {q.Title}{stepSfx}",
                            finishCmd,
                            () => OnCommandRequested?.Invoke(finishCmd),
                            $"quest finish complete turnin now {q.Title} {finishCmd}"));
                    }
                    else if (q.CanCheckNow && !string.IsNullOrEmpty(q.CheckCmd))
                    {
                        string checkCmd = q.CheckCmd;
                        _allEntries.Add(new PaletteEntry(
                            "QUEST STEP",
                            $"Verify Now: {q.Title}{stepSfx}",
                            checkCmd,
                            () => OnCommandRequested?.Invoke(checkCmd),
                            $"quest verify check now {q.Title} {checkCmd}"));
                    }
                }

                // 2. Active quest dialogs & locally resolved 3D Wayfinder trails
                for (int i = 0; i < orderedActive.Count; i++)
                {
                    var q = orderedActive[i];
                    string qKey = q.Key;
                    string stepSfx = FormatQuestStepSuffix(qKey);
                    bool ready = q.CanFinishNow || q.ReadyToFinish;
                    string dlgTitle = ready ? $"Complete Quest [?]: {q.Title}{stepSfx}" : $"Quest Dialog: {q.Title}{stepSfx}";
                    string rTag = FormatQuestReadinessTag(q);
                    string baseSub = !string.IsNullOrEmpty(q.NextStep) ? q.NextStep : $"Turn in to {q.TurnInName}";
                    string dlgSub = !string.IsNullOrEmpty(rTag) ? $"{rTag} {baseSub}" : baseSub;
                    _allEntries.Add(new PaletteEntry(
                        "QUEST",
                        dlgTitle,
                        dlgSub,
                        () => _quests?.OpenQuestDialogByKey(qKey),
                        $"quest active objective dialog {q.Title} {q.TurnInName} {q.NextRoomName}"));

                    bool isTracked = tracked != null && q.Key == tracked.Key;
                    bool needsOrigin = isTracked && _quests.NeedsOriginPortProcurement(_trade);
                    bool needsExp = isTracked && _quests.NeedsSurfaceExpeditionBoarding();
                    string rawRoom = _quests != null ? _quests.ResolveObjectiveRoomEntry(q) : q.NextRoomEntry;
                    string targetRoom = rawRoom;
                    bool redirectedFromServerRoom = !string.Equals(rawRoom, q.NextRoomEntry, StringComparison.OrdinalIgnoreCase);
                    string destLabel = (!redirectedFromServerRoom && !string.IsNullOrEmpty(q.NextRoomName)) ? q.NextRoomName : rawRoom;
                    bool isBoarding = false;
                    if (_station?.Pack != null &&
                        (!string.IsNullOrEmpty(rawRoom) || needsExp) &&
                        _station.TryResolveLocalQuestRoom(rawRoom, needsOrigin, needsExp, out string localRoom, out isBoarding))
                    {
                        bool redirected = redirectedFromServerRoom || !string.Equals(localRoom, rawRoom, StringComparison.OrdinalIgnoreCase);
                        targetRoom = localRoom;
                        if ((redirected || string.IsNullOrEmpty(q.NextRoomName)) &&
                            _station.Pack.Rooms.TryGetValue(localRoom, out var roomDef) &&
                            !string.IsNullOrEmpty(roomDef.Name))
                        {
                            destLabel = roomDef.Name;
                        }
                    }

                    if (!string.IsNullOrEmpty(targetRoom))
                    {
                        string trailTitle = isBoarding
                            ? $"Wayfinder to Board Ship: {destLabel}"
                            : $"Wayfinder to Quest Objective: {destLabel}";
                        _allEntries.Add(new PaletteEntry(
                            "QUEST TRAIL",
                            trailTitle,
                            !string.IsNullOrEmpty(q.NextStep) ? $"{q.Title} — {q.NextStep}" : q.Title,
                            () => OnPlotStationRoomRequested?.Invoke(targetRoom),
                            $"quest trail wayfinder route navigate objective board {q.Title} {destLabel} {q.NextStep}"));
                    }
                }

                // 3. Available quest dialogs & 1-click Accept Now shortcuts (guided offers first)
                var orderedAvail = new List<MushAvailableQuest>(qp.Available);
                orderedAvail.Sort((a, b) =>
                {
                    int ga = IsGuidedQuest(a.Key, a.Category) ? 0 : 1;
                    int gb = IsGuidedQuest(b.Key, b.Category) ? 0 : 1;
                    return ga.CompareTo(gb);
                });
                for (int i = 0; i < orderedAvail.Count; i++)
                {
                    var av = orderedAvail[i];
                    string giverId = av.GiverId;
                    string qKey = av.Key;
                    string stepSfx = FormatQuestStepSuffix(qKey);
                    _allEntries.Add(new PaletteEntry(
                        "QUEST [!]",
                        $"Accept Quest [!]: {av.Title}{stepSfx}",
                        $"Offered by {av.GiverName}",
                        () =>
                        {
                            if (_quests != null && !_quests.OpenNpcQuestDialog(giverId))
                                _quests.OpenQuestDialogByKey(qKey);
                        },
                        $"quest available accept offer npc {av.Title} {av.GiverName}"));

                    if (!string.IsNullOrEmpty(av.AcceptCmd))
                    {
                        string acceptCmd = av.AcceptCmd;
                        _allEntries.Add(new PaletteEntry(
                            "QUEST STEP",
                            $"Accept Now: {av.Title}{stepSfx}",
                            $"{acceptCmd} — {av.GiverName}",
                            () => OnCommandRequested?.Invoke(acceptCmd),
                            $"quest accept now offer {av.Title} {av.GiverName} {acceptCmd}"));
                    }
                }

                if (tracked != null)
                {
                    string pushedSuggestedCmd = string.Empty;
                    if (_quests.TrackerRouteHops > 0 && !string.IsNullOrEmpty(_quests.TrackerRouteCommand))
                    {
                        string rCmd = _quests.TrackerRouteCommand;
                        string rRoom = _quests.TrackerRouteRoomName;
                        int rHops = _quests.TrackerRouteHops;
                        string rTitle = FormatQuestRouteTitle(rCmd, rRoom, rHops);
                        string hopWord = rHops == 1 ? "hop" : "hops";
                        _allEntries.Add(new PaletteEntry(
                            "QUEST ROUTE",
                            rTitle,
                            $"{rHops} {hopWord} · {rCmd}",
                            () => OnCommandRequested?.Invoke(rCmd),
                            $"quest route walk board ashore vehicle bay {rRoom} {rCmd} {tracked.Title}"));
                    }
                    else if (_quests.TryGetStowageAwareSuggestedCommand(tracked, out string sugLbl, out string sugCmd))
                    {
                        string cmdCopy = sugCmd;
                        pushedSuggestedCmd = (cmdCopy ?? string.Empty).Trim();
                        string trackedTag = FormatQuestReadinessTag(tracked);
                        string tagPfx = !string.IsNullOrEmpty(trackedTag) ? $"{trackedTag} " : string.Empty;
                        string sub = !string.IsNullOrEmpty(tracked.NextStep)
                            ? $"{tagPfx}{cmdCopy} — {tracked.NextStep}"
                            : $"{tagPfx}{cmdCopy}";
                        _allEntries.Add(new PaletteEntry(
                            "QUEST STEP",
                            $"Next: {sugLbl}",
                            sub,
                            () => OnPrefillCommandRequested?.Invoke(cmdCopy),
                            $"quest next step action {sugLbl} {cmdCopy} {tracked.Title}"));
                    }

                    if (TacSimQuestJournal.TryGetSurfaceMiningStatusForQuest(tracked, out var mining))
                    {
                        if (!mining.MineDone)
                        {
                            if (!string.Equals(pushedSuggestedCmd, "+chart/ground", StringComparison.OrdinalIgnoreCase))
                            {
                                string oreLabel = (mining.TargetOreSlug ?? "carbon_ore").Replace('_', ' ');
                                _allEntries.Add(new PaletteEntry(
                                    "QUEST GROUND",
                                    $"Ground Chart: {oreLabel} ({mining.RecommendedFamily})",
                                    "+chart/ground — Pick a Ceres landing site (Freeport Commons / Ahuna Flank)",
                                    () => OnPrefillCommandRequested?.Invoke("+chart/ground"),
                                    $"quest ground chart mining {oreLabel} {mining.RecommendedFamily} +chart/ground"));
                            }
                        }
                        else if (!tracked.CanFinishNow && !tracked.TurnInHere)
                        {
                            string miningHold = string.Empty;
                            bool hasMiningHold = _quests.TryGetQuestHoldReadinessSummary(tracked, out miningHold, out _);
                            if (hasMiningHold && miningHold.StartsWith("[RECOVER RIG FIRST]", StringComparison.Ordinal))
                            {
                                if (!string.Equals(pushedSuggestedCmd, "+tv/1 recover go", StringComparison.OrdinalIgnoreCase))
                                {
                                    string oreLabel = (mining.TargetOreSlug ?? "carbon_ore").Replace('_', ' ');
                                    _allEntries.Add(new PaletteEntry(
                                        "QUEST STEP",
                                        "Recover Rig First [RECOVER RIG FIRST]",
                                        $"+tv/1 recover go — Stow mined {oreLabel} in ship hold before returning to port",
                                        () => OnPrefillCommandRequested?.Invoke("+tv/1 recover go"),
                                        $"quest recover rig first {oreLabel} +tv/1 recover go mined ore"));
                                }
                            }
                            else
                            {
                                string retPortId = string.Empty;
                                string retLabel = string.Empty;
                                if (_quests.TryGetActiveDestinationPort(out var retDest))
                                {
                                    retPortId = retDest.PortId;
                                    retLabel = retDest.PortLabel;
                                }
                                else
                                {
                                    string roomEntry = tracked.NextRoomEntry;
                                    if (string.IsNullOrEmpty(roomEntry))
                                    {
                                        for (int cIdx = tracked.Checkpoints.Count - 1; cIdx >= 0; cIdx--)
                                        {
                                            if (!string.IsNullOrEmpty(tracked.Checkpoints[cIdx].RoomEntry))
                                            {
                                                roomEntry = tracked.Checkpoints[cIdx].RoomEntry;
                                                break;
                                            }
                                        }
                                    }
                                    string slug = TacSimQuestJournal.InferStationPortFromRoomEntry(roomEntry);
                                    if (!string.IsNullOrEmpty(slug))
                                    {
                                        retPortId = slug;
                                        retLabel = string.Equals(slug, "ceres", StringComparison.OrdinalIgnoreCase)
                                            ? "Ceres Freeport"
                                            : string.Equals(slug, "stickney", StringComparison.OrdinalIgnoreCase)
                                                ? "Stickney Yards"
                                                : slug;
                                    }
                                }
                                if (!string.IsNullOrEmpty(retPortId))
                                {
                                    string planCmd = $"+ship/plan {retPortId}";
                                    if (!string.Equals(pushedSuggestedCmd, planCmd, StringComparison.OrdinalIgnoreCase))
                                    {
                                        string turnIn = !string.IsNullOrEmpty(tracked.TurnInName) ? tracked.TurnInName : retLabel;
                                        string oreBadge = (hasMiningHold && miningHold.StartsWith("[ORE HOLD READY]", StringComparison.Ordinal))
                                            ? " [ORE HOLD READY]"
                                            : string.Empty;
                                        _allEntries.Add(new PaletteEntry(
                                            "QUEST SAIL",
                                            $"Return -> {retLabel}{oreBadge}",
                                            $"{planCmd} — Return to {turnIn} with mined ore",
                                            () => OnPrefillCommandRequested?.Invoke(planCmd),
                                            $"quest return sail plan {retPortId} {retLabel} {planCmd} mined ore"));
                                    }
                                }
                            }
                        }
                    }

                    if (qp.Active.Count > 1)
                    {
                        int curIdx = 0;
                        for (int i = 0; i < qp.Active.Count; i++)
                        {
                            if (qp.Active[i].Key == tracked.Key) { curIdx = i; break; }
                        }
                        var nextQ = qp.Active[(curIdx + 1) % qp.Active.Count];
                        _allEntries.Add(new PaletteEntry(
                            "QUEST CYCLE",
                            $"Cycle Tracked Quest ({curIdx + 1}/{qp.Active.Count})",
                            $"Switch HUD tracker to {nextQ.Title}",
                            () => _quests?.CycleTrackedQuest(),
                            $"quest cycle switch tracked next {nextQ.Title}"));
                    }
                }
                else if (_quests.TryGetNextGuidedOffer(out var nextGuided))
                {
                    if (_quests.TrackerRouteHops > 0 && !string.IsNullOrEmpty(_quests.TrackerRouteCommand))
                    {
                        string rCmd = _quests.TrackerRouteCommand;
                        string rRoom = _quests.TrackerRouteRoomName;
                        int rHops = _quests.TrackerRouteHops;
                        string rTitle = FormatQuestRouteTitle(rCmd, rRoom, rHops);
                        string hopWord = rHops == 1 ? "hop" : "hops";
                        _allEntries.Add(new PaletteEntry(
                            "QUEST ROUTE",
                            rTitle,
                            $"{rHops} {hopWord} to {nextGuided.GiverRoomName} · {rCmd}",
                            () => OnCommandRequested?.Invoke(rCmd),
                            $"quest route walk board ashore vehicle bay {rRoom} {nextGuided.GiverRoomName} {rCmd} next guided"));
                    }

                    string curPort = _station != null
                        ? TacSimQuestJournal.InferStationPortFromRoomEntry(_station.ActiveRoomEntryId)
                        : null;
                    if (_quests.TryGetNextGuidedOfferDestination(curPort, out var nextDest))
                    {
                        string planCmd = $"+ship/plan {nextDest.PortId}";
                        _allEntries.Add(new PaletteEntry(
                            "QUEST SAIL",
                            $"Sail -> {nextDest.PortLabel}",
                            $"{planCmd} — {nextDest.QuestTitle}",
                            () => OnPrefillCommandRequested?.Invoke(planCmd),
                            $"quest sail plan {nextDest.PortId} {nextDest.PortLabel} next guided"));
                    }

                    if (!string.IsNullOrEmpty(nextGuided.GiverRoomEntry))
                    {
                        string targetRoom = nextGuided.GiverRoomEntry;
                        string destLabel = nextGuided.GiverRoomName;
                        if (_station?.Pack != null &&
                            _station.TryResolveLocalQuestRoom(nextGuided.GiverRoomEntry, false, false, out string localRoom, out _))
                        {
                            bool redirected = !string.Equals(localRoom, nextGuided.GiverRoomEntry, StringComparison.OrdinalIgnoreCase);
                            targetRoom = localRoom;
                            if ((redirected || string.IsNullOrEmpty(nextGuided.GiverRoomName)) &&
                                _station.Pack.Rooms.TryGetValue(localRoom, out var roomDef) &&
                                !string.IsNullOrEmpty(roomDef.Name))
                            {
                                destLabel = roomDef.Name;
                            }
                        }
                        _allEntries.Add(new PaletteEntry(
                            "QUEST TRAIL",
                            $"Wayfinder to Next Guided ({nextGuided.Step}/7): {destLabel}",
                            $"Speak with {nextGuided.GiverLabel} ({nextGuided.LocationLabel})",
                            () => OnPlotStationRoomRequested?.Invoke(targetRoom),
                            $"quest trail wayfinder route navigate next guided {destLabel} {nextGuided.GiverRoomName} {nextGuided.GiverLabel}"));
                    }

                    _allEntries.Add(new PaletteEntry(
                        "QUEST [!]",
                        $"Next Guided ({nextGuided.Step}/7): {nextGuided.ShortLabel}",
                        $"Speak with {nextGuided.GiverLabel} ({nextGuided.GiverRoomName})",
                        () => _quests?.SetOpen(true, QuestJournalTab.Quests),
                        $"quest next guided tour offer {nextGuided.ShortLabel} {nextGuided.GiverLabel} {nextGuided.GiverRoomName}"));
                }
                else if (qp.Available.Count == 0 &&
                         _quests.TryGetGuidedTourProgress(out int tourDone, out int tourTotal, out _, out bool allTourDone) &&
                         allTourDone)
                {
                    _allEntries.Add(new PaletteEntry(
                        "QUEST MASTER",
                        $"★ Corridor Captain ({tourDone}/{tourTotal}) — Jobs Board",
                        "+market/jobs — Browse corridor freight & mining contracts",
                        () => OnPrefillCommandRequested?.Invoke("+market/jobs"),
                        "quest corridor captain jobs board market +market/jobs guided tour complete"));
                }
            }

            // 1. UI Windows & Quick Controls
            _allEntries.Add(new PaletteEntry("UI", "Open Pilot & Vessel Dossier [C]", "Avatar swap, cargo, space fit & ground rigs", () => OnUiActionRequested?.Invoke("ui:sheet")));
            _allEntries.Add(new PaletteEntry("UI", "Open Solar System Holo-Orrery [M]", "2D/3D heliocentric navigation chart", () => OnUiActionRequested?.Invoke("ui:solar_map")));
            _allEntries.Add(new PaletteEntry("UI", "Open Planetary Ground Survey Chart [G]", "Live body.ground mineral wash & landing sites", () => OnUiActionRequested?.Invoke("ui:ground_chart")));
            _allEntries.Add(new PaletteEntry("UI", "Open Nav Directory & Room Roster [N]", "3D wayfinder directory, cast & active ops", () => OnUiActionRequested?.Invoke("ui:ops_dir")));
            _allEntries.Add(new PaletteEntry("UI", "Open Social, Online Pilots, IC Mail & Channels [O]", "Live WHO roster, @mail inbox/compose & comsys channels", () => OnUiActionRequested?.Invoke("ui:social")));
            _allEntries.Add(new PaletteEntry("UI", "Open Mission, Passage, Survey & Claims Journal [J]", "Active quests, passages, surveys, claims & HUD objective pin", () => OnUiActionRequested?.Invoke("ui:journal")));
            _allEntries.Add(new PaletteEntry("UI", "Open RP Scene Tracker & Signal Contacts Intel [L]", "Active timestop scene turn order, recap & light-delay signal contacts", () => OnUiActionRequested?.Invoke("ui:scene_intel")));
            _allEntries.Add(new PaletteEntry("UI", "Open Controls, Keybinds & Macros [F1]", "Keybind reference & F5-F8 custom macros", () => OnUiActionRequested?.Invoke("ui:keybinds")));
            _allEntries.Add(new PaletteEntry("UI", "Open Port Commodity Exchange", "Live market book, cargo gauges & trade", () => OnCommandRequested?.Invoke("ui:open_exchange")));
            _allEntries.Add(new PaletteEntry("UI", "Open Sailing Plan Builder", "Plot corridor fuel burn & departure orders", () => OnCommandRequested?.Invoke("ui:open_sailing")));
            _allEntries.Add(new PaletteEntry("UI", "Open Display, Audio & Camera Options", "Window size, UI scale, audio mixer, camera FOV & HUD density", () => OnUiActionRequested?.Invoke("ui:options")));
            _allEntries.Add(new PaletteEntry("UI", "Toggle Cinematic HUD Mode [F9]", "Hide/restore non-essential HUD chrome for clean immersion", () => OnUiActionRequested?.Invoke("ui:toggle_cinematic_hud")));
            _allEntries.Add(new PaletteEntry("UI", "Cycle Orbital / Vehicle Camera [V]", "Switch Bridge / Chase / Tactical High-Orbit", () => OnUiActionRequested?.Invoke("ui:cycle_cam")));
            _allEntries.Add(new PaletteEntry("UI", "Cycle 3D Bridge Holo-Table Mode", "Toggle Auto / Solar System / Local Tactical Bubble", () => OnUiActionRequested?.Invoke("ui:cycle_holo")));
            _allEntries.Add(new PaletteEntry("UI", "Toggle Tactical Overview Drawer [T]", "Live range-sorted space contacts, surface ore pockets & arena targets", () => OnUiActionRequested?.Invoke("ui:toggle_overview")));
            _allEntries.Add(new PaletteEntry("UI", "Clear 3D Wayfinder Trail", "Hide active holographic floor ribbon", () => OnUiActionRequested?.Invoke("ui:clear_wayfinder")));

            // 2. Live 3D Station Rooms & Ship Compartments (Wayfinder Fast-Trail)
            bool aboardShip = _activeStage == TacSimActiveStage.ShipInteriorWalk || _activeStage == TacSimActiveStage.ShipHelmSeated;
            if (aboardShip && _ship?.Template != null)
            {
                foreach (var comp in _ship.Template.Compartments.Values)
                {
                    if (string.Equals(comp.Id, _ship.ActiveCompartmentId, StringComparison.OrdinalIgnoreCase)) continue;
                    string cid = comp.Id;
                    string cname = string.IsNullOrWhiteSpace(comp.Name) ? cid.ToUpperInvariant() : comp.Name;
                    _allEntries.Add(new PaletteEntry("SHIP TRAIL", $"Walk to {cname}", $"Plot 3D ship deck trail ({cid})", () => OnPlotShipCompartmentRequested?.Invoke(cid)));
                }
            }
            else if (_station?.Pack != null)
            {
                foreach (var room in _station.Pack.Rooms.Values)
                {
                    if (string.Equals(room.EntryId, _station.ActiveRoomEntryId, StringComparison.OrdinalIgnoreCase)) continue;
                    if (_station.FindExitPath(_station.ActiveRoomEntryId, room.EntryId).Count == 0) continue;
                    string rid = room.EntryId;
                    string rname = string.IsNullOrWhiteSpace(room.Name) ? rid : room.Name;
                    _allEntries.Add(new PaletteEntry("3D TRAIL", $"Walk to {rname}", $"Floor {room.FloorIndex + 1} · Plot 3D wayfinder trail", () => OnPlotStationRoomRequested?.Invoke(rid)));
                }
            }

            // 3. Verified Registered mushd Gameplay Commands
            AddCmd(curatedVerbs, "OPS", "Quests & Local Favors", "+quest");
            AddCmd(curatedVerbs, "OPS", "Passenger Fares & Bookings", "+passage");
            AddCmd(curatedVerbs, "OPS", "Survey Commissions", "+survey");
            AddCmd(curatedVerbs, "OPS", "Mining Claims Registry", "+claim/list");
            AddCmd(curatedVerbs, "OPS", "Haul Contracts", "+haul");
            AddCmd(curatedVerbs, "OPS", "Escort Contracts", "+escort");
            AddCmd(curatedVerbs, "OPS", "Open Job Tickets", "+jobs");
            AddCmd(curatedVerbs, "SHIP", "Ship Status & Telemetry", "+ship");
            AddCmd(curatedVerbs, "SHIP", "Cargo Hold Manifest", "+ship/hold");
            AddCmd(curatedVerbs, "SHIP", "Active Sailing Orders", "+ship/orders");
            AddCmd(curatedVerbs, "SHIP", "Co-op Charter Status", "+coop");
            AddCmd(curatedVerbs, "SHIP", "Co-op Financial Ledger", "+coop/statement");
            AddCmd(curatedVerbs, "SHIP", "Available Credits", "+credit");
            AddCmd(curatedVerbs, "SHIP", "Prospecting Licence", "+license");
            AddCmd(curatedVerbs, "SHIP", "Market Listings", "+market/list");
            AddCmd(curatedVerbs, "SPACE", "Helm Status", "+helm/status");
            AddCmd(curatedVerbs, "SPACE", "Tactical Sensor Scan", "+tac/scan");
            AddCmd(curatedVerbs, "SPACE", "Tactical Weapon Mounts", "+tac/status");
            AddCmd(curatedVerbs, "INTEL", "Signal Propagation Contacts", "+contacts");
            AddCmd(curatedVerbs, "INTEL", "Active Tactical Orders", "+orders");
            AddCmd(curatedVerbs, "RP", "Active Scene & Turn Order", "+scene");
            AddCmd(curatedVerbs, "RP", "Scene Narrative Recap", "+recap");
            AddCmd(curatedVerbs, "RP", "Start Room Timestop Scene", "+timestop");
            AddCmd(curatedVerbs, "SPACE", "Ping Asteroid Cluster", "+mine/ping");
            AddCmd(curatedVerbs, "SPACE", "Solar Ephemeris Table", "+chart");
            AddCmd(curatedVerbs, "SPACE", "Planetary Ground Survey", "+chart/ground");
            AddCmd(curatedVerbs, "WORLD", "Step Out of Character", "+ooc");
            AddCmd(curatedVerbs, "WORLD", "Return In Character", "+ic");
            AddCmd(curatedVerbs, "ARENA", "Simulator Match History (Replay IDs)", "sim history");
            AddCmd(curatedVerbs, "WORLD", "Online Pilots", "WHO");
            AddCmd(curatedVerbs, "WORLD", "Look Around", "look");

            // 4. Corridor Solar Ports
            for (int i = 0; i < CorridorPorts.Length; i++)
            {
                string slug = CorridorPorts[i].slug;
                string label = CorridorPorts[i].label;
                _allEntries.Add(new PaletteEntry("SOLAR PORT", $"Chart & Plan: {label}", $"Inspect {slug} in Solar Orrery [M]", () => OnInspectSolarPortRequested?.Invoke(slug)));
            }

            // 5. Live Role-Authorized Server Commands (`commands.list` panel hydration)
            for (int i = 0; i < _serverCommands.Count; i++)
            {
                var sc = _serverCommands[i];
                if (curatedVerbs.Contains(sc.Name)) continue;

                string verb = sc.Name;
                bool takesArgs = CommandTakesArguments(sc.Usage, verb);
                string title = !string.IsNullOrEmpty(sc.Usage) ? sc.Usage : verb;
                if (title.Length > 48) title = title.Substring(0, 48) + "…";
                string sub = !string.IsNullOrEmpty(sc.Summary)
                    ? sc.Summary
                    : (takesArgs ? "Pre-fill command in chat input" : "Execute live server command");
                if (sub.Length > 68) sub = sub.Substring(0, 68) + "…";

                _allEntries.Add(new PaletteEntry(
                    takesArgs ? "SRV FILL" : "SRV CMD",
                    title,
                    sub,
                    takesArgs
                        ? (Action)(() => OnPrefillCommandRequested?.Invoke(verb + " "))
                        : (Action)(() => OnCommandRequested?.Invoke(verb)),
                    sc.ExtraKeywords));
            }
        }

        private void AddCmd(HashSet<string> curatedVerbs, string cat, string title, string cmd)
        {
            curatedVerbs?.Add(cmd);
            _allEntries.Add(new PaletteEntry(cat, title, cmd, () => OnCommandRequested?.Invoke(cmd)));
        }

        public void SetQuery(string query)
        {
            _filteredIndices.Clear();
            var scored = new List<(int idx, int score)>(_allEntries.Count);
            for (int i = 0; i < _allEntries.Count; i++)
            {
                int s = ScoreFuzzyMatch(_allEntries[i].SearchBlob, query);
                if (s >= 0)
                    scored.Add((i, s));
            }
            scored.Sort((a, b) =>
            {
                int c = b.score.CompareTo(a.score);
                return c != 0 ? c : a.idx.CompareTo(b.idx);
            });
            for (int i = 0; i < scored.Count; i++)
                _filteredIndices.Add(scored[i].idx);

            _selectedCursor = _filteredIndices.Count > 0 ? 0 : -1;
            RefreshRowVisuals();
        }

        public void MoveSelection(int delta)
        {
            int count = Mathf.Min(MaxVisibleRows, _filteredIndices.Count);
            if (count <= 0)
            {
                _selectedCursor = -1;
                RefreshRowVisuals();
                return;
            }
            _selectedCursor = ((_selectedCursor < 0 ? 0 : _selectedCursor) + delta % count + count) % count;
            RefreshRowVisuals();
        }

        public bool ExecuteSelected()
        {
            if (_selectedCursor >= 0 && _selectedCursor < _filteredIndices.Count && _selectedCursor < MaxVisibleRows)
            {
                var entry = _allEntries[_filteredIndices[_selectedCursor]];
                SetOpen(false);
                entry.Execute?.Invoke();
                return true;
            }

            // Fallback: if the user typed a direct MUSH command not in the catalog, dispatch it cleanly
            string raw = _searchInput != null ? _searchInput.text.Trim() : string.Empty;
            if (!string.IsNullOrWhiteSpace(raw) && !MushProtocol.ContainsControlChars(raw))
            {
                SetOpen(false);
                OnCommandRequested?.Invoke(raw);
                return true;
            }
            return false;
        }

        private void Update()
        {
            if (!IsOpen) return;
            if (Input.GetKeyDown(KeyCode.DownArrow))
                MoveSelection(+1);
            else if (Input.GetKeyDown(KeyCode.UpArrow))
                MoveSelection(-1);
        }

        private void RefreshRowVisuals()
        {
            int shown = Mathf.Min(MaxVisibleRows, _filteredIndices.Count);
            for (int i = 0; i < _rowPool.Count; i++)
            {
                var row = _rowPool[i];
                if (i < shown)
                {
                    var e = _allEntries[_filteredIndices[i]];
                    row.MatchedIndex = _filteredIndices[i];
                    bool selected = i == _selectedCursor;
                    row.Background.color = selected
                        ? new Color(0.16f, 0.44f, 0.62f, 0.98f)
                        : new Color(0.06f, 0.11f, 0.17f, 0.90f);
                    string catColor = e.Category.StartsWith("QUEST", StringComparison.Ordinal) ? "#FFDD55"
                                    : (e.Category == "3D TRAIL" || e.Category == "SHIP TRAIL" ? "#66FFAA"
                                    : (e.Category == "SOLAR PORT" ? "#FFCC66"
                                    : (e.Category == "SPACE" ? "#FF8866"
                                    : (e.Category == "SRV FILL" ? "#DDBBFF"
                                    : (e.Category == "SRV CMD" ? "#77EEBB" : "#66CCFF")))));
                    row.Label.text = $"<color={catColor}>[{e.Category}]</color> <b>{MushProtocol.EscapeUnityRichText(e.Title)}</b>  <color=#88AAC8>— {MushProtocol.EscapeUnityRichText(e.Subtitle)}</color>";
                    row.Root.SetActive(true);
                }
                else
                {
                    row.MatchedIndex = -1;
                    row.Root.SetActive(false);
                }
            }

            if (_footerText != null)
            {
                string srvNote = _serverCommands.Count > 0 ? $" (+{_serverCommands.Count} live cmds)" : string.Empty;
                _footerText.text = shown > 0
                    ? $"Showing {shown} of {_filteredIndices.Count} matches{srvNote}  |  [Up/Down] Select  [Enter] Execute  [Esc] Close"
                    : "No preset match — press [Enter] to send directly to mushd, or [Esc] to close";
            }
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

        private Button CreateButton(Transform parent, string label, Color bg, Action onClick)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.onClick.AddListener(() => onClick?.Invoke());
            var txt = CreateText(go.transform, label, 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            return btn;
        }

        private InputField CreateInputField(Transform parent, string placeholder, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax)
        {
            var go = CreatePanel(parent, "PaletteSearchInput", aMin, aMax, oMin, oMax, new Color(0.08f, 0.14f, 0.22f, 0.96f));
            var input = go.AddComponent<InputField>();
            var ph = CreateText(go.transform, placeholder, 13, FontStyle.Italic, TextAnchor.MiddleLeft, new Color(0.55f, 0.68f, 0.78f));
            SetRect(ph.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 2f), new Vector2(-10f, -2f));
            var val = CreateText(go.transform, string.Empty, 14, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
            SetRect(val.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 2f), new Vector2(-10f, -2f));
            input.placeholder = ph;
            input.textComponent = val;
            return input;
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
