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
        private static Shader _cachedFallbackShader;

        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        private static void ResetStaticCachesOnSubsystemInit()
        {
            ClearCache();
        }

        public static void ClearCache()
        {
            foreach (var kv in Cache)
            {
                if (kv.Value != null)
                {
                    if (Application.isPlaying) Object.Destroy(kv.Value);
                    else Object.DestroyImmediate(kv.Value);
                }
            }
            Cache.Clear();
            DestroyProceduralTex(ref _proceduralBulkheadTex);
            DestroyProceduralTex(ref _proceduralBulkheadNormal);
            DestroyProceduralTex(ref _proceduralDeckTex);
            DestroyProceduralTex(ref _proceduralDeckNormal);
            DestroyProceduralTex(ref _proceduralCeilingTex);
            DestroyProceduralTex(ref _proceduralCeilingNormal);
            _cachedFallbackShader = null;
        }

        private static void DestroyProceduralTex(ref Texture2D tex)
        {
            if (tex == null) return;
            if (Application.isPlaying) Object.Destroy(tex);
            else Object.DestroyImmediate(tex);
            tex = null;
        }

        public static Shader FindSafeLitShader()
        {
            Shader s = Shader.Find("Universal Render Pipeline/Lit");
            if (s != null) return s;
            s = Shader.Find("Universal Render Pipeline/Simple Lit");
            if (s != null) return s;
            s = Shader.Find("Universal Render Pipeline/Unlit");
            if (s != null) return s;
            s = Shader.Find("Standard");
            if (s != null) return s;
            s = Shader.Find("Sprites/Default");
            if (s != null) return s;
            s = Shader.Find("Hidden/Internal-Colored");
            if (s != null) return s;
            if (_cachedFallbackShader != null) return _cachedFallbackShader;
            _cachedFallbackShader = Shader.Find("Hidden/InternalErrorShader");
            return _cachedFallbackShader;
        }

        public static Shader FindSafeUnlitShader()
        {
            Shader s = Shader.Find("Universal Render Pipeline/Unlit");
            if (s != null) return s;
            s = Shader.Find("Unlit/Color");
            if (s != null) return s;
            s = Shader.Find("Sprites/Default");
            if (s != null) return s;
            return FindSafeLitShader();
        }

        private static Material CreateMaterialSafe(Shader shader, string name)
        {
            Shader resolved = shader != null ? shader : FindSafeLitShader();
            if (resolved != null)
                return new Material(resolved) { name = name };
            Debug.LogError($"[UrpMaterialFactory] No compatible shader found for material '{name}'.");
            return new Material(Shader.Find("Hidden/InternalErrorShader")) { name = name };
        }

        private static Shader FindLitShader() => FindSafeLitShader();
        private static Shader FindUnlitShader() => FindSafeUnlitShader();

        public static Material GetOpaque(string key, Color albedo, float smoothness = 0.35f, float metallic = 0.25f)
        {
            string fullKey = $"opaque:{key}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            var mat = CreateMaterialSafe(FindLitShader(), $"TacSim_{key}");
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

            var mat = CreateMaterialSafe(FindLitShader(), $"TacSim_Emissive_{key}");
            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", baseColor);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", baseColor);
            mat.EnableKeyword("_EMISSION");
            if (mat.HasProperty("_EmissionColor")) mat.SetColor("_EmissionColor", hdrEmission);
            Cache[fullKey] = mat;
            return mat;
        }

        private static Texture2D _proceduralBulkheadTex;
        private static Texture2D _proceduralBulkheadNormal;
        private static Texture2D _proceduralDeckTex;
        private static Texture2D _proceduralDeckNormal;
        private static Texture2D _proceduralCeilingTex;
        private static Texture2D _proceduralCeilingNormal;

        public static Material GetArmourglass(string key, Color tint)
        {
            string fullKey = $"glass:{key}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            var mat = CreateMaterialSafe(FindLitShader(), $"TacSim_Glass_{key}");
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

            bool usingProcAlbedo = albedo == null;
            Texture useAlbedo = !usingProcAlbedo ? albedo : (isFloor ? GetOrBuildProceduralDeckTex() : GetOrBuildProceduralBulkheadTex());
            bool usingProcNormal = normalMap == null && usingProcAlbedo;
            Texture useNormal = normalMap != null ? normalMap : (usingProcNormal ? (isFloor ? GetOrBuildProceduralDeckNormal() : GetOrBuildProceduralBulkheadNormal()) : null);
            if (usingProcAlbedo && useAlbedo != null)
                useAlbedo.wrapMode = TextureWrapMode.Repeat;

            var mat = CreateMaterialSafe(FindLitShader(), $"TacSim_PBR_{key}");
            Color effectiveTint = albedo != null
                ? Color.Lerp(Color.white, tint * 2.2f, 0.55f)
                : tint * 1.95f;
            effectiveTint.a = 1f;

            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", effectiveTint);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", effectiveTint);
            if (mat.HasProperty("_BaseMap")) mat.SetTexture("_BaseMap", useAlbedo);
            if (mat.HasProperty("_MainTex")) mat.SetTexture("_MainTex", useAlbedo);

            Vector2 cleanScale = new Vector2(Mathf.Max(1f, tiling.x), Mathf.Max(1f, tiling.y));
            mat.mainTextureScale = cleanScale;
            if (mat.HasProperty("_BaseMap")) mat.SetTextureScale("_BaseMap", cleanScale);

            if (useNormal != null && mat.HasProperty("_BumpMap"))
            {
                if (usingProcNormal)
                    useNormal.wrapMode = TextureWrapMode.Repeat;
                mat.SetTexture("_BumpMap", useNormal);
                mat.SetTextureScale("_BumpMap", cleanScale);
                if (mat.HasProperty("_BumpScale")) mat.SetFloat("_BumpScale", 0.85f);
                mat.EnableKeyword("_NORMALMAP");
            }
            if (metallicMap != null && mat.HasProperty("_MetallicGlossMap"))
            {
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

        public static Material GetStationCeilingMaterial(string key, Color tint, Vector2 tiling)
        {
            int tx = Mathf.Max(1, Mathf.RoundToInt(tiling.x));
            int ty = Mathf.Max(1, Mathf.RoundToInt(tiling.y));
            string fullKey = $"ceiling:{key}:{tx}x{ty}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            EnsureProceduralCeilingMaps();
            var mat = CreateMaterialSafe(FindLitShader(), $"TacSim_Ceiling_{key}");
            Color col = new Color(tint.r * 1.15f, tint.g * 1.18f, tint.b * 1.22f, 1f);
            if (mat.HasProperty("_BaseColor")) mat.SetColor("_BaseColor", col);
            if (mat.HasProperty("_Color")) mat.SetColor("_Color", col);
            if (mat.HasProperty("_BaseMap")) mat.SetTexture("_BaseMap", _proceduralCeilingTex);
            if (mat.HasProperty("_MainTex")) mat.SetTexture("_MainTex", _proceduralCeilingTex);

            Vector2 cleanScale = new Vector2(tx, ty);
            mat.mainTextureScale = cleanScale;
            if (mat.HasProperty("_BaseMap")) mat.SetTextureScale("_BaseMap", cleanScale);

            if (mat.HasProperty("_BumpMap") && _proceduralCeilingNormal != null)
            {
                mat.SetTexture("_BumpMap", _proceduralCeilingNormal);
                mat.SetTextureScale("_BumpMap", cleanScale);
                if (mat.HasProperty("_BumpScale")) mat.SetFloat("_BumpScale", 0.75f);
                mat.EnableKeyword("_NORMALMAP");
            }
            // Matte acoustic soffit so room point lights never produce glaring mirror hotspots on the ceiling!
            if (mat.HasProperty("_Smoothness")) mat.SetFloat("_Smoothness", 0.10f);
            if (mat.HasProperty("_Glossiness")) mat.SetFloat("_Glossiness", 0.10f);
            if (mat.HasProperty("_Metallic")) mat.SetFloat("_Metallic", 0.16f);

            Cache[fullKey] = mat;
            return mat;
        }

        private static Texture2D GetOrBuildProceduralBulkheadTex()
        {
            EnsureProceduralBulkheadMaps();
            return _proceduralBulkheadTex;
        }

        private static Texture2D GetOrBuildProceduralBulkheadNormal()
        {
            EnsureProceduralBulkheadMaps();
            return _proceduralBulkheadNormal;
        }

        private static void EnsureProceduralBulkheadMaps()
        {
            if (_proceduralBulkheadTex != null && _proceduralBulkheadNormal != null)
                return;

            const int S = 256;
            var px = new Color32[S * S];
            var h = new float[S * S];

            for (int y = 0; y < S; y++)
            {
                for (int x = 0; x < S; x++)
                {
                    int bx = x % 128; // two 2.1m architectural bays per 4.2m horizontal repeat
                    int edgeX = Mathf.Min(bx, 127 - bx);

                    bool kickplate = y < 34;
                    bool conduitRail = y >= 34 && y <= 41;
                    bool wainscot = y > 41 && y < 82;
                    bool dadoTrim = y >= 82 && y <= 86;
                    bool upperField = y > 86 && y < 220;
                    bool crownHeader = y >= 220;

                    float lum;
                    float elevH;

                    if (kickplate)
                    {
                        lum = 0.31f;
                        elevH = 0.25f;
                    }
                    else if (conduitRail)
                    {
                        lum = (y == 34 || y == 41) ? 0.24f : 0.48f;
                        elevH = (y == 34 || y == 41) ? 0.15f : 0.72f;
                    }
                    else if (wainscot)
                    {
                        bool flute = (y % 10 <= 2);
                        lum = flute ? 0.36f : 0.43f;
                        elevH = flute ? 0.35f : 0.52f;
                    }
                    else if (dadoTrim)
                    {
                        lum = (y == 82 || y == 86) ? 0.26f : 0.58f;
                        elevH = 0.78f;
                    }
                    else if (upperField)
                    {
                        int panelEdgeY = Mathf.Min(y - 87, 219 - y);
                        int minPanelEdge = Mathf.Min(edgeX, panelEdgeY);
                        bool recessedFrame = minPanelEdge >= 8 && minPanelEdge <= 10;
                        lum = recessedFrame ? 0.41f : 0.53f;
                        elevH = recessedFrame ? 0.36f : 0.62f;

                        // Subtle corner fastener dimples inside upper bulkhead panel
                        if ((bx == 16 || bx == 111) && (y == 98 || y == 208))
                        {
                            lum = 0.35f;
                            elevH = 0.20f;
                        }
                    }
                    else // crownHeader
                    {
                        bool louver = y >= 230 && y <= 246 && (bx >= 24 && bx <= 103) && (y % 5 <= 1);
                        lum = louver ? 0.23f : 0.35f;
                        elevH = louver ? 0.14f : 0.45f;
                    }

                    // Vertical expansion joint & bevel between bays
                    if (edgeX == 0)
                    {
                        lum *= 0.52f;
                        elevH = 0.05f;
                    }
                    else if (edgeX <= 2)
                    {
                        lum *= 0.84f;
                        elevH *= 0.65f;
                    }

                    // Subtle horizontal brushed-metal micro-grain (seamless)
                    float grain = Mathf.Sin(y * 0.65f + Mathf.Sin(x * 0.09817477f) * 1.4f) * 0.012f;
                    lum = Mathf.Clamp01(lum + grain);

                    byte r = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 242f), 0, 255);
                    byte g = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 248f), 0, 255);
                    byte b = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 255f), 0, 255);
                    if (dadoTrim && y > 82 && y < 86)
                    {
                        // Cool steel-cyan architectural pinstripe along dado rail
                        r = (byte)Mathf.Clamp(r - 14, 0, 255);
                        b = (byte)Mathf.Clamp(b + 12, 0, 255);
                    }

                    px[y * S + x] = new Color32(r, g, b, 255);
                    h[y * S + x] = elevH;
                }
            }

            _proceduralBulkheadTex = CreateSeamlessAlbedo("ProceduralStationBulkhead", S, px);
            _proceduralBulkheadNormal = BuildSeamlessNormalFromHeight("ProceduralStationBulkhead_N", S, h, strength: 2.2f);
        }

        private static Texture2D GetOrBuildProceduralDeckTex()
        {
            EnsureProceduralDeckMaps();
            return _proceduralDeckTex;
        }

        private static Texture2D GetOrBuildProceduralDeckNormal()
        {
            EnsureProceduralDeckMaps();
            return _proceduralDeckNormal;
        }

        private static void EnsureProceduralDeckMaps()
        {
            if (_proceduralDeckTex != null && _proceduralDeckNormal != null)
                return;

            const int S = 256;
            var px = new Color32[S * S];
            var h = new float[S * S];

            for (int y = 0; y < S; y++)
            {
                for (int x = 0; x < S; x++)
                {
                    int tx = x % 128;
                    int ty = y % 128;
                    int edge = Mathf.Min(Mathf.Min(tx, 127 - tx), Mathf.Min(ty, 127 - ty));

                    float lum = 0.44f;
                    float elevH = 0.55f;

                    if (edge == 0)
                    {
                        lum = 0.22f;
                        elevH = 0.08f;
                    }
                    else if (edge <= 2)
                    {
                        lum = 0.34f;
                        elevH = 0.30f;
                    }
                    else if (edge >= 10 && edge <= 11)
                    {
                        lum = 0.37f;
                        elevH = 0.40f;
                    }
                    else if (edge > 14)
                    {
                        // Subtle non-slip deck tread ribs inside plate interior
                        bool rib = (ty % 8 <= 1) && (tx % 32 >= 4 && tx % 32 <= 27);
                        if (rib)
                        {
                            lum = 0.47f;
                            elevH = 0.66f;
                        }
                    }

                    // Recessed tie-down sockets near plate corners
                    if ((tx >= 16 && tx <= 20) || (tx >= 107 && tx <= 111))
                    {
                        if ((ty >= 16 && ty <= 20) || (ty >= 107 && ty <= 111))
                        {
                            lum = 0.28f;
                            elevH = 0.18f;
                        }
                    }

                    byte r = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 240f), 0, 255);
                    byte g = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 246f), 0, 255);
                    byte b = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 255f), 0, 255);
                    px[y * S + x] = new Color32(r, g, b, 255);
                    h[y * S + x] = elevH;
                }
            }

            _proceduralDeckTex = CreateSeamlessAlbedo("ProceduralStationDeck", S, px);
            _proceduralDeckNormal = BuildSeamlessNormalFromHeight("ProceduralStationDeck_N", S, h, strength: 1.8f);
        }

        private static void EnsureProceduralCeilingMaps()
        {
            if (_proceduralCeilingTex != null && _proceduralCeilingNormal != null)
                return;

            const int S = 256;
            var px = new Color32[S * S];
            var h = new float[S * S];

            for (int y = 0; y < S; y++)
            {
                for (int x = 0; x < S; x++)
                {
                    int cx = x % 128;
                    int cy = y % 128;
                    int edge = Mathf.Min(Mathf.Min(cx, 127 - cx), Mathf.Min(cy, 127 - cy));

                    float lum;
                    float elevH;
                    if (edge <= 6)
                    {
                        // Structural T-bar rib framing the coffer
                        lum = edge == 0 ? 0.18f : 0.29f;
                        elevH = edge == 0 ? 0.45f : 0.82f;
                    }
                    else if (edge <= 10)
                    {
                        // Bevelled step into recessed acoustic coffer
                        lum = 0.22f;
                        elevH = 0.45f;
                    }
                    else
                    {
                        // Matte acoustic coffer field
                        bool perf = (cx % 6 == 0) && (cy % 6 == 0);
                        lum = perf ? 0.16f : 0.20f;
                        elevH = perf ? 0.18f : 0.24f;
                    }

                    byte r = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 238f), 0, 255);
                    byte g = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 245f), 0, 255);
                    byte b = (byte)Mathf.Clamp(Mathf.RoundToInt(lum * 255f), 0, 255);
                    px[y * S + x] = new Color32(r, g, b, 255);
                    h[y * S + x] = elevH;
                }
            }

            _proceduralCeilingTex = CreateSeamlessAlbedo("ProceduralStationCeiling", S, px);
            _proceduralCeilingNormal = BuildSeamlessNormalFromHeight("ProceduralStationCeiling_N", S, h, strength: 2.0f);
        }

        private static Texture2D CreateSeamlessAlbedo(string name, int size, Color32[] pixels)
        {
            var tex = new Texture2D(size, size, TextureFormat.RGBA32, true)
            {
                name = name,
                wrapMode = TextureWrapMode.Repeat,
                filterMode = FilterMode.Trilinear,
                anisoLevel = 8
            };
            tex.SetPixels32(pixels);
            tex.Apply(true, false);
            return tex;
        }

        private static Texture2D BuildSeamlessNormalFromHeight(string name, int size, float[] h, float strength)
        {
            var normPx = new Color32[size * size];
            for (int y = 0; y < size; y++)
            {
                int ym = (y - 1 + size) % size;
                int yp = (y + 1) % size;
                for (int x = 0; x < size; x++)
                {
                    int xm = (x - 1 + size) % size;
                    int xp = (x + 1) % size;
                    float dx = (h[y * size + xm] - h[y * size + xp]) * strength;
                    float dy = (h[ym * size + x] - h[yp * size + x]) * strength;
                    Vector3 n = new Vector3(dx, dy, 1f).normalized;
                    byte nxB = (byte)Mathf.Clamp(Mathf.RoundToInt((n.x * 0.5f + 0.5f) * 255f), 0, 255);
                    byte nyB = (byte)Mathf.Clamp(Mathf.RoundToInt((n.y * 0.5f + 0.5f) * 255f), 0, 255);
                    // Pack for URP UnpackNormalMapRGorAG (`packedNormal.a *= packedNormal.r` -> DXT5nm `(1, y, 1, x)`)
                    normPx[y * size + x] = new Color32(255, nyB, 255, nxB);
                }
            }
            var tex = new Texture2D(size, size, TextureFormat.RGBA32, true, true)
            {
                name = name,
                wrapMode = TextureWrapMode.Repeat,
                filterMode = FilterMode.Trilinear,
                anisoLevel = 8
            };
            tex.SetPixels32(normPx);
            tex.Apply(true, false);
            return tex;
        }

        public static Material GetHologramUnlit(string key, Color color)
        {
            string fullKey = $"holo:{key}";
            if (Cache.TryGetValue(fullKey, out Material existing) && existing != null)
                return existing;

            var mat = CreateMaterialSafe(FindUnlitShader(), $"TacSim_Holo_{key}");
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
            var mat = CreateMaterialSafe(FindLitShader(), $"TacSim_Tex_{name}");
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
