using System;
using System.Collections.Generic;
using System.IO;
using TacSim.Core;
using UnityEngine;

namespace TacSim.World
{
    [Serializable]
    public sealed class AtlasFloorDef
    {
        public int Index;
        public string Name;
        public float ZOffsetM;
        public float HeightM;
    }

    [Serializable]
    public sealed class AtlasRoomDef
    {
        public string EntryId; // e.g. "room.c03_arrivals"
        public string Name;
        public string ShortDesc;
        public string Desc;
        public string Kind;
        public bool Restricted;
        public int FloorIndex;
        public float MinX;
        public float MinY;
        public float MaxX;
        public float MaxY;
        [NonSerialized]
        public Dictionary<string, string> Details = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        public List<string> Markers = new List<string>();

        public Vector2 Center2D => new Vector2((MinX + MaxX) * 0.5f, (MinY + MaxY) * 0.5f);
        public Vector2 Size2D => new Vector2(Mathf.Max(2f, MaxX - MinX), Mathf.Max(2f, MaxY - MinY));
    }

    [Serializable]
    public sealed class AtlasExitDef
    {
        public string EntryId;
        public string FromRoom;
        public string ToRoom;
        public string ForwardLabel;
        public string ForwardCommand;
        public string ReverseLabel;
        public string ReverseCommand;
        public string Mode; // "open", "door", "stair", "ladder", "hatch"
        public bool Locked;
        public bool Hidden;
        public float DzM;
        public bool HasExplicitPortal;
        public Vector3 PortalA;
        public Vector3 PortalB;
    }

    [Serializable]
    public sealed class AtlasFeatureDef
    {
        public string EntryId;
        public string FeatureKind; // "wall" | "counter"
        public string RoomId;
        public int FloorIndex;
        public Vector2 PointA;
        public Vector2 PointB;
        public float HeightM;
        public bool BlocksMovement;
    }

    [Serializable]
    public sealed class AtlasNpcSeedDef
    {
        public string EntryId;
        public string Name;
        public string RoomId;
        public string ShortDesc;
        public bool Enabled = true;
        public bool IsMale;
    }

    [Serializable]
    public sealed class AtlasItemSeedDef
    {
        public string EntryId;
        public string Name;
        public string RoomId;
        public string Desc;
        public bool IsInterior;
    }

    [Serializable]
    public sealed class GoodItemDef
    {
        public string Id;           // e.g. "volatiles", "metals", "propellant", "provisions"
        public string Label;        // e.g. "water ice", "metal concentrate", "reaction mass"
        public string GoodClass;    // "trade", "ship_supply", "part", "kit"
        public string LedgerUnit;   // "t", "crew_day", "round", "box", "each"
        public int MassKgPerUnit;   // e.g. 1000 kg/t, 4 kg/crew_day
        public int VolumeLPerUnit;  // e.g. 1400 L/t, 400 L/t, 10 L/crew_day
        public string Stowage;      // "stow.bulk", "stow.propellant", "stow.provisions", "stow.ore"
        public bool IsConsumable;   // true for propellant & provisions (allows +ship/order buy <good> fill)
        public int ReferencePriceCr;
        public string Note;
    }

    public sealed class AtlasNeighborhoodPack
    {
        public string PackId;
        public string Name;
        public Dictionary<int, AtlasFloorDef> Floors = new Dictionary<int, AtlasFloorDef>();
        public Dictionary<string, AtlasRoomDef> Rooms = new Dictionary<string, AtlasRoomDef>(StringComparer.Ordinal);
        public List<AtlasExitDef> Exits = new List<AtlasExitDef>();
        public List<AtlasFeatureDef> Features = new List<AtlasFeatureDef>();
        public List<AtlasNpcSeedDef> SeedNpcs = new List<AtlasNpcSeedDef>();
        public List<AtlasItemSeedDef> SeedItems = new List<AtlasItemSeedDef>();

        public Vector2 GetBoundsCenter2D()
        {
            if (Rooms.Count == 0) return new Vector2(160f, 150f);
            float minX = float.MaxValue, minY = float.MaxValue;
            float maxX = float.MinValue, maxY = float.MinValue;
            foreach (var r in Rooms.Values)
            {
                if (r.MinX < minX) minX = r.MinX;
                if (r.MinY < minY) minY = r.MinY;
                if (r.MaxX > maxX) maxX = r.MaxX;
                if (r.MaxY > maxY) maxY = r.MaxY;
            }
            return new Vector2((minX + maxX) * 0.5f, (minY + maxY) * 0.5f);
        }

        public string ComputeGeometrySignature()
        {
            var sb = new System.Text.StringBuilder(512);
            sb.Append(PackId).Append('|');
            foreach (var fl in Floors)
                sb.Append(fl.Key).Append(':').Append(fl.Value.ZOffsetM.ToString("F2")).Append(':').Append(fl.Value.HeightM.ToString("F2")).Append(';');
            var roomKeys = new List<string>(Rooms.Keys);
            roomKeys.Sort(StringComparer.Ordinal);
            for (int i = 0; i < roomKeys.Count; i++)
            {
                var r = Rooms[roomKeys[i]];
                sb.Append(r.EntryId).Append(':').Append(r.FloorIndex).Append(':')
                  .Append(r.MinX.ToString("F2")).Append(',').Append(r.MinY.ToString("F2")).Append(',')
                  .Append(r.MaxX.ToString("F2")).Append(',').Append(r.MaxY.ToString("F2")).Append(':')
                  .Append(r.Restricted ? 'R' : 'P').Append(';');
            }
            for (int i = 0; i < Exits.Count; i++)
            {
                var ex = Exits[i];
                if (ex.Hidden) continue;
                sb.Append(ex.EntryId).Append(':').Append(ex.FromRoom).Append('>').Append(ex.ToRoom)
                  .Append(':').Append(ex.Locked ? 'L' : 'O').Append(';');
            }
            return sb.ToString();
        }

        public string GetDefaultSpawnRoomEntry()
        {
            if (Rooms.ContainsKey("room.c03_arrivals")) return "room.c03_arrivals";
            if (Rooms.ContainsKey("room.p03_customs")) return "room.p03_customs";
            if (Rooms.ContainsKey("room.l03_arrivals")) return "room.l03_arrivals";
            if (Rooms.ContainsKey("room.simulator_lobby")) return "room.simulator_lobby";
            if (Rooms.ContainsKey("room.welcome_hall")) return "room.welcome_hall";
            foreach (var kv in Rooms)
            {
                if (!kv.Value.Restricted) return kv.Key;
            }
            foreach (var kv in Rooms) return kv.Key;
            return "room.c03_arrivals";
        }

        public string GetBerthGalleryRoomEntry()
        {
            foreach (var kv in Rooms)
            {
                if (kv.Key.IndexOf("berth_gallery", StringComparison.OrdinalIgnoreCase) >= 0 ||
                    kv.Key.IndexOf("berth", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    return kv.Key;
                }
            }
            return null;
        }

        public AtlasNpcSeedDef FindSeedNpcByName(string actorName)
        {
            if (string.IsNullOrWhiteSpace(actorName)) return null;
            string trimmed = actorName.Trim();
            for (int i = 0; i < SeedNpcs.Count; i++)
            {
                if (string.Equals(SeedNpcs[i].Name, trimmed, StringComparison.OrdinalIgnoreCase))
                    return SeedNpcs[i];
            }
            return null;
        }

        public float GetFloorElevationY(int floorIndex)
        {
            if (Floors.TryGetValue(floorIndex, out var f)) return f.ZOffsetM;
            return floorIndex * 5f;
        }

        public float GetFloorHeight(int floorIndex)
        {
            if (Floors.TryGetValue(floorIndex, out var f)) return f.HeightM;
            return 4.5f;
        }

        /// <summary>
        /// Shortest-path BFS over Ceres Freeport exits from <paramref name="fromRoom"/> to <paramref name="toRoom"/>.
        /// Returns ordered (commandAlias, nextRoomEntry) hops so 3D movement always drives exact MUSH movement.
        /// </summary>
        public List<(string command, string targetRoom)> FindExitPath(string fromRoom, string toRoom, bool allowLocked = false)
        {
            var result = new List<(string, string)>();
            if (string.IsNullOrEmpty(fromRoom) || string.IsNullOrEmpty(toRoom) || fromRoom == toRoom)
                return result;

            var queue = new Queue<string>();
            var cameFrom = new Dictionary<string, (string prevRoom, string cmd)>(StringComparer.Ordinal);
            var visited = new HashSet<string>(StringComparer.Ordinal) { fromRoom };
            queue.Enqueue(fromRoom);

            while (queue.Count > 0)
            {
                string cur = queue.Dequeue();
                if (cur == toRoom)
                    break;

                for (int i = 0; i < Exits.Count; i++)
                {
                    var ex = Exits[i];
                    if (ex.Locked && !allowLocked)
                        continue;

                    if (ex.FromRoom == cur && !visited.Contains(ex.ToRoom))
                    {
                        string cmd = TacSimContentLoader.FormatSafeExitCommand(ex.ForwardCommand);
                        if (string.IsNullOrEmpty(cmd)) continue;
                        visited.Add(ex.ToRoom);
                        cameFrom[ex.ToRoom] = (cur, cmd);
                        queue.Enqueue(ex.ToRoom);
                    }
                    else if (ex.ToRoom == cur && !visited.Contains(ex.FromRoom))
                    {
                        string cmd = TacSimContentLoader.FormatSafeExitCommand(ex.ReverseCommand);
                        if (string.IsNullOrEmpty(cmd)) continue;
                        visited.Add(ex.FromRoom);
                        cameFrom[ex.FromRoom] = (cur, cmd);
                        queue.Enqueue(ex.FromRoom);
                    }
                }
            }

            if (!cameFrom.ContainsKey(toRoom))
                return result;

            string walk = toRoom;
            while (walk != fromRoom)
            {
                var step = cameFrom[walk];
                result.Add((step.cmd, walk));
                walk = step.prevRoom;
            }
            result.Reverse();
            return result;
        }
    }

