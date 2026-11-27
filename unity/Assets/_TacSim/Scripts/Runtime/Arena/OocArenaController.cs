using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.Net;
using TacSim.World;
using TMPro;
using UnityEngine;
using UnityEngine.EventSystems;

namespace TacSim.Arena
{
    public enum ArenaImpactKind
    {
        Hit = 0,
        SectionCrit = 1,
        Kill = 2,
        Miss = 3
    }

    public sealed class ArenaWeaponSlotState
    {
        public string Id;
        public bool Ready;
        public int ReadyInMs;
        public int Rounds;
        public int RoundsMax;
        public int HitPercent;
        public int HeatPerShot;
        public int ArmedSlot;
        public int ArmedMinPercent;
        public int ArmedKeeps;
        public string ArmedTarget = string.Empty;
        public int RangeBandIndex;
        public int RangeBandTotal;
        public float MaxRangeM;
        public int DispersionMrad;
    }

    public sealed class ArenaContactState
    {
        public string Designator;   // e.g. "B1", "R2", "?1"
        public string ClassLabel;   // e.g. "line_tank", "biped_mk1"
        public string UnitType;
        public string Party;        // "R", "B", "?"
        public string Relation;     // "friendly", "hostile", "unknown"
        public bool Friendly;
        public int CellX;
        public int CellY;
        public float RangeM;
        public float BearingDeg;
        public float FacingDeg;
        public string Aspect = "front"; // "front" | "left" | "right" | "rear"
        public string Band = string.Empty; // "close" | "medium" | "far"
        public bool Moving;
        public int AgeMs;
        public string HealthWord;   // "intact", "scarred", "crippled", "wreck"
        public bool IsWreck;
        public int HitOddsPct;
        public int ConditionTable = -1;
        public string ConditionBands = string.Empty;
        public int ConditionAgeMs;
        public readonly List<string> FatalSections = new List<string>(2);
    }

    public sealed class ArenaDetachmentSlot
    {
        public int Slot;
        public string Callsign = string.Empty;
        public string Pilot;
        public string Controller = "automated"; // "player" | "npc" | "automated"
        public string OrderPrefill = string.Empty;
    }

    public sealed class ArenaDetachmentOrder
    {
        public string Sequence = string.Empty;
        public string From = string.Empty;
        public string Objective = string.Empty;
        public string Response; // null | "wilco" | "negative"
    }

    /// <summary>
    /// Strict server-projected detachment command roster matching `crates/tacsim/src/detachment/live.rs`
    /// and `web/src/lib/detachment.ts`. Invalid projected commands disable the whole surface (never trimmed/repaired).
    /// </summary>
    public sealed class ArenaDetachmentState
    {
        public string Id = string.Empty;
        public string Revision = string.Empty;
        public string Term = string.Empty;
        public int? LeaderSlot;
        public readonly List<ArenaDetachmentSlot> Slots = new List<ArenaDetachmentSlot>(6);
        public string AllOrderPrefill = string.Empty;
        public ArenaDetachmentOrder MyOrder;
        public string WilcoCommand;
        public string NegativeCommand;

        private static bool IsCounter(string s)
        {
            if (string.IsNullOrEmpty(s) || s.Length > 20) return false;
            if (s.Length > 1 && s[0] == '0') return false;
            for (int i = 0; i < s.Length; i++)
            {
                if (s[i] < '0' || s[i] > '9') return false;
            }
            return ulong.TryParse(s, out _);
        }

        private static bool IsSafeLabel(string s, int maxChars)
        {
            if (string.IsNullOrEmpty(s) || s.Length > maxChars) return false;
            for (int i = 0; i < s.Length; i++)
            {
                char c = s[i];
                if (c < 0x20 || (c >= 0x7F && c <= 0x9F) || c == '\u2028' || c == '\u2029')
                    return false;
            }
            return true;
        }

        public static bool TryDecode(JsonNode node, out ArenaDetachmentState result)
        {
            result = null;
            if (node == null || !node.IsObject) return false;

            string id = node["id"].Kind == JsonKind.String ? node.Str("id", string.Empty) : string.Empty;
            string rev = node["revision"].Kind == JsonKind.String ? node.Str("revision", string.Empty) : string.Empty;
            string term = node["term"].Kind == JsonKind.String ? node.Str("term", string.Empty) : string.Empty;
            if (!IsCounter(id) || !IsCounter(rev) || !IsCounter(term)) return false;

            var slotsArr = node["slots"].AsArray();
            if (slotsArr == null || slotsArr.Count == 0 || slotsArr.Count > 6) return false;

            string prefix = $"ORDER DT:{id}:{rev}:{term} ";
            string allPrefill = node["all_order_prefill"].Kind == JsonKind.String ? node.Str("all_order_prefill", string.Empty) : string.Empty;
            if (!string.Equals(allPrefill, prefix + "ALL ", StringComparison.Ordinal)) return false;

            var parsed = new ArenaDetachmentState
            {
                Id = id,
                Revision = rev,
                Term = term,
                AllOrderPrefill = allPrefill
            };

            var seen = new HashSet<int>();
            for (int i = 0; i < slotsArr.Count; i++)
            {
                var raw = slotsArr[i];
                if (!raw.IsObject) return false;
                if (raw["slot"].Kind != JsonKind.Int && raw["slot"].Kind != JsonKind.Float) return false;
                double rawNum = raw.Double("slot", -1);
                int slot = (int)rawNum;
                if (Math.Abs(rawNum - slot) > 1e-9 || slot < 1 || slot > 6 || !seen.Add(slot)) return false;

                string cs = raw["callsign"].Kind == JsonKind.String ? raw.Str("callsign", string.Empty) : string.Empty;
                if (!IsSafeLabel(cs, 32)) return false;

                string pilot = null;
                if (!raw["pilot"].IsNull)
                {
                    if (raw["pilot"].Kind != JsonKind.String) return false;
                    pilot = raw.Str("pilot", string.Empty);
                    if (!IsSafeLabel(pilot, 96)) return false;
                }

                string ctrl = raw["controller"].Kind == JsonKind.String ? raw.Str("controller", string.Empty) : string.Empty;
                if (ctrl != "player" && ctrl != "npc" && ctrl != "automated") return false;

                string pref = raw["order_prefill"].Kind == JsonKind.String ? raw.Str("order_prefill", string.Empty) : string.Empty;
                if (!string.Equals(pref, $"{prefix}{slot} ", StringComparison.Ordinal)) return false;

                parsed.Slots.Add(new ArenaDetachmentSlot
                {
                    Slot = slot,
                    Callsign = cs,
                    Pilot = pilot,
                    Controller = ctrl,
                    OrderPrefill = pref
                });
            }
            parsed.Slots.Sort((a, b) => a.Slot.CompareTo(b.Slot));

            if (!node["leader_slot"].IsNull)
            {
                if (node["leader_slot"].Kind != JsonKind.Int && node["leader_slot"].Kind != JsonKind.Float) return false;
                double lNum = node.Double("leader_slot", -1);
                int ls = (int)lNum;
                if (Math.Abs(lNum - ls) > 1e-9 || !seen.Contains(ls)) return false;
                parsed.LeaderSlot = ls;
            }

            if (!node["my_order"].IsNull)
            {
                var ord = node["my_order"];
                if (!ord.IsObject) return false;
                string seq = ord["sequence"].Kind == JsonKind.String ? ord.Str("sequence", string.Empty) : string.Empty;
                string from = ord["from"].Kind == JsonKind.String ? ord.Str("from", string.Empty) : string.Empty;
                string obj = ord["objective"].Kind == JsonKind.String ? ord.Str("objective", string.Empty) : string.Empty;
                if (!IsCounter(seq) || !IsSafeLabel(from, 96) || !IsSafeLabel(obj, 256)) return false;

                string resp = null;
                if (!ord["response"].IsNull)
                {
                    if (ord["response"].Kind != JsonKind.String) return false;
                    resp = ord.Str("response", string.Empty);
                    if (resp != "wilco" && resp != "negative") return false;
                }
                parsed.MyOrder = new ArenaDetachmentOrder
                {
                    Sequence = seq,
                    From = from,
                    Objective = obj,
                    Response = resp
                };
            }

            string expectedWilco = parsed.MyOrder != null ? $"WILCO DT:{id}:{rev} {parsed.MyOrder.Sequence}" : null;
            string expectedNeg = parsed.MyOrder != null ? $"NEGATIVE DT:{id}:{rev} {parsed.MyOrder.Sequence}" : null;
            string actualWilco = node["wilco_command"].IsNull ? null : (node["wilco_command"].Kind == JsonKind.String ? node.Str("wilco_command", string.Empty) : "__bad__");
            string actualNeg = node["negative_command"].IsNull ? null : (node["negative_command"].Kind == JsonKind.String ? node.Str("negative_command", string.Empty) : "__bad__");
            if (!string.Equals(actualWilco, expectedWilco, StringComparison.Ordinal) ||
                !string.Equals(actualNeg, expectedNeg, StringComparison.Ordinal))
                return false;

            parsed.WilcoCommand = actualWilco;
            parsed.NegativeCommand = actualNeg;
            result = parsed;
            return true;
        }
    }

    public sealed class ArenaStandingRule
    {
        public int Slot;
        public string Action = string.Empty; // "fire" | "hold" | "lock"
        public string Weapon = string.Empty;
        public string Target = string.Empty;
        public int MinPercent;
    }

    public sealed class ArenaStandingOrderState
    {
        public string Squad = string.Empty;
        public string Objective = string.Empty;
        public string Target = string.Empty;
        public bool HasToCell;
        public int ToCellX;
        public int ToCellY;
        public bool HoldFire;
        public long ExpiresInMs;
    }

    /// <summary>
    /// Structured combat / tactical notice from `arena.cockpit.notices` (`crates/tacsim/src/match_/cockpit/frames.rs`).
    /// </summary>
    public sealed class ArenaCombatNotice
    {
        public string Kind = string.Empty;       // "hit" | "miss" | "kill" | "target_section_lost" | "struck" | "section_lost" | "destroyed" | "refused" | "slot_fired" | ...
        public string Designator = string.Empty;
        public string Was = string.Empty;
        public string Section = string.Empty;
        public string Weapon = string.Empty;
        public string Order = string.Empty;
        public string Cause = string.Empty;
        public string Refusal = string.Empty;
        public string Aspect = string.Empty;
        public string Text = string.Empty;
        public bool Spent;
        public bool Friendly;
        public int Slot;
        public int HitPercent;
        public float DetailRadiusM;
        public float DetailSecs;
        public int DetailFloorPercent;
    }

    /// <summary>
    /// Sealed unit mind metadata projected by `arena.cockpit` (`frames.rs:mind_json`).
    /// </summary>
    public sealed class ArenaMindState
    {
        public string Name = string.Empty; // empty when custom/unnamed -> display "OWN MIND"
        public string CardSchema = string.Empty;
        public readonly List<string> Profiles = new List<string>(4);
        public readonly List<string> SealedRules = new List<string>(8);
        public int ActiveIndex;
        public string ClassWord = string.Empty;
        public string CoreWord = string.Empty;
        public readonly List<string> Channels = new List<string>(3);
        public long? NextSwitchMs;
        public bool IsOff;

        public string ActiveProfileName =>
            ActiveIndex >= 0 && ActiveIndex < Profiles.Count ? Profiles[ActiveIndex] : string.Empty;
    }

    public sealed class ArenaCockpitState
    {
        public ulong MatchId;
        public long ArenaClockMs;
        public string Callsign = "R1";
        public string Party = "R";
        public string UnitType = "tacsim.unit.line_tank";
        public string Mode = "skirmish";
        public string Power = "online";
        public bool IsWreck;
        public bool Immobile;
        public bool MovementBlocked;
        public int OddsFloorPercent;
        public bool HasEcm;
        public bool EmconSilent;
        public float EcmRadiusMeters = 120f;
        public bool IsEcmJammingActive => HasEcm && !EmconSilent && !IsWreck;
        public bool WeaponsFree = true;

        public float Xm = 240f;
        public float Zm = 1280f;
        public float HeadingDeg = 90f;
        public float SpeedKph;
        public float MaxSpeedKph = 54f;
        public float ReverseSpeedKph = 24f;

        public int Heat;
        public int HeatMax = 1000;
        public bool Overheated;

        public int ArmorFrontPct = 100;
        public int ArmorLeftPct = 100;
        public int ArmorRightPct = 100;
        public int ArmorRearPct = 100;
        public int InternalsPct = 100;

        public string LockedDesignator = string.Empty;
        public float LockRangeM;
        public int LockHitPercent;

        public int RedAliveCount = 6;
        public int BlueAliveCount = 6;
        public int HoldControlScore;
        public string SquadPosture = "ENGAGE";
        public string Posture = "return"; // "return" | "guard" | "hold"
        public string MatchBanner = string.Empty;
        public float SightMeters = 120f;
        public float SensorHeightM = 2.5f;
        public string Scope = "clear"; // "clear" | "unidentified" | "hostile"
        public string Tempo = "alert";
        public bool HasReceiver;
        public int ReceiverBudget;
        public int ReceiverBudgetMax;
        public int ReceiverReachPercent = 100;
        public bool HasDynamicObjective;
        public int ObjectiveCellX;
        public int ObjectiveCellY;
        public string ObjectiveLetter = string.Empty;
        public int ObjectiveRadiusCells = 6;
        public ArenaDetachmentState Detachment;
        public bool DetachmentProjectionInvalid;
        public ArenaMindState Mind;
        public ArenaStandingOrderState StandingOrder;
        public bool HasReceivedCatchup;
        public long LastCatchupAwaySec;
        public string MotorGoal = string.Empty;
        public string MotorOutcome = string.Empty;
        public bool HasDriveWaypoint;
        public int DriveWaypointCellX;
        public int DriveWaypointCellY;
        public string DriveWaypointLabel = string.Empty;
        public int MatchKills;
        public int MatchHits;
        public int MatchMisses;

        public readonly List<ArenaWeaponSlotState> Weapons = new List<ArenaWeaponSlotState>();
        public readonly List<ArenaContactState> Contacts = new List<ArenaContactState>();
        public readonly List<ArenaSectionState> Sections = new List<ArenaSectionState>();
        public readonly List<ArenaStandingRule> Rules = new List<ArenaStandingRule>(6);
        public readonly List<List<string>> ScanTables = new List<List<string>>(4);
        public readonly List<ArenaCombatNotice> RecentNotices = new List<ArenaCombatNotice>(10);
    }

    /// <summary>
    /// Pure-online 3D OOC Simulator Arena controller driven exclusively by authoritative
    /// `mushd` `arena.cockpit` (v9) and `arena.map` (v1) WebSocket panel feeds and live
    /// cockpit verbs (`startup`, `speed`, `heading`, `drive`, `lock`, `fire`, `scan`, `emcon`, `order`, `exit`).
    /// </summary>
    public sealed class OocArenaController : MonoBehaviour
    {
        public static readonly Vector3 ArenaStageOrigin = new Vector3(-3200f, 0f, 0f);

        public TacSimAssetCatalog Catalog;
        public DecodedTerrainGrid TerrainGrid { get; private set; }
        public ArenaCockpitState State { get; private set; } = new ArenaCockpitState();

        public SpawnedGroundVehicleRig PlayerRig { get; private set; }
        public Transform PlayerVehicleTransform => PlayerRig?.Root != null ? PlayerRig.Root.transform : null;
        public Transform PlayerCupolaAnchor => PlayerRig?.CupolaCameraAnchor;

        public bool IsArenaActive { get; private set; }
        public bool IsReplayModeActive { get; private set; }
        public bool IsLiveObserverActive { get; private set; }
        public bool IsSpectatorTheaterActive => !IsArenaActive && (IsReplayModeActive || IsLiveObserverActive);
        public bool IsStageVisible => IsArenaActive || IsReplayModeActive || IsLiveObserverActive;
        public bool KeyboardSuspended { get; set; }
        public ArenaPredictiveHelm Helm { get; } = new ArenaPredictiveHelm();
        public ArenaCoalescedThrottle Throttle { get; } = new ArenaCoalescedThrottle();
        public float TargetCruiseSpeedKph => Throttle.OrderedKph.HasValue ? Throttle.OrderedKph.Value : Throttle.PredictedKph();
        public float PredictedHeadingDeg => Helm.PredictedHeadingDeg(CurrentHelmClockMs()) ?? State.HeadingDeg;
        public ulong CachedMapMatchId { get; private set; }

        public ArenaReplayDocument ActiveReplayDoc { get; private set; }
        public ArenaObserverPicture ActiveObserverPicture { get; private set; }
        public ArenaReplaySampledFrame LatestReplayFrame { get; private set; }
        public ulong ActiveSpectatorMatchId { get; private set; }
        public float ReplayCursorMs { get; private set; }
        public bool ReplayPlaying { get; set; }
        public float ReplayPlaybackSpeed { get; set; } = 1f;
        public int ReplayFocusUnitIndex { get; private set; }
        public int ActiveReplayRigCount => _replayRigs.Count;

        public Transform SpectatorFocusTransform
        {
            get
            {
                if (_replayRigs.Count > 0)
                {
                    if (_replayRigs.TryGetValue(ReplayFocusUnitIndex, out var focused) && focused?.Root != null && focused.Root.activeSelf)
                        return focused.Root.transform;
                    foreach (var kv in _replayRigs)
                    {
                        if (kv.Value?.Root != null && kv.Value.Root.activeSelf)
                            return kv.Value.Root.transform;
                    }
                }
                return _spectatorCenterAnchor;
            }
        }

        public Transform SpectatorFocusCupolaAnchor
        {
            get
            {
                if (_replayRigs.TryGetValue(ReplayFocusUnitIndex, out var focused) && focused?.CupolaCameraAnchor != null)
                    return focused.CupolaCameraAnchor;
                return null;
            }
        }

        // Diegetic Simulator Selector Console preferences (sent via live `sim` verbs in `room.simulator_lobby`)
        public string SelectedLobbyTeam { get; set; } = "red";
        public string SelectedLobbyUnit { get; set; } = "line_tank";
        public string SelectedLobbyMode { get; set; } = "skirmish";
        public int SelectedLobbyAiCount { get; set; } = 6;
        public string LiveLobbyMatchState { get; set; } = "idle";
        public string LiveLobbySlotSummary { get; set; } = string.Empty;

        public event Action<string> OnLiveCockpitVerbRequested;
        public event Action OnRequestArenaMapSubscribe;
        public event Action OnRequestReplayDocumentSubscribe;
        public event Action OnArenaStateUpdated;
        public event Action OnReplayStateChanged;
        public event Action OnReplayClosed;
        public event Action<string> OnArenaCombatLog;
        public event Action<ArenaDetachmentOrder> OnNewDetachmentOrderReceived;
        public event Action OnToggleCameraViewRequested;
        public event Action OnCycleOpticsZoomRequested;
        public event Action<bool> OnWeaponShotFired; // (isLaser)
        public event Action<string, int, bool> OnPlayerArmorDamaged; // (facetLabel, deltaPct, internalHit)
        public event Action<ArenaCombatNotice> OnArenaNoticeReceived;

        public int ActiveShotTracerCount => _activeTracers.Count;
        public bool IsMuzzleFlashActive => _playerMuzzleFlash != null && _playerMuzzleFlash.gameObject.activeSelf;
        public bool IsLockedTargetGroundRingVisible => _lockedTargetGroundRing != null && _lockedTargetGroundRing.enabled;
        public bool IsDriveWaypointBeaconVisible => _driveWaypointRoot != null && _driveWaypointRoot.gameObject.activeSelf;
        public bool IsDriveCourseLineVisible => _driveCourseLine != null && _driveCourseLine.enabled;

        public bool TryGetLockedContact(out ArenaContactState contact)
        {
            contact = null;
            if (State == null || string.IsNullOrEmpty(State.LockedDesignator) || State.Contacts == null)
                return false;
            for (int i = 0; i < State.Contacts.Count; i++)
            {
                if (string.Equals(State.Contacts[i].Designator, State.LockedDesignator, StringComparison.OrdinalIgnoreCase))
                {
                    contact = State.Contacts[i];
                    return true;
                }
            }
            return false;
        }

        public IReadOnlyList<string> TryGetContactScanSections(ArenaContactState contact)
        {
            if (contact == null || State == null || contact.ConditionTable < 0 || contact.ConditionTable >= State.ScanTables.Count)
                return null;
            return State.ScanTables[contact.ConditionTable];
        }

        public bool TryGetContactWorldPosition(string designator, out Vector3 worldPos)
        {
            worldPos = Vector3.zero;
            if (string.IsNullOrWhiteSpace(designator)) return false;
            string key = designator.Trim();
            if (_contactRigs.TryGetValue(key, out var rig) && rig?.Root != null)
            {
                worldPos = rig.Root.transform.position + Vector3.up * 2.6f;
                return true;
            }
            if (TerrainGrid != null && State?.Contacts != null)
            {
                for (int i = 0; i < State.Contacts.Count; i++)
                {
                    var ct = State.Contacts[i];
                    if (string.Equals(ct.Designator, key, StringComparison.OrdinalIgnoreCase))
                    {
                        worldPos = ArenaStageOrigin + TerrainGrid.SiteCellToLocalPos(ct.CellX, ct.CellY, 2.6f);
                        return true;
                    }
                }
            }
            return false;
        }

        public int? AutoLockRuleSlot
        {
            get
            {
                if (State?.Rules == null) return null;
                for (int i = 0; i < State.Rules.Count; i++)
                {
                    if (string.Equals(State.Rules[i].Action, "lock", StringComparison.OrdinalIgnoreCase))
                        return State.Rules[i].Slot;
                }
                return null;
            }
        }

        public bool HasAutoLockRule => AutoLockRuleSlot.HasValue;

        public bool HasAutoFireRule
        {
            get
            {
                if (State?.Rules == null) return false;
                for (int i = 0; i < State.Rules.Count; i++)
                {
                    if (string.Equals(State.Rules[i].Action, "fire", StringComparison.OrdinalIgnoreCase))
                        return true;
                }
                return false;
            }
        }

        public void ToggleAutoLockRule()
        {
            int? slot = AutoLockRuleSlot;
            if (slot.HasValue)
            {
                OnLiveCockpitVerbRequested?.Invoke($"MIND RULE {slot.Value} OFF");
            }
            else
            {
                OnLiveCockpitVerbRequested?.Invoke("LOCK nearest WHEN nolock");
            }
        }

