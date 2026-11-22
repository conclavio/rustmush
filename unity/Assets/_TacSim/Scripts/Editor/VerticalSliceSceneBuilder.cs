using TacSim.Arena;
using TacSim.Character;
using TacSim.Core;
using TacSim.Space;
using TacSim.Surface;
using TacSim.UI;
using TacSim.World;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.Rendering;

namespace TacSim.Editor
{
    /// <summary>
    /// Constructs and saves Assets/_TacSim/Scenes/VerticalSlice1.unity with:
    /// - URP Lighting &amp; Global Volume Profile
    /// - 3-Deck Ceres Freeport Station (built from crates/tacsim/content/atlas/neighborhood.ceres_freeport.json)
    /// - Walkable 6-Compartment Dray Ship Interior (built from crates/tacsim/content/units/dray.json)
    /// - 3D Orbital Space Stage + 60 Hz 6-DOF Bow Docking Collar Rig
    /// - 3rd-Person Player Character, Orbit/Mode-Blending Camera Rig, and Diegetic Sci-Fi HUD
    /// </summary>
    public static class VerticalSliceSceneBuilder
    {
        public const string ScenePath = "Assets/_TacSim/Scenes/VerticalSlice1.unity";
        public const string StandaloneScenePath = "Assets/_TacSim/Scenes/StandaloneBootstrap.unity";

        [MenuItem("TacSim/2. Build Vertical Slice Scene", priority = 20)]
        public static void BuildFromMenu()
        {
            BuildVerticalSliceScene();
            Debug.Log($"[TacSim] Built and opened {ScenePath}. Press Play in Unity!");
        }

        public static void BuildFromCli()
        {
            BuildVerticalSliceScene();
        }

        /// <summary>
        /// Builds a clean, minimal bootstrap scene for standalone player builds.
        /// Keeps only Lighting, Global PostProcess Volume, Main Camera + TacSimCameraRig, and
        /// TacSimGameCoordinator referencing TacSimAssetCatalog so no batchmode <c>-nographics</c>
        /// procedural meshes or nested private MonoBehaviours are serialized into <c>level0</c>.
        /// All 5 stages, the player character, and the diegetic HUD are built cleanly on the
        /// player's real GPU during <see cref="TacSimGameCoordinator.EnsureSubsystems"/>.
        /// </summary>
        public static void BuildStandaloneBootstrapScene()
        {
            UrpBootstrap.EnsureUrpAssigned();
            EnsureFolder("Assets/_TacSim", "Scenes");

            AssetPackImporter.EnsureIdaFaberHumanoidRigs();
            var controller = HumanoidAnimatorBuilder.BuildOrRefreshController();
            var catalog = AssetPackImporter.PopulateOrRefreshCatalog(controller);

            var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);

            // 1. Lighting & Sky / Volume
            RenderSettings.ambientMode = AmbientMode.Trilight;
            RenderSettings.ambientSkyColor = new Color(0.24f, 0.30f, 0.38f);
            RenderSettings.ambientEquatorColor = new Color(0.16f, 0.20f, 0.26f);
            RenderSettings.ambientGroundColor = new Color(0.08f, 0.10f, 0.14f);

            var keyLightGo = new GameObject("DirectionalKeyLight");
            keyLightGo.transform.rotation = Quaternion.Euler(46f, -38f, 0f);
            var dirLight = keyLightGo.AddComponent<Light>();
            dirLight.type = LightType.Directional;
            dirLight.color = new Color(0.95f, 0.97f, 1.0f);
            dirLight.intensity = 1.15f;
            dirLight.shadows = LightShadows.Soft;

            var volumeProfile = AssetDatabase.LoadAssetAtPath<VolumeProfile>("Assets/_TacSim/Settings/SampleSceneProfile.asset");
            if (volumeProfile != null)
            {
                var volGo = new GameObject("GlobalPostProcessVolume");
                var vol = volGo.AddComponent<Volume>();
                vol.isGlobal = true;
                vol.sharedProfile = volumeProfile;
            }

            // 2. Main Camera + TacSimCameraRig
            var camGo = new GameObject("Main Camera");
            camGo.tag = "MainCamera";
            var cam = camGo.AddComponent<Camera>();
            cam.clearFlags = CameraClearFlags.SolidColor;
            cam.backgroundColor = new Color(0.02f, 0.03f, 0.06f);
            camGo.AddComponent<AudioListener>();
            var camRig = camGo.AddComponent<TacSimCameraRig>();
            camRig.ControlledCamera = cam;

            // 3. Top-Level Game Coordinator (builds stages, player, and HUD on Start)
            var coordGo = new GameObject("TacSimGameCoordinator");
            var coord = coordGo.AddComponent<TacSimGameCoordinator>();
            coord.AssetCatalog = catalog;
            coord.CameraRig = camRig;

            EditorSceneManager.SaveScene(scene, StandaloneScenePath);
            EditorBuildSettings.scenes = new[]
            {
                new EditorBuildSettingsScene(StandaloneScenePath, enabled: true)
            };
            AssetDatabase.SaveAssets();
        }

        public static void BuildVerticalSliceScene()
        {
            UrpBootstrap.EnsureUrpAssigned();
            EnsureFolder("Assets/_TacSim", "Scenes");

            AssetPackImporter.EnsureIdaFaberHumanoidRigs();
            var controller = HumanoidAnimatorBuilder.BuildOrRefreshController();
            var catalog = AssetPackImporter.PopulateOrRefreshCatalog(controller);

            var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);

