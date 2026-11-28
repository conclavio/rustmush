using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Core;
using TacSim.Net;
using TacSim.World;
using UnityEngine;
using UnityEngine.UI;

namespace TacSim.UI
{
    public enum QuestJournalTab
    {
        Quests = 0,
        Passages = 1,
        Surveys = 2,
        Claims = 3
    }

    public readonly struct QuestTradeNeed
    {
        public readonly string QuestTitle;
        public readonly string CheckpointTitle;
        public readonly string CommoditySlug;
        public readonly int TargetQuantity;
        public readonly bool WantsBuy;
        public readonly bool WantsSell;

        public QuestTradeNeed(string questTitle, string checkpointTitle, string commoditySlug, int targetQuantity, bool wantsBuy, bool wantsSell)
        {
            QuestTitle = questTitle ?? string.Empty;
            CheckpointTitle = checkpointTitle ?? string.Empty;
            CommoditySlug = commoditySlug ?? string.Empty;
            TargetQuantity = Mathf.Max(1, targetQuantity);
            WantsBuy = wantsBuy;
            WantsSell = wantsSell;
        }

        public bool MatchesListing(MarketListingRow row)
        {
            if (row == null || string.IsNullOrEmpty(CommoditySlug)) return false;
            bool sideMatch = row.IsWantedBid ? WantsSell : WantsBuy;
            if (!sideMatch) return false;

            string code = NormalizeGoodSlug(row.GoodCode);
            if (!string.IsNullOrEmpty(code) && string.Equals(code, CommoditySlug, StringComparison.OrdinalIgnoreCase))
                return true;

            string item = NormalizeGoodSlug(row.ItemName);
            return !string.IsNullOrEmpty(item) && string.Equals(item, CommoditySlug, StringComparison.OrdinalIgnoreCase);
        }

        public static string NormalizeGoodSlug(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw)) return string.Empty;
            string s = raw.Trim().ToLowerInvariant();
            int colon = s.LastIndexOf(':');
            if (colon >= 0 && colon < s.Length - 1)
                s = s.Substring(colon + 1);
            s = s.Replace('-', '_').Replace(' ', '_');
            while (s.Contains("__", StringComparison.Ordinal))
                s = s.Replace("__", "_");
            return s;
        }
    }

    public readonly struct QuestSurfaceMiningStatus
    {
        public readonly string QuestTitle;
        public readonly string TargetOreSlug;
        public readonly string RecommendedFamily;
        public readonly bool HasScanStep;
        public readonly bool ScanDone;
        public readonly bool HasSoundStep;
        public readonly bool SoundDone;
        public readonly bool HasMineStep;
        public readonly bool MineDone;
        public readonly string NextStepText;

        public QuestSurfaceMiningStatus(
            string questTitle,
            string targetOreSlug,
            string recommendedFamily,
            bool hasScanStep,
            bool scanDone,
            bool hasSoundStep,
            bool soundDone,
            bool hasMineStep,
            bool mineDone,
            string nextStepText)
        {
            QuestTitle = questTitle ?? string.Empty;
            TargetOreSlug = targetOreSlug ?? string.Empty;
            RecommendedFamily = recommendedFamily ?? "carbon";
            HasScanStep = hasScanStep;
            ScanDone = scanDone;
            HasSoundStep = hasSoundStep;
            SoundDone = soundDone;
            HasMineStep = hasMineStep;
            MineDone = mineDone;
            NextStepText = nextStepText ?? string.Empty;
        }

        public string FormatPrepHintLine()
        {
            if (!ScanDone)
                return "Prep: +license  |  +tv/lease  |  +chart/ground";
            if (!MineDone)
                return "Prep: +chart/ground  |  Recover: +tv/1 recover go";
            return $"Recover: +tv/1 recover go (unload {TargetOreSlug} into hold)";
        }
    }

    public readonly struct QuestRefitHoldReadiness
    {
        public readonly bool CargoEmpty;
        public readonly bool ProvisionsEmpty;
        public readonly int CargoTonnes;
        public readonly int ProvisionsDays;
        public readonly int PropellantTonnes;
        public readonly bool Ready;

        public QuestRefitHoldReadiness(bool cargoEmpty, bool provisionsEmpty, int cargoTonnes, int provisionsDays, int propellantTonnes)
        {
            CargoEmpty = cargoEmpty;
            ProvisionsEmpty = provisionsEmpty;
            CargoTonnes = Mathf.Max(0, cargoTonnes);
            ProvisionsDays = Mathf.Max(0, provisionsDays);
            PropellantTonnes = Mathf.Max(0, propellantTonnes);
            Ready = cargoEmpty && provisionsEmpty;
        }
    }

    public readonly struct QuestSailingPreflightReadiness
    {
        public readonly int PropellantTonnes;
        public readonly bool PropellantOk;
        public readonly int ProvisionsDays;
        public readonly bool ProvisionsOk;
        public readonly bool CargoRequired;
        public readonly string CargoLabel;
        public readonly int CargoQty;
        public readonly int CargoTarget;
        public readonly bool CargoOk;
        public readonly bool Ready;

        public QuestSailingPreflightReadiness(
            int propellantTonnes,
            int provisionsDays,
            bool cargoRequired,
            string cargoLabel,
            int cargoQty,
            int cargoTarget)
        {
            PropellantTonnes = Mathf.Max(0, propellantTonnes);
            PropellantOk = PropellantTonnes > 0;
            ProvisionsDays = Mathf.Max(0, provisionsDays);
            ProvisionsOk = ProvisionsDays > 0;
            CargoRequired = cargoRequired;
            CargoLabel = cargoLabel ?? string.Empty;
            CargoQty = Mathf.Max(0, cargoQty);
            CargoTarget = Mathf.Max(0, cargoTarget);
            CargoOk = !cargoRequired || CargoQty >= Mathf.Max(1, CargoTarget);
            Ready = PropellantOk && ProvisionsOk && CargoOk;
        }

        public string FormatInlineSummary()
        {
            string cargoPart = CargoRequired
                ? $" | {CargoLabel} {CargoQty}/{CargoTarget}" + (CargoOk ? " OK" : "!")
                : string.Empty;
            if (Ready)
                return $"[PRE-FLIGHT OK] Fuel {PropellantTonnes}t | Stores {ProvisionsDays}d{cargoPart}";
            string fuelTxt = PropellantOk ? $"{PropellantTonnes}t OK" : "0t!";
            string storesTxt = ProvisionsOk ? $"{ProvisionsDays}d OK" : "0d!";
            return $"[CHECK BEFORE UNDOCK] Fuel {fuelTxt} | Stores {storesTxt}{cargoPart}";
        }
    }

    public readonly struct QuestDestinationHint
    {
        public readonly string QuestTitle;
        public readonly string PortId;
        public readonly string PortLabel;

        public QuestDestinationHint(string questTitle, string portId, string portLabel)
        {
            QuestTitle = questTitle ?? string.Empty;
            PortId = portId ?? string.Empty;
            PortLabel = portLabel ?? string.Empty;
        }
    }

    public enum GuidedTourStepStatus
    {
        Completed = 0,
        Active = 1,
        Available = 2,
        Upcoming = 3
    }

    public readonly struct GuidedTourRoadmapStep
    {
        public readonly int Step;
        public readonly string Key;
        public readonly string ShortLabel;
        public readonly string GiverLabel;
        public readonly string GiverRoomEntry;
        public readonly string GiverRoomName;
        public readonly string LocationLabel;
        public readonly GuidedTourStepStatus Status;
        public readonly bool ReadyToTurnIn;

        public GuidedTourRoadmapStep(
            int step,
            string key,
            string shortLabel,
            string giverLabel,
            string giverRoomEntry,
            string giverRoomName,
            string locationLabel,
            GuidedTourStepStatus status,
            bool readyToTurnIn)
        {
            Step = step;
            Key = key ?? string.Empty;
            ShortLabel = shortLabel ?? string.Empty;
            GiverLabel = giverLabel ?? string.Empty;
            GiverRoomEntry = giverRoomEntry ?? string.Empty;
            GiverRoomName = giverRoomName ?? string.Empty;
            LocationLabel = locationLabel ?? string.Empty;
            Status = status;
            ReadyToTurnIn = readyToTurnIn;
        }
    }

    /// <summary>
    /// WoW-style Quest System for Unity:
    /// 1. Interactive NPC Quest Dialog Modal (narrative, itemized requirements/objectives, rewards,
    ///    and 1-click Accept / Verify / Complete / Abandon / Wayfinder buttons).
    /// 2. Full `[J]` Quest, Passage, Survey &amp; Mining Claims Journal with Active / Available Here /
    ///    Completed sub-tabs powered by the live `"tacsim.quests"` panel feed.
    /// 3. Collapsible right-edge HUD Quest Tracker Card auto-tracking active server quests (or manual pins).
    /// Powered 100% by live verified `mushd` verbs (`+quest`, `+quest/accept`, `+quest/check`,
    /// `+quest/finish`, `+quest/abandon`, `+passage`, `+survey`, `+claim/list`).
    /// </summary>
    public sealed class TacSimQuestJournal : MonoBehaviour
    {
        public event Action<string> OnCommandRequested;
        public event Action<string> OnPrefillCommandRequested;
        public event Action<string> OnWayfinderRoomRequested;
        public event Action OnQuestsPanelUpdated;
        /// <summary>
        /// Fires on genuine server state transitions after initial hydration:
        /// kind is "accepted", "checkpoint", "ready" (ready to turn in), "completed",
        /// or "abandoned", paired with the quest or checkpoint title.
        /// </summary>
        public event Action<string, string> OnQuestMilestoneReached;

        private const int MaxLinesPerTab = 48;

        private static readonly string[] TabCommands =
        {
            "+quest",
            "+passage",
            "+survey",
            "+claim/list"
        };

        private static readonly string[] TabTitles =
        {
            "QUESTS & FAVORS",
            "PASSENGER FARES",
            "SURVEY COMMISSIONS",
            "MINING CLAIMS"
        };

        private Font _font;
        private GameObject _modalRoot;
        private Text _bodyText;
        private Text _tabHintText;
        private readonly List<Image> _tabImages = new List<Image>();

        // Structured Quests sub-tab bar & action controls inside [J] modal
        private GameObject _questSubBar;
        private Image _subActiveImg;
        private Image _subAvailImg;
        private Image _subDoneImg;
        private Text _subActiveLbl;
        private Text _subAvailLbl;
        private Text _subDoneLbl;
        private GameObject _questStructActions;
        private Button _qPrimaryBtn;
        private Text _qPrimaryLbl;
        private Button _qPrefillBtn;
        private Text _qPrefillLbl;
        private Button _qWayfinderBtn;
        private Button _qAbandonBtn;
        private Text _qAbandonLbl;
        private Button _qNextBtn;

        // WoW-style NPC Quest Dialog Modal
        private GameObject _questDialogRoot;
        private Text _dlgCrestText;
        private Image _dlgCrestBg;
        private Text _dlgTitleText;
        private Text _dlgSubtitleText;
        private Text _dlgBodyText;
        private Text _dlgRewardText;
        private Button _dlgPrimaryBtn;
        private Text _dlgPrimaryLbl;
        private Button _dlgWayfinderBtn;
        private Button _dlgAbandonBtn;
        private Text _dlgAbandonLbl;
        private Button _dlgCycleBtn;
        private Text _dlgCycleLbl;

        // Right-edge Collapsible HUD Quest Tracker Card (docked below TacticalMinimap)
        private GameObject _trackerRoot;
        private RectTransform _trackerRect;
        private GameObject _trackerBody;
        private Text _trackerHeaderText;
        private Text _trackerDetailText;
        private Text _trackerCollapseLbl;
        private Button _trackerActionBtn;
        private Text _trackerActionLbl;
        private Button _trackerNavBtn;
        private Button _trackerCycleBtn;
        private bool _trackerCollapsed;

        private readonly List<string>[] _tabLines =
        {
            new List<string>(),
            new List<string>(),
            new List<string>(),
            new List<string>()
        };
        private readonly List<string>[] _tabPlainLines =
        {
            new List<string>(),
            new List<string>(),
            new List<string>(),
            new List<string>()
        };

        private QuestJournalTab? _capturingTab;
        private float _captureExpiresAt;
        private bool _manualPinActive;
        private int _selectedQuestIndex;
        private string _confirmAbandonKey = string.Empty;
        private string _confirmDeclineKey = string.Empty;
        private bool _hasHydratedPanel;
        private TacSimTradeAndShipState _boundTradeState;
        private readonly HashSet<string> _seenActiveKeys = new HashSet<string>(StringComparer.Ordinal);
        private readonly Dictionary<string, (string title, bool allowRestart)> _seenActiveMeta = new Dictionary<string, (string, bool)>(StringComparer.Ordinal);
        private readonly HashSet<string> _seenReadyKeys = new HashSet<string>(StringComparer.Ordinal);
        private readonly HashSet<string> _seenCheckpointKeys = new HashSet<string>(StringComparer.Ordinal);
        private readonly HashSet<string> _seenCompletedKeys = new HashSet<string>(StringComparer.Ordinal);

        public void BindTradeState(TacSimTradeAndShipState trade)
        {
            if (_boundTradeState != null)
                _boundTradeState.OnStateChanged -= HandleBoundTradeStateChanged;
            _boundTradeState = trade;
            if (_boundTradeState != null)
                _boundTradeState.OnStateChanged += HandleBoundTradeStateChanged;
            HandleBoundTradeStateChanged();
        }

        private void OnDestroy()
        {
            if (_boundTradeState != null)
            {
                _boundTradeState.OnStateChanged -= HandleBoundTradeStateChanged;
                _boundTradeState = null;
            }
        }

        private void HandleBoundTradeStateChanged()
        {
            if (LatestPanel == null) return;
            SyncStructuredTrackerCard();
            RefreshJournalBody();
            if (IsQuestDialogOpen)
                RefreshQuestDialogUi();
        }

        public string JournalBodySnapshot => _bodyText != null ? _bodyText.text : string.Empty;
        public string DialogBodySnapshot => _dlgBodyText != null ? _dlgBodyText.text : string.Empty;

        public bool IsOpen => _modalRoot != null && _modalRoot.activeSelf;
        public bool IsQuestDialogOpen => _questDialogRoot != null && _questDialogRoot.activeSelf;
        public bool HasAnyModalOpen => IsOpen || IsQuestDialogOpen;
        public QuestJournalTab ActiveTab { get; private set; } = QuestJournalTab.Quests;
        public string ActiveQuestSubTab { get; private set; } = "active";
        public MushQuestsPanel LatestPanel { get; private set; }
        public string TrackedQuestKey { get; private set; } = string.Empty;
        public string DialogNpcId { get; private set; } = string.Empty;
        public string DialogQuestKey { get; private set; } = string.Empty;
        public int ActiveQuestCount => LatestPanel?.Active.Count ?? 0;

        /// <summary>
        /// Resolves the highest-urgency quest action available in the player's current room:
        /// gold turn-in (<c>?</c>) first, then gold available offer (<c>!</c>).
        /// </summary>
        public bool TryGetTopRoomQuestAction(
            out string symbol,
            out string npcName,
            out string questTitle,
            out string npcId,
            out string questKey,
            out bool isTurnIn)
        {
            symbol = string.Empty;
            npcName = string.Empty;
            questTitle = string.Empty;
            npcId = string.Empty;
            questKey = string.Empty;
            isTurnIn = false;

            var panel = LatestPanel;
            if (panel == null) return false;

            for (int i = 0; i < panel.RoomMarkers.Count; i++)
            {
                var m = panel.RoomMarkers[i];
                if (string.Equals(m.Marker, "turn_in", StringComparison.OrdinalIgnoreCase))
                {
                    symbol = "?";
                    npcName = m.NpcName ?? string.Empty;
                    questTitle = m.QuestTitle ?? string.Empty;
                    npcId = m.NpcId ?? string.Empty;
                    questKey = m.QuestKey ?? string.Empty;
                    isTurnIn = true;
                    return true;
                }
            }

            for (int i = 0; i < panel.Active.Count; i++)
            {
                var q = panel.Active[i];
                if (q.CanFinishNow)
                {
                    symbol = "?";
                    npcName = q.TurnInName ?? string.Empty;
                    questTitle = q.Title ?? string.Empty;
                    npcId = q.TurnInId ?? string.Empty;
                    questKey = q.Key ?? string.Empty;
                    isTurnIn = true;
                    return true;
                }
            }

            for (int i = 0; i < panel.RoomMarkers.Count; i++)
            {
                var m = panel.RoomMarkers[i];
                if (string.Equals(m.Marker, "available", StringComparison.OrdinalIgnoreCase))
                {
                    symbol = "!";
                    npcName = m.NpcName ?? string.Empty;
                    questTitle = m.QuestTitle ?? string.Empty;
                    npcId = m.NpcId ?? string.Empty;
                    questKey = m.QuestKey ?? string.Empty;
                    isTurnIn = false;
                    return true;
                }
            }

            var offer = GetPrioritizedAvailableOffer(panel);
            if (offer != null)
            {
                symbol = "!";
                npcName = offer.GiverName ?? string.Empty;
                questTitle = offer.Title ?? string.Empty;
                npcId = offer.GiverId ?? string.Empty;
                questKey = offer.Key ?? string.Empty;
                isTurnIn = false;
                return true;
            }

            return false;
        }

        public MushAvailableQuest GetPrioritizedAvailableOffer()
        {
            return GetPrioritizedAvailableOffer(LatestPanel);
        }

        private static MushAvailableQuest GetPrioritizedAvailableOffer(MushQuestsPanel panel)
        {
            if (panel == null || panel.Available.Count == 0) return null;
            for (int i = 0; i < panel.Available.Count; i++)
            {
                var av = panel.Available[i];
                if (av != null &&
                    (string.Equals(av.Category, "guided", StringComparison.OrdinalIgnoreCase) ||
                     GuidedStepNumber(av.Key) > 0))
                {
                    return av;
                }
            }
            return panel.Available[0];
        }

        /// <summary>
        /// Counts distinct actionable quests (`turn_in` / `available`) currently present in this room
        /// across room markers, ready active turn-ins, and room offers.
        /// </summary>
        public int CountActionableRoomQuests()
        {
            var panel = LatestPanel;
            if (panel == null) return 0;
            var keys = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            for (int i = 0; i < panel.RoomMarkers.Count; i++)
            {
                var m = panel.RoomMarkers[i];
                if (m == null) continue;
                if (string.Equals(m.Marker, "turn_in", StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(m.Marker, "available", StringComparison.OrdinalIgnoreCase))
                {
                    keys.Add(!string.IsNullOrEmpty(m.QuestKey) ? m.QuestKey : $"{m.Marker}:{m.NpcId}:{m.QuestTitle}");
                }
            }
            for (int i = 0; i < panel.Active.Count; i++)
            {
                var q = panel.Active[i];
                if (q != null && q.CanFinishNow && !string.IsNullOrEmpty(q.Key))
                    keys.Add(q.Key);
            }
            for (int i = 0; i < panel.Available.Count; i++)
            {
                var av = panel.Available[i];
                if (av != null && !string.IsNullOrEmpty(av.Key))
                    keys.Add(av.Key);
            }
            return keys.Count;
        }
        public int AvailableQuestCount => LatestPanel?.Available.Count ?? 0;
        public int CompletedQuestCount => LatestPanel?.Completed.Count ?? 0;

        public bool HasPinnedObjective => !string.IsNullOrEmpty(PinnedTitle);
        public bool IsTrackerVisible => _trackerRoot != null && _trackerRoot.activeSelf;
        public bool IsTrackerCollapsed => _trackerCollapsed;
        public bool IsTrackerWayfinderVisible => _trackerNavBtn != null && _trackerNavBtn.gameObject.activeSelf;
        public bool IsTrackerCycleVisible => _trackerCycleBtn != null && _trackerCycleBtn.gameObject.activeSelf;
        public bool IsJournalPrefillVisible => _qPrefillBtn != null && _qPrefillBtn.gameObject.activeSelf;
        public bool IsJournalWayfinderVisible => _qWayfinderBtn != null && _qWayfinderBtn.gameObject.activeSelf;
        public string JournalPrefillLabelSnapshot => _qPrefillLbl != null ? _qPrefillLbl.text : string.Empty;
        public string JournalPrimaryLabelSnapshot => _qPrimaryLbl != null ? _qPrimaryLbl.text : string.Empty;
        public string GuidedTourProgressSnapshot
        {
            get
            {
                if (!TryGetGuidedTourProgress(out int done, out int total, out int curStep, out bool allDone))
                    return string.Empty;
                var bar = new StringBuilder(total + 2);
                bar.Append('[');
                for (int s = 1; s <= total; s++)
                {
                    if (s <= done) bar.Append('#');
                    else if (!allDone && s == curStep) bar.Append('>');
                    else bar.Append('-');
                }
                bar.Append(']');
                return allDone
                    ? $"CORRIDOR CAPTAIN {bar} {done}/{total}"
                    : $"TOUR {bar} {done}/{total}";
            }
        }

        public string GuidedTourRoadmapSnapshot
        {
            get
            {
                var steps = new List<GuidedTourRoadmapStep>(GuidedTourRoadmapMeta.Length);
                if (!TryGetGuidedTourRoadmap(steps)) return string.Empty;
                var sb = new StringBuilder(192);
                for (int i = 0; i < steps.Count; i++)
                {
                    if (i > 0) sb.Append("  ");
                    var s = steps[i];
                    string badge = s.Status == GuidedTourStepStatus.Completed ? "[✓]"
                        : s.Status == GuidedTourStepStatus.Active ? (s.ReadyToTurnIn ? "[?]" : "[▸]")
                        : s.Status == GuidedTourStepStatus.Available ? "[!]"
                        : "[○]";
                    sb.Append(badge).Append(' ').Append(s.Step).Append(". ").Append(s.ShortLabel);
                }
                return sb.ToString();
            }
        }

        public bool TryGetActiveCoopAction(out string questTitle, out string label, out string command)
        {
            questTitle = string.Empty;
            label = string.Empty;
            command = string.Empty;
            var active = GetTrackedActiveQuest();
            if (active != null &&
                TryGetSuggestedCommand(active, out string sugLbl, out string sugCmd) &&
                sugCmd.StartsWith("+coop/", StringComparison.OrdinalIgnoreCase))
            {
                questTitle = active.Title;
                label = sugLbl;
                command = sugCmd;
                return true;
            }
            return false;
        }

        public bool TryGetRefitHoldReadiness(TacSimTradeAndShipState trade, out QuestRefitHoldReadiness readiness)
        {
            readiness = default;
            if (trade == null) return false;
            if (!TryGetActiveCoopAction(out _, out _, out string cmd) ||
                !cmd.StartsWith("+coop/refit", StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }
            bool hasCargoLots = false;
            for (int i = 0; i < trade.CargoLots.Count; i++)
            {
                if (trade.CargoLots[i] != null && trade.CargoLots[i].Quantity > 0)
                {
                    hasCargoLots = true;
                    break;
                }
            }
            bool cargoEmpty = trade.HoldMassTonnes <= 0 && !hasCargoLots;
            bool provisionsEmpty = trade.StoresCrewDays <= 0;
            readiness = new QuestRefitHoldReadiness(
                cargoEmpty,
                provisionsEmpty,
                trade.HoldMassTonnes,
                trade.StoresCrewDays,
                trade.TankTonnes);
            return true;
        }

        public bool TryGetSailingPreflightReadiness(TacSimTradeAndShipState trade, out QuestSailingPreflightReadiness readiness)
        {
            readiness = default;
            if (trade == null) return false;
            string curVenue = !string.IsNullOrEmpty(trade.ActiveVenue) ? trade.ActiveVenue : trade.AboardDockedPortHint;
            if (!TryGetActiveDestinationPort(out _) && !TryGetNextGuidedOfferDestination(curVenue, out _)) return false;

            bool cargoRequired = false;
            string cargoLabel = string.Empty;
            int cargoQty = 0;
            int cargoTarget = 0;
            QuestTradeNeed need = default;
            if (TryGetActiveTradeNeed(out need) &&
                !string.Equals(need.CommoditySlug, "propellant", StringComparison.OrdinalIgnoreCase) &&
                !string.Equals(need.CommoditySlug, "provisions", StringComparison.OrdinalIgnoreCase))
            {
                cargoRequired = true;
                cargoLabel = need.CommoditySlug.Replace('_', ' ');
                cargoTarget = Mathf.Max(1, need.TargetQuantity);
                cargoQty = trade.GetHoldCommodityQuantity(need.CommoditySlug);
            }

            readiness = new QuestSailingPreflightReadiness(
                trade.TankTonnes,
                trade.StoresCrewDays,
                cargoRequired,
                cargoLabel,
                cargoQty,
                cargoTarget);
            return true;
        }

        public bool NeedsOriginPortProcurement(TacSimTradeAndShipState trade)
        {
            if (trade == null || string.IsNullOrEmpty(trade.ShipHullId)) return false;
            var tracked = GetTrackedActiveQuest();
            if (tracked == null || tracked.ReadyToFinish || tracked.CanFinishNow) return false;
            QuestTradeNeed need = default;
            if (!TryGetFirstTradeNeedForQuest(tracked, out need)) return false;
            if (!need.WantsBuy || !need.WantsSell) return false;
            if (string.Equals(need.CommoditySlug, "propellant", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(need.CommoditySlug, "provisions", StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }
            return trade.GetHoldCommodityQuantity(need.CommoditySlug) < Mathf.Max(1, need.TargetQuantity);
        }

        private static bool TryGetFirstTradeNeedForQuest(MushActiveQuest q, out QuestTradeNeed need)
        {
            need = default;
            if (q == null) return false;
            for (int cIdx = 0; cIdx < q.Checkpoints.Count; cIdx++)
            {
                var cp = q.Checkpoints[cIdx];
                if (cp.Checked || cp.LiveReady) continue;
                if (TryExtractTradeNeedFromCheckpoint(q.Title, cp, out need))
                    return true;
            }
            return false;
        }

        public bool TryGetQuestHoldReadinessSummary(MushActiveQuest q, out string summary, out bool ready)
        {
            summary = string.Empty;
            ready = false;
            if (q == null || _boundTradeState == null || string.IsNullOrEmpty(_boundTradeState.ShipHullId))
                return false;

            if (TryGetSuggestedCommand(q, out _, out string sugCmd) &&
                sugCmd.StartsWith("+coop/refit", StringComparison.OrdinalIgnoreCase))
            {
                bool hasCargoLots = false;
                for (int i = 0; i < _boundTradeState.CargoLots.Count; i++)
                {
                    if (_boundTradeState.CargoLots[i] != null && _boundTradeState.CargoLots[i].Quantity > 0)
                    {
                        hasCargoLots = true;
                        break;
                    }
                }
                bool cargoEmpty = _boundTradeState.HoldMassTonnes <= 0 && !hasCargoLots;
                bool storesEmpty = _boundTradeState.StoresCrewDays <= 0;
                int tankT = Mathf.Max(0, _boundTradeState.TankTonnes);
                ready = cargoEmpty && storesEmpty;
                string cargoTxt = cargoEmpty ? "Cargo empty" : $"Cargo {_boundTradeState.HoldMassTonnes}t";
                string storesTxt = storesEmpty ? "Stores empty" : $"Stores {_boundTradeState.StoresCrewDays}d";
                string tag = ready ? "[READY]" : "[REFIT CHECK]";
                summary = $"{tag} {cargoTxt} | {storesTxt} | Tank {tankT}t OK";
                return true;
            }

            var qNeeds = new List<QuestTradeNeed>(4);
            for (int cIdx = 0; cIdx < q.Checkpoints.Count; cIdx++)
            {
                var cp = q.Checkpoints[cIdx];
                if (cp.Checked || cp.LiveReady) continue;
                if (TryExtractTradeNeedFromCheckpoint(q.Title, cp, out var extracted))
                    qNeeds.Add(extracted);
            }
            if (qNeeds.Count == 0)
            {
                if (!TryGetSurfaceMiningStatusForQuest(q, out var qm))
                    return false;
                int oreCur = _boundTradeState.GetHoldCommodityQuantity(qm.TargetOreSlug);
                ready = oreCur >= 1;
                string oreTag = ready ? "[ORE HOLD READY]" : "[ORE HOLD]";
                string oreLabel = qm.TargetOreSlug.Replace('_', ' ').ToUpperInvariant();
                summary = $"{oreTag} {oreLabel}: {oreCur}/1 t";
                return true;
            }

            int primaryIdx = 0;
            for (int i = 0; i < qNeeds.Count; i++)
            {
                int curI = _boundTradeState.GetHoldCommodityQuantity(qNeeds[i].CommoditySlug);
                int tgtI = Mathf.Max(1, qNeeds[i].TargetQuantity);
                if (curI < tgtI)
                {
                    primaryIdx = i;
                    break;
                }
            }

            var need = qNeeds[primaryIdx];
            int current = _boundTradeState.GetHoldCommodityQuantity(need.CommoditySlug);
            int target = Mathf.Max(1, need.TargetQuantity);
            string unit = string.Equals(need.CommoditySlug, "propellant", StringComparison.OrdinalIgnoreCase)
                ? "t"
                : string.Equals(need.CommoditySlug, "provisions", StringComparison.OrdinalIgnoreCase)
                    ? "d"
                    : "u";
            string label = need.CommoditySlug.Replace('_', ' ').ToUpperInvariant();
            ready = current >= target;
            string holdTag = ready ? "[HOLD READY]" : "[HOLD]";
            var sb = new StringBuilder(96);
            sb.Append($"{holdTag} {label}: {current}/{target} {unit}");
            if (qNeeds.Count > 1)
            {
                bool appended = false;
                for (int i = 0; i < qNeeds.Count; i++)
                {
                    if (i == primaryIdx) continue;
                    var ex = qNeeds[i];
                    int exCur = _boundTradeState.GetHoldCommodityQuantity(ex.CommoditySlug);
                    int exTgt = Mathf.Max(1, ex.TargetQuantity);
                    string exUnit = string.Equals(ex.CommoditySlug, "propellant", StringComparison.OrdinalIgnoreCase)
                        ? "t"
                        : string.Equals(ex.CommoditySlug, "provisions", StringComparison.OrdinalIgnoreCase)
                            ? "d"
                            : "u";
                    string exLabel = ex.CommoditySlug.Replace('_', ' ').ToUpperInvariant();
                    sb.Append(appended ? ", " : " | ");
                    appended = true;
                    sb.Append(exCur >= exTgt
                        ? $"{exLabel} {exCur}/{exTgt}{exUnit} OK"
                        : $"{exLabel} {exCur}/{exTgt}{exUnit}");
                }
            }
            summary = sb.ToString();
            return true;
        }

        public bool TryGetQuestPreflightReadinessSummary(MushActiveQuest q, out string summary, out bool ready)
        {
            summary = string.Empty;
            ready = false;
            if (q == null || _boundTradeState == null || string.IsNullOrEmpty(_boundTradeState.ShipHullId))
                return false;
            if (!TryGetDestinationPortForQuest(q, out var dest) || string.IsNullOrEmpty(dest.PortLabel))
                return false;

            int fuelT = Mathf.Max(0, _boundTradeState.TankTonnes);
            int provD = Mathf.Max(0, _boundTradeState.StoresCrewDays);
            bool fuelOk = fuelT > 0;
            bool provOk = provD > 0;
            ready = fuelOk && provOk;
            if (ready)
            {
                summary = $"[PRE-FLIGHT OK] Fuel {fuelT}t | Provisions {provD}d → {dest.PortLabel}";
            }
            else
            {
                string fuelTxt = fuelOk ? $"{fuelT}t OK" : "0t!";
                string provTxt = provOk ? $"{provD}d OK" : "0d!";
                summary = $"[CHECK BEFORE UNDOCK] Fuel {fuelTxt} | Provisions {provTxt} → {dest.PortLabel}";
            }
            return true;
        }

        public static string InferStationPortFromRoomEntry(string roomEntry)
        {
            if (string.IsNullOrWhiteSpace(roomEntry)) return null;
            string low = roomEntry.Trim().ToLowerInvariant();
            if (!low.StartsWith("room.", StringComparison.Ordinal) || low.Length < 7)
                return null;
            char prefix = low[5];
            if (!char.IsDigit(low[6]))
                return null;
            switch (prefix)
            {
                case 'c': return "ceres";
                case 'p': return "stickney";
                case 'l': return "luna";
                case 'd': return "deimos";
                case 't': return "titan";
                default: return null;
            }
        }

        public bool NeedsSurfaceExpeditionBoarding()
        {
            var active = GetTrackedActiveQuest();
            if (active == null || active.ReadyToFinish || active.CanFinishNow) return false;
            return TryGetSurfaceMiningStatusForQuest(active, out var mining) && !mining.MineDone;
        }

        public bool TryGetActiveShipTargetCompartment(out string targetCompartment)
        {
            return TryGetActiveShipTargetCompartment(null, out targetCompartment, settledPad: false);
        }

        public bool TryGetActiveShipTargetCompartment(string dockedPortSlug, out string targetCompartment, bool settledPad = false)
        {
            targetCompartment = string.Empty;
            var active = GetTrackedActiveQuest();
            QuestSurfaceMiningStatus mining = default;
            bool hasMining = active != null && TryGetSurfaceMiningStatusForQuest(active, out mining);
            string dockedStation = settledPad ? null : dockedPortSlug;
            if (!string.IsNullOrWhiteSpace(dockedStation))
            {
                bool miningBlocking = hasMining && !mining.MineDone;
                if (active != null &&
                    (!miningBlocking || active.ReadyToFinish || active.CanFinishNow) &&
                    TryGetMinimapObjective(out string objRoom, out _, out _))
                {
                    string targetPort = InferStationPortFromRoomEntry(objRoom);
                    if (!string.IsNullOrEmpty(targetPort) &&
                        string.Equals(targetPort, dockedStation.Trim(), StringComparison.OrdinalIgnoreCase))
                    {
                        targetCompartment = "airlock";
                        return true;
                    }
                }
            }
            if (hasMining)
            {
                if (!mining.MineDone)
                {
                    targetCompartment = !string.IsNullOrWhiteSpace(dockedStation) ? "bridge" : "bay";
                    return true;
                }
                targetCompartment = "bridge";
                return true;
            }
            if (active != null && TryGetDestinationPortForQuest(active, out _))
            {
                targetCompartment = "bridge";
                return true;
            }
            if ((LatestPanel == null || LatestPanel.Active.Count == 0) && TryGetNextGuidedOffer(out var nextOffer))
            {
                string nextPort = InferStationPortFromRoomEntry(nextOffer.GiverRoomEntry);
                if (!string.IsNullOrWhiteSpace(dockedStation) &&
                    !string.IsNullOrEmpty(nextPort) &&
                    string.Equals(nextPort, dockedStation.Trim(), StringComparison.OrdinalIgnoreCase))
                {
                    targetCompartment = "airlock";
                    return true;
                }
                targetCompartment = "bridge";
                return true;
            }
            return false;
        }
        public string PinnedCategory { get; private set; } = string.Empty;
        public string PinnedTitle { get; private set; } = string.Empty;
        public string PinnedDetail { get; private set; } = string.Empty;

        public int GetTabLineCount(QuestJournalTab tab) => _tabLines[(int)tab].Count;
        public string GetTabSnapshot(QuestJournalTab tab) => string.Join("\n", _tabPlainLines[(int)tab]);

        public void BuildUi(Transform hudRoot, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _tabImages.Clear();

            BuildPinnedTrackerCard(hudRoot);
            BuildJournalModal(hudRoot);
            BuildNpcQuestDialogModal(hudRoot);
        }

        private void BuildPinnedTrackerCard(Transform hudRoot)
        {
            // Docked cleanly below TacticalMinimap (`-232..-16`, `-320..-72`) on right edge
            _trackerRoot = CreatePanel(
                hudRoot,
                "PinnedObjectiveTracker",
                new Vector2(1f, 1f),
                new Vector2(1f, 1f),
                new Vector2(-232f, -478f),
                new Vector2(-16f, -328f),
                new Color(0.03f, 0.06f, 0.11f, 0.92f));
            _trackerRect = _trackerRoot.GetComponent<RectTransform>();

            var hdrBar = CreatePanel(_trackerRoot.transform, "TrackerHeader", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -24f), Vector2.zero, new Color(0.16f, 0.22f, 0.28f, 0.97f));
            _trackerHeaderText = CreateText(hdrBar.transform, "OBJECTIVE [J]", 10, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.86f, 0.44f));
            SetRect(_trackerHeaderText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-82f, 0f));

            _trackerCycleBtn = CreateSmallButton(hdrBar.transform, "▸", new Color(0.24f, 0.32f, 0.44f), () => CycleTrackedQuest(), out _);
            SetRect(_trackerCycleBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-78f, 2f), new Vector2(-56f, -2f));
            _trackerCycleBtn.gameObject.SetActive(false);

            var openBtn = CreateSmallButton(hdrBar.transform, "J", new Color(0.16f, 0.38f, 0.50f), () => SetOpen(true), out _);
            SetRect(openBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-52f, 2f), new Vector2(-30f, -2f));

            var colBtn = CreateSmallButton(hdrBar.transform, "—", new Color(0.16f, 0.26f, 0.38f), ToggleTrackerCollapsed, out _trackerCollapseLbl);
            SetRect(colBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-26f, 2f), new Vector2(-4f, -2f));

            _trackerBody = CreatePanel(_trackerRoot.transform, "TrackerBody", Vector2.zero, Vector2.one, new Vector2(0f, 0f), new Vector2(0f, -24f), Color.clear);
            _trackerDetailText = CreateText(_trackerBody.transform, string.Empty, 11, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.95f, 1f));
            _trackerDetailText.supportRichText = true;
            _trackerDetailText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _trackerDetailText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_trackerDetailText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 26f), new Vector2(-8f, -6f));

            _trackerActionBtn = CreateSmallButton(_trackerBody.transform, "DIALOG", new Color(0.56f, 0.40f, 0.12f), TriggerTrackerQuickAction, out _trackerActionLbl);
            SetRect(_trackerActionBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0f, 0f), new Vector2(6f, 4f), new Vector2(102f, 22f));
            _trackerActionBtn.gameObject.SetActive(false);

            _trackerNavBtn = CreateSmallButton(_trackerBody.transform, "NAV", new Color(0.62f, 0.46f, 0.10f), () => TriggerTrackerWayfinder(), out _);
            SetRect(_trackerNavBtn.GetComponent<RectTransform>(), new Vector2(0f, 0f), new Vector2(0f, 0f), new Vector2(106f, 4f), new Vector2(148f, 22f));
            _trackerNavBtn.gameObject.SetActive(false);

            var unpinBtn = CreateSmallButton(_trackerBody.transform, "UNPIN", new Color(0.36f, 0.16f, 0.18f), ClearPinnedObjective, out _);
            SetRect(unpinBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 0f), new Vector2(-62f, 4f), new Vector2(-6f, 22f));

            _trackerRoot.SetActive(false);
        }

        private void BuildJournalModal(Transform hudRoot)
        {
            _modalRoot = CreatePanel(
                hudRoot,
                "QuestAndOpsJournalModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-465f, -305f),
                new Vector2(465f, 305f),
                new Color(0.03f, 0.06f, 0.11f, 0.985f));

            CreatePanel(_modalRoot.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(1f, 0.82f, 0.36f, 0.95f));

            var hdr = CreateText(_modalRoot.transform, "QUEST LOG, PASSAGES, SURVEYS & CLAIMS [J]", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.86f, 0.44f));
            SetRect(hdr.rectTransform, new Vector2(0f, 1f), new Vector2(0.76f, 1f), new Vector2(18f, -42f), new Vector2(0f, -8f));

            var closeBtn = CreateButton(_modalRoot.transform, "CLOSE [ESC]", new Color(0.46f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-118f, -40f), new Vector2(-14f, -10f));

            // 4 Journal Tabs
            var tabStrip = CreatePanel(_modalRoot.transform, "JournalTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -80f), new Vector2(-16f, -46f), Color.clear);
            var hlg = tabStrip.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 6f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            for (int i = 0; i < TabTitles.Length; i++)
            {
                var captured = (QuestJournalTab)i;
                var tb = CreateButton(tabStrip.transform, TabTitles[i], new Color(0.12f, 0.22f, 0.34f), () => SelectTab(captured, queryServer: true));
                _tabImages.Add(tb.GetComponent<Image>());
            }

            // Sub-tab bar for structured Quests (Active / Available Here / Completed)
            _questSubBar = CreatePanel(_modalRoot.transform, "QuestSubTabs", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(16f, -112f), new Vector2(-16f, -84f), Color.clear);
            var shlg = _questSubBar.AddComponent<HorizontalLayoutGroup>();
            shlg.spacing = 6f;
            shlg.childControlWidth = true;
            shlg.childControlHeight = true;
            shlg.childForceExpandWidth = true;

            var bAct = CreateLabeledButton(_questSubBar.transform, "ACTIVE (0)", new Color(0.48f, 0.36f, 0.14f), () => SelectQuestSubTab("active"), out _subActiveLbl);
            _subActiveImg = bAct.GetComponent<Image>();
            var bAv = CreateLabeledButton(_questSubBar.transform, "AVAILABLE HERE (0)", new Color(0.12f, 0.22f, 0.32f), () => SelectQuestSubTab("available"), out _subAvailLbl);
            _subAvailImg = bAv.GetComponent<Image>();
            var bDn = CreateLabeledButton(_questSubBar.transform, "COMPLETED (0)", new Color(0.12f, 0.22f, 0.32f), () => SelectQuestSubTab("completed"), out _subDoneLbl);
            _subDoneImg = bDn.GetComponent<Image>();

            _tabHintText = CreateText(_modalRoot.transform, "Live ledger synchronized via +quest", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.68f, 0.86f, 0.96f));
            SetRect(_tabHintText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(18f, -134f), new Vector2(-18f, -114f));

            var bodyPanel = CreatePanel(_modalRoot.transform, "JournalBody", Vector2.zero, Vector2.one, new Vector2(16f, 98f), new Vector2(-16f, -138f), new Color(0.02f, 0.05f, 0.09f, 0.94f));
            _bodyText = CreateText(bodyPanel.transform, string.Empty, 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.88f, 0.94f, 0.99f));
            _bodyText.supportRichText = true;
            _bodyText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _bodyText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_bodyText.rectTransform, Vector2.zero, Vector2.one, new Vector2(14f, 12f), new Vector2(-14f, -12f));

            // Structured Quest contextual actions row
            _questStructActions = CreatePanel(_modalRoot.transform, "QuestStructActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(16f, 56f), new Vector2(-16f, 92f), Color.clear);
            var qHlg = _questStructActions.AddComponent<HorizontalLayoutGroup>();
            qHlg.spacing = 6f;
            qHlg.childControlWidth = true;
            qHlg.childControlHeight = true;
            qHlg.childForceExpandWidth = true;

            _qNextBtn = CreateButton(_questStructActions.transform, "NEXT QUEST ▸", new Color(0.14f, 0.28f, 0.40f), CycleSelectedJournalQuest);
            _qPrimaryBtn = CreateLabeledButton(_questStructActions.transform, "ACCEPT / VERIFY", new Color(0.66f, 0.46f, 0.12f), TriggerJournalPrimaryAction, out _qPrimaryLbl);
            _qPrefillBtn = CreateLabeledButton(_questStructActions.transform, "PREFILL CMD", new Color(0.22f, 0.46f, 0.24f), () => TriggerJournalSuggestedPrefill(), out _qPrefillLbl);
            _qPrefillBtn.gameObject.SetActive(false);
            _qWayfinderBtn = CreateButton(_questStructActions.transform, "WAYFINDER ROUTE", new Color(0.12f, 0.44f, 0.56f), TriggerJournalWayfinder);
            _qAbandonBtn = CreateLabeledButton(_questStructActions.transform, "ABANDON QUEST", new Color(0.46f, 0.16f, 0.18f), TriggerJournalAbandon, out _qAbandonLbl);

            // Bottom action strip
            var actRow = CreatePanel(_modalRoot.transform, "JournalActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(16f, 14f), new Vector2(-16f, 50f), Color.clear);
            var aHlg = actRow.AddComponent<HorizontalLayoutGroup>();
            aHlg.spacing = 8f;
            aHlg.childControlWidth = true;
            aHlg.childControlHeight = true;
            aHlg.childForceExpandWidth = true;

            CreateButton(actRow.transform, "REFRESH TAB", new Color(0.14f, 0.44f, 0.56f), () => QueryTabFromServer(ActiveTab));
            CreateButton(actRow.transform, "PIN ACTIVE TO HUD", new Color(0.46f, 0.36f, 0.14f), PinActiveTabSummary);
            CreateButton(actRow.transform, "CLEAR PIN", new Color(0.36f, 0.18f, 0.20f), ClearPinnedObjective);
            CreateButton(actRow.transform, "HAUL BOARD (+haul)", new Color(0.16f, 0.38f, 0.48f), () => OnCommandRequested?.Invoke("+haul"));
            CreateButton(actRow.transform, "ESCORT (+escort)", new Color(0.18f, 0.36f, 0.52f), () => OnCommandRequested?.Invoke("+escort"));

            SelectTab(QuestJournalTab.Quests, queryServer: false);
            _modalRoot.SetActive(false);
        }

        private void BuildNpcQuestDialogModal(Transform hudRoot)
        {
            _questDialogRoot = CreatePanel(
                hudRoot,
                "NpcQuestDialogModal",
                new Vector2(0.5f, 0.5f),
                new Vector2(0.5f, 0.5f),
                new Vector2(-355f, -260f),
                new Vector2(355f, 260f),
                new Color(0.04f, 0.06f, 0.10f, 0.99f));

            CreatePanel(_questDialogRoot.transform, "GoldBar", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -4f), Vector2.zero, new Color(1f, 0.80f, 0.24f, 1f));

            var crestGo = CreatePanel(_questDialogRoot.transform, "QuestCrest", new Vector2(0f, 1f), new Vector2(0f, 1f), new Vector2(18f, -60f), new Vector2(60f, -18f), new Color(0.95f, 0.72f, 0.18f, 1f));
            _dlgCrestBg = crestGo.GetComponent<Image>();
            _dlgCrestText = CreateText(crestGo.transform, "!", 26, FontStyle.Bold, TextAnchor.MiddleCenter, new Color(0.08f, 0.05f, 0.01f, 1f));
            SetRect(_dlgCrestText.rectTransform, Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero);

            _dlgTitleText = CreateText(_questDialogRoot.transform, "QUEST DIALOG", 18, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.88f, 0.48f));
            SetRect(_dlgTitleText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(72f, -42f), new Vector2(-118f, -14f));

            _dlgSubtitleText = CreateText(_questDialogRoot.transform, string.Empty, 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.62f, 0.86f, 0.98f));
            SetRect(_dlgSubtitleText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(72f, -64f), new Vector2(-118f, -42f));

            var closeBtn = CreateButton(_questDialogRoot.transform, "CLOSE [ESC]", new Color(0.42f, 0.16f, 0.18f), CloseQuestDialog);
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-110f, -46f), new Vector2(-14f, -16f));

            var bodyPanel = CreatePanel(_questDialogRoot.transform, "DialogBody", Vector2.zero, Vector2.one, new Vector2(18f, 108f), new Vector2(-18f, -74f), new Color(0.02f, 0.04f, 0.08f, 0.95f));
            _dlgBodyText = CreateText(bodyPanel.transform, string.Empty, 13, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.90f, 0.95f, 1f));
            _dlgBodyText.supportRichText = true;
            _dlgBodyText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _dlgBodyText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_dlgBodyText.rectTransform, Vector2.zero, Vector2.one, new Vector2(14f, 12f), new Vector2(-14f, -12f));

            var rewardPanel = CreatePanel(_questDialogRoot.transform, "RewardBar", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(18f, 62f), new Vector2(-18f, 100f), new Color(0.14f, 0.11f, 0.05f, 0.95f));
            _dlgRewardText = CreateText(rewardPanel.transform, "REWARD: —", 12, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.86f, 0.44f));
            _dlgRewardText.supportRichText = true;
            SetRect(_dlgRewardText.rectTransform, Vector2.zero, Vector2.one, new Vector2(12f, 2f), new Vector2(-12f, -2f));

            var btnRow = CreatePanel(_questDialogRoot.transform, "DialogActions", new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(18f, 14f), new Vector2(-18f, 54f), Color.clear);
            var hlg = btnRow.AddComponent<HorizontalLayoutGroup>();
            hlg.spacing = 8f;
            hlg.childControlWidth = true;
            hlg.childControlHeight = true;
            hlg.childForceExpandWidth = true;

            _dlgPrimaryBtn = CreateLabeledButton(btnRow.transform, "ACCEPT QUEST", new Color(0.72f, 0.50f, 0.12f), () => TriggerPrimaryDialogAction(), out _dlgPrimaryLbl);
            _dlgWayfinderBtn = CreateButton(btnRow.transform, "WAYFINDER", new Color(0.14f, 0.44f, 0.56f), TriggerDialogWayfinder);
            _dlgCycleBtn = CreateLabeledButton(btnRow.transform, "OTHER QUEST", new Color(0.16f, 0.28f, 0.42f), CycleNpcDialogQuest, out _dlgCycleLbl);
            _dlgAbandonBtn = CreateLabeledButton(btnRow.transform, "ABANDON", new Color(0.46f, 0.16f, 0.18f), () => AbandonDialogQuest(), out _dlgAbandonLbl);
            CreateButton(btnRow.transform, "NOT NOW", new Color(0.20f, 0.24f, 0.30f), CloseQuestDialog);

            _questDialogRoot.SetActive(false);
        }

        public void ApplyQuestsPanel(MushQuestsPanel panel)
        {
            if (panel == null) return;
            LatestPanel = panel;
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;

            // Detect genuine server-side quest transitions after initial login hydration
            // so celebration callouts and chimes never spam on reconnect.
            var currentActiveKeys = new HashSet<string>(StringComparer.Ordinal);
            var currentCompletedKeys = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < panel.Active.Count; i++)
            {
                if (!string.IsNullOrEmpty(panel.Active[i].Key))
                    currentActiveKeys.Add(panel.Active[i].Key);
            }
            for (int i = 0; i < panel.Completed.Count; i++)
            {
                if (!string.IsNullOrEmpty(panel.Completed[i].Key))
                    currentCompletedKeys.Add(panel.Completed[i].Key);
            }

            List<(string kind, string title)> milestones = null;
            if (_hasHydratedPanel)
            {
                for (int i = 0; i < panel.Completed.Count; i++)
                {
                    var c = panel.Completed[i];
                    if (!string.IsNullOrEmpty(c.Key) && !_seenCompletedKeys.Contains(c.Key))
                    {
                        milestones ??= new List<(string, string)>(4);
                        milestones.Add(("completed", c.Title));
                    }
                }
                for (int i = 0; i < panel.Active.Count; i++)
                {
                    var a = panel.Active[i];
                    if (string.IsNullOrEmpty(a.Key)) continue;
                    bool wasActive = _seenActiveKeys.Contains(a.Key);
                    bool isReady = a.ReadyToFinish || a.CanFinishNow;
                    bool becameReady = isReady && !_seenReadyKeys.Contains(a.Key);
                    if (!wasActive)
                    {
                        milestones ??= new List<(string, string)>(4);
                        milestones.Add(("accepted", a.Title));
                    }
                    else if (!becameReady && a.Checkpoints != null)
                    {
                        for (int cIdx = 0; cIdx < a.Checkpoints.Count; cIdx++)
                        {
                            var cp = a.Checkpoints[cIdx];
                            if (!cp.Checked && !cp.LiveReady) continue;
                            string cpId = string.IsNullOrEmpty(cp.Id) ? cp.Title : cp.Id;
                            string cpKey = $"{a.Key}:{cpId}";
                            if (!_seenCheckpointKeys.Contains(cpKey))
                            {
                                milestones ??= new List<(string, string)>(4);
                                milestones.Add(("checkpoint", cp.Title));
                            }
                        }
                    }
                    if (becameReady)
                    {
                        milestones ??= new List<(string, string)>(4);
                        milestones.Add(("ready", a.Title));
                    }
                }
                foreach (var prev in _seenActiveMeta)
                {
                    if (!currentActiveKeys.Contains(prev.Key) && !currentCompletedKeys.Contains(prev.Key))
                    {
                        milestones ??= new List<(string, string)>(4);
                        string abandonedLabel = prev.Value.allowRestart
                            ? $"{prev.Value.title} (restartable)"
                            : prev.Value.title;
                        milestones.Add(("abandoned", abandonedLabel));
                    }
                }
            }

            _hasHydratedPanel = true;
            _seenActiveKeys.Clear();
            _seenActiveMeta.Clear();
            _seenReadyKeys.Clear();
            _seenCheckpointKeys.Clear();
            for (int i = 0; i < panel.Active.Count; i++)
            {
                var a = panel.Active[i];
                if (string.IsNullOrEmpty(a.Key)) continue;
                _seenActiveKeys.Add(a.Key);
                _seenActiveMeta[a.Key] = (a.Title ?? string.Empty, a.AllowRestart);
                if (a.ReadyToFinish || a.CanFinishNow)
                    _seenReadyKeys.Add(a.Key);
                if (a.Checkpoints != null)
                {
                    for (int cIdx = 0; cIdx < a.Checkpoints.Count; cIdx++)
                    {
                        var cp = a.Checkpoints[cIdx];
                        if (!cp.Checked && !cp.LiveReady) continue;
                        string cpId = string.IsNullOrEmpty(cp.Id) ? cp.Title : cp.Id;
                        _seenCheckpointKeys.Add($"{a.Key}:{cpId}");
                    }
                }
            }
            for (int i = 0; i < panel.Completed.Count; i++)
            {
                if (!string.IsNullOrEmpty(panel.Completed[i].Key))
                    _seenCompletedKeys.Add(panel.Completed[i].Key);
            }

            // Keep TrackedQuestKey valid or auto-track the first active quest
            if (!string.IsNullOrEmpty(TrackedQuestKey))
            {
                bool stillActive = false;
                for (int i = 0; i < panel.Active.Count; i++)
                {
                    if (panel.Active[i].Key == TrackedQuestKey)
                    {
                        stillActive = true;
                        break;
                    }
                }
                if (!stillActive) TrackedQuestKey = string.Empty;
            }
            if (string.IsNullOrEmpty(TrackedQuestKey) && panel.Active.Count > 0)
            {
                TrackedQuestKey = PickDefaultTrackedKey(panel.Active);
            }

            SyncStructuredTrackerCard();
            RefreshJournalBody();
            if (IsQuestDialogOpen)
                RefreshQuestDialogUi();

            OnQuestsPanelUpdated?.Invoke();

            if (milestones != null)
            {
                for (int i = 0; i < milestones.Count; i++)
                    OnQuestMilestoneReached?.Invoke(milestones[i].kind, milestones[i].title);
            }
        }

        private static string PickDefaultTrackedKey(List<MushActiveQuest> active)
        {
            if (active == null || active.Count == 0) return string.Empty;
            for (int i = 0; i < active.Count; i++)
            {
                var q = active[i];
                if (q == null) continue;
                if (string.Equals(q.Category, "guided", StringComparison.OrdinalIgnoreCase) ||
                    GuidedStepNumber(q.Key) > 0)
                {
                    return q.Key;
                }
            }
            return active[0]?.Key ?? string.Empty;
        }

        public MushActiveQuest GetTrackedActiveQuest()
        {
            if (LatestPanel == null) return null;
            var found = FindActiveByKey(TrackedQuestKey);
            if (found != null) return found;
            string defKey = PickDefaultTrackedKey(LatestPanel.Active);
            return FindActiveByKey(defKey) ?? (LatestPanel.Active.Count > 0 ? LatestPanel.Active[0] : null);
        }

        private static readonly (string slug, string[] patterns)[] KnownTradeCommodities =
        {
            ("industrial_parts", new[] { "industrial_parts", "industrial parts" }),
            ("carbon_ore", new[] { "carbon_ore", "carbon ore" }),
            ("structural_alloys", new[] { "structural_alloys", "structural alloys" }),
            ("rare_earths", new[] { "rare_earths", "rare earths" }),
            ("water_ice", new[] { "water_ice", "water ice" }),
            ("provisions", new[] { "provisions" }),
            ("propellant", new[] { "propellant" }),
            ("volatiles", new[] { "volatiles" }),
            ("silicates", new[] { "silicates" }),
            ("metals", new[] { "metals" }),
        };

        public List<QuestTradeNeed> GetActiveTradeNeeds()
        {
            var result = new List<QuestTradeNeed>(4);
            if (LatestPanel == null || LatestPanel.Active.Count == 0) return result;

            var ordered = new List<MushActiveQuest>(LatestPanel.Active.Count);
            var tracked = GetTrackedActiveQuest();
            if (tracked != null) ordered.Add(tracked);
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                if (LatestPanel.Active[i] != tracked)
                    ordered.Add(LatestPanel.Active[i]);
            }

            var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            for (int qIdx = 0; qIdx < ordered.Count; qIdx++)
            {
                var q = ordered[qIdx];
                for (int cIdx = 0; cIdx < q.Checkpoints.Count; cIdx++)
                {
                    var cp = q.Checkpoints[cIdx];
                    if (cp.Checked || cp.LiveReady) continue;
                    if (!TryExtractTradeNeedFromCheckpoint(q.Title, cp, out var extracted))
                        continue;
                    string key = $"{extracted.CommoditySlug}|{(extracted.WantsBuy ? 1 : 0)}|{(extracted.WantsSell ? 1 : 0)}";
                    if (seen.Add(key))
                        result.Add(extracted);
                }
                if (result.Count > 0)
                    return result;
            }
            return result;
        }

        /// <summary>
        /// Extracts the next active trade commodity need (commodity slug, target lot quantity, buy/sell side)
        /// from the tracked (or first matching) active quest so the Visual Commodity Exchange can highlight
        /// matching listings and provide a 1-click "SET QUEST QTY" button.
        /// </summary>
        public bool TryGetActiveTradeNeed(out QuestTradeNeed need)
        {
            var list = GetActiveTradeNeeds();
            if (list.Count > 0)
            {
                need = list[0];
                return true;
            }
            need = default;
            return false;
        }

        /// <summary>
        /// Contextually resolves the most relevant Exchange trade need for live ship stowage and venue listings:
        /// - Advances Step 2 past buy-only items already satisfied in live stowage and prefers an unsatisfied
        ///   item actually listed on the current venue's book.
        /// - Narrows dual buy+sell delivery runs (Steps 3 &amp; 4) to SELL-only once required cargo is loaded.
        /// </summary>
        public bool TryResolveContextualTradeNeed(TacSimTradeAndShipState trade, out QuestTradeNeed need, out List<QuestTradeNeed> allNeeds)
        {
            allNeeds = GetActiveTradeNeeds();
            need = default;
            if (allNeeds.Count == 0) return false;

            var pool = new List<QuestTradeNeed>(allNeeds.Count);
            for (int i = 0; i < allNeeds.Count; i++)
            {
                var n = allNeeds[i];
                bool buySatisfied = trade != null &&
                                    n.WantsBuy &&
                                    !n.WantsSell &&
                                    trade.GetHoldCommodityQuantity(n.CommoditySlug) >= Mathf.Max(1, n.TargetQuantity);
                if (!buySatisfied)
                    pool.Add(n);
            }
            if (pool.Count == 0)
                pool.AddRange(allNeeds);

            QuestTradeNeed picked = pool[0];
            if (trade != null && trade.Listings.Count > 0)
            {
                for (int i = 0; i < pool.Count; i++)
                {
                    bool onBook = false;
                    for (int j = 0; j < trade.Listings.Count; j++)
                    {
                        if (pool[i].MatchesListing(trade.Listings[j]))
                        {
                            onBook = true;
                            break;
                        }
                    }
                    if (onBook)
                    {
                        picked = pool[i];
                        break;
                    }
                }
            }

            if (trade != null &&
                picked.WantsBuy &&
                picked.WantsSell &&
                trade.GetHoldCommodityQuantity(picked.CommoditySlug) >= Mathf.Max(1, picked.TargetQuantity))
            {
                picked = new QuestTradeNeed(
                    picked.QuestTitle,
                    picked.CheckpointTitle,
                    picked.CommoditySlug,
                    picked.TargetQuantity,
                    wantsBuy: false,
                    wantsSell: true);
            }

            need = picked;
            return true;
        }

        private static bool TryExtractTradeNeedFromCheckpoint(string questTitle, MushQuestCheckpoint cp, out QuestTradeNeed need)
        {
            need = default;
            if (cp == null) return false;
            string idLow = (cp.Id ?? string.Empty).ToLowerInvariant();
            string titleLow = (cp.Title ?? string.Empty).ToLowerInvariant();
            string instrLow = (cp.Instructions ?? string.Empty).ToLowerInvariant();
            string primary = $"{idLow} {titleLow}";

            // Do not treat surface mining extraction checkpoints ("Mine 1t carbon_ore") as exchange trade rows
            if (idLow.StartsWith("mine_", StringComparison.Ordinal) ||
                idLow.StartsWith("scan_", StringComparison.Ordinal) ||
                idLow.StartsWith("sound_", StringComparison.Ordinal) ||
                titleLow.StartsWith("mine ", StringComparison.Ordinal))
            {
                return false;
            }

            string matchedSlug = null;
            for (int i = 0; i < KnownTradeCommodities.Length; i++)
            {
                var patterns = KnownTradeCommodities[i].patterns;
                for (int p = 0; p < patterns.Length; p++)
                {
                    if (primary.Contains(patterns[p], StringComparison.Ordinal))
                    {
                        matchedSlug = KnownTradeCommodities[i].slug;
                        break;
                    }
                }
                if (matchedSlug != null) break;
            }
            if (matchedSlug == null)
            {
                for (int i = 0; i < KnownTradeCommodities.Length; i++)
                {
                    var patterns = KnownTradeCommodities[i].patterns;
                    for (int p = 0; p < patterns.Length; p++)
                    {
                        if (instrLow.Contains(patterns[p], StringComparison.Ordinal))
                        {
                            matchedSlug = KnownTradeCommodities[i].slug;
                            break;
                        }
                    }
                    if (matchedSlug != null) break;
                }
            }
            if (string.IsNullOrEmpty(matchedSlug)) return false;

            bool wantsBuy = idLow.StartsWith("buy_", StringComparison.Ordinal) ||
                            idLow.StartsWith("deliver_", StringComparison.Ordinal) ||
                            idLow.StartsWith("return_", StringComparison.Ordinal) ||
                            titleLow.Contains("buy ", StringComparison.Ordinal) ||
                            titleLow.Contains("haul ", StringComparison.Ordinal) ||
                            instrLow.Contains("buy ", StringComparison.Ordinal);
            bool wantsSell = idLow.StartsWith("sell_", StringComparison.Ordinal) ||
                             idLow.StartsWith("deliver_", StringComparison.Ordinal) ||
                             idLow.StartsWith("return_", StringComparison.Ordinal) ||
                             titleLow.Contains("sell ", StringComparison.Ordinal) ||
                             titleLow.Contains("deliver ", StringComparison.Ordinal) ||
                             titleLow.Contains("haul ", StringComparison.Ordinal) ||
                             instrLow.Contains("+market/sell", StringComparison.Ordinal);

            if (!wantsBuy && !wantsSell) return false;

            int qty = ExtractLeadingPositiveInt(cp.Title);
            if (qty <= 0) qty = ExtractLeadingPositiveInt(cp.Instructions);
            if (qty <= 0) qty = 1;

            need = new QuestTradeNeed(questTitle, cp.Title, matchedSlug, qty, wantsBuy, wantsSell);
            return true;
        }

        private static int ExtractLeadingPositiveInt(string text)
        {
            if (string.IsNullOrEmpty(text)) return 0;
            for (int i = 0; i < text.Length; i++)
            {
                if (!char.IsDigit(text[i])) continue;
                int val = 0;
                int j = i;
                while (j < text.Length && char.IsDigit(text[j]) && val < 10000)
                {
                    val = val * 10 + (text[j] - '0');
                    j++;
                }
                if (val > 0) return val;
            }
            return 0;
        }

        /// <summary>
        /// Extracts live Scan / Sound / Mine checkpoint progress for any active surface prospecting quest
        /// so the Surface Quarry HUD and Planetary Ground Chart can render a guided mining checklist.
        /// </summary>
        public bool TryGetActiveSurfaceMiningStatus(out QuestSurfaceMiningStatus status)
        {
            status = default;
            if (LatestPanel == null || LatestPanel.Active.Count == 0) return false;

            var ordered = new List<MushActiveQuest>(LatestPanel.Active.Count);
            var tracked = GetTrackedActiveQuest();
            if (tracked != null) ordered.Add(tracked);
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                if (LatestPanel.Active[i] != tracked)
                    ordered.Add(LatestPanel.Active[i]);
            }

            for (int qIdx = 0; qIdx < ordered.Count; qIdx++)
            {
                if (TryGetSurfaceMiningStatusForQuest(ordered[qIdx], out status))
                    return true;
            }
            return false;
        }

        public static bool TryGetSurfaceMiningStatusForQuest(MushActiveQuest q, out QuestSurfaceMiningStatus status)
        {
            status = default;
            if (q == null) return false;
            bool hasScan = false, scanDone = false;
            bool hasSound = false, soundDone = false;
            bool hasMine = false, mineDone = false;
            string oreSlug = string.Empty;

            for (int cIdx = 0; cIdx < q.Checkpoints.Count; cIdx++)
            {
                var cp = q.Checkpoints[cIdx];
                string idLow = (cp.Id ?? string.Empty).ToLowerInvariant();
                string textLow = $"{idLow} {(cp.Title ?? string.Empty).ToLowerInvariant()} {(cp.Instructions ?? string.Empty).ToLowerInvariant()}";
                bool done = cp.Checked || cp.LiveReady;

                if (idLow.Contains("scan", StringComparison.Ordinal) || textLow.Contains("surface scan", StringComparison.Ordinal) || textLow.Contains("scan go", StringComparison.Ordinal))
                {
                    hasScan = true;
                    scanDone = done;
                }
                else if (idLow.Contains("sound", StringComparison.Ordinal) || textLow.Contains("sound a detected", StringComparison.Ordinal) || textLow.Contains("sound go", StringComparison.Ordinal))
                {
                    hasSound = true;
                    soundDone = done;
                }
                else if (idLow.StartsWith("mine_", StringComparison.Ordinal) || textLow.Contains("mine at least", StringComparison.Ordinal) || textLow.Contains("mine go", StringComparison.Ordinal))
                {
                    hasMine = true;
                    mineDone = done;
                    if (textLow.Contains("carbon_ore", StringComparison.Ordinal) || textLow.Contains("carbon ore", StringComparison.Ordinal))
                        oreSlug = "carbon_ore";
                    else if (textLow.Contains("rare_earths", StringComparison.Ordinal) || textLow.Contains("rare earths", StringComparison.Ordinal))
                        oreSlug = "rare_earths";
                    else if (textLow.Contains("volatiles", StringComparison.Ordinal) || textLow.Contains("water_ice", StringComparison.Ordinal))
                        oreSlug = "volatiles";
                    else if (textLow.Contains("metals", StringComparison.Ordinal))
                        oreSlug = "metals";
                }
            }

            if (hasScan || hasSound || hasMine)
            {
                if (string.IsNullOrEmpty(oreSlug)) oreSlug = "carbon_ore";
                string family = oreSlug == "carbon_ore" ? "carbon"
                              : oreSlug == "rare_earths" ? "strategic"
                              : oreSlug == "metals" ? "metals"
                              : "volatiles";
                status = new QuestSurfaceMiningStatus(
                    q.Title,
                    oreSlug,
                    family,
                    hasScan,
                    scanDone,
                    hasSound,
                    soundDone,
                    hasMine,
                    mineDone,
                    q.NextStep);
                return true;
            }
            return false;
        }

        private static readonly (string portId, string portLabel, string[] patterns)[] KnownDestinations =
        {
            ("stickney", "Stickney Yards", new[] { "+ship/plan stickney", "+ship/order depart stickney", "stickney yards", "stickney" }),
            ("ceres", "Ceres Freeport", new[] { "+ship/plan ceres", "+ship/order depart ceres", "ceres freeport", "back to ceres", "return to ceres" }),
            ("luna", "Aitken Highport", new[] { "+ship/plan luna", "+ship/order depart luna", "aitken highport" }),
            ("deimos", "Voltaire Anchorage", new[] { "+ship/plan deimos", "+ship/order depart deimos", "voltaire anchorage" }),
            ("titan", "Ligeia Terminal", new[] { "+ship/plan titan", "+ship/order depart titan", "ligeia terminal" }),
            ("isidis", "Isidis Concession", new[] { "+ship/plan isidis", "+ship/order depart isidis", "isidis concession" }),
            ("commons", "Freeport Commons", new[] { "+ship/plan commons", "freeport commons" }),
            ("ahuna", "Ahuna Flank", new[] { "+ship/plan ahuna", "ahuna flank" }),
            ("occator", "Occator Rim", new[] { "+ship/plan occator", "occator rim" }),
            ("kerwan", "Kerwan Floor", new[] { "+ship/plan kerwan", "kerwan floor" }),
            ("juling", "Juling Shadow", new[] { "+ship/plan juling", "juling shadow" }),
            ("urvara", "Urvara Reach", new[] { "+ship/plan urvara", "urvara reach" }),
            ("rheasilvia", "Rheasilvia Commons", new[] { "+ship/plan rheasilvia", "rheasilvia commons" }),
        };

        /// <summary>
        /// Extracts an active quest's corridor destination port (for Solar Orrery &amp; Sailing Plan guidance).
        /// </summary>
        public bool TryGetActiveDestinationPort(out QuestDestinationHint hint)
        {
            hint = default;
            if (LatestPanel == null || LatestPanel.Active.Count == 0) return false;

            var ordered = new List<MushActiveQuest>(LatestPanel.Active.Count);
            var tracked = GetTrackedActiveQuest();
            if (tracked != null) ordered.Add(tracked);
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                if (LatestPanel.Active[i] != tracked)
                    ordered.Add(LatestPanel.Active[i]);
            }

            for (int qIdx = 0; qIdx < ordered.Count; qIdx++)
            {
                if (TryGetDestinationPortForQuest(ordered[qIdx], out hint))
                    return true;
            }
            return false;
        }

        private static bool TryGetDestinationPortForQuest(MushActiveQuest q, out QuestDestinationHint hint)
        {
            hint = default;
            if (q == null) return false;
            string[] cmdPrefixes = { "+ship/plan ", "+ship/order depart ", "+ship/undock " };
            bool hasPendingCp = false;
            for (int cIdx = 0; cIdx < q.Checkpoints.Count; cIdx++)
            {
                var cp = q.Checkpoints[cIdx];
                if (cp.Checked || cp.LiveReady) continue;
                hasPendingCp = true;

                string combined = $"{cp.Instructions} {cp.Title} {q.NextStep} {q.Introduction}".ToLowerInvariant();
                for (int pIdx = 0; pIdx < cmdPrefixes.Length; pIdx++)
                {
                    int pos = combined.IndexOf(cmdPrefixes[pIdx], StringComparison.Ordinal);
                    if (pos < 0) continue;
                    int start = pos + cmdPrefixes[pIdx].Length;
                    int end = start;
                    while (end < combined.Length && (char.IsLetterOrDigit(combined[end]) || combined[end] == '_'))
                        end++;
                    if (end > start)
                    {
                        string word = combined.Substring(start, end - start);
                        for (int d = 0; d < KnownDestinations.Length; d++)
                        {
                            if (string.Equals(KnownDestinations[d].portId, word, StringComparison.OrdinalIgnoreCase))
                            {
                                hint = new QuestDestinationHint(q.Title, KnownDestinations[d].portId, KnownDestinations[d].portLabel);
                                return true;
                            }
                        }
                    }
                }

                for (int d = 0; d < KnownDestinations.Length; d++)
                {
                    var pats = KnownDestinations[d].patterns;
                    for (int k = 0; k < pats.Length; k++)
                    {
                        if (combined.Contains(pats[k], StringComparison.Ordinal))
                        {
                            hint = new QuestDestinationHint(q.Title, KnownDestinations[d].portId, KnownDestinations[d].portLabel);
                            return true;
                        }
                    }
                }
            }

            // Once all objectives are satisfied (e.g. post-mine Step 6 on a surface landing pad),
            // fall back to the turn-in room's station port so the Space HUD, Solar Orrery, and
            // Sailing Plan guide the pilot home unless they are already at the turn-in room.
            if (!q.CanFinishNow && !q.TurnInHere && (q.ReadyToFinish || (!hasPendingCp && q.Checkpoints.Count > 0)))
            {
                string roomEntry = q.NextRoomEntry;
                if (string.IsNullOrEmpty(roomEntry))
                {
                    for (int cIdx = q.Checkpoints.Count - 1; cIdx >= 0; cIdx--)
                    {
                        if (!string.IsNullOrEmpty(q.Checkpoints[cIdx].RoomEntry))
                        {
                            roomEntry = q.Checkpoints[cIdx].RoomEntry;
                            break;
                        }
                    }
                }
                string turnInPort = InferStationPortFromRoomEntry(roomEntry);
                if (!string.IsNullOrEmpty(turnInPort))
                {
                    for (int d = 0; d < KnownDestinations.Length; d++)
                    {
                        if (string.Equals(KnownDestinations[d].portId, turnInPort, StringComparison.OrdinalIgnoreCase))
                        {
                            hint = new QuestDestinationHint(q.Title, KnownDestinations[d].portId, KnownDestinations[d].portLabel);
                            return true;
                        }
                    }
                }
            }
            return false;
        }

        /// <summary>
        /// Resolves a corridor destination hint for the between-steps Next Guided Offer when the
        /// pilot is viewing the Solar System Map from a different station/body than the next giver's port.
        /// </summary>
        public bool TryGetNextGuidedOfferDestination(string currentPortOrBody, out QuestDestinationHint hint)
        {
            hint = default;
            if (!TryGetNextGuidedOffer(out var next))
                return false;
            string portSlug = InferStationPortFromRoomEntry(next.GiverRoomEntry);
            if (string.IsNullOrEmpty(portSlug))
                return false;
            for (int d = 0; d < KnownDestinations.Length; d++)
            {
                if (!string.Equals(KnownDestinations[d].portId, portSlug, StringComparison.OrdinalIgnoreCase))
                    continue;
                string pid = KnownDestinations[d].portId;
                string plabel = KnownDestinations[d].portLabel;
                if (!string.IsNullOrWhiteSpace(currentPortOrBody))
                {
                    string curLow = currentPortOrBody.Trim().ToLowerInvariant();
                    string lblLow = plabel.ToLowerInvariant();
                    if (curLow == pid || curLow.Contains(pid, StringComparison.Ordinal) || curLow.Contains(lblLow, StringComparison.Ordinal))
                        return false;
                }
                hint = new QuestDestinationHint(
                    $"Next Guided ({next.Step}/7): {next.ShortLabel} — {next.GiverLabel}",
                    pid,
                    plabel);
                return true;
            }
            return false;
        }

        public bool TryGetMinimapObjective(out string roomEntry, out bool isTurnIn, out bool currentRoomHasOffer)
        {
            roomEntry = string.Empty;
            isTurnIn = false;
            currentRoomHasOffer = false;
            if (LatestPanel == null) return false;

            currentRoomHasOffer = LatestPanel.Available.Count > 0;
            var active = GetTrackedActiveQuest();
            if (active != null)
            {
                isTurnIn = active.ReadyToFinish || active.CanFinishNow;
                roomEntry = active.NextRoomEntry ?? string.Empty;
                if (string.IsNullOrEmpty(roomEntry))
                {
                    for (int i = 0; i < active.Checkpoints.Count; i++)
                    {
                        if (!active.Checkpoints[i].Checked && !string.IsNullOrEmpty(active.Checkpoints[i].RoomEntry))
                        {
                            roomEntry = active.Checkpoints[i].RoomEntry;
                            break;
                        }
                    }
                }
                return !string.IsNullOrEmpty(roomEntry) || currentRoomHasOffer;
            }
            if (!currentRoomHasOffer && TryGetNextGuidedOffer(out var nextOffer) && !string.IsNullOrEmpty(nextOffer.GiverRoomEntry))
            {
                roomEntry = nextOffer.GiverRoomEntry;
                isTurnIn = false;
                return true;
            }
            return currentRoomHasOffer;
        }

        public bool HasQuestForNpc(string npcId)
        {
            if (LatestPanel == null || string.IsNullOrEmpty(npcId)) return false;
            for (int i = 0; i < LatestPanel.RoomMarkers.Count; i++)
            {
                if (LatestPanel.RoomMarkers[i].NpcId == npcId) return true;
            }
            for (int i = 0; i < LatestPanel.Available.Count; i++)
            {
                if (LatestPanel.Available[i].GiverId == npcId) return true;
            }
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                if (LatestPanel.Active[i].TurnInId == npcId || LatestPanel.Active[i].GiverId == npcId)
                    return true;
            }
            return false;
        }

        public bool OpenNpcQuestDialog(string npcId)
        {
            if (LatestPanel == null || string.IsNullOrEmpty(npcId)) return false;
            var orderedKeys = CollectNpcDialogKeys(npcId);
            string pickKey = orderedKeys.Count > 0 ? orderedKeys[0] : string.Empty;
            if (string.IsNullOrEmpty(pickKey))
            {
                int bestRank = int.MaxValue;
                for (int i = 0; i < LatestPanel.RoomMarkers.Count; i++)
                {
                    var m = LatestPanel.RoomMarkers[i];
                    if (m.NpcId != npcId || string.IsNullOrEmpty(m.QuestKey)) continue;
                    int rank = m.Marker == "turn_in" ? 0 : (m.Marker == "available" ? 1 : 2);
                    if (rank < bestRank)
                    {
                        bestRank = rank;
                        pickKey = m.QuestKey;
                    }
                }
            }
            if (string.IsNullOrEmpty(pickKey)) return false;

            DialogNpcId = npcId;
            DialogQuestKey = pickKey;
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;
            if (_modalRoot != null) _modalRoot.SetActive(false);
            if (_questDialogRoot != null) _questDialogRoot.SetActive(true);
            RefreshQuestDialogUi();
            return true;
        }

        public bool OpenQuestDialogByKey(string questKey)
        {
            if (LatestPanel == null || string.IsNullOrEmpty(questKey)) return false;
            DialogNpcId = string.Empty;
            DialogQuestKey = questKey;
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;
            if (_modalRoot != null) _modalRoot.SetActive(false);
            if (_questDialogRoot != null) _questDialogRoot.SetActive(true);
            RefreshQuestDialogUi();
            return true;
        }

        public void CloseQuestDialog()
        {
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;
            if (_questDialogRoot != null)
                _questDialogRoot.SetActive(false);
        }

        public void SelectQuestSubTab(string subTab)
        {
            ActiveQuestSubTab = subTab ?? "active";
            _selectedQuestIndex = ResolveDefaultJournalIndex(ActiveQuestSubTab);
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;
            RefreshJournalBody();
        }

        private int ResolveDefaultJournalIndex(string subTab)
        {
            if (LatestPanel == null) return 0;
            if (string.Equals(subTab, "active", StringComparison.OrdinalIgnoreCase) && LatestPanel.Active.Count > 0)
            {
                for (int i = 0; i < LatestPanel.Active.Count; i++)
                    if (LatestPanel.Active[i].CanFinishNow) return i;
                for (int i = 0; i < LatestPanel.Active.Count; i++)
                    if (LatestPanel.Active[i].CanCheckNow || LatestPanel.Active[i].ReadyToFinish) return i;
                if (!string.IsNullOrEmpty(TrackedQuestKey))
                {
                    for (int i = 0; i < LatestPanel.Active.Count; i++)
                        if (string.Equals(LatestPanel.Active[i].Key, TrackedQuestKey, StringComparison.Ordinal)) return i;
                }
                return 0;
            }
            if (string.Equals(subTab, "available", StringComparison.OrdinalIgnoreCase) && LatestPanel.Available.Count > 0)
            {
                for (int i = 0; i < LatestPanel.Available.Count; i++)
                {
                    var av = LatestPanel.Available[i];
                    if (av != null &&
                        (string.Equals(av.Category, "guided", StringComparison.OrdinalIgnoreCase) ||
                         GuidedStepNumber(av.Key) > 0))
                    {
                        return i;
                    }
                }
            }
            return 0;
        }

        public void TrackQuest(string questKey)
        {
            TrackedQuestKey = questKey ?? string.Empty;
            _manualPinActive = false;
            SyncStructuredTrackerCard();
            OnQuestsPanelUpdated?.Invoke();
        }

        public string CycleTrackedQuest()
        {
            if (LatestPanel == null || LatestPanel.Active.Count == 0)
            {
                TrackedQuestKey = string.Empty;
                return string.Empty;
            }
            if (LatestPanel.Active.Count == 1)
            {
                TrackedQuestKey = LatestPanel.Active[0].Key;
                SyncStructuredTrackerCard();
                return TrackedQuestKey;
            }
            int curIdx = -1;
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                if (LatestPanel.Active[i].Key == TrackedQuestKey)
                {
                    curIdx = i;
                    break;
                }
            }
            int nextIdx = curIdx >= 0 ? (curIdx + 1) % LatestPanel.Active.Count : 1;
            TrackedQuestKey = LatestPanel.Active[nextIdx].Key;
            _manualPinActive = false;
            SyncStructuredTrackerCard();
            OnQuestsPanelUpdated?.Invoke();
            return TrackedQuestKey;
        }

        public void SetOpen(bool open, QuestJournalTab? tabOverride = null)
        {
            if (_modalRoot == null) return;
            if (open) CloseQuestDialog();
            _modalRoot.SetActive(open);
            if (open)
            {
                if (tabOverride.HasValue)
                    SelectTab(tabOverride.Value, queryServer: true);
                else
                    QueryTabFromServer(ActiveTab);
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void ResetSessionState()
        {
            for (int i = 0; i < _tabLines.Length; i++)
            {
                _tabLines[i].Clear();
                _tabPlainLines[i].Clear();
            }
            LatestPanel = null;
            TrackedQuestKey = string.Empty;
            DialogNpcId = string.Empty;
            DialogQuestKey = string.Empty;
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;
            _trackerRouteRoomName = string.Empty;
            _trackerRouteCommand = string.Empty;
            _trackerRouteHops = 0;
            _hasHydratedPanel = false;
            _seenActiveKeys.Clear();
            _seenActiveMeta.Clear();
            _seenReadyKeys.Clear();
            _seenCheckpointKeys.Clear();
            _seenCompletedKeys.Clear();
            _manualPinActive = false;
            _capturingTab = null;
            _captureExpiresAt = 0f;
            ClearPinnedObjective();
            CloseQuestDialog();
            SetOpen(false);
            OnQuestsPanelUpdated?.Invoke();
        }

        public void SelectTab(QuestJournalTab tab, bool queryServer = true)
        {
            ActiveTab = tab;
            int idx = (int)tab;
            for (int i = 0; i < _tabImages.Count; i++)
            {
                _tabImages[i].color = i == idx
                    ? new Color(0.52f, 0.40f, 0.16f, 1f)
                    : new Color(0.10f, 0.20f, 0.30f, 0.94f);
            }
            if (queryServer)
                QueryTabFromServer(tab);
            RefreshJournalBody();
        }

        public void QueryTabFromServer(QuestJournalTab tab)
        {
            BeginCapture(tab, clearExisting: true);
            OnCommandRequested?.Invoke(TabCommands[(int)tab]);
        }

        public void BeginCapture(QuestJournalTab tab, bool clearExisting = true, float windowSec = 4.5f)
        {
            _capturingTab = tab;
            _captureExpiresAt = Time.unscaledTime + windowSec;
            if (clearExisting)
            {
                _tabLines[(int)tab].Clear();
                _tabPlainLines[(int)tab].Clear();
            }
        }

        public bool IngestServerOutput(string plain, string rich)
        {
            if (!_capturingTab.HasValue || string.IsNullOrWhiteSpace(plain))
                return false;
            if (Time.unscaledTime > _captureExpiresAt)
            {
                _capturingTab = null;
                return false;
            }
            string trimmed = plain.Trim();
            if (trimmed.StartsWith("> ", StringComparison.Ordinal) ||
                trimmed.StartsWith("[NET]", StringComparison.Ordinal) ||
                trimmed.StartsWith("[PACK SYNC]", StringComparison.Ordinal))
                return false;

            int idx = (int)_capturingTab.Value;
            string richClean = !string.IsNullOrWhiteSpace(rich) ? rich.TrimEnd('\r') : MushProtocol.EscapeUnityRichText(trimmed);
            _tabPlainLines[idx].Add(trimmed);
            _tabLines[idx].Add(richClean);
            while (_tabPlainLines[idx].Count > MaxLinesPerTab) _tabPlainLines[idx].RemoveAt(0);
            while (_tabLines[idx].Count > MaxLinesPerTab) _tabLines[idx].RemoveAt(0);

            if (IsOpen && ActiveTab == _capturingTab.Value)
                RefreshJournalBody();
            return true;
        }

        public void PinActiveTabSummary()
        {
            int idx = (int)ActiveTab;
            string cat = TabTitles[idx];
            string title;
            string detail;
            if (_tabPlainLines[idx].Count > 0)
            {
                title = _tabPlainLines[idx][0];
                int extra = Mathf.Min(3, _tabPlainLines[idx].Count - 1);
                if (extra > 0)
                {
                    var parts = new List<string>(extra);
                    for (int i = 1; i <= extra; i++)
                        parts.Add(_tabPlainLines[idx][i]);
                    detail = string.Join("\n", parts);
                }
                else
                {
                    detail = $"Run {TabCommands[idx]} to check live progress.";
                }
            }
            else if (ActiveTab == QuestJournalTab.Quests && LatestPanel != null && LatestPanel.Active.Count > 0)
            {
                var q = LatestPanel.Active[0];
                title = q.Title;
                detail = q.NextStep;
            }
            else
            {
                title = $"Check {TabTitles[idx]}";
                detail = $"Open [J] or run {TabCommands[idx]} for live objectives.";
            }
            PinObjective(cat, title, detail);
        }

        public void PinObjective(string category, string title, string detail)
        {
            _manualPinActive = true;
            PinnedCategory = (category ?? "OBJECTIVE").Trim().ToUpperInvariant();
            PinnedTitle = (title ?? string.Empty).Trim();
            PinnedDetail = (detail ?? string.Empty).Trim();
            if (_trackerHeaderText != null)
                _trackerHeaderText.text = string.IsNullOrEmpty(PinnedCategory) ? "OBJECTIVE [J]" : PinnedCategory;
            if (_trackerDetailText != null)
            {
                _trackerDetailText.text = $"<b><color=#66EEFF>{MushProtocol.EscapeUnityRichText(PinnedTitle)}</color></b>\n{MushProtocol.EscapeUnityRichText(PinnedDetail)}";
            }
            if (_trackerActionBtn != null) _trackerActionBtn.gameObject.SetActive(false);
            if (_trackerNavBtn != null) _trackerNavBtn.gameObject.SetActive(false);
        }

        public void ClearPinnedObjective()
        {
            _manualPinActive = false;
            TrackedQuestKey = string.Empty;
            PinnedCategory = string.Empty;
            PinnedTitle = string.Empty;
            PinnedDetail = string.Empty;
            if (_trackerNavBtn != null) _trackerNavBtn.gameObject.SetActive(false);
            if (_trackerRoot != null)
                _trackerRoot.SetActive(false);
        }

        public void ToggleTrackerCollapsed()
        {
            _trackerCollapsed = !_trackerCollapsed;
            if (_trackerBody != null) _trackerBody.SetActive(!_trackerCollapsed);
            if (_trackerCollapseLbl != null) _trackerCollapseLbl.text = _trackerCollapsed ? "+" : "—";
            if (_trackerRect != null)
                _trackerRect.offsetMin = new Vector2(-232f, _trackerCollapsed ? -352f : -478f);
        }

        public void SyncTrackerVisibility(TacSimActiveStage stage, bool rightCornerBusy)
        {
            if (_trackerRoot == null) return;
            bool show = HasPinnedObjective &&
                        !rightCornerBusy &&
                        stage != TacSimActiveStage.SimulatorArena;
            if (_trackerRoot.activeSelf != show)
                _trackerRoot.SetActive(show);
        }

        public bool TriggerPrimaryDialogAction()
        {
            if (LatestPanel == null) return false;
            var active = FindActiveByKey(DialogQuestKey);
            if (active != null)
            {
                if (active.CanFinishNow && !string.IsNullOrEmpty(active.FinishCmd))
                {
                    OnCommandRequested?.Invoke(active.FinishCmd);
                    CloseQuestDialog();
                    return true;
                }
                if (active.CanCheckNow && !string.IsNullOrEmpty(active.CheckCmd))
                {
                    OnCommandRequested?.Invoke(active.CheckCmd);
                    return true;
                }
                TrackQuest(active.Key);
                CloseQuestDialog();
                return true;
            }
            var offer = FindAvailableByKey(DialogQuestKey);
            if (offer != null && !string.IsNullOrEmpty(offer.AcceptCmd))
            {
                TrackedQuestKey = offer.Key;
                _manualPinActive = false;
                OnCommandRequested?.Invoke(offer.AcceptCmd);
                CloseQuestDialog();
                return true;
            }
            return false;
        }

        public bool AbandonDialogQuest()
        {
            var active = FindActiveByKey(DialogQuestKey);
            if (active != null && !string.IsNullOrEmpty(active.AbandonCmd))
            {
                if (_confirmAbandonKey != active.Key)
                {
                    _confirmAbandonKey = active.Key;
                    _confirmDeclineKey = string.Empty;
                    RefreshQuestDialogUi();
                    return false;
                }
                _confirmAbandonKey = string.Empty;
                OnCommandRequested?.Invoke(active.AbandonCmd);
                CloseQuestDialog();
                return true;
            }
            var offer = FindAvailableByKey(DialogQuestKey);
            if (offer != null && !offer.AllowRestart && !string.IsNullOrEmpty(offer.DeclineCmd))
            {
                if (_confirmDeclineKey != offer.Key)
                {
                    _confirmDeclineKey = offer.Key;
                    _confirmAbandonKey = string.Empty;
                    RefreshQuestDialogUi();
                    return false;
                }
                _confirmDeclineKey = string.Empty;
                OnCommandRequested?.Invoke(offer.DeclineCmd);
                CloseQuestDialog();
                return true;
            }
            return false;
        }

        public string DialogCycleLabelSnapshot => _dlgCycleLbl != null ? _dlgCycleLbl.text : string.Empty;
        public string DialogAbandonLabelSnapshot => _dlgAbandonLbl != null ? _dlgAbandonLbl.text : string.Empty;
        public bool IsDialogAbandonVisible => _dlgAbandonBtn != null && _dlgAbandonBtn.gameObject.activeSelf;

        private string _trackerRouteRoomName = string.Empty;
        private string _trackerRouteCommand = string.Empty;
        private int _trackerRouteHops;

        public string TrackerDetailSnapshot => _trackerDetailText != null ? _trackerDetailText.text : string.Empty;
        public string TrackerActionLabelSnapshot => _trackerActionLbl != null ? _trackerActionLbl.text : string.Empty;
        public string TrackerRouteRoomName => _trackerRouteRoomName;
        public string TrackerRouteCommand => _trackerRouteCommand;
        public int TrackerRouteHops => _trackerRouteHops;

        public void SetTrackerRouteBreadcrumb(string nextRoomName, string command, int hops)
        {
            string normName = hops > 0 && !string.IsNullOrWhiteSpace(nextRoomName) && !string.IsNullOrWhiteSpace(command)
                ? nextRoomName.Trim()
                : string.Empty;
            string normCmd = !string.IsNullOrEmpty(normName) ? command.Trim() : string.Empty;
            int normHops = !string.IsNullOrEmpty(normName) ? hops : 0;
            if (string.Equals(_trackerRouteRoomName, normName, StringComparison.Ordinal) &&
                string.Equals(_trackerRouteCommand, normCmd, StringComparison.Ordinal) &&
                _trackerRouteHops == normHops)
                return;

            _trackerRouteRoomName = normName;
            _trackerRouteCommand = normCmd;
            _trackerRouteHops = normHops;
            SyncStructuredTrackerCard();
        }

        private static string FormatTrackerRouteText(string roomName, string command, int hops)
        {
            if (string.Equals(command, "+ship/board", StringComparison.OrdinalIgnoreCase))
                return "+ship/board (Board Ship)";
            if (string.Equals(command, "+ship/leave", StringComparison.OrdinalIgnoreCase))
                return "+ship/leave (Cycle Ashore)";
            if (string.Equals(command, "+tv/bay in", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(command, "+tv/bay", StringComparison.OrdinalIgnoreCase))
                return $"{command} (Vehicle Bay)";
            return $"-> {roomName} ({hops}h)";
        }

        private static string FormatTrackerRouteButtonLabel(string command, int hops)
        {
            if (string.Equals(command, "+ship/board", StringComparison.OrdinalIgnoreCase))
                return "BOARD SHIP";
            if (string.Equals(command, "+ship/leave", StringComparison.OrdinalIgnoreCase))
                return "ASHORE";
            if (string.Equals(command, "+tv/bay in", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(command, "+tv/bay", StringComparison.OrdinalIgnoreCase))
                return "VEHICLE BAY";
            return $"WALK ({hops}h)";
        }

        private void SyncStructuredTrackerCard()
        {
            if (_manualPinActive || LatestPanel == null) return;
            bool hasTour = TryGetGuidedTourProgress(out int tourDone, out int tourTotal, out _, out bool tourAllDone);
            var active = FindActiveByKey(TrackedQuestKey) ?? (LatestPanel.Active.Count > 0 ? LatestPanel.Active[0] : null);
            if (active != null)
            {
                int done = 0;
                var sb = new StringBuilder(192);
                for (int i = 0; i < active.Checkpoints.Count; i++)
                {
                    var cp = active.Checkpoints[i];
                    if (cp.Checked) done++;
                    string sym = cp.Checked ? "<color=#55EE99>[✓]</color>" : (cp.LiveReady ? "<color=#FFDD55>[◆]</color>" : "<color=#99BBCC>[ ]</color>");
                    sb.Append(sym).Append(' ').Append(MushProtocol.EscapeUnityRichText(cp.Title)).Append('\n');
                }
                if (TryGetQuestHoldReadinessSummary(active, out string holdSummary, out bool holdReady))
                {
                    string holdHex = holdReady ? "#55EE99" : "#FFDD44";
                    sb.Append("<color=").Append(holdHex).Append('>')
                      .Append(MushProtocol.EscapeUnityRichText(holdSummary))
                      .Append("</color>\n");
                }
                if (TryGetQuestPreflightReadinessSummary(active, out string pfSummary, out bool pfReady))
                {
                    string pfHex = pfReady ? "#55EE99" : "#FFD166";
                    sb.Append("<color=").Append(pfHex).Append('>')
                      .Append(MushProtocol.EscapeUnityRichText(pfSummary))
                      .Append("</color>\n");
                }
                if (!active.CanFinishNow && !active.ReadyToFinish &&
                    TryGetSurfaceMiningStatusForQuest(active, out var trkMining))
                {
                    sb.Append("<color=#77DDFF>")
                      .Append(MushProtocol.EscapeUnityRichText(trkMining.FormatPrepHintLine()))
                      .Append("</color>\n");
                }
                if (!active.CanFinishNow && !active.CanCheckNow && !string.IsNullOrEmpty(_trackerRouteRoomName) && _trackerRouteHops > 0)
                {
                    sb.Append("<color=#FFDD44>Route:</color> ")
                      .Append(MushProtocol.EscapeUnityRichText(FormatTrackerRouteText(_trackerRouteRoomName, _trackerRouteCommand, _trackerRouteHops)))
                      .Append('\n');
                }
                if (!string.IsNullOrEmpty(active.NextStep))
                    sb.Append("<color=#77DDFF>Next:</color> ").Append(MushProtocol.EscapeUnityRichText(active.NextStep));
                int otherReadyCount = 0;
                MushActiveQuest firstOtherReady = null;
                for (int i = 0; i < LatestPanel.Active.Count; i++)
                {
                    var otherQ = LatestPanel.Active[i];
                    if (otherQ != null &&
                        !string.Equals(otherQ.Key, active.Key, StringComparison.Ordinal) &&
                        otherQ.CanFinishNow)
                    {
                        if (firstOtherReady == null) firstOtherReady = otherQ;
                        otherReadyCount++;
                    }
                }
                if (otherReadyCount > 0)
                {
                    sb.Append("\n<color=#55EE99><b>[?] ");
                    if (otherReadyCount == 1)
                        sb.Append("Ready here: ").Append(MushProtocol.EscapeUnityRichText(firstOtherReady.Title));
                    else
                        sb.Append(otherReadyCount).Append(" ready to turn in here");
                    sb.Append("</b></color>");
                }
                if (LatestPanel.Available.Count > 0)
                {
                    int availCount = LatestPanel.Available.Count;
                    sb.Append("\n<color=#FFDD44><b>[!] ").Append(availCount)
                      .Append(availCount == 1 ? " offer here" : " offers here")
                      .Append("</b></color>");
                }

                string multiPrefix = string.Empty;
                if (LatestPanel.Active.Count > 1)
                {
                    int activeIdx = 0;
                    for (int i = 0; i < LatestPanel.Active.Count; i++)
                    {
                        if (LatestPanel.Active[i].Key == active.Key) { activeIdx = i; break; }
                    }
                    multiPrefix = $"[{activeIdx + 1}/{LatestPanel.Active.Count}] ";
                }
                if (_trackerCycleBtn != null)
                    _trackerCycleBtn.gameObject.SetActive(LatestPanel.Active.Count > 1);

                string tourSuffix = hasTour ? $" · TOUR {tourDone}/{tourTotal}" : string.Empty;
                int activeStepNo = GuidedStepNumber(active.Key);
                string activeStepSuffix = activeStepNo > 0 ? $" [Step {activeStepNo}/7]" : string.Empty;
                PinnedCategory = $"QUEST ({done}/{active.Checkpoints.Count}){tourSuffix} [J]";
                PinnedTitle = active.Title;
                PinnedDetail = active.NextStep;
                if (_trackerHeaderText != null) _trackerHeaderText.text = PinnedCategory;
                if (_trackerDetailText != null)
                {
                    _trackerDetailText.text = $"<b><color=#FFDF77>{multiPrefix}{MushProtocol.EscapeUnityRichText(active.Title)}{activeStepSuffix}</color></b>\n{sb}";
                }
                if (_trackerActionBtn != null)
                {
                    _trackerActionBtn.gameObject.SetActive(true);
                    if (_trackerActionLbl != null)
                    {
                        if (active.CanFinishNow) _trackerActionLbl.text = "COMPLETE [?]";
                        else if (active.CanCheckNow) _trackerActionLbl.text = "VERIFY STEP";
                        else if (!string.IsNullOrEmpty(_trackerRouteCommand))
                            _trackerActionLbl.text = FormatTrackerRouteButtonLabel(_trackerRouteCommand, _trackerRouteHops);
                        else if (!string.IsNullOrEmpty(active.NextRoomEntry)) _trackerActionLbl.text = "WAYFINDER";
                        else _trackerActionLbl.text = "OPEN DIALOG";
                    }
                }
                if (_trackerNavBtn != null)
                {
                    _trackerNavBtn.gameObject.SetActive(!string.IsNullOrEmpty(ResolveObjectiveRoomEntry(active)));
                }
                return;
            }

            if (_trackerCycleBtn != null) _trackerCycleBtn.gameObject.SetActive(false);

            var topOffer = GetPrioritizedAvailableOffer(LatestPanel);
            if (topOffer != null)
            {
                string tourSuffix = hasTour ? $" · TOUR {tourDone}/{tourTotal}" : string.Empty;
                int extraOffers = Mathf.Max(0, LatestPanel.Available.Count - 1);
                string extraSuffix = extraOffers > 0 ? $" (+{extraOffers} more)" : string.Empty;
                int offerStepNo = GuidedStepNumber(topOffer.Key);
                string offerStepSuffix = offerStepNo > 0 ? $" [Step {offerStepNo}/7]" : string.Empty;
                bool isGuided = string.Equals(topOffer.Category, "guided", StringComparison.OrdinalIgnoreCase) || offerStepNo > 0;
                PinnedCategory = $"QUEST AVAILABLE [!]{tourSuffix}";
                PinnedTitle = topOffer.Title;
                PinnedDetail = $"Speak with {topOffer.GiverName}";
                if (_trackerHeaderText != null) _trackerHeaderText.text = PinnedCategory;
                if (_trackerDetailText != null)
                {
                    string kindWord = isGuided ? "guided" : "optional";
                    _trackerDetailText.text = $"<b><color=#FFDD44>[!] {MushProtocol.EscapeUnityRichText(topOffer.Title)}{extraSuffix}{offerStepSuffix}</color></b>\n{MushProtocol.EscapeUnityRichText(topOffer.GiverName)} has a {kindWord} quest.";
                }
                if (_trackerActionBtn != null)
                {
                    _trackerActionBtn.gameObject.SetActive(true);
                    if (_trackerActionLbl != null)
                        _trackerActionLbl.text = !string.IsNullOrEmpty(topOffer.AcceptCmd) ? "ACCEPT [!]" : "OPEN DIALOG";
                }
                if (_trackerNavBtn != null) _trackerNavBtn.gameObject.SetActive(false);
                return;
            }

            if (TryGetNextGuidedOffer(out var nextOffer))
            {
                string tourSuffix = hasTour ? $" · TOUR {tourDone}/{tourTotal}" : string.Empty;
                string curPort = _boundTradeState != null
                    ? (!string.IsNullOrEmpty(_boundTradeState.ActiveVenue)
                        ? _boundTradeState.ActiveVenue
                        : _boundTradeState.AboardDockedPortHint)
                    : string.Empty;
                QuestDestinationHint sailDest = default;
                bool hasSailDest = !string.IsNullOrEmpty(curPort) &&
                    TryGetNextGuidedOfferDestination(curPort, out sailDest);
                PinnedCategory = $"NEXT GUIDED ({nextOffer.Step}/7) [!]{tourSuffix}";
                PinnedTitle = nextOffer.ShortLabel;
                PinnedDetail = $"Speak with {nextOffer.GiverLabel} ({nextOffer.GiverRoomName})";
                if (_trackerHeaderText != null) _trackerHeaderText.text = PinnedCategory;
                if (_trackerDetailText != null)
                {
                    var sb = new StringBuilder(160);
                    sb.Append($"<b><color=#FFDD44>[!] {MushProtocol.EscapeUnityRichText(nextOffer.ShortLabel)}</color></b>\n");
                    sb.Append($"<color=#77DDFF>Speak:</color> {MushProtocol.EscapeUnityRichText(nextOffer.GiverLabel)} ({MushProtocol.EscapeUnityRichText(nextOffer.GiverRoomName)})");
                    if (!string.IsNullOrEmpty(_trackerRouteRoomName) && _trackerRouteHops > 0)
                    {
                        sb.Append($"\n<color=#FFDD44>Route:</color> {MushProtocol.EscapeUnityRichText(FormatTrackerRouteText(_trackerRouteRoomName, _trackerRouteCommand, _trackerRouteHops))}");
                    }
                    if (hasSailDest)
                    {
                        sb.Append($"\n<color=#77DDFF>Sail:</color> +ship/plan {MushProtocol.EscapeUnityRichText(sailDest.PortId)} ({MushProtocol.EscapeUnityRichText(sailDest.PortLabel)})");
                    }
                    _trackerDetailText.text = sb.ToString();
                }
                if (_trackerActionBtn != null)
                {
                    _trackerActionBtn.gameObject.SetActive(true);
                    if (_trackerActionLbl != null)
                    {
                        if (!string.IsNullOrEmpty(_trackerRouteCommand))
                            _trackerActionLbl.text = FormatTrackerRouteButtonLabel(_trackerRouteCommand, _trackerRouteHops);
                        else if (hasSailDest)
                            _trackerActionLbl.text = $"SAIL {sailDest.PortId.ToUpperInvariant()}";
                        else
                            _trackerActionLbl.text = "WAYFINDER";
                    }
                }
                if (_trackerNavBtn != null) _trackerNavBtn.gameObject.SetActive(!string.IsNullOrEmpty(nextOffer.GiverRoomEntry));
                return;
            }

            if (hasTour && tourAllDone)
            {
                PinnedCategory = $"CORRIDOR CAPTAIN ({tourDone}/{tourTotal}) [J]";
                PinnedTitle = "Guided Tour Complete";
                PinnedDetail = $"{tourDone}/{tourTotal} Guided Quests Completed";
                if (_trackerHeaderText != null) _trackerHeaderText.text = PinnedCategory;
                if (_trackerDetailText != null)
                {
                    _trackerDetailText.text = $"<b><color=#55EE99>★ CORRIDOR CAPTAIN</color></b>\nGuided Tour Complete ({tourDone}/{tourTotal}).\n<color=#77DDFF>Jobs Board:</color> +market/jobs";
                }
                if (_trackerActionBtn != null)
                {
                    _trackerActionBtn.gameObject.SetActive(true);
                    if (_trackerActionLbl != null) _trackerActionLbl.text = "LOG [J]";
                }
                if (_trackerNavBtn != null) _trackerNavBtn.gameObject.SetActive(false);
                return;
            }

            PinnedCategory = string.Empty;
            PinnedTitle = string.Empty;
            PinnedDetail = string.Empty;
            if (_trackerActionBtn != null) _trackerActionBtn.gameObject.SetActive(false);
            if (_trackerNavBtn != null) _trackerNavBtn.gameObject.SetActive(false);
        }

        private static string ResolveObjectiveRoomEntry(MushActiveQuest active)
        {
            if (active == null) return string.Empty;
            if (!string.IsNullOrEmpty(active.NextRoomEntry))
                return active.NextRoomEntry;
            for (int i = 0; i < active.Checkpoints.Count; i++)
            {
                if (!active.Checkpoints[i].Checked && !string.IsNullOrEmpty(active.Checkpoints[i].RoomEntry))
                    return active.Checkpoints[i].RoomEntry;
            }
            return string.Empty;
        }

        public bool TriggerTrackerWayfinder()
        {
            var active = GetTrackedActiveQuest();
            string roomEntry = ResolveObjectiveRoomEntry(active);
            if (string.IsNullOrEmpty(roomEntry) && TryGetNextGuidedOffer(out var nextOffer))
                roomEntry = nextOffer.GiverRoomEntry;
            if (string.IsNullOrEmpty(roomEntry)) return false;
            OnWayfinderRoomRequested?.Invoke(roomEntry);
            return true;
        }

        public void TriggerTrackerQuickAction()
        {
            if (LatestPanel == null) return;
            var active = FindActiveByKey(TrackedQuestKey) ?? (LatestPanel.Active.Count > 0 ? LatestPanel.Active[0] : null);
            if (active != null)
            {
                if (active.CanFinishNow && !string.IsNullOrEmpty(active.FinishCmd))
                {
                    OnCommandRequested?.Invoke(active.FinishCmd);
                    return;
                }
                if (active.CanCheckNow && !string.IsNullOrEmpty(active.CheckCmd))
                {
                    OnCommandRequested?.Invoke(active.CheckCmd);
                    return;
                }
                if (!string.IsNullOrEmpty(_trackerRouteCommand))
                {
                    OnCommandRequested?.Invoke(_trackerRouteCommand);
                    return;
                }
                if (!string.IsNullOrEmpty(active.NextRoomEntry))
                {
                    OnWayfinderRoomRequested?.Invoke(active.NextRoomEntry);
                    return;
                }
                OpenQuestDialogByKey(active.Key);
                return;
            }
            var topOffer = GetPrioritizedAvailableOffer(LatestPanel);
            if (topOffer != null)
            {
                if (!string.IsNullOrEmpty(topOffer.AcceptCmd))
                {
                    TrackedQuestKey = topOffer.Key;
                    _manualPinActive = false;
                    OnCommandRequested?.Invoke(topOffer.AcceptCmd);
                    return;
                }
                OpenNpcQuestDialog(topOffer.GiverId);
                return;
            }
            if (TryGetNextGuidedOffer(out var nextOffer))
            {
                if (!string.IsNullOrEmpty(_trackerRouteCommand))
                {
                    OnCommandRequested?.Invoke(_trackerRouteCommand);
                    return;
                }
                string curPort = _boundTradeState != null
                    ? (!string.IsNullOrEmpty(_boundTradeState.ActiveVenue)
                        ? _boundTradeState.ActiveVenue
                        : _boundTradeState.AboardDockedPortHint)
                    : string.Empty;
                if (!string.IsNullOrEmpty(curPort) &&
                    TryGetNextGuidedOfferDestination(curPort, out var sailDest))
                {
                    OnCommandRequested?.Invoke($"+ship/plan {sailDest.PortId}");
                    return;
                }
                if (!string.IsNullOrEmpty(nextOffer.GiverRoomEntry))
                {
                    OnWayfinderRoomRequested?.Invoke(nextOffer.GiverRoomEntry);
                    return;
                }
            }
            if (TryGetGuidedTourProgress(out _, out _, out _, out bool allDone) && allDone)
            {
                ActiveQuestSubTab = "completed";
                SelectTab(QuestJournalTab.Quests);
                SetOpen(true);
            }
        }

        public void TriggerDialogWayfinder()
        {
            var active = FindActiveByKey(DialogQuestKey);
            if (active != null && !string.IsNullOrEmpty(active.NextRoomEntry))
            {
                OnWayfinderRoomRequested?.Invoke(active.NextRoomEntry);
                CloseQuestDialog();
                return;
            }
            var offer = FindAvailableByKey(DialogQuestKey);
            if (offer != null && offer.Checkpoints.Count > 0 && !string.IsNullOrEmpty(offer.Checkpoints[0].RoomEntry))
            {
                OnWayfinderRoomRequested?.Invoke(offer.Checkpoints[0].RoomEntry);
                CloseQuestDialog();
            }
        }

        public void CycleNpcDialogQuest()
        {
            var keys = CollectNpcDialogKeys(DialogNpcId);
            if (keys.Count <= 1) return;
            int idx = keys.IndexOf(DialogQuestKey);
            DialogQuestKey = keys[(idx + 1) % keys.Count];
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;
            RefreshQuestDialogUi();
        }

        private List<string> CollectNpcDialogKeys(string npcId)
        {
            var keys = new List<string>();
            if (LatestPanel == null || string.IsNullOrEmpty(npcId)) return keys;
            // 1. Ready turn-in / verifiable active quests first (`?` gold)
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                var a = LatestPanel.Active[i];
                if ((a.TurnInId == npcId || a.GiverId == npcId) &&
                    (a.CanFinishNow || a.ReadyToFinish || a.CanCheckNow) &&
                    !keys.Contains(a.Key))
                {
                    keys.Add(a.Key);
                }
            }
            // 2. Available quest offers next (`!` gold), guided tour offers before optional offers
            for (int i = 0; i < LatestPanel.Available.Count; i++)
            {
                var av = LatestPanel.Available[i];
                if (av.GiverId == npcId &&
                    (string.Equals(av.Category, "guided", StringComparison.OrdinalIgnoreCase) || GuidedStepNumber(av.Key) > 0) &&
                    !keys.Contains(av.Key))
                {
                    keys.Add(av.Key);
                }
            }
            for (int i = 0; i < LatestPanel.Available.Count; i++)
            {
                var av = LatestPanel.Available[i];
                if (av.GiverId == npcId && !keys.Contains(av.Key))
                    keys.Add(av.Key);
            }
            // 3. Remaining in-progress active quests (`?` silver)
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                var a = LatestPanel.Active[i];
                if ((a.TurnInId == npcId || a.GiverId == npcId) && !keys.Contains(a.Key))
                    keys.Add(a.Key);
            }
            return keys;
        }

        private static readonly (string key, string shortLabel, string giverLabel, string giverRoomEntry, string giverRoomName, string locationLabel)[] GuidedTourRoadmapMeta =
        {
            ("tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", "Co-op Charter", "Maren Halvorsen", "room.c03_arrivals", "Port Watch Arrivals", "Ceres Arrivals -> Registry"),
            ("tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", "Locker & Tanks", "Ilse Oduya-Brandt", "room.c05_registry", "Registry & Co-op Desk", "Ceres Chandlery & Exchange"),
            ("tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery", "Metals Haul", "Ilse Oduya-Brandt", "room.c05_registry", "Registry & Co-op Desk", "Ceres -> Stickney Receiving"),
            ("tacsim.quests.guided_onboarding/quest.guide_04_return_haul", "Return Load", "Jonas Achterberg-Nkemelu", "room.p07_receiving", "Receiving Counter", "Stickney -> Ceres Chandlery"),
            ("tacsim.quests.guided_onboarding/quest.guide_05_refit_prospector", "Dray-P Refit", "Ilse Oduya-Brandt", "room.c05_registry", "Registry & Co-op Desk", "Ceres Registry -> Exchange"),
            ("tacsim.quests.guided_onboarding/quest.guide_06_surface_mining", "Surface Mining", "Rosalind Qureshi-Mbeki", "room.c07_exchange", "Exchange Hall", "Ceres Surface Landing Site"),
            ("tacsim.quests.guided_onboarding/quest.guide_07_sell_mined_ore", "Sell Mined Ore", "Rosalind Qureshi-Mbeki", "room.c07_exchange", "Exchange Hall", "Ceres -> Stickney Service")
        };

        private static readonly string[] GuidedTourKeys =
        {
            GuidedTourRoadmapMeta[0].key,
            GuidedTourRoadmapMeta[1].key,
            GuidedTourRoadmapMeta[2].key,
            GuidedTourRoadmapMeta[3].key,
            GuidedTourRoadmapMeta[4].key,
            GuidedTourRoadmapMeta[5].key,
            GuidedTourRoadmapMeta[6].key
        };

        public bool TryGetGuidedTourRoadmap(List<GuidedTourRoadmapStep> dest)
        {
            if (dest == null) return false;
            dest.Clear();
            if (!TryGetGuidedTourProgress(out _, out _, out _, out _))
                return false;

            var doneSet = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < LatestPanel.Completed.Count; i++)
                doneSet.Add(LatestPanel.Completed[i].Key);

            var activeMap = new Dictionary<string, MushActiveQuest>(StringComparer.Ordinal);
            for (int i = 0; i < LatestPanel.Active.Count; i++)
                activeMap[LatestPanel.Active[i].Key] = LatestPanel.Active[i];

            var availSet = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < LatestPanel.Available.Count; i++)
                availSet.Add(LatestPanel.Available[i].Key);

            for (int i = 0; i < GuidedTourRoadmapMeta.Length; i++)
            {
                var m = GuidedTourRoadmapMeta[i];
                activeMap.TryGetValue(m.key, out var activeMatch);
                GuidedTourStepStatus status = doneSet.Contains(m.key)
                    ? GuidedTourStepStatus.Completed
                    : activeMatch != null
                        ? GuidedTourStepStatus.Active
                        : availSet.Contains(m.key)
                            ? GuidedTourStepStatus.Available
                            : GuidedTourStepStatus.Upcoming;
                bool ready = activeMatch != null && (activeMatch.CanFinishNow || activeMatch.ReadyToFinish);
                dest.Add(new GuidedTourRoadmapStep(i + 1, m.key, m.shortLabel, m.giverLabel, m.giverRoomEntry, m.giverRoomName, m.locationLabel, status, ready));
            }
            return true;
        }

        public bool TryGetNextGuidedOffer(out GuidedTourRoadmapStep nextOffer)
        {
            nextOffer = default;
            if (LatestPanel == null || LatestPanel.Active.Count > 0 || LatestPanel.Available.Count > 0)
                return false;
            var steps = new List<GuidedTourRoadmapStep>(GuidedTourRoadmapMeta.Length);
            if (!TryGetGuidedTourRoadmap(steps))
                return false;
            int doneCount = 0;
            for (int i = 0; i < steps.Count; i++)
                if (steps[i].Status == GuidedTourStepStatus.Completed) doneCount++;
            if (doneCount == 0 || doneCount >= steps.Count)
                return false;
            for (int i = 0; i < steps.Count; i++)
            {
                if (steps[i].Status == GuidedTourStepStatus.Upcoming)
                {
                    nextOffer = steps[i];
                    return true;
                }
            }
            return false;
        }

        public static int GuidedStepNumber(string key)
        {
            if (string.IsNullOrEmpty(key)) return 0;
            for (int i = 0; i < GuidedTourKeys.Length; i++)
            {
                if (string.Equals(GuidedTourKeys[i], key, StringComparison.Ordinal))
                    return i + 1;
            }
            return 0;
        }

        public bool TryGetGuidedTourProgress(out int completed, out int total, out int currentStep, out bool allDone)
        {
            completed = 0;
            total = GuidedTourKeys.Length;
            currentStep = 0;
            allDone = false;
            if (LatestPanel == null) return false;

            var doneSet = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < LatestPanel.Completed.Count; i++)
            {
                if (GuidedStepNumber(LatestPanel.Completed[i].Key) > 0)
                    doneSet.Add(LatestPanel.Completed[i].Key);
            }
            int activeStep = 0;
            for (int i = 0; i < LatestPanel.Active.Count; i++)
            {
                int st = GuidedStepNumber(LatestPanel.Active[i].Key);
                if (st > 0) { activeStep = st; break; }
            }
            int availStep = 0;
            for (int i = 0; i < LatestPanel.Available.Count; i++)
            {
                int st = GuidedStepNumber(LatestPanel.Available[i].Key);
                if (st > 0) { availStep = st; break; }
            }
            if (doneSet.Count == 0 && activeStep == 0 && availStep == 0)
                return false;

            completed = doneSet.Count;
            currentStep = activeStep > 0 ? activeStep : availStep;
            allDone = completed >= total;
            return true;
        }

        public bool TryGetSuggestedCommand(MushActiveQuest active, out string label, out string command)
        {
            label = string.Empty;
            command = string.Empty;
            if (active == null || active.CanFinishNow || active.CanCheckNow || active.ReadyToFinish)
                return false;

            MushQuestCheckpoint pending = null;
            for (int i = 0; i < active.Checkpoints.Count; i++)
            {
                if (!active.Checkpoints[i].Checked && !active.Checkpoints[i].LiveReady)
                {
                    pending = active.Checkpoints[i];
                    break;
                }
            }
            string idLow = (pending?.Id ?? string.Empty).ToLowerInvariant();
            string textLow = $"{idLow} {pending?.Title ?? string.Empty} {pending?.Instructions ?? string.Empty} {active.NextStep ?? string.Empty}".ToLowerInvariant();

            if (textLow.Contains("+coop/refit", StringComparison.Ordinal) || idLow.Contains("refit_dray", StringComparison.Ordinal))
            {
                label = "Refit Dray-P";
                command = "+coop/refit prospector";
                return true;
            }
            if (textLow.Contains("+coop/enroll", StringComparison.Ordinal) || idLow.Contains("enroll_coop", StringComparison.Ordinal))
            {
                label = "Enroll Co-op";
                command = "+coop/enroll ";
                return true;
            }
            if (idLow.Contains("scan", StringComparison.Ordinal) || textLow.Contains("scan go", StringComparison.Ordinal))
            {
                label = "Scan Site";
                command = "+tv/1 scan go";
                return true;
            }
            if (idLow.Contains("sound", StringComparison.Ordinal) || textLow.Contains("sound go", StringComparison.Ordinal))
            {
                label = "Sound Pocket";
                command = "+tv/1 sound go";
                return true;
            }
            if (idLow.StartsWith("mine_", StringComparison.Ordinal) || textLow.Contains("mine go", StringComparison.Ordinal))
            {
                label = "Mine Ore";
                command = "+tv/1 mine go";
                return true;
            }
            if (TryGetActiveSurfaceMiningStatus(out var miningStatus) && miningStatus.MineDone)
            {
                label = "Recover Rig";
                command = "+tv/1 recover go";
                return true;
            }
            if (TryGetActiveDestinationPort(out var dest))
            {
                label = $"Plan {dest.PortLabel}";
                command = $"+ship/plan {dest.PortId}";
                return true;
            }
            if (textLow.Contains("+market/", StringComparison.Ordinal))
            {
                label = "Market Board";
                command = "+market/list";
                return true;
            }
            return false;
        }

        public bool TriggerJournalSuggestedPrefill()
        {
            if (LatestPanel == null) return false;
            if (LatestPanel.Active.Count == 0)
            {
                if (ActiveQuestSubTab == "active" && TryGetNextGuidedOffer(out _))
                {
                    string curPort = _boundTradeState != null
                        ? (!string.IsNullOrEmpty(_boundTradeState.ActiveVenue)
                            ? _boundTradeState.ActiveVenue
                            : _boundTradeState.AboardDockedPortHint)
                        : string.Empty;
                    if (!string.IsNullOrEmpty(curPort) &&
                        TryGetNextGuidedOfferDestination(curPort, out var sailDest))
                    {
                        SetOpen(false);
                        OnPrefillCommandRequested?.Invoke($"+ship/plan {sailDest.PortId}");
                        return true;
                    }
                }
                if (ActiveQuestSubTab == "active" &&
                    TryGetGuidedTourProgress(out _, out _, out _, out bool allTourDone) &&
                    allTourDone)
                {
                    SetOpen(false);
                    OnPrefillCommandRequested?.Invoke("+market/jobs");
                    return true;
                }
                return false;
            }
            var active = LatestPanel.Active[_selectedQuestIndex % LatestPanel.Active.Count];
            if (!TryGetSuggestedCommand(active, out _, out string cmd))
                return false;
            SetOpen(false);
            OnPrefillCommandRequested?.Invoke(cmd);
            return true;
        }

        private void RefreshQuestDialogUi()
        {
            if (LatestPanel == null) return;
            var active = FindActiveByKey(DialogQuestKey);
            var offer = active == null ? FindAvailableByKey(DialogQuestKey) : null;
            var npcKeys = CollectNpcDialogKeys(DialogNpcId);

            if (_dlgCycleBtn != null)
            {
                _dlgCycleBtn.gameObject.SetActive(npcKeys.Count > 1);
                if (_dlgCycleLbl != null)
                {
                    int pos = Mathf.Max(0, npcKeys.IndexOf(DialogQuestKey)) + 1;
                    _dlgCycleLbl.text = $"QUEST {pos}/{npcKeys.Count}";
                }
            }

            if (offer != null)
            {
                if (_dlgCrestText != null) _dlgCrestText.text = "!";
                if (_dlgCrestBg != null) _dlgCrestBg.color = new Color(0.96f, 0.74f, 0.18f, 1f);
                if (_dlgTitleText != null) _dlgTitleText.text = offer.Title.ToUpperInvariant();
                if (_dlgSubtitleText != null)
                {
                    string turnIn = offer.TurnInId != offer.GiverId ? $"  ·  Turn in to: {offer.TurnInName}" : string.Empty;
                    _dlgSubtitleText.text = $"Offered by {offer.GiverName}{turnIn}";
                }
                if (_dlgBodyText != null)
                {
                    var sb = new StringBuilder(512);
                    int offerStep = GuidedStepNumber(offer.Key);
                    if (offerStep > 0)
                        sb.Append($"<b><color=#66EEAA>[Step {offerStep}/7]</color></b>\n");
                    sb.Append(MushProtocol.EscapeUnityRichText(offer.Introduction)).Append("\n\n");
                    sb.Append("<b><color=#FFDF77>REQUIREMENTS &amp; STEPS</color></b>\n");
                    for (int i = 0; i < offer.Checkpoints.Count; i++)
                    {
                        var cp = offer.Checkpoints[i];
                        sb.Append("• <b>").Append(MushProtocol.EscapeUnityRichText(cp.Title)).Append("</b>");
                        if (!string.IsNullOrEmpty(cp.RoomName))
                            sb.Append(" <color=#77DDFF>(").Append(MushProtocol.EscapeUnityRichText(cp.RoomName)).Append(")</color>");
                        sb.Append('\n').Append("<color=#99B8CC>  ").Append(MushProtocol.EscapeUnityRichText(cp.Instructions)).Append("</color>\n");
                    }
                    if (offer.AllowRestart)
                        sb.Append("\n<color=#88AAC8>Optional · Restartable (abandon and restart any time without penalty)</color>");
                    _dlgBodyText.text = sb.ToString();
                }
                if (_dlgRewardText != null)
                    _dlgRewardText.text = $"<b>REWARD:</b> {MushProtocol.EscapeUnityRichText(offer.RewardSummary)}";
                if (_dlgPrimaryBtn != null) _dlgPrimaryBtn.gameObject.SetActive(true);
                if (_dlgPrimaryLbl != null) _dlgPrimaryLbl.text = "ACCEPT QUEST";
                if (_dlgWayfinderBtn != null) _dlgWayfinderBtn.gameObject.SetActive(offer.Checkpoints.Count > 0 && !string.IsNullOrEmpty(offer.Checkpoints[0].RoomEntry));
                bool canDecline = !offer.AllowRestart && !string.IsNullOrEmpty(offer.DeclineCmd);
                if (_dlgAbandonBtn != null) _dlgAbandonBtn.gameObject.SetActive(canDecline);
                if (canDecline && _dlgAbandonLbl != null)
                    _dlgAbandonLbl.text = _confirmDeclineKey == offer.Key ? "CONFIRM DECLINE" : "DECLINE";
                return;
            }

            if (active != null)
            {
                bool goldenQ = active.CanFinishNow || active.ReadyToFinish || active.CanCheckNow;
                if (_dlgCrestText != null) _dlgCrestText.text = "?";
                if (_dlgCrestBg != null) _dlgCrestBg.color = goldenQ ? new Color(0.98f, 0.78f, 0.18f, 1f) : new Color(0.45f, 0.52f, 0.62f, 1f);
                if (_dlgTitleText != null) _dlgTitleText.text = active.Title.ToUpperInvariant();
                if (_dlgSubtitleText != null)
                {
                    string turnIn = active.TurnInId != active.GiverId ? $"  ·  Turn in to: {active.TurnInName}" : string.Empty;
                    _dlgSubtitleText.text = $"Giver: {active.GiverName}{turnIn}";
                }
                if (_dlgBodyText != null)
                {
                    var sb = new StringBuilder(512);
                    int activeStep = GuidedStepNumber(active.Key);
                    if (activeStep > 0)
                        sb.Append($"<b><color=#66EEAA>[Step {activeStep}/7]</color></b>");
                    if (active.CanFinishNow)
                    {
                        if (activeStep > 0) sb.Append("  ");
                        sb.Append("<color=#55EE99><b>[?] READY TO COMPLETE HERE</b></color>");
                    }
                    if (activeStep > 0 || active.CanFinishNow) sb.Append('\n');
                    sb.Append(MushProtocol.EscapeUnityRichText(active.Introduction)).Append("\n\n");
                    sb.Append("<b><color=#FFDF77>OBJECTIVES</color></b>\n");
                    for (int i = 0; i < active.Checkpoints.Count; i++)
                    {
                        var cp = active.Checkpoints[i];
                        string mark = cp.Checked ? "<color=#55EE99>[✓]</color>" : (cp.LiveReady ? "<color=#FFDD55>[◆]</color>" : "<color=#99BBCC>[ ]</color>");
                        sb.Append(mark).Append(" <b>").Append(MushProtocol.EscapeUnityRichText(cp.Title)).Append("</b>");
                        if (!string.IsNullOrEmpty(cp.RoomName))
                            sb.Append(" <color=#77DDFF>(").Append(MushProtocol.EscapeUnityRichText(cp.RoomName)).Append(")</color>");
                        sb.Append('\n');
                        if (!string.IsNullOrEmpty(cp.DetailText))
                            sb.Append("<color=#66EEAA>  ").Append(MushProtocol.EscapeUnityRichText(cp.DetailText)).Append("</color>\n");
                        else
                            sb.Append("<color=#99B8CC>  ").Append(MushProtocol.EscapeUnityRichText(cp.Instructions)).Append("</color>\n");
                    }
                    if (TryGetQuestHoldReadinessSummary(active, out string holdSummary, out bool holdReady))
                    {
                        string holdCol = holdReady ? "#55EE99" : "#FFDD44";
                        sb.Append($"\n<color={holdCol}><b>{MushProtocol.EscapeUnityRichText(holdSummary)}</b></color>\n");
                    }
                    if (TryGetQuestPreflightReadinessSummary(active, out string pfSummary, out bool pfReady))
                    {
                        string pfCol = pfReady ? "#55EE99" : "#FFD166";
                        sb.Append($"<color={pfCol}><b>{MushProtocol.EscapeUnityRichText(pfSummary)}</b></color>\n");
                    }
                    if (TryGetSuggestedCommand(active, out string sugLbl, out string sugCmd))
                    {
                        sb.Append($"\n<color=#FFDD44><b>SUGGESTED CMD:</b></color> <color=#66EEAA>{MushProtocol.EscapeUnityRichText(sugCmd.Trim())}</color> ({MushProtocol.EscapeUnityRichText(sugLbl)})\n");
                    }
                    if (TryGetActiveSurfaceMiningStatus(out var dlgMining))
                    {
                        sb.Append($"<color=#FFDD44><b>PROSPECTING:</b></color> <color=#77DDFF>{MushProtocol.EscapeUnityRichText(dlgMining.FormatPrepHintLine())}</color>\n");
                    }
                    if (active.AllowRestart)
                        sb.Append("\n<color=#88AAC8>Optional · Restartable (abandon and restart any time without penalty)</color>");
                    _dlgBodyText.text = sb.ToString();
                }
                if (_dlgRewardText != null)
                    _dlgRewardText.text = $"<b>REWARD:</b> {MushProtocol.EscapeUnityRichText(active.RewardSummary)}";
                if (_dlgPrimaryBtn != null) _dlgPrimaryBtn.gameObject.SetActive(true);
                if (_dlgPrimaryLbl != null)
                {
                    if (active.CanFinishNow) _dlgPrimaryLbl.text = "COMPLETE QUEST";
                    else if (active.CanCheckNow) _dlgPrimaryLbl.text = "VERIFY OBJECTIVE";
                    else _dlgPrimaryLbl.text = "TRACK ON HUD";
                }
                if (_dlgWayfinderBtn != null) _dlgWayfinderBtn.gameObject.SetActive(!string.IsNullOrEmpty(active.NextRoomEntry));
                if (_dlgAbandonBtn != null) _dlgAbandonBtn.gameObject.SetActive(true);
                if (_dlgAbandonLbl != null)
                    _dlgAbandonLbl.text = _confirmAbandonKey == active.Key ? "CONFIRM ABANDON" : "ABANDON QUEST";
            }
        }

        private void CycleSelectedJournalQuest()
        {
            if (LatestPanel == null) return;
            int count = ActiveQuestSubTab == "available"
                ? LatestPanel.Available.Count
                : ActiveQuestSubTab == "completed"
                    ? LatestPanel.Completed.Count
                    : LatestPanel.Active.Count;
            if (count <= 1) return;
            _selectedQuestIndex = (_selectedQuestIndex + 1) % count;
            _confirmAbandonKey = string.Empty;
            _confirmDeclineKey = string.Empty;
            RefreshJournalBody();
        }

        private void TriggerJournalPrimaryAction()
        {
            if (LatestPanel == null) return;
            if (ActiveQuestSubTab == "available" && LatestPanel.Available.Count > 0)
            {
                var offer = LatestPanel.Available[_selectedQuestIndex % LatestPanel.Available.Count];
                if (!string.IsNullOrEmpty(offer.AcceptCmd))
                {
                    TrackedQuestKey = offer.Key;
                    _manualPinActive = false;
                    OnCommandRequested?.Invoke(offer.AcceptCmd);
                }
                return;
            }
            if (ActiveQuestSubTab == "active" && LatestPanel.Active.Count > 0)
            {
                var active = LatestPanel.Active[_selectedQuestIndex % LatestPanel.Active.Count];
                if (active.CanFinishNow && !string.IsNullOrEmpty(active.FinishCmd))
                    OnCommandRequested?.Invoke(active.FinishCmd);
                else if (active.CanCheckNow && !string.IsNullOrEmpty(active.CheckCmd))
                    OnCommandRequested?.Invoke(active.CheckCmd);
                else
                    TrackQuest(active.Key);
            }
            else if (ActiveQuestSubTab == "active" && LatestPanel.Active.Count == 0 && LatestPanel.Available.Count > 0)
            {
                SelectQuestSubTab("available");
            }
            else if (ActiveQuestSubTab == "active" &&
                     LatestPanel.Active.Count == 0 &&
                     TryGetGuidedTourProgress(out _, out _, out _, out bool allTourDone) &&
                     allTourDone)
            {
                SelectQuestSubTab("completed");
            }
        }

        public void TriggerJournalPrimaryActionForTests() => TriggerJournalPrimaryAction();

        public void TriggerJournalWayfinder()
        {
            if (LatestPanel == null) return;
            if (ActiveQuestSubTab == "active" && LatestPanel.Active.Count > 0)
            {
                var active = LatestPanel.Active[_selectedQuestIndex % LatestPanel.Active.Count];
                if (!string.IsNullOrEmpty(active.NextRoomEntry))
                {
                    OnWayfinderRoomRequested?.Invoke(active.NextRoomEntry);
                    SetOpen(false);
                }
            }
            else if (ActiveQuestSubTab == "active" && LatestPanel.Active.Count == 0 &&
                TryGetNextGuidedOffer(out var nextOffer) && !string.IsNullOrEmpty(nextOffer.GiverRoomEntry))
            {
                OnWayfinderRoomRequested?.Invoke(nextOffer.GiverRoomEntry);
                SetOpen(false);
            }
            else if (ActiveQuestSubTab == "available" && LatestPanel.Available.Count > 0)
            {
                var offer = LatestPanel.Available[_selectedQuestIndex % LatestPanel.Available.Count];
                if (offer.Checkpoints.Count > 0 && !string.IsNullOrEmpty(offer.Checkpoints[0].RoomEntry))
                {
                    OnWayfinderRoomRequested?.Invoke(offer.Checkpoints[0].RoomEntry);
                    SetOpen(false);
                }
            }
        }

        private void TriggerJournalAbandon()
        {
            if (LatestPanel == null) return;
            if (ActiveQuestSubTab == "active" && LatestPanel.Active.Count > 0)
            {
                var active = LatestPanel.Active[_selectedQuestIndex % LatestPanel.Active.Count];
                if (string.IsNullOrEmpty(active.AbandonCmd)) return;
                if (_confirmAbandonKey != active.Key)
                {
                    _confirmAbandonKey = active.Key;
                    _confirmDeclineKey = string.Empty;
                    RefreshJournalBody();
                    return;
                }
                _confirmAbandonKey = string.Empty;
                OnCommandRequested?.Invoke(active.AbandonCmd);
                return;
            }
            if (ActiveQuestSubTab == "available" && LatestPanel.Available.Count > 0)
            {
                var offer = LatestPanel.Available[_selectedQuestIndex % LatestPanel.Available.Count];
                if (offer.AllowRestart || string.IsNullOrEmpty(offer.DeclineCmd)) return;
                if (_confirmDeclineKey != offer.Key)
                {
                    _confirmDeclineKey = offer.Key;
                    _confirmAbandonKey = string.Empty;
                    RefreshJournalBody();
                    return;
                }
                _confirmDeclineKey = string.Empty;
                OnCommandRequested?.Invoke(offer.DeclineCmd);
            }
        }

        private MushActiveQuest FindActiveByKey(string key)
        {
            if (LatestPanel == null || string.IsNullOrEmpty(key)) return null;
            for (int i = 0; i < LatestPanel.Active.Count; i++)
                if (LatestPanel.Active[i].Key == key) return LatestPanel.Active[i];
            return null;
        }

        private MushAvailableQuest FindAvailableByKey(string key)
        {
            if (LatestPanel == null || string.IsNullOrEmpty(key)) return null;
            for (int i = 0; i < LatestPanel.Available.Count; i++)
                if (LatestPanel.Available[i].Key == key) return LatestPanel.Available[i];
            return null;
        }

        private void RefreshJournalBody()
        {
            int idx = (int)ActiveTab;
            bool structuredQuests = ActiveTab == QuestJournalTab.Quests && LatestPanel != null;
            if (_questSubBar != null) _questSubBar.SetActive(structuredQuests);
            if (_questStructActions != null) _questStructActions.SetActive(structuredQuests);

            if (structuredQuests)
            {
                if (_subActiveLbl != null) _subActiveLbl.text = $"ACTIVE ({LatestPanel.Active.Count})";
                if (_subAvailLbl != null) _subAvailLbl.text = $"AVAILABLE HERE ({LatestPanel.Available.Count})";
                if (_subDoneLbl != null) _subDoneLbl.text = $"COMPLETED ({LatestPanel.Completed.Count})";
                if (_subActiveImg != null) _subActiveImg.color = ActiveQuestSubTab == "active" ? new Color(0.56f, 0.42f, 0.14f, 1f) : new Color(0.10f, 0.20f, 0.30f, 0.94f);
                if (_subAvailImg != null) _subAvailImg.color = ActiveQuestSubTab == "available" ? new Color(0.56f, 0.42f, 0.14f, 1f) : new Color(0.10f, 0.20f, 0.30f, 0.94f);
                if (_subDoneImg != null) _subDoneImg.color = ActiveQuestSubTab == "completed" ? new Color(0.14f, 0.46f, 0.32f, 1f) : new Color(0.10f, 0.20f, 0.30f, 0.94f);

                if (_tabHintText != null)
                {
                    string tourSnap = GuidedTourProgressSnapshot;
                    string tourPrefix = !string.IsNullOrEmpty(tourSnap) ? $"GUIDED {tourSnap}  ·  " : string.Empty;
                    _tabHintText.text = $"{tourPrefix}Room: {LatestPanel.RoomName}  ·  {LatestPanel.Active.Count} active / {LatestPanel.Completed.Count} completed";
                }

                var sb = new StringBuilder(512);
                var roadmapSteps = new List<GuidedTourRoadmapStep>(GuidedTourRoadmapMeta.Length);
                if (TryGetGuidedTourRoadmap(roadmapSteps))
                {
                    sb.Append("<b><color=#FFDF77>GUIDED CORRIDOR TOUR:</color></b> ");
                    for (int i = 0; i < roadmapSteps.Count; i++)
                    {
                        if (i > 0) sb.Append("  ");
                        var s = roadmapSteps[i];
                        string color;
                        string badge;
                        if (s.Status == GuidedTourStepStatus.Completed)
                        {
                            color = "#55EE99";
                            badge = "[✓]";
                        }
                        else if (s.Status == GuidedTourStepStatus.Active)
                        {
                            color = s.ReadyToTurnIn ? "#55EE99" : "#FFDF77";
                            badge = s.ReadyToTurnIn ? "[?]" : "[▸]";
                        }
                        else if (s.Status == GuidedTourStepStatus.Available)
                        {
                            color = "#FFEE55";
                            badge = "[!]";
                        }
                        else
                        {
                            color = "#6E859C";
                            badge = "[○]";
                        }
                        sb.Append("<color=").Append(color).Append('>').Append(badge).Append(' ')
                          .Append(s.Step).Append(". ").Append(MushProtocol.EscapeUnityRichText(s.ShortLabel))
                          .Append("</color>");
                    }
                    sb.Append("\n\n");
                }
                if (ActiveQuestSubTab == "active")
                {
                    if (LatestPanel.Active.Count == 0)
                    {
                        bool showPrefill = false;
                        bool showWayfinder = false;
                        bool tourMastered = false;
                        if (TryGetNextGuidedOffer(out var nextOffer))
                        {
                            sb.Append($"<color=#88AAC8>No active quests. <b><color=#FFDD44>Next Guided Step ({nextOffer.Step}/7): {MushProtocol.EscapeUnityRichText(nextOffer.ShortLabel)}</color></b> — speak with <b>{MushProtocol.EscapeUnityRichText(nextOffer.GiverLabel)}</b> in <color=#77DDFF>{MushProtocol.EscapeUnityRichText(nextOffer.GiverRoomName)}</color> ({MushProtocol.EscapeUnityRichText(nextOffer.LocationLabel)}).</color>");
                            showWayfinder = !string.IsNullOrEmpty(nextOffer.GiverRoomEntry);
                            string curPort = _boundTradeState != null
                                ? (!string.IsNullOrEmpty(_boundTradeState.ActiveVenue)
                                    ? _boundTradeState.ActiveVenue
                                    : _boundTradeState.AboardDockedPortHint)
                                : string.Empty;
                            if (!string.IsNullOrEmpty(curPort) &&
                                TryGetNextGuidedOfferDestination(curPort, out var sailDest))
                            {
                                sb.Append($"\n<color=#77DDFF>Sail:</color> +ship/plan {MushProtocol.EscapeUnityRichText(sailDest.PortId)} ({MushProtocol.EscapeUnityRichText(sailDest.PortLabel)})");
                                showPrefill = true;
                                if (_qPrefillLbl != null)
                                    _qPrefillLbl.text = $"SAIL {sailDest.PortId.ToUpperInvariant()}";
                            }
                        }
                        else if (TryGetGuidedTourProgress(out int tourDone, out int tourTotal, out _, out bool allTourDone) && allTourDone)
                        {
                            tourMastered = true;
                            sb.Append($"<color=#55EE99><b>★ CORRIDOR CAPTAIN — GUIDED TOUR COMPLETE ({tourDone}/{tourTotal}) ★</b></color>\n");
                            sb.Append("<color=#88AAC8>All 7 guided onboarding steps complete. Browse corridor haulage, survey commissions (+market/jobs), or optional NPC contracts next.</color>");
                            showPrefill = true;
                            if (_qPrefillLbl != null)
                                _qPrefillLbl.text = "JOBS BOARD";
                        }
                        else
                        {
                            sb.Append("<color=#88AAC8>No active quests. Look for NPCs with a floating golden <b>!</b> marker (such as Maren Halvorsen in Port Watch Arrivals).</color>");
                        }
                        bool showViewOffers = LatestPanel.Available.Count > 0;
                        bool showPrimary = showViewOffers || tourMastered;
                        if (showViewOffers)
                        {
                            sb.Append($"\n<color=#FFDD44><b>[!] {LatestPanel.Available.Count} {(LatestPanel.Available.Count == 1 ? "offer" : "offers")} available here</b></color>");
                            if (_qPrimaryLbl != null)
                                _qPrimaryLbl.text = $"VIEW OFFERS ({LatestPanel.Available.Count})";
                        }
                        else if (tourMastered && _qPrimaryLbl != null)
                        {
                            _qPrimaryLbl.text = $"COMPLETED ({LatestPanel.Completed.Count})";
                        }
                        if (_qPrimaryBtn != null) _qPrimaryBtn.gameObject.SetActive(showPrimary);
                        if (_qPrefillBtn != null) _qPrefillBtn.gameObject.SetActive(showPrefill);
                        if (_qWayfinderBtn != null) _qWayfinderBtn.gameObject.SetActive(showWayfinder);
                        if (_qAbandonBtn != null) _qAbandonBtn.gameObject.SetActive(false);
                        if (_qNextBtn != null) _qNextBtn.gameObject.SetActive(false);
                    }
                    else
                    {
                        int sel = _selectedQuestIndex % LatestPanel.Active.Count;
                        var q = LatestPanel.Active[sel];
                        int activeStep = GuidedStepNumber(q.Key);
                        string activeStepTag = activeStep > 0 ? $" <color=#66EEAA>[Step {activeStep}/7]</color>" : string.Empty;
                        string readyLead = q.CanFinishNow ? "<color=#55EE99>[?]</color> " : string.Empty;
                        sb.Append($"{readyLead}<b><color=#FFDF77>[{sel + 1}/{LatestPanel.Active.Count}] {MushProtocol.EscapeUnityRichText(q.Title)}</color></b>{activeStepTag}\n");
                        if (q.CanFinishNow)
                            sb.Append("<color=#55EE99><b>READY TO COMPLETE HERE</b></color>\n");
                        sb.Append($"<color=#77DDFF>Giver: {MushProtocol.EscapeUnityRichText(q.GiverName)}  ·  Turn in: {MushProtocol.EscapeUnityRichText(q.TurnInName)}</color>\n");
                        sb.Append(MushProtocol.EscapeUnityRichText(q.Introduction)).Append("\n\n");
                        for (int i = 0; i < q.Checkpoints.Count; i++)
                        {
                            var cp = q.Checkpoints[i];
                            string mark = cp.Checked ? "<color=#55EE99>[✓]</color>" : (cp.LiveReady ? "<color=#FFDD55>[◆]</color>" : "<color=#99BBCC>[ ]</color>");
                            sb.Append($"{mark} <b>{MushProtocol.EscapeUnityRichText(cp.Title)}</b>");
                            if (!string.IsNullOrEmpty(cp.RoomName))
                                sb.Append($" <color=#77DDFF>({MushProtocol.EscapeUnityRichText(cp.RoomName)})</color>");
                            sb.Append('\n');
                            if (!string.IsNullOrEmpty(cp.DetailText))
                                sb.Append($"<color=#66EEAA>  {MushProtocol.EscapeUnityRichText(cp.DetailText)}</color>\n");
                            else
                                sb.Append($"<color=#99B8CC>  {MushProtocol.EscapeUnityRichText(cp.Instructions)}</color>\n");
                        }
                        if (TryGetQuestHoldReadinessSummary(q, out string jHoldSummary, out bool jHoldReady))
                        {
                            string holdCol = jHoldReady ? "#55EE99" : "#FFDD44";
                            sb.Append($"\n<color={holdCol}><b>{MushProtocol.EscapeUnityRichText(jHoldSummary)}</b></color>");
                        }
                        if (TryGetQuestPreflightReadinessSummary(q, out string jPfSummary, out bool jPfReady))
                        {
                            string pfCol = jPfReady ? "#55EE99" : "#FFD166";
                            sb.Append($"\n<color={pfCol}><b>{MushProtocol.EscapeUnityRichText(jPfSummary)}</b></color>");
                        }
                        sb.Append($"\n<color=#FFDF77><b>REWARD:</b> {MushProtocol.EscapeUnityRichText(q.RewardSummary)}</color>");
                        if (q.AllowRestart)
                            sb.Append("\n<color=#88AAC8>Optional · Restartable (abandon and restart any time without penalty)</color>");
                        if (_qNextBtn != null) _qNextBtn.gameObject.SetActive(LatestPanel.Active.Count > 1);
                        if (_qPrimaryBtn != null) _qPrimaryBtn.gameObject.SetActive(true);
                        if (_qPrimaryLbl != null)
                        {
                            if (q.CanFinishNow) _qPrimaryLbl.text = "COMPLETE QUEST";
                            else if (q.CanCheckNow) _qPrimaryLbl.text = "VERIFY OBJECTIVE";
                            else _qPrimaryLbl.text = "TRACK ON HUD";
                        }
                        if (_qPrefillBtn != null)
                        {
                            bool hasSuggested = TryGetSuggestedCommand(q, out string sugLbl, out _);
                            _qPrefillBtn.gameObject.SetActive(hasSuggested);
                            if (hasSuggested && _qPrefillLbl != null)
                                _qPrefillLbl.text = sugLbl.ToUpperInvariant();
                        }
                        if (_qWayfinderBtn != null) _qWayfinderBtn.gameObject.SetActive(!string.IsNullOrEmpty(q.NextRoomEntry));
                        if (_qAbandonBtn != null) _qAbandonBtn.gameObject.SetActive(true);
                        if (_qAbandonLbl != null) _qAbandonLbl.text = _confirmAbandonKey == q.Key ? "CONFIRM ABANDON" : "ABANDON QUEST";
                    }
                }
                else if (ActiveQuestSubTab == "available")
                {
                    if (_qPrefillBtn != null) _qPrefillBtn.gameObject.SetActive(false);
                    if (LatestPanel.Available.Count == 0)
                    {
                        sb.Append("<color=#88AAC8>No quests are offered in this room right now.</color>");
                        if (_qPrimaryBtn != null) _qPrimaryBtn.gameObject.SetActive(false);
                        if (_qWayfinderBtn != null) _qWayfinderBtn.gameObject.SetActive(false);
                        if (_qAbandonBtn != null) _qAbandonBtn.gameObject.SetActive(false);
                        if (_qNextBtn != null) _qNextBtn.gameObject.SetActive(false);
                    }
                    else
                    {
                        int sel = _selectedQuestIndex % LatestPanel.Available.Count;
                        var q = LatestPanel.Available[sel];
                        int availStep = GuidedStepNumber(q.Key);
                        string availStepTag = availStep > 0 ? $" <color=#66EEAA>[Step {availStep}/7]</color>" : string.Empty;
                        sb.Append($"<b><color=#FFDD44>[!] [{sel + 1}/{LatestPanel.Available.Count}] {MushProtocol.EscapeUnityRichText(q.Title)}</color></b>{availStepTag}\n");
                        sb.Append($"<color=#77DDFF>Offered by: {MushProtocol.EscapeUnityRichText(q.GiverName)}  ·  Turn in: {MushProtocol.EscapeUnityRichText(q.TurnInName)}</color>\n");
                        sb.Append(MushProtocol.EscapeUnityRichText(q.Introduction)).Append("\n\n");
                        for (int i = 0; i < q.Checkpoints.Count; i++)
                        {
                            var cp = q.Checkpoints[i];
                            sb.Append($"• <b>{MushProtocol.EscapeUnityRichText(cp.Title)}</b> <color=#77DDFF>({MushProtocol.EscapeUnityRichText(cp.RoomName)})</color>\n");
                            sb.Append($"<color=#99B8CC>  {MushProtocol.EscapeUnityRichText(cp.Instructions)}</color>\n");
                        }
                        sb.Append($"\n<color=#FFDF77><b>REWARD:</b> {MushProtocol.EscapeUnityRichText(q.RewardSummary)}</color>");
                        if (q.AllowRestart)
                            sb.Append("\n<color=#88AAC8>Optional · Restartable (abandon and restart any time without penalty)</color>");
                        if (_qNextBtn != null) _qNextBtn.gameObject.SetActive(LatestPanel.Available.Count > 1);
                        if (_qPrimaryBtn != null) _qPrimaryBtn.gameObject.SetActive(true);
                        if (_qPrimaryLbl != null) _qPrimaryLbl.text = "ACCEPT QUEST";
                        if (_qWayfinderBtn != null) _qWayfinderBtn.gameObject.SetActive(q.Checkpoints.Count > 0 && !string.IsNullOrEmpty(q.Checkpoints[0].RoomEntry));
                        bool canDeclineOffer = !q.AllowRestart && !string.IsNullOrEmpty(q.DeclineCmd);
                        if (_qAbandonBtn != null) _qAbandonBtn.gameObject.SetActive(canDeclineOffer);
                        if (canDeclineOffer && _qAbandonLbl != null)
                            _qAbandonLbl.text = _confirmDeclineKey == q.Key ? "CONFIRM DECLINE" : "DECLINE";
                    }
                }
                else
                {
                    if (_qPrimaryBtn != null) _qPrimaryBtn.gameObject.SetActive(false);
                    if (_qPrefillBtn != null) _qPrefillBtn.gameObject.SetActive(false);
                    if (_qWayfinderBtn != null) _qWayfinderBtn.gameObject.SetActive(false);
                    if (_qAbandonBtn != null) _qAbandonBtn.gameObject.SetActive(false);
                    if (_qNextBtn != null) _qNextBtn.gameObject.SetActive(false);
                    if (TryGetGuidedTourProgress(out _, out _, out _, out bool allTourDone) && allTourDone)
                    {
                        sb.Append("<color=#55EE99><b>★ GUIDED CORRIDOR TOUR COMPLETE (7/7) — CORRIDOR CAPTAIN ★</b></color>\n\n");
                    }
                    if (LatestPanel.Completed.Count == 0)
                    {
                        sb.Append("<color=#88AAC8>No completed quests recorded yet.</color>");
                    }
                    else
                    {
                        for (int i = 0; i < LatestPanel.Completed.Count; i++)
                        {
                            var c = LatestPanel.Completed[i];
                            int stepNo = GuidedStepNumber(c.Key);
                            string stepTag = stepNo > 0 ? $" <color=#66EEAA>[Step {stepNo}/7]</color>" : string.Empty;
                            sb.Append($"<color=#55EE99>[✓]</color> <b>{MushProtocol.EscapeUnityRichText(c.Title)}</b>{stepTag} <color=#77DDFF>({MushProtocol.EscapeUnityRichText(c.TurnInName)})</color>\n");
                            sb.Append($"<color=#99B8CC>  {MushProtocol.EscapeUnityRichText(c.Completion)}</color>\n");
                            if (!string.IsNullOrEmpty(c.RewardSummary))
                                sb.Append($"<color=#FFDF77>  REWARD: {MushProtocol.EscapeUnityRichText(c.RewardSummary)}</color>\n");
                        }
                    }
                }

                if (_bodyText != null) _bodyText.text = sb.ToString();
                return;
            }

            if (_tabHintText != null)
                _tabHintText.text = $"{TabTitles[idx]}  ·  Live command: {TabCommands[idx]}  ({(_tabLines[idx].Count)} lines captured)";
            if (_bodyText != null)
            {
                if (_tabLines[idx].Count > 0)
                    _bodyText.text = string.Join("\n", _tabLines[idx]);
                else
                    _bodyText.text = $"<color=#88AAC8>Click <b>REFRESH TAB</b> to query live <b>{TabCommands[idx]}</b> telemetry from mushd.</color>";
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
            return CreateLabeledButton(parent, label, bg, onClick, out _);
        }

        private Button CreateLabeledButton(Transform parent, string label, Color bg, Action onClick, out Text labelTxt)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.onClick.AddListener(() => onClick?.Invoke());
            labelTxt = CreateText(go.transform, label, 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(labelTxt.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            return btn;
        }

        private Button CreateSmallButton(Transform parent, string label, Color bg, Action onClick, out Text labelTxt)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            btn.onClick.AddListener(() => onClick?.Invoke());
            labelTxt = CreateText(go.transform, label, 10, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(labelTxt.rectTransform, Vector2.zero, Vector2.one, new Vector2(2f, 1f), new Vector2(-2f, -1f));
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
