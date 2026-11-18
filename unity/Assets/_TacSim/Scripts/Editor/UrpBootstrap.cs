using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.Universal;

namespace TacSim.Editor
{
    [InitializeOnLoad]
    public static class UrpBootstrap
    {
        static UrpBootstrap()
        {
            EditorApplication.delayCall += EnsureUrpAssigned;
        }

        public static void EnsureUrpAssigned()
        {
            if (GraphicsSettings.defaultRenderPipeline != null)
                return;

            var rp = AssetDatabase.LoadAssetAtPath<RenderPipelineAsset>("Assets/_TacSim/Settings/PC_RPAsset.asset");
            if (rp == null)
            {
                string[] guids = AssetDatabase.FindAssets("t:UniversalRenderPipelineAsset");
                if (guids != null && guids.Length > 0)
                {
                    rp = AssetDatabase.LoadAssetAtPath<RenderPipelineAsset>(AssetDatabase.GUIDToAssetPath(guids[0]));
                }
            }

            if (rp != null)
            {
                GraphicsSettings.defaultRenderPipeline = rp;
                QualitySettings.renderPipeline = rp;
                Debug.Log($"[TacSim] Assigned URP RenderPipelineAsset: {AssetDatabase.GetAssetPath(rp)}");
            }
        }
    }
}
