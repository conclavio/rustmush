#if UNITY_EDITOR
using System.IO;
using TacSim.World;
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;
using UnityEngine;

namespace TacSim.Editor
{
    /// <summary>
    /// Bundles baseline TacSim &amp; Atlas JSON content packs into `Assets/_TacSim/Resources/TacSimFallbackPacks/`
    /// before standalone Player builds so offline-cache-cold standalone `.exe` builds work immediately even when
    /// connecting to an older `mushd` server that does not yet serve `/api/content/manifest`.
    /// </summary>
    public sealed class TacSimPackFallbackBundler : IPreprocessBuildWithReport
    {
        public int callbackOrder => 0;

        public void OnPreprocessBuild(BuildReport report)
        {
            BundleBaselinePacksToResources();
        }

        [MenuItem("TacSim/Bundle Fallback Content Packs to Resources")]
        public static void BundleBaselinePacksToResources()
        {
            string outDir = Path.Combine(Application.dataPath, "_TacSim", "Resources", "TacSimFallbackPacks");
            Directory.CreateDirectory(outDir);

            (string id, string path)[] sources =
            {
                ("neighborhood.ooc_area", TacSimContentLoader.ResolveMushAtlasContentPath("neighborhood.ooc_area.json")),
                ("neighborhood.ceres_freeport", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", "neighborhood.ceres_freeport.json"))),
                ("neighborhood.stickney_yards", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", "neighborhood.stickney_yards.json"))),
                ("neighborhood.aitken_highport", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", "neighborhood.aitken_highport.json"))),
                ("goods", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("market", "goods.json"))),
                ("dray", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("units", "dray.json"))),
                ("dray_p", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("units", "dray_p.json"))),
                ("corvette", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("units", "corvette.json"))),
                ("arena_atlas.proving_ground", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", "proving_ground.json"))),
                ("arena_atlas.mars_canyon", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", "mars_canyon.json"))),
                ("arena_atlas.winter_valley", TacSimContentLoader.ResolveRepoContentPath(Path.Combine("atlas", "winter_valley.json")))
            };

            int copied = 0;
            for (int i = 0; i < sources.Length; i++)
            {
                if (!File.Exists(sources[i].path)) continue;
                string dest = Path.Combine(outDir, sources[i].id + ".txt");
                File.Copy(sources[i].path, dest, overwrite: true);
                copied++;
            }
            AssetDatabase.Refresh();
            Debug.Log($"[TacSimPackFallbackBundler] Bundled {copied} baseline content packs into Resources/TacSimFallbackPacks.");
        }
    }
}
#endif
