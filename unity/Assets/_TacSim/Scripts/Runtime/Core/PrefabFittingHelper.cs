using System;
using System.Collections.Generic;
using UnityEngine;

namespace TacSim.Core
{
    /// <summary>
    /// Instantiates imported commercial kit prefabs (DOTTEAM Heavy Station Kit, Vattalus Fenris,
    /// MSGDI Shipyards/Freighters, Gllow3D ambient workers) with automatic URP material repair,
    /// vendor-script stripping, optional collider stripping, proportional bounds scaling, and
    /// millimeter-exact floor grounding.
    /// </summary>
    public static class PrefabFittingHelper
    {
        private static readonly Dictionary<Material, Material> UrpCache = new Dictionary<Material, Material>();
        private static readonly Dictionary<Texture, Material> OverrideAlbedoCache = new Dictionary<Texture, Material>();
        private static GameObject _inactiveStagingRoot;

        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        private static void ResetCachesOnSubsystemInit()
        {
            ClearCache();
        }

        public static void ClearCache()
        {
            foreach (var kv in UrpCache)
            {
                if (kv.Value != null)
                {
                    if (Application.isPlaying) UnityEngine.Object.Destroy(kv.Value);
                    else UnityEngine.Object.DestroyImmediate(kv.Value);
                }
            }
            UrpCache.Clear();

            foreach (var kv in OverrideAlbedoCache)
            {
                if (kv.Value != null)
                {
                    if (Application.isPlaying) UnityEngine.Object.Destroy(kv.Value);
                    else UnityEngine.Object.DestroyImmediate(kv.Value);
                }
            }
            OverrideAlbedoCache.Clear();

            if (_inactiveStagingRoot != null)
            {
                if (Application.isPlaying) UnityEngine.Object.Destroy(_inactiveStagingRoot);
                else UnityEngine.Object.DestroyImmediate(_inactiveStagingRoot);
                _inactiveStagingRoot = null;
            }
        }

        private static GameObject InstantiateInactiveAndPrepare(
            GameObject prefab,
            Transform targetParent,
            string instanceName,
            bool keepColliders,
            Texture overrideAlbedo)
        {
            // Instantiate under an inactive staging container so vendor demo Awake()/OnEnable() NEVER run
            // before foreign MonoBehaviours are stripped!
            if (_inactiveStagingRoot == null)
            {
                _inactiveStagingRoot = new GameObject("__TacSimPrefabStaging");
                _inactiveStagingRoot.SetActive(false);
                _inactiveStagingRoot.hideFlags = HideFlags.HideAndDontSave;
            }

            var inst = UnityEngine.Object.Instantiate(prefab, _inactiveStagingRoot.transform);
            inst.name = instanceName;
            PrepareInstance(inst, keepColliders, overrideAlbedo);
            inst.transform.SetParent(targetParent, false);
            return inst;
        }

        public static GameObject SpawnGrounded(
            GameObject prefab,
            Transform parent,
            string instanceName,
            Vector3 floorWorldPos,
            Quaternion worldRot,
            float targetHeightM = 0f,
            float maxDimensionM = 0f,
            bool keepColliders = false,
            Texture overrideAlbedo = null)
        {
            if (prefab == null) return null;

            var inst = InstantiateInactiveAndPrepare(prefab, parent, instanceName, keepColliders, overrideAlbedo);
            Vector3 authoredScale = inst.transform.localScale;
            if (authoredScale.sqrMagnitude < 1e-6f) authoredScale = Vector3.one;
            inst.transform.SetPositionAndRotation(floorWorldPos, worldRot);
            inst.transform.localScale = authoredScale;

            if (!TryComputeRendererBounds(inst, out Bounds b))
                return inst;

            float scaleFactor = 1f;
            if (targetHeightM > 0.01f && b.size.y > 0.001f)
            {
                scaleFactor = targetHeightM / b.size.y;
            }
            if (maxDimensionM > 0.01f)
            {
                float maxAxis = Mathf.Max(b.size.x, Mathf.Max(b.size.y, b.size.z)) * scaleFactor;
                if (maxAxis > maxDimensionM && maxAxis > 0.001f)
                    scaleFactor *= maxDimensionM / maxAxis;
            }

            if (Mathf.Abs(scaleFactor - 1f) > 0.001f)
            {
                inst.transform.localScale = authoredScale * scaleFactor;
                TryComputeRendererBounds(inst, out b);
            }

            // Shift so horizontal center sits on floorWorldPos.xz and bottom rests cleanly on floorWorldPos.y
            Vector3 shift = new Vector3(
                floorWorldPos.x - b.center.x,
                floorWorldPos.y - b.min.y,
                floorWorldPos.z - b.center.z);
            inst.transform.position += shift;
            return inst;
        }

