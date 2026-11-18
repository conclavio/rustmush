using System;
using System.Collections.Generic;
using UnityEngine;

namespace TacSim.Core
{
    public enum PlayerAvatarPreset
    {
        StellarTrooperMale = 0,
        StellarTrooperFemale = 1,
        CyberpunkStreetFighterMale = 2,
        CyberpunkStreetFighterFemale = 3
    }

    /// <summary>
    /// Registry of imported commercial prefabs and generated animators.
    /// Populated automatically by TacSim -> 1. Import Vertical Slice Assets,
    /// while remaining safe (null-tolerant with clean URP fallbacks) before import.
    /// </summary>
    [CreateAssetMenu(fileName = "TacSimAssetCatalog", menuName = "TacSim/Asset Catalog")]
    public sealed class TacSimAssetCatalog : ScriptableObject
    {
        public const string ResourcePath = "TacSimAssetCatalog";

        [Header("Animation")]
        public RuntimeAnimatorController humanoidAnimator;

        [Header("Ida Faber Player Avatars")]
        public GameObject stellarTrooperMale;
        public GameObject stellarTrooperFemale;
        public GameObject cyberStreetFighterMale;
        public GameObject cyberStreetFighterFemale;

        [Header("Ida Faber Station Cast NPCs (Female: Techwear Girls, Astra, Celeste)")]
        public List<GameObject> npcCharacterPrefabs = new List<GameObject>();

        [Header("Ida Faber Station Cast NPCs (Male: Cyberpunk SF Boys, Male Troopers)")]
        public List<GameObject> maleNpcCharacterPrefabs = new List<GameObject>();

        [Header("Gllow3D Ambient Static Dockworkers")]
        public List<GameObject> ambientWorkerModels = new List<GameObject>();
        public List<Texture> ambientWorkerAlbedos = new List<Texture>();

        [Header("DOTTEAM Heavy Station Kit Prefabs")]
        public GameObject stationFloorTile;
        public GameObject stationWallPanel;
        public GameObject stationDoorFrame;
        public GameObject stationLockedDoor;
        public GameObject stationWindowWall;
        public GameObject stationStairModule;
        public GameObject stationConsoleProp;
        public GameObject stationDisplayProp;
        public GameObject stationPowerConsoleProp;
        public GameObject stationCounterProp;
        public GameObject stationCrateProp;
        public GameObject stationBarrelProp;
        public GameObject stationBatteryProp;
        public GameObject stationPipeProp;
        public GameObject stationSupportProp;
        public Texture stationWallAlbedo;
        public Texture stationWallNormal;
        public Texture stationWallMetallic;
        public Texture stationFreightWallAlbedo;
        public Texture stationFreightWallNormal;
        public Texture stationFloorAlbedo;
        public Texture stationFloorNormal;
        public Texture stationFloorMetallic;
        public Texture stationRockAlbedo;
        public Texture stationRockNormal;

        [Header("Vattalus Fenris Ship Interior & URP Props")]
        public GameObject fenrisExteriorInteriorPrefab;
        public GameObject fenrisHologramPrefab;
        public GameObject shipAirlockDoorPrefab;
        public GameObject shipBlastDoorPrefab;
        public GameObject shipHelmSeatPrefab;
        public GameObject shipVentMistPrefab;

        [Header("MSGDI Space Hulls, Berths, Drones & Asteroids")]
        public GameObject drayExteriorHullPrefab;
        public GameObject visitingFreighterPrefab;
        public GameObject shipyardCradlePrefab;
        public GameObject collectorDronePrefab;
        public List<GameObject> asteroidPrefabs = new List<GameObject>();

        [Header("Planets & Thruster VFX")]
        public GameObject ceresPlanetPrefab;
        public GameObject marsPlanetPrefab;
        public GameObject moonPlanetPrefab;
        public GameObject thrusterPlumePrefab;

        [Header("Slice 2: Zero-G Mining, Corvette & Space Combat VFX")]
        public GameObject corvetteExteriorPrefab;
        public List<GameObject> oreAsteroidPrefabs = new List<GameObject>();
        public GameObject miningContainerPrefab;
        public GameObject miningLaserBeamPrefab;
        public GameObject railgunImpactPrefab;
        public GameObject explosionVfxPrefab;
        public GameObject dugoutWallPanel;

