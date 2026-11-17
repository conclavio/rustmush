using System.Collections.Generic;
using UnityEngine;
using UnityEngine.Rendering;

namespace TacSim.Core
{
    /// <summary>
    /// Shared URP-first material cache so generated architectural decks, armourglass windows,
    /// holo-tanks, docking rings, and fallback character rigs never render magenta in URP.
    /// </summary>
    public static class UrpMaterialFactory
    {
        private static readonly Dictionary<string, Material> Cache = new Dictionary<string, Material>();

        private static Shader FindLitShader()
        {
            Shader s = Shader.Find("Universal Render Pipeline/Lit");
            if (s != null) return s;
            s = Shader.Find("Universal Render Pipeline/Simple Lit");
            if (s != null) return s;
            return Shader.Find("Standard");
        }

        private static Shader FindUnlitShader()
        {
            Shader s = Shader.Find("Universal Render Pipeline/Unlit");
            if (s != null) return s;
            return Shader.Find("Unlit/Color");
        }

        public static Material GetOpaque(string key, Color albedo, float smoothness = 0.35f, float metallic = 0.25f)
        {
            string fullKey = $"opaque:{key}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            var mat = new Material(FindLitShader()) { name = $"TacSim_{key}" };
            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", albedo);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", albedo);
            if (mat.HasProperty("_Smoothness")) mat.SetFloat("_Smoothness", smoothness);
            if (mat.HasProperty("_Glossiness")) mat.SetFloat("_Glossiness", smoothness);
            if (mat.HasProperty("_Metallic")) mat.SetFloat("_Metallic", metallic);
            Cache[fullKey] = mat;
            return mat;
        }