        public static GameObject SpawnCentered(
            GameObject prefab,
            Transform parent,
            string instanceName,
            Vector3 centerWorldPos,
            Quaternion worldRot,
            float maxDimensionM = 0f,
            bool keepColliders = false,
            Texture overrideAlbedo = null,
            bool scaleUpToTarget = true)
        {
            if (prefab == null) return null;

            var inst = InstantiateInactiveAndPrepare(prefab, parent, instanceName, keepColliders, overrideAlbedo);
            Vector3 authoredScale = inst.transform.localScale;
            if (authoredScale.sqrMagnitude < 1e-6f) authoredScale = Vector3.one;
            // Measure unrotated mesh bounds first so 3/4 yaw rotations don't inflate AABB dimensions,
            // and scale BOTH miniature vendor prefabs (e.g. MSGDI ~0.5m ships/cradles) and oversized prefabs to target world meters!
            inst.transform.SetPositionAndRotation(centerWorldPos, Quaternion.identity);
            inst.transform.localScale = authoredScale;

            if (TryComputeRendererBounds(inst, out Bounds b))
            {
                if (maxDimensionM > 0.01f)
                {
                    float maxAxis = Mathf.Max(b.size.x, Mathf.Max(b.size.y, b.size.z));
                    if (maxAxis > 0.0005f && (scaleUpToTarget || maxAxis > maxDimensionM))
                    {
                        inst.transform.localScale = authoredScale * (maxDimensionM / maxAxis);
                    }
                }
                inst.transform.rotation = worldRot;
                if (TryComputeRendererBounds(inst, out Bounds bRotated))
                    inst.transform.position += (centerWorldPos - bRotated.center);
            }
            else
            {
                inst.transform.rotation = worldRot;
            }
            return inst;
        }

        public static bool TryComputeRendererBounds(GameObject root, out Bounds combined)
        {
            combined = default;
            if (root == null) return false;
            var renderers = root.GetComponentsInChildren<Renderer>(true);
            bool any = false;
            for (int i = 0; i < renderers.Length; i++)
            {
                var r = renderers[i];
                if (r == null || r is ParticleSystemRenderer) continue;
                if (!any)
                {
                    combined = r.bounds;
                    any = true;
                }
                else
                {
                    combined.Encapsulate(r.bounds);
                }
            }
            return any;
        }

        private static void PrepareInstance(GameObject inst, bool keepColliders, Texture overrideAlbedo)
        {
            // Remove any vendor demo MonoBehaviours so foreign triggers/door scripts never conflict
            var behaviours = inst.GetComponentsInChildren<MonoBehaviour>(true);
            for (int i = 0; i < behaviours.Length; i++)
            {
                var mb = behaviours[i];
                if (mb == null) continue;
                string ns = mb.GetType().Namespace ?? string.Empty;
                if (ns.StartsWith("TacSim", StringComparison.Ordinal) ||
                    ns.StartsWith("TMPro", StringComparison.Ordinal) ||
                    ns.StartsWith("UnityEngine", StringComparison.Ordinal))
                    continue;

                mb.enabled = false;
                UnityEngine.Object.DestroyImmediate(mb);
            }

            var animators = inst.GetComponentsInChildren<Animator>(true);
            for (int i = 0; i < animators.Length; i++)
            {
                if (animators[i] != null)
                    animators[i].applyRootMotion = false;
            }

            if (!keepColliders)
            {
                var cols = inst.GetComponentsInChildren<Collider>(true);
                for (int i = 0; i < cols.Length; i++)
                {
                    if (cols[i] != null) cols[i].enabled = false;
                }
            }

            SanitizeRenderersForUrp(inst, overrideAlbedo);
        }

