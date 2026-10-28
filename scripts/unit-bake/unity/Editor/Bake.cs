// Throwaway PoC: bakes one assembled light mech into raw renders (colour-ID
// and world-normal passes) for 8 facings x poses, plus voxel grids at 48 and
// 96 cells, into <project>/bake_out. Never shipped; only the reduced sheets are.
using UnityEngine;
using UnityEditor;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Globalization;

public static class Bake {
    const string Root = "Assets/Mech_Constructor_LtMed/";
    const int Res = 384;
    const float Elev = 8f;
    static readonly (string name, float yaw)[] Facings = {
        ("front", 180), ("front_right", 135), ("right", 90), ("rear_right", 45),
        ("rear", 0), ("rear_left", 315), ("left", 270), ("front_left", 225) };

    static T Load<T>(string p) where T : Object {
        var a = AssetDatabase.LoadAssetAtPath<T>(Root + p);
        if (a == null) throw new System.Exception("missing " + p);
        return a;
    }
    static AnimationClip Clip(string file, string name) {
        foreach (var o in AssetDatabase.LoadAllAssetsAtPath(Root + "Animations/" + file))
            if (o is AnimationClip c && c.name == name) return c;
        throw new System.Exception("missing clip " + name);
    }
    static Transform Find(Transform t, string n) {
        if (t.name == n) return t;
        foreach (Transform c in t) { var r = Find(c, n); if (r) return r; }
        return null;
    }
    static GameObject Attach(string prefab, Transform mount) {
        var go = (GameObject)Object.Instantiate(Load<GameObject>("Prefabs/" + prefab + ".prefab"), mount);
        go.name = prefab;
        go.transform.localPosition = Vector3.zero;
        go.transform.localRotation = Quaternion.identity;
        go.transform.localScale = Vector3.one;
        return go;
    }

    static void MakeReadable() {
        foreach (var g in AssetDatabase.FindAssets("t:Model", new[] { Root + "Models", Root + "Animations" })) {
            var p = AssetDatabase.GUIDToAssetPath(g);
            var mi = (ModelImporter)AssetImporter.GetAtPath(p);
            if (!mi.isReadable) { mi.isReadable = true; mi.SaveAndReimport(); }
        }
        var ti = (TextureImporter)AssetImporter.GetAtPath(Root + "Materials/MC_LtMed_X_Color_ID.png");
        if (!ti.isReadable || ti.mipmapEnabled || ti.textureCompression != TextureImporterCompression.Uncompressed) {
            ti.isReadable = true; ti.mipmapEnabled = false; ti.filterMode = FilterMode.Point;
            ti.textureCompression = TextureImporterCompression.Uncompressed; ti.maxTextureSize = 8192;
            ti.SaveAndReimport();
        }
    }

    // Poses a frame: samples the clip, and for the wreck recentres the body
    // on the origin and the ground (the fall moves the hips off the spot).
    static void Pose(GameObject mech, Renderer[] rs, AnimationClip clip, float t, float yaw, bool recentre) {
        mech.transform.SetPositionAndRotation(Vector3.zero, Quaternion.identity);
        clip.SampleAnimation(mech, t);
        if (recentre) {
            var b = rs[0].bounds; foreach (var r in rs) b.Encapsulate(r.bounds);
            mech.transform.position = new Vector3(-b.center.x, -b.min.y, -b.center.z);
        }
        var off = mech.transform.position;
        var rot = Quaternion.Euler(0, yaw, 0);
        mech.transform.SetPositionAndRotation(rot * off, rot);
    }

    static string F(float f) => f.ToString("0.###", CultureInfo.InvariantCulture);

