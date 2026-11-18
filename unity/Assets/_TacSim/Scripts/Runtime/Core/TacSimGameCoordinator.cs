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

        public TacSimTradeAndShipState TradeState { get; private set; }

        private MushWsClient _ws;
        private TacSimContentSync _contentSync;
        private TacSimStageDirector _stageDirector;

        private bool _isAboardShip;
        private bool _hullIsDocked = true;
        private bool _awaitingInitialRoomSync;
        private bool _limboAutoEnterAttempted;
        private string _serverStationRoomEntry;
        private string _serverShipCompartment;
        private string _pendingMoveTarget;
        private float _nextMoveStepAllowedAt;
        private string _currentClockStr = "08:15 UTC";
        private string _currentHullBadge = "NO CHARTER";
        private MushWsConnectionState _lastBadgeState = MushWsConnectionState.Disconnected;

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
                }
            }

            if (Player.VisualSpawner == null)
                Player.VisualSpawner = Player.GetComponent<CharacterVisualSpawner>() ?? Player.gameObject.AddComponent<CharacterVisualSpawner>();
            Player.VisualSpawner.BuildPlayerVisual(AssetCatalog, PlayerAvatarPreset.StellarTrooperFemale);

            if (CameraRig.ControlledCamera == null)
                CameraRig.ControlledCamera = CameraRig.GetComponentInChildren<Camera>() ?? Camera.main;

            Player.CameraTransform = CameraRig.ControlledCamera.transform;
            CameraRig.WalkFollowTarget = Player.transform;
            CameraRig.BridgeViewAnchor = ShipInterior.BridgeCameraAnchor;
            CameraRig.OrbitalShipTarget = SpaceStage.PlayerShipTransform;
            CameraRig.BowDockingAnchor = SpaceStage.BowDockingCameraAnchor;

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
            Hud.Overlays?.BindControllers(SurfaceStage, ArenaStage);
            TradeState.OnStateChanged += () =>
            {
                if (ShipInterior != null)
                    ShipInterior.UpdateCargoVisualFill(TradeState.HoldMassTonnes);
            };

            _ws = new MushWsClient();
            _contentSync = new TacSimContentSync();
            _stageDirector = new TacSimStageDirector(
                StationWorld,
                ShipInterior,
                SpaceStage,
                SurfaceStage,
                ArenaStage,
                CameraRig,
                Player,
                Audio);
        }

        private void BindEvents()
        {
            Player.OnFocusedInteractableChanged += wi => Hud.SetFocusedInteractable(wi);

            StationWorld.OnInteractableTriggered += HandleWorldInteractable;
            ShipInterior.OnInteractableTriggered += HandleWorldInteractable;

            StationWorld.OnPortalCrossed += HandleStationPortalCrossed;
            ShipInterior.OnPortalCrossed += HandleShipPortalCrossed;

            SpaceStage.OnDockingTapeReadyToSubmit += HandleDockingTapeReady;
            SpaceStage.OnDockingAborted += HandleDockingAborted;
            SpaceStage.OnDockingEndedByServer += HandleDockingEndedByServer;
            SpaceStage.OnSpaceAlert += msg => Hud.AppendLogLine($"<color=#FFAA44>[HELM]</color> {msg}");
            SpaceStage.OnOrbitalTargetClicked += (id, isRock) =>
            {
                Hud.Overlays?.UpdateOrbitalTargetReadout(SpaceStage.SelectedRockId, SpaceStage.LockedContactId);
                Hud.AppendLogLine(isRock
                    ? $"<color=#66FFCC>[MINING]</color> Selected Asteroid Pocket <b>{id}</b>."
                    : $"<color=#FFBB55>[TAC]</color> Locked Naval Contact <b>{id}</b>.");
            };

            SurfaceStage.OnTerrainCellDriveClicked += (sx, sy) =>
            {
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine($"+tv/{SurfaceStage.AddressedRigSlot} drive {sx},{sy} go");
            };
            SurfaceStage.OnSurfaceLogEvent += ln => Hud.AppendLogLine($"<color=#FFCC66>[SURFACE]</color> {ln}");

            ArenaStage.OnLiveCockpitVerbRequested += verb =>
            {
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine(verb);
            };
            ArenaStage.OnRequestArenaMapSubscribe += () =>
            {
                if (_ws != null && _ws.IsConnected)
                    _ws.SendPanelSubscribe("arena.map");
            };
            ArenaStage.OnArenaCombatLog += ln => Hud.AppendLogLine(ln);
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

            if (Hud.Overlays != null)
            {
                Hud.Overlays.OnCommandRequested += ExecutePlayerCommand;
                Hud.Overlays.OnExitSurfaceExcursionRequested += ExitPlanetarySurfaceStage;
                Hud.Overlays.OnArenaCockpitVerbRequested += HandleArenaCockpitVerb;
                Hud.Overlays.OnToggleVehicleCameraViewRequested += ToggleGroundVehicleCameraMode;
            }

            _contentSync.OnSyncLog += msg => Hud.AppendLogLine($"<color=#77CCFF>[PACK SYNC]</color> {msg}");
            _contentSync.OnPacksUpdated += _ => RefreshLoadedContentAfterSync();

            _ws.OnHelloReceived += () => _contentSync.RequestManifestOverWebSocket(_ws, Time.realtimeSinceStartup);
            _ws.OnRoomReceived += HandleServerRoomView;
            _ws.OnNarrativeReceived += HandleServerNarrative;
            _ws.OnChannelReceived += HandleServerChannel;
            _ws.OnComposingReceived += HandleServerComposing;
            _ws.OnSystemLineWithMarkupReceived += HandleServerSystemLine;
            _ws.OnPanelUpdated += HandleServerPanelUpdated;
            _ws.OnServerRequestReceived += req => Hud.ShowServerRequestModal(req);
            _ws.OnConnectionStatusChanged += status => Hud.AppendLogLine($"<color=#88CCFF>[NET]</color> {status}");
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

            if (_stageDirector != null &&
                (_stageDirector.CurrentStage == TacSimActiveStage.StationWalk ||
                 _stageDirector.CurrentStage == TacSimActiveStage.ShipInteriorWalk))
            {
                Sync3DPositionWithServer();
            }

            if (Hud != null && SpaceStage != null)
                Hud.UpdateDockingHud(SpaceStage.ActiveDockingSession, SpaceStage.GuidanceAssistEnabled);
        }

        private void OnDestroy()
        {
            _ws?.Dispose();
        }

        private void RefreshLoadedContentAfterSync()
        {
            TradeState?.ReloadGoodsCatalog();
            if (StationWorld != null && !string.IsNullOrEmpty(StationWorld.LoadedPackId))
            {
                string prevRoom = _serverStationRoomEntry ?? StationWorld.ActiveRoomEntryId;
                StationWorld.BuildStation(AssetCatalog, StationWorld.LoadedPackId);
                if (!string.IsNullOrEmpty(prevRoom))
                    StationWorld.HighlightActiveRoom(prevRoom);
            }
            if (ShipInterior != null && !string.IsNullOrEmpty(ShipInterior.LoadedUnitId))
            {
                ShipInterior.BuildInterior(AssetCatalog, ShipInterior.LoadedUnitId);
            }
        }

        private async void HandleAuthSubmitted(string baseUrl, string account, string password, string charName, bool isRegister, PlayerAvatarPreset avatar)
        {
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

            try
            {
                Hud.SetLoginStatus(isRegister ? $"Registering '{account}' on {baseUrl}..." : $"Signing in to {baseUrl} as '{account}'...");
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

                if (exactMatch != null)
                    HandleCharacterSelected(session, exactMatch, avatar);
                else if (session.Characters.Count == 1)
                    HandleCharacterSelected(session, session.Characters[0], avatar);
                else
                    Hud.ShowCharacterPicker(session);
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"[TacSim Auth] {ex}");
                Hud.SetLoginStatus($"<color=#FF8877>{ex.Message}</color>");
            }
        }

        private async void HandleCharacterSelected(MushAuthSession session, MushCharacterSlot chosen, PlayerAvatarPreset avatar)
        {
            try
            {
                _awaitingInitialRoomSync = true;
                _limboAutoEnterAttempted = false;
                _serverStationRoomEntry = null;
                _serverShipCompartment = null;
                _pendingMoveTarget = null;
                _nextMoveStepAllowedAt = 0f;

                Hud.SetLoginStatus($"Loading server pack cache for <b>{session.BaseHttpUrl}</b>...");
                _contentSync.InitializeDiskCacheForServer(session.BaseHttpUrl);

                Hud.SetLoginStatus($"Attaching WebSocket to <b>{chosen.Name}</b>...");
                Player.VisualSpawner.BuildPlayerVisual(AssetCatalog, avatar);
                await _ws.ConnectAsync(session, chosen.Key, resetResume: true);
                Hud.SetConnectionBadge(_ws.State);
                Hud.ShowLoginOverlay(false);
                Hud.AppendLogLine($"<color=#66FFAA>[CONNECTED]</color> Attached as <b>{chosen.Name}</b> ({session.Account}) on {session.BaseHttpUrl}.");
            }
            catch (Exception ex)
            {
                Debug.LogWarning($"[TacSim WS] {ex}");
                Hud.SetLoginStatus($"<color=#FF8877>WebSocket connect failed: {ex.Message}</color>");
            }
        }

        private void HandleFatalSessionEnded(string reason)
        {
            ArenaStage?.ExitArena();
            SurfaceStage?.ExitSurfaceMode();
            SpaceStage?.StopDockingSession();
            if (Player != null) Player.IsSeatedAtHelm = false;
            CameraRig?.SetMode(CameraRigMode.ThirdPersonWalk);
            string display = string.IsNullOrWhiteSpace(reason)
                ? "<color=#FF8877>Session ended by mushd — please sign in again.</color>"
                : $"<color=#FF8877>{reason}</color>";
            Hud.ShowLoginOverlay(true, display);
        }

        private void Sync3DPositionWithServer()
        {
            if (Player == null || Player.IsSeatedAtHelm || SpaceStage.IsDockingActive)
                return;
            if (_awaitingInitialRoomSync || _ws == null || !_ws.IsConnected)
                return;

            if (_isAboardShip)
            {
                if (string.IsNullOrEmpty(_serverShipCompartment)) return;
                string physComp = ShipInterior.TryFindContainingCompartment(Player.transform.position, ShipInterior.ActiveCompartmentId);
                if (string.IsNullOrEmpty(physComp) || physComp == _serverShipCompartment)
                    return;

                if (Time.time < _nextMoveStepAllowedAt)
                    return;

                var path = ShipInterior.FindExitPath(_serverShipCompartment, physComp);
                if (path.Count > 0)
                {
                    _pendingMoveTarget = path[0].targetCompartment;
                    _nextMoveStepAllowedAt = Time.time + 0.85f;
                    _ws.SendLine(path[0].command);
                }
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

                if (physRoom == _serverStationRoomEntry || Time.time < _nextMoveStepAllowedAt)
                    return;

                var path = StationWorld.FindExitPath(_serverStationRoomEntry, physRoom);
                if (path.Count > 0)
                {
                    _pendingMoveTarget = path[0].targetRoom;
                    _nextMoveStepAllowedAt = Time.time + 0.85f;
                    _ws.SendLine(path[0].command);
                }
            }
        }

        private void FlushPendingRoomMovesBeforeAction()
        {
            if (_ws == null || !_ws.IsConnected || Player == null)
                return;

            if (_isAboardShip)
            {
                string startComp = !string.IsNullOrEmpty(_pendingMoveTarget) ? _pendingMoveTarget : _serverShipCompartment;
                string physComp = ShipInterior.TryFindContainingCompartment(Player.transform.position, startComp, hysteresisM: 0.1f);
                if (!string.IsNullOrEmpty(startComp) && !string.IsNullOrEmpty(physComp) && physComp != startComp)
                {
                    var hops = ShipInterior.FindExitPath(startComp, physComp);
                    for (int i = 0; i < hops.Count; i++)
                        _ws.SendLine(hops[i].command);
                    if (hops.Count > 0)
                    {
                        _pendingMoveTarget = physComp;
                        _nextMoveStepAllowedAt = Time.time + 1.0f;
                    }
                }
            }
            else
            {
                string startRoom = !string.IsNullOrEmpty(_pendingMoveTarget) ? _pendingMoveTarget : _serverStationRoomEntry;
                string physRoom = StationWorld.TryFindContainingRoomWithHysteresis(Player.transform.position, startRoom, hysteresisM: 0.1f);
                if (!string.IsNullOrEmpty(startRoom) && !string.IsNullOrEmpty(physRoom) && physRoom != startRoom)
                {
                    var hops = StationWorld.FindExitPath(startRoom, physRoom);
                    for (int i = 0; i < hops.Count; i++)
                        _ws.SendLine(hops[i].command);
                    if (hops.Count > 0)
                    {
                        _pendingMoveTarget = physRoom;
                        _nextMoveStepAllowedAt = Time.time + 1.0f;
                    }
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
                    if (_ws != null && _ws.IsConnected && !string.IsNullOrEmpty(wi.commandOrKey))
                    {
                        FlushPendingRoomMovesBeforeAction();
                        _ws.SendLine(wi.commandOrKey);
                    }
                    Hud.ShowDetailModal(wi.promptTitle.ToUpperInvariant(), string.IsNullOrEmpty(wi.detailBody) ? "Inspecting..." : wi.detailBody);
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
            string name = wi.commandOrKey;
            string roleNote = !string.IsNullOrWhiteSpace(wi.detailBody)
                ? $"<color=#88CCEE>{wi.detailBody}</color>\n"
                : string.Empty;

            var topics = new List<(string, Action)>();
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

            Hud.ShowDetailModal(
                name.ToUpperInvariant(),
                $"{roleNote}Speak directly with <b>{name}</b>. Type a question below or choose a quick topic:",
                topics,
                npcName: name);
        }

        private static string SuggestUniqueHullName()
        {
            string[] prefixes = { "Drift", "Quiet", "Cinder", "Pale", "Copper", "Vesta", "Tether", "Iron" };
            string[] nouns = { "Lantern", "Kestrel", "Barge", "Moth", "Spindle", "Compass", "Anchor", "Cairn" };
            int seed = Math.Abs(Environment.TickCount);
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
                        ExecutePlayerCommand($"+coop/enrol {chosenName}");
                    }),
                    ("Account Statement", () => ExecutePlayerCommand("+coop/statement"))
                },
                commandPrefix: "+coop/enrol",
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
                        "STICKNEY SERVICE & PROSPECTOR REFIT DESK",
                        "Refit a chartered Dray into a <b>Dray-P Prospector</b> (adds 2-rig stern Vehicle Bay + 86 t ore hold) and lease Co-op terrain rigs (`+tv`):",
                        new List<(string, Action)>
                        {
                            ("Refit Dray-P (+coop/refit prospector)", () => ExecutePlayerCommand("+coop/refit prospector")),
                            ("Lease Rig 1 (+tv/lease go)", () => ExecutePlayerCommand("+tv/lease go")),
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
            Audio?.PlayUiClick();
            long unitMinor = row.UnitPriceMinor > 0 ? row.UnitPriceMinor : (long)Math.Round(row.UnitPriceMajor * 100m);
            string quoteCmd = TradeState.BeginOneClickBuy(row.ListingId, quantity, Time.time, unitMinor);
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
            ExecutePlayerCommand(sellCmd);
            if (_ws != null && _ws.IsConnected && _ws.HasAdvertisedPanel("port.trade"))
            {
                _awaitingAppendedTradePage = false;
                _ws.SendPanelSubscribe("port.trade");
            }
        }

        public void ExecutePlayerCommand(string cmd)
        {
            if (string.IsNullOrWhiteSpace(cmd)) return;
            Audio?.PlayUiClick();

            if (cmd == "ui:open_coop_enrol")
            {
                ShowCoopEnrolPrompt();
                return;
            }
            if (cmd == "ui:refresh_trade")
            {
                RequestPortTradeRefresh();
                return;
            }
            if (cmd == "ui:open_exchange")
            {
                OpenVisualTradeTerminal("PORT EXCHANGE & HOLD LOGISTICS", openSailingTab: false);
                return;
            }
            if (cmd == "ui:open_sailing")
            {
                OpenVisualTradeTerminal("SAILING PLAN & CORRIDOR ROUTE BUILDER", openSailingTab: true);
                return;
            }
            if (cmd == "seat:helm")
            {
                SitAtBridgeHelm();
                return;
            }
            if (cmd == "surface:launch")
            {
                if (_ws != null && _ws.IsConnected)
                {
                    FlushPendingRoomMovesBeforeAction();
                    int slot = SurfaceStage != null ? SurfaceStage.AddressedRigSlot : 1;
                    _ws.SendLine($"+tv/{slot} launch go");
                }
                return;
            }
            if (cmd == "surface:scan" || cmd == "surface:sound" || cmd == "surface:mine" ||
                cmd == "surface:unload" || cmd == "surface:recover" || cmd == "surface:claim_stake")
            {
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
                SurfaceStage?.SetOreFamilyFilter(fam);
                if (_ws != null && _ws.IsConnected)
                    SendSiteMapPull(fam);
                return;
            }
            if (cmd == "space:mine_ping")
            {
                CameraRig.SetMode(CameraRigMode.OrbitalExterior);
                if (_ws != null && _ws.IsConnected) _ws.SendLine("+mine/ping");
                return;
            }
            if (cmd.StartsWith("space:rock:", StringComparison.OrdinalIgnoreCase))
            {
                string rId = cmd.Substring("space:rock:".Length).Trim().ToUpperInvariant();
                SpaceStage.SelectRock(rId);
                Hud.Overlays?.UpdateOrbitalTargetReadout(SpaceStage.SelectedRockId, SpaceStage.LockedContactId);
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine($"+helm/approach {rId}");
                return;
            }
            if (cmd == "space:mine_laser")
            {
                if (string.IsNullOrEmpty(SpaceStage.SelectedRockId))
                {
                    Hud.AppendLogLine("<color=#FFAA55>[MINING]</color> Select an orbital rock (`A`, `B`, or `C`) first.");
                    return;
                }
                CameraRig.SetMode(CameraRigMode.OrbitalExterior);
                if (_ws != null && _ws.IsConnected)
                {
                    _ws.SendLine($"+mine/laser {SpaceStage.SelectedRockId}");
                    if (SpaceStage.HasTargetableRock(SpaceStage.SelectedRockId))
                        SpaceStage.TriggerMiningLaser();
                }
                return;
            }
            if (cmd == "space:mine_drones")
            {
                if (string.IsNullOrEmpty(SpaceStage.SelectedRockId))
                {
                    Hud.AppendLogLine("<color=#FFAA55>[MINING]</color> Select an orbital rock (`A`, `B`, or `C`) before launching collector drones.");
                    return;
                }
                CameraRig.SetMode(CameraRigMode.OrbitalExterior);
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine($"+mine/drones {SpaceStage.SelectedRockId} 1");
                return;
            }
            if (cmd == "space:mine_stake")
            {
                if (string.IsNullOrEmpty(SpaceStage.SelectedRockId))
                {
                    Hud.AppendLogLine("<color=#FFAA55>[MINING]</color> Select an orbital rock (`A`, `B`, or `C`) before staking a claim.");
                    return;
                }
                if (_ws != null && _ws.IsConnected)
                    _ws.SendLine($"+mine/stake {SpaceStage.SelectedRockId}");
                return;
            }
            if (cmd.StartsWith("space:lock:", StringComparison.OrdinalIgnoreCase))
            {
                string tgt = cmd.Substring("space:lock:".Length).Trim();
                SpaceStage.SetLockedContact(tgt);
                Hud.Overlays?.UpdateOrbitalTargetReadout(SpaceStage.SelectedRockId, SpaceStage.LockedContactId);
                if (_ws != null && _ws.IsConnected && !string.IsNullOrEmpty(SpaceStage.LockedContactId))
                    _ws.SendLine($"+tac/lock {SpaceStage.LockedContactId}");
                return;
            }
            if (cmd == "space:tac_beam" || cmd == "space:tac_railgun" || cmd == "space:tac_torp")
            {
                CameraRig.SetMode(CameraRigMode.OrbitalExterior);
                string kind = cmd == "space:tac_railgun" ? "railgun" : (cmd == "space:tac_torp" ? "torp" : "beam");
                if (_ws != null && _ws.IsConnected)
                {
                    _ws.SendLine($"+tac/{kind}");
                    if (SpaceStage.HasPerceivedContact(SpaceStage.LockedContactId))
                        SpaceStage.TriggerNavalWeaponVfx(kind);
                }
                return;
            }
            if (cmd.StartsWith("helm:thrust:", StringComparison.OrdinalIgnoreCase))
            {
                if (int.TryParse(cmd.Substring("helm:thrust:".Length), out int permille))
                {
                    SpaceStage.SetLocalHelmPreview(SpaceStage.ShipYawDeg, SpaceStage.ShipPitchDeg, permille);
                    Hud.UpdateHelmStatus(SpaceStage.ThrottlePermille, SpaceStage.EmconPosture);
                    if (_ws != null && _ws.IsConnected)
                    {
                        FlushPendingRoomMovesBeforeAction();
                        _ws.SendLine($"+helm/thrust {permille}");
                    }
                }
                return;
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
                if (_ws != null && _ws.IsConnected)
                {
                    Hud.AppendLogLine("<color=#66CCFF>> +helm/dock ceres</color>");
                    _ws.SendLine("+helm/dock ceres");
                }
                return;
            }

            if (cmd.Equals("+ship/board", StringComparison.OrdinalIgnoreCase) || cmd.Equals("+ship/leave", StringComparison.OrdinalIgnoreCase))
            {
                Audio?.PlayAirlockCycle();
            }

            if (cmd.StartsWith("+tv/", StringComparison.OrdinalIgnoreCase) && SurfaceStage != null)
            {
                if (cmd.IndexOf("scan", StringComparison.OrdinalIgnoreCase) >= 0)
                    SurfaceStage.TriggerScanPulseVfx(seismicSounding: false);
                else if (cmd.IndexOf("sound", StringComparison.OrdinalIgnoreCase) >= 0)
                    SurfaceStage.TriggerScanPulseVfx(seismicSounding: true);
                else if (cmd.IndexOf("mine", StringComparison.OrdinalIgnoreCase) >= 0)
                    SurfaceStage.TriggerDrillVfx(3.0f);
            }

            if (cmd.StartsWith("+market/list", StringComparison.OrdinalIgnoreCase))
                TradeState.NotifyMarketRefreshRequested();

            Hud.AppendLogLine($"<color=#66CCFF>> {cmd}</color>");
            if (Hud.IsModalOpen)
                Hud.AppendModalResponseLine($"<color=#66CCFF>> {cmd}</color>", resetPreviousResponses: true);

            if (_ws != null && _ws.IsConnected)
            {
                FlushPendingRoomMovesBeforeAction();
                _ws.SendLine(cmd);
            }
            else
            {
                const string msg = "<color=#FF7766>[NET] Reconnecting or disconnected from mushd — command held.</color>";
                Hud.AppendLogLine(msg);
                if (Hud.IsModalOpen)
                    Hud.AppendModalResponseLine(msg);
            }
        }

        private void SitAtBridgeHelm()
        {
            Player.IsSeatedAtHelm = true;
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
            Hud.AppendLogLine(isClampCandidate
                ? "<color=#88DDFF>[DOCKING]</color> Collar gates aligned — submitting 60 Hz RLE tape for server verification..."
                : "<color=#FFAA66>[DOCKING]</color> Collar contact outside gate tolerance — submitting 60 Hz RLE tape...");
            if (_ws != null && _ws.IsConnected)
                _ws.SendLine(tapeCommand);
        }

        private void HandleDockingEndedByServer(string outcome)
        {
            string norm = (outcome ?? "closed").ToLowerInvariant();
            if (norm == "clamped")
            {
                Audio?.PlayDockHardClamp();
                Hud.AppendLogLine("<color=#55FF99>[HARD CLAMP CONFIRMED]</color> Authoritative server collar seal locked.");
                CameraRig.SetMode(CameraRigMode.ShipBridgeHolo);
                Audio?.SetSoundscape(AmbientSoundscape.ShipInterior);
            }
            else
            {
                Hud.AppendLogLine($"<color=#FF8866>[DOCKING CLOSED]</color> Session ended ({norm}).");
                CameraRig.SetMode(Player.IsSeatedAtHelm ? CameraRigMode.OrbitalExterior : CameraRigMode.ThirdPersonWalk);
            }
        }

        private void HandleDockingAborted()
        {
            Hud.AppendLogLine("<color=#FF8866>[DOCKING]</color> Approach aborted.");
            if (_ws != null && _ws.IsConnected)
                _ws.SendLine("+helm/dock abort");
            CameraRig.SetMode(CameraRigMode.OrbitalExterior);
        }

        private void HandleServerRoomView(MushRoomView rv)
        {
            _pendingMoveTarget = null;
            _nextMoveStepAllowedAt = 0f;

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

            bool enteringShipInterior = ShipInterior.TryResolveCompartmentFromRoomView(rv, out string resolvedComp);
            if (enteringShipInterior)
            {
                if (ArenaStage != null && ArenaStage.IsArenaActive)
                    ArenaStage.ExitArena();
                if (SurfaceStage != null && SurfaceStage.IsExcursionActive && resolvedComp != "bay")
                    SurfaceStage.ExitSurfaceMode();

                bool wasAboard = _isAboardShip;
                bool firstSync = _awaitingInitialRoomSync;
                _awaitingInitialRoomSync = false;

                _isAboardShip = true;
                _serverShipCompartment = resolvedComp;
                _serverStationRoomEntry = null;
                ShipInterior.ApplyRoomView(rv);
                Audio?.SetSoundscape(AmbientSoundscape.ShipInterior);

                if (firstSync || !wasAboard || Vector3.Distance(Player.transform.position, ShipInterior.GetCompartmentSpawnWorldPos(resolvedComp)) > 16f)
                {
                    Player.IsSeatedAtHelm = false;
                    Player.TeleportTo(ShipInterior.GetCompartmentSpawnWorldPos(resolvedComp), 180f);
                    CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
                }

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
                Hud.RefreshContextualQuickActions($"interior:{resolvedComp}", Player.IsSeatedAtHelm);
            }
            else if (StationWorld.TryMatchStationRoom(rv, out string matchedStationEntry))
            {
                if (ArenaStage != null && ArenaStage.IsArenaActive && (rv.TacSim == null || !rv.TacSim.SimInPod))
                    ArenaStage.ExitArena();
                if (SurfaceStage != null && SurfaceStage.IsExcursionActive)
                    SurfaceStage.ExitSurfaceMode();

                bool wasAboard = _isAboardShip;
                bool firstSync = _awaitingInitialRoomSync;
                _awaitingInitialRoomSync = false;

                _isAboardShip = false;
                Player.IsSeatedAtHelm = false;
                _serverStationRoomEntry = matchedStationEntry;
                _serverShipCompartment = null;
                StationWorld.ApplyRoomView(rv, _ws != null && _ws.ActiveCharacter != null ? _ws.ActiveCharacter.Id : null);
                Audio?.SetSoundscape(AmbientSoundscape.StationRing);

                if (firstSync || wasAboard || (StationWorld.FindContainingRoomEntry(Player.transform.position) != matchedStationEntry &&
                    Vector3.Distance(Player.transform.position, StationWorld.GetRoomSpawnPosition(matchedStationEntry)) > 32f))
                {
                    Player.TeleportTo(StationWorld.GetRoomSpawnPosition(matchedStationEntry), 0f);
                    CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
                }

                if (rv.TacSim != null && !string.IsNullOrEmpty(rv.TacSim.OwnHullName))
                    _currentHullBadge = $"{rv.TacSim.OwnHullName} — {(rv.TacSim.OwnHullDockedHere ? "CRADLED HERE" : "AWAY")}";

                string surface = rv.Atlas != null && !string.IsNullOrEmpty(rv.Atlas.Surface) ? rv.Atlas.Surface.ToUpperInvariant() : "STATION";
                Hud.UpdateTopBar($"{rv.Name.ToUpperInvariant()} [{surface}]", $"{_currentClockStr} | {_currentHullBadge}");
                Hud.RefreshContextualQuickActions(matchedStationEntry, false);
            }
            else
            {
                _isAboardShip = false;
                _serverStationRoomEntry = null;
                _serverShipCompartment = null;

                string ceresExitAlias = FindExitToCeresFreeport(rv);
                if (_ws != null && _ws.IsConnected && !_limboAutoEnterAttempted && !string.IsNullOrEmpty(ceresExitAlias))
                {
                    _limboAutoEnterAttempted = true;
                    Hud.AppendLogLine($"<color=#88DDFF>[TRANSIT]</color> Stepping from <b>{rv.Name}</b> into Ceres Freeport ({ceresExitAlias})...");
                    _ws.SendLine(ceresExitAlias);
                    return;
                }

                _awaitingInitialRoomSync = false;
                Hud.UpdateTopBar($"{rv.Name.ToUpperInvariant()} [OUTSIDE PORT]", $"{_currentClockStr} | {_currentHullBadge}");
                Hud.RefreshContextualQuickActions(string.Empty, false);
            }

            if (!string.IsNullOrWhiteSpace(rv.Name))
            {
                string descMarkup = !string.IsNullOrWhiteSpace(rv.RichDesc) ? rv.RichDesc : rv.Desc;
                Hud.AppendLogLine($"<color=#88DDFF>[{rv.Name}]</color> {descMarkup}");
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
                    string shortUnit = (s.Unit ?? string.Empty).Replace("tacsim.unit.", string.Empty);
                    sb.AppendFormat("<color={0}>{1}{2}</color> {3} ({4}){5}",
                        col,
                        string.Equals(s.Team, "red", StringComparison.OrdinalIgnoreCase) ? "R" : "B",
                        s.Index + 1,
                        s.PilotName,
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
                    string shortUnit = s.Str("unit", "line_tank").Replace("tacsim.unit.", string.Empty);
                    string safeName = MushProtocol.EscapeUnityRichText(s.Str("pilot_name", string.Empty));
                    sb.AppendFormat("<color={0}>{1}{2}</color> {3} ({4}){5}",
                        col,
                        string.Equals(team, "red", StringComparison.OrdinalIgnoreCase) ? "R" : "B",
                        s.Int("index", 1),
                        safeName,
                        shortUnit,
                        tag);
                }
                ArenaStage.LiveLobbySlotSummary = sb.ToString();
            }
            Hud.Overlays?.RefreshLobbySelectionHighlights();
        }

        private static string FindExitToCeresFreeport(MushRoomView rv)
        {
            if (rv == null || rv.Exits == null) return null;
            for (int i = 0; i < rv.Exits.Count; i++)
            {
                var ex = rv.Exits[i];
                bool matchesCeres = ex.Name.IndexOf("Ceres", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                    ex.Name.IndexOf("Freeport", StringComparison.OrdinalIgnoreCase) >= 0 ||
                                    ex.Name.IndexOf("Arrivals", StringComparison.OrdinalIgnoreCase) >= 0;
                if (matchesCeres)
                    return ex.Aliases.Count > 0 ? ex.Aliases[0] : ex.Name;

                for (int j = 0; j < ex.Aliases.Count; j++)
                {
                    string al = ex.Aliases[j];
                    if (string.Equals(al, "ceres", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(al, "freeport", StringComparison.OrdinalIgnoreCase))
                        return al;
                }
            }
            return null;
        }

        private void HandleServerNarrative(MushNarrativeEvent nar)
        {
            string speaker = nar.Actor != null ? nar.Actor.Name : "";
            string body = !string.IsNullOrEmpty(nar.RichMarkup) ? nar.RichMarkup : nar.Text;
            string formatted = nar.Kind == "say" && !string.IsNullOrEmpty(speaker)
                ? $"<b>{speaker}</b> says, \"{body}\""
                : (!string.IsNullOrEmpty(speaker) ? $"<b>{speaker}</b> {body}" : body);

            Hud.AppendLogLine(formatted);
            if (Hud.IsModalOpen)
                Hud.AppendModalResponseLine(formatted);
            if (nar.Actor != null && !_isAboardShip)
                StationWorld.ShowActorSpeech(nar.Actor.Id, nar.Actor.Name, nar.Text);
        }

        private void HandleServerChannel(MushChannelEvent ch)
        {
            string body = !string.IsNullOrEmpty(ch.RichMarkup) ? ch.RichMarkup : ch.Text;
            Hud.AppendLogLine($"<color=#77DDAA>[{ch.Channel}]</color> {ch.Actor?.Name}: {body}");
        }

        private void HandleServerComposing(MushComposingEvent comp)
        {
            if (comp?.Actor == null || _isAboardShip || StationWorld == null) return;
            if (_ws?.ActiveCharacter != null && comp.Actor.Id == _ws.ActiveCharacter.Id) return;
            StationWorld.SetActorComposing(comp.Actor.Id, comp.Actor.Name, comp.Active);
        }

        private void HandleServerSystemLine(string plainLine, string richMarkup)
        {
            string display = !string.IsNullOrEmpty(richMarkup)
                ? $"<color=#BBD8EE>{richMarkup}</color>"
                : $"<color=#BBD8EE>{plainLine}</color>";
            Hud.AppendLogLine(display);
            if (Hud.IsModalOpen)
                Hud.AppendModalResponseLine(display);

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
                    if (TradeState != null &&
                        TradeState.TryResolvePendingQuoteFromFeed(data, Time.time, out string acceptCmd, out string priceWarn))
                    {
                        if (!string.IsNullOrEmpty(acceptCmd) && _ws != null && _ws.IsConnected)
                        {
                            Hud.AppendLogLine($"<color=#66EEAA>[QUOTE VERIFIED]</color> {acceptCmd}");
                            _ws.SendLine(acceptCmd);
                            _ws.SendPanelSubscribe("port.trade");
                        }
                        else if (!string.IsNullOrEmpty(priceWarn))
                        {
                            Hud.AppendLogLine($"<color=#FFBB55>[PRICE MOVED]</color> {priceWarn}");
                            if (Hud.IsModalOpen)
                                Hud.AppendModalResponseLine($"<color=#FFBB55>{priceWarn}</color>");
                        }
                    }
                    if (data != null && data.IsObject && data["simulator"].IsObject && ArenaStage != null)
                        ApplySimulatorTradeRoster(data["simulator"]);
                    break;
                }
                case "space.holotank":
                    ShipInterior.PopulateHoloBlips(data);
                    SpaceStage.ApplyHolotankFeed(data);
                    Hud.Overlays?.UpdateOrbitalTargetReadout(SpaceStage.SelectedRockId, SpaceStage.LockedContactId);
                    break;
                case "space.cockpit":
                    SpaceStage.ApplyCockpitFeed(data);
                    Hud.UpdateHelmStatus(SpaceStage.ThrottlePermille, SpaceStage.EmconPosture);
                    Hud.Overlays?.UpdateOrbitalTargetReadout(SpaceStage.SelectedRockId, SpaceStage.LockedContactId);
                    break;
                case "space.docking":
                    SpaceStage.StartDockingFromFeed(data);
                    if (SpaceStage.IsDockingActive)
                    {
                        CameraRig.SetMode(CameraRigMode.BowDockingCollar);
                        Audio?.SetSoundscape(AmbientSoundscape.OrbitalSpace);
                    }
                    break;
                case "tv.status":
                    if (SurfaceStage != null)
                        SurfaceStage.ApplyTvStatusFrame(data);
                    if (_ws != null && _ws.IsConnected && data != null && data.IsObject && data.Bool("seated", true) && !data.Bool("cleared", false))
                        SendSiteMapPull(SurfaceStage != null ? SurfaceStage.ActiveOreFamily : "volatiles");
                    break;
                case "site.map":
                    if (SurfaceStage != null && data != null && data.IsObject && data["rows"].IsArray)
                    {
                        SurfaceStage.ApplySiteMapFrame(data);
                        var youArr = data["you"].AsArray();
                        if (!SurfaceStage.IsExcursionActive && youArr != null && youArr.Count >= 2)
                            EnterPlanetarySurfaceStage();
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
                            BindCameraToArenaRig();
                            Hud.UpdateTopBar(
                                $"OOC SIMULATOR ARENA — {ArenaStage.State.Callsign} [{ArenaStage.State.Mode.ToUpperInvariant()}]",
                                $"LIVE MATCH #{ArenaStage.State.MatchId} | WASD: Drive | LMB: Lock | SPACE: Fire");
                        }
                        else if (wasActive)
                        {
                            CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
                            Audio?.SetSoundscape(AmbientSoundscape.StationRing);
                        }
                    }
                    break;
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
            CameraRig.GroundVehicleTarget = SurfaceStage.RoverTransform;
            CameraRig.GroundCupolaAnchor = SurfaceStage.RoverCupolaAnchor;
            CameraRig.SetMode(CameraRigMode.GroundVehicleChase);
            Hud.Overlays?.SetHelmSeatedVisible(false);
            Hud.UpdateTopBar(
                $"{SurfaceStage.ActiveSiteTitle.ToUpperInvariant()} — SURFACE ROVER RIG {SurfaceStage.AddressedRigSlot}",
                "WASD / LMB Terrain: Drive | RMB + Scroll: Camera | Scan -> Sound -> Core Drill");

            if (_ws != null && _ws.IsConnected)
                SendSiteMapPull(SurfaceStage.ActiveOreFamily);
        }

        private void ExitPlanetarySurfaceStage()
        {
            SurfaceStage?.ExitSurfaceMode();
            if (_ws != null && _ws.IsConnected)
                _ws.SendLine($"+tv/{(SurfaceStage != null ? SurfaceStage.AddressedRigSlot : 1)} recover go");
            CameraRig.SetMode(Player.IsSeatedAtHelm ? CameraRigMode.ShipBridgeHolo : CameraRigMode.ThirdPersonWalk);
            Hud.Overlays?.SetHelmSeatedVisible(Player.IsSeatedAtHelm);
            Hud.UpdateTopBar("RETURNED FROM SURFACE EXCURSION", $"{_currentClockStr} | {_currentHullBadge}");
        }

        private void HandleArenaCockpitVerb(string verb)
        {
            if (ArenaStage == null) return;
            ArenaStage.ExecuteCockpitAction(verb);
            if (string.Equals(verb?.Trim(), "exit", StringComparison.OrdinalIgnoreCase) && ArenaStage.IsArenaActive)
                ArenaStage.ExitArena();

            if (!ArenaStage.IsArenaActive)
            {
                CameraRig.SetMode(CameraRigMode.ThirdPersonWalk);
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
