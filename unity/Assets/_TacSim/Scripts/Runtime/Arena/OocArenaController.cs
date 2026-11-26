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
        public string HealthWord;   // "intact", "scarred", "crippled", "wreck"
        public bool IsWreck;
        public int HitOddsPct;
    }

    public sealed class ArenaCockpitState
    {
        public ulong MatchId;
        public string Callsign = "R1";
        public string Party = "R";
        public string UnitType = "tacsim.unit.line_tank";
        public string Mode = "skirmish";
        public string Power = "online";
        public bool IsWreck;
        public bool Immobile;
        public bool HasEcm;
        public bool EmconSilent;
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
        public string MatchBanner = string.Empty;

        public readonly List<ArenaWeaponSlotState> Weapons = new List<ArenaWeaponSlotState>();
        public readonly List<ArenaContactState> Contacts = new List<ArenaContactState>();
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
        public bool KeyboardSuspended { get; set; }
        public float TargetCruiseSpeedKph => _desiredLiveSpeedKph;
        public ulong CachedMapMatchId { get; private set; }

        // Diegetic Simulator Selector Console preferences (sent via live `sim` verbs in `room.simulator_lobby`)
        public string SelectedLobbyTeam { get; set; } = "red";
        public string SelectedLobbyUnit { get; set; } = "line_tank";
        public string SelectedLobbyMode { get; set; } = "skirmish";
        public int SelectedLobbyAiCount { get; set; } = 6;
        public string LiveLobbyMatchState { get; set; } = "idle";
        public string LiveLobbySlotSummary { get; set; } = string.Empty;

        public event Action<string> OnLiveCockpitVerbRequested;
        public event Action OnRequestArenaMapSubscribe;
        public event Action OnArenaStateUpdated;
        public event Action<string> OnArenaCombatLog;
        public event Action OnToggleCameraViewRequested;
        public event Action<bool> OnWeaponShotFired; // (isLaser)

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

        private void OnDestroy()
        {
            if (_terrainRoot != null)
                TacSimTerrainDecoder.DestroyTerrainChunkResources(_terrainRoot);
        }

        public void ResetSessionState()
        {
            IsArenaActive = false;
            CachedMapMatchId = 0;
            _desiredLiveSpeedKph = 0f;
            _lastSentSpeedKph = 0f;
            _lastSentHeadingDeg = -999f;
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
            OnArenaStateUpdated?.Invoke();
        }

        private Transform _terrainRoot;
        private Transform _beaconsRoot;
        private Transform _unitsRoot;
        private Transform _vfxRoot;

        private readonly Dictionary<string, SpawnedGroundVehicleRig> _contactRigs = new Dictionary<string, SpawnedGroundVehicleRig>(StringComparer.OrdinalIgnoreCase);
        private readonly List<ActiveShotTracer> _activeTracers = new List<ActiveShotTracer>();

        private float _desiredLiveSpeedKph;
        private float _desiredLiveHeadingDeg = 90f;
        private float _lastSentSpeedKph;
        private float _lastSentHeadingDeg = -999f;
        private float _nextLiveHelmSendTime;
        private Transform _radarPulseRing;
        private float _radarPulseScale = -1f;

        private sealed class ActiveShotTracer
        {
            public LineRenderer Line;
            public Transform ImpactFlash;
            public float ExpireTime;
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

            if (_terrainRoot != null)
                TacSimTerrainDecoder.DestroyTerrainChunkResources(_terrainRoot);

            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                var ch = transform.GetChild(i).gameObject;
                if (Application.isPlaying) Destroy(ch);
                else DestroyImmediate(ch);
            }
            _terrainRoot = null;
            _contactRigs.Clear();
            _activeTracers.Clear();

            transform.position = ArenaStageOrigin;

            _terrainRoot = new GameObject("ArenaTerrainChunks").transform;
            _terrainRoot.SetParent(transform, false);
            TacSimTerrainDecoder.BuildChunkedTerrainMeshes(TerrainGrid, _terrainRoot, chunkCells: 64);

            _beaconsRoot = new GameObject("KeySpotBeacons").transform;
            _beaconsRoot.SetParent(transform, false);
            BuildKeySpotsAndHoldRing();

            _unitsRoot = new GameObject("ArenaCombatUnits").transform;
            _unitsRoot.SetParent(transform, false);

            _vfxRoot = new GameObject("ArenaCombatVfx").transform;
            _vfxRoot.SetParent(transform, false);

            var pulseGo = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            pulseGo.name = "ArenaRadarPulseRing";
            pulseGo.transform.SetParent(_vfxRoot, false);
            var pulseCol = pulseGo.GetComponent<Collider>();
            if (pulseCol != null) { if (Application.isPlaying) Destroy(pulseCol); else DestroyImmediate(pulseCol); }
            pulseGo.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("ArenaRadarWave", new Color(0.25f, 0.92f, 1.0f, 0.48f));
            pulseGo.SetActive(false);
            _radarPulseRing = pulseGo.transform;
            _radarPulseScale = -1f;

            var sunGo = new GameObject("ArenaDirectionalFill");
            sunGo.transform.SetParent(transform, false);
            sunGo.transform.localPosition = new Vector3(1280f, 360f, 1280f);
            var lt = sunGo.AddComponent<Light>();
            lt.type = LightType.Point;
            lt.range = 3200f;
            lt.intensity = 2.2f;
            lt.color = TerrainGrid.Biome == "desert" ? new Color(1.0f, 0.88f, 0.75f) : new Color(0.92f, 0.96f, 1.0f);
        }

        private void BuildKeySpotsAndHoldRing()
        {
            if (TerrainGrid == null || _beaconsRoot == null) return;

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
                pillar.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit($"SpotBeam_{sp.Letter}", new Color(col.r, col.g, col.b, 0.42f));

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
                    capRing.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetHologramUnlit("HoldRing", new Color(1f, 0.78f, 0.18f, 0.16f));
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
                tmp.rectTransform.sizeDelta = new Vector2(80f, 8f);
                bb.NameplateText = tmp;
            }
        }

        public void ApplyLiveArenaMap(JsonNode mapData)
        {
            if (mapData == null || !mapData.IsObject || !mapData.Bool("seated", true)) return;
            var decoded = TacSimTerrainDecoder.DecodeArenaMapFrame(mapData);
            if (decoded == null) return;
            CachedMapMatchId = (ulong)Math.Max(0L, mapData.Long("match", 0));
            BuildArenaTerrain(Catalog, decoded);
            EnsurePlayerRigSpawned(State.UnitType, State.Callsign, State.Party);
            string safeDisplay = MushProtocol.EscapeUnityRichText(decoded.DisplayName);
            OnArenaCombatLog?.Invoke($"<color=#66EEAA>[ARENA MAP]</color> Loaded {decoded.Width}x{decoded.Height} heightfield ({safeDisplay}, {decoded.KeySpots.Count} key spots).");
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

            IsArenaActive = true;

            ulong matchId = (ulong)Math.Max(0L, d.Long("match", 0));
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
            State.HeadingDeg = d.Float("heading_deg", d.Int("heading_mdeg", 90000) * 0.001f);
            State.SpeedKph = d.Float("speed_kph", d.Int("speed_cms", 0) * 0.036f);
            State.MaxSpeedKph = Mathf.Max(10f, d.Float("max_speed_kph", 54f));
            State.ReverseSpeedKph = Mathf.Max(5f, d.Float("reverse_kph", 20f));

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
                        ArmedTarget = hasArmed ? armedObj.Str("target", string.Empty) : string.Empty
                    });
                }
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
                        HealthWord = c.Str("health", "intact"),
                        IsWreck = c.Bool("reads_dead", false) || c.Str("health") == "wreck",
                        HitOddsPct = ParseContactHitOddsPct(c["odds"])
                    };
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

            var notices = d["notices"].AsArray();
            if (notices != null)
            {
                int nCap = Math.Min(notices.Count, 32);
                for (int i = 0; i < nCap; i++)
                {
                    var n = notices[i];
                    string txt = n.Str("text", string.Empty);
                    string kind = n.Str("kind", string.Empty).ToLowerInvariant();
                    if (!string.IsNullOrEmpty(txt))
                        OnArenaCombatLog?.Invoke($"<color=#FFCC66>[COCKPIT]</color> {TacSim.Net.MushProtocol.EscapeUnityRichText(txt)}");

                    if (kind.Contains("fire") || kind.Contains("hit") || kind.Contains("shot"))
                    {
                        bool isLaser = n.Str("weapon", "").IndexOf("laser", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                       txt.IndexOf("laser", StringComparison.OrdinalIgnoreCase) >= 0;
                        OnWeaponShotFired?.Invoke(isLaser);
                        string tgtDesig = n.Str("designator", State.LockedDesignator);
                        if (_contactRigs.TryGetValue(tgtDesig, out var tgtRig) && PlayerRig?.MuzzlePoint != null && tgtRig?.Root != null)
                        {
                            SpawnShotTracer(PlayerRig.MuzzlePoint.position, tgtRig.Root.transform.position + Vector3.up * 1.5f, isLaser);
                        }
                    }
                }
            }

            if (State.Power == "offline")
            {
                OnLiveCockpitVerbRequested?.Invoke("startup");
            }

            OnArenaStateUpdated?.Invoke();
        }

        private void ParseSectionsIntoState(List<JsonNode> sections)
        {
            if (sections == null || sections.Count == 0) return;
            for (int i = 0; i < sections.Count; i++)
            {
                var sec = sections[i];
                string sid = sec.Str("id", string.Empty);
                int intCur = sec.Int("internals", 100);
                int intMax = Mathf.Max(1, sec.Int("internals_max", 100));
                if (sid == "hull" || i == 0)
                    State.InternalsPct = Mathf.RoundToInt(intCur * 100f / intMax);

                var facets = sec["facets"].AsArray();
                if (facets == null) continue;
                for (int f = 0; f < facets.Count; f++)
                {
                    var fc = facets[f];
                    string fid = fc.Str("id", string.Empty);
                    int pct = Mathf.RoundToInt(fc.Int("armor", 100) * 100f / Mathf.Max(1, fc.Int("armor_max", 100)));
                    if (fid == "front") State.ArmorFrontPct = pct;
                    else if (fid == "left") State.ArmorLeftPct = pct;
                    else if (fid == "right") State.ArmorRightPct = pct;
                    else if (fid == "rear") State.ArmorRearPct = pct;
                }
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
                rig.Root.transform.position = Vector3.Lerp(rig.Root.transform.position, worldPos, 0.45f);
                rig.Root.transform.rotation = Quaternion.Slerp(rig.Root.transform.rotation, Quaternion.Euler(0f, cs.FacingDeg, 0f), 0.45f);
            }

            rig.SetWrecked(cs.IsWreck);
            if (rig.Billboard?.NameplateText != null)
            {
                bool locked = string.Equals(State.LockedDesignator, cs.Designator, StringComparison.OrdinalIgnoreCase);
                string lockMark = locked ? "<color=#FFEE44>>> LOCKED <<</color>\n" : string.Empty;
                rig.Billboard.NameplateText.text = $"{lockMark}{safeDesig} [{safeClass}] {(int)cs.RangeM}m";
            }
        }

        public void ExitArena()
        {
            IsArenaActive = false;
            _desiredLiveSpeedKph = 0f;
            OnArenaStateUpdated?.Invoke();
        }

        public void ExecuteCockpitAction(string verbLine)
        {
            if (string.IsNullOrWhiteSpace(verbLine) || MushProtocol.ContainsControlChars(verbLine)) return;
            string trimmed = verbLine.Trim();
            string low = trimmed.ToLowerInvariant();

            if (low.StartsWith("fire", StringComparison.Ordinal))
                PlayerRig?.PlayWeaponRecoil();
            else if (low == "scan" || low.StartsWith("scan ", StringComparison.Ordinal))
                TriggerRadarPulseVfx();
            else if (low == "stop")
            {
                // COCKPIT_VERBS has no "stop" verb; stopping requires "speed 0" (cockpit.rs:217-235).
                _desiredLiveSpeedKph = 0f;
                _lastSentSpeedKph = 0f;
                trimmed = "speed 0";
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
                trimmed = $"speed {Mathf.RoundToInt(_desiredLiveSpeedKph)}";
                _lastSentSpeedKph = _desiredLiveSpeedKph;
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

            if (!IsArenaActive || TerrainGrid == null)
                return;

            Handle3DBattlefieldMouseAndKeys();
            StepLiveClientInterpolationAndHelm(dt);
        }

        private void Handle3DBattlefieldMouseAndKeys()
        {
            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (!KeyboardSuspended)
            {
                if (Input.GetKeyDown(KeyCode.Space))
                    ExecuteCockpitAction("fire");
                else if (Input.GetKeyDown(KeyCode.V))
                    OnToggleCameraViewRequested?.Invoke();
                else if (Input.GetKeyDown(KeyCode.Alpha1)) ExecuteCockpitAction("throttle slow");
                else if (Input.GetKeyDown(KeyCode.Alpha2)) ExecuteCockpitAction("throttle half");
                else if (Input.GetKeyDown(KeyCode.Alpha3)) ExecuteCockpitAction("throttle cruise");
                else if (Input.GetKeyDown(KeyCode.Alpha4)) ExecuteCockpitAction("throttle full");
                else if (Input.GetKeyDown(KeyCode.Alpha0)) ExecuteCockpitAction("speed 0");
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
            bool changed = false;
            if (!KeyboardSuspended)
            {
                if (Input.GetKey(KeyCode.W) || Input.GetKey(KeyCode.UpArrow))
                {
                    _desiredLiveSpeedKph = Mathf.Clamp(_desiredLiveSpeedKph + 36f * dt, -State.ReverseSpeedKph, State.MaxSpeedKph);
                    changed = true;
                }
                else if (Input.GetKey(KeyCode.S) || Input.GetKey(KeyCode.DownArrow))
                {
                    _desiredLiveSpeedKph = Mathf.Clamp(_desiredLiveSpeedKph - 42f * dt, -State.ReverseSpeedKph, State.MaxSpeedKph);
                    changed = true;
                }
                if (Input.GetKeyDown(KeyCode.X))
                {
                    _desiredLiveSpeedKph = 0f;
                    _lastSentSpeedKph = 0f;
                    OnLiveCockpitVerbRequested?.Invoke("speed 0");
                }

                if (Input.GetKey(KeyCode.D) || Input.GetKey(KeyCode.RightArrow))
                {
                    _desiredLiveHeadingDeg = (_desiredLiveHeadingDeg + 55f * dt + 360f) % 360f;
                    changed = true;
                }
                else if (Input.GetKey(KeyCode.A) || Input.GetKey(KeyCode.LeftArrow))
                {
                    _desiredLiveHeadingDeg = (_desiredLiveHeadingDeg - 55f * dt + 360f) % 360f;
                    changed = true;
                }
            }

            if (changed && Time.time >= _nextLiveHelmSendTime)
            {
                _nextLiveHelmSendTime = Time.time + 0.35f;
                if (Mathf.Abs(Mathf.DeltaAngle(_desiredLiveHeadingDeg, _lastSentHeadingDeg)) >= 4f)
                {
                    int hdgInt = (Mathf.RoundToInt(_desiredLiveHeadingDeg) % 360 + 360) % 360;
                    _lastSentHeadingDeg = hdgInt;
                    OnLiveCockpitVerbRequested?.Invoke($"heading {hdgInt}");
                }
                if (Mathf.Abs(_desiredLiveSpeedKph - _lastSentSpeedKph) >= 3f)
                {
                    int spdInt = Mathf.RoundToInt(_desiredLiveSpeedKph);
                    _lastSentSpeedKph = spdInt;
                    OnLiveCockpitVerbRequested?.Invoke($"speed {spdInt}");
                }
            }

            if (!State.IsWreck && Mathf.Abs(State.SpeedKph) > 0.2f)
            {
                float mps = State.SpeedKph / 3.6f;
                Vector3 fwd = Quaternion.Euler(0f, State.HeadingDeg, 0f) * Vector3.forward;
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
            Vector3 targetPos = ArenaStageOrigin + ArenaWorldMetersToLocalPos(State.Xm, State.Zm);
            PlayerRig.Root.transform.position = Vector3.Lerp(PlayerRig.Root.transform.position, targetPos, 1f - Mathf.Exp(-14f * dt));
            PlayerRig.Root.transform.rotation = Quaternion.Slerp(PlayerRig.Root.transform.rotation, Quaternion.Euler(0f, State.HeadingDeg, 0f), 1f - Mathf.Exp(-12f * dt));
            PlayerRig.SetWrecked(State.IsWreck);

            float? aimYaw = null;
            if (!string.IsNullOrEmpty(State.LockedDesignator) && _contactRigs.TryGetValue(State.LockedDesignator, out var lockedRig) && lockedRig?.Root != null)
            {
                Vector3 d = lockedRig.Root.transform.position - PlayerRig.Root.transform.position;
                if (d.sqrMagnitude > 1f)
                    aimYaw = Mathf.Atan2(d.x, d.z) * Mathf.Rad2Deg;
            }
            PlayerRig.UpdateArticulation(dt, State.SpeedKph, aimYaw);
        }

        private void SpawnShotTracer(Vector3 startWorld, Vector3 endWorld, bool isLaser)
        {
            if (_vfxRoot == null) return;
            var go = new GameObject("ShotTracer");
            go.transform.SetParent(_vfxRoot, false);
            var lr = go.AddComponent<LineRenderer>();
            lr.positionCount = 2;
            lr.SetPosition(0, startWorld);
            lr.SetPosition(1, endWorld);
            lr.startWidth = isLaser ? 0.36f : 0.24f;
            lr.endWidth = isLaser ? 0.24f : 0.12f;
            Color col = isLaser ? new Color(1.0f, 0.22f, 0.35f, 0.95f) : new Color(1.0f, 0.85f, 0.32f, 0.95f);
            lr.sharedMaterial = UrpMaterialFactory.GetHologramUnlit(isLaser ? "ArenaLaser" : "ArenaShell", col);

            var flash = GameObject.CreatePrimitive(PrimitiveType.Sphere);
            flash.transform.SetParent(_vfxRoot, false);
            flash.transform.position = endWorld;
            flash.transform.localScale = Vector3.one * 2.4f;
            var fc = flash.GetComponent<Collider>();
            if (fc != null)
            {
                if (Application.isPlaying) Destroy(fc);
                else DestroyImmediate(fc);
            }
            flash.GetComponent<MeshRenderer>().sharedMaterial = UrpMaterialFactory.GetEmissive("HitFlash", col, col * 2.5f);

            _activeTracers.Add(new ActiveShotTracer
            {
                Line = lr,
                ImpactFlash = flash.transform,
                ExpireTime = Time.time + (isLaser ? 0.32f : 0.20f)
            });
        }

        private void UpdateTracers(float dt)
        {
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
                if (Time.time >= _activeTracers[i].ExpireTime)
                {
                    if (_activeTracers[i].Line != null)
                    {
                        if (Application.isPlaying) Destroy(_activeTracers[i].Line.gameObject);
                        else DestroyImmediate(_activeTracers[i].Line.gameObject);
                    }
                    if (_activeTracers[i].ImpactFlash != null)
                    {
                        if (Application.isPlaying) Destroy(_activeTracers[i].ImpactFlash.gameObject);
                        else DestroyImmediate(_activeTracers[i].ImpactFlash.gameObject);
                    }
                    _activeTracers.RemoveAt(i);
                }
            }
        }
    }
}
