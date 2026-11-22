using System;
using System.Collections.Generic;
using System.IO;
using TacSim.World;
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;
using UnityEngine;

namespace TacSim.Editor
{
    /// <summary>
    /// One-click menu &amp; CLI standalone player packager for Windows x64, Linux x64, and macOS Universal.
    /// Also implements <see cref="IPreprocessBuildWithReport"/> so ANY Unity build (menu, Build Profiles UI,
    /// or headless CI) automatically stages sanitized client-safe fallback packs into
    /// <c>Assets/_TacSim/Resources/TacSimFallbackPacks/</c> — eliminating cold-cache graybox fallbacks on
    /// machines without a local Rust repository checkout while stripping secrets, hidden exits, and NPC prompts.
    /// </summary>
    public sealed class TacSimStandaloneBuilder : IPreprocessBuildWithReport
    {
        public const string FallbackResourcesDir = "Assets/_TacSim/Resources/TacSimFallbackPacks";
        private static bool _stagedDuringExecuteBuild;

        public int callbackOrder => 0;

        public void OnPreprocessBuild(BuildReport report)
        {
            if (_stagedDuringExecuteBuild)
                return;
            // Never invoke AssetDatabase.Refresh(ForceSynchronousImport) inside IPreprocessBuildWithReport.OnPreprocessBuild.
            int count = StageEmbeddedFallbackPacks(explicitOutputDir: null, refreshAssetDatabase: false);
            Debug.Log($"[TacSim Build Preprocess] Staged {count} sanitized fallback content packs into {FallbackResourcesDir}.");
        }

        [MenuItem("TacSim/3. Build Standalone Player (Windows x64)", priority = 30)]
        public static void BuildWindows64Menu()
        {
            string defaultDir = Path.GetFullPath(Path.Combine(Application.dataPath, "..", "Builds", "Windows64"));
            Directory.CreateDirectory(defaultDir);
            string chosen = EditorUtility.SaveFilePanel(
                "Build RustMUSH TacSim 3D Client (Windows x64)",
                defaultDir,
                "RustMUSH_TacSim",
                "exe");
            if (string.IsNullOrWhiteSpace(chosen))
                return;

            BuildReport report = ExecuteStandaloneBuild(BuildTarget.StandaloneWindows64, chosen);
            if (report != null && report.summary.result == BuildResult.Succeeded)
            {
                EditorUtility.RevealInFinder(chosen);
                EditorUtility.DisplayDialog(
                    "TacSim Standalone Build Complete",
                    $"Built standalone player:\n{chosen}\n\nTip: launch with `--server https://dev1.conclav.io` or enter your server URL on the login screen.",
                    "Awesome");
            }
            else
            {
                EditorUtility.DisplayDialog(
                    "TacSim Standalone Build Failed",
                    $"Build ended with result: {(report != null ? report.summary.result.ToString() : "Unknown")}. Check the Unity Console for details.",
                    "OK");
            }
        }

        [MenuItem("TacSim/Utilities/Stage Embedded Fallback Packs", priority = 45)]
        public static int StageEmbeddedFallbackPacksMenu()
        {
            int staged = StageEmbeddedFallbackPacks();
            Debug.Log($"[TacSim] Staged {staged} sanitized fallback content packs into {FallbackResourcesDir}.");
            return staged;
        }

        [MenuItem("TacSim/Utilities/Optimize Pack Textures for Standalone", priority = 46)]
        public static int OptimizePackTexturesMenu()
        {
            int updated = OptimizePackTextureImportSettings();
            Debug.Log($"[TacSim] Optimized {updated} third-party pack texture importer(s) for standalone builds.");
            return updated;
        }

        /// <summary>
        /// Computes the standalone max texture resolution for a third-party asset pack texture,
        /// or returns 0 for first-party <c>Assets/_TacSim/</c> assets (never resized).
        /// </summary>
        public static int ResolveStandalonePackMaxTextureSize(string assetPath)
        {
            if (string.IsNullOrWhiteSpace(assetPath))
                return 0;
            string norm = assetPath.Replace('\\', '/');
            if (!norm.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase) ||
                norm.StartsWith("Assets/_TacSim/", StringComparison.OrdinalIgnoreCase))
                return 0;

