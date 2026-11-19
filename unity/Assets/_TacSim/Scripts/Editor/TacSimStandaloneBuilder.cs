using System;
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
            StageEmbeddedFallbackPacks();
            VerticalSliceSceneBuilder.BuildVerticalSliceScene();

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
            string diskOutDir = !string.IsNullOrWhiteSpace(explicitOutputDir)
                ? Path.GetFullPath(explicitOutputDir)
                : Path.GetFullPath(Path.Combine(Application.dataPath, "_TacSim", "Resources", "TacSimFallbackPacks"));
            Directory.CreateDirectory(diskOutDir);

            // Clean stale staged JSON packs so removed/renamed content files do not linger across builds.
            string[] existingStaged = Directory.GetFiles(diskOutDir, "*.json");
            for (int i = 0; i < existingStaged.Length; i++)
            {
                try { File.Delete(existingStaged[i]); } catch { /* ignore */ }
            }

            int written = 0;
            int neighborhoodsWritten = 0;
            int unitsWritten = 0;

            // 1. Corridor port & OOC neighborhood packs + planetary surface regions & arena atlases
            string oocSrc = TacSimContentLoader.ResolveMushAtlasContentPath("neighborhood.ooc_area.json");
            if (TryStageSanitizedFile(oocSrc, Path.Combine(diskOutDir, "neighborhood.ooc_area.json"), checkAccessibility: true))
            {
                written++;
                neighborhoodsWritten++;
            }

            string atlasDir = TacSimContentLoader.ResolveRepoContentPath("atlas");
            if (Directory.Exists(atlasDir))
            {
                string[] neighborhoodFiles = Directory.GetFiles(atlasDir, "neighborhood.*.json");
                for (int i = 0; i < neighborhoodFiles.Length; i++)
                {
                    string fileName = Path.GetFileName(neighborhoodFiles[i]);
                    if (fileName.IndexOf("staff", StringComparison.OrdinalIgnoreCase) >= 0)
                        continue;
                    if (TryStageSanitizedFile(neighborhoodFiles[i], Path.Combine(diskOutDir, fileName), checkAccessibility: true))
                    {
                        written++;
                        neighborhoodsWritten++;
                    }
                }

                string[] regionFiles = Directory.GetFiles(atlasDir, "region.*.json");
                for (int i = 0; i < regionFiles.Length; i++)
                {
                    string fileName = Path.GetFileName(regionFiles[i]);
                    if (TryStageSanitizedFile(regionFiles[i], Path.Combine(diskOutDir, fileName), checkAccessibility: false))
                        written++;
                }

                string[] arenaAtlasFiles = Directory.GetFiles(atlasDir, "arena_atlas.*.json");
                for (int i = 0; i < arenaAtlasFiles.Length; i++)
                {
                    string fileName = Path.GetFileName(arenaAtlasFiles[i]);
                    if (TryStageSanitizedFile(arenaAtlasFiles[i], Path.Combine(diskOutDir, fileName), checkAccessibility: false))
                        written++;
                }
            }
            else
            {
                Debug.LogWarning($"[TacSim Builder] Atlas content directory not found at '{atlasDir}'.");
            }

            // 2. Cast packs (`cast.<slug>.json`)
            string castDir = TacSimContentLoader.ResolveRepoContentPath("cast");
            if (Directory.Exists(castDir))
            {
                string[] castFiles = Directory.GetFiles(castDir, "*.json");
                for (int i = 0; i < castFiles.Length; i++)
                {
                    string slug = Path.GetFileNameWithoutExtension(castFiles[i]);
                    if (slug.IndexOf("staff", StringComparison.OrdinalIgnoreCase) >= 0)
                        continue;
                    if (TryStageSanitizedFile(castFiles[i], Path.Combine(diskOutDir, $"cast.{slug}.json"), checkAccessibility: false))
                        written++;
                }
            }
            else
            {
                Debug.LogWarning($"[TacSim Builder] Cast content directory not found at '{castDir}'.");
            }

            // 3. Walkable ship hull & vehicle unit templates (`dray.json`, `dray_p.json`, `corvette.json`, etc.)
            string unitsDir = TacSimContentLoader.ResolveRepoContentPath("units");
            if (Directory.Exists(unitsDir))
            {
                string[] unitFiles = Directory.GetFiles(unitsDir, "*.json");
                for (int i = 0; i < unitFiles.Length; i++)
                {
                    string fileName = Path.GetFileName(unitFiles[i]);
                    if (TryStageSanitizedFile(unitFiles[i], Path.Combine(diskOutDir, fileName), checkAccessibility: false))
                    {
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
            string arenasDir = TacSimContentLoader.ResolveRepoContentPath("arenas");
            if (Directory.Exists(arenasDir))
            {
                string[] arenaFiles = Directory.GetFiles(arenasDir, "*.json");
                for (int i = 0; i < arenaFiles.Length; i++)
                {
                    string slug = Path.GetFileNameWithoutExtension(arenaFiles[i]);
                    string dstName = slug.StartsWith("arena.", StringComparison.OrdinalIgnoreCase)
                        ? $"{slug}.json"
                        : $"arena.{slug}.json";
                    if (TryStageSanitizedFile(arenaFiles[i], Path.Combine(diskOutDir, dstName), checkAccessibility: false))
                        written++;
                }
            }

            // 5. Canonical market goods catalog (`goods.json`)
            string goodsSrc = TacSimContentLoader.ResolveRepoContentPath(Path.Combine("market", "goods.json"));
            if (TryStageSanitizedFile(goodsSrc, Path.Combine(diskOutDir, "goods.json"), checkAccessibility: false))
                written++;

            if (written == 0 || neighborhoodsWritten == 0 || unitsWritten == 0)
            {
                Debug.LogError(
                    $"[TacSim Builder] Fallback staging incomplete: total={written}, neighborhoods={neighborhoodsWritten}, units={unitsWritten}. " +
                    $"Verify crates/tacsim/content is reachable from '{Directory.GetCurrentDirectory()}'.");
            }

            if (refreshAssetDatabase)
                AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport);
            return written;
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