        public void SetAutoFireRule(int minOddsPct, bool enable, string weaponId = "ALL")
        {
            string safeWeapon = "ALL";
            if (!string.IsNullOrWhiteSpace(weaponId) && !string.Equals(weaponId, "ALL", StringComparison.OrdinalIgnoreCase))
            {
                if (!MushProtocol.TryValidateSafeCommandToken(weaponId, out string validWpn, maxLen: 32))
                    return;
                safeWeapon = validWpn;
            }

            int clampedOdds = Mathf.Clamp(minOddsPct, 10, 100);
            if (enable)
            {
                if (State != null && !State.WeaponsFree)
                    OnLiveCockpitVerbRequested?.Invoke("FIRE FREE");
                OnLiveCockpitVerbRequested?.Invoke($"FIRE {safeWeapon} WHEN odds >= {clampedOdds}");
                return;
            }

            bool clearedAny = false;
            if (State?.Rules != null)
            {
                for (int i = 0; i < State.Rules.Count; i++)
                {
                    if (string.Equals(State.Rules[i].Action, "fire", StringComparison.OrdinalIgnoreCase))
                    {
                        OnLiveCockpitVerbRequested?.Invoke($"MIND RULE {State.Rules[i].Slot} OFF");
                        clearedAny = true;
                    }
                }
            }
            if (!clearedAny)
                OnLiveCockpitVerbRequested?.Invoke($"FIRE {safeWeapon} OFF");
        }

        public void ToggleAutoFireRule(int minOddsPct, string weaponId = "ALL")
        {
            SetAutoFireRule(minOddsPct, enable: !HasAutoFireRule, weaponId: weaponId);
        }

        public bool SwitchMindProfile(string profileName)
        {
            if (State?.Mind == null || string.IsNullOrWhiteSpace(profileName)) return false;
            if (!MushProtocol.TryValidateSafeCommandToken(profileName, out string safeName, maxLen: 32))
                return false;
            if (string.Equals(State.Mind.ActiveProfileName, safeName, StringComparison.OrdinalIgnoreCase))
                return false;
            OnLiveCockpitVerbRequested?.Invoke($"MIND PROFILE {safeName}");
            return true;
        }

        public bool SetHullPosture(string postureWord)
        {
            if (State == null || string.IsNullOrWhiteSpace(postureWord)) return false;
            string low = postureWord.Trim().ToLowerInvariant();
            if (low != "return" && low != "guard" && low != "hold") return false;
            if (string.Equals(State.Posture, low, StringComparison.OrdinalIgnoreCase))
                return false;
            OnLiveCockpitVerbRequested?.Invoke($"MIND POSTURE {low}");
            return true;
        }

        public bool ClearStandingRuleSlot(int slot)
        {
            if (slot <= 0 || slot > 32) return false;
            OnLiveCockpitVerbRequested?.Invoke($"MIND RULE {slot} OFF");
            return true;
        }

        public bool TryGetLockedContactBearingDeg(out float bearingDeg)
        {
            bearingDeg = 0f;
            if (State == null || string.IsNullOrEmpty(State.LockedDesignator)) return false;
            if (PlayerRig?.Root != null && TryGetContactWorldPosition(State.LockedDesignator, out Vector3 targetPos))
            {
                Vector3 d = targetPos - PlayerRig.Root.transform.position;
                if (d.sqrMagnitude > 0.25f)
                {
                    bearingDeg = ((Mathf.Atan2(d.x, d.z) * Mathf.Rad2Deg) % 360f + 360f) % 360f;
                    return true;
                }
            }
            for (int i = 0; i < State.Contacts.Count; i++)
            {
                if (string.Equals(State.Contacts[i].Designator, State.LockedDesignator, StringComparison.OrdinalIgnoreCase))
                {
                    bearingDeg = ((State.Contacts[i].BearingDeg % 360f) + 360f) % 360f;
                    return true;
                }
            }
            return false;
        }

        public bool TryGetPrimaryWeaponGunnery(out ArenaWeaponSlotState primary)
        {
            primary = null;
            if (State?.Weapons == null || State.Weapons.Count == 0) return false;
            primary = State.Weapons[0];
            return primary != null;
        }

        public bool TryGetActiveObjectiveWorldPos(out Vector3 worldPos, out string label)
        {
            worldPos = Vector3.zero;
            label = string.Empty;
            if (TerrainGrid == null || State == null) return false;

            if (State.HasDynamicObjective)
            {
                worldPos = ArenaStageOrigin + TerrainGrid.SiteCellToLocalPos(State.ObjectiveCellX, State.ObjectiveCellY, 6f);
                label = !string.IsNullOrEmpty(State.ObjectiveLetter) ? State.ObjectiveLetter : "OBJ";
                return true;
            }
            if (!string.IsNullOrWhiteSpace(TerrainGrid.HoldSpotLetter) && TerrainGrid.KeySpots != null)
            {
                for (int i = 0; i < TerrainGrid.KeySpots.Count; i++)
                {
                    var sp = TerrainGrid.KeySpots[i];
                    if (string.Equals(sp.Letter, TerrainGrid.HoldSpotLetter, StringComparison.OrdinalIgnoreCase))
                    {
                        worldPos = ArenaStageOrigin + TerrainGrid.SiteCellToLocalPos(sp.CellX, sp.CellY, 6f);
                        label = sp.Letter;
                        return true;
                    }
                }
            }
            return false;
        }

        public bool TryGetActiveObjectiveBearingDeg(out float bearingDeg, out float rangeM, out string label)
        {
            bearingDeg = 0f;
            rangeM = 0f;
            label = string.Empty;
            if (TerrainGrid == null || State == null) return false;

            Vector3 targetLocal;
            if (State.HasDynamicObjective)
            {
                targetLocal = TerrainGrid.SiteCellToLocalPos(State.ObjectiveCellX, State.ObjectiveCellY, 0f);
                label = !string.IsNullOrEmpty(State.ObjectiveLetter) ? State.ObjectiveLetter : "OBJ";
            }
            else if (!string.IsNullOrWhiteSpace(TerrainGrid.HoldSpotLetter) && TerrainGrid.KeySpots != null)
            {
                ArenaKeySpotDef found = null;
                for (int i = 0; i < TerrainGrid.KeySpots.Count; i++)
                {
                    if (string.Equals(TerrainGrid.KeySpots[i].Letter, TerrainGrid.HoldSpotLetter, StringComparison.OrdinalIgnoreCase))
                    {
                        found = TerrainGrid.KeySpots[i];
                        break;
                    }
                }
                if (found == null) return false;
                targetLocal = TerrainGrid.SiteCellToLocalPos(found.CellX, found.CellY, 0f);
                label = found.Letter;
            }
            else
            {
                return false;
            }

            Vector3 ownLocal = PlayerRig?.Root != null
                ? PlayerRig.Root.transform.position - ArenaStageOrigin
                : ArenaWorldMetersToLocalPos(State.Xm, State.Zm);
            float dx = targetLocal.x - ownLocal.x;
            float dz = targetLocal.z - ownLocal.z;
            rangeM = Mathf.Sqrt(dx * dx + dz * dz);
            bearingDeg = rangeM > 0.5f ? (((Mathf.Atan2(dx, dz) * Mathf.Rad2Deg) % 360f) + 360f) % 360f : State.HeadingDeg;
            return true;
        }

        public bool HandleArenaCombatHotkey(KeyCode key)
        {
            if (KeyboardSuspended) return false;
            switch (key)
            {
                case KeyCode.Space:
                    ExecuteCockpitAction("fire");
                    return true;
                case KeyCode.V:
                    OnToggleCameraViewRequested?.Invoke();
                    return true;
                case KeyCode.Q:
                    OnCycleOpticsZoomRequested?.Invoke();
                    return true;
                case KeyCode.Alpha1:
                    ExecuteCockpitAction("throttle slow");
                    return true;
                case KeyCode.Alpha2:
                    ExecuteCockpitAction("throttle half");
                    return true;
                case KeyCode.Alpha3:
                    ExecuteCockpitAction("throttle cruise");
                    return true;
                case KeyCode.Alpha4:
                    ExecuteCockpitAction("throttle full");
                    return true;
                case KeyCode.Alpha0:
                    ExecuteCockpitAction("speed 0");
                    return true;
                case KeyCode.F:
                    ExecuteCockpitAction(State != null && State.WeaponsFree ? "fire hold" : "fire free");
                    return true;
                case KeyCode.G:
                    ExecuteCockpitAction("fire all");
                    return true;
                case KeyCode.N:
                    ExecuteCockpitAction("scan");
                    return true;
                case KeyCode.M:
                    ExecuteCockpitAction("emcon");
                    return true;
                case KeyCode.Z:
                    ExecuteCockpitAction("startup");
                    return true;
                case KeyCode.L:
                    ToggleAutoLockRule();
                    return true;
                case KeyCode.Y:
                    if (State?.Detachment != null &&
                        !State.DetachmentProjectionInvalid &&
                        State.Detachment.MyOrder != null &&
                        string.IsNullOrEmpty(State.Detachment.MyOrder.Response) &&
                        !string.IsNullOrEmpty(State.Detachment.WilcoCommand))
                    {
                        OnLiveCockpitVerbRequested?.Invoke(State.Detachment.WilcoCommand);
                        return true;
                    }
                    return false;
                default:
                    return false;
            }
        }

        private void OnDestroy()
        {
            if (_propsRoot != null)
                TacSimArenaEnvironmentBuilder.DestroyEnvironmentProps(_propsRoot);
            if (_terrainRoot != null)
                TacSimTerrainDecoder.DestroyTerrainChunkResources(_terrainRoot);
        }

        public int BuiltTreePropCount { get; private set; }
        public int BuiltRockPropCount { get; private set; }
        public int BuiltWallPropCount { get; private set; }
        public int BuiltWaterCellCount { get; private set; }
        public float LastPlayerSlopeTiltDeg { get; private set; }
        public ArenaTerrainTacticalSample CurrentTerrainIntel { get; private set; }
        public ArenaBiomeLightingProfile ActiveBiomeLighting { get; private set; }
        public bool IsDynamicObjectiveRingVisible => _dynamicObjectiveRoot != null && _dynamicObjectiveRoot.gameObject.activeSelf;
        public bool IsPlayerDustPlumeActive => _playerDustRoot != null && _playerDustRoot.gameObject.activeSelf;

        public void ResetSessionState()
        {
            IsArenaActive = false;
            IsReplayModeActive = false;
            IsLiveObserverActive = false;
            ActiveReplayDoc = null;
            ActiveObserverPicture = null;
            LatestReplayFrame = null;
            ActiveSpectatorMatchId = 0;
            ReplayCursorMs = 0f;
            ReplayPlaying = false;
            ReplayPlaybackSpeed = 1f;
            ReplayFocusUnitIndex = 0;
            _pendingOpenKeyframeIndex = -1;
            CachedMapMatchId = 0;
            _desiredLiveSpeedKph = 0f;
            _lastSentSpeedKph = 0f;
            _lastSentHeadingDeg = -999f;
            _lastNotifiedDetachmentOrderKey = string.Empty;
            _nextHeldThrottleBumpMs = 0L;
            _lastFrameAnchorMs = 0L;
            _hasPrevArmorSnapshot = false;
            _prevArmorMatchId = 0;
            _playerMuzzleFlashUntil = -1f;
            if (_playerMuzzleFlash != null)
                _playerMuzzleFlash.gameObject.SetActive(false);
            if (_lockedTargetGroundRing != null)
                _lockedTargetGroundRing.enabled = false;
            if (_weaponEnvelopeRing != null)
                _weaponEnvelopeRing.enabled = false;
            if (_ecmJammerRing != null)
                _ecmJammerRing.enabled = false;
            if (_driveWaypointRoot != null)
                _driveWaypointRoot.gameObject.SetActive(false);
            if (_driveCourseLine != null)
                _driveCourseLine.enabled = false;
            DeactivatePooledImpactsAndPopups();
            Helm.Reset();
            Throttle.Reset();
            LiveLobbyMatchState = "idle";
            LiveLobbySlotSummary = string.Empty;
            State = new ArenaCockpitState();

            foreach (var kv in _contactRigs)
            {
                if (kv.Value?.Root != null)
                {
                    if (Application.isPlaying) Destroy(kv.Value.Root);
                    else DestroyImmediate(kv.Value.Root);
                }
            }
            _contactRigs.Clear();
            ClearReplayRigs();
            OnArenaStateUpdated?.Invoke();
            OnReplayStateChanged?.Invoke();
        }

        private static long CurrentHelmClockMs() => (long)Math.Round(Time.unscaledTime * 1000.0);

        private Transform _terrainRoot;
        private Transform _propsRoot;
        private Transform _beaconsRoot;
        private Transform _unitsRoot;
        private Transform _vfxRoot;
        private Transform _spectatorCenterAnchor;
        private Transform _dynamicObjectiveRoot;
        private Transform _dynamicObjectiveRing;
        private MeshRenderer _dynamicObjectivePillarRenderer;
        private MeshRenderer _dynamicObjectiveRingRenderer;
        private TextMeshPro _dynamicObjectiveLabel;
        private readonly List<TrackedKeySpotBeacon> _keySpotBeacons = new List<TrackedKeySpotBeacon>(8);
        private Transform _playerDustRoot;
        private readonly Transform[] _playerDustPuffs = new Transform[3];
        private Transform _playerMuzzleFlash;
        private MeshRenderer _playerMuzzleFlashRenderer;
        private float _playerMuzzleFlashUntil = -1f;

        private bool _hasPrevArmorSnapshot;
        private ulong _prevArmorMatchId;
        private int _prevArmorFront = 100;
        private int _prevArmorLeft = 100;
        private int _prevArmorRight = 100;
        private int _prevArmorRear = 100;
        private int _prevInternals = 100;

