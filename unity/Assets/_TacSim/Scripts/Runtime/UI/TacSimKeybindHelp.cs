using System;
using TacSim.Net;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    /// <summary>
    /// Interactive `[F1]` Controls / Keybinds Reference &amp; Persistent Custom Macro Hotbar (`F5`–`F8`).
    /// Saves user-defined command macros to `PlayerPrefs` so players can bind favorite `mushd` workflows
    /// to single-key F5–F8 shortcuts across every gameplay stage.
    /// </summary>
    public sealed class TacSimKeybindHelp : MonoBehaviour
    {
        public const int MacroSlotCount = 4;
        private const string PrefLabelPrefix = "TacSim.MacroLabel.";
        private const string PrefCmdPrefix = "TacSim.MacroCmd.";

        private static readonly string[] DefaultLabels = { "Ship Status", "Quests", "Credits", "Tac Scan" };
        private static readonly string[] DefaultCommands = { "+ship", "+quest", "+credit", "+tac/scan" };
        private static readonly string[] ArenaPresetLabels = { "Sensor Scan", "EMCON Toggle", "Squad Engage", "All Stop" };
        private static readonly string[] ArenaPresetCommands = { "scan", "emcon", "order all engage", "speed 0" };

        public event Action<string, string> OnMacroTriggered; // (slotLabel, command)
        public string ArenaKeybindCardSnapshot { get; private set; } = string.Empty;

        private Font _font;
        private GameObject _modalRoot;
        private readonly string[] _macroLabels = new string[MacroSlotCount];
        private readonly string[] _macroCommands = new string[MacroSlotCount];
        private readonly InputField[] _labelInputs = new InputField[MacroSlotCount];
        private readonly InputField[] _cmdInputs = new InputField[MacroSlotCount];
        private Text _saveStatusText;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;

        public bool IsAnyInputFocused
        {
            get
            {
                if (!IsOpen) return false;
                for (int i = 0; i < MacroSlotCount; i++)
                {
                    if ((_labelInputs[i] != null && _labelInputs[i].isFocused) ||
                        (_cmdInputs[i] != null && _cmdInputs[i].isFocused))
                    {
                        return true;
                    }
                }
                return false;
            }
        }

        public string GetMacroLabel(int index) =>
            (index >= 0 && index < MacroSlotCount) ? (_macroLabels[index] ?? string.Empty) : string.Empty;

        public string GetMacroCommand(int index) =>
            (index >= 0 && index < MacroSlotCount) ? (_macroCommands[index] ?? string.Empty) : string.Empty;

        public void BuildModal(Transform parent, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            LoadMacros();

            _modalRoot = CreatePanel(
                parent,
                "KeybindsAndMacrosModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-470f, -300f),
                new Vector2(470f, 300f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            CreatePanel(_modalRoot.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.35f, 0.88f, 1f, 0.95f));

            var hdr = CreateText(_modalRoot.transform, "CONTROLS, KEYBINDS & CUSTOM MACROS [F1]", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.92f, 1f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(0.76f, 1f), new Vector2(18f, -42f), new Vector2(0f, -8f));

            var closeBtn = CreateButton(_modalRoot.transform, "CLOSE [ESC]", new Color(0.46f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -40f), new Vector2(-14f, -10f));

            // Top: 2x2 Keybind Reference Grid
            var grid = CreatePanel(_modalRoot.transform, "KeybindColumns", new Vector2(0f, 0.44f), new Vector2(1f, 1f), new Vector2(16f, 6f), new Vector2(-16f, -48f), Color.clear);
            BuildKeybindCard(
                grid.transform,
                "MMORPG INTERFACE & WINDOWS",
                new Vector2(0f, 0.51f), new Vector2(0.495f, 1f),
                "<b>[Ctrl+K]</b> Quick Command & Fast-Travel Palette\n" +
                "<b>[C]</b> Pilot/Ship Sheet   <b>[M]</b> Solar Orrery   <b>[G]</b> Ground Survey\n" +
                "<b>[N]</b> 3D Wayfinder Directory & Room Roster\n" +
                "<b>[O]</b> Social, Online WHO, IC Mail & Channels\n" +
                "<b>[J]</b> Mission/Passage/Survey/Claims Journal   <b>[L]</b> RP Scene & Signal Intel\n" +
                "<b>[F9]</b> Cinematic HUD  <b>[F10]</b> Minds  <b>[F11]</b> Fullscreen");

            BuildKeybindCard(
                grid.transform,
                "STATION & SHIP WALKING",
                new Vector2(0.505f, 0.51f), new Vector2(1f, 1f),
                "<b>[W A S D]</b> Walk   <b>[Shift]</b> Sprint\n" +
                "<b>[RMB Drag]</b> Orbit Camera   <b>[Scroll]</b> Zoom\n" +
                "<b>[E]</b> Interact (NPC / Airlock / Helm / Desk)\n" +
                "<b>[1–9]</b> Trigger Numbered Action Hotbar\n" +
                "<b>[Minimap Click]</b> Plot 3D Holo Wayfinder Trail\n" +
                "<b>[Enter / Up / Down]</b> Chat & Command History");

            BuildKeybindCard(
                grid.transform,
                "ORBITAL SPACE FLIGHT & 6-DOF DOCKING",
                new Vector2(0f, 0f), new Vector2(0.495f, 0.49f),
                "<b>[V]</b> Cycle Camera: Bridge / Chase / High-Orbit\n" +
                "<b>[Tab]</b> Cycle Radar Contacts & Asteroids\n" +
                "<b>[Space]</b> Fire Primary Beam at Held Lock\n" +
                "<b>[LMB 3D / Bracket]</b> Lock Ship or Approach Rock\n" +
                "<b>Docking:</b> [WASD] Trans  [R/F] Closure  [Arrows] Att\n" +
                "<b>Docking:</b> [Q/E] Roll  [X] Dampeners  [G] Assist");

            string arenaCardBody =
                "<b>[W A S D]</b> Drive / Helm   <b>[1–4 / 0 / X]</b> Throttle / Stop\n" +
                "<b>[LMB Unit]</b> Lock   <b>[Shift+LMB / Minimap]</b> Drive Cell\n" +
                "<b>[Space]</b> Fire   <b>[G]</b> Volley   <b>[F]</b> Hold/Free   <b>[Tab]</b> Cycle\n" +
                "<b>[N]</b> Scan   <b>[M]</b> EMCON   <b>[L]</b> Auto-Lock   <b>[V/Q]</b> Cam/Optics\n" +
                "<b>[Y]</b> Detachment Wilco   <b>[Z]</b> Startup   <b>[F5–F8]</b> Macros\n" +
                "<b>Replay:</b> [Space] Play  [←/→] KF  [[/]] Event  [R] 0:00  [1–6/Tab] Unit";
            ArenaKeybindCardSnapshot = arenaCardBody;
            BuildKeybindCard(
                grid.transform,
                "PLANETARY ROVER, OOC ARENA & REPLAY",
                new Vector2(0.505f, 0f), new Vector2(1f, 0.49f),
                arenaCardBody);

            // Bottom: 4 Persistent Custom Macros (F5-F8)
            var macroBox = CreatePanel(_modalRoot.transform, "CustomMacrosPanel", new Vector2(0f, 0f), new Vector2(1f, 0.43f), new Vector2(16f, 14f), new Vector2(-16f, -4f), new Color(0.05f, 0.09f, 0.15f, 0.94f));
            var mHdr = CreateText(macroBox.transform, "CUSTOM QUICK-MACRO HOTKEYS (F5 – F8)", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.84f, 0.42f));
            SetRect(mHdr.rectTransform, new Vector2(0f, 1f), new Vector2(0.46f, 1f), new Vector2(14f, -30f), new Vector2(0f, -6f));

            var spacePresetBtn = CreateButton(macroBox.transform, "SPACE PRESET", new Color(0.18f, 0.30f, 0.46f), () => ApplySpaceMacroPreset(persist: true));
            SetRect(spacePresetBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-416f, -30f), new Vector2(-288f, -6f));

            var arenaPresetBtn = CreateButton(macroBox.transform, "ARENA PRESET", new Color(0.48f, 0.24f, 0.16f), () => ApplyArenaMacroPreset(persist: true));
            SetRect(arenaPresetBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-280f, -30f), new Vector2(-152f, -6f));

            var saveBtn = CreateButton(macroBox.transform, "SAVE MACROS", new Color(0.14f, 0.48f, 0.38f), SaveMacrosFromInputs);
            SetRect(saveBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-144f, -30f), new Vector2(-12f, -6f));

            for (int i = 0; i < MacroSlotCount; i++)
            {
                int slot = i;
                float yTop = -36f - i * 42f;
                float yBot = yTop - 36f;

                var keyBadge = CreateText(macroBox.transform, $"[F{5 + i}]", 13, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.45f, 0.95f, 1f));
                SetRect(keyBadge.rectTransform, new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(10f, yBot), new Vector2(62f, yTop));

                _labelInputs[i] = CreateInputField(macroBox.transform, "Macro label...", new Vector2(0f, 1f), new Vector2(0.32f, 1f), new Vector2(66f, yBot + 2f), new Vector2(-6f, yTop - 2f));
                _labelInputs[i].SetTextWithoutNotify(_macroLabels[i]);

                _cmdInputs[i] = CreateInputField(macroBox.transform, "Command (e.g. +ship, +quest, +tac/scan)...", new Vector2(0.32f, 1f), new Vector2(1f, 1f), new Vector2(6f, yBot + 2f), new Vector2(-94f, yTop - 2f));
                _cmdInputs[i].SetTextWithoutNotify(_macroCommands[i]);

                var runBtn = CreateButton(macroBox.transform, "RUN", new Color(0.16f, 0.36f, 0.54f), () =>
                {
                    SaveMacrosFromInputs();
                    TriggerMacroSlot(slot);
                });
                SetRect(runBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-86f, yBot + 2f), new Vector2(-12f, yTop - 2f));
            }

            _saveStatusText = CreateText(macroBox.transform, "Press F5, F6, F7, or F8 during gameplay to execute your bound macros.", 11, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.70f, 0.84f, 0.94f));
            SetRect(_saveStatusText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(14f, 6f), new Vector2(-14f, 24f));

            _modalRoot.SetActive(false);
        }

        private void BuildKeybindCard(Transform parent, string title, Vector2 aMin, Vector2 aMax, string richBody)
        {
            var card = CreatePanel(parent, "Card_" + title, aMin, aMax, Vector2.zero, Vector2.zero, new Color(0.05f, 0.09f, 0.14f, 0.92f));
            var hdr = CreateText(card.transform, title, 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.45f, 0.92f, 0.88f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -22f), new Vector2(-10f, -2f));

            var body = CreateText(card.transform, richBody, 11, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.84f, 0.92f, 0.98f));
            body.supportRichText = true;
            SetRect(body.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 4f), new Vector2(-10f, -24f));
        }

        public void SetOpen(bool open)
        {
            if (_modalRoot == null) return;
            _modalRoot.SetActive(open);
            if (open)
            {
                for (int i = 0; i < MacroSlotCount; i++)
                {
                    if (_labelInputs[i] != null) _labelInputs[i].SetTextWithoutNotify(_macroLabels[i]);
                    if (_cmdInputs[i] != null) _cmdInputs[i].SetTextWithoutNotify(_macroCommands[i]);
                }
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void ResetSessionState() => SetOpen(false);

        public void SetMacroSlot(int index, string label, string command, bool persist = true)
        {
            if (index < 0 || index >= MacroSlotCount) return;
            string safeLabel = string.IsNullOrWhiteSpace(label) ? $"Macro F{5 + index}" : label.Trim();
            string safeCmd = (command ?? string.Empty).Trim();
            if (MushProtocol.ContainsControlChars(safeCmd)) return;

            _macroLabels[index] = safeLabel;
            _macroCommands[index] = safeCmd;
            if (_labelInputs[index] != null) _labelInputs[index].SetTextWithoutNotify(safeLabel);
            if (_cmdInputs[index] != null) _cmdInputs[index].SetTextWithoutNotify(safeCmd);

            if (persist && Application.isPlaying)
            {
                PlayerPrefs.SetString(PrefLabelPrefix + index, safeLabel);
                PlayerPrefs.SetString(PrefCmdPrefix + index, safeCmd);
                PlayerPrefs.Save();
            }
        }

        public void SaveMacrosFromInputs()
        {
            for (int i = 0; i < MacroSlotCount; i++)
            {
                string lbl = _labelInputs[i] != null ? _labelInputs[i].text : _macroLabels[i];
                string cmd = _cmdInputs[i] != null ? _cmdInputs[i].text : _macroCommands[i];
                SetMacroSlot(i, lbl, cmd, persist: true);
            }
            if (_saveStatusText != null)
                _saveStatusText.text = "Saved F5–F8 macros cleanly.";
        }

        public void ApplyArenaMacroPreset(bool persist = true)
        {
            for (int i = 0; i < MacroSlotCount; i++)
                SetMacroSlot(i, ArenaPresetLabels[i], ArenaPresetCommands[i], persist);
            if (_saveStatusText != null)
                _saveStatusText.text = "Loaded Arena Combat F5–F8 macro preset (scan / emcon / engage / stop).";
        }

        public void ApplySpaceMacroPreset(bool persist = true)
        {
            for (int i = 0; i < MacroSlotCount; i++)
                SetMacroSlot(i, DefaultLabels[i], DefaultCommands[i], persist);
            if (_saveStatusText != null)
                _saveStatusText.text = "Restored Space & Trade F5–F8 macro preset (+ship / +quest / +credit / +tac/scan).";
        }

        public bool TriggerMacroSlot(int index)
        {
            if (index < 0 || index >= MacroSlotCount) return false;
            string cmd = _macroCommands[index];
            if (string.IsNullOrWhiteSpace(cmd) || MushProtocol.ContainsControlChars(cmd))
                return false;
            OnMacroTriggered?.Invoke(_macroLabels[index], cmd);
            return true;
        }

        private void LoadMacros()
        {
            for (int i = 0; i < MacroSlotCount; i++)
            {
                _macroLabels[i] = Application.isPlaying
                    ? PlayerPrefs.GetString(PrefLabelPrefix + i, DefaultLabels[i])
                    : DefaultLabels[i];
                _macroCommands[i] = Application.isPlaying
                    ? PlayerPrefs.GetString(PrefCmdPrefix + i, DefaultCommands[i])
                    : DefaultCommands[i];
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
            var go = CreatePanel(parent, "MacroInput", aMin, aMax, oMin, oMax, new Color(0.08f, 0.14f, 0.22f, 0.96f));
            var input = go.AddComponent<InputField>();
            var ph = CreateText(go.transform, placeholder, 11, FontStyle.Italic, TextAnchor.MiddleLeft, new Color(0.50f, 0.64f, 0.75f));
            SetRect(ph.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 2f), new Vector2(-8f, -2f));
            var val = CreateText(go.transform, string.Empty, 12, FontStyle.Normal, TextAnchor.MiddleLeft, Color.white);
            SetRect(val.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 2f), new Vector2(-8f, -2f));
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
