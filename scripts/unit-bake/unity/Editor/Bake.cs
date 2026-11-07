// The unit bake (doc 22 D-22-10, cut U.6): one model's bake matrix from one
// named asset pack — both views (`fp` at 3 degrees down, Q-22-3; `iso` at the
// 2.5D's 28), eight facings, six poses (still, four moving, the wreck), each
// rendered orthographic at 384 px five times: a colour-ID pass, a world-normal
// pass, a mask pass (red the kit's metallic map, green a rubber flag from the
// material's name, blue the kit's emission map; U.6 follow-up of 2026-11-06,
// for rubber and lamps), a lit pass (red the unit lit by a key and a fill
// through the kit's normal map, green its albedo's luminance) and a depth pass
// (16 bits of eye depth over red and green) — the last two the shade plane's
// (the same day) — and
// `meta.json` (format 3): every frame's anchor and muzzles, each view's world
// span, the model's size and the provenance the pack records.
//
// Driven by one per-model config (`scripts/unit-bake/models/<model>.json`) and
// run by `scripts/unit-bake/bake.sh`, which hands the renders to the reducer
// (`crates/mush-pixel/examples/unit_reduce.rs`), the only resampler.
//
//   Unity -batchmode -quit -projectPath <proj> -executeMethod Bake.Run \
//     -bakeConfig <model.json> -bakeOut <dir> -bakeCommit <sha> \
//     [-bakeAssetVersion <version>]
//
// Never commit the asset packs, the Unity project or the renders (the owner's
// licensing ruling of 2026-10-29): only this script, the configs and the
// reduced pack ship.
using UnityEngine;
using UnityEditor;
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;

public static class Bake {
    const int Res = 384;
    static readonly (string name, float pitch)[] Views = { ("fp", 3f), ("iso", 28f) };
    // Yaw that turns the model's front (+Z) to each facing, the camera looking
    // along +Z: `rear` shows its back, `right` its front to the viewer's right.
    static readonly (string name, float yaw)[] Facings = {
        ("front", 180), ("front_right", 135), ("right", 90), ("rear_right", 45),
        ("rear", 0), ("rear_left", 315), ("left", 270), ("front_left", 225) };
    static readonly string[] Poses = { "still", "moving0", "moving1", "moving2", "moving3", "wreck" };

    [Serializable] public class ClipRef { public string file = ""; public string name = ""; }
    [Serializable] public class Part { public string prefab = ""; public string mount = ""; public string parent = ""; }
    [Serializable] public class MuzzleRef { public string locator = ""; public string weapon = ""; }
    [Serializable] public class MaterialPart { public string match = ""; public string part = "paint"; }
    [Serializable] public class WreckPose { public float tilt_deg = 9f; public float roll_deg = 5f; }

    [Serializable] public class Config {
        public string model = "";
        public string asset_pack = "";
        public string asset_version = "";
        public string package = "";
        public string root = "";
        public string prefab = "";
        public string primitive = "";
        public Part[] attach = new Part[0];
        public string[] enable = new string[0];
        public string[] disable = new string[0];
        public string id_texture = "";
        public bool albedo_id = false;
        public float albedo_paint_lum = 0f;
        public MaterialPart[] material_parts = new MaterialPart[0];
        public string default_part = "paint";
        // Materials (by name substring) that are rubber: tracks, tyres. The
        // mask pass flags their pixels (green), whatever the colour-ID says.
        public string[] rubber_materials = new[] { "Tracks", "Track", "Tire", "Tyre", "Rubber" };
        public ClipRef still = new ClipRef();
        public ClipRef moving = new ClipRef();
        public ClipRef wreck = new ClipRef();
        public WreckPose wreck_pose = new WreckPose();
        public MuzzleRef[] muzzles = new MuzzleRef[0];
        public int[] size_cm = new int[0];
        public float yaw_offset_deg = 0f;
        public string[] id_classes = new string[0];
    }

    public static void Run() {
        try {
            RunBake();
        } catch (Exception e) {
            Debug.LogError("BAKE FAILED: " + e.Message + "\n" + e.StackTrace);
            EditorApplication.Exit(1);
        }
    }

