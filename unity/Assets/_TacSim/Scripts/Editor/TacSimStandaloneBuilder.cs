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

        public int callbackOrder => 0;

        public void OnPreprocessBuild(BuildReport report)
        {
            int count = StageEmbeddedFallbackPacks();
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
            BuildReport report = BuildPipeline.BuildPlayer(opts);
            Debug.Log($"[TacSim Builder] Build finished: result={report.summary.result} errors={report.summary.totalErrors} warnings={report.summary.totalWarnings}");
            return report;
        }

        public static int StageEmbeddedFallbackPacks()
        {
            string diskOutDir = Path.GetFullPath(Path.Combine(Application.dataPath, "_TacSim", "Resources", "TacSimFallbackPacks"));
            Directory.CreateDirectory(diskOutDir);

            int written = 0;

            // 1. Corridor port & OOC neighborhood packs
            string oocSrc = TacSimContentLoader.ResolveMushAtlasContentPath("neighborhood.ooc_area.json");
            if (TryStageSanitizedFile(oocSrc, Path.Combine(diskOutDir, "neighborhood.ooc_area.json")))
                written++;

            string atlasDir = TacSimContentLoader.ResolveRepoContentPath("atlas");
            if (Directory.Exists(atlasDir))
            {
                string[] neighborhoodFiles = Directory.GetFiles(atlasDir, "neighborhood.*.json");
                for (int i = 0; i < neighborhoodFiles.Length; i++)
                {
                    string fileName = Path.GetFileName(neighborhoodFiles[i]);
                    if (fileName.IndexOf("staff", StringComparison.OrdinalIgnoreCase) >= 0)
                        continue;
                    if (TryStageSanitizedFile(neighborhoodFiles[i], Path.Combine(diskOutDir, fileName)))
                        written++;
                }
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
                    if (TryStageSanitizedFile(castFiles[i], Path.Combine(diskOutDir, $"cast.{slug}.json")))
                        written++;
                }
            }

            // 3. Walkable ship hull unit templates (`dray.json`, `dray_p.json`, `corvette.json`, etc.)
            string unitsDir = TacSimContentLoader.ResolveRepoContentPath("units");
            if (Directory.Exists(unitsDir))
            {
                string[] unitFiles = Directory.GetFiles(unitsDir, "*.json");
                for (int i = 0; i < unitFiles.Length; i++)
                {
                    string fileName = Path.GetFileName(unitFiles[i]);
                    if (TryStageSanitizedFile(unitFiles[i], Path.Combine(diskOutDir, fileName)))
                        written++;
                }
            }

            // 4. Canonical market goods catalog (`goods.json`)
            string goodsSrc = TacSimContentLoader.ResolveRepoContentPath(Path.Combine("market", "goods.json"));
            if (TryStageSanitizedFile(goodsSrc, Path.Combine(diskOutDir, "goods.json")))
                written++;

            AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport);
            return written;
        }

        private static bool TryStageSanitizedFile(string srcPath, string dstPath)
        {
            if (string.IsNullOrEmpty(srcPath) || !File.Exists(srcPath))
                return false;
            string raw = File.ReadAllText(srcPath);
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
