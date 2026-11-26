using System;
using System.Collections.Generic;
using TacSim.Core;
using TacSim.World;
using UnityEngine;

namespace TacSim.Arena
{
    public struct ArenaEnvironmentPropSummary
    {
        public int TreeCount;
        public int RockCount;
        public int WallCount;
        public int WaterCellCount;
    }

    public struct ArenaBiomeLightingProfile
    {
        public string BiomeKey;
        public Color SunColor;
        public float SunIntensity;
        public Color AmbientColor;
        public Color FogColor;
        public float FogDensity;
        public bool EnableFog;
    }

    public struct ArenaTerrainTacticalSample
    {
        public TerrainCellClass OwnCellClass;
        public string CoverLabel;           // "HEAVY WOODS" | "WOODS COVER" | "WALL COVER" | "ROCK COVER" | "ROAD" | "WATER" | "OPEN"
        public bool HasCover;
        public float OwnElevationM;
        public bool HasTargetSample;
        public float ElevationDeltaM;       // ownElev - targetElev (+ = high ground)
        public string ElevationAdvantage;   // "HIGH GROUND" | "LOW GROUND" | "LEVEL"
        public bool IsLineOfSightObstructed;
        public string LineOfSightStatus;    // "CLEAR" | "RIDGE" | "WOODS" | "WALL"
    }

    /// <summary>
    /// Builds ultra-fast batched 3D battlefield cover geometry (pine/canopy woods, craggy rock
    /// boulders, ferrocrete bunker walls, and translucent water surfaces) directly from the
    /// authoritative <see cref="DecodedTerrainGrid"/> in 4 static draw calls with zero per-prop
    /// GameObject churn.
    /// </summary>
    public static class TacSimArenaEnvironmentBuilder
    {
        public const int MaxTreeInstances = 900;
        public const int MaxRockInstances = 650;
        public const int MaxWallInstances = 420;
        public const int MaxWaterQuads = 4096;

        private sealed class MeshBuilder
        {
            public readonly List<Vector3> Verts = new List<Vector3>(2048);
            public readonly List<int> Tris = new List<int>(4096);
            public readonly List<Color> Colors = new List<Color>(2048);

            public void AddQuad(Vector3 a, Vector3 b, Vector3 c, Vector3 d, Color col)
            {
                int i0 = Verts.Count;
                Verts.Add(a); Verts.Add(b); Verts.Add(c); Verts.Add(d);
                Colors.Add(col); Colors.Add(col); Colors.Add(col); Colors.Add(col);
                Tris.Add(i0); Tris.Add(i0 + 1); Tris.Add(i0 + 2);
                Tris.Add(i0); Tris.Add(i0 + 2); Tris.Add(i0 + 3);
            }

            public void AddTri(Vector3 a, Vector3 b, Vector3 c, Color col)
            {
                int i0 = Verts.Count;
                Verts.Add(a); Verts.Add(b); Verts.Add(c);
                Colors.Add(col); Colors.Add(col); Colors.Add(col);
                Tris.Add(i0); Tris.Add(i0 + 1); Tris.Add(i0 + 2);
            }

            public void AddPrism(Vector3 center, Vector3 halfExtents, float yawRad, Color col)
            {
                float cs = Mathf.Cos(yawRad);
                float sn = Mathf.Sin(yawRad);
                Vector3 Local(float lx, float ly, float lz)
                {
                    return new Vector3(
                        center.x + lx * cs - lz * sn,
                        center.y + ly,
                        center.z + lx * sn + lz * cs);
                }

                float hx = halfExtents.x, hy = halfExtents.y, hz = halfExtents.z;
                Vector3 b0 = Local(-hx, -hy, -hz), b1 = Local(hx, -hy, -hz), b2 = Local(hx, -hy, hz), b3 = Local(-hx, -hy, hz);
                Vector3 t0 = Local(-hx, hy, -hz), t1 = Local(hx, hy, -hz), t2 = Local(hx, hy, hz), t3 = Local(-hx, hy, hz);

                AddQuad(t0, t3, t2, t1, col);
                AddQuad(b0, t0, t1, b1, col * 0.90f);
                AddQuad(b1, t1, t2, b2, col * 0.86f);
                AddQuad(b2, t2, t3, b3, col * 0.92f);
                AddQuad(b3, t3, t0, b0, col * 0.84f);
            }