        [Header("Slice 3 & OOC Arena: Rovers, Tanks & Biped Mechs (Slava Z)")]
        public GameObject prospectorRoverPrefab;
        public GameObject escortVehiclePrefab;
        public GameObject lineTankPrefab;
        public GameObject heavyTankPrefab;
        public GameObject tankTurretSinglePrefab;
        public GameObject tankTurretTwinPrefab;
        public GameObject bipedMechPrefab;
        public GameObject bipedCockpitPrefab;
        public GameObject bipedCannonPrefab;
        public GameObject scoutWalkerPrefab;
        public Texture spidersRedAlbedo;
        public Texture spidersBlueAlbedo;
        public Texture ltMedRedAlbedo;
        public Texture ltMedBlueAlbedo;

        public GameObject GetPlayerPrefab(PlayerAvatarPreset preset)
        {
            switch (preset)
            {
                case PlayerAvatarPreset.StellarTrooperMale:
                    return stellarTrooperMale != null ? stellarTrooperMale : FirstNonNullPlayer();
                case PlayerAvatarPreset.StellarTrooperFemale:
                    return stellarTrooperFemale != null ? stellarTrooperFemale : FirstNonNullPlayer();
                case PlayerAvatarPreset.CyberpunkStreetFighterMale:
                    return cyberStreetFighterMale != null ? cyberStreetFighterMale : FirstNonNullPlayer();
                case PlayerAvatarPreset.CyberpunkStreetFighterFemale:
                    return cyberStreetFighterFemale != null ? cyberStreetFighterFemale : FirstNonNullPlayer();
                default:
                    return FirstNonNullPlayer();
            }
        }

        private GameObject FirstNonNullPlayer()
        {
            if (stellarTrooperMale != null) return stellarTrooperMale;
            if (stellarTrooperFemale != null) return stellarTrooperFemale;
            if (cyberStreetFighterMale != null) return cyberStreetFighterMale;
            if (cyberStreetFighterFemale != null) return cyberStreetFighterFemale;
            return null;
        }

        public GameObject GetNpcPrefabForKey(string stableKey, bool isMale = false)
        {
            List<GameObject> pool = isMale && maleNpcCharacterPrefabs != null && maleNpcCharacterPrefabs.Count > 0
                ? maleNpcCharacterPrefabs
                : npcCharacterPrefabs;

            if (pool == null || pool.Count == 0)
                return isMale ? FirstNonNullMaleFallback() : null;

            int validCount = 0;
            for (int i = 0; i < pool.Count; i++)
            {
                if (IsAppropriateNpcPrefab(pool[i])) validCount++;
            }
            if (validCount == 0)
                return isMale ? FirstNonNullMaleFallback() : null;

            uint hash = 2166136261u;
            if (!string.IsNullOrEmpty(stableKey))
            {
                for (int i = 0; i < stableKey.Length; i++)
                {
                    hash ^= stableKey[i];
                    hash *= 16777619u;
                }
            }
            int pick = (int)(hash % (uint)validCount);
            int seen = 0;
            for (int i = 0; i < pool.Count; i++)
            {
                if (!IsAppropriateNpcPrefab(pool[i])) continue;
                if (seen == pick) return pool[i];
                seen++;
            }
            return isMale ? FirstNonNullMaleFallback() : null;
        }

        private GameObject FirstNonNullMaleFallback()
        {
            if (cyberStreetFighterMale != null) return cyberStreetFighterMale;
            if (stellarTrooperMale != null) return stellarTrooperMale;
            return null;
        }

        public static bool IsAppropriateNpcPrefab(GameObject prefab)
        {
            if (prefab == null) return false;
            string n = prefab.name;
            return n.IndexOf("Lingerie", StringComparison.OrdinalIgnoreCase) < 0 &&
                   n.IndexOf("Underwear", StringComparison.OrdinalIgnoreCase) < 0 &&
                   n.IndexOf("Nude", StringComparison.OrdinalIgnoreCase) < 0;
        }

