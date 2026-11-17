using System;
using System.Collections.Generic;
using System.IO;
using TacSim.Core;
using UnityEditor;
using UnityEngine;

namespace TacSim.Editor
{
    /// <summary>
    /// One-click &amp; CLI importer for Vertical Slice 1 commercial 3D assets from C:\Users\faeni\3d_assets
    /// (kept strictly gitignored outside Assets/_TacSim/ per licensing rules):
    /// 1) Imports Ida Faber characters (Stellar Trooper, Cyberpunk Street Fighter, Techwear Girls, Astra, Celeste)
    /// 2) Imports Vattalus Fenris Frigate, DOTTEAM Heavy Station Kit, MSGDI Space Hulls, Solar System Planet Creator, FORGE3D Effects
    /// 3) Unpacks nested URP shader upgrade .unitypackages
    /// 4) Converts Built-in Standard PBR materials in Heavy Station Kit &amp; MSGDI to Universal Render Pipeline/Lit
    /// 5) Configures Humanoid rigs, builds TacSimHumanoid.controller, and populates Resources/TacSimAssetCatalog.asset
    /// </summary>
    public static class AssetPackImporter
    {
        public const string DefaultAssetVaultPath = @"C:\Users\faeni\3d_assets";
        public const string CatalogAssetPath = "Assets/_TacSim/Resources/TacSimAssetCatalog.asset";

        private static readonly string[] CuratedVerticalSlicePackages =
        {
            // Ida Faber Player & Station Cast Characters
            @"Ida Faber\3D ModelsCharactersHumanoidsSci-Fi\Stellar Trooper Sci-Fi Characters.unitypackage",
            @"Ida Faber\3D ModelsCharactersHumanoidsSci-Fi\Cyberpunk Street Fighter.unitypackage",
            @"Ida Faber\3D ModelsCharactersHumanoidsHumans\Techwear Girls.unitypackage",
            @"Ida Faber\3D ModelsCharactersHumanoidsSci-Fi\Stellar Girl Astra.unitypackage",
            @"Ida Faber\3D ModelsCharactersHumanoidsSci-Fi\Stellar Girl Celeste.unitypackage",

            // Walkable Ship Interior
            @"Vattalus Assets\3D ModelsVehiclesSpace\Spaceship with Full Interior Fenris Frigate.unitypackage",

            // Ceres Freeport & Phobos Stickney Station Architecture
            @"DOTTEAM\3D ModelsEnvironmentsSci-Fi\Sci-Fi Heavy Station Kit base AUGMENTED.unitypackage",
            @"DOTTEAM\3D ModelsEnvironmentsSci-Fi\Sci-Fi Heavy Station Kit hangars AUGMENTED.unitypackage",
            @"DOTTEAM\3D ModelsEnvironmentsSci-Fi\Sci-Fi Heavy Station Kit dugout AUGMENTED.unitypackage",

            // Orbital Space Hulls, Corvettes, Berths, Asteroids, Planets & Thruster VFX
            @"MSGDI\3D ModelsVehiclesSpace\Spaceship Freighters.unitypackage",
            @"MSGDI\3D ModelsVehiclesSpace\Spaceship Shipyards.unitypackage",
            @"MSGDI\3D ModelsVehiclesSpace\Spaceship Mining Operation.unitypackage",
            @"MSGDI\3D ModelsVehiclesSpace\Spaceship Pirate Fleet Collection I.unitypackage",
            @"Galactica Studio\3D ModelsEnvironmentsSci-Fi\Solar System Planet Creator.unitypackage",
            @"FORGE3D\Particle Systems\Sci-Fi Effects.unitypackage",

            // Slice 3 & OOC Red vs Blue Arena: Slava Z Rovers, Tanks & Biped Mechs
            @"Slava Z\3D ModelsCharactersRobots\Mech Constructor Spiders and Tanks.unitypackage",
            @"Slava Z\3D ModelsCharactersRobots\Mech Constructor Light and Medium Robots.unitypackage",
            @"Slava Z\3D ModelsVehiclesAir\Sci-Fi Vehicle Constructor.unitypackage"
        };

        private static readonly string[] NestedUrpUpgradePatterns =
        {
            "URP_StellarGirlAstra",
            "URP_StellarGirlCeleste",
            "VattalusAssets_FenrisFrigate_URP",
            "Sci-Fi Effects 2.3 URP"
        };