            public void AddPyramid(Vector3 baseCenter, float radius, float height, int sides, float yawRad, Color col)
            {
                Vector3 apex = baseCenter + new Vector3(0f, height, 0f);
                for (int i = 0; i < sides; i++)
                {
                    float a0 = yawRad + (i / (float)sides) * Mathf.PI * 2f;
                    float a1 = yawRad + ((i + 1) / (float)sides) * Mathf.PI * 2f;
                    Vector3 p0 = baseCenter + new Vector3(Mathf.Cos(a0) * radius, 0f, Mathf.Sin(a0) * radius);
                    Vector3 p1 = baseCenter + new Vector3(Mathf.Cos(a1) * radius, 0f, Mathf.Sin(a1) * radius);
                    AddTri(p0, apex, p1, col * (0.88f + 0.14f * ((i % 2 == 0) ? 1f : 0f)));
                }
            }

            public GameObject FinalizeToChild(Transform parent, string name, Material mat)
            {
                if (Verts.Count == 0 || Tris.Count == 0) return null;
                var mesh = new Mesh { name = name };
                if (Verts.Count > 60000)
                    mesh.indexFormat = UnityEngine.Rendering.IndexFormat.UInt32;
                mesh.SetVertices(Verts);
                mesh.SetColors(Colors);
                mesh.SetTriangles(Tris, 0);
                mesh.RecalculateNormals();
                mesh.RecalculateBounds();

                var go = new GameObject(name);
                go.transform.SetParent(parent, false);
                var mf = go.AddComponent<MeshFilter>();
                mf.sharedMesh = mesh;
                var mr = go.AddComponent<MeshRenderer>();
                mr.sharedMaterial = mat;
                return go;
            }
        }

        private static float Hash01(int x, int y, int salt)
        {
            uint h = (uint)(x * 374761393 + y * 668265263 + salt * 1442695041);
            h = (h ^ (h >> 13)) * 1274126177u;
            return ((h ^ (h >> 16)) & 0xFFFFu) / 65535f;
        }

        public static ArenaBiomeLightingProfile ResolveBiomeLighting(string biome)
        {
            string b = (biome ?? "temperate").Trim().ToLowerInvariant();
            if (b.Contains("desert") || b.Contains("mars"))
            {
                return new ArenaBiomeLightingProfile
                {
                    BiomeKey = "desert",
                    SunColor = new Color(1.0f, 0.88f, 0.72f),
                    SunIntensity = 2.4f,
                    AmbientColor = new Color(0.46f, 0.36f, 0.28f),
                    FogColor = new Color(0.56f, 0.43f, 0.30f),
                    FogDensity = 0.00055f,
                    EnableFog = true
                };
            }
            if (b.Contains("arctic") || b.Contains("winter") || b.Contains("ice"))
            {
                return new ArenaBiomeLightingProfile
                {
                    BiomeKey = "arctic",
                    SunColor = new Color(0.86f, 0.94f, 1.0f),
                    SunIntensity = 2.3f,
                    AmbientColor = new Color(0.36f, 0.44f, 0.54f),
                    FogColor = new Color(0.58f, 0.68f, 0.78f),
                    FogDensity = 0.00065f,
                    EnableFog = true
                };
            }
            if (b.Contains("regolith") || b.Contains("ceres") || b.Contains("vesta") || b.Contains("lunar"))
            {
                return new ArenaBiomeLightingProfile
                {
                    BiomeKey = "regolith",
                    SunColor = new Color(0.98f, 0.98f, 1.0f),
                    SunIntensity = 2.6f,
                    AmbientColor = new Color(0.18f, 0.20f, 0.24f),
                    FogColor = new Color(0.05f, 0.06f, 0.09f),
                    FogDensity = 0.00015f,
                    EnableFog = false
                };
            }
            return new ArenaBiomeLightingProfile
            {
                BiomeKey = "temperate",
                SunColor = new Color(0.94f, 0.97f, 1.0f),
                SunIntensity = 2.2f,
                AmbientColor = new Color(0.32f, 0.38f, 0.42f),
                FogColor = new Color(0.42f, 0.52f, 0.60f),
                FogDensity = 0.00042f,
                EnableFog = true
            };
        }