    static string Arg(string name) {
        var a = Environment.GetCommandLineArgs();
        for (int i = 0; i + 1 < a.Length; i++) if (a[i] == name) return a[i + 1];
        return null;
    }

    static string F(float f) => f.ToString("0.###", CultureInfo.InvariantCulture);

    static string J(string s) {
        var sb = new StringBuilder("\"");
        foreach (var c in s ?? "") {
            if (c == '"' || c == '\\') sb.Append('\\').Append(c);
            else if (c < ' ') sb.Append("\\u").Append(((int)c).ToString("x4"));
            else sb.Append(c);
        }
        return sb.Append('"').ToString();
    }

    static T Load<T>(string path) where T : UnityEngine.Object {
        var a = AssetDatabase.LoadAssetAtPath<T>(path);
        if (a == null) throw new Exception("missing asset " + path + " (is the package imported?)");
        return a;
    }

    static Transform Find(Transform t, string n) {
        if (t.name == n) return t;
        foreach (Transform c in t) { var r = Find(c, n); if (r) return r; }
        return null;
    }

    static IEnumerable<Transform> FindAll(Transform t, string[] segs, int idx = 0) {
        int next = t.name == segs[idx] ? idx + 1 : idx;
        if (next == segs.Length) yield return t;
        else foreach (Transform c in t) foreach (var r in FindAll(c, segs, next)) yield return r;
    }

    static IEnumerable<string> Mounts(Transform t) {
        if (t.name.StartsWith("Mount", StringComparison.OrdinalIgnoreCase)) yield return t.name;
        foreach (Transform c in t) foreach (var m in Mounts(c)) yield return m;
    }

    // A locator by path: `Weapon_cannon_lvl3/Barrel_end` is the first
    // `Barrel_end` under the first `Weapon_cannon_lvl3`.
    static Transform Locate(Transform root, string path) {
        Transform at = root;
        foreach (var seg in path.Split('/')) { at = at ? Find(at, seg) : null; }
        return at;
    }

    static GameObject Place(string root, string prefab, Transform parent) {
        var path = root + prefab + (prefab.EndsWith(".prefab") ? "" : ".prefab");
        var go = parent == null
            ? (GameObject)UnityEngine.Object.Instantiate(Load<GameObject>(path))
            : (GameObject)UnityEngine.Object.Instantiate(Load<GameObject>(path), parent);
        go.name = Path.GetFileNameWithoutExtension(prefab);
        if (parent != null) {
            go.transform.localPosition = Vector3.zero;
            go.transform.localRotation = Quaternion.identity;
            go.transform.localScale = Vector3.one;
        }
        return go;
    }

    static AnimationClip Clip(string root, ClipRef r) {
        if (r == null || string.IsNullOrEmpty(r.file)) return null;
        var found = new List<string>();
        foreach (var o in AssetDatabase.LoadAllAssetsAtPath(root + r.file)) {
            if (o is AnimationClip c && !c.name.StartsWith("__preview__")) {
                if (c.name == r.name || c.name.EndsWith("|" + r.name)) return c;
                found.Add(c.name);
            }
        }
        throw new Exception("no clip " + r.name + " in " + root + r.file + "; it has: " + string.Join(", ", found));
    }

    static Bounds Measure(Renderer[] rs) {
        var b = rs[0].bounds;
        foreach (var r in rs) b.Encapsulate(r.bounds);
        return b;
    }

    // Poses a frame: samples the clip at `t`, tilts it by `tilt` (a wreck
    // with no fall clip), lifts the unit by `lift` so the still pose stands on
    // the ground (a wreck is set on the ground by its own bounds, and a wreck
    // from a clip recentred over the origin, since a fall moves the hips off
    // the spot), holds a moving frame's middle over `hold` (a drive or walk
    // clip's root motion would carry the unit out of the frame), and turns it
    // to `yaw`.
    static void Pose(GameObject unit, Renderer[] rs, AnimationClip clip, float t, float yaw, float lift,
                     bool wreck, bool recentre, WreckPose tilt, Vector3? hold = null) {
        unit.transform.SetPositionAndRotation(Vector3.zero, Quaternion.identity);
        if (clip != null) clip.SampleAnimation(unit, t);
        if (tilt != null)
            unit.transform.rotation = Quaternion.Euler(tilt.tilt_deg, 0, tilt.roll_deg) * unit.transform.rotation;
        var off = unit.transform.position + Vector3.up * lift;
        if (wreck) {
            unit.transform.position = off;
            var b = Measure(rs);
            off += new Vector3(recentre ? -b.center.x : 0, -b.min.y, recentre ? -b.center.z : 0);
        } else if (hold.HasValue) {
            unit.transform.position = off;
            var b = Measure(rs);
            off += new Vector3(hold.Value.x - b.center.x, 0, hold.Value.z - b.center.z);
        }
        var rot = Quaternion.Euler(0, yaw, 0);
        unit.transform.SetPositionAndRotation(rot * off, rot * unit.transform.rotation);
    }