        [MenuItem("TacSim/1. Import Vertical Slice Assets (Ida Faber + Station + Ships)", priority = 10)]
        public static void ImportFromMenu()
        {
            string root = ResolveAssetVaultRoot();
            if (!Directory.Exists(root))
            {
                EditorUtility.DisplayDialog(
                    "TacSim Asset Importer",
                    $"Could not find 3D asset folder at:\n{root}\n\nSet TACSIM_3D_ASSETS environment variable or place packs in {DefaultAssetVaultPath}.",
                    "OK");
                return;
            }

            RunFullImportPipeline(root);
            EditorUtility.DisplayDialog(
                "TacSim Asset Import Complete",
                "Imported Vertical Slice 1 packs, applied nested URP shader packages, upgraded Standard PBR materials to URP/Lit, and populated TacSimAssetCatalog!\n\nNext: run 'TacSim -> 2. Build Vertical Slice Scene'.",
                "Awesome");
        }

        public static void ImportAllBatch()
        {
            string root = ResolveAssetVaultRoot();
            RunFullImportPipeline(root);
        }

        /// <summary>
        /// Batch post-processor invoked after .unitypackage assets are extracted into Assets/:
        /// upgrades Standard materials to URP/Lit, configures Ida Faber Humanoid rigs, builds
        /// TacSimHumanoid.controller, populates TacSimAssetCatalog, and rebuilds VerticalSlice1.unity.
        /// </summary>
        public static void FinalizeImportedAssetsAndRebuildScene()
        {
            SanitizeVendorScriptsForUrp();
            UrpBootstrap.EnsureUrpAssigned();
            int upgradedMats = UpgradeStandardMaterialsToUrpLit();
            RepairIdaFaberHairMaterials();
            EnsureIdaFaberHumanoidRigs();

            var controller = HumanoidAnimatorBuilder.BuildOrRefreshController();
            var catalog = PopulateOrRefreshCatalog(controller);
            VerticalSliceSceneBuilder.BuildVerticalSliceScene();

            Debug.Log(
                $"[TacSim Batch Finalize] Complete! Upgraded {upgradedMats} materials to URP/Lit | " +
                $"TrooperM={(catalog.stellarTrooperMale != null)} TrooperF={(catalog.stellarTrooperFemale != null)} " +
                $"CyberM={(catalog.cyberStreetFighterMale != null)} CyberF={(catalog.cyberStreetFighterFemale != null)} " +
                $"NPCs={catalog.npcCharacterPrefabs.Count} Fenris={(catalog.fenrisExteriorInteriorPrefab != null)} " +
                $"Dray={(catalog.drayExteriorHullPrefab != null)} Ceres={(catalog.ceresPlanetPrefab != null)}");
        }

        public static string ResolveAssetVaultRoot()
        {
            string env = Environment.GetEnvironmentVariable("TACSIM_3D_ASSETS");
            if (!string.IsNullOrWhiteSpace(env) && Directory.Exists(env))
                return env;
            return DefaultAssetVaultPath;
        }

        public static void RunFullImportPipeline(string vaultRoot)
        {
            UrpBootstrap.EnsureUrpAssigned();

            int importedCount = 0;
            foreach (string rel in CuratedVerticalSlicePackages)
            {
                string fullPath = Path.Combine(vaultRoot, rel);
                if (!File.Exists(fullPath))
                {
                    Debug.LogWarning($"[TacSim Importer] Skipping missing package: {fullPath}");
                    continue;
                }
                Debug.Log($"[TacSim Importer] Importing {rel}...");
                UnityEditor.AssetPackage.Package.Import(fullPath, interactive: false);
                importedCount++;
            }

            AssetDatabase.Refresh();

            // Unpack nested URP upgrade .unitypackages shipped inside Astra, Celeste, Fenris, and FORGE3D
            InstallNestedUrpSubPackages();
            SanitizeVendorScriptsForUrp();
            AssetDatabase.Refresh();

            // Convert Built-in Standard materials (DOTTEAM Heavy Station Kit, MSGDI Space Hulls) to URP/Lit
            int upgradedMats = UpgradeStandardMaterialsToUrpLit();
            RepairIdaFaberHairMaterials();
            EnsureIdaFaberHumanoidRigs();

            var controller = HumanoidAnimatorBuilder.BuildOrRefreshController();
            var catalog = PopulateOrRefreshCatalog(controller);

            Debug.Log($"[TacSim Importer] Finished! Imported {importedCount} packs, upgraded {upgradedMats} materials to URP/Lit, catalog ready ({catalog.npcCharacterPrefabs.Count} NPC prefabs).");
        }