        public static void SanitizeRenderersForUrp(GameObject root, Texture overrideAlbedo = null)
        {
            Shader urpLit = UrpMaterialFactory.FindSafeLitShader();
            if (urpLit == null) return;

            var renderers = root.GetComponentsInChildren<Renderer>(true);
            for (int rIdx = 0; rIdx < renderers.Length; rIdx++)
            {
                var r = renderers[rIdx];
                if (r == null || r is ParticleSystemRenderer) continue;

                var mats = r.sharedMaterials;
                if (mats == null || mats.Length == 0) continue;

                bool changed = false;
                var list = new List<Material>(mats.Length);
                for (int i = 0; i < mats.Length; i++)
                {
                    Material m = mats[i];
                    if (m == null)
                    {
                        changed = true;
                        continue;
                    }

                    if (overrideAlbedo != null)
                    {
                        if (!OverrideAlbedoCache.TryGetValue(overrideAlbedo, out Material custom) || custom == null)
                        {
                            custom = new Material(urpLit) { name = $"TacSim_Override_{overrideAlbedo.name}_URP" };
                            if (custom.HasProperty("_BaseMap")) custom.SetTexture("_BaseMap", overrideAlbedo);
                            if (custom.HasProperty("_BaseColor")) custom.SetColor("_BaseColor", Color.white);
                            if (custom.HasProperty("_Smoothness")) custom.SetFloat("_Smoothness", 0.32f);
                            OverrideAlbedoCache[overrideAlbedo] = custom;
                        }
                        list.Add(custom);
                        changed = true;
                        continue;
                    }

                    Shader sh = m.shader;
                    string shName = sh != null ? sh.name : string.Empty;
                    bool isUrpCompatible = sh != null &&
                                           sh.isSupported &&
                                           shName != "Hidden/InternalErrorShader" &&
                                           (shName.StartsWith("Universal Render Pipeline/", StringComparison.Ordinal) ||
                                            shName.StartsWith("Shader Graphs/", StringComparison.Ordinal) ||
                                            shName.StartsWith("Sprites/", StringComparison.Ordinal) ||
                                            shName.StartsWith("UI/", StringComparison.Ordinal) ||
                                            shName.StartsWith("TextMeshPro/", StringComparison.Ordinal));
                    bool needsUpgrade = !isUrpCompatible;

                    if (!needsUpgrade)
                    {
                        list.Add(m);
                        continue;
                    }

                    if (!UrpCache.TryGetValue(m, out Material upg) || upg == null)
                    {
                        upg = new Material(urpLit) { name = m.name + "_AutoURP" };
                        Texture main = m.HasProperty("_BaseMap") ? m.GetTexture("_BaseMap")
                                     : (m.HasProperty("_MainTex") ? m.GetTexture("_MainTex") : null);
                        Color col = m.HasProperty("_BaseColor") ? m.GetColor("_BaseColor")
                                  : (m.HasProperty("_Color") ? m.GetColor("_Color") : new Color(0.55f, 0.58f, 0.62f));
                        Texture bump = m.HasProperty("_BumpMap") ? m.GetTexture("_BumpMap") : null;
                        float metal = m.HasProperty("_Metallic") ? m.GetFloat("_Metallic") : 0.25f;
                        float smooth = m.HasProperty("_Glossiness") ? m.GetFloat("_Glossiness") : 0.42f;

                        if (main != null) upg.SetTexture("_BaseMap", main);
                        upg.SetColor("_BaseColor", col);
                        if (bump != null)
                        {
                            upg.SetTexture("_BumpMap", bump);
                            upg.EnableKeyword("_NORMALMAP");
                        }
                        upg.SetFloat("_Metallic", metal);
                        upg.SetFloat("_Smoothness", smooth);
                        UrpCache[m] = upg;
                    }

                    list.Add(upg);
                    changed = true;
                }

                if (list.Count == 0)
                {
                    list.Add(UrpMaterialFactory.GetOpaque("PropFallback", new Color(0.28f, 0.32f, 0.36f), 0.4f, 0.5f));
                    changed = true;
                }

                if (changed)
                    r.sharedMaterials = list.ToArray();
            }
        }
    }
}
