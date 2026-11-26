using System;
using System.Collections.Generic;
using System.Text;
using TacSim.Arena;
using TacSim.Character;
using TacSim.Net;
using TacSim.Space;
using TacSim.Surface;
using TacSim.UI;
using TacSim.World;
using UnityEngine;

namespace TacSim.Core
{
    /// <summary>
    /// Authoritative pure-online runtime coordinator across all 5 TacSim 3D gameplay stages:
    /// Wires MushWsClient &amp; TacSimContentSync &lt;-&gt; Multi-Port 3D Stations &amp; OOC Simulator Centre
    /// &lt;-&gt; Multi-Hull Ship Interiors (Dray / Dray-P / Corvette) &lt;-&gt; 3D Orbital Flight, Zero-G Mining
    /// &amp; 60 Hz Docking &lt;-&gt; Planetary Surface Rovers &lt;-&gt; Live 6v6 Red vs Blue Arena.
    /// </summary>
    public sealed class TacSimGameCoordinator : MonoBehaviour
    {
        public TacSimAssetCatalog AssetCatalog;
        public StationWorldController StationWorld;
        public ShipInteriorController ShipInterior;
        public SpaceFlightController SpaceStage;
        public SurfaceMiningController SurfaceStage;
        public OocArenaController ArenaStage;
        public TacSimCameraRig CameraRig;
        public ThirdPersonPlayerController Player;
        public TacSimHudController Hud;
        public TacSimAudioSynthesizer Audio;
        public TacSimWayfinder3D Wayfinder;

        public TacSimTradeAndShipState TradeState { get; private set; }
        public TacSimActiveStage ActiveStage => _stageDirector != null ? _stageDirector.CurrentStage : TacSimActiveStage.StationWalk;

        public event Action<string> OnCommandDispatched;

        private MushWsClient _ws;
        private TacSimContentSync _contentSync;
        private TacSimStageDirector _stageDirector;

        private bool _eventsBound;
        private bool _authInFlight;
        private bool _connectInFlight;
        private bool _isAboardShip;
        private bool _hullIsDocked = true;
        private bool _awaitingInitialRoomSync;
        private bool _limboAutoEnterAttempted;
        private MushRoomView _lastRoomView;
        private string _serverStationRoomEntry;
        private string _serverShipCompartment;
        private readonly RoomBoundNavWalker _navWalker = new RoomBoundNavWalker();
        private float _nextMoveStepAllowedAt;
        private bool _simulateConnectedForTests;
        private string _currentClockStr = "08:15 UTC";
        private string _currentHullBadge = "NO CHARTER";
        private MushWsConnectionState _lastBadgeState = MushWsConnectionState.Disconnected;
        private float _lastAutoTradeRefreshAt = -10f;

        private void Start()
        {
            EnsureSubsystems();
            BindEvents();
        }

        public void EnsureSubsystems()
        {
            if (AssetCatalog == null)
                AssetCatalog = TacSimAssetCatalog.LoadOrDefault();
            else
                AssetCatalog.EnsureEditorCatalogHydrated();

            if (Audio == null)
            {
                Audio = FindAnyObjectByType<TacSimAudioSynthesizer>();
                if (Audio == null)
                    Audio = gameObject.GetComponent<TacSimAudioSynthesizer>() ?? gameObject.AddComponent<TacSimAudioSynthesizer>();
            }

            if (TradeState == null)
                TradeState = new TacSimTradeAndShipState();

            if (StationWorld == null)
            {
                StationWorld = FindAnyObjectByType<StationWorldController>();
                if (StationWorld == null)
                {
                    var go = new GameObject("CeresFreeportStation");
                    StationWorld = go.AddComponent<StationWorldController>();
                    StationWorld.BuildStation(AssetCatalog);
                }
            }
            else if (StationWorld.Pack == null)
            {
                StationWorld.BuildStation(AssetCatalog);
            }

            if (ShipInterior == null)
            {
                ShipInterior = FindAnyObjectByType<ShipInteriorController>();
                if (ShipInterior == null)
                {
                    var go = new GameObject("DrayShipInterior");
                    ShipInterior = go.AddComponent<ShipInteriorController>();
                    ShipInterior.BuildInterior(AssetCatalog);
                }
            }
            else if (ShipInterior.Template == null)
            {
                ShipInterior.BuildInterior(AssetCatalog);
            }

            if (SpaceStage == null)
            {
                SpaceStage = FindAnyObjectByType<SpaceFlightController>();
                if (SpaceStage == null)
                {
                    var go = new GameObject("OrbitalSpaceStage");
                    SpaceStage = go.AddComponent<SpaceFlightController>();
                    SpaceStage.BuildSpaceStage(AssetCatalog);
                }
            }
            else if (SpaceStage.PlayerShipTransform == null)
            {
                SpaceStage.BuildSpaceStage(AssetCatalog);
            }

            if (SurfaceStage == null)
            {
                SurfaceStage = FindAnyObjectByType<SurfaceMiningController>();
                if (SurfaceStage == null)
                {
                    var go = new GameObject("PlanetarySurfaceStage");
                    SurfaceStage = go.AddComponent<SurfaceMiningController>();
                    SurfaceStage.BuildSurfaceStage(AssetCatalog, "occator_rim");
                }
            }
            else if (SurfaceStage.TerrainGrid == null)
            {
                SurfaceStage.BuildSurfaceStage(AssetCatalog, "occator_rim");
            }

            if (ArenaStage == null)
            {
                ArenaStage = FindAnyObjectByType<OocArenaController>();
                if (ArenaStage == null)
                {
                    var go = new GameObject("OocArenaStage");
                    ArenaStage = go.AddComponent<OocArenaController>();
                    ArenaStage.Catalog = AssetCatalog;
                }
            }
            else
            {
                ArenaStage.Catalog = AssetCatalog;
            }

            if (CameraRig == null)
            {
                CameraRig = FindAnyObjectByType<TacSimCameraRig>();
                if (CameraRig == null)
                {
                    var camGo = Camera.main != null ? Camera.main.gameObject : new GameObject("Main Camera", typeof(Camera), typeof(AudioListener));
                    camGo.tag = "MainCamera";
                    CameraRig = camGo.GetComponent<TacSimCameraRig>() ?? camGo.AddComponent<TacSimCameraRig>();
                    CameraRig.ControlledCamera = camGo.GetComponent<Camera>();
                }
            }

            bool playerCreated = false;
            if (Player == null)
            {
                Player = FindAnyObjectByType<ThirdPersonPlayerController>();
                if (Player == null)
                {
                    var pGo = new GameObject("PlayerCharacter");
                    pGo.tag = "Player";
                    pGo.AddComponent<CharacterController>();
                    var vis = pGo.AddComponent<CharacterVisualSpawner>();
                    Player = pGo.AddComponent<ThirdPersonPlayerController>();
                    Player.VisualSpawner = vis;
                    vis.BuildPlayerVisual(AssetCatalog, PlayerAvatarPreset.StellarTrooperFemale);
                    Player.TeleportTo(StationWorld.GetRoomSpawnPosition("room.c03_arrivals"), 90f);
                    playerCreated = true;
                }
            }

            if (Player.VisualSpawner == null)
                Player.VisualSpawner = Player.GetComponent<CharacterVisualSpawner>() ?? Player.gameObject.AddComponent<CharacterVisualSpawner>();
            if (!Player.VisualSpawner.HasActiveVisual)
                Player.VisualSpawner.BuildPlayerVisual(AssetCatalog, PlayerAvatarPreset.StellarTrooperFemale);

            if (CameraRig.ControlledCamera == null)
                CameraRig.ControlledCamera = CameraRig.GetComponentInChildren<Camera>() ?? Camera.main;

            Player.CameraTransform = CameraRig.ControlledCamera.transform;
            CameraRig.WalkFollowTarget = Player.transform;
            CameraRig.BridgeViewAnchor = ShipInterior.BridgeCameraAnchor;
            CameraRig.OrbitalShipTarget = SpaceStage.PlayerShipTransform;
            CameraRig.BowDockingAnchor = SpaceStage.BowDockingCameraAnchor;
            if (playerCreated)
                CameraRig.SnapBehindTarget(90f);

            if (Hud == null)
            {
                Hud = FindAnyObjectByType<TacSimHudController>();
                if (Hud == null)
                {
                    var hudGo = new GameObject("TacSimHud");
                    Hud = hudGo.AddComponent<TacSimHudController>();
                }
            }
            if (!Hud.IsInitialized)
                Hud.BuildHud();

            Hud.BindTradeState(TradeState);
            if (Hud.Overlays != null)
                Hud.Overlays.BindControllers(SurfaceStage, ArenaStage, SpaceStage);
            if (Hud.Minimap != null)
                Hud.Minimap.BindWorldSources(StationWorld, ShipInterior, SpaceStage, SurfaceStage, ArenaStage);
            if (Hud.CharacterSheet != null)
                Hud.CharacterSheet.BindSources(TradeState, SpaceStage, SurfaceStage, ArenaStage);
            if (Hud.TacticalBrackets != null)
                Hud.TacticalBrackets.BindControllers(SpaceStage, SurfaceStage, ArenaStage);

            if (Wayfinder == null)
            {
                Wayfinder = FindAnyObjectByType<TacSimWayfinder3D>();
                if (Wayfinder == null)
                {
                    var wfGo = new GameObject("TacSimWayfinder3D");
                    Wayfinder = wfGo.AddComponent<TacSimWayfinder3D>();
                }
            }
            Wayfinder.EnsureVisuals();

            if (Hud.OpsDirectory != null)
                Hud.OpsDirectory.BindSources(StationWorld, ShipInterior, Wayfinder, TradeState, SpaceStage, SurfaceStage, ArenaStage);

            if (_ws == null)
                _ws = new MushWsClient();
            if (_contentSync == null)
                _contentSync = new TacSimContentSync();
            if (_stageDirector == null)
            {
                _stageDirector = new TacSimStageDirector(
                    StationWorld,
                    ShipInterior,
                    SpaceStage,
                    SurfaceStage,
                    ArenaStage,
                    CameraRig,
                    Player,
                    Audio,
                    Hud);
            }

            BindEvents();
        }