        public static ArenaTerrainTacticalSample EvaluateUnitTerrainContext(
            DecodedTerrainGrid grid,
            float ownLocalX,
            float ownLocalZ,
            float sensorHeightM,
            bool hasTarget,
            float targetLocalX,
            float targetLocalZ)
        {
            var res = new ArenaTerrainTacticalSample
            {
                OwnCellClass = TerrainCellClass.Ground,
                CoverLabel = "OPEN",
                ElevationAdvantage = "LEVEL",
                LineOfSightStatus = "CLEAR"
            };
            if (grid == null || grid.Width <= 0 || grid.Height <= 0)
                return res;

            float cellSize = Mathf.Max(0.5f, grid.CellSizeM);
            int gx = Mathf.Clamp(Mathf.FloorToInt(ownLocalX / cellSize), 0, grid.Width - 1);
            int gy = Mathf.Clamp(Mathf.FloorToInt(ownLocalZ / cellSize), 0, grid.Height - 1);
            TerrainCellClass cls = grid.GetCellClass(gx, gy);
            res.OwnCellClass = cls;
            res.OwnElevationM = grid.SampleElevationMeters(ownLocalX, ownLocalZ);

            switch (cls)
            {
                case TerrainCellClass.WoodsHeavy:
                    res.CoverLabel = "HEAVY WOODS";
                    res.HasCover = true;
                    break;
                case TerrainCellClass.WoodsLight:
                    res.CoverLabel = "WOODS COVER";
                    res.HasCover = true;
                    break;
                case TerrainCellClass.Wall:
                case TerrainCellClass.Door:
                case TerrainCellClass.Platform:
                    res.CoverLabel = "WALL COVER";
                    res.HasCover = true;
                    break;
                case TerrainCellClass.Rock:
                case TerrainCellClass.Rubble:
                    res.CoverLabel = "ROCK COVER";
                    res.HasCover = true;
                    break;
                case TerrainCellClass.Road:
                case TerrainCellClass.Paved:
                    res.CoverLabel = "ROAD";
                    res.HasCover = false;
                    break;
                case TerrainCellClass.Water:
                case TerrainCellClass.WaterDeep:
                    res.CoverLabel = "WATER";
                    res.HasCover = false;
                    break;
                default:
                    res.CoverLabel = "OPEN";
                    res.HasCover = false;
                    break;
            }

            if (!hasTarget)
                return res;

            res.HasTargetSample = true;
            float tgtElev = grid.SampleElevationMeters(targetLocalX, targetLocalZ);
            res.ElevationDeltaM = res.OwnElevationM - tgtElev;
            if (res.ElevationDeltaM >= 3.0f) res.ElevationAdvantage = "HIGH GROUND";
            else if (res.ElevationDeltaM <= -3.0f) res.ElevationAdvantage = "LOW GROUND";
            else res.ElevationAdvantage = "LEVEL";

            // 12-step sightline probe from own sensor eye height to target hull center (+2.2m)
            float startY = res.OwnElevationM + Mathf.Clamp(sensorHeightM, 1.2f, 8.0f);
            float endY = tgtElev + 2.2f;
            const int Steps = 12;
            int heavyWoodsCount = 0;
            for (int s = 1; s < Steps; s++)
            {
                float u = s / (float)Steps;
                float sx = Mathf.Lerp(ownLocalX, targetLocalX, u);
                float sz = Mathf.Lerp(ownLocalZ, targetLocalZ, u);
                float rayY = Mathf.Lerp(startY, endY, u);
                float groundY = grid.SampleElevationMeters(sx, sz);
                if (groundY > rayY + 0.6f)
                {
                    res.IsLineOfSightObstructed = true;
                    res.LineOfSightStatus = "RIDGE";
                    return res;
                }
                int cgx = Mathf.Clamp(Mathf.FloorToInt(sx / cellSize), 0, grid.Width - 1);
                int cgy = Mathf.Clamp(Mathf.FloorToInt(sz / cellSize), 0, grid.Height - 1);
                TerrainCellClass midCls = grid.GetCellClass(cgx, cgy);
                if (midCls == TerrainCellClass.Wall && groundY + 2.8f > rayY)
                {
                    res.IsLineOfSightObstructed = true;
                    res.LineOfSightStatus = "WALL";
                    return res;
                }
                if (midCls == TerrainCellClass.WoodsHeavy)
                    heavyWoodsCount++;
            }

            if (heavyWoodsCount >= 2)
            {
                res.IsLineOfSightObstructed = true;
                res.LineOfSightStatus = "WOODS";
            }

            return res;
        }

