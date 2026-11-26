using System;
using System.Collections.Generic;
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

        public PaletteEntry(string category, string title, string subtitle, Action execute)
        {
            Category = category ?? string.Empty;
            Title = title ?? string.Empty;
            Subtitle = subtitle ?? string.Empty;
            SearchBlob = $"{Category} {Title} {Subtitle}".ToLowerInvariant();
            Execute = execute;
        }
    }

    /// <summary>
    /// Spotlight / VSCode / EVE-style `Ctrl+K` Fuzzy Command &amp; Fast-Travel Palette.
    /// Searches live 3D Station rooms &amp; Ship compartments (lighting up 3D Wayfinder trails),
    /// Solar System ports, registered `mushd` verbs, and MMORPG UI modals with full keyboard control.
    /// </summary>
    public sealed class TacSimCommandPalette : MonoBehaviour
    {
        public event Action<string> OnCommandRequested;
        public event Action<string> OnPlotStationRoomRequested;
        public event Action<string> OnPlotShipCompartmentRequested;
        public event Action<string> OnInspectSolarPortRequested;
        public event Action<string> OnUiActionRequested;

        private const int MaxVisibleRows = 10;

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

        private readonly List<PaletteEntry> _allEntries = new List<PaletteEntry>(96);
        private readonly List<int> _filteredIndices = new List<int>(96);
        private int _selectedCursor;

        private StationWorldController _station;
        private ShipInteriorController _ship;
        private TacSimActiveStage _activeStage = TacSimActiveStage.StationWalk;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public bool IsSearchFocused => IsOpen && _searchInput != null && _searchInput.isFocused;
        public int FilteredMatchCount => _filteredIndices.Count;
        public int SelectedIndex => _selectedCursor;
        public string SelectedEntryTitle =>
            (_selectedCursor >= 0 && _selectedCursor < _filteredIndices.Count)
                ? _allEntries[_filteredIndices[_selectedCursor]].Title
                : string.Empty;

        private static readonly (string slug, string label)[] CorridorPorts =
        {
            ("ceres", "Ceres Freeport"),
            ("stickney", "Stickney Yards (Phobos)"),
            ("luna", "Aitken Highport (Luna)"),
            ("deimos", "Voltaire Anchorage (Deimos)"),
            ("titan", "Ligeia Terminal (Titan)"),
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

        public void BindWorldSources(StationWorldController station, ShipInteriorController ship)
        {
            _station = station;
            _ship = ship;
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

        public void ResetSessionState() => SetOpen(false);

        public void RebuildCatalog()
        {
            _allEntries.Clear();

            // 1. UI Windows & Quick Controls
            _allEntries.Add(new PaletteEntry("UI", "Open Pilot & Vessel Dossier [C]", "Avatar swap, cargo, space fit & ground rigs", () => OnUiActionRequested?.Invoke("ui:sheet")));
            _allEntries.Add(new PaletteEntry("UI", "Open Solar System Holo-Orrery [M]", "2D/3D heliocentric navigation chart", () => OnUiActionRequested?.Invoke("ui:solar_map")));
            _allEntries.Add(new PaletteEntry("UI", "Open Nav Directory & Room Roster [N]", "3D wayfinder directory, cast & active ops", () => OnUiActionRequested?.Invoke("ui:ops_dir")));
            _allEntries.Add(new PaletteEntry("UI", "Open Social, Online Pilots, IC Mail & Channels [O]", "Live WHO roster, @mail inbox/compose & comsys channels", () => OnUiActionRequested?.Invoke("ui:social")));
            _allEntries.Add(new PaletteEntry("UI", "Open Mission, Passage, Survey & Claims Journal [J]", "Active quests, passages, surveys, claims & HUD objective pin", () => OnUiActionRequested?.Invoke("ui:journal")));
            _allEntries.Add(new PaletteEntry("UI", "Open Controls, Keybinds & Macros [F1]", "Keybind reference & F5-F8 custom macros", () => OnUiActionRequested?.Invoke("ui:keybinds")));
            _allEntries.Add(new PaletteEntry("UI", "Open Port Commodity Exchange", "Live market book, cargo gauges & trade", () => OnCommandRequested?.Invoke("ui:open_exchange")));
            _allEntries.Add(new PaletteEntry("UI", "Open Sailing Plan Builder", "Plot corridor fuel burn & departure orders", () => OnCommandRequested?.Invoke("ui:open_sailing")));
            _allEntries.Add(new PaletteEntry("UI", "Open Display & Graphics Options", "Window size, fullscreen & UI scale", () => OnUiActionRequested?.Invoke("ui:options")));
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
            AddCmd("OPS", "Quests & Local Favors", "+quest");
            AddCmd("OPS", "Passenger Fares & Bookings", "+passage");
            AddCmd("OPS", "Survey Commissions", "+survey");
            AddCmd("OPS", "Mining Claims Registry", "+claim/list");
            AddCmd("OPS", "Haul Contracts", "+haul");
            AddCmd("OPS", "Escort Contracts", "+escort");
            AddCmd("OPS", "Open Job Tickets", "+jobs");
            AddCmd("SHIP", "Ship Status & Telemetry", "+ship");
            AddCmd("SHIP", "Cargo Hold Manifest", "+ship/hold");
            AddCmd("SHIP", "Active Sailing Orders", "+ship/orders");
            AddCmd("SHIP", "Co-op Charter Status", "+coop");
            AddCmd("SHIP", "Co-op Financial Ledger", "+coop/statement");
            AddCmd("SHIP", "Available Credits", "+credit");
            AddCmd("SHIP", "Prospecting Licence", "+license");
            AddCmd("SHIP", "Market Listings", "+market/list");
            AddCmd("SPACE", "Helm Status", "+helm/status");
            AddCmd("SPACE", "Tactical Sensor Scan", "+tac/scan");
            AddCmd("SPACE", "Tactical Weapon Mounts", "+tac/status");
            AddCmd("SPACE", "Ping Asteroid Cluster", "+mine/ping");
            AddCmd("SPACE", "Solar Ephemeris Table", "+chart");
            AddCmd("WORLD", "Step Out of Character", "+ooc");
            AddCmd("WORLD", "Return In Character", "+ic");
            AddCmd("WORLD", "Online Pilots", "WHO");
            AddCmd("WORLD", "Look Around", "look");

            // 4. Corridor Solar Ports
            for (int i = 0; i < CorridorPorts.Length; i++)
            {
                string slug = CorridorPorts[i].slug;
                string label = CorridorPorts[i].label;
                _allEntries.Add(new PaletteEntry("SOLAR PORT", $"Chart & Plan: {label}", $"Inspect {slug} in Solar Orrery [M]", () => OnInspectSolarPortRequested?.Invoke(slug)));
            }
        }

        private void AddCmd(string cat, string title, string cmd)
        {
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
                    string catColor = e.Category == "3D TRAIL" || e.Category == "SHIP TRAIL" ? "#66FFAA"
                                    : (e.Category == "SOLAR PORT" ? "#FFCC66"
                                    : (e.Category == "SPACE" ? "#FF8866" : "#66CCFF"));
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
                _footerText.text = shown > 0
                    ? $"Showing {shown} of {_filteredIndices.Count} matches  |  [Up/Down] Select  [Enter] Execute  [Esc] Close"
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