        private static void SanitizeVendorScriptsForUrp()
        {
            // 1. Remove Ida Faber's optional demo auto-lightmapper (triggers unwanted GI bakes on every scene open & lacks #if UNITY_EDITOR)
            string autoBakePath = Path.Combine(Application.dataPath, "IdaFaber", "Demo", "S_BakeLight_CanBeDeleted.cs");
            if (File.Exists(autoBakePath))
            {
                File.Delete(autoBakePath);
                if (File.Exists(autoBakePath + ".meta")) File.Delete(autoBakePath + ".meta");
            }

            // 2. Guard Vattalus's legacy Built-in PostProcessing v2 demo camera script when com.unity.postprocessing is absent in URP
            string vattalusPp = Path.Combine(Application.dataPath, "VattalusAssets", "Common", "Scripts", "VattalusPostProcessController.cs");
            if (File.Exists(vattalusPp))
            {
                string src = File.ReadAllText(vattalusPp);
                if (!src.Contains("UNITY_POST_PROCESSING_STACK_V2"))
                {
                    File.WriteAllText(vattalusPp,
                        "using UnityEngine;\n#if UNITY_POST_PROCESSING_STACK_V2\nusing UnityEngine.Rendering.PostProcessing;\n#endif\n" +
                        "public class VattalusPostProcessController : MonoBehaviour {}\n");
                }
            }

            // 3. Deduplicate Slava Z global-namespace Roll_Wheels.cs shipped identically in both Spiders and Vehicles
            string dupRollWheels = Path.Combine(Application.dataPath, "Mech_Constructor_Vehicles", "Scripts", "Roll_Wheels.cs");
            if (File.Exists(dupRollWheels) && File.Exists(Path.Combine(Application.dataPath, "Mech_Constructor_Spiders", "Scripts", "Roll_Wheels.cs")))
            {
                File.Delete(dupRollWheels);
                if (File.Exists(dupRollWheels + ".meta")) File.Delete(dupRollWheels + ".meta");
            }
        }

