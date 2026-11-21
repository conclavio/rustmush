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
        public const int MaxPackBytes = 2 * 1024 * 1024;
        public const long MaxTotalCacheBytes = 64L * 1024L * 1024L;
        public const int MaxCachedPacks = 256;
        private const float MinBatchIntervalSec = 0.45f;
        private const float RequestRetryTimeoutSec = 5.0f;

        private string _cacheRootDir;
        private readonly Dictionary<string, string> _localDigests = new Dictionary<string, string>(StringComparer.Ordinal);
        private readonly Dictionary<string, int> _localByteSizes = new Dictionary<string, int>(StringComparer.Ordinal);
        private readonly Dictionary<string, (string sha256, string json)> _cachedPackPayloads =
            new Dictionary<string, (string, string)>(StringComparer.Ordinal);
        private readonly Dictionary<string, string> _manifestPinnedDigests = new Dictionary<string, string>(StringComparer.Ordinal);
        private readonly Queue<ContentPackManifestItem> _pendingPacks = new Queue<ContentPackManifestItem>();
        private readonly Dictionary<string, (ContentPackManifestItem item, float sentAt, int attempts)> _inFlightPacks =
            new Dictionary<string, (ContentPackManifestItem, float, int)>(StringComparer.Ordinal);
        private readonly Dictionary<string, int> _attemptCounts = new Dictionary<string, int>(StringComparer.Ordinal);
        private readonly HashSet<string> _failedPackIds = new HashSet<string>(StringComparer.Ordinal);
        private readonly HashSet<string> _dirtyCategories = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        private int _syncedSinceLastNotify;
        private bool _manifestDirty;
        private float _lastDispatchRealtime = -1000f;
        private bool _manifestRequested;
        private float _manifestSentAtRealtime = -1000f;
        private int _manifestAttempts;
        private bool _manifestReceived;

        public event Action<string> OnSyncLog;
        public event Action<int> OnPacksUpdated;

        public IReadOnlyCollection<string> LastUpdatedCategories => _dirtyCategories;
        public IReadOnlyCollection<string> FailedPackIds => _failedPackIds;
        public bool HasFailedPacks => _failedPackIds.Count > 0;

        public bool IsSyncComplete => _manifestReceived && _pendingPacks.Count == 0 && _inFlightPacks.Count == 0 && _failedPackIds.Count == 0;
        public int PendingPackCount => _pendingPacks.Count + _inFlightPacks.Count;
        public int CachedPackCount => _localDigests.Count;

        public long CachedTotalBytes
        {
            get
            {
                long total = 0;
                foreach (var kv in _localByteSizes)
                    total += kv.Value;
                return total;
            }
        }

        private static void SafeLogWarn(string msg)
        {
            try { Debug.LogWarning(msg); } catch { /* standalone managed test runner */ }
        }

        public static bool IsValidSha256Hex(string sha)
        {
            if (string.IsNullOrEmpty(sha) || sha.Length != 64)
                return false;
            for (int i = 0; i < 64; i++)
            {
                char c = sha[i];
                bool isHex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
                if (!isHex) return false;
            }
            return true;
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

        /// <summary>
        /// Collision-free, OS-safe cache filename keyed by the SHA-256 hex digest of <paramref name="id"/>.
        /// Prevents slug collisions (e.g. <c>a/b</c> vs <c>a_b</c>), path traversal, and Windows reserved device names (<c>CON</c>, <c>NUL</c>, etc.).
        /// </summary>
        public static string CacheFileNameForPackId(string id)
        {
            return ComputeSha256Hex(id ?? string.Empty) + ".json";
        }

        private static string ComputeHostCacheDirName(string normalizedBaseUrl)
        {
            string key = "local";
            if (!string.IsNullOrWhiteSpace(normalizedBaseUrl) && Uri.TryCreate(normalizedBaseUrl, UriKind.Absolute, out Uri u))
                key = $"{u.Scheme.ToLowerInvariant()}://{u.Host.ToLowerInvariant()}:{u.Port}";
            return "host_" + ComputeSha256Hex(key).Substring(0, 24);
        }

        public void InitializeDiskCacheForServer(string baseHttpUrl, string explicitCacheDir = null)
        {
            _manifestReceived = false;
            _manifestRequested = false;
            _manifestSentAtRealtime = -1000f;
            _manifestAttempts = 0;
            _manifestDirty = false;
            _localDigests.Clear();
            _localByteSizes.Clear();
            _cachedPackPayloads.Clear();
            _manifestPinnedDigests.Clear();
            _pendingPacks.Clear();
            _inFlightPacks.Clear();
            _attemptCounts.Clear();
            _failedPackIds.Clear();
            _dirtyCategories.Clear();
            _dirtyCategories.Add("all");
            _syncedSinceLastNotify = 0;
            _lastDispatchRealtime = -1000f;
            // Clear any dynamic packs retained from a previous server or character session.
            // Cached disk bytes stay private to TacSimContentSync until the current session's live manifest authorizes each (id, sha256).
            TacSimContentLoader.ClearDynamicPacks();

            string normalized = MushAuthClient.NormalizeBaseUrl(baseHttpUrl);
            string hostSlug = ComputeHostCacheDirName(normalized);

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
                    int warmIndexed = LoadCachedPacksIntoPrivatePool();
                    if (warmIndexed > 0)
                        OnSyncLog?.Invoke($"[ContentSync] Indexed {warmIndexed} verified pack(s) in local server cache (awaiting session manifest).");
                }
            }
            catch
            {
                _cacheRootDir = null;
            }
            finally
            {
                // Always notify subscribers so previously loaded server/character models invalidate immediately.
                OnPacksUpdated?.Invoke(0);
            }
        }

        private int LoadCachedPacksIntoPrivatePool()
        {
            if (string.IsNullOrEmpty(_cacheRootDir)) return 0;
            string manifestPath = Path.Combine(_cacheRootDir, "manifest.json");
            if (!File.Exists(manifestPath)) return 0;

            int indexed = 0;
            long totalBytes = 0;
            try
            {
                var manifestInfo = new FileInfo(manifestPath);
                if (manifestInfo.Length > MaxPackBytes)
                    return 0;
                var items = ParseManifestPacks(JsonNode.Parse(File.ReadAllText(manifestPath, Encoding.UTF8)));
                int limit = Math.Min(items.Count, MaxCachedPacks);
                for (int i = 0; i < limit; i++)
                {
                    var item = items[i];
                    if (string.IsNullOrWhiteSpace(item.Id) || string.IsNullOrWhiteSpace(item.Sha256))
                        continue;
                    string packFile = Path.Combine(_cacheRootDir, "packs", CacheFileNameForPackId(item.Id));
                    if (!File.Exists(packFile)) continue;
                    var packInfo = new FileInfo(packFile);
                    if (packInfo.Length > MaxPackBytes) continue;
                    string json = File.ReadAllText(packFile, Encoding.UTF8);
                    int byteCount = Encoding.UTF8.GetByteCount(json);
                    if (byteCount > MaxPackBytes || totalBytes + byteCount > MaxTotalCacheBytes)
                        continue;
                    if (string.Equals(ComputeSha256Hex(json), item.Sha256, StringComparison.OrdinalIgnoreCase))
                    {
                        JsonNode parsedCached;
                        try { parsedCached = JsonNode.Parse(json); }
                        catch (Exception) { continue; }
                        if (parsedCached == null || !parsedCached.IsObject)
                            continue;
                        string lowerSha = item.Sha256.ToLowerInvariant();
                        _localDigests[item.Id] = lowerSha;
                        _localByteSizes[item.Id] = byteCount;
                        totalBytes += byteCount;
                        _cachedPackPayloads[item.Id] = (lowerSha, json);
                        indexed++;
                    }
                }
            }
            catch (Exception ex)
            {
                SafeLogWarn($"[TacSimContentSync] Failed reading local pack cache: {ex.Message}");
            }
            return indexed;
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
                string manifestPath = Path.Combine(_cacheRootDir, "manifest.json");
                string tmpPath = manifestPath + ".tmp";
                File.WriteAllText(tmpPath, doc.ToJson(), Encoding.UTF8);
                if (File.Exists(manifestPath))
                {
                    try
                    {
                        File.Replace(tmpPath, manifestPath, null);
                    }
                    catch
                    {
                        File.Delete(manifestPath);
                        File.Move(tmpPath, manifestPath);
                    }
                }
                else
                {
                    File.Move(tmpPath, manifestPath);
                }
                _manifestDirty = false;
            }
            catch (Exception ex)
            {
                SafeLogWarn($"[TacSimContentSync] Failed saving manifest.json: {ex.Message}");
            }
        }

        public bool StoreVerifiedPack(string id, string expectedSha256, string json)
        {
            if (string.IsNullOrWhiteSpace(id) || string.IsNullOrEmpty(json) || string.IsNullOrWhiteSpace(expectedSha256))
                return false;
            if (!IsValidSha256Hex(expectedSha256))
            {
                SafeLogWarn($"[TacSimContentSync] Missing or malformed SHA-256 digest for pack '{id}'.");
                return false;
            }

            int byteCount = Encoding.UTF8.GetByteCount(json);
            if (byteCount > MaxPackBytes)
            {
                SafeLogWarn($"[TacSimContentSync] Rejecting oversized pack '{id}' ({byteCount} bytes > {MaxPackBytes}).");
                return false;
            }

            string actualSha = ComputeSha256Hex(json);
            if (!string.Equals(actualSha, expectedSha256, StringComparison.OrdinalIgnoreCase))
            {
                SafeLogWarn($"[TacSimContentSync] Digest mismatch for pack '{id}': expected {expectedSha256}, got {actualSha}.");
                return false;
            }

            // Validate JSON syntax before caching, persisting, or registering into live loaders (P2-CLIENT-11).
            try
            {
                var parsed = JsonNode.Parse(json);
                if (parsed == null || !parsed.IsObject)
                {
                    SafeLogWarn($"[TacSimContentSync] Rejecting non-object JSON payload for pack '{id}'.");
                    return false;
                }
            }
            catch (Exception ex)
            {
                SafeLogWarn($"[TacSimContentSync] Rejecting malformed JSON payload for pack '{id}': {ex.Message}");
                return false;
            }

            bool isUpdate = _localDigests.ContainsKey(id);
            if (!isUpdate && _localDigests.Count >= MaxCachedPacks)
            {
                SafeLogWarn($"[TacSimContentSync] Cache file-count quota reached ({MaxCachedPacks}); rejecting pack '{id}'.");
                return false;
            }

            int previousBytes = _localByteSizes.TryGetValue(id, out int prev) ? prev : 0;
            long projectedTotalBytes = CachedTotalBytes - previousBytes + byteCount;
            if (projectedTotalBytes > MaxTotalCacheBytes)
            {
                SafeLogWarn($"[TacSimContentSync] Total cache byte quota exceeded ({projectedTotalBytes} > {MaxTotalCacheBytes}); rejecting pack '{id}'.");
                return false;
            }

            _localDigests[id] = actualSha;
            _localByteSizes[id] = byteCount;
            _cachedPackPayloads[id] = (actualSha, json);
            TacSimContentLoader.RegisterDynamicPack(id, json);

            if (!string.IsNullOrEmpty(_cacheRootDir))
            {
                try
                {
                    string packFile = Path.Combine(_cacheRootDir, "packs", CacheFileNameForPackId(id));
                    string tmpPackFile = packFile + ".tmp";
                    File.WriteAllText(tmpPackFile, json, Encoding.UTF8);
                    if (File.Exists(packFile))
                    {
                        try { File.Replace(tmpPackFile, packFile, null); }
                        catch { File.Delete(packFile); File.Move(tmpPackFile, packFile); }
                    }
                    else
                    {
                        File.Move(tmpPackFile, packFile);
                    }
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
            if (ws == null || !ws.CanEnqueueOutbound)
                return;
            if (ws.IsConnected && !ws.HasAdvertisedPanel("tacsim.content"))
                return;

            _inFlightPacks.Clear();
            _pendingPacks.Clear();
            _attemptCounts.Clear();
            _failedPackIds.Clear();
            _manifestReceived = false;
            _manifestRequested = true;
            _manifestSentAtRealtime = nowRealtime;
            _manifestAttempts = 1;

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
                _manifestRequested = false;
                var remote = ParseManifestPacks(data);
                if (remote.Count > MaxCachedPacks)
                    remote = remote.GetRange(0, MaxCachedPacks);
                _manifestPinnedDigests.Clear();
                _pendingPacks.Clear();
                _inFlightPacks.Clear();
                _attemptCounts.Clear();
                _failedPackIds.Clear();

                // Reset loader dynamic packs so ONLY packs explicitly authorized by this session's
                // live manifest (matching both pack ID and exact SHA-256 digest) become active.
                TacSimContentLoader.ClearDynamicPacks();

                var remoteIds = new HashSet<string>(StringComparer.Ordinal);
                int warmAuthorized = 0;
                int prunedCount = 0;

                for (int i = 0; i < remote.Count; i++)
                {
                    var item = remote[i];
                    if (item == null || string.IsNullOrEmpty(item.Id) || string.IsNullOrEmpty(item.Sha256))
                        continue;
                    if (!remoteIds.Add(item.Id))
                        continue;

                    _manifestPinnedDigests[item.Id] = item.Sha256;

                    if (_cachedPackPayloads.TryGetValue(item.Id, out var cached) &&
                        string.Equals(cached.sha256, item.Sha256, StringComparison.OrdinalIgnoreCase) &&
                        string.Equals(ComputeSha256Hex(cached.json), item.Sha256, StringComparison.OrdinalIgnoreCase))
                    {
                        TacSimContentLoader.RegisterDynamicPack(item.Id, cached.json);
                        warmAuthorized++;
                        if (!string.IsNullOrEmpty(item.Category))
                            _dirtyCategories.Add(item.Category);
                        else
                            _dirtyCategories.Add("all");
                    }
                    else
                    {
                        if (_cachedPackPayloads.ContainsKey(item.Id) || _localDigests.ContainsKey(item.Id))
                        {
                            RemoveCachedPackEntry(item.Id);
                            prunedCount++;
                            _manifestDirty = true;
                        }
                        _pendingPacks.Enqueue(item);
                    }
                }

                // Prune any cached pack IDs omitted from the current session's live manifest
                // (e.g., retired server packs or staff-only packs cached by a prior character).
                prunedCount += PruneRemovedServerPacks(remoteIds);

                if (_manifestDirty)
                    PersistLocalManifest();

                if (_pendingPacks.Count > 0)
                {
                    _syncedSinceLastNotify += warmAuthorized + prunedCount;
                    OnSyncLog?.Invoke($"[ContentSync] Syncing {_pendingPacks.Count} updated pack(s) over WebSocket...");
                    PumpPackQueue(ws, nowRealtime);
                }
                else if (warmAuthorized > 0 || prunedCount > 0)
                {
                    if (warmAuthorized > 0)
                        OnSyncLog?.Invoke($"[ContentSync] Authorized {warmAuthorized} cached pack(s) from live session manifest.");
                    OnPacksUpdated?.Invoke(warmAuthorized + prunedCount);
                    _dirtyCategories.Clear();
                }
            }
            else if (op == "pack")
            {
                string id = data.Str("id", string.Empty);
                string reqId = data.Str("requested_id", id);

                (ContentPackManifestItem item, float sentAt, int attempts) inFlight;
                string inFlightKey = null;
                if (!string.IsNullOrEmpty(reqId) && _inFlightPacks.TryGetValue(reqId, out inFlight))
                {
                    inFlightKey = reqId;
                }
                else if (!string.IsNullOrEmpty(id) && _inFlightPacks.TryGetValue(id, out inFlight))
                {
                    inFlightKey = id;
                }
                else
                {
                    SafeLogWarn($"[TacSimContentSync] Ignoring unsolicited pack frame '{id}' (requested_id='{reqId}').");
                    return;
                }

                _inFlightPacks.Remove(inFlightKey);
                if (!string.IsNullOrEmpty(id) && id != inFlightKey)
                    _inFlightPacks.Remove(id);
                if (!string.IsNullOrEmpty(reqId) && reqId != inFlightKey)
                    _inFlightPacks.Remove(reqId);

                bool stored = false;
                if (data.Bool("found", false))
                {
                    string manifestSha = inFlight.item != null ? inFlight.item.Sha256 : string.Empty;
                    string replySha = data.Str("sha256", string.Empty);
                    string json = data.Str("json", string.Empty);
                    string cat = inFlight.item != null && !string.IsNullOrEmpty(inFlight.item.Category)
                        ? inFlight.item.Category
                        : data.Str("category", string.Empty);
                    string storeId = inFlight.item != null && !string.IsNullOrEmpty(inFlight.item.Id)
                        ? inFlight.item.Id
                        : id;

                    bool replyDisagreesWithPin = !string.IsNullOrEmpty(replySha) &&
                                                 !string.IsNullOrEmpty(manifestSha) &&
                                                 !string.Equals(replySha, manifestSha, StringComparison.OrdinalIgnoreCase);

                    if (!IsValidSha256Hex(manifestSha) || replyDisagreesWithPin)
                    {
                        SafeLogWarn($"[TacSimContentSync] Rejecting pack '{inFlightKey}': digest does not match manifest entry.");
                        OnSyncLog?.Invoke($"[ContentSync] Rejected pack '{storeId}': SHA-256 did not match manifest pin.");
                    }
                    else if (StoreVerifiedPack(storeId, manifestSha, json))
                    {
                        stored = true;
                        _attemptCounts.Remove(storeId);
                        _failedPackIds.Remove(storeId);
                        if (!string.IsNullOrEmpty(id))
                        {
                            _attemptCounts.Remove(id);
                            _failedPackIds.Remove(id);
                        }
                        if (!string.IsNullOrEmpty(reqId))
                        {
                            _attemptCounts.Remove(reqId);
                            _failedPackIds.Remove(reqId);
                        }
                        if (!string.IsNullOrEmpty(cat))
                            _dirtyCategories.Add(cat);
                        else
                            _dirtyCategories.Add("all");
                        _syncedSinceLastNotify++;
                        _manifestDirty = true;
                    }
                    else
                    {
                        OnSyncLog?.Invoke($"[ContentSync] Rejected pack '{storeId}': SHA-256 did not match manifest pin.");
                    }
                }
                else
                {
                    OnSyncLog?.Invoke($"[ContentSync] Pack '{(!string.IsNullOrEmpty(id) ? id : reqId)}' returned found=false.");
                }

                if (!stored && inFlight.item != null)
                {
                    if (inFlight.attempts < MaxPackAttempts)
                    {
                        _pendingPacks.Enqueue(inFlight.item);
                    }
                    else
                    {
                        _failedPackIds.Add(inFlight.item.Id);
                        OnSyncLog?.Invoke($"[ContentSync] Pack '{inFlight.item.Id}' failed after {MaxPackAttempts} attempts.");
                    }
                }

                PumpPackQueue(ws, nowRealtime);
                FlushCompletedSyncNotification();
            }
        }

        private void FlushCompletedSyncNotification()
        {
            // Coalesce burst pack downloads so a cold sync of dozens of packs triggers one manifest write & scene rebuild at completion!
            if (_pendingPacks.Count == 0 && _inFlightPacks.Count == 0)
            {
                if (_manifestDirty)
                    PersistLocalManifest();
                if (_syncedSinceLastNotify > 0)
                {
                    int count = _syncedSinceLastNotify;
                    _syncedSinceLastNotify = 0;
                    OnPacksUpdated?.Invoke(count);
                    _dirtyCategories.Clear();
                }
            }
        }

        private void RemoveCachedPackEntry(string key)
        {
            _localDigests.Remove(key);
            _localByteSizes.Remove(key);
            _cachedPackPayloads.Remove(key);
            if (!string.IsNullOrEmpty(_cacheRootDir))
            {
                try
                {
                    string packFile = Path.Combine(_cacheRootDir, "packs", CacheFileNameForPackId(key));
                    if (File.Exists(packFile))
                        File.Delete(packFile);
                }
                catch { /* ignore disk delete errors */ }
            }
        }

        private int PruneRemovedServerPacks(HashSet<string> remoteIds)
        {
            var staleKeys = new List<string>();
            foreach (var kv in _localDigests)
            {
                if (!remoteIds.Contains(kv.Key))
                    staleKeys.Add(kv.Key);
            }
            if (staleKeys.Count == 0) return 0;

            for (int i = 0; i < staleKeys.Count; i++)
                RemoveCachedPackEntry(staleKeys[i]);

            _dirtyCategories.Add("all");
            _manifestDirty = true;
            return staleKeys.Count;
        }

        /// <summary>
        /// Call from Update() to pace queued pack downloads within session rate limits and retry timed-out packs.
        /// Enforces <see cref="MinBatchIntervalSec"/> both within and between batches and caps retries at <see cref="MaxPackAttempts"/>.
        /// </summary>
        public void PumpPackQueue(MushWsClient ws, float nowRealtime)
        {
            if (ws == null || !ws.CanEnqueueOutbound)
                return;

            // Retry a dropped initial manifest query when online.
            if (_manifestRequested && !_manifestReceived)
            {
                if (_manifestAttempts < MaxPackAttempts &&
                    nowRealtime - _manifestSentAtRealtime >= RequestRetryTimeoutSec &&
                    (!ws.IsConnected || ws.HasAdvertisedPanel("tacsim.content")))
                {
                    _manifestAttempts++;
                    _manifestSentAtRealtime = nowRealtime;
                    var mp = JsonNode.NewObject();
                    mp["op"] = JsonNode.FromString("manifest");
                    ws.SendPanelSubscribe("tacsim.content", mp);
                    OnSyncLog?.Invoke($"[ContentSync] Retrying content manifest request (attempt {_manifestAttempts}/{MaxPackAttempts})...");
                }
                return;
            }

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
                        {
                            _pendingPacks.Enqueue(entry.item);
                        }
                        else
                        {
                            _failedPackIds.Add(entry.item.Id);
                            OnSyncLog?.Invoke($"[ContentSync] Pack '{entry.item.Id}' timed out after {MaxPackAttempts} attempts.");
                        }
                    }
                    FlushCompletedSyncNotification();
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
                {
                    _failedPackIds.Add(next.Id);
                    continue;
                }
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