    [Serializable]
    public sealed class ShipCompartmentDef
    {
        public string Id; // "airlock", "passage", "bridge", "crew", "hold", "engineering"
        public string Name;
        public string ShortDesc;
        public string Desc;
        [NonSerialized]
        public Dictionary<string, string> Details = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        public Vector3 LocalCenter;
        public Vector3 LocalSize;
    }

    [Serializable]
    public sealed class ShipExitPairDef
    {
        public string From;
        public string To;
        public string ForwardCommand;
        public string ReverseCommand;
        public string ForwardLabel;
        public string ReverseLabel;
        /// <summary>True ONLY for statically synthesized non-exit ship actions (+tv/bay in|out).</summary>
        public bool IsStaticAction;
    }

    public sealed class ShipUnitTemplate
    {
        public string UnitId;
        public string Label;
        public Dictionary<string, ShipCompartmentDef> Compartments = new Dictionary<string, ShipCompartmentDef>(StringComparer.Ordinal);
        public List<ShipExitPairDef> Exits = new List<ShipExitPairDef>();

        public List<(string command, string targetCompartment)> FindExitPath(string fromComp, string toComp)
        {
            var result = new List<(string, string)>();
            if (string.IsNullOrEmpty(fromComp) || string.IsNullOrEmpty(toComp) || fromComp == toComp)
                return result;

            var queue = new Queue<string>();
            var cameFrom = new Dictionary<string, (string prev, string cmd)>(StringComparer.Ordinal);
            var visited = new HashSet<string>(StringComparer.Ordinal) { fromComp };
            queue.Enqueue(fromComp);

            while (queue.Count > 0)
            {
                string cur = queue.Dequeue();
                if (cur == toComp) break;

                for (int i = 0; i < Exits.Count; i++)
                {
                    var ex = Exits[i];
                    if (ex.From == cur && !visited.Contains(ex.To))
                    {
                        string cmd = TacSimContentLoader.FormatShipTraversalCommand(ex.ForwardCommand, ex.IsStaticAction);
                        if (string.IsNullOrEmpty(cmd)) continue;
                        visited.Add(ex.To);
                        cameFrom[ex.To] = (cur, cmd);
                        queue.Enqueue(ex.To);
                    }
                    else if (ex.To == cur && !visited.Contains(ex.From))
                    {
                        string cmd = TacSimContentLoader.FormatShipTraversalCommand(ex.ReverseCommand, ex.IsStaticAction);
                        if (string.IsNullOrEmpty(cmd)) continue;
                        visited.Add(ex.From);
                        cameFrom[ex.From] = (cur, cmd);
                        queue.Enqueue(ex.From);
                    }
                }
            }

            if (!cameFrom.ContainsKey(toComp))
                return result;

            string walk = toComp;
            while (walk != fromComp)
            {
                var step = cameFrom[walk];
                result.Add((step.cmd, walk));
                walk = step.prev;
            }
            result.Reverse();
            return result;
        }
    }

    /// <summary>
    /// Resolves canonical TacSim &amp; Atlas JSON content packs with priority:
    /// 1) Dynamically synced server packs (`TacSimContentSync` disk/RAM cache)
    /// 2) Embedded Unity `Resources/TacSimFallbackPacks/&lt;id&gt;` (for standalone builds against older servers)
    /// 3) Local repository `../crates/{tacsim,mush-atlas}/content/` (in Editor / dev checkouts)
    /// 4) Built-in deterministic fallbacks.
    /// </summary>
    public static class TacSimContentLoader
    {
        private static readonly Dictionary<string, string> DynamicPacks = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

        public static void RegisterDynamicPack(string packId, string rawJson)
        {
            if (string.IsNullOrWhiteSpace(packId) || string.IsNullOrWhiteSpace(rawJson))
                return;
            lock (DynamicPacks)
            {
                DynamicPacks[packId.Trim()] = rawJson;
            }
        }

        public static void ClearDynamicPacks()
        {
            lock (DynamicPacks)
            {
                DynamicPacks.Clear();
            }
        }

        public static bool TryGetSyncedOrResourceJson(string packId, out string json)
        {
            json = null;
            if (string.IsNullOrWhiteSpace(packId))
                return false;

            string key = packId.Trim();
            lock (DynamicPacks)
            {
                if (DynamicPacks.TryGetValue(key, out string cached) && !string.IsNullOrEmpty(cached))
                {
                    json = cached;
                    return true;
                }
            }

            // Embedded standalone fallback (`Resources/TacSimFallbackPacks/<key>` staged by TacSimStandaloneBuilder)
            // so cold-cache standalone `.exe` / `.app` builds render the full 16-room 3-deck Ceres Freeport, Dray,
            // and Goods catalog immediately on launch before WebSocket login!
            if (!IsSafeContentSlug(key))
                return false;
            try
            {
                TextAsset embedded = Resources.Load<TextAsset>("TacSimFallbackPacks/" + key);
                if (embedded != null && !string.IsNullOrEmpty(embedded.text))
                {
                    json = embedded.text;
                    return true;
                }
            }
            catch
            {
                // Managed CI runner without Unity native Resources
            }
            return false;
        }

        /// <summary>
        /// Structural check mirroring server `neighborhood_is_player_accessible`: returns false if a neighborhood
        /// pack is explicitly marked staff-only or contains only restricted/staff rooms.
        /// </summary>
        public static bool IsNeighborhoodPackPlayerAccessible(string rawJson)
        {
            if (string.IsNullOrWhiteSpace(rawJson))
                return false;
            try
            {
                JsonNode root = JsonNode.Parse(rawJson);
                if (root == null || !root.IsObject) return false;
                var entriesObj = root["entries"].AsObject();
                if (entriesObj == null || entriesObj.Count == 0) return true;

                int totalRooms = 0;
                int publicRooms = 0;
                foreach (var kv in entriesObj)
                {
                    var entry = kv.Value;
                    if (entry == null || !entry.IsObject) continue;
                    string kind = entry.Str("kind", string.Empty);
                    var def = entry["def"];
                    if (!def.IsObject) continue;

                    if (kind == "neighborhood")
                    {
                        if (string.Equals(def.Str("reveal"), "staff", StringComparison.OrdinalIgnoreCase) ||
                            string.Equals(def.Str("access"), "staff", StringComparison.OrdinalIgnoreCase))
                            return false;
                    }
                    else if (kind == "room")
                    {
                        totalRooms++;
                        bool restricted = string.Equals(def.Str("access"), "restricted", StringComparison.OrdinalIgnoreCase) ||
                                          string.Equals(def.Str("access"), "staff", StringComparison.OrdinalIgnoreCase) ||
                                          string.Equals(def.Str("reveal"), "staff", StringComparison.OrdinalIgnoreCase) ||
                                          def.Bool("restricted", false) ||
                                          def["access"].Bool("restricted", false);
                        if (!restricted)
                            publicRooms++;
                    }
                }
                return totalRooms == 0 || publicRooms > 0;
            }
            catch
            {
                return false;
            }
        }