        private void BindEvents()
        {
            if (_eventsBound) return;
            _eventsBound = true;

            TradeState.OnStateChanged += () =>
            {
                if (ShipInterior != null)
                    ShipInterior.UpdateCargoVisualFill(TradeState.HoldMassTonnes);
            };

            Player.OnFocusedInteractableChanged += wi =>
            {
                if (Hud != null)
                {
                    Hud.SetFocusedInteractable(wi);
                    SyncTargetFrameForInteractable(wi);
                }
            };

            StationWorld.OnInteractableTriggered += HandleWorldInteractable;
            ShipInterior.OnInteractableTriggered += HandleWorldInteractable;
            ShipInterior.OnInteriorRebuilt += () =>
            {
                if (CameraRig != null && ShipInterior != null)
                    CameraRig.BridgeViewAnchor = ShipInterior.BridgeCameraAnchor;
            };

            StationWorld.OnPortalCrossed += HandleStationPortalCrossed;
            ShipInterior.OnPortalCrossed += HandleShipPortalCrossed;

            SpaceStage.OnDockingTapeReadyToSubmit += HandleDockingTapeReady;
            SpaceStage.OnDockingAborted += HandleDockingAborted;
            SpaceStage.OnDockingEndedByServer += HandleDockingEndedByServer;
            SpaceStage.OnSpaceAlert += msg =>
            {
                if (Hud != null)
                    Hud.AppendLogLine($"<color=#FFAA44>[HELM]</color> {MushProtocol.EscapeUnityRichText(msg)}");
            };
            SpaceStage.OnOrbitalTargetClicked += (id, isRock) =>
            {
                if (!MushProtocol.TryValidateSafeCommandToken(id, out string safeToken))
                    return;
                if (Audio != null) Audio.PlayRadarScanPing();
                string safeId = MushProtocol.EscapeUnityRichText(safeToken);
                if (isRock)
                {
                    SpaceStage.NotifyApproachOrdered(safeToken);
                    RefreshOrbitalReadout();
                    if (Hud != null)
                        Hud.AppendLogLine($"<color=#66FFCC>[MINING]</color> Ordering approach toward Asteroid Pocket <b>{safeId}</b> — click <b>MATCH VEL</b> (`+helm/match {safeId}`) once inside 2,000 m & ‹ 5 m/s.");
                    if (_ws != null && _ws.IsConnected)
                        _ws.SendLine($"+helm/approach {safeToken}");
                }
                else
                {
                    if (Hud != null)
                        Hud.AppendLogLine($"<color=#FFBB55>[TAC]</color> Requesting target lock on <b>{safeId}</b>...");
                    if (_ws != null && _ws.IsConnected)
                        _ws.SendLine($"+tac/lock {safeToken}");
                }
            };

            SurfaceStage.OnTerrainCellDriveClicked += (sx, sy) =>
            {
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine($"+tv/{SurfaceStage.AddressedRigSlot} drive {sx},{sy} go");
            };
            SurfaceStage.OnSurfaceLogEvent += ln =>
            {
                if (Hud != null)
                    Hud.AppendLogLine($"<color=#FFCC66>[SURFACE]</color> {MushProtocol.EscapeUnityRichText(ln)}");
            };

            ArenaStage.OnLiveCockpitVerbRequested += verb =>
            {
                if (string.IsNullOrWhiteSpace(verb) || MushProtocol.ContainsControlChars(verb))
                    return;
                if (Audio != null) Audio.PlayForCommandLine(verb);
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine(verb);
            };
            ArenaStage.OnWeaponShotFired += isLaser =>
            {
                if (Audio != null)
                {
                    if (isLaser) Audio.PlayLaserPulse();
                    else Audio.PlayRailgunCrack();
                }
                Hud?.FloatingCallouts?.ShowCallout(
                    isLaser ? "LASER STRIKE" : "MAIN GUN SALVO",
                    isLaser ? new Color(1f, 0.36f, 0.42f, 1f) : new Color(1f, 0.84f, 0.34f, 1f));
            };
            ArenaStage.OnRequestArenaMapSubscribe += () =>
            {
                if (_ws != null && _ws.IsConnected)
                    _ws.SendPanelSubscribe("arena.map");
            };
            ArenaStage.OnArenaCombatLog += ln => { if (Hud != null) Hud.AppendLogLine(ln); };
            ArenaStage.OnToggleCameraViewRequested += ToggleGroundVehicleCameraMode;

            Hud.OnAuthSubmitted += HandleAuthSubmitted;
            Hud.OnCharacterSelected += HandleCharacterSelected;
            Hud.OnCommandSubmitted += ExecutePlayerCommand;
            Hud.OnComposingChanged += active =>
            {
                if (_ws != null && _ws.IsConnected)
                    _ws.SendComposing(active);
            };
            Hud.OnRefreshTradeRequested += RequestPortTradeRefresh;
            Hud.OnLoadMoreTradeRequested += HandleLoadMoreTradeRequested;
            Hud.OnMarketBuyRequested += HandleMarketBuyRequested;
            Hud.OnMarketSellRequested += HandleMarketSellRequested;
            Hud.OnRequestChoiceSubmitted += (req, choice) => _ws?.SendChoiceResponse(req, choice);
            Hud.OnCameraModeRequested += SetCameraMode;
            Hud.OnStandFromHelmRequested += StandFromHelm;
            Hud.OnCycleTargetRequested += HandleCycleTarget;
            Hud.OnCycleOrbitalCameraRequested += HandleCycleOrbitalCamera;
            Hud.OnSpaceQuickFireRequested += HandleSpaceQuickFire;

            if (Hud.Minimap != null)
            {
                Hud.Minimap.OnStationRoomClicked += targetRoomEntry =>
                {
                    PlotStationWayfinderRoute(targetRoomEntry, autoWalk: true);
                };
                Hud.Minimap.OnShipCompartmentClicked += targetComp =>
                {
                    PlotShipWayfinderRoute(targetComp, autoWalk: true);
                };
                Hud.Minimap.OnOrbitalBlipClicked += (id, isRock) =>
                {
                    if (!MushProtocol.TryValidateSafeCommandToken(id, out string safeId)) return;
                    ExecutePlayerCommand(isRock ? $"+helm/approach {safeId}" : $"+tac/lock {safeId}");
                };
                Hud.Minimap.OnSurfaceCellClicked += (sx, sy) =>
                {
                    if (SurfaceStage != null && _ws != null && _ws.IsConnected)
                        _ws.SendLine($"+tv/{SurfaceStage.AddressedRigSlot} drive {sx},{sy} go");
                };
                Hud.Minimap.OnOpenSolarChartRequested += () => Hud.SolarMap?.SetOpen(true);
            }

            if (Hud.Overlays != null)
            {
                Hud.Overlays.OnCommandRequested += ExecutePlayerCommand;
                Hud.Overlays.OnExitSurfaceExcursionRequested += ExitPlanetarySurfaceStage;
                Hud.Overlays.OnArenaCockpitVerbRequested += HandleArenaCockpitVerb;
                Hud.Overlays.OnToggleVehicleCameraViewRequested += ToggleGroundVehicleCameraMode;
            }

            if (Hud.SolarMap != null)
            {
                Hud.SolarMap.OnRefreshChartRequested += () =>
                {
                    if (_ws != null && _ws.IsConnected && _ws.HasAdvertisedPanel("sky.chart"))
                        _ws.SendPanelSubscribe("sky.chart");
                };
                Hud.SolarMap.OnCommandRequested += ExecutePlayerCommand;
                Hud.SolarMap.OnOpenSailingPlanRequested += destPort =>
                {
                    if (!string.IsNullOrEmpty(destPort) && TradeState != null)
                        TradeState.SelectedDestinationPort = destPort;
                    OpenVisualTradeTerminal("SAILING PLAN & CORRIDOR ROUTE BUILDER", openSailingTab: true);
                };
            }

            if (Hud.CharacterSheet != null)
            {
                Hud.CharacterSheet.OnAvatarPresetSelected += preset =>
                {
                    _activeAvatarPreset = preset;
                    if (Player != null && Player.VisualSpawner != null)
                        Player.VisualSpawner.BuildPlayerVisual(AssetCatalog, preset);
                    if (Application.isPlaying)
                    {
                        PlayerPrefs.SetInt("TacSim.LastAvatarPreset", (int)preset);
                        PlayerPrefs.Save();
                    }
                    Hud.AppendLogLine($"<color=#66EEAA>[AVATAR]</color> Switched 3D avatar rig to <b>{preset}</b>.");
                };
                Hud.CharacterSheet.OnCommandRequested += ExecutePlayerCommand;
                Hud.CharacterSheet.OnOpenTradeTerminalRequested += () =>
                    OpenVisualTradeTerminal("CERES PORT EXCHANGE & SHIP LOGISTICS", openSailingTab: false);
                Hud.CharacterSheet.OnOpenSailingPlanRequested += () =>
                    OpenVisualTradeTerminal("SAILING PLAN & CORRIDOR ROUTE BUILDER", openSailingTab: true);
                Hud.CharacterSheet.OnOpenSolarChartRequested += () => Hud.SolarMap?.SetOpen(true);
            }

            if (Hud.TacticalBrackets != null)
            {
                Hud.TacticalBrackets.OnOrbitalBracketClicked += (id, isRock) =>
                {
                    if (!MushProtocol.TryValidateSafeCommandToken(id, out string safeId)) return;
                    if (Audio != null) Audio.PlayRadarScanPing();
                    if (isRock)
                    {
                        SpaceStage?.NotifyApproachOrdered(safeId);
                        ExecutePlayerCommand($"+helm/approach {safeId}");
                    }
                    else
                    {
                        ExecutePlayerCommand($"+tac/lock {safeId}");
                    }
                };
                Hud.TacticalBrackets.OnArenaBracketClicked += desig =>
                {
                    if (!MushProtocol.TryValidateSafeCommandToken(desig, out string safeDesig)) return;
                    HandleArenaCockpitVerb($"lock {safeDesig}");
                };
            }

            if (Wayfinder != null)
            {
                Wayfinder.OnArrivedAtDestination += destName =>
                {
                    if (Audio != null) Audio.PlayUiClick();
                    Hud?.AppendLogLine($"<color=#66FFAA>[WAYFINDER]</color> Arrived at <b>{MushProtocol.EscapeUnityRichText(destName)}</b>.");
                    Hud?.FloatingCallouts?.ShowCallout($"ARRIVED: {destName.ToUpperInvariant()}", new Color(0.35f, 1f, 0.68f, 1f));
                    Hud?.OpsDirectory?.SyncWayfinderBanner(ActiveStage);
                };
            }

            if (Hud.OpsDirectory != null)
            {
                Hud.OpsDirectory.OnPlotStationRoomRequested += (roomId, autoWalk) => PlotStationWayfinderRoute(roomId, autoWalk);
                Hud.OpsDirectory.OnPlotShipCompartmentRequested += (compId, autoWalk) => PlotShipWayfinderRoute(compId, autoWalk);
                Hud.OpsDirectory.OnStepWayfinderWalkRequested += StepActiveWayfinderWalk;
                Hud.OpsDirectory.OnClearWayfinderRequested += () =>
                {
                    Wayfinder?.ClearRoute();
                    Hud.OpsDirectory?.SyncWayfinderBanner(ActiveStage);
                };
                Hud.OpsDirectory.OnPresenceTargetRequested += (id, name, isPlayer) =>
                {
                    if (Hud?.UnitFrames == null || string.IsNullOrEmpty(name)) return;
                    Hud.UnitFrames.SetTarget(
                        isPlayer ? "PILOT" : "CAST",
                        isPlayer ? new Color(0.16f, 0.56f, 0.44f, 0.96f) : new Color(0.18f, 0.44f, 0.64f, 0.96f),
                        name,
                        isPlayer ? "Online Pilot (Room Presence)" : "Station / Ship Cast",
                        1f,
                        "IN ROOM",
                        new Color(0.24f, 0.78f, 0.58f, 0.95f),
                        "LOOK",
                        () => ExecutePlayerCommand($"look {name}"),
                        isPlayer ? "PAGE" : "SAY",
                        () => Hud.PrefillChatInput(isPlayer ? $"page {name}=" : $"say {name}, "));
                };
                Hud.OpsDirectory.OnPrefillChatRequested += prefix => Hud?.PrefillChatInput(prefix);
                Hud.OpsDirectory.OnCommandRequested += ExecutePlayerCommand;
            }

            _contentSync.OnSyncLog += msg =>
            {
                if (Hud != null)
                    Hud.AppendLogLine($"<color=#77CCFF>[PACK SYNC]</color> {MushProtocol.EscapeUnityRichText(msg)}");
            };
            _contentSync.OnPacksUpdated += _ => RefreshLoadedContentAfterSync();

            _ws.OnHelloReceived += () =>
            {
                _contentSync.RequestManifestOverWebSocket(_ws, Time.realtimeSinceStartup);
                if (_ws.HasAdvertisedPanel("sky.chart"))
                    _ws.SendPanelSubscribe("sky.chart");
            };
            _ws.OnRoomReceived += HandleServerRoomView;
            _ws.OnNarrativeReceived += HandleServerNarrative;
            _ws.OnChannelReceived += HandleServerChannel;
            _ws.OnComposingReceived += HandleServerComposing;
            _ws.OnSystemLineWithMarkupReceived += HandleServerSystemLine;
            _ws.OnPanelUpdated += HandleServerPanelUpdated;
            _ws.OnServerRequestReceived += req => { if (Hud != null) Hud.ShowServerRequestModal(req); };
            _ws.OnConnectionStatusChanged += status =>
            {
                if (Hud != null)
                    Hud.AppendLogLine($"<color=#88CCFF>[NET]</color> {MushProtocol.EscapeUnityRichText(MushAuthClient.RedactSensitiveTokens(status))}");
            };
            _ws.OnFatalSessionEnded += HandleFatalSessionEnded;
        }

        private bool _awaitingAppendedTradePage;

        private void Update()
        {
            if (_ws != null)
            {
                _ws.PumpMainThread();
                _contentSync?.PumpPackQueue(_ws, Time.realtimeSinceStartup);
                if (Hud != null && _ws.State != _lastBadgeState)
                {
                    _lastBadgeState = _ws.State;
                    Hud.SetConnectionBadge(_ws.State);
                }
            }

            bool textFocused = Hud != null && Hud.IsTextInputFocused;
            _stageDirector?.SyncFrame(_isAboardShip, textFocused);

            if (!textFocused && Audio != null &&
                (Input.GetKey(KeyCode.LeftControl) || Input.GetKey(KeyCode.RightControl)) &&
                Input.GetKeyDown(KeyCode.M))
            {
                bool muted = Audio.ToggleMuted();
                if (Hud != null)
                    Hud.AppendLogLine(muted
                        ? "<color=#88CCFF>[AUDIO]</color> Master audio muted (Ctrl+M to unmute)."
                        : "<color=#88CCFF>[AUDIO]</color> Master audio unmuted.");
            }

            if (_stageDirector != null &&
                (_stageDirector.CurrentStage == TacSimActiveStage.StationWalk ||
                 _stageDirector.CurrentStage == TacSimActiveStage.ShipInteriorWalk))
            {
                Sync3DPositionWithServer();
            }

            if (Hud != null && SpaceStage != null)
            {
                SyncMmorpgHudFramesAndMinimap();
                bool pilotDocking = Player != null && Player.IsSeatedAtHelm && SpaceStage.IsDockingActive;
                Hud.UpdateDockingHud(pilotDocking ? SpaceStage.ActiveDockingSession : null, SpaceStage.GuidanceAssistEnabled);
            }
        }

        private void OnDestroy()
        {
            _ws?.Dispose();
        }

        private float _nextUnitFrameSyncAt;
        private int _arenaCycleIdx = -1;
        private PlayerAvatarPreset _activeAvatarPreset = PlayerAvatarPreset.StellarTrooperFemale;

        private void PlotStationWayfinderRoute(string targetRoomEntry, bool autoWalk)
        {
            if (StationWorld == null || string.IsNullOrEmpty(targetRoomEntry)) return;
            if (string.Equals(targetRoomEntry, StationWorld.ActiveRoomEntryId, StringComparison.OrdinalIgnoreCase)) return;

            Vector3 pPos = Player != null ? Player.transform.position : StationWorld.GetRoomSpawnPosition(StationWorld.ActiveRoomEntryId);
            bool plotted = Wayfinder != null && Wayfinder.PlotStationRoute(StationWorld, StationWorld.ActiveRoomEntryId, targetRoomEntry, pPos);
            if (!plotted) return;

            if (Audio != null) Audio.PlayUiClick();
            Hud?.OpsDirectory?.SyncWayfinderBanner(ActiveStage);
            Hud?.AppendLogLine($"<color=#66EEFF>[WAYFINDER]</color> 3D trail active to <b>{MushProtocol.EscapeUnityRichText(Wayfinder.TargetDisplayName)}</b> ({Wayfinder.RemainingHops} hop{(Wayfinder.RemainingHops == 1 ? "" : "s")}).");

            if (autoWalk)
                StepActiveWayfinderWalk();
        }

        private void PlotShipWayfinderRoute(string targetComp, bool autoWalk)
        {
            if (ShipInterior == null || string.IsNullOrEmpty(targetComp)) return;
            if (string.Equals(targetComp, ShipInterior.ActiveCompartmentId, StringComparison.OrdinalIgnoreCase)) return;

            Vector3 pPos = Player != null ? Player.transform.position : ShipInterior.GetCompartmentSpawnWorldPos(ShipInterior.ActiveCompartmentId);
            bool plotted = Wayfinder != null && Wayfinder.PlotShipRoute(ShipInterior, ShipInterior.ActiveCompartmentId, targetComp, pPos);
            if (!plotted) return;

            if (Audio != null) Audio.PlayUiClick();
            Hud?.OpsDirectory?.SyncWayfinderBanner(ActiveStage);
            Hud?.AppendLogLine($"<color=#66EEFF>[WAYFINDER]</color> Ship deck trail active to <b>{MushProtocol.EscapeUnityRichText(Wayfinder.TargetDisplayName)}</b> ({Wayfinder.RemainingHops} hop{(Wayfinder.RemainingHops == 1 ? "" : "s")}).");

            if (autoWalk)
                StepActiveWayfinderWalk();
        }

        private void StepActiveWayfinderWalk()
        {
            if (Wayfinder == null || !Wayfinder.HasActiveRoute) return;
            if (Wayfinder.Scope == WayfinderScope.Station && StationWorld != null)
            {
                var path = StationWorld.FindExitPath(StationWorld.ActiveRoomEntryId, Wayfinder.TargetId);
                if (path != null && path.Count > 0)
                {
                    var first = _navWalker.PlanAndDispatchOne(NavWalkerScope.Station, _serverStationRoomEntry, path, Time.time);
                    if (first.HasValue)
                        DispatchNavHop(first.Value, 0.35f);
                }
            }
            else if (Wayfinder.Scope == WayfinderScope.Ship && ShipInterior != null)
            {
                var path = ShipInterior.FindExitPath(ShipInterior.ActiveCompartmentId, Wayfinder.TargetId);
                if (path != null && path.Count > 0)
                {
                    var first = _navWalker.PlanAndDispatchOne(NavWalkerScope.Ship, _serverShipCompartment, path, Time.time);
                    if (first.HasValue)
                        DispatchNavHop(first.Value, 0.35f);
                }
            }
        }

        private void SyncTargetFrameForInteractable(WorldInteractable wi)
        {
            if (Hud?.UnitFrames == null) return;
            if (ActiveStage != TacSimActiveStage.StationWalk && ActiveStage != TacSimActiveStage.ShipInteriorWalk)
                return;
            if (wi == null)
            {
                Hud.UnitFrames.ClearTarget();
                return;
            }
            string crest = wi.kind == InteractableKind.NpcActor ? "NPC"
                : (wi.kind == InteractableKind.AirlockBoard || wi.kind == InteractableKind.AirlockDisembark ? "AIRLK"
                : (wi.kind == InteractableKind.BridgeHelmSeat ? "HELM" : "TERM"));
            Color crestCol = wi.kind == InteractableKind.NpcActor
                ? new Color(0.18f, 0.52f, 0.36f, 0.96f)
                : new Color(0.18f, 0.42f, 0.62f, 0.96f);
            Hud.UnitFrames.SetTarget(
                crest,
                crestCol,
                wi.promptTitle ?? "Target",
                wi.promptSubtitle ?? string.Empty,
                1.0f,
                "IN RANGE — PRESS [E]",
                new Color(0.24f, 0.72f, 0.54f, 0.95f),
                "[E] INTERACT",
                () => HandleWorldInteractable(wi),
                "LOOK",
                () => ExecutePlayerCommand("look"));
        }

        private void HandleCycleOrbitalCamera()
        {
            if (ActiveStage != TacSimActiveStage.ShipHelmSeated || CameraRig == null || (SpaceStage != null && SpaceStage.IsDockingActive))
                return;
            CameraRigMode next = CameraRig.Mode == CameraRigMode.ShipBridgeHolo
                ? CameraRigMode.OrbitalExterior
                : (CameraRig.Mode == CameraRigMode.OrbitalExterior
                    ? CameraRigMode.OrbitalTacticalHigh
                    : CameraRigMode.ShipBridgeHolo);
            SetCameraMode(next);
            string lbl = next == CameraRigMode.OrbitalTacticalHigh
                ? "CAMERA: TACTICAL HIGH-ORBIT"
                : (next == CameraRigMode.OrbitalExterior ? "CAMERA: ORBITAL CHASE" : "CAMERA: BRIDGE HOLO-TANK");
            Hud?.FloatingCallouts?.ShowCallout(lbl, new Color(0.38f, 0.92f, 1f, 1f), 1.6f);
        }

        private void HandleSpaceQuickFire()
        {
            if (ActiveStage != TacSimActiveStage.ShipHelmSeated || SpaceStage == null || SpaceStage.IsDockingActive)
                return;
            if (SpaceStage.ActiveLock != null && !string.IsNullOrEmpty(SpaceStage.ActiveLock.ShipId))
            {
                ExecutePlayerCommand("+tac/beam");
                Hud?.FloatingCallouts?.ShowCallout("WEAPONS FREE — FIRING", new Color(1f, 0.44f, 0.28f, 1f));
            }
        }

        private void HandleCycleTarget()
        {
            if (ActiveStage == TacSimActiveStage.ShipHelmSeated && SpaceStage != null)
            {
                if (SpaceStage.TryCycleNextRadarContact(out string id, out bool isRock))
                {
                    if (MushProtocol.TryValidateSafeCommandToken(id, out string safeId))
                    {
                        if (isRock)
                        {
                            SpaceStage.NotifyApproachOrdered(safeId);
                            ExecutePlayerCommand($"+helm/approach {safeId}");
                        }
                        else
                        {
                            ExecutePlayerCommand($"+tac/lock {safeId}");
                        }
                    }
                }
            }
            else if (ActiveStage == TacSimActiveStage.SimulatorArena && ArenaStage?.State != null)
            {
                var contacts = ArenaStage.State.Contacts;
                if (contacts.Count > 0)
                {
                    for (int step = 0; step < contacts.Count; step++)
                    {
                        _arenaCycleIdx = (_arenaCycleIdx + 1) % contacts.Count;
                        var ct = contacts[_arenaCycleIdx];
                        if (ct.Friendly || ct.IsWreck || string.IsNullOrEmpty(ct.Designator)) continue;
                        if (MushProtocol.TryValidateSafeCommandToken(ct.Designator, out string safeDes))
                        {
                            HandleArenaCockpitVerb($"lock {safeDes}");
                            break;
                        }
                    }
                }
            }
            else
            {
                ExecutePlayerCommand("look");
            }
        }