        private readonly Dictionary<string, SpawnedGroundVehicleRig> _contactRigs = new Dictionary<string, SpawnedGroundVehicleRig>(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<int, SpawnedGroundVehicleRig> _replayRigs = new Dictionary<int, SpawnedGroundVehicleRig>();
        private readonly List<ArenaReplayShot> _scratchWindowShots = new List<ArenaReplayShot>(16);
        private readonly List<ActiveShotTracer> _activeTracers = new List<ActiveShotTracer>();
        private readonly List<PooledImpactBurst> _impactBurstPool = new List<PooledImpactBurst>(8);
        private readonly List<PooledWorldPopup> _worldPopupPool = new List<PooledWorldPopup>(10);
        private int _nextImpactPoolCursor;
        private int _nextPopupPoolCursor;
        private int _pendingOpenKeyframeIndex = -1;
        private string _lastNotifiedDetachmentOrderKey = string.Empty;
        private long _nextHeldThrottleBumpMs;
        private long _lastFrameAnchorMs;

        private float _desiredLiveSpeedKph;
        private float _desiredLiveHeadingDeg = 90f;
        private float _lastSentSpeedKph;
        private float _lastSentHeadingDeg = -999f;
        private float _nextLiveHelmSendTime;
        private Transform _radarPulseRing;
        private float _radarPulseScale = -1f;
        private LineRenderer _arenaRangeRing;
        private LineRenderer _weaponEnvelopeRing;
        private LineRenderer _ecmJammerRing;
        private LineRenderer _arenaLockBeam;
        private LineRenderer _lockedTargetGroundRing;
        private Transform _driveWaypointRoot;
        private TextMeshPro _driveWaypointLabel;
        private LineRenderer _driveCourseLine;

        public bool IsArenaLockBeamActive => _arenaLockBeam != null && _arenaLockBeam.enabled;
        public bool IsArenaRangeRingVisible => _arenaRangeRing != null && _arenaRangeRing.enabled;
        public bool IsWeaponEnvelopeRingVisible => _weaponEnvelopeRing != null && _weaponEnvelopeRing.enabled;
        public bool IsEcmJammerRingVisible => _ecmJammerRing != null && _ecmJammerRing.enabled;
        public string ActiveHoldZoneVisualOwner { get; private set; } = "NEUTRAL";
        public int ActiveImpactBurstCount { get; private set; }
        public int ActiveWorldPopupCount { get; private set; }
        public string LastWorldPopupText { get; private set; } = string.Empty;
        public ArenaImpactKind LastImpactKind { get; private set; }

        public string GetContactNameplateTextForTests(string designator)
        {
            if (TryGetContactRig(designator, out var rig) && rig?.Billboard?.NameplateText != null)
                return rig.Billboard.NameplateText.text;
            return string.Empty;
        }

        public float GetContactHealthBarFillFracForTests(string designator)
        {
            return TryGetContactRig(designator, out var rig) && rig != null ? rig.CurrentHealthBarFillFrac : -1f;
        }

        public bool IsContactHealthBarVisibleForTests(string designator)
        {
            return TryGetContactRig(designator, out var rig) && rig != null && rig.IsHealthBarVisible;
        }

        public string GetKeySpotLabelForTests(string letter)
        {
            if (string.IsNullOrWhiteSpace(letter)) return string.Empty;
            for (int i = 0; i < _keySpotBeacons.Count; i++)
            {
                if (string.Equals(_keySpotBeacons[i].Letter, letter.Trim(), StringComparison.OrdinalIgnoreCase) &&
                    _keySpotBeacons[i].Label != null)
                {
                    return _keySpotBeacons[i].Label.text;
                }
            }
            return string.Empty;
        }

        private sealed class TrackedKeySpotBeacon
        {
            public string Letter = string.Empty;
            public string DisplayName = string.Empty;
            public int CellX;
            public int CellY;
            public bool IsHoldSpot;
            public Vector3 WorldPos;
            public TextMeshPro Label;
            public MeshRenderer PillarRenderer;
            public MeshRenderer HoldRingRenderer;
        }

        private sealed class PooledImpactBurst
        {
            public GameObject Root;
            public Transform CoreSphere;
            public MeshRenderer CoreRenderer;
            public Transform ShockRing;
            public MeshRenderer RingRenderer;
            public ArenaImpactKind Kind;
            public float Elapsed;
            public float Duration;
            public bool Active;
        }

        private sealed class PooledWorldPopup
        {
            public GameObject Root;
            public TextMeshPro Label;
            public Vector3 BasePos;
            public float BaseScale = 1f;
            public float Elapsed;
            public float Duration;
            public bool Active;
        }

        private sealed class ActiveShotTracer
        {
            public LineRenderer Line;
            public Transform ProjectileBolt;
            public Transform ImpactFlash;
            public Transform ShockwaveRing;
            public Vector3 StartWorld;
            public Vector3 EndWorld;
            public bool IsLaser;
            public float Elapsed;
            public float Duration;
        }

        public static bool IsLiveCockpitFrameSeated(JsonNode d)
        {
            if (d == null || !d.IsObject) return false;
            if (d.Bool("cleared", false)) return false;
            if (!d.Bool("seated", true)) return false;
            if (d.Long("match", 0) <= 0) return false;
            if (string.IsNullOrWhiteSpace(d.Str("callsign", string.Empty))) return false;
            return true;
        }

        public void BuildArenaTerrain(TacSimAssetCatalog catalog, DecodedTerrainGrid grid)
        {
            Catalog = catalog;
            TerrainGrid = grid ?? TacSimTerrainDecoder.LoadStaticArena("proving_ground");

            if (_propsRoot != null)
                TacSimArenaEnvironmentBuilder.DestroyEnvironmentProps(_propsRoot);
            if (_terrainRoot != null)
                TacSimTerrainDecoder.DestroyTerrainChunkResources(_terrainRoot);

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _terrainRoot = null;
            _propsRoot = null;
            _dynamicObjectiveRoot = null;
            _dynamicObjectiveRing = null;
            _dynamicObjectivePillarRenderer = null;
            _dynamicObjectiveRingRenderer = null;
            _dynamicObjectiveLabel = null;
            _keySpotBeacons.Clear();
            ActiveHoldZoneVisualOwner = "NEUTRAL";
            _playerDustRoot = null;
            _playerMuzzleFlash = null;
            _playerMuzzleFlashRenderer = null;
            _playerMuzzleFlashUntil = -1f;
            _weaponEnvelopeRing = null;
            _ecmJammerRing = null;
            _lockedTargetGroundRing = null;
            _driveWaypointRoot = null;
            _driveWaypointLabel = null;
            _driveCourseLine = null;
            _contactRigs.Clear();
            _replayRigs.Clear();
            _activeTracers.Clear();
            _impactBurstPool.Clear();
            _worldPopupPool.Clear();
            _nextImpactPoolCursor = 0;
            _nextPopupPoolCursor = 0;
            ActiveImpactBurstCount = 0;
            ActiveWorldPopupCount = 0;
            LastWorldPopupText = string.Empty;

            transform.position = ArenaStageOrigin;

            var centerGo = new GameObject("SpectatorCenterAnchor");
            centerGo.transform.SetParent(transform, false);
            float midX = TerrainGrid.Width * TerrainGrid.CellSizeM * 0.5f;
            float midZ = TerrainGrid.Height * TerrainGrid.CellSizeM * 0.5f;
            centerGo.transform.localPosition = new Vector3(midX, TerrainGrid.SampleElevationMeters(midX, midZ) + 2f, midZ);
            _spectatorCenterAnchor = centerGo.transform;

            _terrainRoot = new GameObject("ArenaTerrainChunks").transform;
            _terrainRoot.SetParent(transform, false);
            TacSimTerrainDecoder.BuildChunkedTerrainMeshes(TerrainGrid, _terrainRoot, chunkCells: 64);

            _propsRoot = new GameObject("ArenaTerrainProps").transform;
            _propsRoot.SetParent(transform, false);
            var propSummary = TacSimArenaEnvironmentBuilder.BuildArenaProps(TerrainGrid, _propsRoot);
            BuiltTreePropCount = propSummary.TreeCount;
            BuiltRockPropCount = propSummary.RockCount;
            BuiltWallPropCount = propSummary.WallCount;
            BuiltWaterCellCount = propSummary.WaterCellCount;

            _beaconsRoot = new GameObject("KeySpotBeacons").transform;
            _beaconsRoot.SetParent(transform, false);
            BuildKeySpotsAndHoldRing();
            BuildDynamicObjectiveBeacon();

            _unitsRoot = new GameObject("ArenaCombatUnits").transform;
            _unitsRoot.SetParent(transform, false);

            _vfxRoot = new GameObject("ArenaCombatVfx").transform;
            _vfxRoot.SetParent(transform, false);
            BuildPlayerDustTrailVisual();
            BuildPlayerMuzzleFlashVisual();

            var pulseGo = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            pulseGo.name = "ArenaRadarPulseRing";
            pulseGo.transform.SetParent(_vfxRoot, false);
            var pulseCol = pulseGo.GetComponent<Collider>();
            if (pulseCol != null) { if (Application.isPlaying) Destroy(pulseCol); else DestroyImmediate(pulseCol); }
            pulseGo.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaRadarWave", new Color(0.25f, 0.92f, 1.0f, 0.48f));
            pulseGo.SetActive(false);
            _radarPulseRing = pulseGo.transform;
            _radarPulseScale = -1f;

            var rangeGo = new GameObject("ArenaWeaponRangeRing");
            rangeGo.transform.SetParent(_vfxRoot, false);
            _arenaRangeRing = rangeGo.AddComponent<LineRenderer>();
            _arenaRangeRing.useWorldSpace = true;
            _arenaRangeRing.loop = true;
            _arenaRangeRing.startWidth = 0.35f;
            _arenaRangeRing.endWidth = 0.35f;
            _arenaRangeRing.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaRangeRingMat", new Color(0.24f, 0.86f, 0.96f, 0.32f));
            _arenaRangeRing.positionCount = 40;
            _arenaRangeRing.enabled = false;

            var wpnEnvGo = new GameObject("ArenaPrimaryWeaponEnvelopeRing");
            wpnEnvGo.transform.SetParent(_vfxRoot, false);
            _weaponEnvelopeRing = wpnEnvGo.AddComponent<LineRenderer>();
            _weaponEnvelopeRing.useWorldSpace = true;
            _weaponEnvelopeRing.loop = true;
            _weaponEnvelopeRing.startWidth = 0.30f;
            _weaponEnvelopeRing.endWidth = 0.30f;
            _weaponEnvelopeRing.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaWeaponEnvelopeMat", new Color(0.45f, 0.92f, 0.56f, 0.28f));
            _weaponEnvelopeRing.positionCount = 40;
            _weaponEnvelopeRing.enabled = false;

            var ecmRingGo = new GameObject("ArenaEcmJammerRing");
            ecmRingGo.transform.SetParent(_vfxRoot, false);
            _ecmJammerRing = ecmRingGo.AddComponent<LineRenderer>();
            _ecmJammerRing.useWorldSpace = true;
            _ecmJammerRing.loop = true;
            _ecmJammerRing.startWidth = 0.48f;
            _ecmJammerRing.endWidth = 0.48f;
            _ecmJammerRing.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaEcmJammerMat", new Color(0.80f, 0.44f, 1.0f, 0.44f));
            _ecmJammerRing.positionCount = 36;
            _ecmJammerRing.enabled = false;

            var lockGo = new GameObject("ArenaTargetLockBeam");
            lockGo.transform.SetParent(_vfxRoot, false);
            _arenaLockBeam = lockGo.AddComponent<LineRenderer>();
            _arenaLockBeam.useWorldSpace = true;
            _arenaLockBeam.positionCount = 2;
            _arenaLockBeam.startWidth = 0.24f;
            _arenaLockBeam.endWidth = 0.16f;
            _arenaLockBeam.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaLockBeamMat", new Color(1.0f, 0.32f, 0.20f, 0.65f));
            _arenaLockBeam.enabled = false;

            var lockRingGo = new GameObject("ArenaLockedTargetGroundRing");
            lockRingGo.transform.SetParent(_vfxRoot, false);
            _lockedTargetGroundRing = lockRingGo.AddComponent<LineRenderer>();
            _lockedTargetGroundRing.useWorldSpace = true;
            _lockedTargetGroundRing.loop = true;
            _lockedTargetGroundRing.startWidth = 0.42f;
            _lockedTargetGroundRing.endWidth = 0.42f;
            _lockedTargetGroundRing.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaLockGroundRingMat", new Color(1.0f, 0.72f, 0.18f, 0.82f));
            _lockedTargetGroundRing.positionCount = 28;
            _lockedTargetGroundRing.enabled = false;

            var wpGo = new GameObject("ArenaDriveWaypointBeacon");
            wpGo.transform.SetParent(_vfxRoot, false);
            _driveWaypointRoot = wpGo.transform;

            var wpPillar = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            wpPillar.name = "WaypointPillar";
            wpPillar.transform.SetParent(_driveWaypointRoot, false);
            wpPillar.transform.localPosition = new Vector3(0f, 11f, 0f);
            wpPillar.transform.localScale = new Vector3(0.95f, 11f, 0.95f);
            var wpc = wpPillar.GetComponent<Collider>();
            if (wpc != null) { if (Application.isPlaying) Destroy(wpc); else DestroyImmediate(wpc); }
            wpPillar.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaDriveWpPillar", new Color(0.24f, 0.92f, 1.0f, 0.50f));

            var wpRing = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            wpRing.name = "WaypointGroundRing";
            wpRing.transform.SetParent(_driveWaypointRoot, false);
            wpRing.transform.localPosition = new Vector3(0f, 0.35f, 0f);
            wpRing.transform.localScale = new Vector3(11f, 0.12f, 11f);
            var wrc = wpRing.GetComponent<Collider>();
            if (wrc != null) { if (Application.isPlaying) Destroy(wrc); else DestroyImmediate(wrc); }
            wpRing.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaDriveWpRing", new Color(0.24f, 0.92f, 1.0f, 0.34f));

            var wpBbGo = new GameObject("WaypointLabel");
            wpBbGo.transform.SetParent(_driveWaypointRoot, false);
            wpBbGo.transform.localPosition = new Vector3(0f, 24f, 0f);
            var wpBb = wpBbGo.AddComponent<WorldBillboard>();
            var wpTxtGo = new GameObject("Txt");
            wpTxtGo.transform.SetParent(wpBbGo.transform, false);
            var wpTmp = wpTxtGo.AddComponent<TextMeshPro>();
            wpTmp.richText = false;
            wpTmp.text = "WAYPOINT";
            wpTmp.fontSize = 15f;
            wpTmp.color = new Color(0.45f, 0.96f, 1.0f);
            wpTmp.alignment = TextAlignmentOptions.Center;
            wpTmp.textWrappingMode = TextWrappingModes.NoWrap;
            wpTmp.rectTransform.sizeDelta = new Vector2(84f, 8f);
            wpBb.NameplateText = wpTmp;
            _driveWaypointLabel = wpTmp;
            wpGo.SetActive(false);

            var courseGo = new GameObject("ArenaDriveCourseLine");
            courseGo.transform.SetParent(_vfxRoot, false);
            _driveCourseLine = courseGo.AddComponent<LineRenderer>();
            _driveCourseLine.useWorldSpace = true;
            _driveCourseLine.positionCount = 12;
            _driveCourseLine.startWidth = 0.28f;
            _driveCourseLine.endWidth = 0.28f;
            _driveCourseLine.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaDriveCourseMat", new Color(0.24f, 0.92f, 1.0f, 0.56f));
            _driveCourseLine.enabled = false;

            ActiveBiomeLighting = TacSimArenaEnvironmentBuilder.ResolveBiomeLighting(TerrainGrid.Biome);

            var sunGo = new GameObject("ArenaDirectionalFill");
            sunGo.transform.SetParent(transform, false);
            sunGo.transform.localPosition = new Vector3(1280f, 360f, 1280f);
            var lt = sunGo.AddComponent<Light>();
            lt.type = LightType.Point;
            lt.range = 3200f;
            lt.intensity = ActiveBiomeLighting.SunIntensity;
            lt.color = ActiveBiomeLighting.SunColor;

            if (IsArenaActive || IsSpectatorTheaterActive)
            {
                RenderSettings.ambientLight = ActiveBiomeLighting.AmbientColor;
                RenderSettings.fog = ActiveBiomeLighting.EnableFog;
                RenderSettings.fogColor = ActiveBiomeLighting.FogColor;
                RenderSettings.fogDensity = ActiveBiomeLighting.FogDensity;
            }
        }

        private void BuildKeySpotsAndHoldRing()
        {
            if (TerrainGrid == null || _beaconsRoot == null) return;
            _keySpotBeacons.Clear();

            for (int i = 0; i < TerrainGrid.KeySpots.Count; i++)
            {
                var sp = TerrainGrid.KeySpots[i];
                Vector3 localPos = TerrainGrid.SiteCellToLocalPos(sp.CellX, sp.CellY, 0f);
                Vector3 worldPos = ArenaStageOrigin + localPos;

                bool isHold = string.Equals(sp.Letter, TerrainGrid.HoldSpotLetter, StringComparison.OrdinalIgnoreCase);
                Color col = isHold ? new Color(1.0f, 0.82f, 0.25f) : new Color(0.35f, 0.90f, 1.0f);

                var spotRoot = new GameObject($"KeySpot_{sp.Letter}");
                spotRoot.transform.SetParent(_beaconsRoot, false);
                spotRoot.transform.position = worldPos;

                var sc = spotRoot.AddComponent<SphereCollider>();
                sc.radius = 12f;

                var pillar = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
                pillar.transform.SetParent(spotRoot.transform, false);
                pillar.transform.localPosition = new Vector3(0f, 14f, 0f);
                pillar.transform.localScale = new Vector3(1.4f, 14f, 1.4f);
                var pc = pillar.GetComponent<Collider>();
                if (pc != null) { if (Application.isPlaying) Destroy(pc); else DestroyImmediate(pc); }
                var pillarRenderer = pillar.GetComponent<MeshRenderer>();
                pillarRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"SpotBeam_{sp.Letter}", new Color(col.r, col.g, col.b, 0.42f));

                MeshRenderer holdRingRenderer = null;
                if (isHold)
                {
                    float ringDiameter = Mathf.Max(20f, TerrainGrid.HoldRadiusCells * TerrainGrid.CellSizeM * 2f);
                    var capRing = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
                    capRing.name = "HoldCaptureZone";
                    capRing.transform.SetParent(spotRoot.transform, false);
                    capRing.transform.localPosition = new Vector3(0f, 0.35f, 0f);
                    capRing.transform.localScale = new Vector3(ringDiameter, 0.12f, ringDiameter);
                    var cc = capRing.GetComponent<Collider>();
                    if (cc != null) { if (Application.isPlaying) Destroy(cc); else DestroyImmediate(cc); }
                    holdRingRenderer = capRing.GetComponent<MeshRenderer>();
                    holdRingRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("HoldRing_NEUTRAL", new Color(1f, 0.78f, 0.18f, 0.18f));
                }

                var bbGo = new GameObject("SpotLabel");
                bbGo.transform.SetParent(spotRoot.transform, false);
                bbGo.transform.localPosition = new Vector3(0f, 30f, 0f);
                var bb = bbGo.AddComponent<WorldBillboard>();
                var txtGo = new GameObject("Txt");
                txtGo.transform.SetParent(bbGo.transform, false);
                var tmp = txtGo.AddComponent<TextMeshPro>();
                tmp.richText = false;
                string safeLetter = MushProtocol.EscapeUnityRichText(sp.Letter ?? string.Empty);
                string safeSpotName = MushProtocol.EscapeUnityRichText((sp.Name ?? string.Empty).ToUpperInvariant());
                tmp.text = $"[{safeLetter}] {safeSpotName}";
                tmp.fontSize = 16f;
                tmp.color = col;
                tmp.alignment = TextAlignmentOptions.Center;
                tmp.textWrappingMode = TextWrappingModes.NoWrap;
                tmp.rectTransform.sizeDelta = new Vector2(104f, 8f);
                bb.NameplateText = tmp;

                _keySpotBeacons.Add(new TrackedKeySpotBeacon
                {
                    Letter = safeLetter,
                    DisplayName = safeSpotName,
                    CellX = sp.CellX,
                    CellY = sp.CellY,
                    IsHoldSpot = isHold,
                    WorldPos = worldPos,
                    Label = tmp,
                    PillarRenderer = pillarRenderer,
                    HoldRingRenderer = holdRingRenderer
                });
            }
        }

        private void BuildDynamicObjectiveBeacon()
        {
            if (_beaconsRoot == null) return;
            var objGo = new GameObject("DynamicObjectiveZone");
            objGo.transform.SetParent(_beaconsRoot, false);
            _dynamicObjectiveRoot = objGo.transform;

            var pillar = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            pillar.name = "ObjectivePillar";
            pillar.transform.SetParent(_dynamicObjectiveRoot, false);
            pillar.transform.localPosition = new Vector3(0f, 18f, 0f);
            pillar.transform.localScale = new Vector3(1.8f, 18f, 1.8f);
            var pc = pillar.GetComponent<Collider>();
            if (pc != null) { if (Application.isPlaying) Destroy(pc); else DestroyImmediate(pc); }
            _dynamicObjectivePillarRenderer = pillar.GetComponent<MeshRenderer>();
            _dynamicObjectivePillarRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("DynamicObjPillar_NEUTRAL", new Color(1.0f, 0.84f, 0.22f, 0.45f));

            var capRing = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            capRing.name = "ObjectivePerimeterRing";
            capRing.transform.SetParent(_dynamicObjectiveRoot, false);
            capRing.transform.localPosition = new Vector3(0f, 0.4f, 0f);
            capRing.transform.localScale = new Vector3(60f, 0.14f, 60f);
            var cc = capRing.GetComponent<Collider>();
            if (cc != null) { if (Application.isPlaying) Destroy(cc); else DestroyImmediate(cc); }
            _dynamicObjectiveRingRenderer = capRing.GetComponent<MeshRenderer>();
            _dynamicObjectiveRingRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit("DynamicObjRing_NEUTRAL", new Color(1.0f, 0.80f, 0.18f, 0.22f));
            _dynamicObjectiveRing = capRing.transform;

            var bbGo = new GameObject("ObjectiveLabel");
            bbGo.transform.SetParent(_dynamicObjectiveRoot, false);
            bbGo.transform.localPosition = new Vector3(0f, 38f, 0f);
            var bb = bbGo.AddComponent<WorldBillboard>();
            var txtGo = new GameObject("Txt");
            txtGo.transform.SetParent(bbGo.transform, false);
            var tmp = txtGo.AddComponent<TextMeshPro>();
            tmp.richText = false;
            tmp.text = "OBJECTIVE";
            tmp.fontSize = 18f;
            tmp.color = new Color(1f, 0.88f, 0.30f);
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.NoWrap;
            tmp.rectTransform.sizeDelta = new Vector2(96f, 10f);
            bb.NameplateText = tmp;
            _dynamicObjectiveLabel = tmp;

            objGo.SetActive(false);
        }

        private void BuildPlayerDustTrailVisual()
        {
            if (_vfxRoot == null) return;
            var root = new GameObject("PlayerTrackDustPlume");
            root.transform.SetParent(_vfxRoot, false);
            _playerDustRoot = root.transform;
            Material dustMat = UrpMaterialFactory.GetHologramUnlit("ArenaTrackDust", new Color(0.76f, 0.68f, 0.56f, 0.34f));

            for (int i = 0; i < _playerDustPuffs.Length; i++)
            {
                var puff = GameObject.CreatePrimitive(PrimitiveType.Cube);
                puff.name = $"DustPuff_{i}";
                puff.transform.SetParent(_playerDustRoot, false);
                var col = puff.GetComponent<Collider>();
                if (col != null) { if (Application.isPlaying) Destroy(col); else DestroyImmediate(col); }
                puff.GetComponent<MeshRenderer>().sharedMaterial = dustMat;
                _playerDustPuffs[i] = puff.transform;
            }
            root.SetActive(false);
        }

        private void BuildPlayerMuzzleFlashVisual()
        {
            if (_vfxRoot == null) return;
            var go = GameObject.CreatePrimitive(PrimitiveType.Sphere);
            go.name = "PlayerMuzzleFlash";
            go.transform.SetParent(_vfxRoot, false);
            go.transform.localScale = new Vector3(1.5f, 1.5f, 2.2f);
            var col = go.GetComponent<Collider>();
            if (col != null) { if (Application.isPlaying) Destroy(col); else DestroyImmediate(col); }
            _playerMuzzleFlashRenderer = go.GetComponent<MeshRenderer>();
            _playerMuzzleFlashRenderer.sharedMaterial = UrpMaterialFactory.GetEmissive(
                "ArenaMuzzleBallistic",
                new Color(1.0f, 0.88f, 0.38f),
                new Color(1.0f, 0.82f, 0.25f) * 3.2f);
            go.SetActive(false);
            _playerMuzzleFlash = go.transform;
        }

        public bool TryGetContactRig(string designator, out SpawnedGroundVehicleRig rig)
        {
            rig = null;
            if (string.IsNullOrWhiteSpace(designator)) return false;
            return _contactRigs.TryGetValue(designator.Trim(), out rig) && rig?.Root != null;
        }

        public static Quaternion ComputeArenaChassisRotation(
            DecodedTerrainGrid grid,
            Vector3 localPos,
            float headingDeg,
            bool isBiped,
            out float slopeTiltDeg)
        {
            slopeTiltDeg = 0f;
            if (grid == null)
                return Quaternion.Euler(0f, headingDeg, 0f);

            float rad = headingDeg * Mathf.Deg2Rad;
            Vector3 fwd = new Vector3(Mathf.Sin(rad), 0f, Mathf.Cos(rad));
            Vector3 right = new Vector3(Mathf.Cos(rad), 0f, -Mathf.Sin(rad));
            const float ForeAftSample = 2.6f;
            const float SideSample = 1.9f;

            float yFwd = grid.SampleElevationMeters(localPos.x + fwd.x * ForeAftSample, localPos.z + fwd.z * ForeAftSample);
            float yAft = grid.SampleElevationMeters(localPos.x - fwd.x * ForeAftSample, localPos.z - fwd.z * ForeAftSample);
            float yRight = grid.SampleElevationMeters(localPos.x + right.x * SideSample, localPos.z + right.z * SideSample);
            float yLeft = grid.SampleElevationMeters(localPos.x - right.x * SideSample, localPos.z - right.z * SideSample);

            float scale = isBiped ? 0.32f : 1.0f;
            float maxAngle = isBiped ? 10f : 26f;
            float pitchDeg = Mathf.Clamp(-Mathf.Atan2(yFwd - yAft, ForeAftSample * 2f) * Mathf.Rad2Deg * scale, -maxAngle, maxAngle);
            float rollDeg = Mathf.Clamp(Mathf.Atan2(yRight - yLeft, SideSample * 2f) * Mathf.Rad2Deg * scale, -maxAngle, maxAngle);
            slopeTiltDeg = Mathf.Sqrt(pitchDeg * pitchDeg + rollDeg * rollDeg);
            return Quaternion.Euler(pitchDeg, headingDeg, rollDeg);
        }

        public void ApplyLiveArenaMap(JsonNode mapData)
        {
            if (mapData == null || !mapData.IsObject) return;
            bool allowSpectatorMap = IsReplayModeActive || IsLiveObserverActive;
            if (!mapData.Bool("seated", true) && !allowSpectatorMap) return;
            var decoded = TacSimTerrainDecoder.DecodeArenaMapFrame(mapData);
            if (decoded == null) return;
            CachedMapMatchId = (ulong)Math.Max(0L, mapData.Long("match", 0));
            BuildArenaTerrain(Catalog, decoded);
            if (IsArenaActive)
            {
                EnsurePlayerRigSpawned(State.UnitType, State.Callsign, State.Party);
            }
            else if (IsSpectatorTheaterActive && ActiveReplayDoc != null)
            {
                SyncReplay3DRigs(immediate: true);
            }
            string safeDisplay = MushProtocol.EscapeUnityRichText(decoded.DisplayName);
            OnArenaCombatLog?.Invoke($"<color=#66EEAA>[ARENA MAP]</color> Loaded {decoded.Width}x{decoded.Height} heightfield ({safeDisplay}, {decoded.KeySpots.Count} key spots).");
        }

        public void HandleReplayPanelFrame(JsonNode data)
        {
            if (data == null || !data.IsObject) return;
            // A pilot actively seated in a live arena pod stays in their cockpit.
            if (IsArenaActive) return;

            if (TacSimArenaReplayModel.IsReplayUnavailable(data))
            {
                if (IsReplayModeActive)
                    CloseReplayOrObserver();
                return;
            }

            if (TacSimArenaReplayModel.TryDecodeReplayGridTrigger(data, out ulong trigMatch, out int kfIndex))
            {
                bool sameMatch = IsReplayModeActive && ActiveSpectatorMatchId == trigMatch;
                IsLiveObserverActive = false;
                ActiveObserverPicture = null;
                IsReplayModeActive = true;
                ActiveSpectatorMatchId = trigMatch;
                _pendingOpenKeyframeIndex = kfIndex;
                if (TerrainGrid == null)
                    BuildArenaTerrain(Catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));
                if (!sameMatch)
                {
                    ActiveReplayDoc = null;
                    LatestReplayFrame = null;
                    ReplayCursorMs = 0f;
                    ReplayFocusUnitIndex = 0;
                    ReplayPlaying = false;
                    ClearReplayRigs();
                }
                OnRequestReplayDocumentSubscribe?.Invoke();
                if (trigMatch != 0 && trigMatch != CachedMapMatchId)
                    OnRequestArenaMapSubscribe?.Invoke();
                OnReplayStateChanged?.Invoke();
                return;
            }

            if (TacSimArenaReplayModel.TryDecodeReplayDocument(data, out var doc))
            {
                IsLiveObserverActive = false;
                ActiveObserverPicture = null;
                IsReplayModeActive = true;
                ActiveSpectatorMatchId = doc.MatchId;
                ActiveReplayDoc = doc;
                if (TerrainGrid == null)
                    BuildArenaTerrain(Catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));
                if (doc.MatchId != 0 && doc.MatchId != CachedMapMatchId)
                    OnRequestArenaMapSubscribe?.Invoke();

                if (_pendingOpenKeyframeIndex >= 0 && doc.Keyframes.Count > 0)
                {
                    int clampedKf = Mathf.Clamp(_pendingOpenKeyframeIndex, 0, doc.Keyframes.Count - 1);
                    ReplayCursorMs = doc.Keyframes[clampedKf].TimeMs;
                }
                else
                {
                    ReplayCursorMs = Mathf.Clamp(ReplayCursorMs, 0f, doc.EndMs);
                }
                _pendingOpenKeyframeIndex = -1;
                if (ReplayFocusUnitIndex >= doc.Roster.Count)
                    ReplayFocusUnitIndex = 0;