        /// <summary>
        /// In the Unity Editor, automatically populates any newly added Slice 1B/1C asset slots
        /// from imported packages on disk so entering Play Mode immediately renders full commercial dressing.
        /// </summary>
        public void EnsureEditorCatalogHydrated()
        {
#if UNITY_EDITOR
            stationFloorTile = LoadIfNull(stationFloorTile, "Assets/Heavy Station Kit/BASE/Prefabs/Floors/Floor Base 1.prefab");
            stationWallPanel = LoadIfNull(stationWallPanel, "Assets/Heavy Station Kit/HANGARS/Prefabs/Walls/Wall_1.prefab");
            stationWindowWall = LoadIfNull(stationWindowWall, "Assets/Heavy Station Kit/HANGARS/Prefabs/Walls/Wall_2_Win.prefab");
            stationDoorFrame = LoadIfNull(stationDoorFrame, "Assets/Heavy Station Kit/BASE/Prefabs/Arches/Arche A1.prefab");
            stationLockedDoor = LoadIfNull(stationLockedDoor, "Assets/Heavy Station Kit/BASE/Prefabs/Doors/B2_Door_Blocked.prefab");
            stationConsoleProp = LoadIfNull(stationConsoleProp, "Assets/Heavy Station Kit/BASE/Prefabs/Equipment/B2_Cons.prefab");
            stationDisplayProp = LoadIfNull(stationDisplayProp, "Assets/Heavy Station Kit/BASE/Prefabs/Displays/B2_Disp_TB1.prefab");
            stationPowerConsoleProp = LoadIfNull(stationPowerConsoleProp, "Assets/Heavy Station Kit/BASE/Prefabs/Equipment/B2_Cons_Power.prefab");
            stationCrateProp = LoadIfNull(stationCrateProp, "Assets/Heavy Station Kit/HANGARS/Prefabs/Props/hang_box1.prefab");
            stationBarrelProp = LoadIfNull(stationBarrelProp, "Assets/Heavy Station Kit/BASE/Prefabs/Props/Barrel1.prefab");
            stationBatteryProp = LoadIfNull(stationBatteryProp, "Assets/Heavy Station Kit/HANGARS/Prefabs/Props/hang_Battery1.prefab");
            stationPipeProp = LoadIfNull(stationPipeProp, "Assets/Heavy Station Kit/BASE/Prefabs/Pipeline/B2_Pipe1.prefab");
            stationSupportProp = LoadIfNull(stationSupportProp, "Assets/Heavy Station Kit/HANGARS/Prefabs/Supports/Support52.prefab");

            if (stationWallAlbedo == null)
                stationWallAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Walls/B2_Walls0_A.png");
            if (stationWallNormal == null)
                stationWallNormal = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Walls/B2_Walls0_N.png");
            if (stationWallMetallic == null)
                stationWallMetallic = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Walls/B2_Walls0_M.tga");
            if (stationFreightWallAlbedo == null)
                stationFreightWallAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Walls/B2_Walls2_A.png");
            if (stationFreightWallNormal == null)
                stationFreightWallNormal = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Walls/B2_Walls2_N.png");
            if (stationFloorAlbedo == null)
                stationFloorAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Floors/B2_Floors_A.png");
            if (stationFloorNormal == null)
                stationFloorNormal = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Floors/B2_Floors_N.png");
            if (stationFloorMetallic == null)
                stationFloorMetallic = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Floors/B2_Floors_M.tga");
            if (stationRockAlbedo == null)
                stationRockAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Ground/2K-ground_12_basecolor.png");
            if (stationRockNormal == null)
                stationRockNormal = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Heavy Station Kit/BASE/Textures/Ground/2K-ground_12_normal.png");

            fenrisExteriorInteriorPrefab = LoadIfNull(fenrisExteriorInteriorPrefab, "Assets/VattalusAssets/FenrisFrigate/_URP/Prefabs/Fenris_Exterior+Interior_Prefab.prefab");
            shipAirlockDoorPrefab = LoadIfNull(shipAirlockDoorPrefab, "Assets/VattalusAssets/Common_URP/Prefabs/Doors/AirlockDoor_300cm_01_Skin1_Prefab.prefab");
            shipBlastDoorPrefab = LoadIfNull(shipBlastDoorPrefab, "Assets/VattalusAssets/Common_URP/Prefabs/Doors/BlastDoor_300cm_01_Skin1_Prefab.prefab");
            shipHelmSeatPrefab = LoadIfNull(shipHelmSeatPrefab, "Assets/VattalusAssets/Common_URP/Prefabs/Seats/VA_Seat01_Red_Prefab.prefab");
            shipVentMistPrefab = LoadIfNull(shipVentMistPrefab, "Assets/VattalusAssets/Common/Particles/VentMistParticlesM.prefab");

            drayExteriorHullPrefab = LoadIfNull(drayExteriorHullPrefab, "Assets/Freighters/Prefabs/FreighterClassII/Grey/FreighterClassIIGrey.prefab");
            visitingFreighterPrefab = LoadIfNull(visitingFreighterPrefab, "Assets/Freighters/Prefabs/FreighterClassIII/Yellow/FreighterClassIIIYellow.prefab");
            shipyardCradlePrefab = LoadIfNull(shipyardCradlePrefab, "Assets/Shipyards/Prefabs/FrigateShipyard/FrigateShipyardGrey.prefab");
            collectorDronePrefab = LoadIfNull(collectorDronePrefab, "Assets/MiningOperation/Prefabs/ResourceCollector/ResourceCollectorYellow.prefab");
            thrusterPlumePrefab = LoadIfNull(thrusterPlumePrefab, "Assets/VattalusAssets/Common/ThrusterEffects/ThrusterParticles_MainL.prefab");
            thrusterPlumePrefab = LoadIfNull(thrusterPlumePrefab, "Assets/FORGE3D/Sci-Fi Effects/Effects/Flames/flames_flame_blue.prefab");

            // Slice 2: Zero-G Mining, Corvette & Space Combat VFX
            corvetteExteriorPrefab = LoadIfNull(corvetteExteriorPrefab, "Assets/PirateFleetMegaPackI/Prefabs/HammerheadClassPirateFrigate/Grey/HammerheadClassPirateFrigateGrey.prefab");
            miningContainerPrefab = LoadIfNull(miningContainerPrefab, "Assets/MiningOperation/Prefabs/ResourceContainerMediumMasterPrefab.prefab");
            miningLaserBeamPrefab = LoadIfNull(miningLaserBeamPrefab, "Assets/Mech_Constructor_Spiders/Prefabs/FX_Laser_Ray.prefab");
            explosionVfxPrefab = LoadIfNull(explosionVfxPrefab, "Assets/FORGE3D/Sci-Fi Effects/Effects/Explosions/Explosion_001.prefab");

            if (oreAsteroidPrefabs == null) oreAsteroidPrefabs = new List<GameObject>();
            if (oreAsteroidPrefabs.Count == 0)
            {
                string[] orePaths =
                {
                    "Assets/MiningOperation/Prefabs/Asteroids/Asteroid1Resource.prefab",
                    "Assets/MiningOperation/Prefabs/Asteroids/Asteroid2Resource.prefab",
                    "Assets/MiningOperation/Prefabs/Asteroids/Asteroid3Resource.prefab"
                };
                for (int i = 0; i < orePaths.Length; i++)
                {
                    var go = UnityEditor.AssetDatabase.LoadAssetAtPath<GameObject>(orePaths[i]);
                    if (go != null) oreAsteroidPrefabs.Add(go);
                }
            }

            // Slice 3 & OOC Arena: Slava Z Rovers, Tanks, Biped Mechs & Red/Blue Squadron Liveries
            prospectorRoverPrefab = LoadIfNull(prospectorRoverPrefab, "Assets/Mech_Constructor_Vehicles/Prefabs/X_Demo_Units/Bug_Rover.prefab");
            escortVehiclePrefab = LoadIfNull(escortVehiclePrefab, "Assets/Mech_Constructor_Vehicles/Prefabs/X_Demo_Units/Quad_Police.prefab");
            lineTankPrefab = LoadIfNull(lineTankPrefab, "Assets/Mech_Constructor_Spiders/Prefabs/Tanks/Chassis_Tank_Lvl2.prefab");
            heavyTankPrefab = LoadIfNull(heavyTankPrefab, "Assets/Mech_Constructor_Spiders/Prefabs/Tanks/Chassis_Tank_Lvl3.prefab");
            tankTurretSinglePrefab = LoadIfNull(tankTurretSinglePrefab, "Assets/Mech_Constructor_Spiders/Prefabs/Cockpit_Gun.prefab");
            tankTurretTwinPrefab = LoadIfNull(tankTurretTwinPrefab, "Assets/Mech_Constructor_Spiders/Prefabs/Cockpit_Gun_2x.prefab");
            bipedMechPrefab = LoadIfNull(bipedMechPrefab, "Assets/Mech_Constructor_LtMed/Prefabs/Legs_Med.prefab");
            bipedCockpitPrefab = LoadIfNull(bipedCockpitPrefab, "Assets/Mech_Constructor_LtMed/Prefabs/Cockpit_fox.prefab");
            bipedCannonPrefab = LoadIfNull(bipedCannonPrefab, "Assets/Mech_Constructor_LtMed/Prefabs/Weapon_cannon_lvl3.prefab");
            scoutWalkerPrefab = LoadIfNull(scoutWalkerPrefab, "Assets/Mech_Constructor_Spiders/Prefabs/Legs_Spider_Lt.prefab");

            if (spidersRedAlbedo == null)
                spidersRedAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Mech_Constructor_Spiders/Materials/MC_Spiders_Diffuse_Red.png");
            if (spidersBlueAlbedo == null)
                spidersBlueAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Mech_Constructor_Spiders/Materials/MC_Spiders_Diffuse_Blue.png");
            if (ltMedRedAlbedo == null)
                ltMedRedAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Mech_Constructor_LtMed/Materials/MC_LtMed_Diffuse_Red.png");
            if (ltMedBlueAlbedo == null)
                ltMedBlueAlbedo = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>("Assets/Mech_Constructor_LtMed/Materials/MC_LtMed_Diffuse_Blue.png");

            if (maleNpcCharacterPrefabs == null) maleNpcCharacterPrefabs = new List<GameObject>();
            if (maleNpcCharacterPrefabs.Count == 0)
            {
                string[] malePaths =
                {
                    "Assets/IdaFaber/Prefabs/Boy/SK_CYBER_SF_M_01 Variant.prefab",
                    "Assets/IdaFaber/Prefabs/Boy/SK_CYBER_SF_M_02 Variant.prefab",
                    "Assets/IdaFaber/Prefabs/Boy/SK_CYBER_SF_M_03 Variant.prefab",
                    "Assets/IdaFaber/Prefabs/Boy/SK_CYBER_SF_M_04 Variant.prefab",
                    "Assets/Ida Faber/Stellar Trooper/Prefabs/SK_M_TROOPER_01 Variant.prefab",
                    "Assets/Ida Faber/Stellar Trooper/Prefabs/SK_M_TROOPER_02 Variant.prefab",
                    "Assets/Ida Faber/Stellar Trooper/Prefabs/SK_M_TROOPER_03 Variant.prefab"
                };
                for (int i = 0; i < malePaths.Length; i++)
                {
                    var go = UnityEditor.AssetDatabase.LoadAssetAtPath<GameObject>(malePaths[i]);
                    if (IsAppropriateNpcPrefab(go) && !maleNpcCharacterPrefabs.Contains(go))
                        maleNpcCharacterPrefabs.Add(go);
                }
            }

            if (ambientWorkerModels == null) ambientWorkerModels = new List<GameObject>();
            if (ambientWorkerAlbedos == null) ambientWorkerAlbedos = new List<Texture>();
            if (ambientWorkerModels.Count == 0)
            {
                string[] gllowStems = { "Gllow_DockWorker_01", "Gllow_StationEngineer", "Gllow_ColonyGuard", "Gllow_CorpoFactor" };
                for (int i = 0; i < gllowStems.Length; i++)
                {
                    var mdl = UnityEditor.AssetDatabase.LoadAssetAtPath<GameObject>($"Assets/Gllow3D/{gllowStems[i]}.fbx");
                    var tex = UnityEditor.AssetDatabase.LoadAssetAtPath<Texture>($"Assets/Gllow3D/{gllowStems[i]}_Albedo.png");
                    if (mdl != null)
                    {
                        ambientWorkerModels.Add(mdl);
                        ambientWorkerAlbedos.Add(tex);
                    }
                }
            }
#endif
        }

#if UNITY_EDITOR
        private static GameObject LoadIfNull(GameObject current, string assetPath)
        {
            if (current != null) return current;
            return UnityEditor.AssetDatabase.LoadAssetAtPath<GameObject>(assetPath);
        }
#endif

        public static TacSimAssetCatalog LoadOrDefault()
        {
            var cat = Resources.Load<TacSimAssetCatalog>(ResourcePath);
            if (cat == null)
            {
                cat = CreateInstance<TacSimAssetCatalog>();
            }
            cat.EnsureEditorCatalogHydrated();
            return cat;
        }
    }
}