        private void SyncMmorpgHudFramesAndMinimap()
        {
            if (Hud == null) return;
            TacSimActiveStage stage = ActiveStage;

            // Update corner tactical minimap (self-throttled inside SyncMinimap)
            if (Hud.Minimap != null)
            {
                Vector3 pos = Player != null ? Player.transform.position : Vector3.zero;
                float yaw = Player != null ? Player.transform.eulerAngles.y : 0f;
                if (stage == TacSimActiveStage.ShipHelmSeated && SpaceStage != null)
                    yaw = SpaceStage.ShipYawDeg;
                else if (stage == TacSimActiveStage.PlanetarySurface && SurfaceStage?.RoverTransform != null)
                    yaw = SurfaceStage.RoverTransform.eulerAngles.y;

                bool rightCornerBusy =
                    Hud.IsDockingHudVisible ||
                    (Hud.Overlays != null && Hud.Overlays.IsSpaceFireCardVisible) ||
                    stage == TacSimActiveStage.PlanetarySurface ||
                    stage == TacSimActiveStage.SimulatorArena;

                Hud.Minimap.SyncMinimap(stage, pos, yaw, rightCornerBusy);
            }

            // Update 3D Holographic Wayfinder ribbon & compass banner every frame
            if (Wayfinder != null)
            {
                Vector3 wpPos = Player != null ? Player.transform.position : Vector3.zero;
                Wayfinder.SyncProgress(stage, StationWorld, ShipInterior, wpPos, Time.deltaTime);
                Hud.OpsDirectory?.SyncWayfinderBanner(stage);
            }

            // Update 3D screen-space tactical brackets & directional armor/compass widgets every frame
            if (Hud.TacticalBrackets != null)
            {
                Camera activeCam = CameraRig != null && CameraRig.ControlledCamera != null ? CameraRig.ControlledCamera : Camera.main;
                bool hideBrackets = Hud.IsLoginOverlayVisible || Hud.IsModalOpen ||
                    (Hud.SolarMap != null && Hud.SolarMap.IsOpen) ||
                    (Hud.DisplayOptions != null && Hud.DisplayOptions.IsOpen);
                Hud.TacticalBrackets.SyncBrackets(stage, activeCam, hideBrackets);
            }

            if (Hud.UnitFrames == null) return;
            if (Application.isPlaying && Time.unscaledTime < _nextUnitFrameSyncAt) return;
            _nextUnitFrameSyncAt = Time.unscaledTime + 0.15f;

            Hud.OpsDirectory?.UpdateLiveRoom(_lastRoomView, stage);

            if (Hud.CharacterSheet != null)
            {
                string pilotName = _ws != null && _ws.ActiveCharacter != null && !string.IsNullOrEmpty(_ws.ActiveCharacter.Name)
                    ? _ws.ActiveCharacter.Name
                    : "Pilot";
                string locName = _lastRoomView != null && !string.IsNullOrEmpty(_lastRoomView.Name)
                    ? _lastRoomView.Name
                    : (StationWorld?.Pack?.Name ?? "Ceres Freeport");
                Hud.CharacterSheet.SetPilotContext(pilotName, locName, _currentHullBadge, stage, _activeAvatarPreset);
            }

            string creditsStr = TradeState != null ? TradeState.CreditsText : "—";

            switch (stage)
            {
                case TacSimActiveStage.StationWalk:
                case TacSimActiveStage.ShipInteriorWalk:
                {
                    bool aboard = stage == TacSimActiveStage.ShipInteriorWalk;
                    int holdMass = TradeState != null ? TradeState.HoldMassTonnes : 0;
                    int holdCap = TradeState != null ? Mathf.Max(1, TradeState.HoldMassLimitTonnes) : 100;
                    int tank = TradeState != null ? TradeState.TankTonnes : 0;
                    int tankCap = TradeState != null ? Mathf.Max(1, TradeState.TankLimitTonnes) : 80;
                    string title = !string.IsNullOrEmpty(_currentHullBadge) ? _currentHullBadge : (aboard ? "ABOARD HULL" : "PILOT");
                    string sub = aboard
                        ? $"Deck: {(ShipInterior?.ActiveCompartmentId ?? "airlock").ToUpperInvariant()}"
                        : (StationWorld?.Pack?.Name ?? "Ceres Freeport");
                    Hud.UnitFrames.UpdatePlayerFrame(
                        aboard ? "SHIP" : "STN",
                        aboard ? new Color(0.16f, 0.44f, 0.62f, 0.96f) : new Color(0.14f, 0.46f, 0.44f, 0.96f),
                        title,
                        sub,
                        holdMass / (float)holdCap,
                        $"HOLD {holdMass}/{holdCap} t",
                        new Color(0.20f, 0.68f, 0.92f, 0.95f),
                        tank / (float)tankCap,
                        $"FUEL {tank}/{tankCap} t",
                        new Color(0.90f, 0.56f, 0.18f, 0.95f),
                        $"CREDITS: {creditsStr}");
                    break;
                }
                case TacSimActiveStage.ShipHelmSeated:
                {
                    float throttleFrac = SpaceStage != null ? Mathf.Clamp01(SpaceStage.ThrottlePermille / 1000f) : 0f;
                    int tank = TradeState != null ? TradeState.TankTonnes : 0;
                    int tankCap = TradeState != null ? Mathf.Max(1, TradeState.TankLimitTonnes) : 80;
                    string emcon = SpaceStage != null ? SpaceStage.EmconPosture.ToUpperInvariant() : "STD";
                    Hud.UnitFrames.UpdatePlayerFrame(
                        "ORB",
                        new Color(0.18f, 0.34f, 0.64f, 0.96f),
                        !string.IsNullOrEmpty(_currentHullBadge) ? _currentHullBadge : "HELM ACTIVE",
                        $"EMCON {emcon} | TAB: Cycle Contact",
                        throttleFrac,
                        $"TORCH {Mathf.RoundToInt(throttleFrac * 100f)}%",
                        new Color(0.92f, 0.48f, 0.16f, 0.95f),
                        tank / (float)tankCap,
                        $"REACTION {tank}/{tankCap} t",
                        new Color(0.24f, 0.72f, 0.94f, 0.95f),
                        $"CONTACTS: {(SpaceStage?.RadarBlips?.Count ?? 0)} | [M] CHART");

                    if (SpaceStage?.ActiveLock != null && !string.IsNullOrEmpty(SpaceStage.ActiveLock.ShipId))
                    {
                        var lk = SpaceStage.ActiveLock;
                        double rangeKm = lk.RangeM / 1000.0;
                        Hud.UnitFrames.SetTarget(
                            "LOCK",
                            new Color(0.72f, 0.22f, 0.18f, 0.96f),
                            string.IsNullOrEmpty(lk.Label) ? lk.ShipId : lk.Label,
                            $"{rangeKm:0.0} km | Aspect: {lk.Facet.ToUpperInvariant()}",
                            1.0f,
                            $"LOCKED ({lk.Facet.ToUpperInvariant()})",
                            new Color(0.88f, 0.32f, 0.24f, 0.95f),
                            "BEAM",
                            () => ExecutePlayerCommand("+tac/beam"),
                            "SCAN",
                            () => ExecutePlayerCommand("+tac/scan"));
                    }
                    else if (!string.IsNullOrEmpty(SpaceStage?.SelectedRockId))
                    {
                        string rk = SpaceStage.SelectedRockId;
                        Hud.UnitFrames.SetTarget(
                            "ROCK",
                            new Color(0.56f, 0.42f, 0.14f, 0.96f),
                            $"ASTEROID {rk}",
                            "Zero-G Ore Pocket",
                            1f,
                            "MINING TARGET",
                            new Color(0.86f, 0.66f, 0.22f, 0.95f),
                            "MATCH VEL",
                            () => ExecutePlayerCommand($"+helm/match {rk}"),
                            "LASER",
                            () => ExecutePlayerCommand("space:mine_laser"));
                    }
                    else
                    {
                        Hud.UnitFrames.ClearTarget();
                    }
                    break;
                }
                case TacSimActiveStage.PlanetarySurface:
                {
                    int batt = SurfaceStage != null ? SurfaceStage.BatteryPct : 100;
                    int hop = SurfaceStage != null ? SurfaceStage.HopperLoadedTonnes : 0;
                    int hopCap = SurfaceStage != null ? Mathf.Max(1, SurfaceStage.HopperCapacityTonnes) : 6;
                    Hud.UnitFrames.UpdatePlayerFrame(
                        "SURF",
                        new Color(0.48f, 0.34f, 0.14f, 0.96f),
                        $"ROVER RIG {SurfaceStage?.AddressedRigSlot ?? 1}",
                        SurfaceStage?.ActiveSiteTitle ?? "Planetary Surface",
                        batt / 100f,
                        $"BATTERY {batt}%",
                        batt > 30 ? new Color(0.24f, 0.78f, 0.44f, 0.95f) : new Color(0.92f, 0.32f, 0.22f, 0.95f),
                        hop / (float)hopCap,
                        $"HOPPER {hop}/{hopCap} t",
                        new Color(0.24f, 0.74f, 0.92f, 0.95f),
                        $"CELL ({SurfaceStage?.AuthoritativeSiteCellX ?? 0}, {SurfaceStage?.AuthoritativeSiteCellY ?? 0})");
                    Hud.UnitFrames.ClearTarget();
                    break;
                }
                case TacSimActiveStage.SimulatorArena:
                {
                    var st = ArenaStage?.State;
                    if (st != null)
                    {
                        float avgArmor = (st.ArmorFrontPct + st.ArmorLeftPct + st.ArmorRightPct + st.ArmorRearPct) * 0.25f;
                        float heatFrac = st.HeatMax > 0 ? Mathf.Clamp01(st.Heat / (float)st.HeatMax) : 0f;
                        bool isRed = string.Equals(st.Party, "R", StringComparison.OrdinalIgnoreCase);
                        Hud.UnitFrames.UpdatePlayerFrame(
                            isRed ? "RED" : "BLUE",
                            isRed ? new Color(0.68f, 0.20f, 0.20f, 0.96f) : new Color(0.18f, 0.38f, 0.72f, 0.96f),
                            $"{st.Callsign} [{st.Mode.ToUpperInvariant()}]",
                            $"{st.SpeedKph:0} kph | HDG {st.HeadingDeg:000}°",
                            avgArmor / 100f,
                            $"ARMOR {Mathf.RoundToInt(avgArmor)}% (F{st.ArmorFrontPct})",
                            avgArmor > 45f ? new Color(0.22f, 0.76f, 0.44f, 0.95f) : new Color(0.90f, 0.30f, 0.22f, 0.95f),
                            heatFrac,
                            st.Overheated ? "OVERHEATED!" : $"HEAT {Mathf.RoundToInt(heatFrac * 100f)}%",
                            st.Overheated ? new Color(0.96f, 0.22f, 0.18f, 0.95f) : new Color(0.92f, 0.56f, 0.16f, 0.95f),
                            $"RED {st.RedAliveCount} vs BLUE {st.BlueAliveCount} | TAB: Lock");

                        if (!string.IsNullOrEmpty(st.LockedDesignator))
                        {
                            string des = st.LockedDesignator;
                            Hud.UnitFrames.SetTarget(
                                "LOCK",
                                new Color(0.76f, 0.20f, 0.18f, 0.96f),
                                $"CONTACT {des}",
                                $"{st.LockRangeM:0} m | Hit Odds {st.LockHitPercent}%",
                                st.LockHitPercent / 100f,
                                $"SOLUTION {st.LockHitPercent}%",
                                new Color(0.92f, 0.42f, 0.24f, 0.95f),
                                "FIRE",
                                () => HandleArenaCockpitVerb("fire"),
                                "SCAN",
                                () => HandleArenaCockpitVerb("scan"));
                        }
                        else
                        {
                            Hud.UnitFrames.ClearTarget();
                        }
                    }
                    break;
                }
            }
        }

        private void RefreshOrbitalReadout()
        {
            if (Hud?.Overlays == null || SpaceStage == null) return;
            Hud.Overlays.UpdateOrbitalTargetReadout(
                SpaceStage.SelectedRockId,
                SpaceStage.LockedContactId,
                SpaceStage.HasClusterMiningTelemetry ? SpaceStage.AuthoritativeMatchedRockId : null,
                SpaceStage.HasClusterMiningTelemetry ? SpaceStage.AuthoritativeApproachingRockId : null,
                SpaceStage.ActiveLock?.Facet);
        }

        /// <summary>
        /// Resolves orbital mining UI actions (<c>space:helm_match</c>, <c>space:mine_laser</c>, <c>space:mine_drones</c>)
        /// into canonical <c>mushd</c> commands, gating extraction on authoritative RCS velocity match (<c>+helm/match &lt;rock&gt;</c>).
        /// </summary>
        public static string ResolveOrbitalMiningCommand(
            string uiAction,
            string selectedRockId,
            bool hasClusterTelemetry,
            string authoritativeMatchedRockId,
            out string refusalNotice)
        {
            refusalNotice = null;
            if (!MushProtocol.TryValidateSafeCommandToken(selectedRockId, out string validRock))
            {
                refusalNotice = "Select an orbital rock (`A`, `B`, or `C`) first.";
                return null;
            }
            string rock = validRock.ToUpperInvariant();
            string safeRock = MushProtocol.EscapeUnityRichText(rock);

            if (string.Equals(uiAction, "space:helm_match", StringComparison.OrdinalIgnoreCase))
                return $"+helm/match {rock}";

            // Both `+mine/laser` and `+mine/drones` unconditionally require `session.matched_rock == Some(rock)` on `mushd`.
            if (!string.Equals(authoritativeMatchedRockId, rock, StringComparison.OrdinalIgnoreCase))
            {
                refusalNotice = $"Lock RCS velocity match on Rock {safeRock} first (click <b>MATCH VEL</b> / `+helm/match {safeRock}` inside 2,000 m & ‹ 5 m/s).";
                return null;
            }

            if (string.Equals(uiAction, "space:mine_laser", StringComparison.OrdinalIgnoreCase))
                return $"+mine/laser {rock}";
            if (string.Equals(uiAction, "space:mine_drones", StringComparison.OrdinalIgnoreCase))
                return $"+mine/drones {rock} 1";

            refusalNotice = $"Unknown orbital mining action `{MushProtocol.EscapeUnityRichText(uiAction)}`.";
            return null;
        }