    // A material's saved texture, float or colour property by name, read
    // from its serialized properties so a material whose shader is missing
    // in this project still answers.
    static Texture SavedTexture(Material mat, string name) {
        var so = new SerializedObject(mat);
        var list = so.FindProperty("m_SavedProperties.m_TexEnvs");
        for (int i = 0; list != null && i < list.arraySize; i++) {
            var e = list.GetArrayElementAtIndex(i);
            if (e.FindPropertyRelative("first").stringValue == name)
                return e.FindPropertyRelative("second.m_Texture").objectReferenceValue as Texture;
        }
        return null;
    }
    static float SavedFloat(Material mat, string name, float dflt) {
        var so = new SerializedObject(mat);
        var list = so.FindProperty("m_SavedProperties.m_Floats");
        for (int i = 0; list != null && i < list.arraySize; i++) {
            var e = list.GetArrayElementAtIndex(i);
            if (e.FindPropertyRelative("first").stringValue == name) return e.FindPropertyRelative("second").floatValue;
        }
        return dflt;
    }
    static Color SavedColor(Material mat, string name, Color dflt) {
        var so = new SerializedObject(mat);
        var list = so.FindProperty("m_SavedProperties.m_Colors");
        for (int i = 0; list != null && i < list.arraySize; i++) {
            var e = list.GetArrayElementAtIndex(i);
            if (e.FindPropertyRelative("first").stringValue == name) return e.FindPropertyRelative("second").colorValue;
        }
        return dflt;
    }

    // A data texture (metallic, emission) read as data: linear, no mips,
    // uncompressed, so the mask pass samples what the file says.
    static Texture PrepareDataTexture(Texture tex) {
        var path = AssetDatabase.GetAssetPath(tex);
        var ti = AssetImporter.GetAtPath(path) as TextureImporter;
        if (ti == null) return tex;
        if (ti.sRGBTexture || ti.mipmapEnabled || ti.textureCompression != TextureImporterCompression.Uncompressed) {
            ti.sRGBTexture = false; ti.mipmapEnabled = false;
            ti.textureCompression = TextureImporterCompression.Uncompressed; ti.maxTextureSize = 8192;
            ti.SaveAndReimport();
        }
        return Load<Texture2D>(path);
    }

    static Texture PrepareAlbedoIdTexture(Texture tex) {
        var path = AssetDatabase.GetAssetPath(tex);
        var ti = AssetImporter.GetAtPath(path) as TextureImporter;
        if (ti == null) return tex;
        if (ti.sRGBTexture || ti.mipmapEnabled || ti.filterMode != FilterMode.Point || ti.textureCompression != TextureImporterCompression.Uncompressed) {
            ti.sRGBTexture = false; ti.mipmapEnabled = false; ti.filterMode = FilterMode.Point;
            ti.textureCompression = TextureImporterCompression.Uncompressed; ti.maxTextureSize = 8192;
            ti.SaveAndReimport();
        }
        return Load<Texture2D>(path);
    }

    static Texture2D PrepareIdTexture(string path) {
        var ti = (TextureImporter)AssetImporter.GetAtPath(path);
        if (ti == null) throw new Exception("missing id texture " + path);
        if (ti.mipmapEnabled || ti.filterMode != FilterMode.Point || ti.textureCompression != TextureImporterCompression.Uncompressed) {
            ti.mipmapEnabled = false; ti.filterMode = FilterMode.Point;
            ti.textureCompression = TextureImporterCompression.Uncompressed; ti.maxTextureSize = 8192;
            ti.SaveAndReimport();
        }
        return Load<Texture2D>(path);
    }