            string file = Path.GetFileName(norm).ToLowerInvariant();
            if (file.Contains("teeth") ||
                file.Contains("eye") ||
                file.Contains("lash") ||
                file.Contains("brow") ||
                file.Contains("cornea") ||
                file.Contains("tongue") ||
                file.Contains("mouth") ||
                file.Contains("wind") ||
                file.Contains("specular") ||
                file.Contains("roughness") ||
                file.Contains("metallic") ||
                file.Contains("smoothness") ||
                file.Contains("maskmap") ||
                file.Contains("occlusion") ||
                file.Contains("_ao") ||
                file.Contains("alpha") ||
                file.Contains("emissive") ||
                file.Contains("emission"))
            {
                return 512;
            }

            return 1024;
        }

        /// <summary>
        /// Ensures every third-party texture actually referenced by the build scene or asset catalog
        /// uses GPU-compressed formats (DXT/BC7) capped at 1024 px (512 px for secondary/detail maps)
        /// instead of 4K uncompressed RGBA32 textures (85 MB each). Subsequent runs are near-instant no-ops.
        /// </summary>
        public static int OptimizePackTextureImportSettings()
        {
#if UNITY_5_3_OR_NEWER
            string[] roots =
            {
                VerticalSliceSceneBuilder.ScenePath,
                AssetPackImporter.CatalogAssetPath
            };
            string[] deps = AssetDatabase.GetDependencies(roots, true);
            var toReimport = new System.Collections.Generic.List<string>();

            AssetDatabase.StartAssetEditing();
            try
            {
                for (int i = 0; i < deps.Length; i++)
                {
                    string dep = deps[i];
                    int desiredMax = ResolveStandalonePackMaxTextureSize(dep);
                    if (desiredMax <= 0)
                        continue;

                    var importer = AssetImporter.GetAtPath(dep) as TextureImporter;
                    if (importer == null)
                        continue;

                    bool changed = false;
                    if (importer.maxTextureSize > desiredMax)
                    {
                        importer.maxTextureSize = desiredMax;
                        changed = true;
                    }
                    if (importer.textureCompression != TextureImporterCompression.Compressed)
                    {
                        importer.textureCompression = TextureImporterCompression.Compressed;
                        changed = true;
                    }

                    TextureImporterPlatformSettings standalone = importer.GetPlatformTextureSettings("Standalone");
                    if (standalone.overridden)
                    {
                        bool platChanged = false;
                        if (standalone.maxTextureSize > desiredMax)
                        {
                            standalone.maxTextureSize = desiredMax;
                            platChanged = true;
                        }
                        if (standalone.textureCompression != TextureImporterCompression.Compressed)
                        {
                            standalone.textureCompression = TextureImporterCompression.Compressed;
                            platChanged = true;
                        }
                        if (standalone.format == TextureImporterFormat.RGBA32 ||
                            standalone.format == TextureImporterFormat.RGB24 ||
                            standalone.format == TextureImporterFormat.ARGB32)
                        {
                            standalone.format = TextureImporterFormat.Automatic;
                            platChanged = true;
                        }
                        if (platChanged)
                        {
                            importer.SetPlatformTextureSettings(standalone);
                            changed = true;
                        }
                    }

                    if (changed)
                    {
                        importer.SaveAndReimport();
                        toReimport.Add(dep);
                    }
                }
            }
            finally
            {
                AssetDatabase.StopAssetEditing();
            }

            if (toReimport.Count > 0)
            {
                AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport);
                Debug.Log($"[TacSim Builder] Compressed & capped {toReimport.Count} third-party build texture(s) to <= 1024/512 px.");
            }
            return toReimport.Count;
#else
            return 0;
#endif
        }

        public static void BuildWindows64Batch()
        {
            string outPath = ResolveBatchOutputPath("Windows64", "RustMUSH_TacSim.exe");
            RequireSucceeded(ExecuteStandaloneBuild(BuildTarget.StandaloneWindows64, outPath), outPath);
        }

        public static void BuildLinux64Batch()
        {
            string outPath = ResolveBatchOutputPath("Linux64", "RustMUSH_TacSim.x86_64");
            RequireSucceeded(ExecuteStandaloneBuild(BuildTarget.StandaloneLinux64, outPath), outPath);
        }

        public static void BuildMacOSBatch()
        {
            string outPath = ResolveBatchOutputPath("macOS", "RustMUSH_TacSim.app");
            RequireSucceeded(ExecuteStandaloneBuild(BuildTarget.StandaloneOSX, outPath), outPath);
        }

        public static BuildReport ExecuteStandaloneBuild(BuildTarget target, string outputFilePath)
        {
            UrpBootstrap.EnsureUrpAssigned();
            AssetPackImporter.EnsureIdaFaberHumanoidRigs();
            AssetPackImporter.PopulateOrRefreshCatalog();
            StageEmbeddedFallbackPacks();
            VerticalSliceSceneBuilder.BuildVerticalSliceScene();
            OptimizePackTextureImportSettings();

            EditorBuildSettings.scenes = new[]
            {
                new EditorBuildSettingsScene(VerticalSliceSceneBuilder.ScenePath, true)
            };

            string parentDir = Path.GetDirectoryName(outputFilePath);
            if (!string.IsNullOrEmpty(parentDir))
                Directory.CreateDirectory(parentDir);

            var opts = new BuildPlayerOptions
            {
                scenes = new[] { VerticalSliceSceneBuilder.ScenePath },
                locationPathName = outputFilePath,
                target = target,
                options = BuildOptions.CompressWithLz4
            };

            Debug.Log($"[TacSim Builder] Building {target} standalone player -> {outputFilePath}...");
            _stagedDuringExecuteBuild = true;
            BuildReport report;
            try
            {
                report = BuildPipeline.BuildPlayer(opts);
            }
            finally
            {
                _stagedDuringExecuteBuild = false;
            }
            if (report != null)
                Debug.Log($"[TacSim Builder] Build finished: result={report.summary.result} errors={report.summary.totalErrors} warnings={report.summary.totalWarnings}");
            else
                Debug.LogError("[TacSim Builder] BuildPipeline.BuildPlayer returned null BuildReport.");
            return report;
        }

        public static int StageEmbeddedFallbackPacks()
        {
            return StageEmbeddedFallbackPacks(explicitOutputDir: null, refreshAssetDatabase: true);
        }

        public static int StageEmbeddedFallbackPacks(string explicitOutputDir, bool refreshAssetDatabase)
        {
            return StageEmbeddedFallbackPacksFromRoots(
                explicitOutputDir,
                tacsimContentRoot: null,
                mushAtlasContentRoot: null,
                refreshAssetDatabase: refreshAssetDatabase,
                deleteFile: null);
        }

        public static int StageEmbeddedFallbackPacksFromRoots(
            string explicitOutputDir,
            string tacsimContentRoot,
            string mushAtlasContentRoot,
            bool refreshAssetDatabase = false,
            Action<string> deleteFile = null)
        {
            string diskOutDir = !string.IsNullOrWhiteSpace(explicitOutputDir)
                ? Path.GetFullPath(explicitOutputDir)
                : Path.GetFullPath(Path.Combine(Application.dataPath, "_TacSim", "Resources", "TacSimFallbackPacks"));

            string stagingDir = Path.Combine(Path.GetTempPath(), "TacSimFallbackStaging_" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(stagingDir);

            try
            {
                string ResolveTacsim(string rel) => tacsimContentRoot != null
                    ? Path.Combine(tacsimContentRoot, rel)
                    : TacSimContentLoader.ResolveRepoContentPath(rel);

                string ResolveMushAtlas(string rel) => mushAtlasContentRoot != null
                    ? Path.Combine(mushAtlasContentRoot, rel)
                    : TacSimContentLoader.ResolveMushAtlasContentPath(rel);

                var stagedFiles = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                int written = 0;
                int neighborhoodsWritten = 0;
                int castWritten = 0;
                int unitsWritten = 0;
                int arenasWritten = 0;
                int goodsWritten = 0;

                // Phase 1: Stage and sanitize the complete replacement set into a clean temporary folder
                // BEFORE deleting or overwriting anything in `diskOutDir`.

                // 1. Corridor port & OOC neighborhood packs + planetary surface regions & arena atlases
                string oocSrc = ResolveMushAtlas("neighborhood.ooc_area.json");
                if (TryStageSanitizedFile(oocSrc, Path.Combine(stagingDir, "neighborhood.ooc_area.json"), checkAccessibility: true))
                {
                    stagedFiles.Add("neighborhood.ooc_area.json");
                    written++;
                    neighborhoodsWritten++;
                }

                string atlasDir = ResolveTacsim("atlas");
                if (Directory.Exists(atlasDir))
                {
                    string[] neighborhoodFiles = Directory.GetFiles(atlasDir, "neighborhood.*.json");
                    for (int i = 0; i < neighborhoodFiles.Length; i++)
                    {
                        string fileName = Path.GetFileName(neighborhoodFiles[i]);
                        if (fileName.IndexOf("staff", StringComparison.OrdinalIgnoreCase) >= 0)
                            continue;
                        if (TryStageSanitizedFile(neighborhoodFiles[i], Path.Combine(stagingDir, fileName), checkAccessibility: true))
                        {
                            stagedFiles.Add(fileName);
                            written++;
                            neighborhoodsWritten++;
                        }
                    }

                    string[] regionFiles = Directory.GetFiles(atlasDir, "region.*.json");
                    for (int i = 0; i < regionFiles.Length; i++)
                    {
                        string fileName = Path.GetFileName(regionFiles[i]);
                        if (TryStageSanitizedFile(regionFiles[i], Path.Combine(stagingDir, fileName), checkAccessibility: false))
                        {
                            stagedFiles.Add(fileName);
                            written++;
                        }
                    }

                    string[] arenaAtlasFiles = Directory.GetFiles(atlasDir, "arena_atlas.*.json");
                    for (int i = 0; i < arenaAtlasFiles.Length; i++)
                    {
                        string fileName = Path.GetFileName(arenaAtlasFiles[i]);
                        if (TryStageSanitizedFile(arenaAtlasFiles[i], Path.Combine(stagingDir, fileName), checkAccessibility: false))
                        {
                            stagedFiles.Add(fileName);
                            written++;
                        }
                    }
                }
                else
                {
                    Debug.LogWarning($"[TacSim Builder] Atlas content directory not found at '{atlasDir}'.");
                }

                // 2. Cast packs (`cast.<slug>.json`)
                string castDir = ResolveTacsim("cast");
                if (Directory.Exists(castDir))
                {
                    string[] castFiles = Directory.GetFiles(castDir, "*.json");
                    for (int i = 0; i < castFiles.Length; i++)
                    {
                        string slug = Path.GetFileNameWithoutExtension(castFiles[i]);
                        if (slug.IndexOf("staff", StringComparison.OrdinalIgnoreCase) >= 0)
                            continue;
                        string dstName = $"cast.{slug}.json";
                        if (TryStageSanitizedFile(castFiles[i], Path.Combine(stagingDir, dstName), checkAccessibility: false))
                        {
                            stagedFiles.Add(dstName);
                            written++;
                            castWritten++;
                        }
                    }
                }
                else
                {
                    Debug.LogWarning($"[TacSim Builder] Cast content directory not found at '{castDir}'.");
                }

                // 3. Walkable ship hull & vehicle unit templates (`dray.json`, `dray_p.json`, `corvette.json`, etc.)
                string unitsDir = ResolveTacsim("units");
                if (Directory.Exists(unitsDir))
                {
                    string[] unitFiles = Directory.GetFiles(unitsDir, "*.json");
                    for (int i = 0; i < unitFiles.Length; i++)
                    {
                        string fileName = Path.GetFileName(unitFiles[i]);
                        if (TryStageSanitizedFile(unitFiles[i], Path.Combine(stagingDir, fileName), checkAccessibility: false))
                        {
                            stagedFiles.Add(fileName);
                            written++;
                            unitsWritten++;
                        }
                    }
                }
                else
                {
                    Debug.LogWarning($"[TacSim Builder] Units content directory not found at '{unitsDir}'.");
                }

                // 4. Arena battle maps (`arenas/<slug>.json` -> `arena.<slug>.json` matching TacSimTerrainDecoder)
                string arenasDir = ResolveTacsim("arenas");
                if (Directory.Exists(arenasDir))
                {
                    string[] arenaFiles = Directory.GetFiles(arenasDir, "*.json");
                    for (int i = 0; i < arenaFiles.Length; i++)
                    {
                        string slug = Path.GetFileNameWithoutExtension(arenaFiles[i]);
                        string dstName = slug.StartsWith("arena.", StringComparison.OrdinalIgnoreCase)
                            ? $"{slug}.json"
                            : $"arena.{slug}.json";
                        if (TryStageSanitizedFile(arenaFiles[i], Path.Combine(stagingDir, dstName), checkAccessibility: false))
                        {
                            stagedFiles.Add(dstName);
                            written++;
                            arenasWritten++;
                        }
                    }
                }

                // 5. Canonical market goods catalog (`goods.json`)
                string goodsSrc = ResolveTacsim(Path.Combine("market", "goods.json"));
                if (TryStageSanitizedFile(goodsSrc, Path.Combine(stagingDir, "goods.json"), checkAccessibility: false))
                {
                    stagedFiles.Add("goods.json");
                    written++;
                    goodsWritten++;
                }

                // Phase 2: Validate complete replacement set BEFORE mutating `diskOutDir`.
                var missing = new List<string>();
                if (written == 0) missing.Add("any fallback pack");
                if (neighborhoodsWritten == 0) missing.Add("neighborhoods");
                if (!stagedFiles.Contains("neighborhood.ceres_freeport.json")) missing.Add("neighborhood.ceres_freeport.json");
                if (!stagedFiles.Contains("neighborhood.ooc_area.json")) missing.Add("neighborhood.ooc_area.json");
                if (castWritten == 0) missing.Add("cast packs");
                if (unitsWritten == 0 || !stagedFiles.Contains("dray.json")) missing.Add("units/dray.json");
                if (arenasWritten == 0) missing.Add("arenas");
                if (goodsWritten == 0 || !stagedFiles.Contains("goods.json")) missing.Add("market/goods.json");

                if (missing.Count > 0)
                {
                    string err =
                        $"[TacSim Builder] Fallback staging incomplete (missing: {string.Join(", ", missing)}; " +
                        $"total={written}, neighborhoods={neighborhoodsWritten}, cast={castWritten}, units={unitsWritten}, arenas={arenasWritten}, goods={goodsWritten}). " +
                        $"Verify crates/tacsim/content is reachable from '{Directory.GetCurrentDirectory()}'.";
                    Debug.LogError(err);
                    throw new BuildFailedException(err);
                }

                // Phase 3: Remove retired packs cleanly (failing the build if any stale file cannot be deleted),
                // then commit the validated replacement set into `diskOutDir`.
                Directory.CreateDirectory(diskOutDir);
                Action<string> deleter = deleteFile ?? File.Delete;

                string[] existingStaged = Directory.GetFiles(diskOutDir, "*.json");
                for (int i = 0; i < existingStaged.Length; i++)
                {
                    string existingPath = existingStaged[i];
                    string existingName = Path.GetFileName(existingPath);
                    if (stagedFiles.Contains(existingName))
                        continue;

                    try
                    {
                        deleter(existingPath);
                    }
                    catch (Exception ex)
                    {
                        string err = $"[TacSim Builder] Failed deleting retired fallback pack '{existingPath}': {ex.Message}";
                        Debug.LogError(err);
                        throw new BuildFailedException(err);
                    }

                    if (File.Exists(existingPath))
                    {
                        string err = $"[TacSim Builder] Retired fallback pack '{existingPath}' still exists after deletion.";
                        Debug.LogError(err);
                        throw new BuildFailedException(err);
                    }

                    string metaPath = existingPath + ".meta";
                    if (File.Exists(metaPath))
                    {
                        try
                        {
                            deleter(metaPath);
                        }
                        catch (Exception ex)
                        {
                            string err = $"[TacSim Builder] Failed deleting retired fallback pack metadata '{metaPath}': {ex.Message}";
                            Debug.LogError(err);
                            throw new BuildFailedException(err);
                        }
                    }
                }

                foreach (string fileName in stagedFiles)
                {
                    string src = Path.Combine(stagingDir, fileName);
                    string dst = Path.Combine(diskOutDir, fileName);
                    try
                    {
                        File.Copy(src, dst, overwrite: true);
                    }
                    catch (Exception ex)
                    {
                        string err = $"[TacSim Builder] Failed publishing fallback pack '{dst}': {ex.Message}";
                        Debug.LogError(err);
                        throw new BuildFailedException(err);
                    }
                }

                // Post-commit verification: ensure no retired JSON packs remain and every required pack was published.
                string[] published = Directory.GetFiles(diskOutDir, "*.json");
                if (published.Length != stagedFiles.Count)
                {
                    string err = $"[TacSim Builder] Fallback output verification failed: expected {stagedFiles.Count} packs in '{diskOutDir}', found {published.Length}.";
                    Debug.LogError(err);
                    throw new BuildFailedException(err);
                }
                for (int i = 0; i < published.Length; i++)
                {
                    string pubName = Path.GetFileName(published[i]);
                    if (!stagedFiles.Contains(pubName))
                    {
                        string err = $"[TacSim Builder] Unexpected retired fallback pack '{published[i]}' remained after staging.";
                        Debug.LogError(err);
                        throw new BuildFailedException(err);
                    }
                }

                if (refreshAssetDatabase)
                    AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport);
                return written;
            }
            finally
            {
                try
                {
                    if (Directory.Exists(stagingDir))
                        Directory.Delete(stagingDir, recursive: true);
                }
                catch
                {
                    /* best-effort temp staging cleanup */
                }
            }
        }

        private static bool TryStageSanitizedFile(string srcPath, string dstPath, bool checkAccessibility)
        {
            if (string.IsNullOrEmpty(srcPath) || !File.Exists(srcPath))
                return false;
            string raw = File.ReadAllText(srcPath);
            if (checkAccessibility && !TacSimContentLoader.IsNeighborhoodPackPlayerAccessible(raw))
                return false;
            string sanitized = TacSimContentLoader.SanitizePackJsonForClient(raw);
            File.WriteAllText(dstPath, sanitized);
            return true;
        }

        private static string ResolveBatchOutputPath(string folderName, string exeName)
        {
            string envOut = Environment.GetEnvironmentVariable("TACSIM_BUILD_OUTPUT");
            if (!string.IsNullOrWhiteSpace(envOut))
                return Path.GetFullPath(envOut.Trim());
            return Path.GetFullPath(Path.Combine(Application.dataPath, "..", "Builds", folderName, exeName));
        }

        private static void RequireSucceeded(BuildReport report, string outPath)
        {
            if (report == null || report.summary.result != BuildResult.Succeeded)
                throw new InvalidOperationException($"Standalone build to '{outPath}' failed with status '{(report != null ? report.summary.result.ToString() : "null")}'.");
        }
    }
}