        public static Material GetEmissive(string key, Color baseColor, Color hdrEmission)
        {
            string fullKey = $"emissive:{key}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            var mat = new Material(FindLitShader()) { name = $"TacSim_Emissive_{key}" };
            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", baseColor);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", baseColor);
            mat.EnableKeyword("_EMISSION");
            if (mat.HasProperty("_EmissionColor")) mat.SetColor("_EmissionColor", hdrEmission);
            Cache[fullKey] = mat;
            return mat;
        }

        private static Texture2D _proceduralBulkheadTex;
        private static Texture2D _proceduralDeckTex;

        public static Material GetArmourglass(string key, Color tint)
        {
            string fullKey = $"glass:{key}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            var mat = new Material(FindLitShader()) { name = $"TacSim_Glass_{key}" };
            if (mat.HasProperty("_Surface")) mat.SetFloat("_Surface", 1f); // Transparent in URP Lit
            if (mat.HasProperty("_Blend")) mat.SetFloat("_Blend", 0f);
            mat.EnableKeyword("_SURFACE_TYPE_TRANSPARENT");
            mat.DisableKeyword("_ALPHAPREMULTIPLY_ON");
            mat.SetOverrideTag("RenderType", "Transparent");
            mat.SetInt("_SrcBlend", (int)BlendMode.SrcAlpha);
            mat.SetInt("_DstBlend", (int)BlendMode.OneMinusSrcAlpha);
            mat.SetInt("_SrcBlendAlpha", (int)BlendMode.One);
            mat.SetInt("_DstBlendAlpha", (int)BlendMode.OneMinusSrcAlpha);
            mat.SetInt("_ZWrite", 0);
            mat.renderQueue = (int)RenderQueue.Transparent;
            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", tint);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", tint);
            if (mat.HasProperty("_Smoothness")) mat.SetFloat("_Smoothness", 0.42f);
            if (mat.HasProperty("_Metallic")) mat.SetFloat("_Metallic", 0.05f);
            Cache[fullKey] = mat;
            return mat;
        }

        public static Material GetTiledPbrSurface(
            string key,
            Texture albedo,
            Texture normalMap,
            Texture metallicMap,
            Color tint,
            Vector2 tiling,
            bool isFloor,
            float smoothness = 0.40f,
            float metallic = 0.42f)
        {
            int tx = Mathf.Max(1, Mathf.RoundToInt(tiling.x * 2f));
            int ty = Mathf.Max(1, Mathf.RoundToInt(tiling.y * 2f));
            string fullKey = $"pbr:{key}:{tx}x{ty}:{(albedo != null ? albedo.name : "proc")}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            Texture useAlbedo = albedo != null ? albedo : (isFloor ? GetOrBuildProceduralDeckTex() : GetOrBuildProceduralBulkheadTex());
            useAlbedo.wrapMode = TextureWrapMode.Repeat;

            var mat = new Material(FindLitShader()) { name = $"TacSim_PBR_{key}" };
            Color effectiveTint = albedo != null
                ? Color.Lerp(Color.white, tint * 2.2f, 0.55f)
                : tint * 2.1f;
            effectiveTint.a = 1f;

            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", effectiveTint);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", effectiveTint);
            if (mat.HasProperty("_BaseMap")) mat.SetTexture("_BaseMap", useAlbedo);
            if (mat.HasProperty("_MainTex")) mat.SetTexture("_MainTex", useAlbedo);

            Vector2 cleanScale = new Vector2(Mathf.Max(1f, tiling.x), Mathf.Max(1f, tiling.y));
            mat.mainTextureScale = cleanScale;
            if (mat.HasProperty("_BaseMap")) mat.SetTextureScale("_BaseMap", cleanScale);

            if (normalMap != null && mat.HasProperty("_BumpMap"))
            {
                normalMap.wrapMode = TextureWrapMode.Repeat;
                mat.SetTexture("_BumpMap", normalMap);
                mat.SetTextureScale("_BumpMap", cleanScale);
                mat.EnableKeyword("_NORMALMAP");
            }
            if (metallicMap != null && mat.HasProperty("_MetallicGlossMap"))
            {
                metallicMap.wrapMode = TextureWrapMode.Repeat;
                mat.SetTexture("_MetallicGlossMap", metallicMap);
                mat.SetTextureScale("_MetallicGlossMap", cleanScale);
                mat.EnableKeyword("_METALLICSPECGLOSSMAP");
            }

            if (mat.HasProperty("_Smoothness")) mat.SetFloat("_Smoothness", smoothness);
            if (mat.HasProperty("_Glossiness")) mat.SetFloat("_Glossiness", smoothness);
            if (mat.HasProperty("_Metallic")) mat.SetFloat("_Metallic", metallic);

            Cache[fullKey] = mat;
            return mat;
        }

        private static Texture2D GetOrBuildProceduralBulkheadTex()
        {
            if (_proceduralBulkheadTex != null) return _proceduralBulkheadTex;
            const int S = 128;
            var tex = new Texture2D(S, S, TextureFormat.RGBA32, true)
            {
                name = "ProceduralBulkheadPlate",
                wrapMode = TextureWrapMode.Repeat,
                filterMode = FilterMode.Bilinear
            };
            var px = new Color32[S * S];
            for (int y = 0; y < S; y++)
            {
                for (int x = 0; x < S; x++)
                {
                    bool vSeam = (x <= 1 || x >= S - 2 || Mathf.Abs(x - S / 2) <= 1);
                    bool hSeam = (y <= 1 || y >= S - 2 || y == 28 || y == S - 28);
                    bool dado = y < 28;
                    bool hazard = y >= 26 && y <= 30 && ((x + y) % 16 < 8);
                    float v = dado ? 0.42f : 0.56f;
                    if (vSeam || hSeam) v *= 0.55f;
                    // Corner rivets
                    if ((x == 6 || x == S / 2 - 6 || x == S / 2 + 6 || x == S - 6) && (y == 34 || y == S - 8))
                        v = 0.78f;

                    byte c = (byte)Mathf.Clamp(Mathf.RoundToInt(v * 255f), 0, 255);
                    px[y * S + x] = hazard
                        ? new Color32(185, 145, 52, 255)
                        : new Color32((byte)(c * 0.94f), (byte)(c * 0.97f), c, 255);
                }
            }
            tex.SetPixels32(px);
            tex.Apply(true, false);
            _proceduralBulkheadTex = tex;
            return tex;
        }

        private static Texture2D GetOrBuildProceduralDeckTex()
        {
            if (_proceduralDeckTex != null) return _proceduralDeckTex;
            const int S = 128;
            var tex = new Texture2D(S, S, TextureFormat.RGBA32, true)
            {
                name = "ProceduralDeckGrating",
                wrapMode = TextureWrapMode.Repeat,
                filterMode = FilterMode.Bilinear
            };
            var px = new Color32[S * S];
            for (int y = 0; y < S; y++)
            {
                for (int x = 0; x < S; x++)
                {
                    bool border = (x % 64 <= 1) || (y % 64 <= 1);
                    bool tread = ((x + y) % 12 < 3) && (((x / 12) + (y / 12)) % 2 == 0);
                    float v = border ? 0.34f : (tread ? 0.62f : 0.50f);
                    byte c = (byte)Mathf.Clamp(Mathf.RoundToInt(v * 255f), 0, 255);
                    px[y * S + x] = new Color32((byte)(c * 0.95f), (byte)(c * 0.98f), c, 255);
                }
            }
            tex.SetPixels32(px);
            tex.Apply(true, false);
            _proceduralDeckTex = tex;
            return tex;
        }

        public static Material GetHologramUnlit(string key, Color color)
        {
            string fullKey = $"holo:{key}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            var mat = new Material(FindUnlitShader()) { name = $"TacSim_Holo_{key}" };
            if (mat.HasProperty("_Surface")) mat.SetFloat("_Surface", 1f);
            mat.SetOverrideTag("RenderType", "Transparent");
            mat.SetInt("_SrcBlend", (int)BlendMode.SrcAlpha);
            mat.SetInt("_DstBlend", (int)BlendMode.One);
            mat.SetInt("_ZWrite", 0);
            mat.renderQueue = (int)RenderQueue.Transparent + 10;
            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", color);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", color);
            Cache[fullKey] = mat;
            return mat;
        }

        public static Material CreateTexturedLit(string name, Texture2D albedoTex, float smoothness = 0.14f, float metallic = 0.06f)
        {
            var mat = new Material(FindLitShader()) { name = $"TacSim_Tex_{name}" };
            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", Color.white);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", Color.white);
            if (mat.HasProperty("_BaseMap")) mat.SetTexture("_BaseMap", albedoTex);
            if (mat.HasProperty("_MainTex")) mat.SetTexture("_MainTex", albedoTex);
            if (mat.HasProperty("_Smoothness")) mat.SetFloat("_Smoothness", smoothness);
            if (mat.HasProperty("_Glossiness")) mat.SetFloat("_Glossiness", smoothness);
            if (mat.HasProperty("_Metallic")) mat.SetFloat("_Metallic", metallic);
            return mat;
        }
    }
}