        /// <summary>
        /// Strips server-only narrative secrets, lore, quests, hooks, vendor lane schedules, hidden exits,
        /// restricted-room descriptions, and private NPC prompts before staging fallback packs into a client build.
        /// Mirrors <c>crates/tacsim/src/client_sync.rs</c>'s public projection boundary.
        /// </summary>
        public static string SanitizePackJsonForClient(string rawJson)
        {
            if (string.IsNullOrWhiteSpace(rawJson))
                return rawJson;

            JsonNode root = JsonNode.Parse(rawJson);
            if (root == null || !root.IsObject)
                return rawJson;

            var rootObj = root.AsObject();
            rootObj.Remove("brief");
            rootObj.Remove("notes");
            rootObj.Remove("secrets");

            var entriesObj = root["entries"].AsObject();
            if (entriesObj != null)
            {
                // First pass: identify restricted rooms so items inside them can also be sanitized.
                var restrictedRoomIds = new HashSet<string>(StringComparer.Ordinal);
                foreach (var kv in entriesObj)
                {
                    var entry = kv.Value;
                    if (entry == null || !entry.IsObject || entry.Str("kind") != "room") continue;
                    var def = entry["def"];
                    if (!def.IsObject) continue;
                    bool restricted = string.Equals(def.Str("access"), "restricted", StringComparison.OrdinalIgnoreCase) ||
                                      string.Equals(def.Str("access"), "staff", StringComparison.OrdinalIgnoreCase) ||
                                      def.Bool("restricted", false) ||
                                      def["access"].Bool("restricted", false);
                    if (restricted)
                        restrictedRoomIds.Add(kv.Key);
                }

                var removeKeys = new List<string>();
                foreach (var kv in entriesObj)
                {
                    var entry = kv.Value;
                    if (entry == null || !entry.IsObject)
                    {
                        removeKeys.Add(kv.Key);
                        continue;
                    }

                    string kind = entry.Str("kind", string.Empty);
                    bool allowedKind = kind == "neighborhood" || kind == "locality" || kind == "site" ||
                                       kind == "structure" || kind == "room" || kind == "exit" ||
                                       kind == "feature" || kind == "item" || kind == "region" ||
                                       kind == "heightfield" || kind == "terrain" || kind == "connection" ||
                                       kind == "checkpoint" || kind == "npc" || kind == "person" ||
                                       kind == "unit" || kind == "weapon" || kind == "arena" ||
                                       kind == "body" || kind == "location" || kind == "route" || kind == "port";
                    if (!allowedKind)
                    {
                        removeKeys.Add(kv.Key);
                        continue;
                    }

                    var def = entry["def"];
                    var defObj = def.AsObject();
                    if (defObj == null)
                        continue;

                    if (string.Equals(def.Str("reveal"), "staff", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(def.Str("visibility"), "hidden", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(def["portal"].Str("visibility"), "hidden", StringComparison.OrdinalIgnoreCase) ||
                        string.Equals(def.Str("lock"), "staff", StringComparison.OrdinalIgnoreCase) ||
                        def.Bool("hidden", false) ||
                        def["portal"].Bool("hidden", false))
                    {
                        removeKeys.Add(kv.Key);
                        continue;
                    }

                    defObj.Remove("notes");
                    defObj.Remove("secrets");

                    if (kind == "room")
                    {
                        if (restrictedRoomIds.Contains(kv.Key))
                        {
                            defObj.Remove("desc");
                            defObj.Remove("short_desc");
                            defObj.Remove("details");
                            defObj.Remove("geometry");
                        }
                    }
                    else if (kind == "item")
                    {
                        string roomId = def.Str("room", string.Empty);
                        if (!string.IsNullOrEmpty(roomId) && restrictedRoomIds.Contains(roomId))
                        {
                            removeKeys.Add(kv.Key);
                            continue;
                        }
                    }
                    else if (kind == "npc" || kind == "person")
                    {
                        var stripNpcKeys = new List<string>();
                        foreach (var nk in defObj)
                        {
                            string k = nk.Key;
                            bool keep = k == "name" || k == "aliases" || k == "room" || k == "home" ||
                                        k == "enabled" || k == "short_desc" || k == "desc" || k == "profile";
                            if (!keep)
                                stripNpcKeys.Add(k);
                        }
                        for (int s = 0; s < stripNpcKeys.Count; s++)
                            defObj.Remove(stripNpcKeys[s]);

                        var profileObj = def["profile"].AsObject();
                        if (profileObj != null)
                        {
                            bool enabled = profileObj.ContainsKey("enabled") ? def["profile"].Bool("enabled", true) : true;
                            string bg = def["profile"].Str("background", string.Empty);
                            if (!string.IsNullOrEmpty(bg))
                            {
                                int semi = bg.IndexOf(';');
                                int dot = bg.IndexOf('.');
                                int cut = semi > 0 && (dot < 0 || semi < dot) ? semi : dot;
                                if (cut > 0 && cut <= 96)
                                    bg = bg.Substring(0, cut).Trim();
                                else if (bg.Length > 96)
                                    bg = bg.Substring(0, 93).Trim() + "...";
                            }
                            profileObj.Clear();
                            profileObj["enabled"] = JsonNode.FromBool(enabled);
                            if (!string.IsNullOrEmpty(bg))
                                profileObj["background"] = JsonNode.FromString(bg);
                        }
                    }
                }

                for (int i = 0; i < removeKeys.Count; i++)
                    entriesObj.Remove(removeKeys[i]);
            }

            return root.ToJson();
        }

        /// <summary>
        /// Strict slug allowlist (<c>^[A-Za-z0-9._-]+$</c>, rejecting <c>..</c>, separators, colons, and empty/overlong IDs).
        /// Prevents Windows UNC/SMB NTLMv2 disclosure and local <c>.json</c> path traversal.
        /// </summary>
        public static bool IsSafeContentSlug(string slug)
        {
            if (string.IsNullOrEmpty(slug) || slug.Length > 128)
                return false;
            if (slug == "." || slug == ".." || slug.Contains("..", StringComparison.Ordinal))
                return false;
            for (int i = 0; i < slug.Length; i++)
            {
                char c = slug[i];
                bool ok = (c >= 'A' && c <= 'Z') ||
                          (c >= 'a' && c <= 'z') ||
                          (c >= '0' && c <= '9') ||
                          c == '.' || c == '_' || c == '-';
                if (!ok)
                    return false;
            }
            return true;
        }

        private static bool IsSafeRelativeContentPath(string relativePath)
        {
            if (string.IsNullOrWhiteSpace(relativePath))
                return false;
            if (Path.IsPathRooted(relativePath))
                return false;
            if (relativePath.StartsWith("\\", StringComparison.Ordinal) ||
                relativePath.StartsWith("/", StringComparison.Ordinal) ||
                relativePath.IndexOf(':') >= 0)
                return false;

            string[] segments = relativePath.Split(new[] { '/', '\\' });
            if (segments.Length == 0)
                return false;
            for (int i = 0; i < segments.Length; i++)
            {
                string seg = segments[i];
                if (!IsSafeContentSlug(seg))
                    return false;
            }
            return true;
        }

        private static bool TryResolveContainedPath(string contentRoot, string relativePath, out string resolved)
        {
            resolved = string.Empty;
            if (string.IsNullOrWhiteSpace(contentRoot) || !IsSafeRelativeContentPath(relativePath))
                return false;
            try
            {
                string canonicalRoot = Path.GetFullPath(contentRoot)
                    .TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
                string rootPrefix = canonicalRoot + Path.DirectorySeparatorChar;
                string candidate = Path.GetFullPath(Path.Combine(canonicalRoot, relativePath));
                StringComparison cmp = Path.DirectorySeparatorChar == '\\'
                    ? StringComparison.OrdinalIgnoreCase
                    : StringComparison.Ordinal;
                if (!candidate.StartsWith(rootPrefix, cmp))
                    return false;
                resolved = candidate;
                return true;
            }
            catch
            {
                return false;
            }
        }

        public static string ResolveRepoContentPath(string relativeUnderTacsimContent)
        {
            if (!IsSafeRelativeContentPath(relativeUnderTacsimContent))
                return string.Empty;

            // Application.dataPath is <repo>/unity/Assets when running inside Unity
            try
            {
                string assetsDir = Application.dataPath;
                if (!string.IsNullOrEmpty(assetsDir))
                {
                    string root = Path.Combine(assetsDir, "..", "..", "crates", "tacsim", "content");
                    if (TryResolveContainedPath(root, relativeUnderTacsimContent, out string candidate) &&
                        (File.Exists(candidate) || Directory.Exists(candidate)))
                        return candidate;
                }
            }
            catch
            {
                // Standalone managed test runner without Unity native runtime
            }

            string dir = Directory.GetCurrentDirectory();
            for (int depth = 0; depth < 6 && !string.IsNullOrEmpty(dir); depth++)
            {
                string root = Path.Combine(dir, "crates", "tacsim", "content");
                if (TryResolveContainedPath(root, relativeUnderTacsimContent, out string candidate) &&
                    (File.Exists(candidate) || Directory.Exists(candidate)))
                    return candidate;
                dir = Directory.GetParent(dir)?.FullName;
            }

            string fallbackRoot = Path.Combine(Directory.GetCurrentDirectory(), "crates", "tacsim", "content");
            return TryResolveContainedPath(fallbackRoot, relativeUnderTacsimContent, out string fallback)
                ? fallback
                : string.Empty;
        }

        public static string ResolveMushAtlasContentPath(string relativeUnderMushAtlasContent)
        {
            if (!IsSafeRelativeContentPath(relativeUnderMushAtlasContent))
                return string.Empty;

            try
            {
                string assetsDir = Application.dataPath;
                if (!string.IsNullOrEmpty(assetsDir))
                {
                    string root = Path.Combine(assetsDir, "..", "..", "crates", "mush-atlas", "content");
                    if (TryResolveContainedPath(root, relativeUnderMushAtlasContent, out string candidate) &&
                        (File.Exists(candidate) || Directory.Exists(candidate)))
                        return candidate;
                }
            }
            catch
            {
                // Standalone managed test runner without Unity native runtime
            }

            string dir = Directory.GetCurrentDirectory();
            for (int depth = 0; depth < 6 && !string.IsNullOrEmpty(dir); depth++)
            {
                string root = Path.Combine(dir, "crates", "mush-atlas", "content");
                if (TryResolveContainedPath(root, relativeUnderMushAtlasContent, out string candidate) &&
                    (File.Exists(candidate) || Directory.Exists(candidate)))
                    return candidate;
                dir = Directory.GetParent(dir)?.FullName;
            }

            string fallbackRoot = Path.Combine(Directory.GetCurrentDirectory(), "crates", "mush-atlas", "content");
            return TryResolveContainedPath(fallbackRoot, relativeUnderMushAtlasContent, out string fallback)
                ? fallback
                : string.Empty;
        }

        public static AtlasNeighborhoodPack LoadCeresFreeport()
        {
            return LoadNeighborhoodPackById("neighborhood.ceres_freeport");
        }

        public static AtlasNeighborhoodPack LoadNeighborhoodPackById(string packId)
        {
            string normalized = string.IsNullOrWhiteSpace(packId)
                ? "neighborhood.ceres_freeport"
                : packId.Trim().ToLowerInvariant();
            if (!normalized.StartsWith("neighborhood.", StringComparison.Ordinal))
                normalized = "neighborhood." + normalized;

            string slug = normalized.Substring("neighborhood.".Length);
            if (!IsSafeContentSlug(slug) || !IsSafeContentSlug(normalized))
            {
                Debug.LogWarning($"[TacSimContentLoader] Rejected unsafe neighborhood pack id '{packId}'; building fallback layout.");
                return BuildFallbackFreeport();
            }

            AtlasNeighborhoodPack pack;
            if (TryGetSyncedOrResourceJson(normalized, out string syncedNeighborhoodJson))
            {
                pack = ParseNeighborhoodJson(syncedNeighborhoodJson, normalized);
            }
            else
            {
                string path = normalized == "neighborhood.ooc_area"
                    ? ResolveMushAtlasContentPath("neighborhood.ooc_area.json")
                    : ResolveRepoContentPath(Path.Combine("atlas", $"{normalized}.json"));

                if (string.IsNullOrEmpty(path) || !File.Exists(path))
                {
                    string alt = ResolveMushAtlasContentPath($"{normalized}.json");
                    if (!string.IsNullOrEmpty(alt) && File.Exists(alt)) path = alt;
                }

                if (!string.IsNullOrEmpty(path) && File.Exists(path))
                {
                    pack = ParseNeighborhoodJson(File.ReadAllText(path), normalized);
                }
                else
                {
                    Debug.LogWarning($"[TacSimContentLoader] Could not find {path}; building fallback layout for {normalized}.");
                    pack = normalized == "neighborhood.ooc_area" ? BuildFallbackOocArea() : BuildFallbackFreeport();
                }
            }

            if (TryGetSyncedOrResourceJson($"cast.{slug}", out string syncedCastJson))
            {
                MergeCastJsonIntoPack(pack, syncedCastJson);
            }
            else
            {
                string castPath = ResolveRepoContentPath(Path.Combine("cast", $"{slug}.json"));
                if (!string.IsNullOrEmpty(castPath) && File.Exists(castPath))
                {
                    MergeCastJsonIntoPack(pack, File.ReadAllText(castPath));
                }
            }
            return pack;
        }

        public static void MergeCastJsonIntoPack(AtlasNeighborhoodPack pack, string castJson)
        {
            if (pack == null || string.IsNullOrWhiteSpace(castJson)) return;
            var root = JsonNode.Parse(castJson);
            var entries = root["entries"].AsObject();
            if (entries == null) return;

            foreach (var kv in entries)
            {
                var entry = kv.Value;
                string kind = entry.Str("kind", string.Empty);
                if (kind != "npc" && kind != "person") continue;

                var def = entry["def"];
                var profile = def["profile"];
                bool enabled = true;
                if (profile.IsObject && profile.ContainsKey("enabled"))
                    enabled = profile.Bool("enabled", true);
                else if (def.ContainsKey("enabled"))
                    enabled = def.Bool("enabled", true);

                string fullBg = profile.IsObject ? profile.Str("background", string.Empty) : def.Str("short_desc", string.Empty);
                string bg = fullBg;
                if (!string.IsNullOrEmpty(bg))
                {
                    int semi = bg.IndexOf(';');
                    int dot = bg.IndexOf('.');
                    int cut = semi > 0 && (dot < 0 || semi < dot) ? semi : dot;
                    if (cut > 0 && cut < 110) bg = bg.Substring(0, cut).Trim();
                    else if (bg.Length > 96) bg = bg.Substring(0, 93).Trim() + "...";
                }

                string name = def.Str("name", kv.Key);
                if (pack.FindSeedNpcByName(name) != null) continue;

                pack.SeedNpcs.Add(new AtlasNpcSeedDef
                {
                    EntryId = kv.Key,
                    Name = name,
                    RoomId = def.Str("room", def.Str("home", string.Empty)),
                    ShortDesc = bg,
                    Enabled = enabled,
                    IsMale = InferIsMale(name, fullBg)
                });
            }
        }

        private static int ParseSurfaceFloorIndex(JsonNode surfaceNode, Dictionary<int, AtlasFloorDef> floors)
        {
            if (surfaceNode != null && !surfaceNode.IsNull)
            {
                if (surfaceNode.Kind == JsonKind.String &&
                    string.Equals(surfaceNode.AsString(), "ground", StringComparison.OrdinalIgnoreCase))
                    return 0;

                if (surfaceNode.IsObject)
                {
                    var floorTuple = surfaceNode["floor"].AsArray();
                    if (floorTuple != null && floorTuple.Count >= 2)
                        return floorTuple[1].AsInt(0);
                    if (surfaceNode.ContainsKey("ground"))
                        return 0;
                }
            }

            if (floors != null && floors.ContainsKey(0))
                return 0;
            if (floors != null && floors.ContainsKey(1))
                return 1;
            return 0;
        }

        public static AtlasNeighborhoodPack ParseNeighborhoodJson(string json, string defaultPackId = "neighborhood.ceres_freeport")
        {
            var root = JsonNode.Parse(json);
            var pack = new AtlasNeighborhoodPack
            {
                PackId = root["manifest"].Str("id", string.IsNullOrEmpty(defaultPackId) ? "neighborhood.ceres_freeport" : defaultPackId),
                Name = "Ceres Freeport"
            };

            var entries = root["entries"].AsObject();
            if (entries == null) return pack;

            // 1. Structure floors first so rooms know deck elevations
            foreach (var kv in entries)
            {
                var entry = kv.Value;
                if (entry.Str("kind") != "structure") continue;
                var floorsArr = entry["def"]["floors"].AsArray();
                if (floorsArr == null) continue;
                for (int i = 0; i < floorsArr.Count; i++)
                {
                    var fl = floorsArr[i];
                    int idx = fl.Int("index", i);
                    pack.Floors[idx] = new AtlasFloorDef
                    {
                        Index = idx,
                        Name = fl.Str("name", $"Deck {idx}"),
                        ZOffsetM = fl.Float("z_offset_m", idx * 5f),
                        HeightM = fl.Float("height_m", 4.5f)
                    };
                }
            }

            // 2. Rooms, Exits, Features, NPCs
            foreach (var kv in entries)
            {
                string id = kv.Key;
                var entry = kv.Value;
                string kind = entry.Str("kind", string.Empty);
                var def = entry["def"];

                if (kind == "neighborhood")
                {
                    pack.Name = def.Str("name", pack.Name);
                }
                else if (kind == "room")
                {
                    var rect = def["footprint"]["rect"].AsArray();
                    if (rect == null || rect.Count < 2) continue;
                    int floorIdx = ParseSurfaceFloorIndex(def["surface"], pack.Floors);

                    var r = new AtlasRoomDef
                    {
                        EntryId = id,
                        Name = def.Str("name", id),
                        ShortDesc = def.Str("short_desc", string.Empty),
                        Desc = def.Str("desc", string.Empty),
                        Kind = def.Str("kind", "indoor"),
                        Restricted = def.Str("access", "public") == "restricted",
                        FloorIndex = floorIdx,
                        MinX = rect[0][0].AsFloat(0f),
                        MinY = rect[0][1].AsFloat(0f),
                        MaxX = rect[1][0].AsFloat(10f),
                        MaxY = rect[1][1].AsFloat(10f)
                    };

                    var detailsObj = def["details"].AsObject();
                    if (detailsObj != null)
                    {
                        foreach (var dk in detailsObj)
                            r.Details[dk.Key] = dk.Value.AsString(string.Empty);
                    }

                    var markers = def["markers"].AsArray();
                    if (markers != null)
                    {
                        for (int m = 0; m < markers.Count; m++)
                            r.Markers.Add(markers[m].AsString(string.Empty));
                    }

                    pack.Rooms[id] = r;
                }
                else if (kind == "exit")
                {
                    string from = def.Str("from", string.Empty);
                    string to = def.Str("to", string.Empty);
                    if (from == "limbo" || to == "limbo") continue;

                    var names = def["names"].AsArray();
                    ParseExitSpec(names != null && names.Count > 0 ? names[0].AsString("") : "", out string fwdLabel, out string fwdCmd);
                    ParseExitSpec(names != null && names.Count > 1 ? names[1].AsString("") : "", out string revLabel, out string revCmd);

                    var portal = def["portal"];
                    var ex = new AtlasExitDef
                    {
                        EntryId = id,
                        FromRoom = from,
                        ToRoom = to,
                        ForwardLabel = fwdLabel,
                        ForwardCommand = fwdCmd,
                        ReverseLabel = revLabel,
                        ReverseCommand = revCmd,
                        Mode = portal.Str("mode", "door"),
                        Locked = !string.IsNullOrEmpty(def.Str("lock")),
                        Hidden = portal.Str("visibility", "public") == "hidden",
                        DzM = portal.Float("dz_m", 0f)
                    };

                    var at = portal["at"].AsArray();
                    if (at != null && at.Count >= 2 && at[0].Count >= 3 && at[1].Count >= 3)
                    {
                        ex.HasExplicitPortal = true;
                        // Atlas (x East, y North, z Up) -> Unity (x East, y Up, z North)
                        ex.PortalA = new Vector3(at[0][0].AsFloat(), at[0][2].AsFloat(), at[0][1].AsFloat());
                        ex.PortalB = new Vector3(at[1][0].AsFloat(), at[1][2].AsFloat(), at[1][1].AsFloat());
                    }

                    pack.Exits.Add(ex);
                }
                else if (kind == "feature")
                {
                    var poly = def["polyline"].AsArray();
                    if (poly == null || poly.Count < 2) continue;
                    int floorIdx = ParseSurfaceFloorIndex(def["surface"], pack.Floors);

                    var zRange = def["z_range"].AsArray();
                    float h = (zRange != null && zRange.Count >= 2) ? (zRange[1].AsFloat(1.1f) - zRange[0].AsFloat(0f)) : 1.1f;

                    pack.Features.Add(new AtlasFeatureDef
                    {
                        EntryId = id,
                        FeatureKind = def.Str("feature_kind", "wall"),
                        RoomId = def.Str("room", string.Empty),
                        FloorIndex = floorIdx,
                        PointA = new Vector2(poly[0][0].AsFloat(), poly[0][1].AsFloat()),
                        PointB = new Vector2(poly[1][0].AsFloat(), poly[1][1].AsFloat()),
                        HeightM = Mathf.Max(0.5f, h),
                        BlocksMovement = def["blocks"].Bool("movement", false)
                    });
                }
                else if (kind == "npc" || kind == "person")
                {
                    bool enabled = !def.ContainsKey("enabled") || def.Bool("enabled", true);
                    string room = def.Str("room", def.Str("home", string.Empty));
                    if (!string.IsNullOrEmpty(room))
                    {
                        string npcName = def.Str("name", id);
                        string npcBio = def.Str("short_desc", def.Str("persona", string.Empty));
                        pack.SeedNpcs.Add(new AtlasNpcSeedDef
                        {
                            EntryId = id,
                            Name = npcName,
                            RoomId = room,
                            ShortDesc = npcBio,
                            Enabled = enabled,
                            IsMale = InferIsMale(npcName, npcBio)
                        });
                    }
                }
                else if (kind == "item")
                {
                    string room = def.Str("room", string.Empty);
                    if (!string.IsNullOrEmpty(room))
                    {
                        pack.SeedItems.Add(new AtlasItemSeedDef
                        {
                            EntryId = id,
                            Name = def.Str("name", id),
                            RoomId = room,
                            Desc = def.Str("desc", string.Empty),
                            IsInterior = def.Bool("interior", false)
                        });
                    }
                }
            }

            return pack;
        }

        public static bool InferIsMale(string actorName, string bioOrPersona)
        {
            string n = actorName ?? string.Empty;
            string[] knownMaleNames =
            {
                "Dov Akintola", "Nkem Halvorsen", "Corin Mbatha",
                "Matthias Kalu", "Halvard Ndiaye", "Eskil Varga", "Tomasz Wen",
                "Watch Officer Kade", "Marcus", "Anton", "Viktor"
            };
            for (int i = 0; i < knownMaleNames.Length; i++)
            {
                if (n.IndexOf(knownMaleNames[i], StringComparison.OrdinalIgnoreCase) >= 0)
                    return true;
            }

            if (!string.IsNullOrEmpty(bioOrPersona))
            {
                string lower = " " + bioOrPersona.ToLowerInvariant() + " ";
                int maleHits = (lower.Contains(" he ") ? 1 : 0) + (lower.Contains(" his ") ? 1 : 0) + (lower.Contains(" him ") ? 1 : 0);
                int femaleHits = (lower.Contains(" she ") ? 1 : 0) + (lower.Contains(" her ") ? 1 : 0);
                if (maleHits > femaleHits) return true;
            }
            return false;
        }

        public static List<GoodItemDef> LoadGoodsTable()
        {
            if (TryGetSyncedOrResourceJson("goods", out string syncedGoods))
            {
                return ParseGoodsJson(syncedGoods);
            }
            string path = ResolveRepoContentPath(Path.Combine("market", "goods.json"));
            if (!string.IsNullOrEmpty(path) && File.Exists(path))
            {
                return ParseGoodsJson(File.ReadAllText(path));
            }
            return BuildFallbackGoodsTable();
        }

        public static List<GoodItemDef> ParseGoodsJson(string json)
        {
            var list = new List<GoodItemDef>();
            if (string.IsNullOrWhiteSpace(json))
                return BuildFallbackGoodsTable();

            var root = JsonNode.Parse(json);
            var goods = root["goods"].AsArray();
            if (goods == null)
                return BuildFallbackGoodsTable();

            for (int i = 0; i < goods.Count; i++)
            {
                var g = goods[i];
                var measures = g["measures"];
                string consumable = g.Str("consumable", string.Empty);
                list.Add(new GoodItemDef
                {
                    Id = g.Str("id", string.Empty),
                    Label = g.Str("label", string.Empty),
                    GoodClass = g.Str("class", "trade"),
                    LedgerUnit = g.Str("ledger_unit", "t"),
                    MassKgPerUnit = measures.IsObject ? measures.Int("si.mass_kg", 0) : 0,
                    VolumeLPerUnit = measures.IsObject ? measures.Int("si.volume_l", 0) : 0,
                    Stowage = g.Str("stowage", "stow.bulk"),
                    IsConsumable = !string.IsNullOrEmpty(consumable) && consumable != "null",
                    ReferencePriceCr = g.Int("reference_major", 100),
                    Note = g.Str("note", string.Empty)
                });
            }
            return list.Count > 0 ? list : BuildFallbackGoodsTable();
        }

        private static List<GoodItemDef> BuildFallbackGoodsTable()
        {
            return new List<GoodItemDef>
            {
                new GoodItemDef { Id = "metals", Label = "metal concentrate", GoodClass = "trade", LedgerUnit = "t", MassKgPerUnit = 1000, VolumeLPerUnit = 400, Stowage = "stow.bulk", ReferencePriceCr = 200 },
                new GoodItemDef { Id = "volatiles", Label = "water ice", GoodClass = "trade", LedgerUnit = "t", MassKgPerUnit = 1000, VolumeLPerUnit = 1400, Stowage = "stow.bulk", ReferencePriceCr = 100 },
                new GoodItemDef { Id = "structural_alloys", Label = "structural alloy stock", GoodClass = "trade", LedgerUnit = "t", MassKgPerUnit = 1000, VolumeLPerUnit = 300, Stowage = "stow.bulk", ReferencePriceCr = 400 },
                new GoodItemDef { Id = "industrial_parts", Label = "yard fittings, crated", GoodClass = "trade", LedgerUnit = "t", MassKgPerUnit = 1000, VolumeLPerUnit = 2300, Stowage = "stow.bulk", ReferencePriceCr = 400 },
                new GoodItemDef { Id = "propellant", Label = "reaction mass", GoodClass = "trade", LedgerUnit = "t", MassKgPerUnit = 1000, VolumeLPerUnit = 1000, Stowage = "stow.propellant", IsConsumable = true, ReferencePriceCr = 40 },
                new GoodItemDef { Id = "provisions", Label = "voyage provisions", GoodClass = "ship_supply", LedgerUnit = "crew_day", MassKgPerUnit = 4, VolumeLPerUnit = 10, Stowage = "stow.provisions", IsConsumable = true, ReferencePriceCr = 10 }
            };
        }

        public static ShipUnitTemplate LoadDrayTemplate()
        {
            return LoadShipTemplate("tacsim.unit.dray");
        }

        public static ShipUnitTemplate LoadShipTemplate(string unitId)
        {
            string normalized = string.IsNullOrWhiteSpace(unitId) ? "tacsim.unit.dray" : unitId.Trim().ToLowerInvariant();
            if (!normalized.StartsWith("tacsim.unit.", StringComparison.Ordinal))
                normalized = "tacsim.unit." + normalized;

            string fileStem = normalized.Substring("tacsim.unit.".Length);
            if (!IsSafeContentSlug(fileStem) || !IsSafeContentSlug(normalized))
            {
                Debug.LogWarning($"[TacSimContentLoader] Rejected unsafe ship unit id '{unitId}'; using fallback Dray.");
                return BuildFallbackDray("tacsim.unit.dray");
            }
            if (TryGetSyncedOrResourceJson(fileStem, out string syncedUnitJson))
            {
                return ParseShipUnitJson(syncedUnitJson, normalized);
            }
            string path = ResolveRepoContentPath(Path.Combine("units", $"{fileStem}.json"));
            if (!string.IsNullOrEmpty(path) && File.Exists(path))
            {
                return ParseShipUnitJson(File.ReadAllText(path), normalized);
            }
            return BuildFallbackDray(normalized);
        }

        public static ShipUnitTemplate ParseShipUnitJson(string json, string expectedUnitId = null)
        {
            var root = JsonNode.Parse(json);
            var tpl = new ShipUnitTemplate
            {
                UnitId = string.IsNullOrEmpty(expectedUnitId) ? "tacsim.unit.dray" : expectedUnitId,
                Label = "Dray-class bulk hauler"
            };

            // Standard 3D compartment deck plan along ship Z axis (+Z bow/forward, -Z stern/aft, +Y upper deck)
            var deckLayout = GetStandardDrayCompartmentBounds();

            var entries = root["entries"].AsObject();
            JsonNode unitEntry = JsonNode.Null;
            if (entries != null)
            {
                if (!string.IsNullOrEmpty(expectedUnitId) && entries.TryGetValue(expectedUnitId, out var matched))
                {
                    unitEntry = matched;
                }
                else
                {
                    foreach (var kv in entries)
                    {
                        if (kv.Value.Str("kind") == "unit")
                        {
                            tpl.UnitId = kv.Key;
                            unitEntry = kv.Value;
                            break;
                        }
                    }
                }
            }

            if (unitEntry != null && unitEntry.IsObject)
            {
                var def = unitEntry["def"];
                tpl.Label = def.Str("label", tpl.Label);
                var interior = def["interior"];
                var rooms = interior["rooms"].AsArray();
                if (rooms != null)
                {
                    for (int i = 0; i < rooms.Count; i++)
                    {
                        var r = rooms[i];
                        string rKind = r.Str("kind");
                        if (rKind != "walkable" && rKind != "bay") continue;
                        string id = r.Str("id", string.Empty);
                        deckLayout.TryGetValue(id, out var bounds);
                        var comp = new ShipCompartmentDef
                        {
                            Id = id,
                            Name = r.Str("name", id),
                            ShortDesc = r.Str("short_desc", rKind == "bay" ? "Two-rig stern vehicle bay (RIG 1 & RIG 2)." : string.Empty),
                            Desc = r.Str("desc", string.Empty),
                            LocalCenter = bounds.size.sqrMagnitude > 0.1f ? bounds.center : new Vector3(0f, 0f, -i * 6f),
                            LocalSize = bounds.size.sqrMagnitude > 0.1f ? bounds.size : new Vector3(6f, 3.2f, 6f)
                        };
                        var det = r["details"].AsObject();
                        if (det != null)
                        {
                            foreach (var dk in det)
                                comp.Details[dk.Key] = dk.Value.AsString(string.Empty);
                        }
                        tpl.Compartments[id] = comp;
                    }
                }

                var exits = interior["exits"].AsArray();
                if (exits != null)
                {
                    for (int i = 0; i < exits.Count; i++)
                    {
                        var ex = exits[i];
                        var names = ex["names"].AsArray();
                        ParseExitSpec(names != null && names.Count > 0 ? names[0].AsString("") : "", out string fLbl, out string fCmd);
                        ParseExitSpec(names != null && names.Count > 1 ? names[1].AsString("") : "", out string rLbl, out string rCmd);
                        tpl.Exits.Add(new ShipExitPairDef
                        {
                            From = ex.Str("from", string.Empty),
                            To = ex.Str("to", string.Empty),
                            ForwardLabel = fLbl,
                            ForwardCommand = fCmd,
                            ReverseLabel = rLbl,
                            ReverseCommand = rCmd,
                            IsStaticAction = false
                        });
                    }
                }

                // Dray-P stern Vehicle Bay is reached via canonical +tv/bay in (from Hold) and +tv/bay out (from Bay)
                if (tpl.Compartments.ContainsKey("hold") && tpl.Compartments.ContainsKey("bay"))
                {
                    tpl.Exits.Add(new ShipExitPairDef
                    {
                        From = "hold",
                        To = "bay",
                        ForwardLabel = "Vehicle Bay",
                        ForwardCommand = "+tv/bay in",
                        ReverseLabel = "Cargo Hold",
                        ReverseCommand = "+tv/bay out",
                        IsStaticAction = true
                    });
                }
            }

            if (tpl.Compartments.Count == 0)
                return BuildFallbackDray(tpl.UnitId);
            return tpl;
        }

        private static Dictionary<string, (Vector3 center, Vector3 size)> GetStandardDrayCompartmentBounds()
        {
            // Contiguous 3-level ship spine so the player can walk continuously from Airlock -> Passage -> Crew / Bridge / Relay / Hold / Engineering / Bay!
            return new Dictionary<string, (Vector3 center, Vector3 size)>(StringComparer.Ordinal)
            {
                // Main spine (Y = 0, floor = -1.6m)
                ["airlock"]     = (new Vector3(0f, 0f, 13f),      new Vector3(5.5f, 3.2f, 6f)),
                ["passage"]     = (new Vector3(0f, 0f, 4f),       new Vector3(5.5f, 3.2f, 12f)),
                ["crew"]        = (new Vector3(0f, 0f, -6f),      new Vector3(7.5f, 3.2f, 8f)),
                // Starboard side compartment off passage (Halcyon relief cruiser entanglement relay; flush at X = +2.75m)
                ["relay"]       = (new Vector3(5.25f, 0f, 3f),    new Vector3(5.0f, 3.2f, 6f)),
                // Single-cab surface/crawler rigs
                ["cab"]         = (new Vector3(0f, 0f, 0f),       new Vector3(4.2f, 2.8f, 4.8f)),
                // Upper forward bridge deck (Y = +3.4m, floor = +1.8m, ramp/stair from passage)
                ["bridge"]      = (new Vector3(0f, 3.4f, 9f),     new Vector3(7.5f, 3.2f, 9f)),
                // Lower cargo hold, engineering crawlway & Dray-P/Halcyon stern Vehicle Bay (flush floor Y = -5.3m, flush Z = -14.0m)
                ["hold"]        = (new Vector3(0f, -3.6f, 2f),    new Vector3(9.0f, 3.4f, 12f)),
                ["engineering"] = (new Vector3(0f, -3.6f, -9f),   new Vector3(7.5f, 3.4f, 10f)),
                ["bay"]         = (new Vector3(0f, -3.5f, -19.5f), new Vector3(9.0f, 3.6f, 11f))
            };
        }

        public static bool HasControlChar(string input)
        {
            if (string.IsNullOrEmpty(input)) return false;
            for (int i = 0; i < input.Length; i++)
            {
                char c = input[i];
                if (c < 0x20 || (c >= 0x7f && c <= 0x9f))
                    return true;
            }
            return false;
        }

        /// <summary>
        /// Formats an authored room/portal exit name or alias into a safe `goto &lt;exit&gt;`
        /// command, or null when blank or containing C0/DEL/C1 control characters (EXT-P1-6).
        /// </summary>
        public static string FormatSafeExitCommand(string rawExit)
        {
            if (string.IsNullOrWhiteSpace(rawExit))
                return null;
            string trimmed = rawExit.Trim();
            if (trimmed.Length == 0 || HasControlChar(trimmed))
                return null;
            return "goto " + trimmed;
        }

        /// <summary>
        /// Formats a ship-interior traversal command: static non-exit ship-bay actions
        /// (`+tv/bay in`, `+tv/bay out`) are allowlisted ONLY when <paramref name="isStaticAction"/> is true;
        /// all authored ship exits are unconditionally prefixed with `goto ` (EXT-P1-6).
        /// </summary>
        public static string FormatShipTraversalCommand(string rawCommand, bool isStaticAction = false)
        {
            if (isStaticAction)
            {
                if (string.IsNullOrWhiteSpace(rawCommand))
                    return null;
                string trimmed = rawCommand.Trim();
                return (string.Equals(trimmed, "+tv/bay in", StringComparison.Ordinal) ||
                        string.Equals(trimmed, "+tv/bay out", StringComparison.Ordinal))
                    ? trimmed
                    : null;
            }
            return FormatSafeExitCommand(rawCommand);
        }

        private static void ParseExitSpec(string raw, out string displayLabel, out string shortAlias)
        {
            if (string.IsNullOrWhiteSpace(raw) || HasControlChar(raw))
            {
                displayLabel = "Exit";
                shortAlias = string.Empty;
                return;
            }
            string[] parts = raw.Split(';');
            string head = parts[0].Trim();
            displayLabel = string.IsNullOrEmpty(head) ? "Exit" : head;
            string alias = parts.Length > 1 ? parts[1].Trim() : string.Empty;
            shortAlias = !string.IsNullOrEmpty(alias) ? alias : head;
            if (FormatSafeExitCommand(shortAlias) == null)
                shortAlias = string.Empty;
        }

        private static AtlasNeighborhoodPack BuildFallbackFreeport()
        {
            var pack = new AtlasNeighborhoodPack { PackId = "neighborhood.ceres_freeport", Name = "Ceres Freeport" };
            pack.Floors[0] = new AtlasFloorDef { Index = 0, Name = "Freight Deck", ZOffsetM = 0f, HeightM = 6f };
            pack.Floors[1] = new AtlasFloorDef { Index = 1, Name = "Public Deck", ZOffsetM = 6f, HeightM = 4.5f };
            pack.Rooms["room.c01_berth_gallery"] = new AtlasRoomDef { EntryId = "room.c01_berth_gallery", Name = "Visiting Berth Gallery", FloorIndex = 1, MinX = 110, MinY = 110, MaxX = 210, MaxY = 125 };
            pack.Rooms["room.c02_pressure"] = new AtlasRoomDef { EntryId = "room.c02_pressure", Name = "Pressure Lobby", FloorIndex = 1, MinX = 110, MinY = 125, MaxX = 140, MaxY = 140 };
            pack.Rooms["room.c03_arrivals"] = new AtlasRoomDef { EntryId = "room.c03_arrivals", Name = "Port Watch Arrivals", FloorIndex = 1, MinX = 110, MinY = 140, MaxX = 140, MaxY = 165 };
            pack.Rooms["room.c04_concourse"] = new AtlasRoomDef { EntryId = "room.c04_concourse", Name = "Freeport Concourse", FloorIndex = 1, MinX = 140, MinY = 140, MaxX = 175, MaxY = 165 };
            pack.Rooms["room.c05_registry"] = new AtlasRoomDef { EntryId = "room.c05_registry", Name = "Registry & Co-op Desk", FloorIndex = 1, MinX = 110, MinY = 165, MaxX = 140, MaxY = 190 };
            pack.Rooms["room.c07_exchange"] = new AtlasRoomDef { EntryId = "room.c07_exchange", Name = "Exchange Hall", FloorIndex = 1, MinX = 175, MinY = 140, MaxX = 210, MaxY = 165 };
            return pack;
        }

        private static AtlasNeighborhoodPack BuildFallbackOocArea()
        {
            var pack = new AtlasNeighborhoodPack { PackId = "neighborhood.ooc_area", Name = "OOC Simulator Centre" };
            pack.Floors[1] = new AtlasFloorDef { Index = 1, Name = "Main Floor", ZOffsetM = 0f, HeightM = 4.5f };
            pack.Rooms["room.welcome_hall"] = new AtlasRoomDef { EntryId = "room.welcome_hall", Name = "Welcome Hall", FloorIndex = 1, MinX = 95, MinY = 65, MaxX = 117, MaxY = 85 };
            pack.Rooms["room.simulator_lobby"] = new AtlasRoomDef { EntryId = "room.simulator_lobby", Name = "Simulator Lobby", FloorIndex = 1, MinX = 117, MinY = 65, MaxX = 145, MaxY = 85 };
            pack.Rooms["room.sim_red"] = new AtlasRoomDef { EntryId = "room.sim_red", Name = "Red Room", FloorIndex = 1, MinX = 117, MinY = 85, MaxX = 145, MaxY = 105 };
            pack.Rooms["room.sim_blue"] = new AtlasRoomDef { EntryId = "room.sim_blue", Name = "Blue Room", FloorIndex = 1, MinX = 117, MinY = 45, MaxX = 145, MaxY = 65 };
            pack.Exits.Add(new AtlasExitDef { EntryId = "exit.hall_simulator_lobby", FromRoom = "room.welcome_hall", ToRoom = "room.simulator_lobby", ForwardLabel = "Simulator Lobby", ForwardCommand = "simulator", ReverseLabel = "Welcome Hall", ReverseCommand = "w", Mode = "door" });
            pack.Exits.Add(new AtlasExitDef { EntryId = "exit.simulator_lobby_red", FromRoom = "room.simulator_lobby", ToRoom = "room.sim_red", ForwardLabel = "Red Room", ForwardCommand = "red", ReverseLabel = "Simulator Lobby", ReverseCommand = "lobby", Mode = "door" });
            pack.Exits.Add(new AtlasExitDef { EntryId = "exit.simulator_lobby_blue", FromRoom = "room.simulator_lobby", ToRoom = "room.sim_blue", ForwardLabel = "Blue Room", ForwardCommand = "blue", ReverseLabel = "Simulator Lobby", ReverseCommand = "lobby", Mode = "door" });
            pack.SeedItems.Add(new AtlasItemSeedDef { EntryId = "item.simulator_selector", Name = "Simulator Selector", RoomId = "room.simulator_lobby" });
            for (int i = 1; i <= 6; i++)
            {
                pack.SeedItems.Add(new AtlasItemSeedDef { EntryId = $"item.red_pod_{i}", Name = $"Red Pod {i}", RoomId = "room.sim_red", IsInterior = true });
                pack.SeedItems.Add(new AtlasItemSeedDef { EntryId = $"item.blue_pod_{i}", Name = $"Blue Pod {i}", RoomId = "room.sim_blue", IsInterior = true });
            }
            return pack;
        }

        private static ShipUnitTemplate BuildFallbackDray(string unitId = "tacsim.unit.dray")
        {
            var tpl = new ShipUnitTemplate { UnitId = unitId, Label = "Dray-class bulk hauler" };
            foreach (var kv in GetStandardDrayCompartmentBounds())
            {
                if (kv.Key == "bay" && unitId != "tacsim.unit.dray_p") continue;
                tpl.Compartments[kv.Key] = new ShipCompartmentDef
                {
                    Id = kv.Key,
                    Name = CultureInfoLabel(kv.Key),
                    LocalCenter = kv.Value.center,
                    LocalSize = kv.Value.size
                };
            }
            return tpl;
        }

        private static string CultureInfoLabel(string s)
        {
            if (string.IsNullOrEmpty(s)) return "Compartment";
            return char.ToUpperInvariant(s[0]) + s.Substring(1);
        }
    }

    public enum NavWalkerScope
    {
        Station,
        Ship
    }

    public readonly struct QueuedNavHop
    {
        public readonly NavWalkerScope Scope;
        public readonly string FromRoom;
        public readonly string TargetRoom;
        public readonly string Command;

        public QueuedNavHop(NavWalkerScope scope, string fromRoom, string targetRoom, string command)
        {
            Scope = scope;
            FromRoom = fromRoom;
            TargetRoom = targetRoom;
            Command = command;
        }
    }

    public readonly struct NavRoomUpdateResult
    {
        public readonly string AcknowledgedTarget;
        public readonly QueuedNavHop? NextHopToSend;
        public readonly bool CanceledOnMismatch;

        public NavRoomUpdateResult(string acknowledgedTarget, QueuedNavHop? nextHopToSend, bool canceledOnMismatch)
        {
            AcknowledgedTarget = acknowledgedTarget;
            NextHopToSend = nextHopToSend;
            CanceledOnMismatch = canceledOnMismatch;
        }
    }

    /// <summary>
    /// Room-bound single-hop navigation queue (EXT-P1-6):
    /// binds every queued hop to its expected source room and scope, sends at most ONE hop
    /// at a time, waits for an authoritative room update matching the expected destination
    /// before advancing the next hop, and cancels all remaining queued hops on any room
    /// mismatch, scope/pack change, timeout, or unrelated player action.
    /// </summary>
    public sealed class RoomBoundNavWalker
    {
        public const float InFlightTimeoutSeconds = 3.5f;

        private QueuedNavHop? _inFlight;
        private float _inFlightSentAtSec = -100f;
        private readonly Queue<QueuedNavHop> _queue = new Queue<QueuedNavHop>();

        public QueuedNavHop? InFlight => _inFlight;
        public string InFlightTarget => _inFlight.HasValue ? _inFlight.Value.TargetRoom : null;
        public float InFlightSentAtSec => _inFlightSentAtSec;
        public int QueuedCount => _queue.Count;

        public bool HasInFlight(float nowSec)
        {
            return _inFlight.HasValue && (nowSec - _inFlightSentAtSec) < InFlightTimeoutSeconds;
        }

        public bool ExpireIfStale(float nowSec)
        {
            if (_inFlight.HasValue && (nowSec - _inFlightSentAtSec) >= InFlightTimeoutSeconds)
            {
                Cancel();
                return true;
            }
            return false;
        }

        public void Cancel()
        {
            _inFlight = null;
            _inFlightSentAtSec = -100f;
            _queue.Clear();
        }

        public QueuedNavHop? PlanAndDispatchOne(
            NavWalkerScope scope,
            string currentServerRoom,
            IReadOnlyList<(string command, string targetRoom)> hops,
            float nowSec)
        {
            ExpireIfStale(nowSec);
            if (HasInFlight(nowSec))
                return null;

            _queue.Clear();
            if (string.IsNullOrEmpty(currentServerRoom) || hops == null || hops.Count == 0)
                return null;

            var bound = new List<QueuedNavHop>(hops.Count);
            string prev = currentServerRoom;
            for (int i = 0; i < hops.Count; i++)
            {
                string cmd = hops[i].command;
                string target = hops[i].targetRoom;
                if (string.IsNullOrEmpty(cmd) || string.IsNullOrEmpty(target))
                {
                    Cancel();
                    return null;
                }
                bound.Add(new QueuedNavHop(scope, prev, target, cmd));
                prev = target;
            }

            QueuedNavHop first = bound[0];
            if (!string.Equals(first.FromRoom, currentServerRoom, StringComparison.Ordinal))
                return null;

            _inFlight = first;
            _inFlightSentAtSec = nowSec;
            for (int i = 1; i < bound.Count; i++)
                _queue.Enqueue(bound[i]);

            return first;
        }

        public void ReplaceQueuedTailWhileInFlight(
            NavWalkerScope scope,
            IReadOnlyList<(string command, string targetRoom)> hopsFromInFlightTarget,
            float nowSec)
        {
            ExpireIfStale(nowSec);
            if (!_inFlight.HasValue || _inFlight.Value.Scope != scope)
                return;

            _queue.Clear();
            if (hopsFromInFlightTarget == null || hopsFromInFlightTarget.Count == 0)
                return;

            string prev = _inFlight.Value.TargetRoom;
            for (int i = 0; i < hopsFromInFlightTarget.Count; i++)
            {
                string cmd = hopsFromInFlightTarget[i].command;
                string target = hopsFromInFlightTarget[i].targetRoom;
                if (string.IsNullOrEmpty(cmd) || string.IsNullOrEmpty(target))
                {
                    _queue.Clear();
                    return;
                }
                _queue.Enqueue(new QueuedNavHop(scope, prev, target, cmd));
                prev = target;
            }
        }

        public NavRoomUpdateResult OnAuthoritativeRoomUpdate(
            NavWalkerScope scope,
            string confirmedRoom,
            float nowSec,
            bool allowAdvance = true)
        {
            if (!_inFlight.HasValue)
            {
                _queue.Clear();
                return new NavRoomUpdateResult(null, null, canceledOnMismatch: false);
            }

            QueuedNavHop prev = _inFlight.Value;
            if (prev.Scope != scope || !string.Equals(prev.TargetRoom, confirmedRoom, StringComparison.Ordinal))
            {
                Cancel();
                return new NavRoomUpdateResult(null, null, canceledOnMismatch: true);
            }

            _inFlight = null;
            _inFlightSentAtSec = -100f;

            if (!allowAdvance)
            {
                _queue.Clear();
                return new NavRoomUpdateResult(confirmedRoom, null, canceledOnMismatch: false);
            }

            if (_queue.Count == 0)
                return new NavRoomUpdateResult(confirmedRoom, null, canceledOnMismatch: false);

            QueuedNavHop next = _queue.Dequeue();
            if (next.Scope != scope ||
                !string.Equals(next.FromRoom, confirmedRoom, StringComparison.Ordinal) ||
                string.IsNullOrEmpty(next.Command))
            {
                Cancel();
                return new NavRoomUpdateResult(confirmedRoom, null, canceledOnMismatch: true);
            }

            _inFlight = next;
            _inFlightSentAtSec = nowSec;
            return new NavRoomUpdateResult(confirmedRoom, next, canceledOnMismatch: false);
        }
    }
}
