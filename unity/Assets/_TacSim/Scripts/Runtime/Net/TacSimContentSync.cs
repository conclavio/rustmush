using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text;
using System.Threading.Tasks;
using TacSim.Core;
using TacSim.World;
using UnityEngine;

namespace TacSim.Net
{
    public sealed class ContentPackManifestItem
    {
        public string Id;
        public string Category;
        public string Sha256;
        public int Bytes;
    }

    /// <summary>
    /// Dynamically synchronizes authored TacSim &amp; Atlas JSON content packs by SHA-256 digest
    /// from mushd (`GET /api/content/manifest` + `GET /api/content/packs/{id}` over HTTP, or
    /// `tacsim.content` panel queries over WebSocket), caching verified packs under
    /// `Application.persistentDataPath/TacSimContentCache/&lt;host&gt;`.
    /// Gracefully no-ops if connected to an older server returning 404.
    /// </summary>
    public sealed class TacSimContentSync
    {
        private static readonly HttpClient SharedHttp = new HttpClient { Timeout = TimeSpan.FromSeconds(5) };

        private string _cacheRootDir;
        private readonly Dictionary<string, string> _localDigests = new Dictionary<string, string>(StringComparer.Ordinal);
        private bool _httpManifestSucceeded;

        public event Action<string> OnSyncLog;
        public event Action<int> OnPacksUpdated;

        public bool HttpManifestSucceeded => _httpManifestSucceeded;

        public static string ComputeSha256Hex(string utf8Text)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(utf8Text ?? string.Empty);
            using (var sha = SHA256.Create())
            {
                byte[] hash = sha.ComputeHash(bytes);
                var sb = new StringBuilder(64);
                for (int i = 0; i < hash.Length; i++)
                    sb.Append(hash[i].ToString("x2"));
                return sb.ToString();
            }
        }