                ReplayPlaying = doc.EndMs > 0 && ReplayCursorMs < doc.EndMs;
                ClearReplayRigs();
                SyncReplay3DRigs(immediate: true);
                OnReplayStateChanged?.Invoke();
            }
        }

        public void HandleObserverPanelFrame(JsonNode data)
        {
            if (data == null || !data.IsObject) return;
            if (IsArenaActive) return;

            if (TacSimArenaReplayModel.IsObserverCleared(data))
            {
                if (IsLiveObserverActive)
                    CloseReplayOrObserver();
                return;
            }

            if (!TacSimArenaReplayModel.TryDecodeObserverPicture(data, out var pic))
                return;

            bool matchChanged = !IsLiveObserverActive || ActiveSpectatorMatchId != pic.MatchId;
            IsReplayModeActive = false;
            IsLiveObserverActive = true;
            ReplayPlaying = false;
            ActiveSpectatorMatchId = pic.MatchId;
            ActiveObserverPicture = pic;
            ActiveReplayDoc = TacSimArenaReplayModel.FromObserverPicture(pic);
            ReplayCursorMs = pic.NowMs;
            if (ReplayFocusUnitIndex >= pic.Units.Count)
                ReplayFocusUnitIndex = 0;

            if (TerrainGrid == null)
                BuildArenaTerrain(Catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));
            if (matchChanged)
            {
                ClearReplayRigs();
                if (pic.MatchId != 0 && pic.MatchId != CachedMapMatchId)
                    OnRequestArenaMapSubscribe?.Invoke();
            }

            SyncReplay3DRigs(immediate: matchChanged);
            OnReplayStateChanged?.Invoke();
        }

        public void SeekReplayMs(float targetMs)
        {
            if (!IsReplayModeActive || ActiveReplayDoc == null) return;
            ReplayCursorMs = Mathf.Clamp(targetMs, 0f, ActiveReplayDoc.EndMs);
            SyncReplay3DRigs(immediate: true);
            OnReplayStateChanged?.Invoke();
        }

        public void ToggleReplayPlaying()
        {
            if (!IsReplayModeActive || ActiveReplayDoc == null) return;
            if (!ReplayPlaying && ReplayCursorMs >= ActiveReplayDoc.EndMs - 5f && ActiveReplayDoc.EndMs > 0)
                ReplayCursorMs = 0f;
            ReplayPlaying = !ReplayPlaying;
            OnReplayStateChanged?.Invoke();
        }

        public void SetReplaySpeed(float speed)
        {
            ReplayPlaybackSpeed = Mathf.Clamp(speed, 0.25f, 8f);
            OnReplayStateChanged?.Invoke();
        }

        public void StepReplayKeyframe(int delta)
        {
            if (!IsReplayModeActive || ActiveReplayDoc == null || ActiveReplayDoc.Keyframes.Count == 0) return;
            ReplayPlaying = false;
            float cur = ReplayCursorMs;
            long target = (long)cur;
            if (delta < 0)
            {
                for (int i = ActiveReplayDoc.Keyframes.Count - 1; i >= 0; i--)
                {
                    if (ActiveReplayDoc.Keyframes[i].TimeMs < cur - 1f)
                    {
                        target = ActiveReplayDoc.Keyframes[i].TimeMs;
                        break;
                    }
                }
            }
            else
            {
                for (int i = 0; i < ActiveReplayDoc.Keyframes.Count; i++)
                {
                    if (ActiveReplayDoc.Keyframes[i].TimeMs > cur + 1f)
                    {
                        target = ActiveReplayDoc.Keyframes[i].TimeMs;
                        break;
                    }
                }
            }
            SeekReplayMs(target);
        }

        public void SelectReplayFocusUnit(int rosterIndex)
        {
            int n = ActiveReplayDoc != null ? ActiveReplayDoc.Roster.Count : 0;
            if (n <= 0) return;
            ReplayFocusUnitIndex = ((rosterIndex % n) + n) % n;
            OnReplayStateChanged?.Invoke();
        }

        public void CycleReplayFocusUnit(int delta = 1)
        {
            SelectReplayFocusUnit(ReplayFocusUnitIndex + delta);
        }

        /// <summary>
        /// Jumps the replay cursor to the next (`delta &gt; 0`) or previous (`delta &lt; 0`) combat highlight
        /// (any wreck or hit shot in `ActiveReplayDoc`) and pauses on that timestamp.
        /// </summary>
        public bool StepReplayToEvent(int delta)
        {
            if (!IsReplayModeActive || ActiveReplayDoc == null) return false;
            var eventTimes = new List<long>(ActiveReplayDoc.Destroyed.Count + ActiveReplayDoc.Shots.Count);
            for (int i = 0; i < ActiveReplayDoc.Destroyed.Count; i++)
                eventTimes.Add(ActiveReplayDoc.Destroyed[i].TimeMs);
            for (int i = 0; i < ActiveReplayDoc.Shots.Count; i++)
            {
                if (string.Equals(ActiveReplayDoc.Shots[i].Outcome, "hit", StringComparison.OrdinalIgnoreCase))
                    eventTimes.Add(ActiveReplayDoc.Shots[i].TimeMs);
            }
            if (eventTimes.Count == 0) return false;
            eventTimes.Sort();

            float cur = ReplayCursorMs;
            long? target = null;
            if (delta >= 0)
            {
                for (int i = 0; i < eventTimes.Count; i++)
                {
                    if (eventTimes[i] > cur + 1f) { target = eventTimes[i]; break; }
                }
            }
            else
            {
                for (int i = eventTimes.Count - 1; i >= 0; i--)
                {
                    if (eventTimes[i] < cur - 1f) { target = eventTimes[i]; break; }
                }
            }
            if (!target.HasValue) return false;
            ReplayPlaying = false;
            SeekReplayMs(target.Value);
            return true;
        }

        public void CloseReplayOrObserver()
        {
            if (!IsReplayModeActive && !IsLiveObserverActive) return;
            IsReplayModeActive = false;
            IsLiveObserverActive = false;
            ReplayPlaying = false;
            ActiveReplayDoc = null;
            ActiveObserverPicture = null;
            LatestReplayFrame = null;
            ActiveSpectatorMatchId = 0;
            ReplayCursorMs = 0f;
            _pendingOpenKeyframeIndex = -1;
            ClearReplayRigs();
            if (_lockedTargetGroundRing != null)
                _lockedTargetGroundRing.enabled = false;
            OnReplayStateChanged?.Invoke();
            OnReplayClosed?.Invoke();
        }

        private void ClearReplayRigs()
        {
            foreach (var kv in _replayRigs)
            {
                if (kv.Value?.Root != null)
                {
                    if (Application.isPlaying) Destroy(kv.Value.Root);
                    else DestroyImmediate(kv.Value.Root);
                }
            }
            _replayRigs.Clear();
        }

        private Vector3 ReplayCellToWorldPos(float cellX, float cellY)
        {
            if (TerrainGrid == null) return ArenaStageOrigin;
            float siteCellM = TerrainGrid.CellSizeM / Math.Max(1, TerrainGrid.SpanCells);
            float lx = (cellX - TerrainGrid.OriginCellX + 0.5f) * siteCellM;
            float lz = (cellY - TerrainGrid.OriginCellY + 0.5f) * siteCellM;
            float maxX = TerrainGrid.Width * TerrainGrid.CellSizeM;
            float maxZ = TerrainGrid.Height * TerrainGrid.CellSizeM;
            lx = Mathf.Clamp(lx, 0f, maxX);
            lz = Mathf.Clamp(lz, 0f, maxZ);
            float ly = TerrainGrid.SampleElevationMeters(lx, lz);
            return ArenaStageOrigin + new Vector3(lx, ly, lz);
        }

        private void SyncReplay3DRigs(bool immediate, float dt = 0.016f)
        {
            if (ActiveReplayDoc == null || _unitsRoot == null || TerrainGrid == null) return;
            LatestReplayFrame = TacSimArenaReplayModel.SampleInterpolatedFrame(ActiveReplayDoc, ReplayCursorMs);

            if (PlayerRig?.Root != null && PlayerRig.Root.activeSelf)
                PlayerRig.Root.SetActive(false);

            int n = ActiveReplayDoc.Roster.Count;
            for (int i = 0; i < n; i++)
            {
                var entry = ActiveReplayDoc.Roster[i];
                ArenaReplayUnitPose? pose = i < LatestReplayFrame.Units.Length ? LatestReplayFrame.Units[i] : null;
                bool wrecked = i < LatestReplayFrame.IsWrecked.Length && LatestReplayFrame.IsWrecked[i];

                if (!pose.HasValue)
                {
                    if (_replayRigs.TryGetValue(i, out var hidden) && hidden?.Root != null && hidden.Root.activeSelf)
                        hidden.Root.SetActive(false);
                    continue;
                }

                Vector3 worldPos = ReplayCellToWorldPos(pose.Value.CellX, pose.Value.CellY);
                Vector3 localPos = worldPos - ArenaStageOrigin;
                bool isBipedGuess = false;
                if (!_replayRigs.TryGetValue(i, out var rig) || rig?.Root == null)
                {
                    string safeCs = MushProtocol.EscapeUnityRichText(entry.Callsign);
                    rig = TacSimVehicleVisualFactory.SpawnArenaCombatUnit(Catalog, _unitsRoot, "line_tank", safeCs, entry.PartyCode, worldPos, pose.Value.HeadingDeg);
                    rig.Root.name = $"ReplayUnit_{i}_{entry.Callsign}";
                    _replayRigs[i] = rig;
                    isBipedGuess = rig.IsBiped;
                    rig.Root.transform.rotation = ComputeArenaChassisRotation(TerrainGrid, localPos, pose.Value.HeadingDeg, isBipedGuess, out _);
                }
                else
                {
                    if (!rig.Root.activeSelf) rig.Root.SetActive(true);
                    isBipedGuess = rig.IsBiped;
                    Vector3 prevPos = rig.Root.transform.position;
                    float blend = immediate ? 1f : (1f - Mathf.Exp(-14f * dt));
                    Quaternion slopeRot = ComputeArenaChassisRotation(TerrainGrid, localPos, pose.Value.HeadingDeg, isBipedGuess, out _);
                    rig.Root.transform.position = Vector3.Lerp(prevPos, worldPos, blend);
                    rig.Root.transform.rotation = Quaternion.Slerp(rig.Root.transform.rotation, slopeRot, blend);
                    float movedKph = !immediate && dt > 0.0001f ? (Vector3.Distance(prevPos, rig.Root.transform.position) / dt) * 3.6f : 0f;
                    rig.UpdateArticulation(dt, wrecked ? 0f : movedKph, null);
                }

                rig.SetWrecked(wrecked);
                if (rig.Billboard?.NameplateText != null)
                {
                    string focusMark = i == ReplayFocusUnitIndex ? "<color=#FFEE44>[CAM]</color> " : string.Empty;
                    string wreckTag = wrecked ? " <color=#FF6666>[WRECK]</color>" : string.Empty;
                    rig.Billboard.NameplateText.text = $"{focusMark}{MushProtocol.EscapeUnityRichText(entry.Callsign)}{wreckTag}";
                }
            }

            if (_lockedTargetGroundRing != null)
            {
                if (_replayRigs.TryGetValue(ReplayFocusUnitIndex, out var focRig) && focRig?.Root != null && focRig.Root.activeSelf)
                {
                    Vector3 fPos = focRig.Root.transform.position;
                    const int Segs = 28;
                    float rad = 5.4f;
                    _lockedTargetGroundRing.positionCount = Segs;
                    for (int s = 0; s < Segs; s++)
                    {
                        float ang = (s / (float)Segs) * Mathf.PI * 2f;
                        float wx = fPos.x + Mathf.Cos(ang) * rad;
                        float wz = fPos.z + Mathf.Sin(ang) * rad;
                        float wy = ArenaStageOrigin.y + TerrainGrid.SampleElevationMeters(wx - ArenaStageOrigin.x, wz - ArenaStageOrigin.z) + 0.55f;
                        _lockedTargetGroundRing.SetPosition(s, new Vector3(wx, wy, wz));
                    }
                    _lockedTargetGroundRing.enabled = true;
                }
                else
                {
                    _lockedTargetGroundRing.enabled = false;
                }
            }
        }

        public void ApplyLiveCockpitFrame(JsonNode d)
        {
            if (d == null || !d.IsObject) return;
            if (!IsLiveCockpitFrameSeated(d))
            {
                if (IsArenaActive)
                    ExitArena();
                return;
            }

            if (IsReplayModeActive || IsLiveObserverActive)
            {
                IsReplayModeActive = false;
                IsLiveObserverActive = false;
                ReplayPlaying = false;
                ActiveReplayDoc = null;
                ActiveObserverPicture = null;
                LatestReplayFrame = null;
                ClearReplayRigs();
                OnReplayStateChanged?.Invoke();
            }

            IsArenaActive = true;

            ulong prevMatchId = State.MatchId;
            ulong matchId = (ulong)Math.Max(0L, d.Long("match", 0));
            if (matchId != 0 && prevMatchId != 0 && matchId != prevMatchId)
            {
                State.MatchKills = 0;
                State.MatchHits = 0;
                State.MatchMisses = 0;
                State.RecentNotices.Clear();
            }
            State.MatchId = matchId;
            if (TerrainGrid == null)
            {
                BuildArenaTerrain(Catalog, TacSimTerrainDecoder.LoadStaticArena("proving_ground"));
            }
            if (matchId != 0 && matchId != CachedMapMatchId)
            {
                CachedMapMatchId = matchId;
                OnRequestArenaMapSubscribe?.Invoke();
            }

            string prevUnitType = State.UnitType;
            string prevParty = State.Party;

            State.Callsign = d.Str("callsign", State.Callsign);
            State.Party = d.Str("party", State.Party);
            State.UnitType = d.Str("unit_type", State.UnitType);
            State.Mode = d.Str("mode", State.Mode);
            State.Power = d.Str("power", "online");
            State.IsWreck = d.Bool("wreck", false);
            State.Immobile = d.Bool("immobile", false);
            State.MovementBlocked = d.Bool("blocked", false);
            if (d["odds_floor_permille"].Kind == JsonKind.Int || d["odds_floor_permille"].Kind == JsonKind.Float)
                State.OddsFloorPercent = Mathf.Clamp(d.Int("odds_floor_permille", 0) / 10, 0, 100);
            if (d["sight_m"].Kind == JsonKind.Int || d["sight_m"].Kind == JsonKind.Float)
                State.SightMeters = Mathf.Clamp(d.Float("sight_m", 120f), 20f, 1200f);
            if (d["sensor_height_cm"].Kind == JsonKind.Int || d["sensor_height_cm"].Kind == JsonKind.Float)
                State.SensorHeightM = Mathf.Clamp(d.Float("sensor_height_cm", 250f) * 0.01f, 1.2f, 8.0f);
            string rawScope = d.Str("scope", string.Empty);
            if (!string.IsNullOrEmpty(rawScope)) State.Scope = rawScope;
            string rawTempo = d.Str("tempo", string.Empty);
            if (!string.IsNullOrEmpty(rawTempo)) State.Tempo = rawTempo;
            string rawPosture = d.Str("posture", string.Empty);
            if (!string.IsNullOrEmpty(rawPosture)) State.Posture = rawPosture;

            var rcvrNode = d["receiver"];
            if (rcvrNode != null && rcvrNode.IsObject)
            {
                State.HasReceiver = true;
                State.ReceiverBudget = rcvrNode.Int("budget", 0);
                State.ReceiverBudgetMax = Mathf.Max(0, rcvrNode.Int("budget_max", 0));
                State.ReceiverReachPercent = Mathf.Clamp(rcvrNode.Int("reach_percent", 100), 0, 100);
            }
            else
            {
                State.HasReceiver = false;
                State.ReceiverReachPercent = 100;
            }

            var mindNode = d["mind"];
            if (mindNode != null && mindNode.IsObject)
            {
                var ms = new ArenaMindState
                {
                    Name = mindNode["name"].IsNull ? string.Empty : mindNode.Str("name", string.Empty),
                    ActiveIndex = Mathf.Max(0, mindNode.Int("active", 0)),
                    ClassWord = mindNode.Str("class", string.Empty),
                    CoreWord = mindNode.Str("core", string.Empty),
                    NextSwitchMs = (mindNode["next_switch_ms"].Kind == JsonKind.Int || mindNode["next_switch_ms"].Kind == JsonKind.Float)
                        ? mindNode.Long("next_switch_ms", 0)
                        : (long?)null,
                    IsOff = mindNode.Bool("off", false)
                };
                var cardObj = mindNode["card"];
                if (cardObj != null && cardObj.IsObject)
                    ms.CardSchema = cardObj.Str("schema", string.Empty);
                var srArr = mindNode["sealed_rules"].AsArray();
                if (srArr != null)
                {
                    int srCap = Math.Min(srArr.Count, 12);
                    for (int sIdx = 0; sIdx < srCap; sIdx++)
                    {
                        string sr = srArr[sIdx].AsString(string.Empty);
                        if (!string.IsNullOrEmpty(sr)) ms.SealedRules.Add(sr);
                    }
                }
                var profArr = mindNode["profiles"].AsArray();
                if (profArr != null)
                {
                    int pCap = Math.Min(profArr.Count, 12);
                    for (int p = 0; p < pCap; p++)
                    {
                        string pn = profArr[p].AsString(string.Empty);
                        if (!string.IsNullOrEmpty(pn)) ms.Profiles.Add(pn);
                    }
                }
                var chArr = mindNode["channels"].AsArray();
                if (chArr != null)
                {
                    int chCap = Math.Min(chArr.Count, 6);
                    for (int ch = 0; ch < chCap; ch++)
                    {
                        string cn = chArr[ch].AsString(string.Empty);
                        if (!string.IsNullOrEmpty(cn)) ms.Channels.Add(cn);
                    }
                }
                State.Mind = ms;
            }
            else if (d.ContainsKey("mind"))
            {
                State.Mind = null;
            }

            var ordNode = d["order"];
            if (ordNode != null && ordNode.IsObject)
            {
                var stOrd = new ArenaStandingOrderState
                {
                    Squad = ordNode.Str("squad", string.Empty),
                    Objective = ordNode.Str("objective", string.Empty),
                    Target = ordNode["target"].IsNull ? string.Empty : ordNode.Str("target", string.Empty),
                    HoldFire = ordNode.Bool("hold_fire", false),
                    ExpiresInMs = Math.Max(0L, ordNode.Long("expires_in_ms", 0))
                };
                var toArr = ordNode["to"].AsArray();
                if (toArr != null && toArr.Count >= 2)
                {
                    stOrd.HasToCell = true;
                    stOrd.ToCellX = toArr[0].AsInt(0);
                    stOrd.ToCellY = toArr[1].AsInt(0);
                }
                State.StandingOrder = stOrd;
            }
            else if (d.ContainsKey("order"))
            {
                State.StandingOrder = null;
            }

            var catchupNode = d["catchup"];
            if (catchupNode != null && catchupNode.IsObject)
            {
                long awayS = Math.Max(0L, catchupNode.Long("away_s", 0));
                int newC = Math.Max(0, catchupNode.Int("contacts_new", 0));
                int lostC = Math.Max(0, catchupNode.Int("contacts_lost", 0));
                var secsChanged = catchupNode["sections"].AsArray();
                int secCount = secsChanged != null ? secsChanged.Count : 0;
                State.HasReceivedCatchup = true;
                State.LastCatchupAwaySec = awayS;
                OnArenaCombatLog?.Invoke(
                    $"<color=#66DDFF>[CATCH-UP]</color> Reclaimed hull after {awayS}s away (+{newC}/-{lostC} contacts, {secCount} section updates).");
            }

            State.Rules.Clear();
            var rulesArr = d["rules"].AsArray();
            if (rulesArr != null)
            {
                int rCap = Math.Min(rulesArr.Count, 12);
                for (int i = 0; i < rCap; i++)
                {
                    var r = rulesArr[i];
                    if (r == null || !r.IsObject) continue;
                    int slot = r.Int("slot", 0);
                    string action = r.Str("action", string.Empty);
                    if (slot > 0 && !string.IsNullOrEmpty(action))
                    {
                        State.Rules.Add(new ArenaStandingRule
                        {
                            Slot = slot,
                            Action = action,
                            Weapon = r["weapon"].IsNull ? string.Empty : r.Str("weapon", string.Empty),
                            Target = r["target"].IsNull ? string.Empty : r.Str("target", string.Empty),
                            MinPercent = r.Int("percent", 0)
                        });
                    }
                }
            }

            var objNode = d["objective"];
            if (objNode != null && objNode.IsObject)
            {
                var oc = objNode["cell"].AsArray();
                if (oc != null && oc.Count >= 2)
                {
                    State.HasDynamicObjective = true;
                    State.ObjectiveCellX = oc[0].AsInt(128);
                    State.ObjectiveCellY = oc[1].AsInt(128);
                    State.ObjectiveLetter = objNode.Str("letter", string.Empty);
                    State.ObjectiveRadiusCells = Mathf.Clamp(objNode.Int("radius_cells", 6), 1, 64);
                }
            }
            else if (d.ContainsKey("objective") && objNode != null && objNode.IsNull)
            {
                State.HasDynamicObjective = false;
            }

            if (d["hold_score"].Kind == JsonKind.Int || d["hold_score"].Kind == JsonKind.Float)
                State.HoldControlScore = d.Int("hold_score", State.HoldControlScore);
            else if (d["hold_control_score"].Kind == JsonKind.Int || d["hold_control_score"].Kind == JsonKind.Float)
                State.HoldControlScore = d.Int("hold_control_score", State.HoldControlScore);

            // Live frames.rs:633 emits "ecm": "on" (radiating) | "off" (silent) | null (no pod); keep object fallback too.
            var ecmNode = d["ecm"];
            if (ecmNode.Kind == JsonKind.String)
            {
                State.HasEcm = true;
                State.EmconSilent = string.Equals(ecmNode.AsString(), "off", StringComparison.OrdinalIgnoreCase);
            }
            else if (ecmNode.IsObject)
            {
                State.HasEcm = true;
                State.EmconSilent = ecmNode.Bool("emcon", false);
            }
            else
            {
                State.HasEcm = false;
                State.EmconSilent = false;
            }

            State.Xm = d.Int("x_cm", (int)(State.Xm * 100f)) * 0.01f;
            State.Zm = d.Int("y_cm", (int)(State.Zm * 100f)) * 0.01f;
            int headingMdeg = d["heading_mdeg"].Kind == JsonKind.Int || d["heading_mdeg"].Kind == JsonKind.Float
                ? d.Int("heading_mdeg", 90_000)
                : Mathf.RoundToInt(d.Float("heading_deg", 90f) * 1000f);
            State.HeadingDeg = ArenaPredictiveHelm.Wrap360Mdeg(headingMdeg) * 0.001f;
            State.SpeedKph = d.Float("speed_kph", d.Int("speed_cms", 0) * 0.036f);
            State.MaxSpeedKph = Mathf.Max(10f, d.Float("max_speed_kph", 54f));
            State.ReverseSpeedKph = Mathf.Max(5f, d.Float("reverse_kph", 20f));

            long nowMs = CurrentHelmClockMs();
            _lastFrameAnchorMs = nowMs;
            long? frameTMs = (d["t_ms"].Kind == JsonKind.Int || d["t_ms"].Kind == JsonKind.Float)
                ? d.Long("t_ms", 0)
                : (long?)null;
            if (frameTMs.HasValue && frameTMs.Value >= 0L)
                State.ArenaClockMs = frameTMs.Value;
            int? turnRateMdegS = (d["turn_rate_mdeg_s"].Kind == JsonKind.Int || d["turn_rate_mdeg_s"].Kind == JsonKind.Float)
                ? d.Int("turn_rate_mdeg_s", 0)
                : (int?)null;
            bool canTurn = !State.IsWreck && !State.Immobile &&
                           !string.Equals(State.Power, "off", StringComparison.OrdinalIgnoreCase) &&
                           !string.Equals(State.Power, "offline", StringComparison.OrdinalIgnoreCase) &&
                           !string.Equals(State.Power, "starting", StringComparison.OrdinalIgnoreCase);
            Helm.Observe(new ArenaHelmFrame
            {
                TMs = frameTMs,
                HeadingMdeg = headingMdeg,
                Tempo = d.Str("tempo", string.Empty),
                TurnRateMdegS = turnRateMdegS,
                CanTurn = canTurn
            }, nowMs);

            Throttle.Observe(new ArenaThrottleFrame
            {
                SpeedKph = Mathf.RoundToInt(State.SpeedKph),
                MaxKph = (d["max_speed_kph"].Kind == JsonKind.Int || d["max_speed_kph"].Kind == JsonKind.Float)
                    ? Mathf.RoundToInt(State.MaxSpeedKph)
                    : (int?)null,
                ReverseKph = (d["reverse_kph"].Kind == JsonKind.Int || d["reverse_kph"].Kind == JsonKind.Float)
                    ? Mathf.RoundToInt(State.ReverseSpeedKph)
                    : (int?)null,
                Stuck = State.IsWreck || State.Immobile
            });

            var heatObj = d["heat"];
            if (heatObj.IsObject)
            {
                State.Heat = heatObj.Int("heat", 0);
                State.HeatMax = Mathf.Max(1, heatObj.Int("max", 1000));
                State.Overheated = heatObj.Bool("overheated", false);
            }

            ParseSectionsIntoState(d["sections"].AsArray());

            State.LockedDesignator = d.Str("lock_designator", string.Empty);
            State.LockRangeM = d.Float("lock_range_m", 0f);
            State.LockHitPercent = d.Int("lock_percent", 0);
            State.WeaponsFree = d.Bool("weapons_free", true);

            State.Weapons.Clear();
            var wArr = d["weapons"].AsArray();
            if (wArr != null)
            {
                int wCap = Math.Min(wArr.Count, 16);
                for (int i = 0; i < wCap; i++)
                {
                    var w = wArr[i];
                    var armedObj = w["armed"];
                    bool hasArmed = armedObj.IsObject;
                    var rbObj = w["range_band"];
                    bool hasRb = rbObj != null && rbObj.IsObject;
                    float maxRangeM = hasRb ? rbObj.Int("max_range_cm", 0) * 0.01f : 0f;
                    if (maxRangeM <= 0.1f)
                    {
                        var bandsArr = w["bands_cm"].AsArray();
                        if (bandsArr != null && bandsArr.Count > 0)
                            maxRangeM = bandsArr[bandsArr.Count - 1].AsInt(0) * 0.01f;
                    }
                    State.Weapons.Add(new ArenaWeaponSlotState
                    {
                        Id = w.Str("id", $"wpn_{i + 1}"),
                        Ready = w.Bool("ready", true),
                        ReadyInMs = w.Int("ready_in_ms", 0),
                        Rounds = w.Int("rounds", 0),
                        RoundsMax = w.Int("rounds_max", 0),
                        HitPercent = w.Int("percent", State.LockHitPercent),
                        HeatPerShot = w.Int("heat_per_shot", 0),
                        ArmedSlot = hasArmed ? armedObj.Int("slot", 1) : 0,
                        ArmedMinPercent = hasArmed ? armedObj.Int("percent", 0) : 0,
                        ArmedKeeps = hasArmed ? armedObj.Int("keeps", 0) : 0,
                        ArmedTarget = hasArmed ? armedObj.Str("target", string.Empty) : string.Empty,
                        RangeBandIndex = hasRb ? rbObj.Int("index", 0) : 0,
                        RangeBandTotal = hasRb ? rbObj.Int("of", 0) : 0,
                        MaxRangeM = maxRangeM,
                        DispersionMrad = hasRb ? rbObj.Int("dispersion_mrad", 0) : 0
                    });
                }
            }

            State.ScanTables.Clear();
            var stArr = d["scan_tables"].AsArray();
            if (stArr != null)
            {
                int tCap = Math.Min(stArr.Count, 16);
                for (int t = 0; t < tCap; t++)
                {
                    var rowArr = stArr[t].AsArray();
                    var table = new List<string>(8);
                    if (rowArr != null)
                    {
                        int sCap = Math.Min(rowArr.Count, 24);
                        for (int sIdx = 0; sIdx < sCap; sIdx++)
                        {
                            string secName = rowArr[sIdx].AsString(string.Empty);
                            if (!string.IsNullOrEmpty(secName))
                                table.Add(secName);
                        }
                    }
                    State.ScanTables.Add(table);
                }
            }

            var motorNode = d["motor"];
            if (motorNode != null && motorNode.IsObject)
            {
                State.MotorGoal = motorNode.Str("goal", string.Empty);
                State.MotorOutcome = motorNode["outcome"].IsNull ? string.Empty : motorNode.Str("outcome", string.Empty);
                if (!string.IsNullOrEmpty(State.MotorOutcome))
                    State.HasDriveWaypoint = false;
            }
            else
            {
                if (!string.IsNullOrEmpty(State.MotorGoal))
                    State.HasDriveWaypoint = false;
                State.MotorGoal = string.Empty;
                State.MotorOutcome = string.Empty;
            }

            State.Contacts.Clear();
            var activeDesignators = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var cArr = d["contacts"].AsArray();
            int redVisibleAlive = State.IsWreck || State.Party != "R" ? 0 : 1;
            int blueVisibleAlive = State.IsWreck || State.Party != "B" ? 0 : 1;

            if (cArr != null)
            {
                const int MaxArenaContacts = 64;
                int cCap = Math.Min(cArr.Count, MaxArenaContacts);
                for (int i = 0; i < cCap; i++)
                {
                    var c = cArr[i];
                    var cellArr = c["cell"].AsArray();
                    if (cellArr == null || cellArr.Count < 2) continue;

                    string rawDesig = c.Str("designator", $"C{i + 1}");
                    string desig = MushProtocol.TryValidateSafeCommandToken(rawDesig, out string safeDesig) ? safeDesig : $"C{i + 1}";
                    activeDesignators.Add(desig);
                    float bearingDeg = c.Float("bearing_deg", 0f);
                    var cs = new ArenaContactState
                    {
                        Designator = desig,
                        ClassLabel = c.Str("class", "tank"),
                        UnitType = c.Str("unit_type", c.Str("class", "line_tank")),
                        Party = c.Str("party", c.Bool("friendly", false) ? State.Party : (State.Party == "R" ? "B" : "R")),
                        Relation = c.Str("relation", c.Bool("friendly", false) ? "friendly" : "hostile"),
                        Friendly = c.Bool("friendly", false),
                        CellX = cellArr[0].AsInt(128),
                        CellY = cellArr[1].AsInt(128),
                        RangeM = c.Float("range_m", 0f),
                        BearingDeg = bearingDeg,
                        FacingDeg = ParseFacingWordOrDeg(c["facing"], bearingDeg),
                        Aspect = c.Str("aspect", "front"),
                        Band = c.Str("band", string.Empty),
                        Moving = c.Bool("moving", false),
                        AgeMs = Mathf.Max(0, c.Int("age_ms", 0)),
                        HealthWord = c.Str("health", "intact"),
                        IsWreck = c.Bool("reads_dead", false) || c.Str("health") == "wreck",
                        HitOddsPct = ParseContactHitOddsPct(c["odds"])
                    };
                    var condNode = c["condition"];
                    if (condNode != null && condNode.IsObject)
                    {
                        cs.ConditionTable = condNode.Int("table", -1);
                        cs.ConditionBands = condNode.Str("bands", string.Empty);
                        cs.ConditionAgeMs = Mathf.Max(0, condNode.Int("age_ms", 0));
                    }
                    var fatalArr = c["fatal"].AsArray();
                    if (fatalArr != null)
                    {
                        int fCap = Math.Min(fatalArr.Count, 8);
                        for (int f = 0; f < fCap; f++)
                        {
                            string fs = fatalArr[f].AsString(string.Empty);
                            if (!string.IsNullOrEmpty(fs))
                                cs.FatalSections.Add(fs);
                        }
                    }
                    if (!cs.IsWreck)
                    {
                        if (cs.Party == "R") redVisibleAlive++;
                        else if (cs.Party == "B") blueVisibleAlive++;
                    }
                    State.Contacts.Add(cs);
                    SyncLiveContact3DRig(cs);
                }
            }
            State.RedAliveCount = redVisibleAlive;
            State.BlueAliveCount = blueVisibleAlive;

            var stale = new List<string>();
            foreach (var kv in _contactRigs)
            {
                if (!activeDesignators.Contains(kv.Key))
                {
                    if (kv.Value?.Root != null)
                    {
                        if (Application.isPlaying) Destroy(kv.Value.Root);
                        else DestroyImmediate(kv.Value.Root);
                    }
                    stale.Add(kv.Key);
                }
            }
            for (int i = 0; i < stale.Count; i++) _contactRigs.Remove(stale[i]);

            if (PlayerRig == null || prevUnitType != State.UnitType || prevParty != State.Party)
                EnsurePlayerRigSpawned(State.UnitType, State.Callsign, State.Party);

            UpdateOwnPlayerRigPose(0.1f);
            DetectAndEmitPlayerArmorDamage();

            // Strict server-projected detachment roster & orders (`crates/tacsim/src/detachment/live.rs`)
            var detNode = d["detachment"];
            if (detNode != null && !detNode.IsNull)
            {
                if (ArenaDetachmentState.TryDecode(detNode, out var det))
                {
                    State.Detachment = det;
                    State.DetachmentProjectionInvalid = false;
                    if (det.MyOrder != null && string.IsNullOrEmpty(det.MyOrder.Response))
                    {
                        string ordKey = $"{det.Id}:{det.Revision}:{det.MyOrder.Sequence}";
                        if (!string.Equals(ordKey, _lastNotifiedDetachmentOrderKey, StringComparison.Ordinal))
                        {
                            _lastNotifiedDetachmentOrderKey = ordKey;
                            OnNewDetachmentOrderReceived?.Invoke(det.MyOrder);
                        }
                    }
                    else if (det.MyOrder == null)
                    {
                        _lastNotifiedDetachmentOrderKey = string.Empty;
                    }
                }
                else
                {
                    State.Detachment = null;
                    State.DetachmentProjectionInvalid = true;
                }
            }
            else
            {
                State.Detachment = null;
                State.DetachmentProjectionInvalid = false;
                _lastNotifiedDetachmentOrderKey = string.Empty;
            }

            var notices = d["notices"].AsArray();
            if (notices != null)
            {
                int nCap = Math.Min(notices.Count, 32);
                for (int i = 0; i < nCap; i++)
                {
                    var n = notices[i];
                    string txt = n.Str("text", string.Empty);
                    string kind = n.Str("kind", string.Empty).ToLowerInvariant();
                    string desig = n["designator"].IsNull ? string.Empty : n.Str("designator", string.Empty);
                    string was = n["was"].IsNull ? string.Empty : n.Str("was", string.Empty);
                    string section = n["section"].IsNull ? string.Empty : n.Str("section", string.Empty);
                    string weapon = n["weapon"].IsNull ? string.Empty : n.Str("weapon", string.Empty);
                    string order = n["order"].IsNull ? string.Empty : n.Str("order", string.Empty);
                    string cause = n["cause"].IsNull ? string.Empty : n.Str("cause", string.Empty);
                    string refusal = n["refusal"].IsNull ? string.Empty : n.Str("refusal", string.Empty);
                    string aspect = n["aspect"].IsNull ? string.Empty : n.Str("aspect", string.Empty);

                    float detailRadiusM = 0f;
                    float detailSecs = 0f;
                    int detailFloorPct = 0;
                    var detObj = n["detail"];
                    if (detObj != null && detObj.IsObject)
                    {
                        detailRadiusM = detObj.Float("radius_m", 0f);
                        detailSecs = detObj.Float("secs", 0f);
                        int floorPm = detObj.Int("floor_permille", 0);
                        if (floorPm > 0) detailFloorPct = floorPm / 10;
                    }

                    var parsedNotice = new ArenaCombatNotice
                    {
                        Kind = kind,
                        Designator = desig,
                        Was = was,
                        Section = section,
                        Weapon = weapon,
                        Order = order,
                        Cause = cause,
                        Refusal = refusal,
                        Aspect = aspect,
                        Text = txt,
                        Spent = n.Bool("spent", false),
                        Friendly = n.Bool("friendly", false),
                        Slot = n.Int("slot", 0),
                        HitPercent = n.Int("percent", 0),
                        DetailRadiusM = detailRadiusM,
                        DetailSecs = detailSecs,
                        DetailFloorPercent = detailFloorPct
                    };

                    if (State.RecentNotices.Count >= 10)
                        State.RecentNotices.RemoveAt(0);
                    State.RecentNotices.Add(parsedNotice);

                    if ((kind == "emcon_on" || kind == "emcon_off") && detailRadiusM > 1f)
                    {
                        State.EcmRadiusMeters = Mathf.Clamp(detailRadiusM, 20f, 500f);
                        SyncArenaTacticalVisuals();
                    }

                    if (kind == "hit") State.MatchHits++;
                    else if (kind == "miss") State.MatchMisses++;
                    else if (kind == "kill") State.MatchKills++;
                    else if (kind == "drive_arrived" || kind == "drive_no_route")
                    {
                        State.HasDriveWaypoint = false;
                        SyncArenaTacticalVisuals();
                    }

                    if (!string.IsNullOrEmpty(txt))
                        OnArenaCombatLog?.Invoke($"<color=#FFCC66>[COCKPIT]</color> {TacSim.Net.MushProtocol.EscapeUnityRichText(txt)}");

                    if (kind == "hit" || kind == "miss" || kind == "kill" || kind == "slot_fired" ||
                        kind == "target_section_lost" || kind.Contains("fire") || kind.Contains("shot"))
                    {
                        bool isLaser = weapon.IndexOf("laser", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                       txt.IndexOf("laser", StringComparison.OrdinalIgnoreCase) >= 0;
                        string tgtDesig = !string.IsNullOrEmpty(desig) ? desig : State.LockedDesignator;
                        if (kind != "target_section_lost")
                        {
                            OnWeaponShotFired?.Invoke(isLaser);
                            TriggerPlayerMuzzleFlash(isLaser);
                        }
                        if (_contactRigs.TryGetValue(tgtDesig, out var tgtRig) && tgtRig?.Root != null)
                        {
                            Vector3 tgtPt = tgtRig.Root.transform.position + Vector3.up * 1.6f;
                            string secUp = (section ?? string.Empty).Replace("_", " ").Trim().ToUpperInvariant();
                            if (kind == "hit" || kind == "target_section_lost" || kind == "kill")
                                tgtRig.TriggerHitFlash(isCrit: kind == "target_section_lost" || kind == "kill");
                            if (kind == "kill")
                            {
                                SpawnImpactBurst(tgtPt, ArenaImpactKind.Kill);
                                SpawnWorldCombatPopup(tgtPt + Vector3.up * 4.8f, $"DESTROYED! {tgtDesig}", new Color(1.0f, 0.28f, 0.20f), 1.35f);
                            }
                            else if (kind == "target_section_lost")
                            {
                                SpawnImpactBurst(tgtPt, ArenaImpactKind.SectionCrit);
                                string critLbl = !string.IsNullOrEmpty(secUp) ? $"CRIT! {secUp}" : "CRIT!";
                                SpawnWorldCombatPopup(tgtPt + Vector3.up * 4.2f, critLbl, new Color(1.0f, 0.78f, 0.22f), 1.18f);
                            }
                            else if (kind == "hit")
                            {
                                SpawnImpactBurst(tgtPt, ArenaImpactKind.Hit);
                                string hitLbl = !string.IsNullOrEmpty(secUp) ? $"HIT! {secUp}" : "HIT!";
                                SpawnWorldCombatPopup(tgtPt + Vector3.up * 3.6f, hitLbl, new Color(0.50f, 1.0f, 0.72f), 1.0f);
                            }
                            else if (kind == "miss")
                            {
                                Vector3 missPt = tgtPt + tgtRig.Root.transform.right * 6.5f;
                                SpawnImpactBurst(missPt, ArenaImpactKind.Miss);
                                SpawnWorldCombatPopup(missPt + Vector3.up * 2.2f, "MISS", new Color(0.68f, 0.84f, 0.94f), 0.85f);
                            }
                            if (kind != "target_section_lost" && PlayerRig?.MuzzlePoint != null)
                            {
                                Vector3 tracerEnd = kind == "miss" ? (tgtPt + tgtRig.Root.transform.right * 6.5f) : tgtPt;
                                SpawnShotTracer(PlayerRig.MuzzlePoint.position, tracerEnd, isLaser);
                            }
                        }
                    }
                    else if ((kind == "struck" || kind == "struck_spent" || kind == "section_lost" || kind == "destroyed") &&
                             PlayerRig?.Root != null)
                    {
                        Vector3 ownPt = PlayerRig.Root.transform.position + Vector3.up * 1.8f;
                        string secUp = (section ?? string.Empty).Replace("_", " ").Trim().ToUpperInvariant();
                        if (kind == "destroyed")
                        {
                            SpawnImpactBurst(ownPt, ArenaImpactKind.Kill);
                            SpawnWorldCombatPopup(ownPt + Vector3.up * 4.8f, "HULL DESTROYED!", new Color(1.0f, 0.22f, 0.18f), 1.35f);
                        }
                        else if (kind == "section_lost")
                        {
                            SpawnImpactBurst(ownPt, ArenaImpactKind.SectionCrit);
                            SpawnWorldCombatPopup(ownPt + Vector3.up * 4.2f, !string.IsNullOrEmpty(secUp) ? $"LOST {secUp}!" : "SECTION LOST!", new Color(1.0f, 0.62f, 0.18f), 1.18f);
                        }
                        else
                        {
                            SpawnImpactBurst(ownPt, ArenaImpactKind.Hit);
                            SpawnWorldCombatPopup(ownPt + Vector3.up * 3.5f, !string.IsNullOrEmpty(secUp) ? $"{secUp} HIT" : "HULL HIT", new Color(1.0f, 0.56f, 0.24f), 1.0f);
                        }
                    }

                    OnArenaNoticeReceived?.Invoke(parsedNotice);
                }
            }

            if (State.Power == "offline")
            {
                OnLiveCockpitVerbRequested?.Invoke("startup");
            }

            OnArenaStateUpdated?.Invoke();
        }

        private void DetectAndEmitPlayerArmorDamage()
        {
            if (State == null) return;
            if (!_hasPrevArmorSnapshot || _prevArmorMatchId != State.MatchId)
            {
                _prevArmorMatchId = State.MatchId;
                _prevArmorFront = State.ArmorFrontPct;
                _prevArmorLeft = State.ArmorLeftPct;
                _prevArmorRight = State.ArmorRightPct;
                _prevArmorRear = State.ArmorRearPct;
                _prevInternals = State.InternalsPct;
                _hasPrevArmorSnapshot = true;
                return;
            }

            int dFront = _prevArmorFront - State.ArmorFrontPct;
            int dLeft = _prevArmorLeft - State.ArmorLeftPct;
            int dRight = _prevArmorRight - State.ArmorRightPct;
            int dRear = _prevArmorRear - State.ArmorRearPct;
            int dInt = _prevInternals - State.InternalsPct;

            _prevArmorFront = State.ArmorFrontPct;
            _prevArmorLeft = State.ArmorLeftPct;
            _prevArmorRight = State.ArmorRightPct;
            _prevArmorRear = State.ArmorRearPct;
            _prevInternals = State.InternalsPct;

            string bestFacet = "INTERNALS";
            int bestDrop = dInt;
            if (dFront > 0 && dFront >= bestDrop) { bestFacet = "FRONT"; bestDrop = dFront; }
            if (dLeft > 0 && dLeft >= bestDrop) { bestFacet = "LEFT"; bestDrop = dLeft; }
            if (dRight > 0 && dRight >= bestDrop) { bestFacet = "RIGHT"; bestDrop = dRight; }
            if (dRear > 0 && dRear >= bestDrop) { bestFacet = "REAR"; bestDrop = dRear; }

            bool internalHit = dInt > 0;
            if (bestDrop > 0 || internalHit)
            {
                int reportDelta = bestDrop > 0 ? bestDrop : dInt;
                OnPlayerArmorDamaged?.Invoke(bestFacet, reportDelta, internalHit);
                PlayerRig?.TriggerHitFlash(internalHit);
                if (PlayerRig?.Root != null)
                {
                    Vector3 hullHit = PlayerRig.Root.transform.position + Vector3.up * 1.8f;
                    Vector3 origin = hullHit + Vector3.up * 4.5f;
                    if (!string.IsNullOrEmpty(State.LockedDesignator) &&
                        _contactRigs.TryGetValue(State.LockedDesignator, out var hostileRig) &&
                        hostileRig?.Root != null)
                    {
                        origin = hostileRig.MuzzlePoint != null
                            ? hostileRig.MuzzlePoint.position
                            : hostileRig.Root.transform.position + Vector3.up * 1.8f;
                    }
                    SpawnShotTracer(origin, hullHit, isLaser: false);
                }
            }
        }

        private void ParseSectionsIntoState(List<JsonNode> sections)
        {
            State.Sections.Clear();
            if (sections == null || sections.Count == 0) return;
            int cap = Math.Min(sections.Count, 24);
            for (int i = 0; i < cap; i++)
            {
                var sec = sections[i];
                if (sec == null || !sec.IsObject) continue;
                string sid = sec.Str("id", string.Empty);
                int intCur = sec.Int("internals", 100);
                int intMax = Mathf.Max(1, sec.Int("internals_max", 100));
                if (sid == "hull" || i == 0)
                    State.InternalsPct = Mathf.RoundToInt(intCur * 100f / intMax);

                var secState = new ArenaSectionState
                {
                    Id = sid,
                    Condition = sec.Str("condition", "intact"),
                    Internals = intCur,
                    InternalsMax = intMax
                };

                var facets = sec["facets"].AsArray();
                if (facets != null)
                {
                    for (int f = 0; f < facets.Count && f < 8; f++)
                    {
                        var fc = facets[f];
                        if (fc == null || !fc.IsObject) continue;
                        string fid = fc.Str("id", string.Empty);
                        int armCur = fc.Int("armor", 100);
                        int armMax = Mathf.Max(1, fc.Int("armor_max", 100));
                        secState.Facets.Add(new ArenaFacetState
                        {
                            Id = fid,
                            Armor = armCur,
                            ArmorMax = armMax
                        });
                        int pct = Mathf.RoundToInt(armCur * 100f / armMax);
                        if (fid == "front") State.ArmorFrontPct = pct;
                        else if (fid == "left") State.ArmorLeftPct = pct;
                        else if (fid == "right") State.ArmorRightPct = pct;
                        else if (fid == "rear") State.ArmorRearPct = pct;
                    }
                }
                State.Sections.Add(secState);
            }
        }

        public static int ParseContactHitOddsPct(JsonNode oddsNode)
        {
            if (oddsNode == null || oddsNode.IsNull) return 0;
            // Live frames.rs:363-377 emits an array [{ weapon, p_permille, percent }, ...] with default weapon first.
            if (oddsNode.IsArray)
            {
                var arr = oddsNode.AsArray();
                if (arr == null || arr.Count == 0) return 0;
                for (int i = 0; i < arr.Count; i++)
                {
                    var row = arr[i];
                    if (!row.IsObject) continue;
                    int pct = row.ContainsKey("percent") ? row.Int("percent", 0) : row.Int("p_permille", 0) / 10;
                    return Mathf.Clamp(pct, 0, 100);
                }
                return 0;
            }
            if (oddsNode.IsObject)
            {
                int pct = oddsNode.ContainsKey("percent") ? oddsNode.Int("percent", 0) : oddsNode.Int("p_permille", 0) / 10;
                return Mathf.Clamp(pct, 0, 100);
            }
            return 0;
        }

        public static float ParseFacingWordOrDeg(JsonNode node, float bearingFromViewerDeg = 0f)
        {
            if (node == null || node.IsNull) return 0f;
            if (node.Kind == JsonKind.Int || node.Kind == JsonKind.Float) return node.AsFloat(0f);
            string s = node.AsString("").Trim().ToLowerInvariant();
            // Live frames.rs:346 emits 8-way viewer-relative aspect words ("front".."front_left").
            // Vector from contact back toward viewer is (bearingFromViewerDeg + 180).
            switch (s)
            {
                case "front":       return (bearingFromViewerDeg + 180f) % 360f;
                case "front_right": return (bearingFromViewerDeg + 135f) % 360f;
                case "right":       return (bearingFromViewerDeg + 90f) % 360f;
                case "rear_right":  return (bearingFromViewerDeg + 45f) % 360f;
                case "rear":        return ((bearingFromViewerDeg % 360f) + 360f) % 360f;
                case "rear_left":   return (bearingFromViewerDeg + 315f) % 360f;
                case "left":        return (bearingFromViewerDeg + 270f) % 360f;
                case "front_left":  return (bearingFromViewerDeg + 225f) % 360f;
                // Legacy compass strings
                case "n":  return 0f;
                case "ne": return 45f;
                case "e":  return 90f;
                case "se": return 135f;
                case "s":  return 180f;
                case "sw": return 225f;
                case "w":  return 270f;
                case "nw": return 315f;
                default:   return 0f;
            }
        }

        public Vector3 ArenaWorldMetersToLocalPos(float worldXm, float worldZm)
        {
            if (TerrainGrid == null)
                return new Vector3(worldXm, 0f, worldZm);
            float siteCellM = TerrainGrid.CellSizeM / Math.Max(1, TerrainGrid.SpanCells);
            float originXm = TerrainGrid.OriginCellX * siteCellM;
            float originZm = TerrainGrid.OriginCellY * siteCellM;
            float lx = Mathf.Clamp(worldXm - originXm, 0f, TerrainGrid.Width * TerrainGrid.CellSizeM);
            float lz = Mathf.Clamp(worldZm - originZm, 0f, TerrainGrid.Height * TerrainGrid.CellSizeM);
            float ly = TerrainGrid.SampleElevationMeters(lx, lz);
            return new Vector3(lx, ly, lz);
        }

        private string ResolveDefaultAdvanceSpot()
        {
            if (TerrainGrid != null)
            {
                if (TerrainGrid.KeySpots != null && TerrainGrid.KeySpots.Count > 0)
                {
                    if (!string.IsNullOrWhiteSpace(TerrainGrid.HoldSpotLetter))
                    {
                        string hold = TerrainGrid.HoldSpotLetter.Trim();
                        for (int i = 0; i < TerrainGrid.KeySpots.Count; i++)
                        {
                            if (string.Equals(TerrainGrid.KeySpots[i].Letter, hold, StringComparison.OrdinalIgnoreCase) &&
                                MushProtocol.TryValidateSafeCommandToken(TerrainGrid.KeySpots[i].Letter, out string matchedHold, maxLen: 16))
                                return matchedHold;
                        }
                    }
                    if (MushProtocol.TryValidateSafeCommandToken(TerrainGrid.KeySpots[0].Letter, out string firstSpot, maxLen: 16))
                        return firstSpot;
                }
                if (MushProtocol.TryValidateSafeCommandToken(TerrainGrid.HoldSpotLetter, out string holdFallback, maxLen: 16))
                    return holdFallback;
            }
            return "B";
        }

        private void EnsurePlayerRigSpawned(string unitType, string callsign, string party)
        {
            if (_unitsRoot == null) return;
            if (PlayerRig?.Root != null)
            {
                if (Application.isPlaying) Destroy(PlayerRig.Root);
                else DestroyImmediate(PlayerRig.Root);
            }

            Vector3 wp = ArenaStageOrigin + ArenaWorldMetersToLocalPos(State.Xm, State.Zm);
            string safeCallsign = MushProtocol.EscapeUnityRichText(callsign);
            PlayerRig = TacSimVehicleVisualFactory.SpawnArenaCombatUnit(Catalog, _unitsRoot, unitType, $"{safeCallsign} [YOU]", party, wp, State.HeadingDeg);
            _desiredLiveHeadingDeg = State.HeadingDeg;
        }

        private void SyncLiveContact3DRig(ArenaContactState cs)
        {
            if (_unitsRoot == null || TerrainGrid == null) return;
            Vector3 localPos = TerrainGrid.SiteCellToLocalPos(cs.CellX, cs.CellY, 0f);
            Vector3 worldPos = ArenaStageOrigin + localPos;

            string safeDesig = MushProtocol.EscapeUnityRichText(cs.Designator);
            string safeClass = MushProtocol.EscapeUnityRichText(cs.ClassLabel);
            if (!_contactRigs.TryGetValue(cs.Designator, out var rig) || rig?.Root == null)
            {
                rig = TacSimVehicleVisualFactory.SpawnArenaCombatUnit(Catalog, _unitsRoot, cs.UnitType, safeDesig, cs.Party, worldPos, cs.FacingDeg);
                rig.Root.name = $"ArenaContact_{cs.Designator}";
                _contactRigs[cs.Designator] = rig;
            }
            else
            {
                Quaternion slopeRot = ComputeArenaChassisRotation(TerrainGrid, localPos, cs.FacingDeg, rig.IsBiped, out _);
                rig.Root.transform.position = Vector3.Lerp(rig.Root.transform.position, worldPos, 0.45f);
                rig.Root.transform.rotation = Quaternion.Slerp(rig.Root.transform.rotation, slopeRot, 0.45f);
            }

            rig.SetDamageState(cs.HealthWord, cs.IsWreck);
            if (!cs.IsWreck)
            {
                float? contactAimYaw = null;
                if (!cs.Friendly)
                {
                    Vector3 ownWorld = PlayerRig?.Root != null
                        ? PlayerRig.Root.transform.position
                        : ArenaStageOrigin + ArenaWorldMetersToLocalPos(State.Xm, State.Zm);
                    Vector3 toOwn = ownWorld - rig.Root.transform.position;
                    if (toOwn.sqrMagnitude > 1f)
                        contactAimYaw = Mathf.Atan2(toOwn.x, toOwn.z) * Mathf.Rad2Deg;
                }
                rig.UpdateArticulation(0.16f, 0f, contactAimYaw);
            }
            if (rig.Billboard?.NameplateText != null)
            {
                bool locked = string.Equals(State.LockedDesignator, cs.Designator, StringComparison.OrdinalIgnoreCase);
                string lockMark = locked ? "<color=#FFEE44>>> LOCKED <<</color>\n" : string.Empty;
                var sb = new System.Text.StringBuilder(72);
                sb.Append(lockMark).Append(safeDesig).Append(" [").Append(safeClass).Append("] ").Append((int)cs.RangeM).Append('m');
                if (cs.IsWreck)
                {
                    sb.Append(" <color=#FF6666>[WRECK]</color>");
                }
                else
                {
                    if (!cs.Friendly && cs.HitOddsPct > 0)
                    {
                        string oddsHex = cs.HitOddsPct >= 70 ? "#55FF88" : (cs.HitOddsPct >= 40 ? "#FFCC44" : "#FF6655");
                        sb.Append(" · <color=").Append(oddsHex).Append('>').Append(cs.HitOddsPct).Append("%</color>");
                    }
                    if (!string.IsNullOrEmpty(cs.Aspect))
                    {
                        string aspUp = MushProtocol.EscapeUnityRichText(cs.Aspect.Replace("_", " ").Trim().ToUpperInvariant());
                        if (string.Equals(cs.Aspect, "rear", StringComparison.OrdinalIgnoreCase))
                            sb.Append(" · <color=#FFDD55>").Append(aspUp).Append("</color>");
                        else
                            sb.Append(" · ").Append(aspUp);
                    }
                    string hw = (cs.HealthWord ?? string.Empty).Trim().ToLowerInvariant();
                    if (hw == "crippled" || hw == "critical")
                        sb.Append(" <color=#FF5544>[CRIT]</color>");
                    else if (hw == "scarred" || hw == "damaged")
                        sb.Append(" <color=#FFAA44>[SCAR]</color>");
                    if (cs.Moving)
                        sb.Append(" <color=#66DDFF>▶</color>");
                }
                rig.Billboard.NameplateText.text = sb.ToString();
            }
        }

        public void ExitArena()
        {
            IsArenaActive = false;
            _desiredLiveSpeedKph = 0f;
            _hasPrevArmorSnapshot = false;
            _prevArmorMatchId = 0;
            _playerMuzzleFlashUntil = -1f;
            if (_playerMuzzleFlash != null)
                _playerMuzzleFlash.gameObject.SetActive(false);
            if (_lockedTargetGroundRing != null)
                _lockedTargetGroundRing.enabled = false;
            if (_weaponEnvelopeRing != null)
                _weaponEnvelopeRing.enabled = false;
            if (_ecmJammerRing != null)
                _ecmJammerRing.enabled = false;
            if (_driveWaypointRoot != null)
                _driveWaypointRoot.gameObject.SetActive(false);
            if (_driveCourseLine != null)
                _driveCourseLine.enabled = false;
            Helm.Reset();
            Throttle.Reset();
            OnArenaStateUpdated?.Invoke();
        }

        public void ExecuteCockpitAction(string verbLine)
        {
            if (string.IsNullOrWhiteSpace(verbLine) || MushProtocol.ContainsControlChars(verbLine)) return;
            string trimmed = verbLine.Trim();
            string low = trimmed.ToLowerInvariant();

            if (low.StartsWith("fire", StringComparison.Ordinal))
            {
                PlayerRig?.PlayWeaponRecoil();
                if (!low.StartsWith("fire hold", StringComparison.Ordinal) &&
                    !low.StartsWith("fire free", StringComparison.Ordinal) &&
                    low.IndexOf(" when ", StringComparison.Ordinal) < 0 &&
                    !low.EndsWith(" off", StringComparison.Ordinal))
                {
                    bool isLaser = low.IndexOf("laser", StringComparison.Ordinal) >= 0;
                    TriggerPlayerMuzzleFlash(isLaser);
                }
            }
            else if (low == "scan")
            {
                TriggerRadarPulseVfx();
            }
            else if (low.StartsWith("scan ", StringComparison.Ordinal))
            {
                string target = trimmed.Substring("scan ".Length).Trim();
                if (!MushProtocol.TryValidateSafeCommandToken(target, out string safeTarget))
                    return;
                trimmed = $"scan {safeTarget}";
                TriggerRadarPulseVfx();
            }
            else if (low == "stop" || low == "speed 0")
            {
                // COCKPIT_VERBS has no "stop" verb; stopping requires "speed 0" (cockpit.rs:217-235).
                _desiredLiveSpeedKph = 0f;
                _lastSentSpeedKph = 0f;
                State.HasDriveWaypoint = false;
                Throttle.SyncExternalSpeedOrder(0);
                trimmed = "speed 0";
                SyncArenaTacticalVisuals();
            }
            else if (low.StartsWith("drive ", StringComparison.Ordinal))
            {
                string dest = trimmed.Substring("drive ".Length).Trim();
                TryRecordDriveWaypoint(dest);
            }
            else if (low.StartsWith("heading ", StringComparison.Ordinal))
            {
                State.HasDriveWaypoint = false;
                SyncArenaTacticalVisuals();
            }
            else if (low == "emcon")
            {
                // Bare "emcon" is read-only status print (notices.rs:2005); emit explicit toggle "emcon on" / "emcon off".
                trimmed = State.EmconSilent ? "emcon on" : "emcon off";
            }
            else if (low == "order all advance")
            {
                // ORDER <squad|ALL> ADVANCE <spot> requires a spot argument (orders.rs:191-193).
                trimmed = $"order all advance {ResolveDefaultAdvanceSpot()}";
            }
            else if (low.StartsWith("lock ", StringComparison.Ordinal))
            {
                string target = trimmed.Substring("lock ".Length).Trim();
                if (!MushProtocol.TryValidateSafeCommandToken(target, out string safeTarget))
                    return;
                trimmed = $"lock {safeTarget}";
            }
            else if (low.StartsWith("throttle ", StringComparison.Ordinal))
            {
                string arg = low.Substring("throttle ".Length).Trim();
                if (arg == "slow") _desiredLiveSpeedKph = State.MaxSpeedKph * 0.25f;
                else if (arg == "half") _desiredLiveSpeedKph = State.MaxSpeedKph * 0.5f;
                else if (arg == "cruise") _desiredLiveSpeedKph = State.MaxSpeedKph * 0.75f;
                else if (arg == "full") _desiredLiveSpeedKph = State.MaxSpeedKph;
                int rounded = Mathf.RoundToInt(_desiredLiveSpeedKph);
                trimmed = $"speed {rounded}";
                _lastSentSpeedKph = rounded;
                Throttle.SyncExternalSpeedOrder(rounded);
            }

            OnLiveCockpitVerbRequested?.Invoke(trimmed);
        }

        private void TriggerRadarPulseVfx()
        {
            if (_radarPulseRing == null || PlayerRig?.Root == null) return;
            _radarPulseScale = 4f;
            _radarPulseRing.position = PlayerRig.Root.transform.position + Vector3.up * 0.8f;
            _radarPulseRing.gameObject.SetActive(true);
        }

        private void Update()
        {
            float dt = Mathf.Max(0.0001f, Time.deltaTime);
            UpdateTracers(dt);

            if (TerrainGrid == null)
                return;

            if (IsArenaActive)
            {
                Handle3DBattlefieldMouseAndKeys();
                StepLiveClientInterpolationAndHelm(dt);
                return;
            }

            if (IsSpectatorTheaterActive)
            {
                UpdateSpectatorPlayback(dt);
            }
        }

        public bool HandleReplayTheaterHotkey(KeyCode key)
        {
            if (KeyboardSuspended || !IsSpectatorTheaterActive) return false;
            switch (key)
            {
                case KeyCode.V:
                    OnToggleCameraViewRequested?.Invoke();
                    return true;
                case KeyCode.Q:
                    OnCycleOpticsZoomRequested?.Invoke();
                    return true;
                case KeyCode.Tab:
                    CycleReplayFocusUnit(+1);
                    return true;
                case KeyCode.Space:
                    if (!IsReplayModeActive) return false;
                    ToggleReplayPlaying();
                    return true;
                case KeyCode.LeftArrow:
                    if (!IsReplayModeActive) return false;
                    StepReplayKeyframe(-1);
                    return true;
                case KeyCode.RightArrow:
                    if (!IsReplayModeActive) return false;
                    StepReplayKeyframe(+1);
                    return true;
                case KeyCode.LeftBracket:
                    if (!IsReplayModeActive) return false;
                    StepReplayToEvent(-1);
                    return true;
                case KeyCode.RightBracket:
                    if (!IsReplayModeActive) return false;
                    StepReplayToEvent(+1);
                    return true;
                default:
                    return false;
            }
        }

        public void StepReplayPlaybackForTests(float dt)
        {
            UpdateSpectatorPlayback(Mathf.Max(0.0001f, dt));
        }

        private void UpdateSpectatorPlayback(float dt)
        {
            if (!KeyboardSuspended)
            {
                if (Input.GetKeyDown(KeyCode.V)) HandleReplayTheaterHotkey(KeyCode.V);
                else if (Input.GetKeyDown(KeyCode.Q)) HandleReplayTheaterHotkey(KeyCode.Q);
                else if (Input.GetKeyDown(KeyCode.Space)) HandleReplayTheaterHotkey(KeyCode.Space);
                else if (Input.GetKeyDown(KeyCode.LeftArrow)) HandleReplayTheaterHotkey(KeyCode.LeftArrow);
                else if (Input.GetKeyDown(KeyCode.RightArrow)) HandleReplayTheaterHotkey(KeyCode.RightArrow);
                else if (Input.GetKeyDown(KeyCode.LeftBracket)) HandleReplayTheaterHotkey(KeyCode.LeftBracket);
                else if (Input.GetKeyDown(KeyCode.RightBracket)) HandleReplayTheaterHotkey(KeyCode.RightBracket);
            }

            if (IsReplayModeActive && ReplayPlaying && ActiveReplayDoc != null)
            {
                float prevMs = ReplayCursorMs;
                ReplayCursorMs += dt * 1000f * ReplayPlaybackSpeed;
                if (ReplayCursorMs >= ActiveReplayDoc.EndMs)
                {
                    ReplayCursorMs = ActiveReplayDoc.EndMs;
                    ReplayPlaying = false;
                    OnReplayStateChanged?.Invoke();
                }

                SyncReplay3DRigs(immediate: false, dt: dt);

                TacSimArenaReplayModel.CollectShotsInWindow(ActiveReplayDoc, prevMs, ReplayCursorMs, _scratchWindowShots);
                for (int i = 0; i < _scratchWindowShots.Count; i++)
                {
                    var shot = _scratchWindowShots[i];
                    if (_replayRigs.TryGetValue(shot.ShooterIndex, out var shRig) &&
                        _replayRigs.TryGetValue(shot.TargetIndex, out var tgRig) &&
                        shRig?.Root != null && tgRig?.Root != null)
                    {
                        Vector3 start = shRig.MuzzlePoint != null ? shRig.MuzzlePoint.position : shRig.Root.transform.position + Vector3.up * 1.8f;
                        Vector3 end = tgRig.Root.transform.position + Vector3.up * 1.6f;
                        bool isHit = string.Equals(shot.Outcome, "hit", StringComparison.OrdinalIgnoreCase);
                        Vector3 impactPt = isHit ? end : (end + tgRig.Root.transform.right * 4.5f);
                        SpawnShotTracer(start, impactPt, isLaser: isHit);
                        shRig.PlayWeaponRecoil();
                        if (isHit)
                        {
                            tgRig.TriggerHitFlash(isCrit: false);
                            SpawnImpactBurst(impactPt, ArenaImpactKind.Hit);
                            SpawnWorldCombatPopup(impactPt + Vector3.up * 3.8f, "HIT!", new Color(1.0f, 0.86f, 0.36f), 1.0f);
                        }
                        else
                        {
                            SpawnImpactBurst(impactPt, ArenaImpactKind.Miss);
                        }
                        OnWeaponShotFired?.Invoke(isHit);
                    }
                }

                var destroyed = ActiveReplayDoc.Destroyed;
                for (int i = 0; i < destroyed.Count; i++)
                {
                    var w = destroyed[i];
                    if (w.TimeMs > prevMs && w.TimeMs <= ReplayCursorMs &&
                        _replayRigs.TryGetValue(w.RosterIndex, out var wRig) &&
                        wRig?.Root != null)
                    {
                        Vector3 wPos = wRig.Root.transform.position + Vector3.up * 1.8f;
                        string cs = (w.RosterIndex >= 0 && w.RosterIndex < ActiveReplayDoc.Roster.Count)
                            ? ActiveReplayDoc.Roster[w.RosterIndex].Callsign
                            : $"U{w.RosterIndex + 1}";
                        wRig.TriggerHitFlash(isCrit: true);
                        SpawnImpactBurst(wPos, ArenaImpactKind.Kill);
                        SpawnWorldCombatPopup(wPos + Vector3.up * 5.0f, $"WRECKED! {cs}", new Color(1.0f, 0.28f, 0.20f), 1.35f);
                    }
                }
            }
            else if (IsLiveObserverActive && ActiveReplayDoc != null)
            {
                SyncReplay3DRigs(immediate: false, dt: dt);
            }
        }

        private void Handle3DBattlefieldMouseAndKeys()
        {
            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (!KeyboardSuspended)
            {
                if (Input.GetKeyDown(KeyCode.Space)) HandleArenaCombatHotkey(KeyCode.Space);
                else if (Input.GetKeyDown(KeyCode.V)) HandleArenaCombatHotkey(KeyCode.V);
                else if (Input.GetKeyDown(KeyCode.Q)) HandleArenaCombatHotkey(KeyCode.Q);
                else if (Input.GetKeyDown(KeyCode.Alpha1)) HandleArenaCombatHotkey(KeyCode.Alpha1);
                else if (Input.GetKeyDown(KeyCode.Alpha2)) HandleArenaCombatHotkey(KeyCode.Alpha2);
                else if (Input.GetKeyDown(KeyCode.Alpha3)) HandleArenaCombatHotkey(KeyCode.Alpha3);
                else if (Input.GetKeyDown(KeyCode.Alpha4)) HandleArenaCombatHotkey(KeyCode.Alpha4);
                else if (Input.GetKeyDown(KeyCode.Alpha0)) HandleArenaCombatHotkey(KeyCode.Alpha0);
                else if (Input.GetKeyDown(KeyCode.F)) HandleArenaCombatHotkey(KeyCode.F);
                else if (Input.GetKeyDown(KeyCode.G)) HandleArenaCombatHotkey(KeyCode.G);
                else if (Input.GetKeyDown(KeyCode.N)) HandleArenaCombatHotkey(KeyCode.N);
                else if (Input.GetKeyDown(KeyCode.M)) HandleArenaCombatHotkey(KeyCode.M);
                else if (Input.GetKeyDown(KeyCode.Z)) HandleArenaCombatHotkey(KeyCode.Z);
                else if (Input.GetKeyDown(KeyCode.L)) HandleArenaCombatHotkey(KeyCode.L);
                else if (Input.GetKeyDown(KeyCode.Y)) HandleArenaCombatHotkey(KeyCode.Y);
            }

            if (!overUi && Input.GetMouseButtonDown(0))
            {
                Camera cam = Camera.main;
                if (cam != null)
                {
                    Ray ray = cam.ScreenPointToRay(Input.mousePosition);
                    if (Physics.Raycast(ray, out RaycastHit hit, 3500f))
                    {
                        string hn = hit.collider.gameObject.name;
                        if (hn.StartsWith("ArenaContact_", StringComparison.OrdinalIgnoreCase))
                        {
                            string desig = hn.Substring("ArenaContact_".Length);
                            if (MushProtocol.TryValidateSafeCommandToken(desig, out string safeDesig))
                                ExecuteCockpitAction($"lock {safeDesig}");
                            return;
                        }
                        if (hn.StartsWith("KeySpot_", StringComparison.OrdinalIgnoreCase))
                        {
                            string letter = hn.Substring("KeySpot_".Length);
                            if (MushProtocol.TryValidateSafeCommandToken(letter, out string safeLetter, maxLen: 16))
                                ExecuteCockpitAction($"drive {safeLetter}");
                            return;
                        }

                        if (Input.GetKey(KeyCode.LeftShift) || Input.GetKey(KeyCode.RightShift))
                        {
                            Vector3 lp = hit.point - ArenaStageOrigin;
                            int cx = TerrainGrid.LocalPosToSiteCellX(lp.x);
                            int cy = TerrainGrid.LocalPosToSiteCellY(lp.z);
                            ExecuteCockpitAction($"drive {cx},{cy}");
                        }
                    }
                }
            }
        }

        private void StepLiveClientInterpolationAndHelm(float dt)
        {
            long nowMs = CurrentHelmClockMs();
            int turnDir = 0;

            if (!KeyboardSuspended)
            {
                bool fwdDown = Input.GetKeyDown(KeyCode.W) || Input.GetKeyDown(KeyCode.UpArrow);
                bool revDown = Input.GetKeyDown(KeyCode.S) || Input.GetKeyDown(KeyCode.DownArrow);
                bool fwdHeld = Input.GetKey(KeyCode.W) || Input.GetKey(KeyCode.UpArrow);
                bool revHeld = Input.GetKey(KeyCode.S) || Input.GetKey(KeyCode.DownArrow);

                if (fwdDown)
                {
                    Throttle.Bump(+1, nowMs);
                    _nextHeldThrottleBumpMs = nowMs + ArenaCoalescedThrottle.CoalesceMs;
                }
                else if (revDown)
                {
                    Throttle.Bump(-1, nowMs);
                    _nextHeldThrottleBumpMs = nowMs + ArenaCoalescedThrottle.CoalesceMs;
                }
                else if (fwdHeld && !revHeld && nowMs >= _nextHeldThrottleBumpMs)
                {
                    Throttle.Bump(+1, nowMs);
                    _nextHeldThrottleBumpMs = nowMs + ArenaCoalescedThrottle.CoalesceMs;
                }
                else if (revHeld && !fwdHeld && nowMs >= _nextHeldThrottleBumpMs)
                {
                    Throttle.Bump(-1, nowMs);
                    _nextHeldThrottleBumpMs = nowMs + ArenaCoalescedThrottle.CoalesceMs;
                }

                if (Input.GetKeyDown(KeyCode.X))
                {
                    Helm.Stop(nowMs);
                    Throttle.Stop(nowMs);
                }

                bool rightHeld = Input.GetKey(KeyCode.D) || Input.GetKey(KeyCode.RightArrow);
                bool leftHeld = Input.GetKey(KeyCode.A) || Input.GetKey(KeyCode.LeftArrow);
                if (rightHeld && !leftHeld) turnDir = +1;
                else if (leftHeld && !rightHeld) turnDir = -1;
            }

            if (turnDir != Helm.Turning)
            {
                if (turnDir == 0) Helm.Stop(nowMs);
                else Helm.Start(turnDir, nowMs);
            }

            Helm.Tick(nowMs);
            Throttle.Tick(nowMs);

            var headingOrders = Helm.TakeOrders();
            for (int i = 0; i < headingOrders.Count; i++)
                OnLiveCockpitVerbRequested?.Invoke(headingOrders[i]);

            var speedOrders = Throttle.TakeOrders();
            for (int i = 0; i < speedOrders.Count; i++)
                OnLiveCockpitVerbRequested?.Invoke(speedOrders[i]);

            float smoothHeading = PredictedHeadingDeg;
            _desiredLiveHeadingDeg = smoothHeading;
            _desiredLiveSpeedKph = TargetCruiseSpeedKph;

            bool withinReckonCap = _lastFrameAnchorMs <= 0L || (nowMs - _lastFrameAnchorMs) <= Helm.ReckonCapMs;
            if (!State.IsWreck && withinReckonCap && Mathf.Abs(State.SpeedKph) > 0.2f)
            {
                float mps = State.SpeedKph / 3.6f;
                Vector3 fwd = Quaternion.Euler(0f, smoothHeading, 0f) * Vector3.forward;
                float siteCellM = TerrainGrid.CellSizeM / Math.Max(1, TerrainGrid.SpanCells);
                float minWorldX = TerrainGrid.OriginCellX * siteCellM + 10f;
                float minWorldZ = TerrainGrid.OriginCellY * siteCellM + 10f;
                float maxWorldX = minWorldX + Math.Max(10f, TerrainGrid.Width * TerrainGrid.CellSizeM - 20f);
                float maxWorldZ = minWorldZ + Math.Max(10f, TerrainGrid.Height * TerrainGrid.CellSizeM - 20f);
                State.Xm = Mathf.Clamp(State.Xm + fwd.x * mps * dt, minWorldX, maxWorldX);
                State.Zm = Mathf.Clamp(State.Zm + fwd.z * mps * dt, minWorldZ, maxWorldZ);
            }
            UpdateOwnPlayerRigPose(dt);
        }

        private void UpdateOwnPlayerRigPose(float dt)
        {
            if (PlayerRig?.Root == null || TerrainGrid == null) return;
            Vector3 localPos = ArenaWorldMetersToLocalPos(State.Xm, State.Zm);
            Vector3 targetPos = ArenaStageOrigin + localPos;
            float shownHeading = PredictedHeadingDeg;
            Quaternion slopeRot = ComputeArenaChassisRotation(TerrainGrid, localPos, shownHeading, PlayerRig.IsBiped, out float tiltDeg);
            LastPlayerSlopeTiltDeg = tiltDeg;

            PlayerRig.Root.transform.position = Vector3.Lerp(PlayerRig.Root.transform.position, targetPos, 1f - Mathf.Exp(-14f * dt));
            PlayerRig.Root.transform.rotation = Quaternion.Slerp(PlayerRig.Root.transform.rotation, slopeRot, 1f - Mathf.Exp(-18f * dt));
            int minArmor = Mathf.Min(Mathf.Min(State.ArmorFrontPct, State.ArmorLeftPct), Mathf.Min(State.ArmorRightPct, State.ArmorRearPct));
            string ownHealthWord = State.IsWreck
                ? "wreck"
                : (State.InternalsPct < 50 || minArmor < 35)
                    ? "crippled"
                    : (State.InternalsPct < 90 || minArmor < 75)
                        ? "scarred"
                        : "intact";
            PlayerRig.SetDamageState(ownHealthWord, State.IsWreck);
            PlayerRig.SetHealthBarFraction((minArmor + State.InternalsPct) / 200f, visible: !State.IsWreck);
            PlayerRig.SetOperationalBeacon(State.Overheated, State.Immobile, State.IsWreck);

            float? aimYaw = null;
            if (!string.IsNullOrEmpty(State.LockedDesignator) && _contactRigs.TryGetValue(State.LockedDesignator, out var lockedRig) && lockedRig?.Root != null)
            {
                Vector3 d = lockedRig.Root.transform.position - PlayerRig.Root.transform.position;
                if (d.sqrMagnitude > 1f)
                    aimYaw = Mathf.Atan2(d.x, d.z) * Mathf.Rad2Deg;
            }
            PlayerRig.UpdateArticulation(dt, State.SpeedKph, aimYaw);
            SyncLiveContactsArticulation(dt);
            SyncPlayerDustPlume(shownHeading);
            SyncArenaTacticalVisuals();
        }

        private void SyncLiveContactsArticulation(float dt)
        {
            if (State?.Contacts == null || _contactRigs.Count == 0) return;
            Vector3 ownWorld = PlayerRig?.Root != null
                ? PlayerRig.Root.transform.position
                : ArenaStageOrigin + ArenaWorldMetersToLocalPos(State.Xm, State.Zm);

            for (int i = 0; i < State.Contacts.Count; i++)
            {
                var cs = State.Contacts[i];
                if (cs.IsWreck) continue;
                if (!_contactRigs.TryGetValue(cs.Designator, out var rig) || rig?.Root == null) continue;
                float? contactAimYaw = null;
                if (!cs.Friendly)
                {
                    Vector3 toOwn = ownWorld - rig.Root.transform.position;
                    if (toOwn.sqrMagnitude > 1f)
                        contactAimYaw = Mathf.Atan2(toOwn.x, toOwn.z) * Mathf.Rad2Deg;
                }
                rig.UpdateArticulation(dt, cs.Moving ? 28f : 0f, contactAimYaw);
            }
        }

        private void TryRecordDriveWaypoint(string dest)
        {
            if (State == null || string.IsNullOrWhiteSpace(dest)) return;
            string clean = dest.Trim();
            int comma = clean.IndexOf(',');
            if (comma > 0 && comma < clean.Length - 1 &&
                int.TryParse(clean.Substring(0, comma).Trim(), out int cx) &&
                int.TryParse(clean.Substring(comma + 1).Trim(), out int cy))
            {
                State.HasDriveWaypoint = true;
                State.DriveWaypointCellX = cx;
                State.DriveWaypointCellY = cy;
                State.DriveWaypointLabel = $"CELL {cx},{cy}";
                SyncArenaTacticalVisuals();
                return;
            }
            if (TerrainGrid?.KeySpots != null)
            {
                for (int i = 0; i < TerrainGrid.KeySpots.Count; i++)
                {
                    var sp = TerrainGrid.KeySpots[i];
                    if (string.Equals(sp.Letter, clean, StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(sp.Name, clean, StringComparison.OrdinalIgnoreCase))
                    {
                        State.HasDriveWaypoint = true;
                        State.DriveWaypointCellX = sp.CellX;
                        State.DriveWaypointCellY = sp.CellY;
                        State.DriveWaypointLabel = $"[{sp.Letter}] {(sp.Name ?? string.Empty).ToUpperInvariant()}";
                        SyncArenaTacticalVisuals();
                        return;
                    }
                }
            }
        }

        private void SyncPlayerDustPlume(float headingDeg)
        {
            if (_playerDustRoot == null) return;
            bool active = PlayerRig?.Root != null && !State.IsWreck && Mathf.Abs(State.SpeedKph) > 4f;
            if (_playerDustRoot.gameObject.activeSelf != active)
                _playerDustRoot.gameObject.SetActive(active);
            if (!active) return;

            float rad = headingDeg * Mathf.Deg2Rad;
            Vector3 back = new Vector3(-Mathf.Sin(rad), 0f, -Mathf.Cos(rad));
            Vector3 side = new Vector3(Mathf.Cos(rad), 0f, -Mathf.Sin(rad));
            Vector3 basePos = PlayerRig.Root.transform.position;
            float t = Time.unscaledTime * 3.8f;

            for (int i = 0; i < _playerDustPuffs.Length; i++)
            {
                var puff = _playerDustPuffs[i];
                if (puff == null) continue;
                float phase = (t + i * 0.33f) % 1f;
                float trailDist = 2.8f + phase * 4.5f;
                float latSign = (i % 2 == 0) ? -1.15f : 1.15f;
                puff.position = basePos + back * trailDist + side * latSign + Vector3.up * (0.45f + phase * 1.15f);
                float sz = 0.7f + phase * 1.25f;
                puff.localScale = new Vector3(sz, sz * 0.65f, sz);
            }
        }

        public void SyncArenaTacticalVisuals()
        {
            if (_dynamicObjectiveRoot != null && TerrainGrid != null)
            {
                bool showObj = State != null && State.HasDynamicObjective;
                if (_dynamicObjectiveRoot.gameObject.activeSelf != showObj)
                    _dynamicObjectiveRoot.gameObject.SetActive(showObj);
                if (showObj)
                {
                    Vector3 objWorld = ArenaStageOrigin + TerrainGrid.SiteCellToLocalPos(State.ObjectiveCellX, State.ObjectiveCellY, 0f);
                    _dynamicObjectiveRoot.position = objWorld;
                    if (_dynamicObjectiveRing != null)
                    {
                        float dia = Mathf.Max(20f, State.ObjectiveRadiusCells * TerrainGrid.CellSizeM * 2f);
                        _dynamicObjectiveRing.localScale = new Vector3(dia, 0.14f, dia);
                    }
                    if (_dynamicObjectiveLabel != null)
                    {
                        _dynamicObjectiveLabel.text = !string.IsNullOrEmpty(State.ObjectiveLetter)
                            ? $"[{State.ObjectiveLetter}] HOLD ZONE"
                            : "HOLD OBJECTIVE";
                    }
                }
            }

            if (PlayerRig?.Root == null || TerrainGrid == null || State == null || State.IsWreck)
            {
                if (_arenaRangeRing != null) _arenaRangeRing.enabled = false;
                if (_arenaLockBeam != null) _arenaLockBeam.enabled = false;
                if (_lockedTargetGroundRing != null) _lockedTargetGroundRing.enabled = false;
                if (_driveWaypointRoot != null) _driveWaypointRoot.gameObject.SetActive(false);
                if (_driveCourseLine != null) _driveCourseLine.enabled = false;
                return;
            }

            Vector3 ownPos = PlayerRig.Root.transform.position;
            SyncKeySpotBeaconsAndHoldZoneTint(ownPos);

            if (_arenaRangeRing != null)
            {
                const int Segs = 40;
                float rangeRadiusM = Mathf.Clamp(State.SightMeters > 10f ? State.SightMeters : 120f, 24f, 450f);
                Color ringCol = string.Equals(State.Scope, "hostile", StringComparison.OrdinalIgnoreCase)
                    ? new Color(1.0f, 0.34f, 0.24f, 0.42f)
                    : (string.Equals(State.Scope, "unidentified", StringComparison.OrdinalIgnoreCase)
                        ? new Color(1.0f, 0.78f, 0.24f, 0.38f)
                        : new Color(0.24f, 0.86f, 0.96f, 0.32f));
                _arenaRangeRing.startColor = ringCol;
                _arenaRangeRing.endColor = ringCol;
                _arenaRangeRing.positionCount = Segs;
                for (int i = 0; i < Segs; i++)
                {
                    float ang = (i / (float)Segs) * Mathf.PI * 2f;
                    float wx = ownPos.x + Mathf.Cos(ang) * rangeRadiusM;
                    float wz = ownPos.z + Mathf.Sin(ang) * rangeRadiusM;
                    float wy = ArenaStageOrigin.y + TerrainGrid.SampleElevationMeters(wx - ArenaStageOrigin.x, wz - ArenaStageOrigin.z) + 0.65f;
                    _arenaRangeRing.SetPosition(i, new Vector3(wx, wy, wz));
                }
                _arenaRangeRing.enabled = true;
            }

            if (_weaponEnvelopeRing != null)
            {
                if (TryGetPrimaryWeaponGunnery(out var primaryWpn) && primaryWpn.MaxRangeM > 15f)
                {
                    const int WpnSegs = 40;
                    float wpnRadiusM = Mathf.Clamp(primaryWpn.MaxRangeM, 18f, 520f);
                    Color wCol = primaryWpn.Ready
                        ? new Color(0.45f, 0.94f, 0.56f, 0.30f)
                        : new Color(0.95f, 0.78f, 0.28f, 0.24f);
                    _weaponEnvelopeRing.startColor = wCol;
                    _weaponEnvelopeRing.endColor = wCol;
                    _weaponEnvelopeRing.positionCount = WpnSegs;
                    for (int i = 0; i < WpnSegs; i++)
                    {
                        float ang = (i / (float)WpnSegs) * Mathf.PI * 2f;
                        float wx = ownPos.x + Mathf.Cos(ang) * wpnRadiusM;
                        float wz = ownPos.z + Mathf.Sin(ang) * wpnRadiusM;
                        float wy = ArenaStageOrigin.y + TerrainGrid.SampleElevationMeters(wx - ArenaStageOrigin.x, wz - ArenaStageOrigin.z) + 0.50f;
                        _weaponEnvelopeRing.SetPosition(i, new Vector3(wx, wy, wz));
                    }
                    _weaponEnvelopeRing.enabled = true;
                }
                else
                {
                    _weaponEnvelopeRing.enabled = false;
                }
            }

            if (_ecmJammerRing != null)
            {
                if (State.IsEcmJammingActive)
                {
                    const int EcmSegs = 36;
                    float pulse = 1.0f + 0.03f * Mathf.Sin(Time.unscaledTime * 4.5f);
                    float ecmRadiusM = Mathf.Clamp(State.EcmRadiusMeters, 20f, 500f) * pulse;
                    Color eCol = new Color(0.80f, 0.44f, 1.0f, 0.44f);
                    _ecmJammerRing.startColor = eCol;
                    _ecmJammerRing.endColor = eCol;
                    _ecmJammerRing.positionCount = EcmSegs;
                    for (int i = 0; i < EcmSegs; i++)
                    {
                        float ang = (i / (float)EcmSegs) * Mathf.PI * 2f;
                        float wx = ownPos.x + Mathf.Cos(ang) * ecmRadiusM;
                        float wz = ownPos.z + Mathf.Sin(ang) * ecmRadiusM;
                        float wy = ArenaStageOrigin.y + TerrainGrid.SampleElevationMeters(wx - ArenaStageOrigin.x, wz - ArenaStageOrigin.z) + 1.05f;
                        _ecmJammerRing.SetPosition(i, new Vector3(wx, wy, wz));
                    }
                    _ecmJammerRing.enabled = true;
                }
                else
                {
                    _ecmJammerRing.enabled = false;
                }
            }

            Vector3 targetPos = Vector3.zero;
            bool hasLiveLock = TryGetLockedContact(out var lockedCt) &&
                               !lockedCt.IsWreck &&
                               TryGetContactWorldPosition(State.LockedDesignator, out targetPos);

            Vector3 ownLocalForIntel = ownPos - ArenaStageOrigin;
            Vector3 tgtLocalForIntel = hasLiveLock ? (targetPos - ArenaStageOrigin) : Vector3.zero;
            CurrentTerrainIntel = TacSimArenaEnvironmentBuilder.EvaluateUnitTerrainContext(
                TerrainGrid,
                ownLocalForIntel.x,
                ownLocalForIntel.z,
                State.SensorHeightM,
                hasLiveLock,
                tgtLocalForIntel.x,
                tgtLocalForIntel.z);

            if (_arenaLockBeam != null)
            {
                if (hasLiveLock)
                {
                    Color beamCol = CurrentTerrainIntel.IsLineOfSightObstructed
                        ? new Color(1.0f, 0.72f, 0.20f, 0.42f)
                        : new Color(1.0f, 0.30f, 0.20f, 0.72f);
                    _arenaLockBeam.startColor = beamCol;
                    _arenaLockBeam.endColor = beamCol;
                    _arenaLockBeam.SetPosition(0, ownPos + Vector3.up * 2.4f);
                    _arenaLockBeam.SetPosition(1, targetPos);
                    _arenaLockBeam.enabled = true;
                }
                else
                {
                    _arenaLockBeam.enabled = false;
                }
            }

            if (_lockedTargetGroundRing != null)
            {
                if (hasLiveLock)
                {
                    const int LockSegs = 28;
                    float pulse = 5.6f + Mathf.Sin(Time.unscaledTime * 5.2f) * 0.65f;
                    float rotOff = Time.unscaledTime * 1.4f;
                    _lockedTargetGroundRing.positionCount = LockSegs;
                    for (int i = 0; i < LockSegs; i++)
                    {
                        float ang = rotOff + (i / (float)LockSegs) * Mathf.PI * 2f;
                        float wx = targetPos.x + Mathf.Cos(ang) * pulse;
                        float wz = targetPos.z + Mathf.Sin(ang) * pulse;
                        float wy = ArenaStageOrigin.y + TerrainGrid.SampleElevationMeters(wx - ArenaStageOrigin.x, wz - ArenaStageOrigin.z) + 0.55f;
                        _lockedTargetGroundRing.SetPosition(i, new Vector3(wx, wy, wz));
                    }
                    _lockedTargetGroundRing.enabled = true;
                }
                else
                {
                    _lockedTargetGroundRing.enabled = false;
                }
            }

            if (_driveWaypointRoot != null && _driveCourseLine != null)
            {
                bool showWp = State.HasDriveWaypoint;
                if (showWp)
                {
                    Vector3 wpLocal = TerrainGrid.SiteCellToLocalPos(State.DriveWaypointCellX, State.DriveWaypointCellY, 0f);
                    Vector3 wpWorld = ArenaStageOrigin + wpLocal;
                    Vector2 flatDelta = new Vector2(wpWorld.x - ownPos.x, wpWorld.z - ownPos.z);
                    if (flatDelta.magnitude < 14f && string.IsNullOrEmpty(State.MotorGoal))
                    {
                        State.HasDriveWaypoint = false;
                        showWp = false;
                    }
                    else
                    {
                        _driveWaypointRoot.position = wpWorld;
                        if (_driveWaypointLabel != null)
                            _driveWaypointLabel.text = !string.IsNullOrEmpty(State.DriveWaypointLabel)
                                ? $"WAYPOINT {State.DriveWaypointLabel}"
                                : "DRIVE WAYPOINT";
                        const int CoursePts = 12;
                        _driveCourseLine.positionCount = CoursePts;
                        for (int i = 0; i < CoursePts; i++)
                        {
                            float u = i / (float)(CoursePts - 1);
                            float wx = Mathf.Lerp(ownPos.x, wpWorld.x, u);
                            float wz = Mathf.Lerp(ownPos.z, wpWorld.z, u);
                            float wy = ArenaStageOrigin.y + TerrainGrid.SampleElevationMeters(wx - ArenaStageOrigin.x, wz - ArenaStageOrigin.z) + 0.85f;
                            _driveCourseLine.SetPosition(i, new Vector3(wx, wy, wz));
                        }
                    }
                }
                if (_driveWaypointRoot.gameObject.activeSelf != showWp)
                    _driveWaypointRoot.gameObject.SetActive(showWp);
                _driveCourseLine.enabled = showWp;
            }
        }

        private void SyncKeySpotBeaconsAndHoldZoneTint(Vector3 ownPos)
        {
            if (TerrainGrid == null || State == null) return;

            bool hasHoldZone = false;
            Vector3 holdCenterWorld = Vector3.zero;
            float holdRadiusM = 40f;

            if (State.HasDynamicObjective)
            {
                hasHoldZone = true;
                holdCenterWorld = ArenaStageOrigin + TerrainGrid.SiteCellToLocalPos(State.ObjectiveCellX, State.ObjectiveCellY, 0f);
                holdRadiusM = Mathf.Max(15f, State.ObjectiveRadiusCells * TerrainGrid.CellSizeM);
            }
            else
            {
                for (int i = 0; i < _keySpotBeacons.Count; i++)
                {
                    if (_keySpotBeacons[i].IsHoldSpot)
                    {
                        hasHoldZone = true;
                        holdCenterWorld = _keySpotBeacons[i].WorldPos;
                        holdRadiusM = Mathf.Max(15f, TerrainGrid.HoldRadiusCells * TerrainGrid.CellSizeM);
                        break;
                    }
                }
            }

            string owner = "NEUTRAL";
            if (State.HoldControlScore > 5)
            {
                owner = "RED";
            }
            else if (State.HoldControlScore < -5)
            {
                owner = "BLUE";
            }
            else if (hasHoldZone)
            {
                int redIn = 0;
                int blueIn = 0;
                float rSq = holdRadiusM * holdRadiusM;

                if (!State.IsWreck)
                {
                    float odx = ownPos.x - holdCenterWorld.x;
                    float odz = ownPos.z - holdCenterWorld.z;
                    if (odx * odx + odz * odz <= rSq)
                    {
                        if (string.Equals(State.Party, "R", StringComparison.OrdinalIgnoreCase)) redIn++;
                        else if (string.Equals(State.Party, "B", StringComparison.OrdinalIgnoreCase)) blueIn++;
                    }
                }

                for (int i = 0; i < State.Contacts.Count; i++)
                {
                    var ct = State.Contacts[i];
                    if (ct.IsWreck) continue;
                    Vector3 ctWorld = ArenaStageOrigin + TerrainGrid.SiteCellToLocalPos(ct.CellX, ct.CellY, 0f);
                    float cdx = ctWorld.x - holdCenterWorld.x;
                    float cdz = ctWorld.z - holdCenterWorld.z;
                    if (cdx * cdx + cdz * cdz <= rSq)
                    {
                        if (string.Equals(ct.Party, "R", StringComparison.OrdinalIgnoreCase)) redIn++;
                        else if (string.Equals(ct.Party, "B", StringComparison.OrdinalIgnoreCase)) blueIn++;
                    }
                }

                if (redIn > 0 && blueIn > 0) owner = "CONTESTED";
                else if (redIn > 0) owner = "RED";
                else if (blueIn > 0) owner = "BLUE";
            }

            ActiveHoldZoneVisualOwner = owner;

            Color holdColor;
            switch (owner)
            {
                case "RED":
                    holdColor = new Color(0.98f, 0.26f, 0.22f);
                    break;
                case "BLUE":
                    holdColor = new Color(0.24f, 0.64f, 1.0f);
                    break;
                case "CONTESTED":
                    holdColor = new Color(1.0f, 0.86f, 0.20f);
                    break;
                default:
                    holdColor = new Color(0.98f, 0.78f, 0.26f);
                    break;
            }

            string ownerSuffix = owner != "NEUTRAL" ? $" [{owner}]" : string.Empty;

            for (int i = 0; i < _keySpotBeacons.Count; i++)
            {
                var b = _keySpotBeacons[i];
                float dx = b.WorldPos.x - ownPos.x;
                float dz = b.WorldPos.z - ownPos.z;
                int distM = Mathf.RoundToInt(Mathf.Sqrt(dx * dx + dz * dz));

                if (b.Label != null)
                {
                    b.Label.text = b.IsHoldSpot
                        ? $"[{b.Letter}] {b.DisplayName} · {distM}m{ownerSuffix}"
                        : $"[{b.Letter}] {b.DisplayName} · {distM}m";
                    if (b.IsHoldSpot)
                        b.Label.color = holdColor;
                }
                if (b.IsHoldSpot)
                {
                    if (b.HoldRingRenderer != null)
                        b.HoldRingRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"HoldRing_{owner}", new Color(holdColor.r, holdColor.g, holdColor.b, owner == "NEUTRAL" ? 0.18f : 0.28f));
                    if (b.PillarRenderer != null)
                        b.PillarRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"HoldBeam_{owner}", new Color(holdColor.r, holdColor.g, holdColor.b, 0.48f));
                }
            }

            if (State.HasDynamicObjective && _dynamicObjectiveRoot != null && _dynamicObjectiveRoot.gameObject.activeSelf)
            {
                float dx = holdCenterWorld.x - ownPos.x;
                float dz = holdCenterWorld.z - ownPos.z;
                int distM = Mathf.RoundToInt(Mathf.Sqrt(dx * dx + dz * dz));
                if (_dynamicObjectiveRingRenderer != null)
                    _dynamicObjectiveRingRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"DynamicObjRing_{owner}", new Color(holdColor.r, holdColor.g, holdColor.b, owner == "NEUTRAL" ? 0.22f : 0.30f));
                if (_dynamicObjectivePillarRenderer != null)
                    _dynamicObjectivePillarRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"DynamicObjPillar_{owner}", new Color(holdColor.r, holdColor.g, holdColor.b, 0.48f));
                if (_dynamicObjectiveLabel != null)
                {
                    string baseTxt = !string.IsNullOrEmpty(State.ObjectiveLetter)
                        ? $"[{State.ObjectiveLetter}] HOLD ZONE"
                        : "HOLD OBJECTIVE";
                    _dynamicObjectiveLabel.text = $"{baseTxt} · {distM}m{ownerSuffix}";
                    _dynamicObjectiveLabel.color = holdColor;
                }
            }
        }

        private void TriggerPlayerMuzzleFlash(bool isLaser)
        {
            if (_playerMuzzleFlash == null || PlayerRig == null) return;
            Transform anchor = PlayerRig.MuzzlePoint != null ? PlayerRig.MuzzlePoint : PlayerRig.Root?.transform;
            if (anchor == null) return;
            _playerMuzzleFlash.position = anchor.position;
            _playerMuzzleFlash.rotation = anchor.rotation;
            _playerMuzzleFlash.localScale = isLaser ? new Vector3(1.1f, 1.1f, 1.8f) : new Vector3(1.7f, 1.7f, 2.5f);
            if (_playerMuzzleFlashRenderer != null)
            {
                Color col = isLaser ? new Color(1.0f, 0.25f, 0.42f) : new Color(1.0f, 0.88f, 0.36f);
                _playerMuzzleFlashRenderer.sharedMaterial = UrpMaterialFactory.GetEmissive(
                    isLaser ? "ArenaMuzzleLaser" : "ArenaMuzzleBallistic",
                    col,
                    col * 3.4f);
            }
            _playerMuzzleFlashUntil = 0.09f;
            _playerMuzzleFlash.gameObject.SetActive(true);
        }

        private void SpawnShotTracer(Vector3 startWorld, Vector3 endWorld, bool isLaser)
        {
            if (_vfxRoot == null) return;
            var go = new GameObject("ShotTracer");
            go.transform.SetParent(_vfxRoot, false);
            var lr = go.AddComponent<LineRenderer>();
            lr.useWorldSpace = true;
            lr.positionCount = 2;
            lr.SetPosition(0, startWorld);
            lr.SetPosition(1, isLaser ? endWorld : Vector3.Lerp(startWorld, endWorld, 0.28f));
            lr.startWidth = isLaser ? 0.42f : 0.26f;
            lr.endWidth = isLaser ? 0.26f : 0.10f;
            Color col = isLaser ? new Color(1.0f, 0.22f, 0.35f, 0.95f) : new Color(1.0f, 0.85f, 0.32f, 0.95f);
            lr.sharedMaterial = UrpMaterialFactory.GetHologramUnlit(isLaser ? "ArenaLaser" : "ArenaShell", col);

            Transform boltTransform = null;
            if (!isLaser)
            {
                var bolt = GameObject.CreatePrimitive(PrimitiveType.Cube);
                bolt.name = "ShellBolt";
                bolt.transform.SetParent(_vfxRoot, false);
                bolt.transform.position = startWorld;
                Vector3 dir = endWorld - startWorld;
                if (dir.sqrMagnitude > 1e-4f)
                    bolt.transform.rotation = Quaternion.LookRotation(dir.normalized, Vector3.up);
                bolt.transform.localScale = new Vector3(0.45f, 0.45f, 3.6f);
                var bc = bolt.GetComponent<Collider>();
                if (bc != null)
                {
                    if (Application.isPlaying) Destroy(bc);
                    else DestroyImmediate(bc);
                }
                bolt.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetEmissive("ArenaShellBolt", col, col * 3.2f);
                boltTransform = bolt.transform;
            }

            var flash = GameObject.CreatePrimitive(PrimitiveType.Sphere);
            flash.name = "ImpactFlash";
            flash.transform.SetParent(_vfxRoot, false);
            flash.transform.position = endWorld;
            flash.transform.localScale = isLaser ? Vector3.one * 2.4f : Vector3.one * 0.4f;
            var fc = flash.GetComponent<Collider>();
            if (fc != null)
            {
                if (Application.isPlaying) Destroy(fc);
                else DestroyImmediate(fc);
            }
            flash.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetEmissive("HitFlash", col, col * 2.8f);
            if (!isLaser)
                flash.SetActive(false);

            var ring = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            ring.name = "ImpactShockwave";
            ring.transform.SetParent(_vfxRoot, false);
            ring.transform.position = endWorld;
            ring.transform.localScale = new Vector3(1.2f, 0.08f, 1.2f);
            var rc = ring.GetComponent<Collider>();
            if (rc != null)
            {
                if (Application.isPlaying) Destroy(rc);
                else DestroyImmediate(rc);
            }
            ring.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit(
                isLaser ? "ArenaLaserShock" : "ArenaShellShock",
                new Color(col.r, col.g, col.b, 0.55f));
            if (!isLaser)
                ring.SetActive(false);

            _activeTracers.Add(new ActiveShotTracer
            {
                Line = lr,
                ProjectileBolt = boltTransform,
                ImpactFlash = flash.transform,
                ShockwaveRing = ring.transform,
                StartWorld = startWorld,
                EndWorld = endWorld,
                IsLaser = isLaser,
                Elapsed = 0f,
                Duration = isLaser ? 0.34f : 0.28f
            });
        }

        public void StepTracersForTests(float dt)
        {
            UpdateTracers(Mathf.Max(0.0001f, dt));
        }

        private void UpdateTracers(float dt)
        {
            if (_playerMuzzleFlashUntil > 0f && _playerMuzzleFlash != null)
            {
                _playerMuzzleFlashUntil -= dt;
                if (_playerMuzzleFlashUntil <= 0f)
                {
                    _playerMuzzleFlashUntil = -1f;
                    _playerMuzzleFlash.gameObject.SetActive(false);
                }
                else if (PlayerRig?.MuzzlePoint != null)
                {
                    _playerMuzzleFlash.position = PlayerRig.MuzzlePoint.position;
                }
            }

            if (_radarPulseScale >= 0f && _radarPulseRing != null)
            {
                _radarPulseScale += dt * 420f;
                if (PlayerRig?.Root != null)
                    _radarPulseRing.position = PlayerRig.Root.transform.position + Vector3.up * 0.8f;
                _radarPulseRing.localScale = new Vector3(_radarPulseScale, 0.14f, _radarPulseScale);
                if (_radarPulseScale > 900f)
                {
                    _radarPulseScale = -1f;
                    _radarPulseRing.gameObject.SetActive(false);
                }
            }

            for (int i = _activeTracers.Count - 1; i >= 0; i--)
            {
                var tr = _activeTracers[i];
                tr.Elapsed += dt;
                float u = Mathf.Clamp01(tr.Elapsed / Mathf.Max(0.01f, tr.Duration));

                if (tr.Elapsed >= tr.Duration)
                {
                    DestroyTracerObject(tr.Line != null ? tr.Line.gameObject : null);
                    DestroyTracerObject(tr.ProjectileBolt != null ? tr.ProjectileBolt.gameObject : null);
                    DestroyTracerObject(tr.ImpactFlash != null ? tr.ImpactFlash.gameObject : null);
                    DestroyTracerObject(tr.ShockwaveRing != null ? tr.ShockwaveRing.gameObject : null);
                    _activeTracers.RemoveAt(i);
                    continue;
                }

                if (tr.IsLaser)
                {
                    float fade = 1f - u;
                    if (tr.Line != null)
                    {
                        tr.Line.startWidth = 0.42f * fade;
                        tr.Line.endWidth = 0.24f * fade;
                    }
                    if (tr.ImpactFlash != null)
                    {
                        float s = 2.6f * (0.4f + fade * 0.8f);
                        tr.ImpactFlash.localScale = Vector3.one * s;
                    }
                    if (tr.ShockwaveRing != null)
                    {
                        float ringDia = 1.6f + u * 6.5f;
                        tr.ShockwaveRing.localScale = new Vector3(ringDia, 0.08f, ringDia);
                    }
                }
                else
                {
                    // Ballistic shell travels during u in [0..0.55], bursts into impact flash & shockwave during [0.55..1.0]
                    float flightU = Mathf.Clamp01(u / 0.55f);
                    float tailU = Mathf.Clamp01((u - 0.16f) / 0.55f);
                    Vector3 headPos = Vector3.Lerp(tr.StartWorld, tr.EndWorld, flightU);
                    Vector3 tailPos = Vector3.Lerp(tr.StartWorld, tr.EndWorld, tailU);

                    if (tr.Line != null)
                    {
                        tr.Line.SetPosition(0, tailPos);
                        tr.Line.SetPosition(1, headPos);
                        float fade = 1f - u * 0.7f;
                        tr.Line.startWidth = 0.14f * fade;
                        tr.Line.endWidth = 0.26f * fade;
                    }
                    if (tr.ProjectileBolt != null)
                    {
                        bool inFlight = u < 0.58f;
                        if (tr.ProjectileBolt.gameObject.activeSelf != inFlight)
                            tr.ProjectileBolt.gameObject.SetActive(inFlight);
                        if (inFlight)
                            tr.ProjectileBolt.position = headPos;
                    }
                    if (u >= 0.52f)
                    {
                        float impU = Mathf.Clamp01((u - 0.52f) / 0.48f);
                        if (tr.ImpactFlash != null)
                        {
                            if (!tr.ImpactFlash.gameObject.activeSelf) tr.ImpactFlash.gameObject.SetActive(true);
                            float fScale = Mathf.Lerp(3.2f, 0.5f, impU);
                            tr.ImpactFlash.localScale = Vector3.one * fScale;
                        }
                        if (tr.ShockwaveRing != null)
                        {
                            if (!tr.ShockwaveRing.gameObject.activeSelf) tr.ShockwaveRing.gameObject.SetActive(true);
                            float ringDia = 1.4f + impU * 8.5f;
                            tr.ShockwaveRing.localScale = new Vector3(ringDia, 0.10f, ringDia);
                        }
                    }
                }
            }

            StepPooledImpactsAndPopups(dt);
        }

        private void EnsureImpactBurstPool()
        {
            if (_vfxRoot == null || _impactBurstPool.Count >= 8) return;
            for (int i = _impactBurstPool.Count; i < 8; i++)
            {
                var root = new GameObject($"ImpactBurst_{i}");
                root.transform.SetParent(_vfxRoot, false);

                var core = GameObject.CreatePrimitive(PrimitiveType.Sphere);
                core.name = "FireballCore";
                core.transform.SetParent(root.transform, false);
                var cc = core.GetComponent<Collider>();
                if (cc != null)
                {
                    if (Application.isPlaying) Destroy(cc);
                    else DestroyImmediate(cc);
                }

                var ring = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
                ring.name = "BlastShockwave";
                ring.transform.SetParent(root.transform, false);
                var rc = ring.GetComponent<Collider>();
                if (rc != null)
                {
                    if (Application.isPlaying) Destroy(rc);
                    else DestroyImmediate(rc);
                }

                root.SetActive(false);
                _impactBurstPool.Add(new PooledImpactBurst
                {
                    Root = root,
                    CoreSphere = core.transform,
                    CoreRenderer = core.GetComponent<MeshRenderer>(),
                    ShockRing = ring.transform,
                    RingRenderer = ring.GetComponent<MeshRenderer>(),
                    Active = false
                });
            }
        }

        public void SpawnImpactBurst(Vector3 worldPos, ArenaImpactKind kind)
        {
            if (_vfxRoot == null) return;
            EnsureImpactBurstPool();
            if (_impactBurstPool.Count == 0) return;

            var burst = _impactBurstPool[_nextImpactPoolCursor % _impactBurstPool.Count];
            _nextImpactPoolCursor = (_nextImpactPoolCursor + 1) % _impactBurstPool.Count;

            burst.Kind = kind;
            burst.Elapsed = 0f;
            burst.Duration = kind == ArenaImpactKind.Kill
                ? 1.15f
                : (kind == ArenaImpactKind.SectionCrit ? 0.68f : (kind == ArenaImpactKind.Hit ? 0.46f : 0.40f));
            burst.Active = true;
            LastImpactKind = kind;

            if (burst.Root != null)
            {
                burst.Root.transform.position = worldPos;
                burst.Root.SetActive(true);
            }

            Color coreCol;
            Color ringCol;
            string key;
            switch (kind)
            {
                case ArenaImpactKind.Kill:
                    coreCol = new Color(1.0f, 0.28f, 0.14f);
                    ringCol = new Color(1.0f, 0.56f, 0.18f, 0.65f);
                    key = "ArenaImpactKill";
                    break;
                case ArenaImpactKind.SectionCrit:
                    coreCol = new Color(1.0f, 0.64f, 0.14f);
                    ringCol = new Color(1.0f, 0.80f, 0.22f, 0.58f);
                    key = "ArenaImpactCrit";
                    break;
                case ArenaImpactKind.Miss:
                    coreCol = new Color(0.66f, 0.62f, 0.54f);
                    ringCol = new Color(0.72f, 0.68f, 0.60f, 0.36f);
                    key = "ArenaImpactMiss";
                    break;
                default:
                    coreCol = new Color(1.0f, 0.82f, 0.28f);
                    ringCol = new Color(1.0f, 0.88f, 0.40f, 0.50f);
                    key = "ArenaImpactHit";
                    break;
            }

            if (burst.CoreRenderer != null)
                burst.CoreRenderer.sharedMaterial = UrpMaterialFactory.GetEmissive(key + "_Core", coreCol, coreCol * (kind == ArenaImpactKind.Miss ? 1.0f : 3.2f));
            if (burst.RingRenderer != null)
                burst.RingRenderer.sharedMaterial = UrpMaterialFactory.GetHologramUnlit(key + "_Ring", ringCol);

            if (burst.CoreSphere != null)
                burst.CoreSphere.localScale = Vector3.one * (kind == ArenaImpactKind.Kill ? 2.4f : 1.1f);
            if (burst.ShockRing != null)
                burst.ShockRing.localScale = new Vector3(1.5f, 0.09f, 1.5f);

            RecountActivePooledVfx();
        }

        private void EnsureWorldPopupPool()
        {
            if (_vfxRoot == null || _worldPopupPool.Count >= 10) return;
            for (int i = _worldPopupPool.Count; i < 10; i++)
            {
                var go = new GameObject($"WorldCombatPopup_{i}");
                go.transform.SetParent(_vfxRoot, false);
                var tmp = go.AddComponent<TextMeshPro>();
                tmp.fontSize = 10f;
                tmp.fontStyle = FontStyles.Bold;
                tmp.alignment = TextAlignmentOptions.Center;
                tmp.text = string.Empty;
                go.AddComponent<WorldBillboard>();
                go.SetActive(false);

                _worldPopupPool.Add(new PooledWorldPopup
                {
                    Root = go,
                    Label = tmp,
                    Active = false
                });
            }
        }

        public void SpawnWorldCombatPopup(Vector3 worldPos, string text, Color color, float scale = 1.0f)
        {
            if (_vfxRoot == null || string.IsNullOrWhiteSpace(text)) return;
            EnsureWorldPopupPool();
            if (_worldPopupPool.Count == 0) return;

            var pop = _worldPopupPool[_nextPopupPoolCursor % _worldPopupPool.Count];
            _nextPopupPoolCursor = (_nextPopupPoolCursor + 1) % _worldPopupPool.Count;

            pop.BasePos = worldPos;
            pop.BaseScale = Mathf.Clamp(scale, 0.7f, 1.6f);
            pop.Elapsed = 0f;
            pop.Duration = 1.35f;
            pop.Active = true;
            LastWorldPopupText = text;

            if (pop.Label != null)
            {
                pop.Label.text = text;
                pop.Label.color = color;
            }
            if (pop.Root != null)
            {
                pop.Root.transform.position = worldPos;
                pop.Root.transform.localScale = Vector3.one * pop.BaseScale;
                pop.Root.SetActive(true);
            }

            RecountActivePooledVfx();
        }

        private void DeactivatePooledImpactsAndPopups()
        {
            for (int i = 0; i < _impactBurstPool.Count; i++)
            {
                _impactBurstPool[i].Active = false;
                if (_impactBurstPool[i].Root != null)
                    _impactBurstPool[i].Root.SetActive(false);
            }
            for (int i = 0; i < _worldPopupPool.Count; i++)
            {
                _worldPopupPool[i].Active = false;
                if (_worldPopupPool[i].Root != null)
                    _worldPopupPool[i].Root.SetActive(false);
            }
            ActiveImpactBurstCount = 0;
            ActiveWorldPopupCount = 0;
        }

        private void StepPooledImpactsAndPopups(float dt)
        {
            int activeImpacts = 0;
            for (int i = 0; i < _impactBurstPool.Count; i++)
            {
                var b = _impactBurstPool[i];
                if (!b.Active) continue;
                b.Elapsed += dt;
                if (b.Elapsed >= b.Duration)
                {
                    b.Active = false;
                    if (b.Root != null) b.Root.SetActive(false);
                    continue;
                }
                activeImpacts++;
                float u = Mathf.Clamp01(b.Elapsed / Mathf.Max(0.01f, b.Duration));
                float coreMax = b.Kind == ArenaImpactKind.Kill ? 6.8f
                    : (b.Kind == ArenaImpactKind.SectionCrit ? 4.2f : (b.Kind == ArenaImpactKind.Hit ? 2.8f : 2.2f));
                float ringMax = b.Kind == ArenaImpactKind.Kill ? 19.0f
                    : (b.Kind == ArenaImpactKind.SectionCrit ? 11.5f : (b.Kind == ArenaImpactKind.Hit ? 7.5f : 5.0f));

                float coreEnv = u < 0.25f ? Mathf.Lerp(0.45f, 1.0f, u / 0.25f) : Mathf.Lerp(1.0f, 0.12f, (u - 0.25f) / 0.75f);
                if (b.CoreSphere != null)
                    b.CoreSphere.localScale = Vector3.one * (coreMax * coreEnv);
                if (b.ShockRing != null)
                {
                    float ringDia = Mathf.Lerp(1.4f, ringMax, u);
                    b.ShockRing.localScale = new Vector3(ringDia, 0.08f * (1f - u * 0.6f), ringDia);
                }
            }
            ActiveImpactBurstCount = activeImpacts;

            int activePopups = 0;
            for (int i = 0; i < _worldPopupPool.Count; i++)
            {
                var p = _worldPopupPool[i];
                if (!p.Active) continue;
                p.Elapsed += dt;
                if (p.Elapsed >= p.Duration)
                {
                    p.Active = false;
                    if (p.Root != null) p.Root.SetActive(false);
                    continue;
                }
                activePopups++;
                float u = Mathf.Clamp01(p.Elapsed / Mathf.Max(0.01f, p.Duration));
                if (p.Root != null)
                {
                    p.Root.transform.position = p.BasePos + Vector3.up * (u * 5.2f);
                    float popScale = p.BaseScale * (u < 0.15f ? Mathf.Lerp(0.75f, 1.15f, u / 0.15f) : Mathf.Lerp(1.15f, 0.90f, (u - 0.15f) / 0.85f));
                    p.Root.transform.localScale = Vector3.one * popScale;
                }
            }
            ActiveWorldPopupCount = activePopups;
        }

        private void RecountActivePooledVfx()
        {
            int imp = 0;
            for (int i = 0; i < _impactBurstPool.Count; i++)
                if (_impactBurstPool[i].Active) imp++;
            ActiveImpactBurstCount = imp;

            int pop = 0;
            for (int i = 0; i < _worldPopupPool.Count; i++)
                if (_worldPopupPool[i].Active) pop++;
            ActiveWorldPopupCount = pop;
        }

        private static void DestroyTracerObject(GameObject go)
        {
            if (go == null) return;
            if (Application.isPlaying) Destroy(go);
            else DestroyImmediate(go);
        }
    }
}