    public static void Run() {
        MakeReadable();
        string outDir = Path.GetFullPath("bake_out");
        Directory.CreateDirectory(outDir);
        var idTex = Load<Texture2D>("Materials/MC_LtMed_X_Color_ID.png");
        var idMat = new Material(Shader.Find("Hidden/IdOut")); idMat.mainTexture = idTex;
        var nMat = new Material(Shader.Find("Hidden/NormalOut"));

        // Asymmetric light mech: cannon right, machine gun left, rockets on top.
        var mech = (GameObject)Object.Instantiate(Load<GameObject>("Prefabs/Legs_Lt.prefab"));
        mech.name = "Mech";
        var cockpit = Attach("Cockpit_fox", Find(mech.transform, "Mount_top"));
        Attach("Weapon_cannon_lvl3", Find(cockpit.transform, "Mount_Weapon_R"));
        Attach("Weapon_machinegun_lvl3", Find(cockpit.transform, "Mount_Weapon_L"));
        Attach("Weapon_rockets_shoulder_lvl1", Find(cockpit.transform, "Mount_Weapon_top"));
        foreach (var an in mech.GetComponentsInChildren<Animator>()) an.enabled = false;
        var renderers = mech.GetComponentsInChildren<Renderer>();

        var idle = Clip("Legs_Lt@Legs_lt_idle.fbx", "Legs_lt_idle_simple");
        var walk = Clip("Legs_Lt@Legs_lt_walk.fbx", "Legs_lt_walk");
        var fall = Clip("Legs_Lt@Legs_lt_fall_back.fbx", "Legs_lt_fall_back");
        var poses = new List<(string name, AnimationClip clip, float t)> { ("still", idle, 0) };
        for (int k = 0; k < 4; k++) poses.Add(("moving" + k, walk, walk.length * k / 4f));
        poses.Add(("wreck", fall, fall.length));

        // Framing from the still pose: span S covers height and turning radius.
        idle.SampleAnimation(mech, 0);
        var b = renderers[0].bounds; foreach (var r in renderers) b.Encapsulate(r.bounds);
        float H = b.max.y;
        float R = Mathf.Max(Mathf.Abs(b.min.x), Mathf.Abs(b.max.x), Mathf.Abs(b.min.z), Mathf.Abs(b.max.z));
        float S = Mathf.Max(H, 2 * R) * 1.1f;

        var camGo = new GameObject("Cam"); var cam = camGo.AddComponent<Camera>();
        cam.orthographic = true; cam.orthographicSize = S / 2; cam.aspect = 1;
        cam.clearFlags = CameraClearFlags.SolidColor; cam.backgroundColor = new Color(0, 0, 0, 0);
        cam.nearClipPlane = 0.01f; cam.farClipPlane = 200;
        cam.transform.rotation = Quaternion.Euler(Elev, 0, 0);
        cam.transform.position = new Vector3(0, S * 0.45f, 0) - cam.transform.forward * 50;
        var rt = new RenderTexture(Res, Res, 24, RenderTextureFormat.ARGB32, RenderTextureReadWrite.Linear);
        rt.antiAliasing = 1; cam.targetTexture = rt;
        var read = new Texture2D(Res, Res, TextureFormat.RGBA32, false, true);

        var meta = new StringBuilder();
        meta.Append("{\"res\":").Append(Res).Append(",\"span_m\":").Append(F(S))
            .Append(",\"height_m\":").Append(F(H)).Append(",\"elev_deg\":").Append(F(Elev))
            .Append(",\"frames\":[");
        bool first = true;
        foreach (var (fname, yaw) in Facings) {
            foreach (var (pname, clip, t) in poses) {
                Pose(mech, renderers, clip, t, yaw, pname == "wreck");
                foreach (var (tag, mat) in new[] { ("id", idMat), ("n", nMat) }) {
                    foreach (var r in renderers) {
                        var m = new Material[r.sharedMaterials.Length];
                        for (int i = 0; i < m.Length; i++) m[i] = mat;
                        r.sharedMaterials = m;
                    }
                    cam.Render();
                    RenderTexture.active = rt; read.ReadPixels(new Rect(0, 0, Res, Res), 0, 0); read.Apply(); RenderTexture.active = null;
                    File.WriteAllBytes(Path.Combine(outDir, $"{fname}_{pname}_{tag}.png"), read.EncodeToPNG());
                }
                var anchor = cam.WorldToViewportPoint(Vector3.zero);
                meta.Append(first ? "" : ",").Append("{\"facing\":\"").Append(fname).Append("\",\"pose\":\"").Append(pname)
                    .Append("\",\"anchor\":[").Append(F(anchor.x * Res)).Append(',').Append(F((1 - anchor.y) * Res)).Append("],\"muzzles\":{");
                first = false;
                bool mf = true;
                foreach (var w in new[] { "Weapon_cannon_lvl3", "Weapon_machinegun_lvl3", "Weapon_rockets_shoulder_lvl1" }) {
                    var wt = Find(mech.transform, w);
                    var bt = wt ? Find(wt, "Barrel_end") : null;
                    if (!bt) continue;
                    var v = cam.WorldToViewportPoint(bt.position);
                    meta.Append(mf ? "" : ",").Append('"').Append(w).Append("\":[").Append(F(v.x * Res)).Append(',')
                        .Append(F((1 - v.y) * Res)).Append(',').Append(F(v.z)).Append(']');
                    mf = false;
                }
                meta.Append("}}");
            }
        }
        meta.Append("]}");
        File.WriteAllText(Path.Combine(outDir, "meta.json"), meta.ToString());

        // Voxels: mech at yaw 0 (front toward +Z), grid x,z in [-S/2,S/2], y in [0,S].
        // Layout: index (y * N + z) * N + x; value 0 empty, else 1 + colour-ID bits (r4 g2 b1).
        Physics.queriesHitBackfaces = true;
        var idPixels = idTex.GetPixels32(); int tw = idTex.width, th = idTex.height;
        foreach (int N in new[] { 48, 96 }) {
            foreach (var (pname, clip, t) in poses) {
                Pose(mech, renderers, clip, t, 0, pname == "wreck");
                var cols = new List<GameObject>();
                foreach (var r in renderers) {
                    Mesh mesh;
                    var cg = new GameObject("col");
                    cg.transform.SetPositionAndRotation(r.transform.position, r.transform.rotation);
                    if (r is SkinnedMeshRenderer sk) { mesh = new Mesh(); sk.BakeMesh(mesh, true); cg.transform.localScale = Vector3.one; }
                    else { mesh = r.GetComponent<MeshFilter>().sharedMesh; cg.transform.localScale = r.transform.lossyScale; }
                    cg.AddComponent<MeshCollider>().sharedMesh = mesh;
                    cols.Add(cg);
                }
                Physics.SyncTransforms();
                float v = S / N;
                var grid = new byte[N * N * N];
                Vector3 origin = new Vector3(-S / 2, 0, -S / 2);
                // Every crossing of every part along each axis ray (Unity's
                // RaycastAll gives one hit per collider, so march each part's
                // collider past each hit). A crossing's cell takes its class
                // (the surface); per part, cells between an entering and a
                // leaving crossing are inside (parts interpenetrate, so parity
                // is per part), and a cell two axes of three call inside is
                // filled with the entering crossing's class.
                var votes = new byte[N * N * N];
                var fill = new byte[N * N * N];
                var surf = new byte[N * N * N];
                var hitsBuf = new List<(float d, byte cls)>();
                foreach (var cg in cols) {
                    var mc = cg.GetComponent<MeshCollider>();
                    System.Array.Clear(votes, 0, votes.Length);
                    System.Array.Clear(fill, 0, fill.Length);
                    for (int axis = 0; axis < 3; axis++)
                        for (int i = 0; i < N; i++)
                            for (int j = 0; j < N; j++) {
                                float a = (i + 0.5f) * v, c = (j + 0.5f) * v;
                                Vector3 p0, dir;
                                if (axis == 0) { p0 = origin + new Vector3(-1, a, c); dir = Vector3.right; }
                                else if (axis == 1) { p0 = origin + new Vector3(a, -1, c); dir = Vector3.up; }
                                else { p0 = origin + new Vector3(a, c, -1); dir = Vector3.forward; }
                                hitsBuf.Clear();
                                float travelled = 0;
                                while (travelled < S + 2 && hitsBuf.Count < 64) {
                                    if (!mc.Raycast(new Ray(p0 + dir * travelled, dir), out var h, S + 2 - travelled)) break;
                                    travelled += h.distance;
                                    var uv = h.textureCoord;
                                    int px = Mathf.Clamp((int)(uv.x * tw), 0, tw - 1), py = Mathf.Clamp((int)(uv.y * th), 0, th - 1);
                                    var col = idPixels[py * tw + px];
                                    hitsBuf.Add((travelled, (byte)(1 + (col.r > 127 ? 4 : 0) + (col.g > 127 ? 2 : 0) + (col.b > 127 ? 1 : 0))));
                                    travelled += 1e-4f;
                                }
                                foreach (var (d, cls) in hitsBuf) {
                                    int kk = Mathf.FloorToInt((d - 1) / v);
                                    if (kk < 0 || kk >= N) continue;
                                    int x, y, z;
                                    if (axis == 0) { x = kk; y = i; z = j; } else if (axis == 1) { x = i; y = kk; z = j; } else { x = i; y = j; z = kk; }
                                    surf[(y * N + z) * N + x] = cls;
                                }
                                for (int hI = 0; hI + 1 < hitsBuf.Count; hI += 2) {
                                    int k0 = Mathf.CeilToInt((hitsBuf[hI].d - 1) / v - 0.5f);
                                    int k1 = Mathf.FloorToInt((hitsBuf[hI + 1].d - 1) / v - 0.5f);
                                    for (int kk = Mathf.Max(k0, 0); kk <= Mathf.Min(k1, N - 1); kk++) {
                                        int x, y, z;
                                        if (axis == 0) { x = kk; y = i; z = j; } else if (axis == 1) { x = i; y = kk; z = j; } else { x = i; y = j; z = kk; }
                                        int idx = (y * N + z) * N + x;
                                        votes[idx]++;
                                        if (fill[idx] == 0) fill[idx] = hitsBuf[hI].cls;
                                    }
                                }
                            }
                    for (int idx = 0; idx < grid.Length; idx++)
                        if (votes[idx] >= 2 && grid[idx] == 0) grid[idx] = fill[idx];
                }
                for (int idx = 0; idx < grid.Length; idx++)
                    if (surf[idx] != 0) grid[idx] = surf[idx];
                File.WriteAllBytes(Path.Combine(outDir, $"vox{N}_{pname}.bin"), grid);
                foreach (var cg in cols) Object.DestroyImmediate(cg);
            }
        }
        Debug.Log("BAKE DONE S=" + S + " H=" + H);
    }
}
