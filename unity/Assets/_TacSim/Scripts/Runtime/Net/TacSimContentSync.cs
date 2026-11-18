using System;
using System.Collections.Generic;
using System.IO;
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
    /// Dynamically synchronizes player-safe projected TacSim &amp; Atlas JSON content packs by SHA-256
    /// digest over the authenticated `tacsim.content` WebSocket panel feed (working identically in
    /// all-in-one `mushd` and split `mush-gateway` + `mush-server` deployments), caching verified
    /// packs under `Application.persistentDataPath/TacSimContentCache/&lt;host&gt;`.
    /// Paces in-flight `pack` queries (&lt;= 4 concurrent, min 0.45s batch spacing, with timeout
    /// retry) so the 30-token / 10-per-second session flood bucket is never exceeded.
    /// </summary>
    public sealed class TacSimContentSync
    {
        public const int MaxInFlightPackRequests = 4;
        public const int MaxPackAttempts = 3;
        private const float MinBatchIntervalSec = 0.45f;
        private const float RequestRetryTimeoutSec = 5.0f;

        private string _cacheRootDir;
        private readonly Dictionary<string, string> _localDigests = new Dictionary<string, string>(StringComparer.Ordinal);
        private readonly Queue<ContentPackManifestItem> _pendingPacks = new Queue<ContentPackManifestItem>();
        private readonly Dictionary<string, (ContentPackManifestItem item, float sentAt, int attempts)> _inFlightPacks =
            new Dictionary<string, (ContentPackManifestItem, float, int)>(StringComparer.Ordinal);
        private readonly Dictionary<string, int> _attemptCounts = new Dictionary<string, int>(StringComparer.Ordinal);
        private float _lastDispatchRealtime = -1000f;
        private bool _manifestReceived;

        public event Action<string> OnSyncLog;
        public event Action<int> OnPacksUpdated;

        public bool IsSyncComplete => _manifestReceived && _pendingPacks.Count == 0 && _inFlightPacks.Count == 0;
        public int PendingPackCount => _pendingPacks.Count + _inFlightPacks.Count;

        private static void SafeLogWarn(string msg)
        {
            try { Debug.LogWarning(msg); } catch { /* standalone managed test runner */ }
        }

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

        public void InitializeDiskCacheForServer(string baseHttpUrl, string explicitCacheDir = null)
        {
            _manifestReceived = false;
            _localDigests.Clear();
            _pendingPacks.Clear();
            _inFlightPacks.Clear();
            _attemptCounts.Clear();
            _lastDispatchRealtime = -1000f;
            // Clear any dynamic packs retained from a previous server connection before hydrating this server's cache.
            TacSimContentLoader.ClearDynamicPacks();

            string hostSlug = "local";
            string normalized = MushAuthClient.NormalizeBaseUrl(baseHttpUrl);
            if (!string.IsNullOrWhiteSpace(normalized) && Uri.TryCreate(normalized, UriKind.Absolute, out Uri u))
                hostSlug = SanitizeFileSlug($"{u.Host}_{u.Port}");

            try
            {
                if (!string.IsNullOrEmpty(explicitCacheDir))
                {
                    _cacheRootDir = Path.Combine(explicitCacheDir, hostSlug);
                }
                else
                {
                    string persistent = Application.persistentDataPath;
                    _cacheRootDir = !string.IsNullOrEmpty(persistent)
                        ? Path.Combine(persistent, "TacSimContentCache", hostSlug)
                        : null;
                }

                if (!string.IsNullOrEmpty(_cacheRootDir))
                {
                    Directory.CreateDirectory(Path.Combine(_cacheRootDir, "packs"));
                    int warmLoaded = LoadCachedPacksIntoMemory();
                    if (warmLoaded > 0)
                    {
                        OnSyncLog?.Invoke($"[ContentSync] Hydrated {warmLoaded} verified pack(s) from local server cache.");
                        OnPacksUpdated?.Invoke(warmLoaded);
                    }
                }
            }
            catch
            {
                _cacheRootDir = null;
            }
        }

        private int LoadCachedPacksIntoMemory()
        {
            if (string.IsNullOrEmpty(_cacheRootDir)) return 0;
            string manifestPath = Path.Combine(_cacheRootDir, "manifest.json");
            if (!File.Exists(manifestPath)) return 0;

            int loaded = 0;
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
                        loaded++;
                    }
                }
            }
            catch (Exception ex)
            {
                SafeLogWarn($"[TacSimContentSync] Failed reading local pack cache: {ex.Message}");
            }
            return loaded;
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
                doc["version"] = JsonNode.FromLong(2);
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

        public Task SyncOverHttpAsync(string baseHttpUrl)
        {
            // Content sync runs over authenticated WebSocket `tacsim.content` so split
            // `mush-gateway` + `mush-server` and all-in-one `mushd` behave identically.
            return Task.CompletedTask;
        }

        public void RequestManifestOverWebSocket(MushWsClient ws, float nowRealtime = 0f)
        {
            if (ws == null || !ws.IsConnected)
                return;
            if (!ws.HasAdvertisedPanel("tacsim.content"))
                return;

            _inFlightPacks.Clear();
            _pendingPacks.Clear();
            _attemptCounts.Clear();
            _manifestReceived = false;

            var p = JsonNode.NewObject();
            p["op"] = JsonNode.FromString("manifest");
            ws.SendPanelSubscribe("tacsim.content", p);
        }

        public void HandleWebSocketContentPanel(MushWsClient ws, JsonNode data, float nowRealtime = 0f)
        {
            if (data == null || !data.IsObject) return;
            string op = data.Str("op", string.Empty);
            if (op == "manifest")
            {
                _manifestReceived = true;
                var remote = ParseManifestPacks(data);
                PruneRemovedServerPacks(remote);
                var changed = ComputeChangedPacks(_localDigests, remote);
                _pendingPacks.Clear();
                _inFlightPacks.Clear();
                _attemptCounts.Clear();
                for (int i = 0; i < changed.Count; i++)
                    _pendingPacks.Enqueue(changed[i]);

                if (_pendingPacks.Count > 0)
                {
                    OnSyncLog?.Invoke($"[ContentSync] Syncing {_pendingPacks.Count} updated pack(s) over WebSocket...");
                    PumpPackQueue(ws, nowRealtime);
                }
            }
            else if (op == "pack")
            {
                string id = data.Str("id", string.Empty);
                string reqId = data.Str("requested_id", id);
                _inFlightPacks.Remove(id);
                if (!string.IsNullOrEmpty(reqId))
                    _inFlightPacks.Remove(reqId);

                if (data.Bool("found", false))
                {
                    string sha = data.Str("sha256", string.Empty);
                    string json = data.Str("json", string.Empty);
                    if (StoreVerifiedPack(id, sha, json))
                    {
                        _attemptCounts.Remove(id);
                        if (!string.IsNullOrEmpty(reqId))
                            _attemptCounts.Remove(reqId);
                        PersistLocalManifest();
                        OnPacksUpdated?.Invoke(1);
                    }
                }
                PumpPackQueue(ws, nowRealtime);
            }
        }

        private void PruneRemovedServerPacks(List<ContentPackManifestItem> remote)
        {
            var remoteIds = new HashSet<string>(StringComparer.Ordinal);
            for (int i = 0; i < remote.Count; i++)
                remoteIds.Add(remote[i].Id);

            var staleKeys = new List<string>();
            foreach (var kv in _localDigests)
            {
                if (!remoteIds.Contains(kv.Key))
                    staleKeys.Add(kv.Key);
            }
            if (staleKeys.Count == 0) return;

            for (int i = 0; i < staleKeys.Count; i++)
            {
                string key = staleKeys[i];
                _localDigests.Remove(key);
                if (!string.IsNullOrEmpty(_cacheRootDir))
                {
                    try
                    {
                        string packFile = Path.Combine(_cacheRootDir, "packs", SanitizeFileSlug(key) + ".json");
                        if (File.Exists(packFile))
                            File.Delete(packFile);
                    }
                    catch { /* ignore disk delete errors */ }
                }
            }

            // Persist the reduced manifest BEFORE reloading dynamic packs into memory!
            PersistLocalManifest();
            TacSimContentLoader.ClearDynamicPacks();
            LoadCachedPacksIntoMemory();
            OnPacksUpdated?.Invoke(staleKeys.Count);
        }

        /// <summary>
        /// Call from Update() to pace queued pack downloads within session rate limits and retry timed-out packs.
        /// Enforces <see cref="MinBatchIntervalSec"/> both within and between batches and caps retries at <see cref="MaxPackAttempts"/>.
        /// </summary>
        public void PumpPackQueue(MushWsClient ws, float nowRealtime)
        {
            if (ws == null) return;

            // Re-queue any in-flight pack that timed out (e.g. dropped by flood limiter or reconnect)
            if (_inFlightPacks.Count > 0)
            {
                List<string> timedOut = null;
                foreach (var kv in _inFlightPacks)
                {
                    if (nowRealtime - kv.Value.sentAt >= RequestRetryTimeoutSec)
                    {
                        if (timedOut == null) timedOut = new List<string>();
                        timedOut.Add(kv.Key);
                    }
                }
                if (timedOut != null)
                {
                    for (int i = 0; i < timedOut.Count; i++)
                    {
                        var entry = _inFlightPacks[timedOut[i]];
                        _inFlightPacks.Remove(timedOut[i]);
                        if (entry.attempts < MaxPackAttempts)
                            _pendingPacks.Enqueue(entry.item);
                    }
                }
            }

            if (_pendingPacks.Count == 0 || _inFlightPacks.Count >= MaxInFlightPackRequests)
                return;

            // Unconditionally enforce minimum interval between dispatch bursts, even after an earlier batch completes.
            if (nowRealtime - _lastDispatchRealtime < MinBatchIntervalSec)
                return;

            int sent = 0;
            while (_pendingPacks.Count > 0 && _inFlightPacks.Count < MaxInFlightPackRequests)
            {
                var next = _pendingPacks.Dequeue();
                if (_localDigests.TryGetValue(next.Id, out string have) &&
                    string.Equals(have, next.Sha256, StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }
                int attempt = (_attemptCounts.TryGetValue(next.Id, out int prev) ? prev : 0) + 1;
                if (attempt > MaxPackAttempts)
                    continue;
                _attemptCounts[next.Id] = attempt;
                _inFlightPacks[next.Id] = (next, nowRealtime, attempt);
                var p = JsonNode.NewObject();
                p["op"] = JsonNode.FromString("pack");
                p["id"] = JsonNode.FromString(next.Id);
                ws.SendPanelSubscribe("tacsim.content", p);
                sent++;
            }
            if (sent > 0)
                _lastDispatchRealtime = nowRealtime;
        }
    }
}