        private static void InstallNestedUrpSubPackages()
        {
            string assetsDiskRoot = Application.dataPath;
            string[] nestedPackages = Directory.GetFiles(assetsDiskRoot, "*.unitypackage", SearchOption.AllDirectories);
            foreach (string pkgPath in nestedPackages)
            {
                string fileName = Path.GetFileNameWithoutExtension(pkgPath);
                foreach (string pattern in NestedUrpUpgradePatterns)
                {
                    if (fileName.IndexOf(pattern, StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        Debug.Log($"[TacSim Importer] Applying nested URP upgrade package: {fileName}");
                        UnityEditor.AssetPackage.Package.Import(pkgPath, interactive: false);
                        break;
                    }
                }
            }
        }

        [MenuItem("TacSim/Utilities/Upgrade All Standard Materials to URP Lit", priority = 40)]
        public static int UpgradeStandardMaterialsToUrpLit()
        {
            Shader urpLit = Shader.Find("Universal Render Pipeline/Lit");
            if (urpLit == null)
            {
                Debug.LogWarning("[TacSim Importer] Could not find Universal Render Pipeline/Lit shader.");
                return 0;
            }

            string[] matGuids = AssetDatabase.FindAssets("t:Material");
            int converted = 0;

            foreach (string guid in matGuids)
            {
                string path = AssetDatabase.GUIDToAssetPath(guid);
                if (string.IsNullOrEmpty(path) || path.StartsWith("Packages/", StringComparison.Ordinal))
                    continue;

                var mat = AssetDatabase.LoadAssetAtPath<Material>(path);
                if (mat == null || mat.shader == null) continue;

                string shaderName = mat.shader.name;
                bool isBuiltInStandard = shaderName == "Standard" ||
                                         shaderName == "Standard (Specular setup)" ||
                                         shaderName.StartsWith("Legacy Shaders/", StringComparison.Ordinal);
                if (!isBuiltInStandard) continue;

                Texture mainTex = mat.HasProperty("_MainTex") ? mat.GetTexture("_MainTex") : null;
                Vector2 mainScale = mat.HasProperty("_MainTex") ? mat.GetTextureScale("_MainTex") : Vector2.one;
                Vector2 mainOffset = mat.HasProperty("_MainTex") ? mat.GetTextureOffset("_MainTex") : Vector2.zero;
                Color baseColor = mat.HasProperty("_Color") ? mat.GetColor("_Color") : Color.white;
                Texture bumpMap = mat.HasProperty("_BumpMap") ? mat.GetTexture("_BumpMap") : null;
                float bumpScale = mat.HasProperty("_BumpScale") ? mat.GetFloat("_BumpScale") : 1f;
                Texture metalMap = mat.HasProperty("_MetallicGlossMap") ? mat.GetTexture("_MetallicGlossMap") : null;
                float metallic = mat.HasProperty("_Metallic") ? mat.GetFloat("_Metallic") : 0f;
                float smoothness = mat.HasProperty("_Glossiness") ? mat.GetFloat("_Glossiness") : 0.45f;
                Texture occlusion = mat.HasProperty("_OcclusionMap") ? mat.GetTexture("_OcclusionMap") : null;
                bool hadEmission = mat.IsKeywordEnabled("_EMISSION");
                Texture emissionMap = mat.HasProperty("_EmissionMap") ? mat.GetTexture("_EmissionMap") : null;
                Color emissionColor = mat.HasProperty("_EmissionColor") ? mat.GetColor("_EmissionColor") : Color.black;

                mat.shader = urpLit;
                if (mainTex != null && mat.HasProperty("_BaseMap"))
                {
                    mat.SetTexture("_BaseMap", mainTex);
                    mat.SetTextureScale("_BaseMap", mainScale);
                    mat.SetTextureOffset("_BaseMap", mainOffset);
                }
                if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", baseColor);
                if (bumpMap != null && mat.HasProperty("_BumpMap"))
                {
                    mat.SetTexture("_BumpMap", bumpMap);
                    mat.EnableKeyword("_NORMALMAP");
                    if (mat.HasProperty("_BumpScale")) mat.SetFloat("_BumpScale", bumpScale);
                }
                if (metalMap != null && mat.HasProperty("_MetallicGlossMap"))
                {
                    mat.SetTexture("_MetallicGlossMap", metalMap);
                    mat.EnableKeyword("_METALLICSPECGLOSSMAP");
                }
                if (mat.HasProperty("_Metallic")) mat.SetFloat("_Metallic", metallic);
                if (mat.HasProperty("_Smoothness")) mat.SetFloat("_Smoothness", smoothness);
                if (occlusion != null && mat.HasProperty("_OcclusionMap"))
                    mat.SetTexture("_OcclusionMap", occlusion);

                if (hadEmission || emissionMap != null || emissionColor.maxColorComponent > 0.01f)
                {
                    mat.EnableKeyword("_EMISSION");
                    if (emissionMap != null && mat.HasProperty("_EmissionMap"))
                        mat.SetTexture("_EmissionMap", emissionMap);
                    if (mat.HasProperty("_EmissionColor"))
                        mat.SetColor("_EmissionColor", emissionColor);
                }

                EditorUtility.SetDirty(mat);
                converted++;
            }

            if (converted > 0)
                AssetDatabase.SaveAssets();
            return converted;
        }

        private static void RepairIdaFaberHairMaterials()
        {
            string[] matGuids = AssetDatabase.FindAssets("t:Material");
            int repaired = 0;

            foreach (string guid in matGuids)
            {
                string path = AssetDatabase.GUIDToAssetPath(guid);
                if (string.IsNullOrEmpty(path) ||
                    (path.IndexOf("Ida Faber", StringComparison.OrdinalIgnoreCase) < 0 &&
                     path.IndexOf("IdaFaber", StringComparison.OrdinalIgnoreCase) < 0))
                    continue;

                var mat = AssetDatabase.LoadAssetAtPath<Material>(path);
                if (mat == null || mat.shader == null || !mat.HasProperty("_Basecolor_Map"))
                    continue;

                bool dirty = false;
                Texture cur = mat.GetTexture("_Basecolor_Map");
                if (cur == null || cur.name.IndexOf("Placeholder", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    var so = new SerializedObject(mat);
                    var texEnvs = so.FindProperty("m_SavedProperties.m_TexEnvs");
                    Texture replacement = null;
                    if (texEnvs != null && texEnvs.isArray)
                    {
                        for (int i = 0; i < texEnvs.arraySize; i++)
                        {
                            var elem = texEnvs.GetArrayElementAtIndex(i);
                            string key = elem.FindPropertyRelative("first")?.stringValue;
                            if (key == "_BaseColor_Map" || key == "_BaseColorMap" || key == "_BaseMap")
                            {
                                var texObj = elem.FindPropertyRelative("second.m_Texture")?.objectReferenceValue as Texture;
                                if (texObj != null && texObj.name.IndexOf("Placeholder", StringComparison.OrdinalIgnoreCase) < 0)
                                {
                                    replacement = texObj;
                                    break;
                                }
                            }
                        }
                    }

                    if (replacement != null)
                    {
                        mat.SetTexture("_Basecolor_Map", replacement);
                        dirty = true;
                    }
                }

                if (mat.name.IndexOf("HAIRHAT", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    mat.EnableKeyword("_ALPHATEST_ON");
                    if (mat.HasProperty("_AlphaClip")) mat.SetFloat("_AlphaClip", 1f);
                    if (mat.HasProperty("_AlphaCutoffEnable")) mat.SetFloat("_AlphaCutoffEnable", 1f);
                    if (mat.HasProperty("_Root_Color")) mat.SetColor("_Root_Color", new Color(0.11f, 0.09f, 0.08f, 1f));
                    dirty = true;
                }

                if (dirty)
                {
                    EditorUtility.SetDirty(mat);
                    repaired++;
                }
            }

            if (repaired > 0)
                AssetDatabase.SaveAssets();
        }

        private static void EnsureIdaFaberHumanoidRigs()
        {
            string[] modelGuids = AssetDatabase.FindAssets("t:Model");
            foreach (string guid in modelGuids)
            {
                string path = AssetDatabase.GUIDToAssetPath(guid);
                if (path.IndexOf("Ida Faber", StringComparison.OrdinalIgnoreCase) < 0 &&
                    path.IndexOf("IdaFaber", StringComparison.OrdinalIgnoreCase) < 0)
                    continue;

                var importer = AssetImporter.GetAtPath(path) as ModelImporter;
                if (importer != null && importer.animationType != ModelImporterAnimationType.Human)
                {
                    importer.animationType = ModelImporterAnimationType.Human;
                    importer.SaveAndReimport();
                }
            }
        }

        [InitializeOnLoadMethod]
        private static void AutoEnsureCatalogHydratedOnLoad()
        {
            EditorApplication.delayCall += () =>
            {
                if (EditorApplication.isPlayingOrWillChangePlaymode) return;
                SanitizeVendorScriptsForUrp();
                var catalog = AssetDatabase.LoadAssetAtPath<TacSimAssetCatalog>(CatalogAssetPath);
                if (catalog == null) return;
                if (catalog.stationDoorFrame == null ||
                    catalog.shipHelmSeatPrefab == null ||
                    catalog.shipyardCradlePrefab == null ||
                    catalog.lineTankPrefab == null ||
                    catalog.prospectorRoverPrefab == null ||
                    catalog.corvetteExteriorPrefab == null ||
                    catalog.maleNpcCharacterPrefabs == null ||
                    catalog.maleNpcCharacterPrefabs.Count == 0)
                {
                    UpgradeStandardMaterialsToUrpLit();
                    PopulateOrRefreshCatalog(catalog.humanoidAnimator);
                }
            };
        }

        public static TacSimAssetCatalog PopulateOrRefreshCatalog(RuntimeAnimatorController controller = null)
        {
            EnsureFolder("Assets/_TacSim", "Resources");
            var catalog = AssetDatabase.LoadAssetAtPath<TacSimAssetCatalog>(CatalogAssetPath);
            if (catalog == null)
            {
                catalog = ScriptableObject.CreateInstance<TacSimAssetCatalog>();
                AssetDatabase.CreateAsset(catalog, CatalogAssetPath);
            }

            if (controller == null)
                controller = AssetDatabase.LoadAssetAtPath<RuntimeAnimatorController>(HumanoidAnimatorBuilder.ControllerPath)
                             ?? HumanoidAnimatorBuilder.BuildOrRefreshController();
            catalog.humanoidAnimator = controller;

            // Player Avatars (Ida Faber Stellar Trooper & Cyberpunk Street Fighter)
            catalog.stellarTrooperMale = LoadFirstExistingPrefab(
                "Assets/Ida Faber/Stellar Trooper/Prefabs/SK_M_TROOPER_01 Variant.prefab",
                "SK_M_TROOPER");
            catalog.stellarTrooperFemale = LoadFirstExistingPrefab(
                "Assets/Ida Faber/Stellar Trooper/Prefabs/SK_F_TROOPER_01 Variant.prefab",
                "SK_F_TROOPER");
            catalog.cyberStreetFighterMale = LoadFirstExistingPrefab(
                "Assets/IdaFaber/Prefabs/Boy/SK_CYBER_SF_M_01 Variant.prefab",
                "SK_CYBER_SF_M");
            catalog.cyberStreetFighterFemale = LoadFirstExistingPrefab(
                "Assets/IdaFaber/Prefabs/Girl/SK_CYBER_SF_F_01 Variant.prefab",
                "SK_CYBER_SF_F");

            // Female NPC Cast (Techwear Girls, Stellar Girl Astra, Stellar Girl Celeste — excluding Lingerie variants)
            catalog.npcCharacterPrefabs.Clear();
            CollectMatchingPrefabs(catalog.npcCharacterPrefabs, "SK_TWG_");
            CollectMatchingPrefabs(catalog.npcCharacterPrefabs, "SK_StellarGirl_");
            CollectMatchingPrefabs(catalog.npcCharacterPrefabs, "SK_Celeste_");

            // Male NPC Cast (Cyberpunk SF Boys & Stellar Male Troopers — excluding Lingerie variants)
            catalog.maleNpcCharacterPrefabs.Clear();
            CollectMatchingPrefabs(catalog.maleNpcCharacterPrefabs, "SK_CYBER_SF_M_");
            CollectMatchingPrefabs(catalog.maleNpcCharacterPrefabs, "SK_M_TROOPER_");

            // DOTTEAM Heavy Station Kit (BASE + HANGARS)
            catalog.stationFloorTile = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Floors/Floor Base 1.prefab", "Floor Base 1");
            catalog.stationWallPanel = LoadFirstExistingPrefab("Assets/Heavy Station Kit/HANGARS/Prefabs/Walls/Wall_1.prefab", "Wall_1");
            catalog.stationWindowWall = LoadFirstExistingPrefab("Assets/Heavy Station Kit/HANGARS/Prefabs/Walls/Wall_2_Win.prefab", "Wall_2_Win");
            catalog.stationDoorFrame = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Arches/Arche A1.prefab", "Arche A1");
            catalog.stationLockedDoor = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Doors/B2_Door_Blocked.prefab", "B2_Door_Blocked");
            catalog.stationConsoleProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Equipment/B2_Cons.prefab", "B2_Cons");
            catalog.stationDisplayProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Displays/B2_Disp_TB1.prefab", "B2_Disp_TB1");
            catalog.stationPowerConsoleProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Equipment/B2_Cons_Power.prefab", "B2_Cons_Power");
            catalog.stationCrateProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/HANGARS/Prefabs/Props/hang_box1.prefab", "hang_box1");
            catalog.stationBarrelProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Props/Barrel1.prefab", "Barrel1");
            catalog.stationBatteryProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/HANGARS/Prefabs/Props/hang_Battery1.prefab", "hang_Battery1");
            catalog.stationPipeProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/BASE/Prefabs/Pipeline/B2_Pipe1.prefab", "B2_Pipe1");
            catalog.stationSupportProp = LoadFirstExistingPrefab("Assets/Heavy Station Kit/HANGARS/Prefabs/Supports/Support52.prefab", "Support52");

            // Vattalus Fenris Frigate Interior & URP Ship Props
            catalog.fenrisExteriorInteriorPrefab = LoadFirstExistingPrefab(
                "Assets/VattalusAssets/FenrisFrigate/_URP/Prefabs/Fenris_Exterior+Interior_Prefab.prefab",
                "Fenris_Exterior+Interior");
            catalog.shipAirlockDoorPrefab = LoadFirstExistingPrefab(
                "Assets/VattalusAssets/Common_URP/Prefabs/Doors/AirlockDoor_300cm_01_Skin1_Prefab.prefab",
                "AirlockDoor_300cm");
            catalog.shipBlastDoorPrefab = LoadFirstExistingPrefab(
                "Assets/VattalusAssets/Common_URP/Prefabs/Doors/BlastDoor_300cm_01_Skin1_Prefab.prefab",
                "BlastDoor_300cm");
            catalog.shipHelmSeatPrefab = LoadFirstExistingPrefab(
                "Assets/VattalusAssets/Common_URP/Prefabs/Seats/VA_Seat01_Red_Prefab.prefab",
                "VA_Seat01");
            catalog.shipVentMistPrefab = LoadFirstExistingPrefab(
                "Assets/VattalusAssets/Common/Particles/VentMistParticlesM.prefab",
                "VentMistParticlesM");

            // MSGDI Hulls, Shipyards, Drones, Asteroids + Planet + Thruster Plumes
            catalog.drayExteriorHullPrefab = LoadFirstExistingPrefab(
                "Assets/Freighters/Prefabs/FreighterClassII/Grey/FreighterClassIIGrey.prefab",
                "FreighterClassII");
            catalog.visitingFreighterPrefab = LoadFirstExistingPrefab(
                "Assets/Freighters/Prefabs/FreighterClassIII/Yellow/FreighterClassIIIYellow.prefab",
                "FreighterClassIII");
            catalog.shipyardCradlePrefab = LoadFirstExistingPrefab(
                "Assets/Shipyards/Prefabs/FrigateShipyard/FrigateShipyardGrey.prefab",
                "FrigateShipyard");
            catalog.collectorDronePrefab = LoadFirstExistingPrefab(
                "Assets/MiningOperation/Prefabs/ResourceCollector/ResourceCollectorYellow.prefab",
                "ResourceCollector");
            catalog.ceresPlanetPrefab = LoadFirstExistingPrefab("", "Planet2");
            catalog.thrusterPlumePrefab = LoadFirstExistingPrefab(
                "Assets/VattalusAssets/Common/ThrusterEffects/ThrusterParticles_MainL.prefab",
                "flames_flame_blue");

            catalog.asteroidPrefabs.Clear();
            CollectMatchingPrefabs(catalog.asteroidPrefabs, "Asteroid", maxCount: 6);

            // Staged Gllow3D Ambient Static Dockworkers (when present in gitignored Assets/Gllow3D/)
            catalog.ambientWorkerModels.Clear();
            catalog.ambientWorkerAlbedos.Clear();
            string[] gllowStems = { "Gllow_DockWorker_01", "Gllow_StationEngineer", "Gllow_ColonyGuard", "Gllow_CorpoFactor" };
            for (int i = 0; i < gllowStems.Length; i++)
            {
                var mdl = AssetDatabase.LoadAssetAtPath<GameObject>($"Assets/Gllow3D/{gllowStems[i]}.fbx");
                var tex = AssetDatabase.LoadAssetAtPath<Texture>($"Assets/Gllow3D/{gllowStems[i]}_Albedo.png");
                if (mdl != null)
                {
                    catalog.ambientWorkerModels.Add(mdl);
                    catalog.ambientWorkerAlbedos.Add(tex);
                }
            }

            catalog.EnsureEditorCatalogHydrated();
            EditorUtility.SetDirty(catalog);
            AssetDatabase.SaveAssets();
            return catalog;
        }

        private static GameObject LoadFirstExistingPrefab(string exactPath, string searchSubstring)
        {
            if (!string.IsNullOrEmpty(exactPath))
            {
                var direct = AssetDatabase.LoadAssetAtPath<GameObject>(exactPath);
                if (direct != null) return direct;
            }
            if (string.IsNullOrEmpty(searchSubstring)) return null;
            string[] guids = AssetDatabase.FindAssets($"{searchSubstring} t:Prefab");
            foreach (string guid in guids)
            {
                string p = AssetDatabase.GUIDToAssetPath(guid);
                var go = AssetDatabase.LoadAssetAtPath<GameObject>(p);
                if (go != null && TacSimAssetCatalog.IsAppropriateNpcPrefab(go)) return go;
            }
            return null;
        }

        private static void CollectMatchingPrefabs(List<GameObject> target, string substring, int maxCount = 20)
        {
            string[] guids = AssetDatabase.FindAssets($"{substring} t:Prefab");
            foreach (string guid in guids)
            {
                if (target.Count >= maxCount) break;
                string path = AssetDatabase.GUIDToAssetPath(guid);
                var prefab = AssetDatabase.LoadAssetAtPath<GameObject>(path);
                if (TacSimAssetCatalog.IsAppropriateNpcPrefab(prefab) && !target.Contains(prefab))
                    target.Add(prefab);
            }
        }

        private static void EnsureFolder(string parent, string child)
        {
            string full = $"{parent}/{child}";
            if (!AssetDatabase.IsValidFolder(full))
                AssetDatabase.CreateFolder(parent, child);
        }
    }
}