            // 1. Lighting & Sky / Volume
            RenderSettings.ambientMode = AmbientMode.Trilight;
            RenderSettings.ambientSkyColor = new Color(0.24f, 0.30f, 0.38f);
            RenderSettings.ambientEquatorColor = new Color(0.16f, 0.20f, 0.26f);
            RenderSettings.ambientGroundColor = new Color(0.08f, 0.10f, 0.14f);

            var keyLightGo = new GameObject("DirectionalKeyLight");
            keyLightGo.transform.rotation = Quaternion.Euler(46f, -38f, 0f);
            var dirLight = keyLightGo.AddComponent<Light>();
            dirLight.type = LightType.Directional;
            dirLight.color = new Color(0.95f, 0.97f, 1.0f);
            dirLight.intensity = 1.15f;
            dirLight.shadows = LightShadows.Soft;

            var volumeProfile = AssetDatabase.LoadAssetAtPath<VolumeProfile>("Assets/_TacSim/Settings/SampleSceneProfile.asset");
            if (volumeProfile != null)
            {
                var volGo = new GameObject("GlobalPostProcessVolume");
                var vol = volGo.AddComponent<Volume>();
                vol.isGlobal = true;
                vol.sharedProfile = volumeProfile;
            }

            // 2. Ceres Freeport Station (3 decks)
            var stationGo = new GameObject("CeresFreeportStation");
            var station = stationGo.AddComponent<StationWorldController>();
            station.BuildStation(catalog);

            // 3. Dray Ship Walkable Interior (6 compartments + Bridge Holo-Tank)
            var shipGo = new GameObject("DrayShipInterior");
            var shipInterior = shipGo.AddComponent<ShipInteriorController>();
            shipInterior.BuildInterior(catalog);

            // 4. Orbital Space Flight & 60 Hz 6-DOF Docking Stage
            var spaceGo = new GameObject("OrbitalSpaceStage");
            var spaceStage = spaceGo.AddComponent<SpaceFlightController>();
            spaceStage.BuildSpaceStage(catalog);

            // 4B. Planetary Surface Mining Stage (Slice 3) & OOC Red vs Blue Arena Stage
            var surfGo = new GameObject("PlanetarySurfaceStage");
            var surfStage = surfGo.AddComponent<SurfaceMiningController>();
            surfStage.BuildSurfaceStage(catalog, "occator_rim");

            var arenaGo = new GameObject("OocArenaStage");
            var arenaStage = arenaGo.AddComponent<OocArenaController>();
            arenaStage.Catalog = catalog;

            // 5. Main Camera + TacSimCameraRig
            var camGo = new GameObject("Main Camera");
            camGo.tag = "MainCamera";
            var cam = camGo.AddComponent<Camera>();
            cam.clearFlags = CameraClearFlags.SolidColor;
            cam.backgroundColor = new Color(0.02f, 0.03f, 0.06f);
            camGo.AddComponent<AudioListener>();
            var camRig = camGo.AddComponent<TacSimCameraRig>();
            camRig.ControlledCamera = cam;

            // 6. 3rd-Person Player Character spawned in Port Watch Arrivals
            var playerGo = new GameObject("PlayerCharacter");
            playerGo.tag = "Player";
            playerGo.AddComponent<CharacterController>();
            var vis = playerGo.AddComponent<CharacterVisualSpawner>();
            var player = playerGo.AddComponent<ThirdPersonPlayerController>();
            player.VisualSpawner = vis;
            player.CameraTransform = cam.transform;
            vis.BuildPlayerVisual(catalog, PlayerAvatarPreset.StellarTrooperFemale);

            Vector3 spawnPos = station.GetRoomSpawnPosition("room.c03_arrivals");
            player.TeleportTo(spawnPos, 90f);

            camRig.WalkFollowTarget = player.transform;
            camRig.BridgeViewAnchor = shipInterior.BridgeCameraAnchor;
            camRig.OrbitalShipTarget = spaceStage.PlayerShipTransform;
            camRig.BowDockingAnchor = spaceStage.BowDockingCameraAnchor;
            camRig.SnapBehindTarget(90f);

            // 7. Diegetic HUD & Top-Level Game Coordinator
            var hudGo = new GameObject("TacSimHud");
            var hud = hudGo.AddComponent<TacSimHudController>();
            hud.BuildHud();

            var coordGo = new GameObject("TacSimGameCoordinator");
            var coord = coordGo.AddComponent<TacSimGameCoordinator>();
            coord.AssetCatalog = catalog;
            coord.StationWorld = station;
            coord.ShipInterior = shipInterior;
            coord.SpaceStage = spaceStage;
            coord.SurfaceStage = surfStage;
            coord.ArenaStage = arenaStage;
            coord.CameraRig = camRig;
            coord.Player = player;
            coord.Hud = hud;

            EditorSceneManager.SaveScene(scene, ScenePath);
            EditorBuildSettings.scenes = new[]
            {
                new EditorBuildSettingsScene(ScenePath, enabled: true)
            };
            AssetDatabase.SaveAssets();
        }

        private static void EnsureFolder(string parent, string child)
        {
            string full = $"{parent}/{child}";
            if (!AssetDatabase.IsValidFolder(full))
                AssetDatabase.CreateFolder(parent, child);
        }
    }
}