        public static List<ContentPackManifestItem> ParseManifestPacks(JsonNode rootOrPacksNode)
        {
            var result = new List<ContentPackManifestItem>();
            if (rootOrPacksNode == null || rootOrPacksNode.IsNull) return result;

            var arr = rootOrPacksNode.IsArray ? rootOrPacksNode.AsArray() : rootOrPacksNode["packs"].AsArray();
            if (arr == null) return result;

            for (int i = 0; i < arr.Count; i++)
            {
                var p = arr[i];
                if (!p.IsObject) continue;
                string id = p.Str("id", string.Empty);
                string sha = p.Str("sha256", string.Empty);
                if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(sha)) continue;
                result.Add(new ContentPackManifestItem
                {
                    Id = id,
                    Category = p.Str("category", string.Empty),
                    Sha256 = sha.ToLowerInvariant(),
                    Bytes = p.Int("bytes", 0)
                });
            }
            return result;
        }

        /// <summary>
        /// Pure manifest diff: returns remote entries that are missing locally or have a changed SHA-256 digest.
        /// </summary>
        public static List<ContentPackManifestItem> ComputeChangedPacks(
            IReadOnlyDictionary<string, string> localDigests,
            IReadOnlyList<ContentPackManifestItem> remoteItems)
        {
            var changed = new List<ContentPackManifestItem>();
            if (remoteItems == null) return changed;
            for (int i = 0; i < remoteItems.Count; i++)
            {
                var item = remoteItems[i];
                if (item == null || string.IsNullOrEmpty(item.Id) || string.IsNullOrEmpty(item.Sha256))
                    continue;
                if (localDigests != null &&
                    localDigests.TryGetValue(item.Id, out string existingSha) &&
                    string.Equals(existingSha, item.Sha256, StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }
                changed.Add(item);
            }
            return changed;
        }

        private static string SanitizeFileSlug(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw)) return "default";
            var sb = new StringBuilder(raw.Length);
            for (int i = 0; i < raw.Length; i++)
            {
                char c = raw[i];
                if (char.IsLetterOrDigit(c) || c == '.' || c == '_' || c == '-')
                    sb.Append(c);
                else
                    sb.Append('_');
            }
            return sb.ToString();
        }

        public void InitializeDiskCacheForServer(string baseHttpUrl)
        {
            _httpManifestSucceeded = false;
            _localDigests.Clear();

            string hostSlug = "local";
            if (!string.IsNullOrWhiteSpace(baseHttpUrl) && Uri.TryCreate(baseHttpUrl.Trim(), UriKind.Absolute, out Uri u))
                hostSlug = SanitizeFileSlug($"{u.Host}_{u.Port}");

            try
            {
                string persistent = Application.persistentDataPath;
                if (!string.IsNullOrEmpty(persistent))
                {
                    _cacheRootDir = Path.Combine(persistent, "TacSimContentCache", hostSlug);
                    Directory.CreateDirectory(Path.Combine(_cacheRootDir, "packs"));
                    LoadCachedPacksIntoMemory();
                }
            }
            catch
            {
                _cacheRootDir = null;
            }
        }

        private void LoadCachedPacksIntoMemory()
        {
            if (string.IsNullOrEmpty(_cacheRootDir)) return;
            string manifestPath = Path.Combine(_cacheRootDir, "manifest.json");
            if (!File.Exists(manifestPath)) return;

            try
            {
                var items = ParseManifestPacks(JsonNode.Parse(File.ReadAllText(manifestPath)));
                for (int i = 0; i < items.Count; i++)
                {
                    var item = items[i];
                    string packFile = Path.Combine(_cacheRootDir, "packs", SanitizeFileSlug(item.Id) + ".json");
                    if (!File.Exists(packFile)) continue;
                    string json = File.ReadAllText(packFile);
                    if (string.Equals(ComputeSha256Hex(json), item.Sha256, StringComparison.OrdinalIgnoreCase))
                    {
                        _localDigests[item.Id] = item.Sha256;
                        TacSimContentLoader.RegisterDynamicPack(item.Id, json);
                    }
                }
            }
            catch (Exception ex)
            {
                SafeLogWarn($"[TacSimContentSync] Failed reading local pack cache: {ex.Message}");
            }
        }

        private void PersistLocalManifest()
        {
            if (string.IsNullOrEmpty(_cacheRootDir)) return;
            try
            {
                var packsArr = JsonNode.NewArray();
                foreach (var kv in _localDigests)
                {
                    var row = JsonNode.NewObject();
                    row["id"] = JsonNode.FromString(kv.Key);
                    row["sha256"] = JsonNode.FromString(kv.Value);
                    packsArr.Add(row);
                }
                var doc = JsonNode.NewObject();
                doc["version"] = JsonNode.FromLong(1);
                doc["packs"] = packsArr;
                File.WriteAllText(Path.Combine(_cacheRootDir, "manifest.json"), doc.ToJson());
            }
            catch (Exception ex)
            {
                SafeLogWarn($"[TacSimContentSync] Failed saving manifest.json: {ex.Message}");
            }
        }

        public bool StoreVerifiedPack(string id, string expectedSha256, string json)
        {
            if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(json))
                return false;

            string actualSha = ComputeSha256Hex(json);
            if (!string.IsNullOrEmpty(expectedSha256) &&
                !string.Equals(actualSha, expectedSha256, StringComparison.OrdinalIgnoreCase))
            {
                SafeLogWarn($"[TacSimContentSync] Digest mismatch for pack '{id}': expected {expectedSha256}, got {actualSha}.");
                return false;
            }

            _localDigests[id] = actualSha;
            TacSimContentLoader.RegisterDynamicPack(id, json);

            if (!string.IsNullOrEmpty(_cacheRootDir))
            {
                try
                {
                    string packFile = Path.Combine(_cacheRootDir, "packs", SanitizeFileSlug(id) + ".json");
                    File.WriteAllText(packFile, json);
                }
                catch (Exception ex)
                {
                    SafeLogWarn($"[TacSimContentSync] Disk cache write failed for '{id}': {ex.Message}");
                }
            }
            return true;
        }

        public async Task SyncOverHttpAsync(string baseHttpUrl)
        {
            InitializeDiskCacheForServer(baseHttpUrl);
            string baseClean = MushAuthClient.NormalizeBaseUrl(baseHttpUrl);
            try
            {
                using (var resp = await SharedHttp.GetAsync($"{baseClean}/api/content/manifest"))
                {
                    if (resp.StatusCode == HttpStatusCode.NotFound)
                    {
                        // Server does not expose HTTP pack endpoint yet (e.g. pre-redeploy or split gateway); WS or bundled fallback handles it.
                        return;
                    }
                    if (!resp.IsSuccessStatusCode)
                        return;

                    string manifestJson = await resp.Content.ReadAsStringAsync();
                    var remoteItems = ParseManifestPacks(JsonNode.Parse(manifestJson));
                    if (remoteItems.Count == 0) return;

                    _httpManifestSucceeded = true;
                    var changed = ComputeChangedPacks(_localDigests, remoteItems);
                    if (changed.Count == 0)
                    {
                        OnSyncLog?.Invoke($"Content packs up-to-date ({remoteItems.Count} verified cached packs).");
                        return;
                    }

                    int updated = 0;
                    for (int i = 0; i < changed.Count; i++)
                    {
                        var item = changed[i];
                        string url = $"{baseClean}/api/content/packs/{Uri.EscapeDataString(item.Id)}?digest={Uri.EscapeDataString(item.Sha256)}";
                        using (var pResp = await SharedHttp.GetAsync(url))
                        {
                            if (!pResp.IsSuccessStatusCode) continue;
                            string packJson = await pResp.Content.ReadAsStringAsync();
                            if (StoreVerifiedPack(item.Id, item.Sha256, packJson))
                                updated++;
                        }
                    }

                    if (updated > 0)
                    {
                        PersistLocalManifest();
                        OnSyncLog?.Invoke($"Synced {updated} updated content pack(s) from mushd.");
                        OnPacksUpdated?.Invoke(updated);
                    }
                }
            }
            catch (Exception ex)
            {
                SafeLogWarn($"[TacSimContentSync] HTTP content check skipped ({ex.Message}); using cached/fallback packs.");
            }
        }

        private static void SafeLogWarn(string message)
        {
            try { Debug.LogWarning(message); } catch { /* Headless standalone test runner without Unity native runtime */ }
        }

        public void RequestWebSocketManifestIfNeeded(MushWsClient ws)
        {
            if (_httpManifestSucceeded || ws == null || !ws.IsConnected)
                return;
            if (!ws.HasAdvertisedPanel("tacsim.content"))
                return;

            var p = JsonNode.NewObject();
            p["op"] = JsonNode.FromString("manifest");
            ws.SendPanelSubscribe("tacsim.content", p);
        }

        public void HandleWebSocketContentPanel(MushWsClient ws, JsonNode data)
        {
            if (data == null || !data.IsObject) return;
            string op = data.Str("op", string.Empty);
            if (op == "manifest")
            {
                var remote = ParseManifestPacks(data);
                var changed = ComputeChangedPacks(_localDigests, remote);
                if (changed.Count == 0 || ws == null || !ws.IsConnected) return;

                for (int i = 0; i < changed.Count; i++)
                {
                    var p = JsonNode.NewObject();
                    p["op"] = JsonNode.FromString("pack");
                    p["id"] = JsonNode.FromString(changed[i].Id);
                    ws.SendPanelSubscribe("tacsim.content", p);
                }
            }
            else if (op == "pack" && data.Bool("found", false))
            {
                string id = data.Str("id", string.Empty);
                string sha = data.Str("sha256", string.Empty);
                string json = data.Str("json", string.Empty);
                if (StoreVerifiedPack(id, sha, json))
                {
                    PersistLocalManifest();
                    OnPacksUpdated?.Invoke(1);
                }
            }
        }
    }
}