        private void RefreshLoadedContentAfterSync()
        {
            var cats = _contentSync?.LastUpdatedCategories;
            bool rebuildAll = cats == null || cats.Count == 0;
            bool dirtyMarket = rebuildAll;
            bool dirtyStation = rebuildAll;
            bool dirtyUnit = rebuildAll;
            bool dirtyRegion = rebuildAll;
            bool dirtyArena = rebuildAll;
            if (!rebuildAll)
            {
                foreach (string c in cats)
                {
                    if (string.Equals(c, "all", StringComparison.OrdinalIgnoreCase))
                    {
                        dirtyMarket = dirtyStation = dirtyUnit = dirtyRegion = dirtyArena = true;
                        break;
                    }
                    if (string.Equals(c, "market", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(c, "economy", StringComparison.OrdinalIgnoreCase)) dirtyMarket = true;
                    if (string.Equals(c, "neighborhood", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(c, "atlas", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(c, "cast", StringComparison.OrdinalIgnoreCase)) dirtyStation = true;
                    if (string.Equals(c, "unit", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(c, "weapon", StringComparison.OrdinalIgnoreCase)) dirtyUnit = true;
                    if (string.Equals(c, "region", StringComparison.OrdinalIgnoreCase)) dirtyRegion = true;
                    if (string.Equals(c, "arena", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(c, "arena_atlas", StringComparison.OrdinalIgnoreCase)) dirtyArena = true;
                }
            }

            if (dirtyMarket)
                TradeState?.ReloadGoodsCatalog();

            if (dirtyStation && StationWorld != null)
            {
                if (!_isAboardShip && _lastRoomView != null)
                {
                    string prevLoadedPack = StationWorld.LoadedPackId;
                    string desiredPack = _lastRoomView.Atlas != null && !string.IsNullOrEmpty(_lastRoomView.Atlas.Pack)
                        ? _lastRoomView.Atlas.Pack
                        : (_lastRoomView.Atlas != null ? _lastRoomView.Atlas.NeighborhoodPack : null);
                    string rebuildPackId = !string.IsNullOrEmpty(desiredPack) ? desiredPack : StationWorld.LoadedPackId;
                    bool rebuiltGeometry = !string.IsNullOrEmpty(rebuildPackId) &&
                                           StationWorld.RebuildStationIfContentChanged(AssetCatalog, rebuildPackId);

                    if (rebuiltGeometry)
                    {
                        _navWalker.Cancel();
                    }

                    if (rebuiltGeometry && StationWorld.TryMatchStationRoom(_lastRoomView, out string syncedEntry))
                    {
                        bool switchedPack = !string.Equals(prevLoadedPack, StationWorld.LoadedPackId, StringComparison.OrdinalIgnoreCase);
                        string physRoom = Player != null
                            ? StationWorld.TryFindContainingRoomWithHysteresis(Player.transform.position, StationWorld.ActiveRoomEntryId)
                            : null;
                        bool playerWalkedAhead = !switchedPack && !string.IsNullOrEmpty(physRoom) &&
                                                 StationWorld.AreRoomsConnectedWithinHops(syncedEntry, physRoom, maxHops: 3);

                        _serverStationRoomEntry = syncedEntry;
                        StationWorld.ApplyRoomView(
                            _lastRoomView,
                            _ws != null && _ws.ActiveCharacter != null ? _ws.ActiveCharacter.Id : null,
                            updateHighlight: !playerWalkedAhead || physRoom == syncedEntry);
                        if (playerWalkedAhead && physRoom != syncedEntry)
                            StationWorld.HighlightActiveRoom(physRoom);

                        if (Player != null && (switchedPack || !playerWalkedAhead))
                        {
                            Player.TeleportTo(StationWorld.GetRoomSpawnPosition(syncedEntry), 0f);
                        }
                        string activeUiRoom = playerWalkedAhead && !string.IsNullOrEmpty(physRoom) ? physRoom : syncedEntry;
                        string surface = _lastRoomView.Atlas != null && !string.IsNullOrEmpty(_lastRoomView.Atlas.Surface)
                            ? _lastRoomView.Atlas.Surface.ToUpperInvariant()
                            : "STATION";
                        Hud?.UpdateTopBar($"{_lastRoomView.Name.ToUpperInvariant()} [{surface}]", $"{_currentClockStr} | {_currentHullBadge}");
                        Hud?.RefreshContextualQuickActions(activeUiRoom, false);
                    }
                }
                else if (!string.IsNullOrEmpty(StationWorld.LoadedPackId))
                {
                    string prevRoom = StationWorld.ActiveRoomEntryId ?? _serverStationRoomEntry;
                    if (StationWorld.RebuildStationIfContentChanged(AssetCatalog, StationWorld.LoadedPackId))
                    {
                        _navWalker.Cancel();
                        if (!string.IsNullOrEmpty(prevRoom))
                            StationWorld.HighlightActiveRoom(prevRoom);
                    }
                }
            }
            if (dirtyUnit && ShipInterior != null && !string.IsNullOrEmpty(ShipInterior.LoadedUnitId))
            {
                _navWalker.Cancel();
                ShipInterior.BuildInterior(AssetCatalog, ShipInterior.LoadedUnitId);
                if (CameraRig != null)
                    CameraRig.BridgeViewAnchor = ShipInterior.BridgeCameraAnchor;
                if (_isAboardShip && _lastRoomView != null)
                    ShipInterior.ApplyRoomView(_lastRoomView, _ws != null && _ws.ActiveCharacter != null ? _ws.ActiveCharacter.Id : null);
            }
            if (dirtyRegion && SurfaceStage != null && !SurfaceStage.IsExcursionActive && !string.IsNullOrEmpty(SurfaceStage.ActiveSiteSlug))
            {
                SurfaceStage.BuildSurfaceStage(AssetCatalog, SurfaceStage.ActiveSiteSlug);
            }
            if (dirtyArena && ArenaStage != null && !ArenaStage.IsArenaActive)
            {
                ArenaStage.Catalog = AssetCatalog;
            }
        }

        /// <summary>
        /// Clears all session-scoped world, trade, space, surface, and arena runtime state so switching accounts
        /// or characters after disconnect never leaks prior state into the next login.
        /// </summary>
        public void ResetSessionState()
        {
            _isAboardShip = false;
            _hullIsDocked = true;
            _awaitingInitialRoomSync = true;
            _limboAutoEnterAttempted = false;
            _lastRoomView = null;
            _serverStationRoomEntry = null;
            _serverShipCompartment = null;
            _navWalker.Cancel();
            _nextMoveStepAllowedAt = 0f;
            _currentHullBadge = "NO CHARTER";
            _awaitingAppendedTradePage = false;
            _lastAutoTradeRefreshAt = -10f;

            TacSimContentLoader.ClearDynamicPacks();
            TradeState?.ResetSessionState();
            if (ArenaStage != null) ArenaStage.ResetSessionState();
            if (SurfaceStage != null) SurfaceStage.ResetSessionState();
            if (SpaceStage != null) SpaceStage.ResetSessionState();
            if (ShipInterior != null) ShipInterior.ResetSessionState();
            if (StationWorld != null) StationWorld.ResetSessionState();
            if (Wayfinder != null) Wayfinder.ClearRoute();
            if (Player != null)
            {
                Player.IsSeatedAtHelm = false;
                Player.SetStagePhysicsSuspended(false);
            }
            if (CameraRig != null)
                CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
            if (Hud != null)
                Hud.RefreshContextualQuickActions(string.Empty, false);
        }

        private async void HandleAuthSubmitted(string baseUrl, string account, string password, string charName, bool isRegister, PlayerAvatarPreset avatar)
        {
            if (_authInFlight || _connectInFlight)
            {
                if (Hud != null)
                    Hud.SetLoginStatus("Authentication already in progress...");
                return;
            }
            if (string.IsNullOrWhiteSpace(account) || string.IsNullOrEmpty(password))
            {
                Hud.SetLoginStatus("Please enter both Account Name and Password.");
                return;
            }
            if (isRegister && string.IsNullOrWhiteSpace(charName))
            {
                Hud.SetLoginStatus("Please enter a Character Name to create with your new account.");
                return;
            }

            _authInFlight = true;
            try
            {
                string safeAccount = MushProtocol.EscapeUnityRichText(account);
                string safeUrl = MushProtocol.EscapeUnityRichText(MushAuthClient.SanitizeUrlForDisplayOrStorage(baseUrl));
                Hud.SetLoginStatus(isRegister ? $"Registering '{safeAccount}' on {safeUrl}..." : $"Signing in to {safeUrl} as '{safeAccount}'...");
                MushAuthSession session = isRegister
                    ? await MushAuthClient.RegisterAsync(baseUrl, account, password, charName)
                    : await MushAuthClient.LoginAsync(baseUrl, account, password);

                if (session.Characters.Count == 0)
                {
                    Hud.SetLoginStatus("Login succeeded, but account has no characters.");
                    return;
                }

                MushCharacterSlot exactMatch = null;
                if (!string.IsNullOrWhiteSpace(charName))
                {
                    for (int i = 0; i < session.Characters.Count; i++)
                    {
                        if (string.Equals(session.Characters[i].Name, charName, StringComparison.OrdinalIgnoreCase))
                        {
                            exactMatch = session.Characters[i];
                            break;
                        }
                    }
                }

                _authInFlight = false;
                if (exactMatch != null)
                    HandleCharacterSelected(session, exactMatch, avatar);
                else if (session.Characters.Count == 1)
                    HandleCharacterSelected(session, session.Characters[0], avatar);
                else
                    Hud.ShowCharacterPicker(session);
            }
            catch (Exception ex)
            {
                string redacted = MushAuthClient.RedactSensitiveTokens(ex.ToString());
                Debug.LogWarning($"[TacSim Auth] {redacted}");
                string safeMsg = MushProtocol.EscapeUnityRichText(MushAuthClient.RedactSensitiveTokens(ex.Message));
                Hud.SetLoginStatus($"<color=#FF8877>{safeMsg}</color>");
            }
            finally
            {
                _authInFlight = false;
            }
        }

        private async void HandleCharacterSelected(MushAuthSession session, MushCharacterSlot chosen, PlayerAvatarPreset avatar)
        {
            if (_connectInFlight)
            {
                if (Hud != null)
                    Hud.SetLoginStatus("Connection already in progress...");
                return;
            }

            _connectInFlight = true;
            try
            {
                ResetSessionState();

                string safeUrl = MushProtocol.EscapeUnityRichText(MushAuthClient.SanitizeUrlForDisplayOrStorage(session.BaseHttpUrl));
                string safeChar = MushProtocol.EscapeUnityRichText(chosen.Name);
                string safeAcct = MushProtocol.EscapeUnityRichText(session.Account);
                Hud.SetLoginStatus($"Loading server pack cache for <b>{safeUrl}</b>...");
                _contentSync.InitializeDiskCacheForServer(session.BaseHttpUrl);

                Hud.SetLoginStatus($"Attaching WebSocket to <b>{safeChar}</b>...");
                _activeAvatarPreset = avatar;
                Player.VisualSpawner.BuildPlayerVisual(AssetCatalog, avatar);
                Hud.CharacterSheet?.SetPilotContext(chosen.Name, "Ceres Freeport", _currentHullBadge, ActiveStage, avatar);
                await _ws.ConnectAsync(session, chosen.Key, resetResume: true);
                Hud.SetConnectionBadge(_ws.State);
                Hud.ShowLoginOverlay(false);
                Hud.AppendLogLine($"<color=#66FFAA>[CONNECTED]</color> Attached as <b>{safeChar}</b> ({safeAcct}) on {safeUrl}.");
            }
            catch (Exception ex)
            {
                string redacted = MushAuthClient.RedactSensitiveTokens(ex.ToString());
                Debug.LogWarning($"[TacSim WS] {redacted}");
                string safeMsg = MushProtocol.EscapeUnityRichText(MushAuthClient.RedactSensitiveTokens(ex.Message));
                Hud.SetLoginStatus($"<color=#FF8877>WebSocket connect failed: {safeMsg}</color>");
            }
            finally
            {
                _connectInFlight = false;
            }
        }

        private void HandleFatalSessionEnded(string reason)
        {
            ResetSessionState();
            string safeReason = MushProtocol.EscapeUnityRichText(MushAuthClient.RedactSensitiveTokens(reason));
            string display = string.IsNullOrWhiteSpace(safeReason)
                ? "<color=#FF8877>Session ended by mushd — please sign in again.</color>"
                : $"<color=#FF8877>{safeReason}</color>";
            if (Hud != null)
                Hud.ShowLoginOverlay(true, display);
        }

        private void DispatchNavHop(QueuedNavHop hop, float cooldownSec)
        {
            _nextMoveStepAllowedAt = Time.time + cooldownSec;
            OnCommandDispatched?.Invoke(hop.Command);
            if (_ws != null && _ws.IsConnected)
                _ws.SendLine(hop.Command);
        }

        private void Sync3DPositionWithServer()
        {
            if (Player == null || Player.PhysicsSuspended || Player.IsSeatedAtHelm || SpaceStage.IsDockingActive)
                return;
            bool wsReady = (_ws != null && _ws.IsConnected) || _simulateConnectedForTests;
            if (_awaitingInitialRoomSync || !wsReady)
                return;

            // Strictly one outbound room-bound move step in flight at a time so multi-hop corridors
            // never blast ahead or overshoot past the player's physical room before `RoomView` confirms!
            _navWalker.ExpireIfStale(Time.time);
            bool moveInFlight = _navWalker.HasInFlight(Time.time);

            if (_isAboardShip)
            {
                if (string.IsNullOrEmpty(_serverShipCompartment)) return;
                string physComp = ShipInterior.TryFindContainingCompartment(Player.transform.position, ShipInterior.ActiveCompartmentId, hysteresisM: 0.55f);
                if (string.IsNullOrEmpty(physComp) || physComp == _serverShipCompartment)
                    return;

                if (moveInFlight || Time.time < _nextMoveStepAllowedAt)
                    return;

                var path = ShipInterior.FindExitPath(_serverShipCompartment, physComp);
                var first = _navWalker.PlanAndDispatchOne(NavWalkerScope.Ship, _serverShipCompartment, path, Time.time);
                if (first.HasValue)
                    DispatchNavHop(first.Value, 0.35f);
            }
            else
            {
                if (string.IsNullOrEmpty(_serverStationRoomEntry)) return;
                string physRoom = StationWorld.TryFindContainingRoomWithHysteresis(Player.transform.position, StationWorld.ActiveRoomEntryId);
                if (string.IsNullOrEmpty(physRoom)) return;

                if (physRoom != StationWorld.ActiveRoomEntryId)
                {
                    StationWorld.HighlightActiveRoom(physRoom);
                    Hud.RefreshContextualQuickActions(physRoom, false);
                }

                if (physRoom == _serverStationRoomEntry || moveInFlight || Time.time < _nextMoveStepAllowedAt)
                    return;

                var path = StationWorld.FindExitPath(_serverStationRoomEntry, physRoom);
                var first = _navWalker.PlanAndDispatchOne(NavWalkerScope.Station, _serverStationRoomEntry, path, Time.time);
                if (first.HasValue)
                    DispatchNavHop(first.Value, 0.35f);
            }
        }

        private void FlushPendingRoomMovesBeforeAction()
        {
            bool wsReady = (_ws != null && _ws.IsConnected) || _simulateConnectedForTests;
            if (!wsReady || _awaitingInitialRoomSync || Player == null)
                return;

            _navWalker.ExpireIfStale(Time.time);

            if (_isAboardShip)
            {
                if (string.IsNullOrEmpty(_serverShipCompartment))
                    return;
                string startHint = !string.IsNullOrEmpty(_navWalker.InFlightTarget) ? _navWalker.InFlightTarget : _serverShipCompartment;
                string physComp = ShipInterior.TryFindContainingCompartment(Player.transform.position, startHint, hysteresisM: 0.1f);
                if (string.IsNullOrEmpty(physComp))
                    return;

                if (_navWalker.HasInFlight(Time.time))
                {
                    string inFlightTarget = _navWalker.InFlightTarget;
                    if (!string.IsNullOrEmpty(inFlightTarget) && physComp != inFlightTarget)
                    {
                        var tail = ShipInterior.FindExitPath(inFlightTarget, physComp);
                        _navWalker.ReplaceQueuedTailWhileInFlight(NavWalkerScope.Ship, tail, Time.time);
                    }
                    return;
                }

                if (physComp != _serverShipCompartment)
                {
                    var hops = ShipInterior.FindExitPath(_serverShipCompartment, physComp);
                    var first = _navWalker.PlanAndDispatchOne(NavWalkerScope.Ship, _serverShipCompartment, hops, Time.time);
                    if (first.HasValue)
                        DispatchNavHop(first.Value, 0.6f);
                }
            }
            else
            {
                if (string.IsNullOrEmpty(_serverStationRoomEntry))
                    return;
                string startHint = !string.IsNullOrEmpty(_navWalker.InFlightTarget) ? _navWalker.InFlightTarget : _serverStationRoomEntry;
                string physRoom = StationWorld.TryFindContainingRoomWithHysteresis(Player.transform.position, startHint, hysteresisM: 0.1f);
                if (string.IsNullOrEmpty(physRoom))
                    return;

                if (_navWalker.HasInFlight(Time.time))
                {
                    string inFlightTarget = _navWalker.InFlightTarget;
                    if (!string.IsNullOrEmpty(inFlightTarget) && physRoom != inFlightTarget)
                    {
                        var tail = StationWorld.FindExitPath(inFlightTarget, physRoom);
                        _navWalker.ReplaceQueuedTailWhileInFlight(NavWalkerScope.Station, tail, Time.time);
                    }
                    return;
                }

                if (physRoom != _serverStationRoomEntry)
                {
                    var hops = StationWorld.FindExitPath(_serverStationRoomEntry, physRoom);
                    var first = _navWalker.PlanAndDispatchOne(NavWalkerScope.Station, _serverStationRoomEntry, hops, Time.time);
                    if (first.HasValue)
                        DispatchNavHop(first.Value, 0.6f);
                }
            }
        }

        private void HandleStationPortalCrossed(ExitPortalTrigger portal, string targetRoomEntry, string commandAlias) { }
        private void HandleShipPortalCrossed(ExitPortalTrigger portal, string targetCompartment, string commandAlias) { }

        private void HandleWorldInteractable(WorldInteractable wi)
        {
            if (wi == null) return;
            switch (wi.kind)
            {
                case InteractableKind.RoomDetailPlaque:
                    if (_ws != null && _ws.IsConnected &&
                        !string.IsNullOrEmpty(wi.commandOrKey) &&
                        wi.commandOrKey.StartsWith("look ", StringComparison.OrdinalIgnoreCase) &&
                        MushProtocol.TryValidateSafeKeywordOrName(wi.commandOrKey.Substring(5), out string safePlaqueKw))
                    {
                        FlushPendingRoomMovesBeforeAction();
                        _ws.SendLine($"look {safePlaqueKw}");
                    }
                    string safePlaqueTitle = MushProtocol.EscapeUnityRichText((wi.promptTitle ?? string.Empty).ToUpperInvariant());
                    string safePlaqueBody = string.IsNullOrEmpty(wi.detailBody)
                        ? "Inspecting..."
                        : MushProtocol.EscapeUnityRichText(wi.detailBody);
                    Hud.ShowDetailModal(safePlaqueTitle, safePlaqueBody);
                    break;

                case InteractableKind.NpcActor:
                    OpenNpcConversationModal(wi);
                    break;

                case InteractableKind.StationServiceDesk:
                    OpenServiceDeskModal(wi.commandOrKey);
                    break;

                case InteractableKind.AirlockBoard:
                    ExecutePlayerCommand("+ship/board");
                    break;

                case InteractableKind.AirlockDisembark:
                    ExecutePlayerCommand("+ship/leave");
                    break;

                case InteractableKind.BridgeHelmSeat:
                    SitAtBridgeHelm();
                    break;

                case InteractableKind.SimulatorPodBoard:
                    ExecutePlayerCommand("board");
                    break;

                case InteractableKind.VehicleBayRig:
                    Hud.ShowDetailModal(
                        "DRAY-P VEHICLE BAY — PROSPECTOR RIG CRADLE",
                        "Launch your tracked prospecting rig down the stern ramp onto a planetary surface site (`Occator Rim`, `Rheasilvia`, `Ahuna Flank`):",
                        new List<(string, Action)>
                        {
                            ("Launch Down Stern Ramp (+tv/1 launch go)", () => ExecutePlayerCommand("surface:launch")),
                            ("Lease Rig (+tv/lease go)", () => ExecutePlayerCommand("+tv/lease go")),
                            ("Return to Hold (+tv/bay out)", () => ExecutePlayerCommand("+tv/bay out"))
                        });
                    break;

                default:
                    if (!string.IsNullOrEmpty(wi.commandOrKey))
                        ExecutePlayerCommand(wi.commandOrKey);
                    break;
            }
        }

        private void OpenNpcConversationModal(WorldInteractable wi)
        {
            bool hasValidName = MushProtocol.TryValidateSafeKeywordOrName(wi.commandOrKey, out string name);
            string displayName = hasValidName ? name : (wi.commandOrKey ?? string.Empty);
            string safeName = MushProtocol.EscapeUnityRichText(displayName);
            string roleNote = !string.IsNullOrWhiteSpace(wi.detailBody)
                ? $"<color=#88CCEE>{MushProtocol.EscapeUnityRichText(wi.detailBody)}</color>\n"
                : string.Empty;

            var topics = new List<(string, Action)>();
            if (hasValidName)
            {
                if (name.IndexOf("Ilse", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    topics.Add(("How do I enrol a Dray?", () => ExecutePlayerCommand($"say {name}, how do I enrol for a Co-op Dray?")));
                    topics.Add(("Explain the lay & buyout", () => ExecutePlayerCommand($"say {name}, how do the lay and buyout work?")));
                    topics.Add(("Check my statement", () => ExecutePlayerCommand($"say {name}, can you check where my Co-op statement stands?")));
                }
                else if (name.IndexOf("Qureshi", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    topics.Add(("Ask about volatiles & fuel", () => ExecutePlayerCommand($"say {name}, what are Occator's volatiles and reaction mass lines looking like?")));
                    topics.Add(("Ask about +survey work", () => ExecutePlayerCommand($"say {name}, what survey commissions is Occator running?")));
                }
                else if (name.IndexOf("Margit", StringComparison.OrdinalIgnoreCase) >= 0 || name.IndexOf("Søndergaard", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    topics.Add(("Ask about concentrate & alloy", () => ExecutePlayerCommand($"say {name}, how are the concentrate ask and Freeport Works alloy bid looking?")));
                    topics.Add(("Which claims consign here?", () => ExecutePlayerCommand($"say {name}, which Belt claims consigned the concentrate on the board?")));
                }
                else if (name.IndexOf("Dov", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    topics.Add(("Ask about provisions cases", () => ExecutePlayerCommand($"say {name}, how many cases of provisions do I need for Stickney?")));
                    topics.Add(("Ask about bunkering schedule", () => ExecutePlayerCommand($"say {name}, how do I put reaction mass on my loading schedule?")));
                }
                else if (name.IndexOf("Halvorsen", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    topics.Add(("Ask about Clearing & +credit", () => ExecutePlayerCommand($"say {name}, how does Ceres Clearing handle pilot accounts?")));
                    topics.Add(("What is dry money?", () => ExecutePlayerCommand($"say {name}, what do people mean by dry money versus wet money?")));
                }
                else if (name.IndexOf("Mbatha", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    topics.Add(("Ask about hull valuation", () => ExecutePlayerCommand($"say {name}, how does Tycho Mutual value a Dray on the board?")));
                    topics.Add(("Ask about Re-float Program", () => ExecutePlayerCommand($"say {name}, how does the Re-float Program work?")));
                }
                else if (name.IndexOf("Petrossian", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    topics.Add(("Ask about bunks & crew calls", () => ExecutePlayerCommand($"say {name}, any bunks open or runs on the job board tonight?")));
                    topics.Add(("Who's in port?", () => ExecutePlayerCommand($"say {name}, who got back to port last?")));
                }
                else
                {
                    topics.Add(("Ask about First Captain hulls", () => ExecutePlayerCommand($"say {name}, how do I charter a Co-op Dray?")));
                    topics.Add(("Ask about Stickney & Kirkwood lane", () => ExecutePlayerCommand($"say {name}, how is the lane to Stickney looking?")));
                }
            }

            Hud.ShowDetailModal(
                safeName.ToUpperInvariant(),
                $"{roleNote}Speak directly with <b>{safeName}</b>. Type a question below or choose a quick topic:",
                topics,
                npcName: hasValidName ? name : null);
        }

        private static string SuggestUniqueHullName()
        {
            string[] prefixes = { "Drift", "Quiet", "Cinder", "Pale", "Copper", "Vesta", "Tether", "Iron" };
            string[] nouns = { "Lantern", "Kestrel", "Barge", "Moth", "Spindle", "Compass", "Anchor", "Cairn" };
            int seed = Environment.TickCount & int.MaxValue;
            return $"{prefixes[seed % prefixes.Length]} {nouns[(seed / 13) % nouns.Length]} {(seed % 89) + 10}";
        }

        private void ShowCoopEnrolPrompt()
        {
            string suggested = SuggestUniqueHullName();
            Hud.ShowDetailModal(
                "CO-OP FIRST CAPTAIN PROGRAMME DESK",
                "The Co-op stakes the Dray hull and your first hold; it takes its lay from settled sales until buyout.\n\nEnter your hull's registry name below (must be unique on the server) and click <b>Enrol Charter Hull</b>:",
                new List<(string, Action)>
                {
                    ("Read Terms (+coop)", () => ExecutePlayerCommand("+coop")),
                    ("Enrol Charter Hull", () =>
                    {
                        string chosenName = Hud.GetModalInputValue();
                        if (string.IsNullOrWhiteSpace(chosenName))
                            chosenName = suggested;
                        ExecutePlayerCommand($"+coop/enroll {chosenName}");
                    }),
                    ("Account Statement", () => ExecutePlayerCommand("+coop/statement"))
                },
                commandPrefix: "+coop/enroll",
                defaultInputValue: suggested);
        }

        private void OpenServiceDeskModal(string deskKey)
        {
            Audio?.PlayUiClick();
            switch (deskKey)
            {
                case "desk:coop":
                    ShowCoopEnrolPrompt();
                    break;

                case "desk:exchange":
                    OpenVisualTradeTerminal("FREEPORT COMMODITY EXCHANGE", openSailingTab: false);
                    break;

                case "desk:chandlery":
                    OpenVisualTradeTerminal("CO-OP CHANDLERY & OCCATOR BUNKERING", openSailingTab: false);
                    break;

                case "desk:dockmaster":
                    OpenVisualTradeTerminal("DOCKMASTER — SAILING PLANS & CLEARANCES", openSailingTab: true);
                    break;

                case "desk:refit":
                    Hud.ShowDetailModal(
                        "STICKNEY YARD REFIT & SERVICE DESK",
                        "Refit a chartered Dray into a <b>Dray-P Prospector</b> (adds 2-rig stern Vehicle Bay + 86 t ore hold), inspect Stickney yard works, or review rig status (Occator terrain rigs are leased at Ceres Freeport Exchange):",
                        new List<(string, Action)>
                        {
                            ("Refit Dray-P (+coop/refit prospector)", () => ExecutePlayerCommand("+coop/refit prospector")),
                            ("Inspect Works (+works)", () => ExecutePlayerCommand("+works")),
                            ("Rig Status (+tv)", () => ExecutePlayerCommand("+tv")),
                            ("Check Ship Status (+ship)", () => ExecutePlayerCommand("+ship"))
                        });
                    break;

                case "desk:sim_lobby":
                    Hud.Overlays?.ShowArenaLobbyModal(true);
                    break;

                default:
                    ExecutePlayerCommand(deskKey);
                    break;
            }
        }

        private void OpenVisualTradeTerminal(string title, bool openSailingTab)
        {
            _awaitingAppendedTradePage = false;
            Hud.OpenTradeAndSailingModal(title, openSailingTab);
            TradeState.NotifyMarketRefreshRequested();
            if (_ws == null || !_ws.IsConnected)
                return;

            FlushPendingRoomMovesBeforeAction();
            if (_ws.HasAdvertisedPanel("port.trade"))
            {
                _ws.SendPanelSubscribe("port.trade");
                if (openSailingTab)
                    _ws.SendLine("+ship/orders");
            }
            else
            {
                _ws.SendLine("+credit");
                _ws.SendLine("+ship");
                if (openSailingTab)
                    _ws.SendLine("+ship/orders");
                else
                    _ws.SendLine("+market/list");
            }
        }

        private void RequestPortTradeRefresh()
        {
            _awaitingAppendedTradePage = false;
            if (_ws == null || !_ws.IsConnected)
                return;

            FlushPendingRoomMovesBeforeAction();
            if (_ws.HasAdvertisedPanel("port.trade"))
            {
                _ws.SendPanelSubscribe("port.trade");
            }
            else
            {
                _ws.SendLine("+market/list");
                _ws.SendLine("+ship");
                _ws.SendLine("+credit");
            }
        }

        private void HandleLoadMoreTradeRequested(ulong afterListingId)
        {
            if (_ws == null || !_ws.IsConnected || !_ws.HasAdvertisedPanel("port.trade"))
                return;
            _awaitingAppendedTradePage = true;
            var p = JsonNode.NewObject();
            p["after"] = JsonNode.FromLong((long)afterListingId);
            _ws.SendPanelSubscribe("port.trade", p);
        }

        private void HandleMarketBuyRequested(MarketListingRow row, int quantity, bool maxFit)
        {
            if (row == null) return;
            if (Audio != null) Audio.PlayMarketChime();
            long unitMinor = row.UnitPriceMinor > 0 ? row.UnitPriceMinor : (long)Math.Round(row.UnitPriceMajor * 100m);
            string quoteCmd = TradeState.BeginOneClickBuy(row.ListingId, quantity, Time.time, unitMinor);
            if (string.IsNullOrEmpty(quoteCmd))
            {
                Hud.AppendLogLine("<color=#FFAA55>[MARKET]</color> Cannot place 1-click buy without a known positive listing price; refresh the trade board first.");
                return;
            }
            ExecutePlayerCommand(quoteCmd);
            if (_ws != null && _ws.IsConnected && _ws.HasAdvertisedPanel("port.trade"))
            {
                _awaitingAppendedTradePage = false;
                _ws.SendPanelSubscribe("port.trade");
            }
        }

        private void HandleMarketSellRequested(MarketListingRow row, int quantity, bool sellAll)
        {
            if (row == null) return;
            Audio?.PlayUiClick();
            string sellCmd = TacSimTradeAndShipState.FormatOneClickSellCommand(row.ListingId, quantity, sellAll);
            string safeItem = MushProtocol.EscapeUnityRichText(row.ItemName);
            string safeBuyer = MushProtocol.EscapeUnityRichText(string.IsNullOrEmpty(row.VendorName) ? row.Venue : row.VendorName);
            string safePrice = MushProtocol.EscapeUnityRichText(row.UnitPriceText);
            string qtyWord = sellAll ? "ALL available hold stock" : $"{Mathf.Max(1, quantity)} unit(s)";

            Hud.ShowDetailModal(
                "CONFIRM MARKET SALE",
                $"Sell <b>{qtyWord}</b> of <b>{safeItem}</b> into Bid <b>#{row.ListingId}</b> ({safeBuyer}) at <b>{safePrice}</b> per unit?\n\nCommand: <b>{MushProtocol.EscapeUnityRichText(sellCmd)}</b>",
                new List<(string, Action)>
                {
                    ("CONFIRM SALE", () =>
                    {
                        ExecutePlayerCommand(sellCmd);
                        if (_ws != null && _ws.IsConnected && _ws.HasAdvertisedPanel("port.trade"))
                        {
                            _awaitingAppendedTradePage = false;
                            _ws.SendPanelSubscribe("port.trade");
                        }
                    }),
                    ("BACK TO EXCHANGE", () => OpenVisualTradeTerminal("PORT EXCHANGE & HOLD LOGISTICS", openSailingTab: false))
                });
        }

        public void SendRawWireCommand(string cmd)
        {
            if (string.IsNullOrWhiteSpace(cmd)) return;
            if (Audio != null) Audio.PlayForCommandLine(cmd);
            if (cmd.StartsWith("+market/list", StringComparison.OrdinalIgnoreCase))
                TradeState?.NotifyMarketRefreshRequested();

            string safeCmd = MushProtocol.EscapeUnityRichText(cmd);
            if (Hud != null)
            {
                Hud.AppendLogLine($"<color=#66CCFF>> {safeCmd}</color>");
                if (Hud.IsModalOpen)
                    Hud.AppendModalResponseLine($"<color=#66CCFF>> {safeCmd}</color>", resetPreviousResponses: true);
            }

            OnCommandDispatched?.Invoke(cmd);

            if (_ws != null && _ws.IsConnected)
            {
                FlushPendingRoomMovesBeforeAction();
                _navWalker.Cancel();
                _ws.SendLine(cmd);
            }
            else
            {
                _navWalker.Cancel();
                LogDisconnectedCommandNotice();
            }
        }

        private void LogDisconnectedCommandNotice()
        {
            const string msg = "<color=#FF7766>[NET] Disconnected from mushd — command not sent.</color>";
            if (Hud != null)
            {
                Hud.AppendLogLine(msg);
                if (Hud.IsModalOpen)
                    Hud.AppendModalResponseLine(msg);
            }
        }

        private static bool IsKnownOreFamilyToken(string token)
        {
            return string.Equals(token, "volatiles", StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(token, "metals", StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(token, "rare", StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(token, "silicates", StringComparison.OrdinalIgnoreCase) ||
                   string.Equals(token, "all", StringComparison.OrdinalIgnoreCase);
        }

        public static string NormalizeCorridorPortSlug(string rawHint)
        {
            if (string.IsNullOrWhiteSpace(rawHint)) return null;
            string low = rawHint.Trim().ToLowerInvariant();
            if (low.Contains("stickney") || low.Contains("phobos")) return "stickney";
            if (low.Contains("aitken") || low.Contains("luna")) return "luna";
            if (low.Contains("voltaire") || low.Contains("deimos")) return "deimos";
            if (low.Contains("ligeia") || low.Contains("titan")) return "titan";
            if (low.Contains("rheasilvia") || low.Contains("vesta")) return "rheasilvia";
            if (low.Contains("kerwan")) return "kerwan";
            if (low.Contains("ahuna")) return "ahuna";
            if (low.Contains("juling")) return "juling";
            if (low.Contains("occator")) return "occator";
            if (low.Contains("urvara")) return "urvara";
            if (low.Contains("commons")) return "commons";
            if (low.Contains("ceres") || low.Contains("freeport")) return "ceres";
            return null;
        }

        public static string ResolveDockingTargetPort(
            string activeSpaceSiteId,
            string aboardDockedPortHint,
            string activeVenue,
            string loadedStationPackId,
            string selectedDestinationPort)
        {
            return NormalizeCorridorPortSlug(activeSpaceSiteId)
                ?? NormalizeCorridorPortSlug(aboardDockedPortHint)
                ?? NormalizeCorridorPortSlug(activeVenue)
                ?? NormalizeCorridorPortSlug(loadedStationPackId)
                ?? NormalizeCorridorPortSlug(selectedDestinationPort)
                ?? "ceres";
        }

        public void ExecutePlayerCommand(string cmd)
        {
            if (string.IsNullOrWhiteSpace(cmd)) return;

            if (cmd == "ui:open_coop_enrol")
            {
                if (Audio != null) Audio.PlayUiClick();
                ShowCoopEnrolPrompt();
                return;
            }
            if (cmd == "ui:refresh_trade")
            {
                if (Audio != null) Audio.PlayUiClick();
                RequestPortTradeRefresh();
                return;
            }
            if (cmd == "ui:open_exchange")
            {
                if (Audio != null) Audio.PlayUiClick();
                OpenVisualTradeTerminal("PORT EXCHANGE & HOLD LOGISTICS", openSailingTab: false);
                return;
            }
            if (cmd == "ui:open_sailing")
            {
                if (Audio != null) Audio.PlayUiClick();
                OpenVisualTradeTerminal("SAILING PLAN & CORRIDOR ROUTE BUILDER", openSailingTab: true);
                return;
            }
            if (cmd == "seat:helm")
            {
                if (Audio != null) Audio.PlayUiClick();
                SitAtBridgeHelm();
                return;
            }
            if (cmd == "surface:launch")
            {
                if (Audio != null) Audio.PlayUiClick();
                if (_ws != null && _ws.IsConnected)
                {
                    FlushPendingRoomMovesBeforeAction();
                    int slot = SurfaceStage != null ? SurfaceStage.AddressedRigSlot : 1;
                    _ws.SendLine($"+tv/{slot} launch go");
                }
                else
                {
                    LogDisconnectedCommandNotice();
                }
                return;
            }
            if (cmd == "surface:scan" || cmd == "surface:sound" || cmd == "surface:mine" ||
                cmd == "surface:unload" || cmd == "surface:recover" || cmd == "surface:claim_stake")
            {
                if (Audio != null && (cmd == "surface:scan" || cmd == "surface:sound"))
                    Audio.PlayRadarScanPing();
                int slot = SurfaceStage != null ? SurfaceStage.AddressedRigSlot : 1;
                string realCmd = cmd == "surface:scan" ? $"+tv/{slot} scan go"
                               : cmd == "surface:sound" ? $"+tv/{slot} sound go"
                               : cmd == "surface:mine" ? $"+tv/{slot} mine go"
                               : cmd == "surface:unload" ? $"+tv/{slot} unload go"
                               : cmd == "surface:recover" ? $"+tv/{slot} recover go"
                               : $"+claim/stake {slot} go";
                ExecutePlayerCommand(realCmd);
                return;
            }
            if (cmd.StartsWith("surface:family:", StringComparison.OrdinalIgnoreCase))
            {
                string fam = cmd.Substring("surface:family:".Length).Trim();
                if (IsKnownOreFamilyToken(fam))
                {
                    if (Audio != null) Audio.PlayUiClick();
                    SurfaceStage?.SetOreFamilyFilter(fam);
                    if (_ws != null && _ws.IsConnected)
                        SendSiteMapPull(fam);
                    else
                        LogDisconnectedCommandNotice();
                    return;
                }
            }
            if (cmd == "space:mine_ping")
            {
                if (Audio != null) Audio.PlayRadarScanPing();
                SetCameraMode(CameraRigMode.OrbitalExterior);
                if (_ws != null && _ws.IsConnected) _ws.SendLine("+mine/ping");
                else LogDisconnectedCommandNotice();
                return;
            }
            if (cmd.StartsWith("space:rock:", StringComparison.OrdinalIgnoreCase))
            {
                string rawRock = cmd.Substring("space:rock:".Length);
                if (MushProtocol.TryValidateSafeCommandToken(rawRock, out string validRock))
                {
                    string rId = validRock.ToUpperInvariant();
                    if (Audio != null) Audio.PlayUiClick();
                    SpaceStage.NotifyApproachOrdered(rId);
                    RefreshOrbitalReadout();
                    if (_ws != null && _ws.IsConnected)
                        _ws.SendLine($"+helm/approach {rId}");
                    else
                        LogDisconnectedCommandNotice();
                    return;
                }
            }
            if (cmd == "space:helm_match" || cmd == "space:mine_laser" || cmd == "space:mine_drones")
            {
                string wireCmd = ResolveOrbitalMiningCommand(
                    cmd,
                    SpaceStage.SelectedRockId,
                    SpaceStage.HasClusterMiningTelemetry,
                    SpaceStage.AuthoritativeMatchedRockId,
                    out string refusal);
                if (string.IsNullOrEmpty(wireCmd))
                {
                    if (Audio != null) Audio.PlayUiClick();
                    Hud.AppendLogLine($"<color=#FFAA55>[MINING]</color> {refusal}");
                    return;
                }
                if (Audio != null)
                {
                    if (cmd == "space:mine_laser") Audio.PlayLaserPulse();
                    else Audio.PlayUiClick();
                }
                SetCameraMode(CameraRigMode.OrbitalExterior);
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine(wireCmd);
                else
                    LogDisconnectedCommandNotice();
                return;
            }
            if (cmd == "space:mine_stake")
            {
                if (Audio != null) Audio.PlayUiClick();
                if (!MushProtocol.TryValidateSafeCommandToken(SpaceStage.SelectedRockId, out string stakeRock))
                {
                    Hud.AppendLogLine("<color=#FFAA55>[MINING]</color> Select an orbital rock (`A`, `B`, or `C`) before staking a claim.");
                    return;
                }
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine($"+mine/stake {stakeRock.ToUpperInvariant()}");
                else
                    LogDisconnectedCommandNotice();
                return;
            }
            if (cmd.StartsWith("space:lock:", StringComparison.OrdinalIgnoreCase))
            {
                string rawTgt = cmd.Substring("space:lock:".Length);
                if (MushProtocol.TryValidateSafeCommandToken(rawTgt, out string tgt))
                {
                    if (Audio != null) Audio.PlayRadarScanPing();
                    if (_ws != null && _ws.IsConnected)
                        _ws.SendLine($"+tac/lock {tgt}");
                    else
                        LogDisconnectedCommandNotice();
                    return;
                }
            }
            if (cmd == "space:tac_beam" || cmd == "space:tac_railgun" || cmd == "space:tac_torp")
            {
                if (Audio != null)
                {
                    if (cmd == "space:tac_railgun") Audio.PlayRailgunCrack();
                    else if (cmd == "space:tac_torp") Audio.PlayTorpedoLaunch();
                    else Audio.PlayLaserPulse();
                }
                SetCameraMode(CameraRigMode.OrbitalExterior);
                string kind = cmd == "space:tac_railgun" ? "railgun" : (cmd == "space:tac_torp" ? "torp" : "beam");
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine($"+tac/{kind}");
                else
                    LogDisconnectedCommandNotice();
                return;
            }
            if (cmd.StartsWith("helm:thrust:", StringComparison.OrdinalIgnoreCase))
            {
                string suffix = cmd.Substring("helm:thrust:".Length).Trim();
                if (suffix.IndexOf(' ') < 0 && int.TryParse(suffix, out int permille) && permille >= 0 && permille <= 1000)
                {
                    if (Audio != null) Audio.PlayUiClick();
                    SpaceStage.SetLocalHelmPreview(SpaceStage.ShipYawDeg, SpaceStage.ShipPitchDeg, permille);
                    Hud.UpdateHelmStatus(SpaceStage.ThrottlePermille, SpaceStage.EmconPosture);
                    if (_ws != null && _ws.IsConnected)
                    {
                        FlushPendingRoomMovesBeforeAction();
                        _ws.SendLine($"+helm/thrust {permille}");
                    }
                    else
                    {
                        LogDisconnectedCommandNotice();
                    }
                    return;
                }
            }
            if (cmd == "action:undock_selected")
            {
                string dest = TradeState != null && !string.IsNullOrEmpty(TradeState.SelectedDestinationPort)
                    ? TradeState.SelectedDestinationPort
                    : "stickney";
                ExecutePlayerCommand($"+ship/undock {dest}");
                return;
            }
            if (cmd == "action:start_docking")
            {
                // When underway in space, prefer the active space site or the player's selected
                // corridor destination over a stale departure station pack/venue.
                bool inSpace = _isAboardShip && !_hullIsDocked;
                string dockTarget = ResolveDockingTargetPort(
                    SpaceStage != null ? SpaceStage.ActiveSiteId : null,
                    inSpace ? null : (TradeState != null ? TradeState.AboardDockedPortHint : null),
                    inSpace ? null : (TradeState != null ? TradeState.ActiveVenue : null),
                    inSpace ? null : (StationWorld != null ? StationWorld.LoadedPackId : null),
                    TradeState != null ? TradeState.SelectedDestinationPort : null);
                ExecutePlayerCommand($"+helm/dock {dockTarget}");
                return;
            }

            if (Audio != null)
            {
                Audio.PlayForCommandLine(cmd);
                Audio.PlayUiClick();
            }

            if (cmd.StartsWith("+market/list", StringComparison.OrdinalIgnoreCase))
                TradeState.NotifyMarketRefreshRequested();

            string safeCmd = MushProtocol.EscapeUnityRichText(cmd);
            Hud.AppendLogLine($"<color=#66CCFF>> {safeCmd}</color>");
            if (Hud.IsModalOpen)
                Hud.AppendModalResponseLine($"<color=#66CCFF>> {safeCmd}</color>", resetPreviousResponses: true);

            OnCommandDispatched?.Invoke(cmd);

            if (_ws != null && _ws.IsConnected)
            {
                FlushPendingRoomMovesBeforeAction();
                // Any explicit typed/UI command may initiate a server-driven room change (e.g. typing `concourse` or `bridge`
                // while standing still); cancel pending queued navigation so HandleServerRoomView teleports the stationary avatar
                // and stale queued hops never fire after an unrelated command!
                _navWalker.Cancel();
                _ws.SendLine(cmd);
            }
            else
            {
                _navWalker.Cancel();
                LogDisconnectedCommandNotice();
            }
        }

        private void SitAtBridgeHelm()
        {
            Player.IsSeatedAtHelm = true;
            _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);
            if (ShipInterior.HelmSeatStandSpot != null)
                Player.TeleportTo(ShipInterior.HelmSeatStandSpot.position, 0f);

            CameraRigMode targetCam = _hullIsDocked ? CameraRigMode.ShipBridgeHolo : CameraRigMode.OrbitalExterior;
            CameraRig.SetMode(targetCam);
            _stageDirector?.ApplySoundscapeForCamera(targetCam, isAboardShip: true);
            Hud.RefreshContextualQuickActions("interior:bridge", isSeatedAtHelm: true);
            Hud.UpdateHelmStatus(SpaceStage.ThrottlePermille, SpaceStage.EmconPosture);

            if (_ws != null && _ws.IsConnected)
                _ws.SendLine("+helm/station command");
        }

        private void StandFromHelm()
        {
            SpaceStage.StopDockingSession();
            Player.IsSeatedAtHelm = false;
            _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);
            CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
            _stageDirector?.ApplySoundscapeForCamera(CameraRigMode.ThirdPersonWalk, isAboardShip: true);
            Hud.RefreshContextualQuickActions("interior:bridge", isSeatedAtHelm: false);
        }

        private void SetCameraMode(CameraRigMode mode)
        {
            CameraRig.SetMode(mode);
            _stageDirector?.ApplySoundscapeForCamera(mode, _isAboardShip);
        }

        private void HandleDockingTapeReady(string tapeCommand, bool isClampCandidate)
        {
            if (string.IsNullOrWhiteSpace(tapeCommand) ||
                MushProtocol.ContainsControlChars(tapeCommand) ||
                !tapeCommand.StartsWith("+helm/dock tape ", StringComparison.Ordinal))
            {
                return;
            }
            Hud.AppendLogLine(isClampCandidate
                ? "<color=#88DDFF>[DOCKING]</color> Collar gates aligned — submitting 60 Hz RLE tape for server verification..."
                : "<color=#FFAA66>[DOCKING]</color> Collar contact outside gate tolerance — submitting 60 Hz RLE tape...");
            if (_ws != null && _ws.IsConnected)
                _ws.SendLine(tapeCommand);
        }

        private void HandleDockingEndedByServer(string outcome)
        {
            string norm = (outcome ?? "closed").ToLowerInvariant();
            bool seatedAtHelm = Player != null && Player.IsSeatedAtHelm;
            if (norm == "clamped")
            {
                Audio?.PlayDockHardClamp();
                Hud?.AppendLogLine("<color=#55FF99>[HARD CLAMP CONFIRMED]</color> Authoritative server collar seal locked.");
                CameraRig?.SetMode(seatedAtHelm ? CameraRigMode.ShipBridgeHolo : CameraRigMode.ThirdPersonWalk);
                Audio?.SetSoundscape(AmbientSoundscape.ShipInterior);
            }
            else
            {
                Hud?.AppendLogLine($"<color=#FF8866>[DOCKING CLOSED]</color> Session ended ({MushProtocol.EscapeUnityRichText(norm)}).");
                CameraRig?.SetMode(seatedAtHelm ? CameraRigMode.OrbitalExterior : CameraRigMode.ThirdPersonWalk);
            }
        }

        private void HandleDockingAborted()
        {
            Hud?.AppendLogLine("<color=#FF8866>[DOCKING]</color> Approach aborted.");
            if (_ws != null && _ws.IsConnected)
                _ws.SendLine("+helm/dock abort");
            bool seatedAtHelm = Player != null && Player.IsSeatedAtHelm;
            CameraRig?.SetMode(seatedAtHelm ? CameraRigMode.OrbitalExterior : CameraRigMode.ThirdPersonWalk);
        }

        private void HandleServerRoomView(MushRoomView rv)
        {
            _lastRoomView = rv;
            _nextMoveStepAllowedAt = Time.time + 0.22f;

            if (rv.IcTime != null)
                _currentClockStr = $"{rv.IcTime.Hour:D2}:{rv.IcTime.Minute:D2} IC ({rv.IcTime.TimeOfDay})";

            if (rv.TacSim != null)
            {
                if (rv.TacSim.Stowage != null)
                    TradeState?.ApplyStowageAmbient(rv.TacSim.Stowage);
                else if (rv.TacSim.OwnStowage != null)
                    TradeState?.ApplyStowageAmbient(rv.TacSim.OwnStowage);

                if (rv.TacSim.Mode == "simulator" && ArenaStage != null)
                {
                    ApplySimulatorRoomAmbient(rv.TacSim);
                    if (_ws != null && _ws.IsConnected && _ws.HasAdvertisedPanel("port.trade"))
                        _ws.SendPanelSubscribe("port.trade");
                }
            }

            string prevStationPackId = StationWorld != null ? StationWorld.LoadedPackId : null;
            string prevShipUnitId = ShipInterior != null ? ShipInterior.LoadedUnitId : null;

            bool enteringShipInterior = ShipInterior.TryResolveCompartmentFromRoomView(rv, out string resolvedComp);
            if (enteringShipInterior)
            {
                if (ArenaStage != null && ArenaStage.IsArenaActive)
                    ArenaStage.ExitArena();
                if (SurfaceStage != null && SurfaceStage.IsExcursionActive && (resolvedComp != "bay" || !SurfaceStage.RigOnGround))
                    ReconcileSurfaceExcursionEnded();

                bool wasAboard = _isAboardShip;
                bool firstSync = _awaitingInitialRoomSync;
                bool shipUnitChanged = !string.Equals(prevShipUnitId, ShipInterior.LoadedUnitId, StringComparison.OrdinalIgnoreCase);
                _awaitingInitialRoomSync = false;

                _isAboardShip = true;
                if (!firstSync && !wasAboard)
                    Audio?.PlayAirlockCycle();
                _serverShipCompartment = resolvedComp;
                _serverStationRoomEntry = null;
                ShipInterior.ApplyRoomView(rv, _ws != null && _ws.ActiveCharacter != null ? _ws.ActiveCharacter.Id : null);
                if (SurfaceStage == null || !SurfaceStage.IsExcursionActive)
                    Audio?.SetSoundscape(AmbientSoundscape.ShipInterior);

                string physComp = Player != null
                    ? ShipInterior.TryFindContainingCompartment(Player.transform.position, ShipInterior.ActiveCompartmentId, hysteresisM: 0.55f)
                    : null;

                string acknowledgedWalkTarget = null;
                QueuedNavHop? nextQueuedHop = null;
                if (!wasAboard || firstSync || shipUnitChanged)
                {
                    _navWalker.Cancel();
                }
                else
                {
                    bool allowAdvance = !string.IsNullOrEmpty(physComp) && physComp != resolvedComp;
                    var navRes = _navWalker.OnAuthoritativeRoomUpdate(NavWalkerScope.Ship, resolvedComp, Time.time, allowAdvance);
                    acknowledgedWalkTarget = navRes.AcknowledgedTarget;
                    nextQueuedHop = navRes.NextHopToSend;
                }

                bool alreadyInComp = wasAboard && !firstSync && !shipUnitChanged &&
                                     !string.IsNullOrEmpty(physComp) && physComp == resolvedComp;
                bool correlatedShipWalkAhead = wasAboard && !firstSync && !shipUnitChanged &&
                                               !string.IsNullOrEmpty(acknowledgedWalkTarget) &&
                                               acknowledgedWalkTarget == resolvedComp &&
                                               !string.IsNullOrEmpty(physComp) &&
                                               ShipInterior.FindExitPath(resolvedComp, physComp).Count > 0;
                bool keepPhysicalShipPose = alreadyInComp || correlatedShipWalkAhead;

                if (!keepPhysicalShipPose && (SurfaceStage == null || !SurfaceStage.IsExcursionActive))
                {
                    SpaceStage?.StopDockingSession();
                    Player.IsSeatedAtHelm = false;
                    Player.TeleportTo(ShipInterior.GetCompartmentSpawnWorldPos(resolvedComp), 180f);
                    CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
                    CameraRig.SnapBehindTarget(180f);
                }

                if (SurfaceStage == null || !SurfaceStage.IsExcursionActive)
                {
                    if (rv.TacSim != null)
                    {
                        _hullIsDocked = rv.TacSim.Docked || rv.TacSim.SettledPad;
                        _currentHullBadge = $"{rv.TacSim.HullName} ({rv.TacSim.Mark}) — {(rv.TacSim.Docked ? "DOCKED" : (rv.TacSim.SettledPad ? "SETTLED PAD" : "UNDER WAY"))}";
                        Hud.UpdateTopBar(
                            $"{rv.TacSim.HullName.ToUpperInvariant()} [{rv.TacSim.ClassWord}] — {rv.Name.ToUpperInvariant()}",
                            $"{_currentClockStr} | {_currentHullBadge}");
                    }
                    else
                    {
                        Hud.UpdateTopBar($"SHIP INTERIOR — {rv.Name.ToUpperInvariant()}", $"{_currentClockStr} | {_currentHullBadge}");
                    }
                    string activeUiComp = correlatedShipWalkAhead && !string.IsNullOrEmpty(physComp) ? physComp : resolvedComp;
                    Hud.RefreshContextualQuickActions($"interior:{activeUiComp}", Player.IsSeatedAtHelm);
                }

                bool wsReadyShip = (_ws != null && _ws.IsConnected) || _simulateConnectedForTests;
                if (nextQueuedHop.HasValue && wsReadyShip &&
                    string.Equals(nextQueuedHop.Value.FromRoom, resolvedComp, StringComparison.Ordinal))
                {
                    DispatchNavHop(nextQueuedHop.Value, 0.35f);
                }
            }
            else if (StationWorld.TryMatchStationRoom(rv, out string matchedStationEntry))
            {
                if (ArenaStage != null && ArenaStage.IsArenaActive && (rv.TacSim == null || !rv.TacSim.SimInPod))
                    ArenaStage.ExitArena();
                if (SurfaceStage != null && SurfaceStage.IsExcursionActive)
                    ReconcileSurfaceExcursionEnded();

                bool wasAboard = _isAboardShip;
                bool firstSync = _awaitingInitialRoomSync;
                bool stationPackSwitched = !string.Equals(prevStationPackId, StationWorld.LoadedPackId, StringComparison.OrdinalIgnoreCase);
                _awaitingInitialRoomSync = false;

                _isAboardShip = false;
                if (!firstSync && wasAboard)
                    Audio?.PlayAirlockCycle();
                SpaceStage?.StopDockingSession();
                Player.IsSeatedAtHelm = false;
                _serverStationRoomEntry = matchedStationEntry;
                _serverShipCompartment = null;

                string physRoom = Player != null
                    ? StationWorld.TryFindContainingRoomWithHysteresis(Player.transform.position, StationWorld.ActiveRoomEntryId)
                    : null;

                string acknowledgedWalkTarget = null;
                QueuedNavHop? nextQueuedHop = null;
                if (wasAboard || firstSync || stationPackSwitched)
                {
                    _navWalker.Cancel();
                }
                else
                {
                    bool allowAdvance = !string.IsNullOrEmpty(physRoom) && physRoom != matchedStationEntry;
                    var navRes = _navWalker.OnAuthoritativeRoomUpdate(NavWalkerScope.Station, matchedStationEntry, Time.time, allowAdvance);
                    acknowledgedWalkTarget = navRes.AcknowledgedTarget;
                    nextQueuedHop = navRes.NextHopToSend;
                }

                bool alreadyInRoom = !firstSync && !wasAboard && !stationPackSwitched &&
                                     !string.IsNullOrEmpty(physRoom) && physRoom == matchedStationEntry;
                bool correlatedStationWalkAhead = !firstSync && !wasAboard && !stationPackSwitched &&
                                                  !string.IsNullOrEmpty(acknowledgedWalkTarget) &&
                                                  acknowledgedWalkTarget == matchedStationEntry &&
                                                  !string.IsNullOrEmpty(physRoom) &&
                                                  StationWorld.AreRoomsConnectedWithinHops(matchedStationEntry, physRoom, maxHops: 3);
                bool keepPhysicalStationPose = alreadyInRoom || correlatedStationWalkAhead;
                bool playerAheadOfRoomView = correlatedStationWalkAhead && physRoom != matchedStationEntry;

                StationWorld.ApplyRoomView(
                    rv,
                    _ws != null && _ws.ActiveCharacter != null ? _ws.ActiveCharacter.Id : null,
                    updateHighlight: !playerAheadOfRoomView);
                Audio?.SetSoundscape(AmbientSoundscape.StationRing);

                if (!keepPhysicalStationPose && (ArenaStage == null || !ArenaStage.IsArenaActive))
                {
                    Player.TeleportTo(StationWorld.GetRoomSpawnPosition(matchedStationEntry), 0f);
                    CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
                    CameraRig.SnapBehindTarget(0f);
                }

                if (rv.TacSim != null && !string.IsNullOrEmpty(rv.TacSim.OwnHullName))
                    _currentHullBadge = $"{rv.TacSim.OwnHullName} — {(rv.TacSim.OwnHullDockedHere ? "CRADLED HERE" : "AWAY")}";

                bool isBerthGalleryRoom =
                    (rv.TacSim != null && string.Equals(rv.TacSim.Mode, "berth_gallery", StringComparison.OrdinalIgnoreCase)) ||
                    matchedStationEntry.IndexOf("berth_gallery", StringComparison.OrdinalIgnoreCase) >= 0;
                if (isBerthGalleryRoom && _ws != null && _ws.IsConnected && _ws.HasAdvertisedPanel("port.trade"))
                {
                    _awaitingAppendedTradePage = false;
                    _ws.SendPanelSubscribe("port.trade");
                }

                if (!playerAheadOfRoomView && (ArenaStage == null || !ArenaStage.IsArenaActive))
                {
                    string surface = rv.Atlas != null && !string.IsNullOrEmpty(rv.Atlas.Surface) ? rv.Atlas.Surface.ToUpperInvariant() : "STATION";
                    Hud.UpdateTopBar($"{rv.Name.ToUpperInvariant()} [{surface}]", $"{_currentClockStr} | {_currentHullBadge}");
                    Hud.RefreshContextualQuickActions(matchedStationEntry, false);
                }

                bool wsReadyStation = (_ws != null && _ws.IsConnected) || _simulateConnectedForTests;
                if (nextQueuedHop.HasValue && wsReadyStation &&
                    string.Equals(nextQueuedHop.Value.FromRoom, matchedStationEntry, StringComparison.Ordinal))
                {
                    DispatchNavHop(nextQueuedHop.Value, 0.35f);
                }
            }
            else
            {
                _navWalker.Cancel();
                _isAboardShip = false;
                _serverStationRoomEntry = null;
                _serverShipCompartment = null;

                bool wsReady = (_ws != null && _ws.IsConnected) || _simulateConnectedForTests;
                if (wsReady && _awaitingInitialRoomSync && !_limboAutoEnterAttempted && IsLimboStartRoom(rv))
                {
                    string ceresExitCmd = FindExitToCeresFreeport(rv);
                    if (!string.IsNullOrEmpty(ceresExitCmd))
                    {
                        _limboAutoEnterAttempted = true;
                        _awaitingInitialRoomSync = false;
                        string safeRoom = MushProtocol.EscapeUnityRichText(rv.Name);
                        string safeCmd = MushProtocol.EscapeUnityRichText(ceresExitCmd);
                        Hud.AppendLogLine($"<color=#88DDFF>[TRANSIT]</color> Stepping from <b>{safeRoom}</b> into Ceres Freeport ({safeCmd})...");
                        if (_ws != null && _ws.IsConnected)
                            _ws.SendLine(ceresExitCmd);
                        else
                            OnCommandDispatched?.Invoke(ceresExitCmd);
                        return;
                    }
                }

                _awaitingInitialRoomSync = false;
                Hud.UpdateTopBar($"{rv.Name.ToUpperInvariant()} [OUTSIDE PORT]", $"{_currentClockStr} | {_currentHullBadge}");
                Hud.RefreshContextualQuickActions(string.Empty, false);
            }

            _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);

            if (!string.IsNullOrWhiteSpace(rv.Name))
            {
                string safeRoomName = MushProtocol.EscapeUnityRichText(rv.Name);
                string descMarkup = !string.IsNullOrWhiteSpace(rv.RichDesc) ? rv.RichDesc : MushProtocol.EscapeUnityRichText(rv.Desc);
                Hud.AppendLogLine($"<color=#88DDFF>[{safeRoomName}]</color> {descMarkup}");
            }
        }

        private void ApplySimulatorRoomAmbient(TacSimRoomAmbient sim)
        {
            if (!string.IsNullOrEmpty(sim.SimMatchState))
                ArenaStage.LiveLobbyMatchState = sim.SimMatchState;
            if (sim.SimAiFill > 0)
                ArenaStage.SelectedLobbyAiCount = sim.SimAiFill;
            if (!string.IsNullOrEmpty(sim.SimMyTeam))
                ArenaStage.SelectedLobbyTeam = sim.SimMyTeam;
            if (!string.IsNullOrEmpty(sim.SimMyUnit))
                ArenaStage.SelectedLobbyUnit = sim.SimMyUnit.Replace("tacsim.unit.", string.Empty);

            if (sim.SimSlots != null && sim.SimSlots.Count > 0)
            {
                var sb = new StringBuilder(192);
                sb.Append("CLAIMED SLOTS: ");
                for (int i = 0; i < sim.SimSlots.Count; i++)
                {
                    var s = sim.SimSlots[i];
                    if (i > 0) sb.Append("  |  ");
                    string col = string.Equals(s.Team, "red", StringComparison.OrdinalIgnoreCase) ? "#FF7766" : "#66BBFF";
                    string tag = s.Seated ? " [IN POD]" : string.Empty;
                    string shortUnit = MushProtocol.EscapeUnityRichText((s.Unit ?? string.Empty).Replace("tacsim.unit.", string.Empty));
                    string safePilot = MushProtocol.EscapeUnityRichText(s.PilotName);
                    sb.AppendFormat("<color={0}>{1}{2}</color> {3} ({4}){5}",
                        col,
                        string.Equals(s.Team, "red", StringComparison.OrdinalIgnoreCase) ? "R" : "B",
                        s.Index >= 1 ? s.Index : s.Index + 1,
                        safePilot,
                        shortUnit,
                        tag);
                }
                ArenaStage.LiveLobbySlotSummary = sb.ToString();
            }
            else
            {
                ArenaStage.LiveLobbySlotSummary = string.Empty;
            }

            Hud.Overlays?.RefreshLobbySelectionHighlights();
        }

        private void ApplySimulatorTradeRoster(JsonNode simNode)
        {
            if (simNode == null || !simNode.IsObject || ArenaStage == null) return;
            string ms = simNode.Str("match_state", string.Empty);
            if (!string.IsNullOrEmpty(ms))
                ArenaStage.LiveLobbyMatchState = ms;
            int ai = simNode.Int("ai_fill", 0);
            if (ai > 0)
                ArenaStage.SelectedLobbyAiCount = ai;
            string myTeam = simNode.Str("my_team", string.Empty);
            if (!string.IsNullOrEmpty(myTeam))
                ArenaStage.SelectedLobbyTeam = myTeam;
            string myUnit = simNode.Str("my_unit", string.Empty);
            if (!string.IsNullOrEmpty(myUnit))
                ArenaStage.SelectedLobbyUnit = myUnit.Replace("tacsim.unit.", string.Empty);

            var slotsArr = simNode["slots"].AsArray();
            if (slotsArr != null && slotsArr.Count > 0)
            {
                var sb = new StringBuilder(192);
                sb.Append("CLAIMED SLOTS: ");
                for (int i = 0; i < slotsArr.Count; i++)
                {
                    var s = slotsArr[i];
                    if (!s.IsObject) continue;
                    if (i > 0) sb.Append("  |  ");
                    string team = s.Str("team", "red");
                    string col = string.Equals(team, "red", StringComparison.OrdinalIgnoreCase) ? "#FF7766" : "#66BBFF";
                    string tag = s.Bool("seated", false) ? " [IN POD]" : string.Empty;
                    string shortUnit = MushProtocol.EscapeUnityRichText(s.Str("unit", "line_tank").Replace("tacsim.unit.", string.Empty));
                    string safeName = MushProtocol.EscapeUnityRichText(s.Str("pilot_name", string.Empty));
                    int rawIdx = s.Int("index", 1);
                    sb.AppendFormat("<color={0}>{1}{2}</color> {3} ({4}){5}",
                        col,
                        string.Equals(team, "red", StringComparison.OrdinalIgnoreCase) ? "R" : "B",
                        rawIdx >= 1 ? rawIdx : rawIdx + 1,
                        safeName,
                        shortUnit,
                        tag);
                }
                ArenaStage.LiveLobbySlotSummary = sb.ToString();
            }
            else if (slotsArr != null)
            {
                ArenaStage.LiveLobbySlotSummary = string.Empty;
            }
            Hud.Overlays?.RefreshLobbySelectionHighlights();
        }

        public static bool IsLimboStartRoom(MushRoomView rv)
        {
            if (rv == null) return false;
            if (string.Equals(rv.Id, "#0", StringComparison.Ordinal))
                return true;
            if (rv.Atlas != null &&
                (string.Equals(rv.Atlas.Entry, "limbo", StringComparison.OrdinalIgnoreCase) ||
                 string.Equals(rv.Atlas.Entry, "room.limbo", StringComparison.OrdinalIgnoreCase)))
                return true;
            return string.Equals((rv.Name ?? string.Empty).Trim(), "Limbo", StringComparison.OrdinalIgnoreCase) &&
                   rv.Atlas == null &&
                   rv.TacSim == null;
        }

        public static string FindExitToCeresFreeport(MushRoomView rv)
        {
            if (rv == null || rv.Exits == null || !IsLimboStartRoom(rv)) return null;
            for (int i = 0; i < rv.Exits.Count; i++)
            {
                var ex = rv.Exits[i];
                bool matchesCeres = ex.Name.IndexOf("Ceres", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                    ex.Name.IndexOf("Freeport", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                    ex.Name.IndexOf("Arrivals", StringComparison.OrdinalIgnoreCase) >= 0;
                if (matchesCeres)
                {
                    string candidate = ex.Aliases.Count > 0 ? ex.Aliases[0] : ex.Name;
                    string safeCmd = TacSimContentLoader.FormatSafeExitCommand(candidate);
                    if (!string.IsNullOrEmpty(safeCmd))
                        return safeCmd;
                }

                for (int j = 0; j < ex.Aliases.Count; j++)
                {
                    string al = ex.Aliases[j];
                    if (string.Equals(al, "ceres", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(al, "freeport", StringComparison.OrdinalIgnoreCase))
                    {
                        string safeCmd = TacSimContentLoader.FormatSafeExitCommand(al);
                        if (!string.IsNullOrEmpty(safeCmd))
                            return safeCmd;
                    }
                }
            }
            return null;
        }

        private void HandleServerNarrative(MushNarrativeEvent nar)
        {
            string speaker = nar.Actor != null ? MushProtocol.EscapeUnityRichText(nar.Actor.Name) : "";
            string body = !string.IsNullOrEmpty(nar.RichMarkup) ? nar.RichMarkup : MushProtocol.EscapeUnityRichText(nar.Text);
            string formatted = nar.Kind == "say" && !string.IsNullOrEmpty(speaker)
                ? $"<b>{speaker}</b> says, \"{body}\""
                : (!string.IsNullOrEmpty(speaker) ? $"<b>{speaker}</b> {body}" : body);

            Hud.AppendLogLine(formatted);
            if (Hud.IsModalOpen)
                Hud.AppendModalResponseLine(formatted);

            if (nar.Actor != null)
            {
                if (_isAboardShip)
                    ShipInterior?.ShowActorSpeech(nar.Actor.Id, nar.Actor.Name, nar.Text);
                else
                    StationWorld?.ShowActorSpeech(nar.Actor.Id, nar.Actor.Name, nar.Text);
            }

            // Event-driven live roster/berth refresh when another pilot acts in Simulator Lobby, Berth Gallery,
            // or while a station modal is open (debounced >= 0.6s so room chatter never floods `port.trade`).
            if (!_isAboardShip && _ws != null && _ws.IsConnected && _ws.HasAdvertisedPanel("port.trade"))
            {
                bool inLiveRosterRoom =
                    string.Equals(_serverStationRoomEntry, "room.simulator_lobby", StringComparison.OrdinalIgnoreCase) ||
                    (!string.IsNullOrEmpty(_serverStationRoomEntry) &&
                     _serverStationRoomEntry.IndexOf("berth_gallery", StringComparison.OrdinalIgnoreCase) >= 0);
                if ((inLiveRosterRoom || (Hud != null && Hud.IsModalOpen)) &&
                    (Time.unscaledTime - _lastAutoTradeRefreshAt) >= 0.6f)
                {
                    _lastAutoTradeRefreshAt = Time.unscaledTime;
                    _awaitingAppendedTradePage = false;
                    _ws.SendPanelSubscribe("port.trade");
                }
            }
        }

        private void HandleServerChannel(MushChannelEvent ch)
        {
            string safeChan = MushProtocol.EscapeUnityRichText(ch.Channel);
            string safeSpeaker = MushProtocol.EscapeUnityRichText(ch.Actor?.Name ?? string.Empty);
            string body = !string.IsNullOrEmpty(ch.RichMarkup) ? ch.RichMarkup : MushProtocol.EscapeUnityRichText(ch.Text);
            Hud.AppendLogLine($"<color=#77DDAA>[{safeChan}]</color> {safeSpeaker}: {body}");
        }

        private void HandleServerComposing(MushComposingEvent comp)
        {
            if (comp?.Actor == null) return;
            if (_ws?.ActiveCharacter != null && comp.Actor.Id == _ws.ActiveCharacter.Id) return;
            if (_isAboardShip)
                ShipInterior?.SetActorComposing(comp.Actor.Id, comp.Actor.Name, comp.Active);
            else
                StationWorld?.SetActorComposing(comp.Actor.Id, comp.Actor.Name, comp.Active);
        }

        private void HandleServerSystemLine(string plainLine, string richMarkup)
        {
            string display = !string.IsNullOrEmpty(richMarkup)
                ? $"<color=#BBD8EE>{richMarkup}</color>"
                : $"<color=#BBD8EE>{MushProtocol.EscapeUnityRichText(plainLine)}</color>";
            Hud.AppendLogLine(display);
            if (Hud.IsModalOpen)
                Hud.AppendModalResponseLine(display);

            // Trigger surface/orbital action VFX only upon authoritative server outcome lines!
            if (!string.IsNullOrEmpty(plainLine))
            {
                if (SurfaceStage != null && SurfaceStage.IsExcursionActive)
                {
                    if (plainLine.IndexOf("sounding", StringComparison.OrdinalIgnoreCase) >= 0 &&
                        plainLine.IndexOf("refused", StringComparison.OrdinalIgnoreCase) < 0)
                    {
                        SurfaceStage.TriggerScanPulseVfx(seismicSounding: true);
                        if (Audio != null) Audio.PlayRadarScanPing();
                        Hud?.FloatingCallouts?.ShowCallout("SEISMIC SOUNDING PULSE", new Color(1f, 0.78f, 0.28f, 1f));
                    }
                    else if (plainLine.IndexOf("scan", StringComparison.OrdinalIgnoreCase) >= 0 &&
                             plainLine.IndexOf("field", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        SurfaceStage.TriggerScanPulseVfx(seismicSounding: false);
                        if (Audio != null) Audio.PlayRadarScanPing();
                        Hud?.FloatingCallouts?.ShowCallout("SURFACE RADAR SWEEP", new Color(0.32f, 0.94f, 1f, 1f));
                    }
                    else if (plainLine.IndexOf("mining cycle", StringComparison.OrdinalIgnoreCase) >= 0 ||
                             plainLine.IndexOf("hopper", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        SurfaceStage.TriggerDrillVfx(3.0f);
                        Hud?.FloatingCallouts?.ShowCallout("CORE DRILL EXTRACTING ORE", new Color(0.35f, 1f, 0.62f, 1f));
                    }
                }

                if (SpaceStage != null && SpaceStage.HasPerceivedContact(SpaceStage.LockedContactId))
                {
                    if (plainLine.IndexOf("Launched torpedo salvo", StringComparison.OrdinalIgnoreCase) >= 0 ||
                        plainLine.IndexOf("torpedo launched", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        SpaceStage.TriggerNavalWeaponVfx("torp");
                        if (Audio != null) Audio.PlayTorpedoLaunch();
                        Hud?.FloatingCallouts?.ShowCallout("TORPEDO SALVO AWAY", new Color(1f, 0.56f, 0.22f, 1f));
                    }
                    else if (plainLine.IndexOf("Railgun salvo fired", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        SpaceStage.TriggerNavalWeaponVfx("railgun");
                        if (Audio != null) Audio.PlayRailgunCrack();
                        Hud?.FloatingCallouts?.ShowCallout("SPINAL RAILGUN SALVO", new Color(1f, 0.86f, 0.34f, 1f));
                    }
                    else if (plainLine.IndexOf("Laser battery fired", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        SpaceStage.TriggerNavalWeaponVfx("beam");
                        if (Audio != null) Audio.PlayLaserPulse();
                        Hud?.FloatingCallouts?.ShowCallout("LASER BATTERY FIRED", new Color(1f, 0.36f, 0.40f, 1f));
                    }
                }
            }

            if (_isAboardShip) ShipInterior.UpdateGaugeText(plainLine);
            else StationWorld.AppendBoardLine(plainLine);

            if (TradeState != null && TradeState.IngestServerLine(plainLine, Time.time, out var followUps) &&
                followUps != null && _ws != null && _ws.IsConnected)
            {
                for (int i = 0; i < followUps.Count; i++)
                {
                    string nextCmd = followUps[i];
                    if (nextCmd == "ui:refresh_trade")
                    {
                        RequestPortTradeRefresh();
                        continue;
                    }
                    _ws.SendLine(nextCmd);
                }
            }
        }

        private void HandleServerPanelUpdated(string feed, JsonNode data)
        {
            switch (feed)
            {
                case "tacsim.content":
                    _contentSync?.HandleWebSocketContentPanel(_ws, data, Time.realtimeSinceStartup);
                    break;
                case "port.trade":
                {
                    bool append = _awaitingAppendedTradePage;
                    _awaitingAppendedTradePage = false;
                    TradeState?.ApplyPortTradeFeed(data, appendPage: append);
                    if (TradeState != null && TradeState.HasAuthoritativeBerthFeed && StationWorld != null)
                        StationWorld.ApplyBerthDockedHulls(TradeState.BerthDockedHulls);

                    if (TradeState != null &&
                        TradeState.TryResolvePendingQuoteFromFeed(data, Time.time, out string acceptCmd, out string priceWarn))
                    {
                        if (!string.IsNullOrEmpty(acceptCmd) && _ws != null && _ws.IsConnected)
                        {
                            if (Audio != null) Audio.PlayMarketChime();
                            Hud.AppendLogLine($"<color=#66EEAA>[QUOTE VERIFIED]</color> {MushProtocol.EscapeUnityRichText(acceptCmd)}");
                            _ws.SendLine(acceptCmd);
                            _ws.SendPanelSubscribe("port.trade");
                        }
                        else if (!string.IsNullOrEmpty(priceWarn))
                        {
                            string safeWarn = MushProtocol.EscapeUnityRichText(priceWarn);
                            Hud.AppendLogLine($"<color=#FFBB55>[PRICE MOVED]</color> {safeWarn}");
                            if (Hud.IsModalOpen)
                                Hud.AppendModalResponseLine($"<color=#FFBB55>{safeWarn}</color>");
                        }
                    }
                    if (data != null && data.IsObject && data["simulator"].IsObject && ArenaStage != null)
                        ApplySimulatorTradeRoster(data["simulator"]);
                    break;
                }
                case "sky.chart":
                {
                    var chart = TacSimSolarChartFrame.FromJson(data);
                    if (chart != null)
                    {
                        Hud?.SolarMap?.ApplyChartFrame(chart);
                        ShipInterior?.ApplySolarChartFrame(chart);
                    }
                    break;
                }
                case "space.holotank":
                    ShipInterior.PopulateHoloBlips(data);
                    SpaceStage.ApplyHolotankFeed(data);
                    RefreshOrbitalReadout();
                    break;
                case "space.cockpit":
                    SpaceStage.ApplyCockpitFeed(data);
                    Hud.UpdateHelmStatus(SpaceStage.ThrottlePermille, SpaceStage.EmconPosture);
                    RefreshOrbitalReadout();
                    break;
                case "space.docking":
                    HandleDockingPanelFrame(data);
                    break;
                case "tv.status":
                    if (SurfaceStage != null)
                    {
                        SurfaceStage.ApplyTvStatusFrame(data);
                        if (SurfaceStage.IsExcursionActive && !SurfaceStage.RigOnGround)
                            ReconcileSurfaceExcursionEnded();
                    }
                    if (_ws != null && _ws.IsConnected && data != null && data.IsObject && !data.Bool("cleared", false))
                    {
                        bool rigActiveOnGround = (SurfaceStage != null && SurfaceStage.RigOnGround) ||
                                                 (data.ContainsKey("seated") && data.Bool("seated", false));
                        if (rigActiveOnGround)
                            SendSiteMapPull(SurfaceStage != null ? SurfaceStage.ActiveOreFamily : "volatiles");
                    }
                    break;
                case "site.map":
                    if (SurfaceStage != null && data != null && data.IsObject && data["rows"].IsArray)
                    {
                        SurfaceStage.ApplySiteMapFrame(data);
                        var youArr = data["you"].AsArray();
                        if (!SurfaceStage.IsExcursionActive && SurfaceStage.RigOnGround && youArr != null && youArr.Count >= 2)
                            EnterPlanetarySurfaceStage();
                        else if (SurfaceStage.IsExcursionActive && (!SurfaceStage.RigOnGround || youArr == null || youArr.Count < 2))
                            ReconcileSurfaceExcursionEnded();
                    }
                    break;
                case "arena.map":
                    if (ArenaStage != null)
                    {
                        ArenaStage.ApplyLiveArenaMap(data);
                        if (ArenaStage.IsArenaActive)
                            BindCameraToArenaRig();
                    }
                    break;
                case "arena.cockpit":
                    if (ArenaStage != null)
                    {
                        bool wasActive = ArenaStage.IsArenaActive;
                        ArenaStage.ApplyLiveCockpitFrame(data);
                        if (ArenaStage.IsArenaActive)
                        {
                            _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);
                            BindCameraToArenaRig();
                            Hud.UpdateTopBar(
                                $"OOC SIMULATOR ARENA — {ArenaStage.State.Callsign} [{ArenaStage.State.Mode.ToUpperInvariant()}]",
                                $"LIVE MATCH #{ArenaStage.State.MatchId} | WASD: Drive | LMB: Lock | SPACE: Fire");
                        }
                        else if (wasActive)
                        {
                            _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);
                            CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
                            if (Player != null) CameraRig.SnapBehindTarget(Player.transform.eulerAngles.y);
                            Audio?.SetSoundscape(AmbientSoundscape.StationRing);
                        }
                    }
                    break;
            }
        }

        public void HandleDockingPanelFrame(JsonNode data)
        {
            if (SpaceStage == null) return;
            bool isTerminalFrame = data == null || !data.IsObject || data.Bool("closed", false) || data.Bool("refused", false);
            if (isTerminalFrame)
            {
                if (SpaceStage.IsDockingActive)
                    SpaceStage.StartDockingFromFeed(data);
                return;
            }

            // Server broadcasts `space.docking` to every player aboard the hull (`commands.rs:push_space_panels`).
            // Gate pilot 6-DOF docking mode, bow camera, and walk suspension strictly on whether the local player
            // is actually seated at the bridge helm so unseated passengers keep walking unimpeded.
            bool inVehicleOrArena = (ArenaStage != null && ArenaStage.IsArenaActive) ||
                                    (SurfaceStage != null && SurfaceStage.IsExcursionActive);
            bool seatedAtHelm = !inVehicleOrArena && Player != null && Player.IsSeatedAtHelm;
            if (!seatedAtHelm)
                return;

            SpaceStage.StartDockingFromFeed(data);
            if (SpaceStage.IsDockingActive)
            {
                CameraRig?.SetMode(CameraRigMode.BowDockingCollar);
                if (Audio != null) Audio.SetSoundscape(AmbientSoundscape.OrbitalSpace);
                _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);
            }
        }

        private void SendSiteMapPull(string family)
        {
            if (_ws == null || !_ws.IsConnected) return;
            var q = JsonNode.NewObject();
            q["view"] = JsonNode.FromString("drive");
            q["rig"] = JsonNode.FromLong(SurfaceStage != null ? SurfaceStage.AddressedRigSlot : 1);
            if (!string.IsNullOrEmpty(family))
                q["family"] = JsonNode.FromString(family);
            _ws.SendPanelSubscribe("site.map", q);
        }

        private void EnterPlanetarySurfaceStage()
        {
            if (SurfaceStage == null) return;
            if (ArenaStage != null && ArenaStage.IsArenaActive)
                ArenaStage.ExitArena();

            SurfaceStage.EnterSurfaceMode();
            _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);
            CameraRig.GroundVehicleTarget = SurfaceStage.RoverTransform;
            CameraRig.GroundCupolaAnchor = SurfaceStage.RoverCupolaAnchor;
            CameraRig.SetMode(CameraRigMode.GroundVehicleChase);
            Hud.Overlays?.SetHelmSeatedVisible(false);
            Hud.UpdateTopBar(
                $"{SurfaceStage.ActiveSiteTitle.ToUpperInvariant()} — SURFACE ROVER RIG {SurfaceStage.AddressedRigSlot}",
                "WASD / LMB Terrain: Drive | RMB + Scroll: Camera | Scan -> Sound -> Core Drill");
        }

        private void ExitPlanetarySurfaceStage()
        {
            if (SurfaceStage == null) return;
            if (!SurfaceStage.RigOnGround)
            {
                ReconcileSurfaceExcursionEnded();
                return;
            }
            ExecutePlayerCommand($"+tv/{SurfaceStage.AddressedRigSlot} recover go");
        }

        private void ReconcileSurfaceExcursionEnded()
        {
            if (SurfaceStage == null) return;
            SurfaceStage.ExitSurfaceMode();
            _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);

            bool seatedAtHelm = Player != null && Player.IsSeatedAtHelm;
            CameraRigMode restoredMode = seatedAtHelm
                ? (_hullIsDocked ? CameraRigMode.ShipBridgeHolo : CameraRigMode.OrbitalExterior)
                : CameraRigMode.ThirdPersonWalk;
            if (CameraRig != null)
            {
                CameraRig.SetMode(restoredMode);
                if (restoredMode == CameraRigMode.ThirdPersonWalk && Player != null)
                    CameraRig.SnapBehindTarget(Player.transform.eulerAngles.y);
            }
            _stageDirector?.ApplySoundscapeForCamera(restoredMode, _isAboardShip);

            if (Hud != null)
            {
                Hud.Overlays?.SetHelmSeatedVisible(seatedAtHelm);
                Hud.UpdateTopBar("RETURNED FROM SURFACE EXCURSION", $"{_currentClockStr} | {_currentHullBadge}");
                if (_isAboardShip && !string.IsNullOrEmpty(_serverShipCompartment))
                    Hud.RefreshContextualQuickActions($"interior:{_serverShipCompartment}", seatedAtHelm);
                else if (!string.IsNullOrEmpty(_serverStationRoomEntry))
                    Hud.RefreshContextualQuickActions(_serverStationRoomEntry, false);
            }
        }

        private void HandleArenaCockpitVerb(string verb)
        {
            if (ArenaStage == null) return;
            ArenaStage.ExecuteCockpitAction(verb);
            if (string.Equals(verb?.Trim(), "exit", StringComparison.OrdinalIgnoreCase) && ArenaStage.IsArenaActive)
                ArenaStage.ExitArena();

            if (!ArenaStage.IsArenaActive)
            {
                _stageDirector?.SyncFrame(_isAboardShip, Hud != null && Hud.IsTextInputFocused);
                CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
                if (Player != null) CameraRig.SnapBehindTarget(Player.transform.eulerAngles.y);
                Audio?.SetSoundscape(AmbientSoundscape.StationRing);
                Hud.UpdateTopBar("OOC SIMULATOR POD BAY", $"{_currentClockStr} | {_currentHullBadge}");
            }
        }

        private void BindCameraToArenaRig()
        {
            if (ArenaStage?.PlayerVehicleTransform == null) return;
            CameraRig.GroundVehicleTarget = ArenaStage.PlayerVehicleTransform;
            CameraRig.GroundCupolaAnchor = ArenaStage.PlayerCupolaAnchor;
            if (CameraRig.Mode != CameraRigMode.GroundVehicleChase && CameraRig.Mode != CameraRigMode.GroundVehicleCupola)
                CameraRig.SetMode(CameraRigMode.GroundVehicleChase);
        }

        private void ToggleGroundVehicleCameraMode()
        {
            if (CameraRig.Mode == CameraRigMode.GroundVehicleChase)
                CameraRig.SetMode(CameraRigMode.GroundVehicleCupola);
            else
                CameraRig.SetMode(CameraRigMode.GroundVehicleChase);
        }
    }
}