        public static ArenaEnvironmentPropSummary BuildArenaProps(DecodedTerrainGrid grid, Transform propsRoot)
        {
            var summary = new ArenaEnvironmentPropSummary();
            if (grid == null || propsRoot == null || grid.Classes == null || grid.Width <= 0 || grid.Height <= 0)
                return summary;

            DestroyEnvironmentProps(propsRoot);

            string biome = (grid.Biome ?? "desert").ToLowerInvariant();
            bool isDesert = biome.Contains("desert") || biome.Contains("mars");
            bool isArctic = biome.Contains("arctic") || biome.Contains("winter");

            Color canopyBase = isArctic ? new Color(0.46f, 0.58f, 0.54f)
                             : isDesert ? new Color(0.36f, 0.42f, 0.24f)
                             : new Color(0.16f, 0.36f, 0.19f);
            Color trunkBase = new Color(0.28f, 0.20f, 0.14f);
            Color rockBase = isDesert ? new Color(0.50f, 0.29f, 0.21f)
                           : isArctic ? new Color(0.56f, 0.60f, 0.66f)
                           : new Color(0.36f, 0.38f, 0.42f);
            Color wallBase = new Color(0.32f, 0.35f, 0.39f);

            var trunks = new MeshBuilder();
            var canopies = new MeshBuilder();
            var rocks = new MeshBuilder();
            var walls = new MeshBuilder();
            var water = new MeshBuilder();

            // Stride automatically if a huge 256x256 map has thousands of wooded cells so we get even coverage across the entire battlefield!
            int totalWoods = 0, totalRocks = 0, totalWalls = 0, totalWater = 0;
            for (int i = 0; i < grid.Classes.Length; i++)
            {
                var c = grid.Classes[i];
                if (c == TerrainCellClass.WoodsLight || c == TerrainCellClass.WoodsHeavy) totalWoods++;
                else if (c == TerrainCellClass.Rock || c == TerrainCellClass.Rubble) totalRocks++;
                else if (c == TerrainCellClass.Wall) totalWalls++;
                else if (c == TerrainCellClass.Water || c == TerrainCellClass.WaterDeep) totalWater++;
            }

            float woodKeepProb = totalWoods > 0 ? Mathf.Clamp01(MaxTreeInstances / (totalWoods * 1.4f)) : 1f;
            float rockKeepProb = totalRocks > 0 ? Mathf.Clamp01(MaxRockInstances / (float)totalRocks) : 1f;
            float wallKeepProb = totalWalls > 0 ? Mathf.Clamp01(MaxWallInstances / (float)totalWalls) : 1f;
            float waterKeepProb = totalWater > 0 ? Mathf.Clamp01(MaxWaterQuads / (float)totalWater) : 1f;
            float cell = Mathf.Max(2f, grid.CellSizeM);

            for (int cy = 0; cy < grid.Height; cy++)
            {
                for (int cx = 0; cx < grid.Width; cx++)
                {
                    var cls = grid.Classes[cy * grid.Width + cx];
                    if (cls == TerrainCellClass.Ground || cls == TerrainCellClass.Grass ||
                        cls == TerrainCellClass.Sand || cls == TerrainCellClass.Road ||
                        cls == TerrainCellClass.Paved || cls == TerrainCellClass.Regolith)
                        continue;

                    float baseX = (cx + 0.5f) * cell;
                    float baseZ = (cy + 0.5f) * cell;

                    if (cls == TerrainCellClass.WoodsLight || cls == TerrainCellClass.WoodsHeavy)
                    {
                        if (summary.TreeCount >= MaxTreeInstances) continue;
                        if (Hash01(cx, cy, 11) > woodKeepProb) continue;

                        int perCell = cls == TerrainCellClass.WoodsHeavy ? 2 : 1;
                        for (int t = 0; t < perCell && summary.TreeCount < MaxTreeInstances; t++)
                        {
                            float ox = (Hash01(cx, cy, 20 + t * 3) - 0.5f) * cell * 0.72f;
                            float oz = (Hash01(cx, cy, 21 + t * 3) - 0.5f) * cell * 0.72f;
                            float tx = baseX + ox;
                            float tz = baseZ + oz;
                            float ty = grid.SampleElevationMeters(tx, tz) - 0.35f;
                            float scale = (cls == TerrainCellClass.WoodsHeavy ? 1.15f : 0.88f) * (0.82f + Hash01(cx, cy, 22 + t * 3) * 0.42f);
                            float yaw = Hash01(cx, cy, 23 + t * 3) * Mathf.PI * 2f;

                            float trunkH = 2.4f * scale;
                            trunks.AddPrism(new Vector3(tx, ty + trunkH * 0.5f, tz), new Vector3(0.28f * scale, trunkH * 0.5f, 0.28f * scale), yaw, trunkBase);
                            canopies.AddPyramid(new Vector3(tx, ty + trunkH * 0.65f, tz), 2.1f * scale, 4.6f * scale, 5, yaw, canopyBase);
                            canopies.AddPyramid(new Vector3(tx, ty + trunkH * 1.45f, tz), 1.45f * scale, 3.6f * scale, 5, yaw + 0.6f, canopyBase * 1.12f);
                            summary.TreeCount++;
                        }
                    }
                    else if (cls == TerrainCellClass.Rock || cls == TerrainCellClass.Rubble)
                    {
                        if (summary.RockCount >= MaxRockInstances) continue;
                        if (Hash01(cx, cy, 31) > rockKeepProb) continue;

                        float ox = (Hash01(cx, cy, 32) - 0.5f) * cell * 0.55f;
                        float oz = (Hash01(cx, cy, 33) - 0.5f) * cell * 0.55f;
                        float rx = baseX + ox;
                        float rz = baseZ + oz;
                        float ry = grid.SampleElevationMeters(rx, rz) - 0.40f;
                        float s = (cls == TerrainCellClass.Rock ? 1.35f : 0.85f) * (0.75f + Hash01(cx, cy, 34) * 0.55f);
                        float yaw = Hash01(cx, cy, 35) * Mathf.PI * 2f;

                        rocks.AddPyramid(new Vector3(rx, ry, rz), 2.2f * s, 2.5f * s, 5, yaw, rockBase);
                        summary.RockCount++;
                    }
                    else if (cls == TerrainCellClass.Wall)
                    {
                        if (summary.WallCount >= MaxWallInstances) continue;
                        if (Hash01(cx, cy, 41) > wallKeepProb) continue;

                        float wy = grid.SampleElevationMeters(baseX, baseZ) - 0.25f;
                        bool alignX = (cx > 0 && grid.GetCellClass(cx - 1, cy) == TerrainCellClass.Wall) ||
                                      (cx + 1 < grid.Width && grid.GetCellClass(cx + 1, cy) == TerrainCellClass.Wall);
                        Vector3 half = alignX
                            ? new Vector3(cell * 0.48f, 1.45f, Mathf.Min(1.2f, cell * 0.16f))
                            : new Vector3(Mathf.Min(1.2f, cell * 0.16f), 1.45f, cell * 0.48f);
                        walls.AddPrism(new Vector3(baseX, wy + half.y, baseZ), half, 0f, wallBase);
                        summary.WallCount++;
                    }
                    else if (cls == TerrainCellClass.Water || cls == TerrainCellClass.WaterDeep)
                    {
                        if (summary.WaterCellCount >= MaxWaterQuads) continue;
                        if (Hash01(cx, cy, 51) > waterKeepProb) continue;

                        float wy = grid.SampleElevationMeters(baseX, baseZ) + (cls == TerrainCellClass.WaterDeep ? 0.42f : 0.22f);
                        float h = cell * 0.52f;
                        Color wCol = cls == TerrainCellClass.WaterDeep
                            ? new Color(0.08f, 0.28f, 0.48f, 0.72f)
                            : new Color(0.14f, 0.42f, 0.62f, 0.56f);
                        water.AddQuad(
                            new Vector3(baseX - h, wy, baseZ - h),
                            new Vector3(baseX - h, wy, baseZ + h),
                            new Vector3(baseX + h, wy, baseZ + h),
                            new Vector3(baseX + h, wy, baseZ - h),
                            wCol);
                        summary.WaterCellCount++;
                    }
                }
            }

            trunks.FinalizeToChild(propsRoot, "ArenaWoodsTrunksBatch", UrpMaterialFactory.GetOpaque("ArenaTreeTrunk", trunkBase, 0.12f, 0.04f));
            canopies.FinalizeToChild(propsRoot, "ArenaWoodsCanopyBatch", UrpMaterialFactory.GetOpaque("ArenaTreeCanopy", canopyBase, 0.18f, 0.02f));
            rocks.FinalizeToChild(propsRoot, "ArenaRockCoverBatch", UrpMaterialFactory.GetOpaque("ArenaRockCover", rockBase, 0.20f, 0.08f));
            walls.FinalizeToChild(propsRoot, "ArenaBunkerWallBatch", UrpMaterialFactory.GetOpaque("ArenaBunkerWall", wallBase, 0.28f, 0.35f));
            water.FinalizeToChild(propsRoot, "ArenaWaterSurfaceBatch", UrpMaterialFactory.GetArmourglass("ArenaWaterSurface", new Color(0.12f, 0.40f, 0.62f, 0.54f)));

            return summary;
        }

        public static void DestroyEnvironmentProps(Transform propsRoot)
        {
            if (propsRoot == null) return;
            for (int i = propsRoot.childCount - 1; i >= 0; i--)
            {
                var ch = propsRoot.GetChild(i).gameObject;
                var mf = ch.GetComponent<MeshFilter>();
                if (mf != null && mf.sharedMesh != null)
                {
                    if (Application.isPlaying) UnityEngine.Object.Destroy(mf.sharedMesh);
                    else UnityEngine.Object.DestroyImmediate(mf.sharedMesh);
                    mf.sharedMesh = null;
                }
                if (Application.isPlaying) UnityEngine.Object.Destroy(ch);
                else UnityEngine.Object.DestroyImmediate(ch);
            }
        }
    }
}
