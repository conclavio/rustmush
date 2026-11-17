using TacSim.Arena;
using TacSim.Character;
using TacSim.Space;
using TacSim.Surface;
using TacSim.World;
using UnityEngine;

namespace TacSim.Core
{
    public enum TacSimActiveStage
    {
        StationWalk,
        ShipInteriorWalk,
        ShipHelmSeated,
        PlanetarySurface,
        SimulatorArena
    }

    /// <summary>
    /// Enforces strict single-stage 3D GameObject visibility, camera background/sky color,
    /// ambient soundscape routing, and keyboard/orbit input suspension across all 5 gameplay stages.
    /// </summary>
    public sealed class TacSimStageDirector
    {
        private readonly StationWorldController _station;
        private readonly ShipInteriorController _shipInterior;
        private readonly SpaceFlightController _spaceStage;
        private readonly SurfaceMiningController _surfaceStage;
        private readonly OocArenaController _arenaStage;
        private readonly TacSimCameraRig _cameraRig;
        private readonly ThirdPersonPlayerController _player;
        private readonly TacSimAudioSynthesizer _audio;

        public TacSimActiveStage CurrentStage { get; private set; } = TacSimActiveStage.StationWalk;

        public TacSimStageDirector(
            StationWorldController station,
            ShipInteriorController shipInterior,
            SpaceFlightController spaceStage,
            SurfaceMiningController surfaceStage,
            OocArenaController arenaStage,
            TacSimCameraRig cameraRig,
            ThirdPersonPlayerController player,
            TacSimAudioSynthesizer audio)
        {
            _station = station;
            _shipInterior = shipInterior;
            _spaceStage = spaceStage;
            _surfaceStage = surfaceStage;
            _arenaStage = arenaStage;
            _cameraRig = cameraRig;
            _player = player;
            _audio = audio;
        }

        public void SyncFrame(bool isAboardShip, bool isTextInputFocused)
        {
            bool inArena = _arenaStage != null && _arenaStage.IsArenaActive;
            bool inSurface = !inArena && _surfaceStage != null && _surfaceStage.IsExcursionActive;
            bool seatedAtHelm = !inArena && !inSurface && _player != null && _player.IsSeatedAtHelm;

            CurrentStage = inArena ? TacSimActiveStage.SimulatorArena
                         : inSurface ? TacSimActiveStage.PlanetarySurface
                         : seatedAtHelm ? TacSimActiveStage.ShipHelmSeated
                         : isAboardShip ? TacSimActiveStage.ShipInteriorWalk
                         : TacSimActiveStage.StationWalk;

            SetActiveIfChanged(_station != null ? _station.gameObject : null, CurrentStage == TacSimActiveStage.StationWalk);
            SetActiveIfChanged(_shipInterior != null ? _shipInterior.gameObject : null, CurrentStage == TacSimActiveStage.ShipInteriorWalk || CurrentStage == TacSimActiveStage.ShipHelmSeated);
            SetActiveIfChanged(_spaceStage != null ? _spaceStage.gameObject : null, CurrentStage == TacSimActiveStage.ShipHelmSeated);
            SetActiveIfChanged(_surfaceStage != null ? _surfaceStage.gameObject : null, CurrentStage == TacSimActiveStage.PlanetarySurface);
            SetActiveIfChanged(_arenaStage != null ? _arenaStage.gameObject : null, CurrentStage == TacSimActiveStage.SimulatorArena);

            if (_cameraRig != null && _cameraRig.ControlledCamera != null)
            {
                _cameraRig.ControlledCamera.clearFlags = CameraClearFlags.SolidColor;
                if (inArena)
                {
                    string biome = _arenaStage.TerrainGrid != null ? _arenaStage.TerrainGrid.Biome : "desert";
                    _cameraRig.ControlledCamera.backgroundColor = biome == "desert"
                        ? new Color(0.18f, 0.11f, 0.09f)
                        : (biome == "arctic" ? new Color(0.14f, 0.19f, 0.25f) : new Color(0.11f, 0.15f, 0.20f));
                }
                else
                {
                    _cameraRig.ControlledCamera.backgroundColor = new Color(0.02f, 0.03f, 0.05f);
                }
            }

            bool dockingActive = _spaceStage != null && _spaceStage.IsDockingActive;
            bool inVehicleStage = inArena || inSurface;
            if (_player != null)
                _player.InputSuspended = isTextInputFocused || dockingActive || inVehicleStage;
            if (_cameraRig != null)
                _cameraRig.OrbitInputEnabled = !isTextInputFocused && !dockingActive;
            if (_surfaceStage != null)
                _surfaceStage.KeyboardSuspended = isTextInputFocused;
            if (_arenaStage != null)
                _arenaStage.KeyboardSuspended = isTextInputFocused;
        }

        public void ApplySoundscapeForCamera(CameraRigMode mode, bool isAboardShip)
        {
            if (_audio == null) return;
            if (mode == CameraRigMode.OrbitalExterior || mode == CameraRigMode.BowDockingCollar)
                _audio.SetSoundscape(AmbientSoundscape.OrbitalSpace);
            else if (isAboardShip)
                _audio.SetSoundscape(AmbientSoundscape.ShipInterior);
            else
                _audio.SetSoundscape(AmbientSoundscape.StationRing);
        }

        private static void SetActiveIfChanged(GameObject go, bool active)
        {
            if (go != null && go.activeSelf != active)
                go.SetActive(active);
        }
    }
}