    // The colour-ID colour of a part (the reducer's default table reads it:
    // green paint, blue secondary, black dark metal, red accent, white bright).
    static Color PartColour(string part) {
        switch (part) {
            case "second": return new Color(0, 0, 1);
            case "dark": return new Color(0, 0, 0);
            case "accent": return new Color(1, 0, 0);
            case "bright": return new Color(1, 1, 1);
            case "paint": return new Color(0, 1, 0);
            default: throw new Exception("part " + part + " is not paint/second/dark/accent/bright");
        }
    }

    public static void RunBake() {
        string cfgPath = Arg("-bakeConfig") ?? throw new Exception("no -bakeConfig <model.json>");
        string outDir = Path.GetFullPath(Arg("-bakeOut") ?? "bake_out");
        string commit = Arg("-bakeCommit") ?? "";
        var cfg = JsonUtility.FromJson<Config>(File.ReadAllText(cfgPath));
        string version = string.IsNullOrEmpty(cfg.asset_version) ? (Arg("-bakeAssetVersion") ?? "") : cfg.asset_version;
        if (cfg.size_cm == null || cfg.size_cm.Length != 2 || cfg.size_cm[1] <= 0)
            throw new Exception(cfgPath + ": size_cm must be [width, height], the look's");
        Directory.CreateDirectory(outDir);
        string root = cfg.root == "" || cfg.root.EndsWith("/") ? cfg.root : cfg.root + "/";

        // 1. The unit: a prefab with its parts in their mounts, or a primitive.
        GameObject unit;
        var built = new List<string>();
        if (!string.IsNullOrEmpty(cfg.primitive)) {
            var type = (PrimitiveType)Enum.Parse(typeof(PrimitiveType), cfg.primitive);
            unit = new GameObject("Unit");
            var body = GameObject.CreatePrimitive(type);
            body.transform.SetParent(unit.transform, false);
            body.transform.localScale = new Vector3(cfg.size_cm[0] / 100f * 0.5f, cfg.size_cm[1] / 100f, cfg.size_cm[0] / 100f);
            body.transform.localPosition = new Vector3(0, cfg.size_cm[1] / 200f, 0);
            UnityEngine.Object.DestroyImmediate(body.GetComponent<Collider>());
            built.Add("primitive " + cfg.primitive);
        } else {
            unit = Place(root, cfg.prefab, null);
            built.Add(cfg.prefab);
            var placed = new Dictionary<string, GameObject> { { "", unit } };
            foreach (var p in cfg.attach) {
                if (!placed.TryGetValue(p.parent ?? "", out var parent))
                    throw new Exception("attach " + p.prefab + ": its parent '" + p.parent + "' is not placed (list parents first)");
                var mount = Find(parent.transform, p.mount);
                if (mount == null)
                    throw new Exception("attach " + p.prefab + ": no mount '" + p.mount + "' on " + parent.name +
                                        "; its mounts: " + string.Join(", ", Mounts(parent.transform).Distinct()));
                var go = Place(root, p.prefab, mount);
                placed[go.name] = go;
                built.Add(p.prefab + " on " + p.mount);
            }
            foreach (var pat in cfg.enable) {
                var hits = FindAll(unit.transform, pat.Split('/')).ToArray();
                if (hits.Length == 0) throw new Exception("enable: no child '" + pat + "' on " + unit.name);
                foreach (var h in hits) {
                    for (var p = h; p != null && p != unit.transform; p = p.parent) p.gameObject.SetActive(true);
                }
                built.Add("+" + pat);
            }
            foreach (var pat in cfg.disable) {
                var hits = FindAll(unit.transform, pat.Split('/')).ToArray();
                if (hits.Length == 0) throw new Exception("disable: no child '" + pat + "' on " + unit.name);
                foreach (var h in hits) h.gameObject.SetActive(false);
                built.Add("-" + pat);
            }
        }
        foreach (var an in unit.GetComponentsInChildren<Animator>()) an.enabled = false;
        var rs = unit.GetComponentsInChildren<Renderer>()
            .Where(r => r is MeshRenderer || r is SkinnedMeshRenderer).ToArray();
        if (rs.Length == 0) throw new Exception("the unit has nothing to render");

        // 2. The passes' materials.
        var original = rs.Select(r => r.sharedMaterials).ToArray();
        Material[][] idMats;
        if (!string.IsNullOrEmpty(cfg.id_texture)) {
            var m = new Material(Shader.Find("Hidden/IdOut")) { mainTexture = PrepareIdTexture(root + cfg.id_texture) };
            idMats = original.Select(ms => ms.Select(_ => m).ToArray()).ToArray();
        } else {
            var flat = new Dictionary<string, Material>();
            Material For(string part) {
                if (!flat.TryGetValue(part, out var m)) {
                    m = new Material(Shader.Find("Hidden/FlatId"));
                    m.SetColor("_Color", PartColour(part));
                    flat[part] = m;
                }
                return m;
            }
            var albIdOf = new Dictionary<Material, Material>();
            idMats = original.Select(ms => ms.Select(mat => {
                var name = mat ? mat.name : "";
                var hit = cfg.material_parts.FirstOrDefault(mp => name.IndexOf(mp.match, StringComparison.OrdinalIgnoreCase) >= 0);
                if (hit != null) return For(hit.part);
                if (cfg.albedo_id && mat != null) {
                    if (albIdOf.TryGetValue(mat, out var done)) return done;
                    var albedo = SavedTexture(mat, "_BaseMap") ?? SavedTexture(mat, "_MainTex");
                    if (albedo != null) {
                        var im = new Material(Shader.Find("Hidden/IdOut")) { mainTexture = PrepareAlbedoIdTexture(albedo) };
                        im.SetFloat("_AlbedoId", 1);
                        im.SetFloat("_PaintLum", cfg.albedo_paint_lum);
                        albIdOf[mat] = im;
                        return im;
                    }
                }
                return For(cfg.default_part);
            }).ToArray()).ToArray();
        }
        var nMat = new Material(Shader.Find("Hidden/NormalOut"));
        var nMats = original.Select(ms => ms.Select(_ => nMat).ToArray()).ToArray();
        // The mask pass reads each original material's own maps: metallic
        // (red; its alpha is smoothness) and emission, when the material
        // assigns them. The slots are read from the material's saved
        // properties, not through its shader: the scratch project has no
        // render pipeline the kit's shaders need, so a kit material has no
        // live shader here and `HasProperty` would say no to every slot. A
        // material without a metallic map is as metallic as its `_Metallic`
        // slider says (0 when it has none), so a kit without maps bakes as
        // before; emission counts when a map is assigned and the emission
        // colour is not black.
        var maskOf = new Dictionary<Material, Material>();
        Material MaskFor(Material mat) {
            if (mat == null) return MaskFor(new Material(Shader.Find("Standard")));
            if (maskOf.TryGetValue(mat, out var done)) return done;
            var mm = new Material(Shader.Find("Hidden/MaskOut"));
            var metalMap = SavedTexture(mat, "_MetallicGlossMap");
            if (metalMap != null) { mm.SetTexture("_MetallicGlossMap", PrepareDataTexture(metalMap)); mm.SetFloat("_HasMetalMap", 1); }
            mm.SetFloat("_Metallic", SavedFloat(mat, "_Metallic", 0));
            var rubber = cfg.rubber_materials.Any(r => mat.name.IndexOf(r, StringComparison.OrdinalIgnoreCase) >= 0);
            mm.SetFloat("_Rubber", rubber ? 1 : 0);
            var emisMap = SavedTexture(mat, "_EmissionMap");
            var ec = SavedColor(mat, "_EmissionColor", Color.black);
            if (emisMap != null && ec.maxColorComponent > 0.01f) { mm.SetTexture("_EmissionMap", PrepareDataTexture(emisMap)); mm.SetFloat("_HasEmisMap", 1); }
            mm.SetColor("_EmissionColor", ec);
            maskOf[mat] = mm;
            return mm;
        }
        var mMats = original.Select(ms => ms.Select(MaskFor).ToArray()).ToArray();
        // The lit pass lights the kit's real surface: its normal map under a
        // key light and a fill, and its albedo's luminance for the painted
        // detail; the depth pass gives the reducer occlusion and part edges.
        var litOf = new Dictionary<Material, Material>();
        Material LitFor(Material mat) {
            if (mat == null) return LitFor(new Material(Shader.Find("Standard")));
            if (litOf.TryGetValue(mat, out var done)) return done;
            var lm = new Material(Shader.Find("Hidden/LitOut"));
            var albedo = SavedTexture(mat, "_BaseMap");
            if (albedo == null) albedo = SavedTexture(mat, "_MainTex");
            if (albedo != null) { lm.SetTexture("_MainTex", albedo); lm.SetFloat("_HasAlbedo", 1); }
            var bump = SavedTexture(mat, "_BumpMap");
            if (bump != null) { lm.SetTexture("_BumpMap", bump); lm.SetFloat("_HasBump", 1); }
            litOf[mat] = lm;
            return lm;
        }
        var lMats = original.Select(ms => ms.Select(LitFor).ToArray()).ToArray();
        var dMat = new Material(Shader.Find("Hidden/DepthOut"));
        var dMats = original.Select(ms => ms.Select(_ => dMat).ToArray()).ToArray();
        // The reducer's light, world space (the camera only pitches): upper
        // left, toward the viewer; the fill low from the other side.
        Shader.SetGlobalVector("_KeyDir", new Vector4(-0.4f, 0.8f, -0.45f, 0));
        Shader.SetGlobalVector("_FillDir", new Vector4(0.6f, 0.15f, -0.5f, 0));

        // 3. The poses' clips.
        var still = Clip(root, cfg.still);
        var moving = Clip(root, cfg.moving);
        var fall = Clip(root, cfg.wreck);
        var clips = new[] { still, moving, fall }.Where(c => c != null).Select(c => c.name).ToList();
        // A pose index to (clip, time, whether a wreck, whether tilted): the
        // moving cycle from its clip (four still frames without one), the wreck
        // from its fall clip's last frame or, without one, the still tilted.
        (AnimationClip clip, float t, bool wreck, bool tilted) PoseOf(int k) {
            if (k == 0) return (still, 0f, false, false);
            if (k <= 4) return moving != null ? (moving, moving.length * (k - 1) / 4f, false, false) : (still, 0f, false, false);
            return fall != null ? (fall, fall.length, true, false) : (still, 0f, true, true);
        }

        // 4. The model to its look's height, standing on the ground.
        Pose(unit, rs, still, 0, 0, 0, false, false, null);
        var b0 = Measure(rs);
        if (b0.size.y <= 0) throw new Exception("the unit has no height");
        unit.transform.localScale = unit.transform.localScale * (cfg.size_cm[1] / 100f / b0.size.y);
        Pose(unit, rs, still, 0, 0, 0, false, false, null);
        float lift = -Measure(rs).min.y;
        Pose(unit, rs, still, 0, 0, lift, false, false, null);
        var middle = Measure(rs).center;
        Pose(unit, rs, still, 0, cfg.yaw_offset_deg, lift, false, false, null);
        var b = Measure(rs);
        float H = b.max.y;
        float Rc = 0;
        foreach (var x in new[] { b.min.x, b.max.x })
            foreach (var z in new[] { b.min.z, b.max.z })
                Rc = Mathf.Max(Rc, Mathf.Sqrt(x * x + z * z));
        int widthCm = Mathf.RoundToInt(Mathf.Max(b.size.x, b.size.z) * 100);
        int heightCm = Mathf.RoundToInt(H * 100);

        // 5. The camera and the renders, view by view.
        var cam = new GameObject("Cam").AddComponent<Camera>();
        cam.orthographic = true; cam.aspect = 1;
        cam.clearFlags = CameraClearFlags.SolidColor; cam.backgroundColor = new Color(0, 0, 0, 0);
        cam.nearClipPlane = 0.01f; cam.farClipPlane = 400;
        var rt = new RenderTexture(Res, Res, 24, RenderTextureFormat.ARGB32, RenderTextureReadWrite.Linear) { antiAliasing = 1 };
        cam.targetTexture = rt;
        var read = new Texture2D(Res, Res, TextureFormat.RGBA32, false, true);

        var views = new List<string>();
        foreach (var (vname, pitch) in Views) {
            float p = pitch * Mathf.Deg2Rad;
            // The frame spans the unit at any facing: its turning circle across,
            // its height and that circle's depth foreshortened down it.
            float S = Mathf.Max(2 * Rc, H * Mathf.Cos(p) + 2 * Rc * Mathf.Sin(p)) * 1.08f;
            cam.orthographicSize = S / 2;
            cam.transform.rotation = Quaternion.Euler(pitch, 0, 0);
            cam.transform.position = cam.transform.up * (H * Mathf.Cos(p) / 2) - cam.transform.forward * 200;
            var frames = new List<string>();
            foreach (var (fname, yaw) in Facings) {
                for (int k = 0; k < Poses.Length; k++) {
                    var (clip, t, wreck, tilted) = PoseOf(k);
                    Pose(unit, rs, clip, t, yaw + cfg.yaw_offset_deg, lift, wreck, wreck && !tilted,
                         tilted ? cfg.wreck_pose : null, k >= 1 && k <= 4 ? middle : (Vector3?)null);
                    foreach (var (tag, mats) in new[] { ("id", idMats), ("n", nMats), ("m", mMats), ("l", lMats), ("d", dMats) }) {
                        for (int i = 0; i < rs.Length; i++) rs[i].sharedMaterials = mats[i];
                        cam.Render();
                        RenderTexture.active = rt;
                        read.ReadPixels(new Rect(0, 0, Res, Res), 0, 0); read.Apply();
                        RenderTexture.active = null;
                        File.WriteAllBytes(Path.Combine(outDir, $"{vname}_{fname}_{Poses[k]}_{tag}.png"), read.EncodeToPNG());
                    }
                    var a = cam.WorldToViewportPoint(Vector3.zero);
                    var muzzles = new List<string>();
                    foreach (var mz in cfg.muzzles) {
                        var at = Locate(unit.transform, mz.locator);
                        if (!at) throw new Exception("no muzzle locator " + mz.locator);
                        var v = cam.WorldToViewportPoint(at.position);
                        muzzles.Add(J(mz.weapon) + ":[" + F(v.x * Res) + "," + F((1 - v.y) * Res) + "]");
                    }
                    frames.Add("{\"facing\":" + J(fname) + ",\"pose\":" + J(Poses[k]) +
                               ",\"anchor\":[" + F(a.x * Res) + "," + F((1 - a.y) * Res) + "]" +
                               ",\"muzzles\":{" + string.Join(",", muzzles) + "}}");
                }
            }
            views.Add("{\"view\":" + J(vname) + ",\"pitch_deg\":" + F(pitch) + ",\"span_cm\":" + F(S * 100) +
                      ",\"frames\":[" + string.Join(",", frames) + "]}");
        }
        for (int i = 0; i < rs.Length; i++) rs[i].sharedMaterials = original[i];

        var meta = new StringBuilder();
        meta.Append("{\"format\":3,\"model\":").Append(J(cfg.model)).Append(",\"res\":").Append(Res)
            .Append(",\"size_cm\":[").Append(widthCm).Append(',').Append(heightCm).Append(']')
            .Append(",\"provenance\":{\"asset_pack\":").Append(J(cfg.asset_pack))
            .Append(",\"asset_version\":").Append(J(version))
            .Append(",\"prefab\":").Append(J(string.Join(" + ", built)))
            .Append(",\"clips\":[").Append(string.Join(",", clips.Select(J))).Append(']')
            .Append(",\"bake_commit\":").Append(J(commit)).Append('}');
        if (cfg.id_classes != null && cfg.id_classes.Length > 0)
            meta.Append(",\"id_classes\":[").Append(string.Join(",", cfg.id_classes.Select(J))).Append(']');
        meta.Append(",\"views\":[").Append(string.Join(",", views)).Append("]}");
        File.WriteAllText(Path.Combine(outDir, "meta.json"), meta.ToString());
        Debug.Log("BAKE DONE " + cfg.model + ": " + heightCm + " cm tall, " + widthCm + " cm wide, " +
                  (Views.Length * Facings.Length * Poses.Length * 5) + " renders in " + outDir);
    }
}
