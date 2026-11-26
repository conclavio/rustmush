using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Net.WebSockets;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using TacSim.Core;

namespace TacSim.Net
{
    public enum MushWsConnectionState
    {
        Disconnected,
        Connecting,
        Online,
        Reconnecting,
        SessionEnded
    }

    public enum WsCloseDisposition
    {
        ReconnectBackoff,
        Cooloff60s,
        FatalSessionEnded
    }

    /// <summary>
    /// Live WebSocket session client for mush-net-web (/ws?char=...&amp;resume=...) using
    /// Sec-WebSocket-Protocol: rustmush.v1.&lt;token&gt;.
    /// Implements ClientCommand::Caps handshake, exponential-backoff auto-reconnect with
    /// ?resume=&lt;last_seq&gt;, 4401/4429 close-code handling, ViewEnvelope cursor checks, and Composing.
    /// </summary>
    public sealed class MushWsClient : IDisposable
    {
        public const int CloseCodeBadSession = 4401;
        public const int CloseCodeTokenCooloff = 4429;
        public const int MaxInboundFrameBytes = 4 * 1024 * 1024;
        public const int MaxInboundMessageBytes = MaxInboundFrameBytes;
        public const long MaxInboundQueueBytes = 16L * 1024L * 1024L;
        public const int MaxOutboundFrameBytes = 272 * 1024;
        public const int MaxInboundQueueSize = 2048;
        public const int MaxInboundQueueCount = MaxInboundQueueSize;
        public const int MaxOutboundQueueCount = 256;
        public const int MaxOutboundQueueSize = MaxOutboundQueueCount;
        public const int MaxPanelFeeds = 64;
        public const int MaxViewCursors = 64;
        public const int MaxAdvertisedPanels = 128;
        public const int MaxReplyUnwrapDepth = 1;
        private const int ConnectTimeoutMs = 15000;
        private static readonly float[] BackoffSeconds = { 0.5f, 1.0f, 2.0f, 5.0f, 15.0f };
        private static readonly Regex Http401Regex = new Regex(
            @"(?<![0-9])(HTTP/\d(?:\.\d)?\s+401|status\s+code\s*:?\s*['""]?401['""]?|401\s*\(?\s*Unauthorized\s*\)?|close\s+code\s*4401|'\s*401\s*')(?![0-9])",
            RegexOptions.Compiled | RegexOptions.IgnoreCase);

        public static MushWsClient LatestInstance { get; private set; }
        public static event Action<MushWsClient> OnInstanceCreated;

        public MushWsClient()
        {
            LatestInstance = this;
            OnInstanceCreated?.Invoke(this);
        }

        private sealed class SocketRuntime
        {
            public readonly ClientWebSocket Socket;
            public readonly CancellationTokenSource Cts;
            public int RemainingLoops = 2;

            public SocketRuntime(ClientWebSocket socket, CancellationTokenSource cts)
            {
                Socket = socket;
                Cts = cts;
            }

            public void ReleaseLoop()
            {
                if (Interlocked.Decrement(ref RemainingLoops) == 0)
                {
                    try { Socket.Dispose(); } catch { /* ignore */ }
                    try { Cts.Dispose(); } catch { /* ignore */ }
                }
            }
        }

        private readonly object _socketLock = new object();
        private readonly object _inboundLock = new object();
        private readonly SemaphoreSlim _connectGate = new SemaphoreSlim(1, 1);
        private readonly SemaphoreSlim _outboundSignal = new SemaphoreSlim(0);
        private SocketRuntime _runtime;
        private readonly Queue<(long epoch, MushEnvelope env, int bytes)> _reliableInbound = new Queue<(long, MushEnvelope, int)>();
        private readonly Dictionary<string, (long epoch, MushEnvelope env, int bytes)> _latestTelemetry = new Dictionary<string, (long, MushEnvelope, int)>(StringComparer.Ordinal);
        private readonly Queue<string> _telemetryOrder = new Queue<string>();
        private long _inboundQueuedBytes;
        private bool _inboundOverflowDetected;
        private readonly ConcurrentQueue<string> _handshakeOutbound = new ConcurrentQueue<string>();
        private readonly ConcurrentQueue<string> _priorityOutbound = new ConcurrentQueue<string>();
        private readonly ConcurrentQueue<string> _outbound = new ConcurrentQueue<string>();
        private readonly ConcurrentQueue<(long epoch, int? code, string message)> _socketDrops = new ConcurrentQueue<(long, int?, string)>();
        private readonly Dictionary<string, JsonNode> _panelFeeds = new Dictionary<string, JsonNode>(StringComparer.Ordinal);
        private readonly Dictionary<string, ViewStreamCursor> _viewCursors = new Dictionary<string, ViewStreamCursor>(StringComparer.Ordinal);
        private readonly HashSet<string> _advertisedPanels = new HashSet<string>(StringComparer.Ordinal);

        private static readonly HashSet<string> ReplaceableTelemetryFeeds = new HashSet<string>(StringComparer.Ordinal)
        {
            "space.cockpit",
            "space.holotank",
            "space.docking",
            "arena.cockpit",
            "arena.observer",
            "arena.replay",
            "arena.map",
            "site.map",
            "tv.status",
            "sky.chart"
        };

        private MushAuthSession _activeAuth;
        private string _activeCharKey;
        private ulong _lastSeq;
        private ulong _nextQueryId = 1;
        private long _sessionEpoch;
        private volatile bool _isConnected;
        private bool _hasAttemptedConnect;
        private bool _subscribedForCurrentSocket;
        private bool _intentionalDisconnect = true;
        private bool _reconnectInFlight;
        private int _reconnectAttempt;
        private int _consecutiveParseErrors;
        private double _nextReconnectAtMonotonicSec = -1.0;

        private static double MonotonicSeconds() =>
            Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency;

        public bool IsConnected
        {
            get
            {
                if (!_isConnected) return false;
                lock (_socketLock)
                {
                    return _runtime != null && _runtime.Socket.State == WebSocketState.Open;
                }
            }
        }

        /// <summary>
        /// True when a live socket is open OR on a freshly constructed offline test client before any
        /// Connect/Disconnect lifecycle transition (preserving headless unit tests that inspect outbound queues).
        /// </summary>
        public bool CanEnqueueOutbound =>
            IsConnected || (!_hasAttemptedConnect && State == MushWsConnectionState.Disconnected);

        public MushWsConnectionState State { get; private set; } = MushWsConnectionState.Disconnected;
        public ulong LastSeq => _lastSeq;
        public int InboundQueueCount { get { lock (_inboundLock) { return _reliableInbound.Count + _latestTelemetry.Count; } } }
        public long InboundQueueBytes { get { lock (_inboundLock) { return _inboundQueuedBytes; } } }
        public int OutboundQueueCount => _handshakeOutbound.Count + _priorityOutbound.Count + _outbound.Count;
        public int PanelFeedCount => _panelFeeds.Count;
        public int ViewCursorCount => _viewCursors.Count;
        public int ReconnectAttemptCount => _reconnectAttempt;
        public bool InboundOverflowDetected { get { lock (_inboundLock) { return _inboundOverflowDetected; } } }
        public int PendingReliableInboundCount { get { lock (_inboundLock) { return _reliableInbound.Count; } } }
        public int PendingTelemetryInboundCount { get { lock (_inboundLock) { return _latestTelemetry.Count; } } }
        public MushObjRef ActiveCharacter { get; private set; }
        public string ConnectedServer { get; private set; }
        public IReadOnlyCollection<string> AdvertisedPanels => _advertisedPanels;

        public bool HasAdvertisedPanel(string feed) => !string.IsNullOrEmpty(feed) && _advertisedPanels.Contains(feed);

        public event Action<MushRoomView> OnRoomReceived;
        public event Action<MushNarrativeEvent> OnNarrativeReceived;
        public event Action<MushChannelEvent> OnChannelReceived;
        public event Action<MushComposingEvent> OnComposingReceived;
        public event Action<List<MushWhoEntry>> OnWhoReceived;
        public event Action<int> OnMailNoticeReceived;
        public event Action<string, string> OnSystemLineWithMarkupReceived; // (plainText, richMarkup)
        public event Action<string> OnSystemLineReceived;
        public event Action<string, JsonNode> OnPanelUpdated;
        public event Action<ulong, string, JsonNode> OnInvokeResultReceived; // (requestId, service, resultPayload)
        public event Action<MushServerRequest> OnServerRequestReceived;
        public event Action<string> OnConnectionStatusChanged;
        public event Action OnHelloReceived;
        public event Action<string> OnFatalSessionEnded;

        public static WsCloseDisposition ClassifyCloseCode(int? closeCode)
        {
            if (closeCode == CloseCodeBadSession)
                return WsCloseDisposition.FatalSessionEnded;
            if (closeCode == CloseCodeTokenCooloff)
                return WsCloseDisposition.Cooloff60s;
            return WsCloseDisposition.ReconnectBackoff;
        }

        private static bool HasUnauthorizedHttpStatus(Exception ex)
        {
            if (ex == null) return false;
            if (ex is WebException we && we.Response is HttpWebResponse hwr && hwr.StatusCode == HttpStatusCode.Unauthorized)
                return true;
            if (ex is HttpRequestException)
            {
                // Unity's .NET Standard 2.1 profile does not expose HttpRequestException.StatusCode at compile time;
                // inspect it dynamically so .NET 5+/8 runtimes still check StatusCode while Unity 6000 compiles cleanly.
                var prop = ex.GetType().GetProperty("StatusCode");
                if (prop != null)
                {
                    object val = prop.GetValue(ex, null);
                    if (val is HttpStatusCode code && code == HttpStatusCode.Unauthorized)
                        return true;
                }
            }
            return false;
        }

        public static bool IsAuthRejectionException(Exception ex)
        {
            for (Exception cur = ex; cur != null; cur = cur.InnerException)
            {
                if (HasUnauthorizedHttpStatus(cur))
                    return true;
                string msg = cur.Message;
                if (!string.IsNullOrEmpty(msg) && Http401Regex.IsMatch(msg))
                    return true;
            }
            return false;
        }

        public static string BuildWsQuery(string characterKey, ulong lastSeq)
        {
            string escapedChar = Uri.EscapeDataString(characterKey ?? string.Empty);
            // Fresh attach MUST omit &resume=... (ws.rs treats resume=0 as a protocol refusal if no session ring exists)
            return lastSeq > 0
                ? $"char={escapedChar}&resume={lastSeq}"
                : $"char={escapedChar}";
        }

        private static bool DefaultAllowInsecureNonLoopback()
        {
#if UNITY_EDITOR || DEVELOPMENT_BUILD
            return true;
#else
            return false;
#endif
        }

        public static Uri BuildValidatedWebSocketUri(
            string httpBaseUrl,
            string characterKey,
            ulong lastSeq,
            bool allowInsecureNonLoopback = false)
        {
            string wsBase = MushAuthClient.ToWebSocketBaseUrl(httpBaseUrl);
            if (MushProtocol.ContainsControlChars(wsBase) ||
                wsBase.IndexOf('?') >= 0 ||
                wsBase.IndexOf('#') >= 0)
            {
                throw new InvalidOperationException("WebSocket server base URL must not contain query strings, fragments, or control characters.");
            }
            if (!Uri.TryCreate(wsBase, UriKind.Absolute, out Uri baseUri))
                throw new InvalidOperationException("Invalid WebSocket server URL.");
            if (!string.IsNullOrEmpty(baseUri.UserInfo) ||
                !string.IsNullOrEmpty(baseUri.Query) ||
                !string.IsNullOrEmpty(baseUri.Fragment))
            {
                throw new InvalidOperationException("WebSocket server base URL must not contain credentials (userinfo), query strings, or fragments.");
            }

            bool isWss = string.Equals(baseUri.Scheme, "wss", StringComparison.OrdinalIgnoreCase);
            bool isWs = string.Equals(baseUri.Scheme, "ws", StringComparison.OrdinalIgnoreCase);
            if (!isWss && !isWs)
                throw new InvalidOperationException($"Unsupported WebSocket URL scheme '{baseUri.Scheme}'; use wss://.");

            if (!isWss && !allowInsecureNonLoopback && !MushAuthClient.IsLoopbackUri(baseUri))
            {
                throw new InvalidOperationException(
                    $"Refusing cleartext WebSocket connection ('{baseUri.Scheme}://{baseUri.Authority}') to non-loopback host in non-dev build; use https:// / wss://.");
            }

            string query = BuildWsQuery(characterKey, lastSeq);
            string basePath = baseUri.AbsolutePath.TrimEnd('/');
            var builder = new UriBuilder(baseUri)
            {
                UserName = string.Empty,
                Password = string.Empty,
                Path = basePath + "/ws",
                Query = query,
                Fragment = string.Empty
            };
            Uri uri = builder.Uri;

            if (!string.Equals(uri.Scheme, baseUri.Scheme, StringComparison.OrdinalIgnoreCase) ||
                !string.Equals(uri.Authority, baseUri.Authority, StringComparison.OrdinalIgnoreCase) ||
                !string.IsNullOrEmpty(uri.UserInfo))
            {
                throw new InvalidOperationException("WebSocket endpoint URI changed target origin.");
            }
            return uri;
        }

        public static bool TryValidateInboundFrame(
            WebSocketMessageType messageType,
            long accumulatedBytes,
            int chunkBytes,
            out WebSocketCloseStatus closeStatus,
            out string error)
        {
            if (messageType == WebSocketMessageType.Binary)
            {
                closeStatus = WebSocketCloseStatus.InvalidMessageType;
                error = "Rejected unsupported binary WebSocket frame.";
                return false;
            }
            if (chunkBytes < 0 || accumulatedBytes < 0 || accumulatedBytes + chunkBytes > MaxInboundMessageBytes)
            {
                closeStatus = WebSocketCloseStatus.MessageTooBig;
                error = $"Inbound WebSocket frame exceeded {MaxInboundMessageBytes} bytes.";
                return false;
            }
            closeStatus = WebSocketCloseStatus.NormalClosure;
            error = null;
            return true;
        }

        public async Task ConnectAsync(MushAuthSession auth, string characterKey, bool resetResume = true)
        {
            await _connectGate.WaitAsync();
            try
            {
                _hasAttemptedConnect = true;
                CloseSocketOnly();
                _activeAuth = auth;
                _activeCharKey = characterKey;
                _intentionalDisconnect = false;
                _reconnectInFlight = false;
                _nextReconnectAtMonotonicSec = -1.0;

                if (resetResume)
                {
                    _lastSeq = 0;
                    _reconnectAttempt = 0;
                    _consecutiveParseErrors = 0;
                    _panelFeeds.Clear();
                    _viewCursors.Clear();
                    _advertisedPanels.Clear();
                }

                await OpenSocketInternalAsync(isReconnect: !resetResume && _lastSeq > 0);
            }
            finally
            {
                _connectGate.Release();
            }
        }

        private async Task OpenSocketInternalAsync(bool isReconnect)
        {
            if (_activeAuth == null || string.IsNullOrEmpty(_activeAuth.Token))
                throw new InvalidOperationException("No active session credentials.");

            CloseSocketOnly();
            DrainQueue(_handshakeOutbound);
            DrainQueue(_priorityOutbound);
            DrainQueue(_outbound);

            long epoch = Interlocked.Increment(ref _sessionEpoch);
            ClearInboundQueues();
            DrainQueue(_socketDrops);
            _subscribedForCurrentSocket = false;

            var cts = new CancellationTokenSource();
            var ws = new ClientWebSocket();
            ws.Options.AddSubProtocol($"rustmush.v1.{_activeAuth.Token}");
            ws.Options.KeepAliveInterval = TimeSpan.FromSeconds(20);

            var uri = BuildValidatedWebSocketUri(
                _activeAuth.BaseHttpUrl,
                _activeCharKey,
                _lastSeq,
                DefaultAllowInsecureNonLoopback());

            State = isReconnect ? MushWsConnectionState.Reconnecting : MushWsConnectionState.Connecting;
            OnConnectionStatusChanged?.Invoke(isReconnect
                ? $"Reconnecting to {uri.Host} (resume seq {_lastSeq})..."
                : $"Connecting to {uri.Host}...");

            try
            {
                using (var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(cts.Token))
                {
                    timeoutCts.CancelAfter(ConnectTimeoutMs);
                    await ws.ConnectAsync(uri, timeoutCts.Token);
                }
            }
            catch
            {
                try { ws.Dispose(); } catch { /* ignore */ }
                try { cts.Dispose(); } catch { /* ignore */ }
                throw;
            }

            var rt = new SocketRuntime(ws, cts);
            bool activated = false;
            lock (_socketLock)
            {
                if (!_intentionalDisconnect && Interlocked.Read(ref _sessionEpoch) == epoch)
                {
                    _runtime = rt;
                    _isConnected = true;
                    activated = true;
                }
            }
            if (!activated)
            {
                try { ws.Abort(); } catch { /* ignore */ }
                try { ws.Dispose(); } catch { /* ignore */ }
                try { cts.Dispose(); } catch { /* ignore */ }
                return;
            }

            // NOTE (U-H04): Do NOT reset _reconnectAttempt here; wait until the server actually completes `hello`
            // so immediate 1012/1013 post-upgrade closes still obey exponential backoff instead of 2 Hz storms.
            _nextReconnectAtMonotonicSec = -1.0;
            State = MushWsConnectionState.Online;

            // Declare structured true_color Web client capabilities immediately upon attach (dedicated lane so ping floods cannot evict caps).
            EnqueueHandshakeRaw(MushProtocol.BuildCapsCommand(120, 40));
            OnConnectionStatusChanged?.Invoke($"Connected to {uri.Host}");

            _ = Task.Run(() => ReceiveLoopAsync(rt, epoch));
            _ = Task.Run(() => SendLoopAsync(rt, epoch));
        }

        public void Disconnect()
        {
            _hasAttemptedConnect = true;
            Interlocked.Increment(ref _sessionEpoch);
            _intentionalDisconnect = true;
            _activeAuth = null;
            _activeCharKey = null;
            _nextReconnectAtMonotonicSec = -1.0;
            _reconnectInFlight = false;
            _subscribedForCurrentSocket = false;
            State = MushWsConnectionState.Disconnected;
            ClearInboundQueues();
            DrainQueue(_socketDrops);
            DrainQueue(_handshakeOutbound);
            DrainQueue(_priorityOutbound);
            DrainQueue(_outbound);
            CloseSocketOnly();
        }

        private void CloseSocketOnly()
        {
            SocketRuntime prev;
            lock (_socketLock)
            {
                _isConnected = false;
                prev = _runtime;
                _runtime = null;
            }
            if (prev != null)
            {
                // Cancel + abort only; SocketRuntime.ReleaseLoop() disposes once both background loops exit.
                try { prev.Cts.Cancel(); } catch { /* ignore */ }
                try { prev.Socket.Abort(); } catch { /* ignore */ }
            }
        }

        public void Dispose()
        {
            Disconnect();
            if (ReferenceEquals(LatestInstance, this))
                LatestInstance = null;
        }

        public void SendLine(string commandLine)
        {
            if (string.IsNullOrWhiteSpace(commandLine) || MushProtocol.ContainsControlChars(commandLine))
                return;
            EnqueueRaw(MushProtocol.BuildLineCommand(commandLine.Trim()));
        }

        public void SendComposing(bool active)
        {
            EnqueueRaw(MushProtocol.BuildComposingCommand(active));
        }

        public ulong SendPanelSubscribe(string feed, JsonNode queryParams = null)
        {
            ulong id = _nextQueryId++;
            EnqueueRaw(MushProtocol.BuildPanelSubscribeCommand(id, feed, queryParams));
            return id;
        }

        public ulong SendWhoQuery()
        {
            ulong id = _nextQueryId++;
            EnqueueRaw(MushProtocol.BuildWhoQueryCommand(id));
            return id;
        }

        public ulong SendMailListQuery(string folder = null)
        {
            ulong id = _nextQueryId++;
            EnqueueRaw(MushProtocol.BuildMailListQueryCommand(id, folder));
            return id;
        }

        public ulong SendMindMarketListings(string afterCode = null)
        {
            ulong id = _nextQueryId++;
            string safeCode = MushProtocol.NormalizeListingCode(afterCode);
            EnqueueRaw(MushProtocol.BuildMindMarketQueryCommand(id, "listings", afterListingCode: safeCode));
            return id;
        }

        public ulong SendMindMarketHoldings(string section = "paid", long? afterNumber = null)
        {
            ulong id = _nextQueryId++;
            EnqueueRaw(MushProtocol.BuildMindMarketQueryCommand(id, "holdings", section: section, afterHoldingNumber: afterNumber));
            return id;
        }

        public ulong SendMindInvoke(JsonNode mindRequest)
        {
            ulong id = _nextQueryId++;
            string encoded = MushProtocol.BuildMindInvokeCommand(id, mindRequest);
            if (Encoding.UTF8.GetByteCount(encoded) > MaxOutboundFrameBytes)
                return 0;
            if (!EnqueueRaw(encoded))
                return 0;
            return id;
        }

        public void SendChoiceResponse(MushServerRequest req, string choiceKey)
        {
            if (req == null) return;
            EnqueueRaw(MushProtocol.BuildChoiceResponse(req.Id, choiceKey, req.Epoch, req.Kind));
        }

        public JsonNode GetPanelFeed(string feedId)
        {
            if (feedId != null && _panelFeeds.TryGetValue(feedId, out JsonNode val))
                return val;
            return JsonNode.Null;
        }

        public void InjectSyntheticEnvelope(MushEnvelope env)
        {
            if (env != null)
            {
                int estimatedBytes = env.Data != null ? Math.Max(64, env.Data.ToJson().Length) : 64;
                InjectSyntheticEnvelope(env, estimatedBytes);
            }
        }

        public bool InjectSyntheticEnvelope(MushEnvelope env, int estimatedBytes)
        {
            if (env == null) return true;
            _intentionalDisconnect = false;
            return EnqueueInboundBounded(Interlocked.Read(ref _sessionEpoch), env, Math.Max(1, estimatedBytes));
        }

        public bool TryDequeueOutbound(out string rawJson)
        {
            if (_handshakeOutbound.TryDequeue(out rawJson))
                return true;
            if (_priorityOutbound.TryDequeue(out rawJson))
                return true;
            return _outbound.TryDequeue(out rawJson);
        }

        public void PumpMainThread()
        {
            long currentEpoch = Interlocked.Read(ref _sessionEpoch);
            while (_socketDrops.TryDequeue(out var drop))
            {
                if (drop.epoch == currentEpoch)
                    HandleSocketDroppedOnMainThread(drop.code, drop.message);
            }

            if (_intentionalDisconnect)
            {
                ClearInboundQueues();
                return;
            }

            if (!_isConnected && !_reconnectInFlight && _nextReconnectAtMonotonicSec > 0.0 && MonotonicSeconds() >= _nextReconnectAtMonotonicSec)
            {
                _ = AttemptAutoReconnectAsync();
            }

            int processed = 0;
            while (processed < 64 && !_intentionalDisconnect && TryDequeueNextInbound(currentEpoch, out var env, out bool isCoalescedTelemetry))
            {
                processed++;
                // Never overwrite _lastSeq with 0 (out-of-band hello/resumed/ping frames carry seq: 0),
                // accept lower sequence numbers after a non-replayed fresh incarnation on reliable frames,
                // and never let coalesced telemetry rewind _lastSeq below already-dispatched reliable events
                // or advance _lastSeq past an overflow gap.
                if (env.Seq > 0)
                {
                    if (isCoalescedTelemetry)
                    {
                        if (!InboundOverflowDetected && env.Seq > _lastSeq)
                            _lastSeq = env.Seq;
                    }
                    else
                    {
                        _lastSeq = env.Seq;
                    }
                }
                DispatchEvent(env);
            }
        }

        private void HandleSocketDroppedOnMainThread(int? closeCode, string detail)
        {
            if (_intentionalDisconnect) return;

            string safeDetail = MushAuthClient.RedactSensitiveTokens(detail);
            var disp = ClassifyCloseCode(closeCode);
            if (disp == WsCloseDisposition.FatalSessionEnded)
            {
                _intentionalDisconnect = true;
                _activeAuth = null;
                _lastSeq = 0;
                State = MushWsConnectionState.SessionEnded;
                string reason = string.IsNullOrWhiteSpace(safeDetail) ? "Session ended or credentials expired." : safeDetail;
                OnConnectionStatusChanged?.Invoke($"Session ended (4401): {reason}");
                OnFatalSessionEnded?.Invoke(reason);
                return;
            }

            if (_activeAuth == null) return;

            float delaySec;
            if (disp == WsCloseDisposition.Cooloff60s)
            {
                delaySec = 60f;
            }
            else
            {
                int idx = Math.Min(_reconnectAttempt, BackoffSeconds.Length - 1);
                float baseDelay = BackoffSeconds[idx];
                // Deterministic lightweight +/-15% jitter avoids reconnect thundering herds.
                float jitterFactor = 0.85f + (((_reconnectAttempt * 37 + Environment.TickCount) & 0xFF) / 255f) * 0.30f;
                delaySec = baseDelay * jitterFactor;
                _reconnectAttempt++;
            }

            State = MushWsConnectionState.Reconnecting;
            _nextReconnectAtMonotonicSec = MonotonicSeconds() + delaySec;
            OnConnectionStatusChanged?.Invoke($"{safeDetail} — reconnecting in {delaySec:0.#}s...");
        }

        private async Task AttemptAutoReconnectAsync()
        {
            if (_intentionalDisconnect || _activeAuth == null || _reconnectInFlight)
                return;

            if (!await _connectGate.WaitAsync(0))
                return;

            _reconnectInFlight = true;
            _nextReconnectAtMonotonicSec = -1.0;
            try
            {
                await OpenSocketInternalAsync(isReconnect: true);
            }
            catch (Exception ex)
            {
                if (IsAuthRejectionException(ex))
                {
                    HandleSocketDroppedOnMainThread(CloseCodeBadSession, "Session token expired; please sign in again.");
                }
                else
                {
                    string safeMsg = MushAuthClient.RedactSensitiveTokens(ex.Message);
                    HandleSocketDroppedOnMainThread(null, $"Reconnect failed ({safeMsg})");
                }
            }
            finally
            {
                _reconnectInFlight = false;
                _connectGate.Release();
            }
        }

        private void SubscribeStandardPanels()
        {
            _subscribedForCurrentSocket = true;
            foreach (string feed in _advertisedPanels)
            {
                if (feed == "space.cockpit" || feed == "space.holotank" || feed == "space.docking" || feed == "sky.chart" ||
                    feed == "tv.status" || feed == "arena.cockpit" || feed == "arena.observer" || feed == "arena.replay" ||
                    feed == "scenes" || feed == "tactics.contacts")
                {
                    SendPanelSubscribe(feed);
                }
            }
        }

        private void DispatchEvent(MushEnvelope env)
        {
            if (env == null) return;
            DispatchEvent(env.EventType, env.Data, env.PongAlreadySent);
        }

        private void DispatchEvent(string type, JsonNode d, bool pongAlreadySent = false)
        {
            int replyDepth = 0;
            while (type == "reply")
            {
                if (replyDepth >= MaxReplyUnwrapDepth || d == null || !d.IsObject)
                    return;
                var inner = d["event"];
                if (inner == null || !inner.IsObject)
                    return;
                type = inner.Str("t", string.Empty);
                d = inner["d"];
                replyDepth++;
            }

            switch (type)
            {
                case "hello":
                    _reconnectAttempt = 0;
                    _consecutiveParseErrors = 0;
                    ConnectedServer = d.Str("server", "rustmush");
                    _advertisedPanels.Clear();
                    var panels = d["panels"].AsArray();
                    if (panels != null)
                    {
                        int cap = Math.Min(panels.Count, MaxAdvertisedPanels);
                        for (int i = 0; i < cap; i++)
                        {
                            string feed = panels[i].AsString();
                            if (!string.IsNullOrEmpty(feed))
                                _advertisedPanels.Add(feed);
                        }
                    }
                    SubscribeStandardPanels();
                    OnHelloReceived?.Invoke();
                    break;

                case "resumed":
                    _reconnectAttempt = 0;
                    if (!d.Bool("replayed", true))
                    {
                        _lastSeq = 0;
                        _panelFeeds.Clear();
                        _viewCursors.Clear();
                        if (!_subscribedForCurrentSocket)
                            SubscribeStandardPanels();
                        SendLine("look");
                    }
                    break;

                case "session_info":
                    ActiveCharacter = MushProtocol.ParseObjRef(d["character"]);
                    break;

                case "ping":
                    if (!pongAlreadySent)
                    {
                        ulong nonce = d.ULong("nonce", 0);
                        EnqueuePriorityRaw(MushProtocol.BuildPongCommand(nonce));
                    }
                    break;

                case "room":
                    var rv = MushProtocol.ParseRoomView(d);
                    if (rv != null) OnRoomReceived?.Invoke(rv);
                    break;

                case "narrative":
                    var nar = MushProtocol.ParseNarrative(d);
                    if (nar != null) OnNarrativeReceived?.Invoke(nar);
                    break;

                case "channel":
                    var ch = MushProtocol.ParseChannel(d);
                    if (ch != null) OnChannelReceived?.Invoke(ch);
                    break;

                case "composing":
                    var comp = MushProtocol.ParseComposing(d);
                    if (comp != null) OnComposingReceived?.Invoke(comp);
                    break;

                case "sys":
                    string sysPlain = MushProtocol.FlattenRichText(d);
                    string sysRich = MushProtocol.RenderRichTextToUnityMarkup(d);
                    if (!string.IsNullOrEmpty(sysPlain))
                    {
                        OnSystemLineWithMarkupReceived?.Invoke(sysPlain, sysRich);
                        OnSystemLineReceived?.Invoke(sysPlain);
                    }
                    break;

                case "notice":
                    string noticePlain = MushProtocol.FlattenRichText(d["text"]);
                    string noticeRich = MushProtocol.RenderRichTextToUnityMarkup(d["text"]);
                    if (!string.IsNullOrEmpty(noticePlain))
                    {
                        OnSystemLineWithMarkupReceived?.Invoke($"[NOTICE] {noticePlain}", $"<color=#FFCC66>[NOTICE]</color> {noticeRich}");
                        OnSystemLineReceived?.Invoke($"[NOTICE] {noticePlain}");
                    }
                    break;

                case "mail_notice":
                {
                    int unread = d.Int("unread", d.Int("count", 0));
                    OnMailNoticeReceived?.Invoke(unread);
                    string summary = MushProtocol.FlattenRichText(d["text"]);
                    string msg = !string.IsNullOrEmpty(summary)
                        ? $"[MAIL] {summary}"
                        : (unread > 0 ? $"[MAIL] You have {unread} unread message(s)." : "[MAIL] New mail notification.");
                    string rich = $"<color=#FFCC66>{MushProtocol.EscapeUnityRichText(msg)}</color>";
                    OnSystemLineWithMarkupReceived?.Invoke(msg, rich);
                    OnSystemLineReceived?.Invoke(msg);
                    break;
                }

                case "who":
                {
                    var whoList = MushProtocol.ParseWhoEntries(d);
                    OnWhoReceived?.Invoke(whoList);
                    if (whoList.Count > 0 || (d != null && d.IsObject && (d["entries"].IsArray || d["players"].IsArray)))
                    {
                        var names = new List<string>(whoList.Count);
                        for (int i = 0; i < whoList.Count; i++)
                            names.Add(whoList[i].Name);
                        string plain = $"[WHO] {names.Count} online" + (names.Count > 0 ? ": " + string.Join(", ", names) : ".");
                        string rich = $"<color=#99CCEE>{MushProtocol.EscapeUnityRichText(plain)}</color>";
                        OnSystemLineWithMarkupReceived?.Invoke(plain, rich);
                        OnSystemLineReceived?.Invoke(plain);
                    }
                    break;
                }

                case "prompt":
                {
                    string promptPlain = MushProtocol.FlattenRichText(d["text"].IsNull ? d : d["text"]);
                    string promptRich = MushProtocol.RenderRichTextToUnityMarkup(d["text"].IsNull ? d : d["text"]);
                    if (!string.IsNullOrEmpty(promptPlain))
                    {
                        OnSystemLineWithMarkupReceived?.Invoke(promptPlain, promptRich);
                        OnSystemLineReceived?.Invoke(promptPlain);
                    }
                    break;
                }

                case "weather":
                    string wxPlain = MushProtocol.FlattenRichText(d["text"]);
                    string wxRich = MushProtocol.RenderRichTextToUnityMarkup(d["text"]);
                    if (!string.IsNullOrEmpty(wxPlain))
                    {
                        OnSystemLineWithMarkupReceived?.Invoke(wxPlain, wxRich);
                        OnSystemLineReceived?.Invoke(wxPlain);
                    }
                    break;

                case "panel":
                    ApplyPanelFrame(d);
                    break;

                case "view":
                    ApplyViewEnvelope(d);
                    break;

                case "invoke_result":
                    if (d != null && d.IsObject && d["result"].IsObject)
                    {
                        ulong invId = d.ULong("id", 0);
                        string svc = d["result"].Str("service", string.Empty);
                        var resNode = d["result"]["result"];
                        OnInvokeResultReceived?.Invoke(invId, svc, resNode != null ? resNode.Clone() : JsonNode.Null);
                    }
                    break;

                case "request":
                    var req = MushProtocol.ParseRequest(d);
                    if (req != null) OnServerRequestReceived?.Invoke(req);
                    break;

                case "disconnect":
                    string reason = MushProtocol.FlattenRichText(d["reason"]);
                    _intentionalDisconnect = true;
                    _isConnected = false;
                    _activeAuth = null;
                    _activeCharKey = null;
                    State = MushWsConnectionState.SessionEnded;
                    OnConnectionStatusChanged?.Invoke($"Disconnected: {reason}");
                    OnFatalSessionEnded?.Invoke(reason);
                    break;
            }
        }

        private void ApplyPanelFrame(JsonNode d)
        {
            if (d == null || !d.IsObject) return;
            string feed = d.Str("feed", string.Empty);
            if (string.IsNullOrEmpty(feed)) return;
            string kind = d.Str("kind", "keyframe");
            var payload = d["data"];

            if (kind == "delta")
            {
                // Pulled TacSim panels emit full keyframes; if a feed signals a delta invalidation,
                // request a fresh keyframe rather than applying an invented shallow object merge.
                if (_advertisedPanels.Contains(feed))
                    SendPanelSubscribe(feed);
                return;
            }

            if (!_panelFeeds.ContainsKey(feed) && _panelFeeds.Count >= MaxPanelFeeds)
                return;

            var copy = payload != null ? payload.Clone() : JsonNode.NewObject();
            _panelFeeds[feed] = copy;
            OnPanelUpdated?.Invoke(feed, copy);
        }

        private void ApplyViewEnvelope(JsonNode d)
        {
            if (d == null || !d.IsObject) return;
            string viewId = d.Str("view", string.Empty);
            if (string.IsNullOrEmpty(viewId)) return;

            if (!_viewCursors.TryGetValue(viewId, out var cursor))
            {
                if (_viewCursors.Count >= MaxViewCursors)
                    return;
                cursor = new ViewStreamCursor();
                _viewCursors[viewId] = cursor;
            }

            var disp = MushProtocol.EvaluateViewCursor(cursor, d);
            switch (disp)
            {
                case ViewCursorDisposition.ApplySnapshot:
                {
                    if (!_panelFeeds.ContainsKey(viewId) && _panelFeeds.Count >= MaxPanelFeeds)
                        return;
                    var copy = d["data"] != null ? d["data"].Clone() : JsonNode.NewObject();
                    _panelFeeds[viewId] = copy;
                    OnPanelUpdated?.Invoke(viewId, copy);
                    break;
                }
                case ViewCursorDisposition.ApplyDelta:
                {
                    var payload = d["data"];
                    if (_panelFeeds.TryGetValue(viewId, out JsonNode existing) && existing.IsObject && payload != null && payload.IsObject)
                    {
                        existing.MergeObjectDelta(payload);
                        OnPanelUpdated?.Invoke(viewId, existing);
                    }
                    break;
                }
                case ViewCursorDisposition.NeedsResubscribe:
                {
                    _viewCursors.Remove(viewId);
                    if (_advertisedPanels.Contains(viewId))
                        SendPanelSubscribe(viewId);
                    break;
                }
                case ViewCursorDisposition.IgnoreDuplicateOrStale:
                    break;
            }
        }

        internal bool EnqueueHandshakeRaw(string json)
        {
            if (string.IsNullOrEmpty(json)) return false;
            if (Encoding.UTF8.GetByteCount(json) > MaxOutboundFrameBytes) return false;
            if (!CanEnqueueOutbound) return false;
            if (_handshakeOutbound.Count >= 8) return false;
            _handshakeOutbound.Enqueue(json);
            _outboundSignal.Release();
            return true;
        }

        private bool EnqueuePriorityRaw(string json)
        {
            if (string.IsNullOrEmpty(json)) return false;
            if (Encoding.UTF8.GetByteCount(json) > MaxOutboundFrameBytes) return false;
            if (!CanEnqueueOutbound) return false;
            while (_priorityOutbound.Count >= 64 && _priorityOutbound.TryDequeue(out _)) { }
            _priorityOutbound.Enqueue(json);
            _outboundSignal.Release();
            return true;
        }

        private bool EnqueueRaw(string json)
        {
            if (string.IsNullOrEmpty(json)) return false;
            if (Encoding.UTF8.GetByteCount(json) > MaxOutboundFrameBytes) return false;
            if (!CanEnqueueOutbound) return false;
            if (_outbound.Count >= MaxOutboundQueueSize)
                return false;
            _outbound.Enqueue(json);
            _outboundSignal.Release();
            return true;
        }

        public static bool TryGetReplaceableTelemetryKey(MushEnvelope env, out string key)
        {
            key = null;
            if (env == null || env.Data == null || !env.Data.IsObject)
                return false;

            if (env.EventType == "panel")
            {
                string kind = env.Data.Str("kind", "keyframe");
                if (kind == "delta")
                    return false;
                string feed = env.Data.Str("feed", string.Empty);
                if (!ReplaceableTelemetryFeeds.Contains(feed))
                    return false;
                var payload = env.Data["data"];
                if (feed == "space.docking" && payload != null && payload.IsObject &&
                    (payload.Bool("closed", false) || payload.Bool("refused", false)))
                    return false;
                if (feed == "tv.status" && payload != null && payload.IsObject && payload.Bool("cleared", false))
                    return false;
                key = "panel:" + feed;
                return true;
            }

            return false;
        }

        private static bool TryGetReplaceableViewId(MushEnvelope env, out string viewId)
        {
            viewId = null;
            if (env == null || env.EventType != "view" || env.Data == null || !env.Data.IsObject)
                return false;
            string id = env.Data.Str("view", string.Empty);
            if (!ReplaceableTelemetryFeeds.Contains(id))
                return false;
            viewId = id;
            return true;
        }

        private bool EnqueueInboundBounded(long epoch, MushEnvelope env, int frameBytes)
        {
            if (env == null) return true;
            int clampedBytes = Math.Max(1, Math.Min(frameBytes, MaxInboundMessageBytes));
            lock (_inboundLock)
            {
                if (_inboundOverflowDetected)
                    return false;

                if (TryGetReplaceableTelemetryKey(env, out string telemetryKey))
                {
                    int replacedBytes = 0;
                    bool isNewKey = !_latestTelemetry.TryGetValue(telemetryKey, out var existingTel);
                    if (!isNewKey)
                        replacedBytes = existingTel.bytes;
                    long nextTelBytes = _inboundQueuedBytes - replacedBytes + clampedBytes;
                    if (nextTelBytes > MaxInboundQueueBytes)
                    {
                        CompactReplaceableViewsLocked();
                        nextTelBytes = _inboundQueuedBytes - replacedBytes + clampedBytes;
                        if (nextTelBytes > MaxInboundQueueBytes)
                        {
                            _inboundOverflowDetected = true;
                            return false;
                        }
                    }
                    _latestTelemetry[telemetryKey] = (epoch, env, clampedBytes);
                    _inboundQueuedBytes = Math.Max(0L, nextTelBytes);
                    if (isNewKey)
                        _telemetryOrder.Enqueue(telemetryKey);
                    return true;
                }

                // Preserve causal ordering when a reliable terminal keyframe or reset event supersedes queued telemetry,
                // without letting a delta invalidation drop an earlier queued keyframe for the same feed.
                if (env.EventType == "panel" && env.Data != null && env.Data.IsObject &&
                    env.Data.Str("kind", "keyframe") != "delta")
                {
                    string feed = env.Data.Str("feed", string.Empty);
                    if (!string.IsNullOrEmpty(feed))
                    {
                        string tKey = "panel:" + feed;
                        if (_latestTelemetry.TryGetValue(tKey, out var removedTel))
                        {
                            _latestTelemetry.Remove(tKey);
                            _inboundQueuedBytes = Math.Max(0L, _inboundQueuedBytes - removedTel.bytes);
                        }
                    }
                }
                else if (env.EventType == "disconnect" ||
                         (env.EventType == "resumed" && env.Data != null && env.Data.IsObject && !env.Data.Bool("replayed", true)))
                {
                    foreach (var kv in _latestTelemetry)
                        _inboundQueuedBytes = Math.Max(0L, _inboundQueuedBytes - kv.Value.bytes);
                    _latestTelemetry.Clear();
                    _telemetryOrder.Clear();
                }

                if (_reliableInbound.Count >= MaxInboundQueueSize || _inboundQueuedBytes + clampedBytes > MaxInboundQueueBytes)
                    CompactReplaceableViewsLocked();

                if (_reliableInbound.Count >= MaxInboundQueueSize || _inboundQueuedBytes + clampedBytes > MaxInboundQueueBytes)
                {
                    // Never drop queued reliable events or advance _lastSeq past an omitted gap;
                    // latch overflow so ReceiveLoopAsync triggers a clean reconnect/resync from _lastSeq.
                    _inboundOverflowDetected = true;
                    return false;
                }

                _reliableInbound.Enqueue((epoch, env, clampedBytes));
                _inboundQueuedBytes += clampedBytes;
                return true;
            }
        }

        private void CompactReplaceableViewsLocked()
        {
            if (_reliableInbound.Count == 0) return;

            var items = _reliableInbound.ToArray();
            var lastIndexByView = new Dictionary<string, int>(StringComparer.Ordinal);
            for (int i = 0; i < items.Length; i++)
            {
                if (TryGetReplaceableViewId(items[i].env, out string viewId))
                    lastIndexByView[viewId] = i;
            }
            if (lastIndexByView.Count == 0)
                return;

            _reliableInbound.Clear();
            long reliableBytes = 0;
            for (int i = 0; i < items.Length; i++)
            {
                if (TryGetReplaceableViewId(items[i].env, out string viewId) && lastIndexByView[viewId] != i)
                    continue;
                _reliableInbound.Enqueue(items[i]);
                reliableBytes += items[i].bytes;
            }
            long telemetryBytes = 0;
            foreach (var kv in _latestTelemetry)
                telemetryBytes += kv.Value.bytes;
            _inboundQueuedBytes = reliableBytes + telemetryBytes;
        }

        private bool TryDequeueNextInbound(long currentEpoch, out MushEnvelope env, out bool isCoalescedTelemetry)
        {
            lock (_inboundLock)
            {
                while (_reliableInbound.Count > 0)
                {
                    var item = _reliableInbound.Dequeue();
                    _inboundQueuedBytes = Math.Max(0L, _inboundQueuedBytes - item.bytes);
                    if (item.epoch != currentEpoch)
                        continue;
                    env = item.env;
                    isCoalescedTelemetry = false;
                    return true;
                }

                while (_telemetryOrder.Count > 0)
                {
                    string key = _telemetryOrder.Dequeue();
                    if (!_latestTelemetry.TryGetValue(key, out var item))
                        continue;
                    _latestTelemetry.Remove(key);
                    _inboundQueuedBytes = Math.Max(0L, _inboundQueuedBytes - item.bytes);
                    if (item.epoch != currentEpoch)
                        continue;
                    env = item.env;
                    isCoalescedTelemetry = true;
                    return true;
                }
            }
            env = null;
            isCoalescedTelemetry = false;
            return false;
        }

        private void ClearInboundQueues()
        {
            lock (_inboundLock)
            {
                _reliableInbound.Clear();
                _latestTelemetry.Clear();
                _telemetryOrder.Clear();
                _inboundQueuedBytes = 0;
                _inboundOverflowDetected = false;
            }
        }

        private static void DrainQueue<T>(ConcurrentQueue<T> q)
        {
            while (q.TryDequeue(out _)) { }
        }

        private bool HandleSocketLoopFailureForEpoch(long epoch, SocketRuntime rt, int? closeCode, string message)
        {
            bool isCurrent;
            lock (_socketLock)
            {
                isCurrent = Interlocked.Read(ref _sessionEpoch) == epoch &&
                            (rt == null || ReferenceEquals(_runtime, rt));
                if (isCurrent)
                    _isConnected = false;
            }
            if (!isCurrent)
                return false;
            _socketDrops.Enqueue((epoch, closeCode, message));
            return true;
        }

        internal bool HandleSocketLoopFailureForEpoch(long epoch, int? closeCode, string message)
        {
            return HandleSocketLoopFailureForEpoch(epoch, null, closeCode, message);
        }

        private async Task ReceiveLoopAsync(SocketRuntime rt, long epoch)
        {
            var ws = rt.Socket;
            var ct = rt.Cts.Token;
            var buffer = new byte[16384];
            try
            {
                while (!ct.IsCancellationRequested && ws.State == WebSocketState.Open && Interlocked.Read(ref _sessionEpoch) == epoch)
                {
                    using (var ms = new MemoryStream())
                    {
                        WebSocketReceiveResult res;
                        do
                        {
                            res = await ws.ReceiveAsync(new ArraySegment<byte>(buffer), ct);
                            if (res.MessageType == WebSocketMessageType.Close)
                            {
                                int? closeCode = res.CloseStatus.HasValue ? (int)res.CloseStatus.Value : (int?)null;
                                string desc = !string.IsNullOrEmpty(res.CloseStatusDescription)
                                    ? MushAuthClient.RedactSensitiveTokens(res.CloseStatusDescription)
                                    : $"WebSocket closed ({(closeCode.HasValue ? closeCode.Value.ToString() : "EOF")})";
                                HandleSocketLoopFailureForEpoch(epoch, rt, closeCode, desc);
                                return;
                            }
                            if (!TryValidateInboundFrame(res.MessageType, ms.Length, res.Count, out WebSocketCloseStatus closeStatus, out string frameErr))
                            {
                                try { ws.Abort(); } catch { /* ignore */ }
                                HandleSocketLoopFailureForEpoch(epoch, rt, (int)closeStatus, frameErr);
                                return;
                            }
                            ms.Write(buffer, 0, res.Count);
                        } while (!res.EndOfMessage);

                        if (Interlocked.Read(ref _sessionEpoch) != epoch)
                            return;

                        if (res.MessageType == WebSocketMessageType.Text)
                        {
                            string json = Encoding.UTF8.GetString(ms.GetBuffer(), 0, (int)ms.Length);
                            try
                            {
                                var env = MushProtocol.ParseEnvelope(json);
                                if (env != null)
                                {
                                    _consecutiveParseErrors = 0;
                                    // Answer keepalive pings immediately on the priority lane so heavy frames never starve heartbeats.
                                    if (env.EventType == "ping" && env.Data != null && env.Data.IsObject && Interlocked.Read(ref _sessionEpoch) == epoch)
                                    {
                                        ulong nonce = env.Data.ULong("nonce", 0);
                                        EnqueuePriorityRaw(MushProtocol.BuildPongCommand(nonce));
                                        env.PongAlreadySent = true;
                                    }
                                    if (!EnqueueInboundBounded(epoch, env, (int)ms.Length))
                                    {
                                        try { ws.Abort(); } catch { /* ignore */ }
                                        HandleSocketLoopFailureForEpoch(epoch, rt, null, "Inbound reliable queue overflow; resyncing session");
                                        return;
                                    }
                                }
                            }
                            catch (Exception)
                            {
                                // Isolate malformed frame without tearing down the socket or trapping ?resume=<_lastSeq>.
                                if (Interlocked.Read(ref _sessionEpoch) == epoch)
                                {
                                    int errs = Interlocked.Increment(ref _consecutiveParseErrors);
                                    if (errs >= 3)
                                        _lastSeq = 0;
                                }
                            }
                        }
                    }
                }
            }
            catch (OperationCanceledException) { }
            catch (Exception ex)
            {
                string safeMsg = MushAuthClient.RedactSensitiveTokens(ex.Message);
                HandleSocketLoopFailureForEpoch(epoch, rt, null, $"WebSocket interrupted ({safeMsg})");
            }
            finally
            {
                rt.ReleaseLoop();
            }
        }

        private async Task SendLoopAsync(SocketRuntime rt, long epoch)
        {
            var ws = rt.Socket;
            var ct = rt.Cts.Token;
            try
            {
                while (!ct.IsCancellationRequested && ws.State == WebSocketState.Open && Interlocked.Read(ref _sessionEpoch) == epoch)
                {
                    if (_handshakeOutbound.TryDequeue(out string handshakeMsg))
                    {
                        byte[] bytes = Encoding.UTF8.GetBytes(handshakeMsg);
                        await ws.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, ct);
                        continue;
                    }
                    if (_priorityOutbound.TryDequeue(out string prioMsg))
                    {
                        byte[] bytes = Encoding.UTF8.GetBytes(prioMsg);
                        await ws.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, ct);
                        continue;
                    }
                    if (_outbound.TryDequeue(out string msg))
                    {
                        byte[] bytes = Encoding.UTF8.GetBytes(msg);
                        await ws.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, ct);
                        continue;
                    }
                    await _outboundSignal.WaitAsync(100, ct);
                }
            }
            catch (OperationCanceledException) { }
            catch (Exception ex)
            {
                try { ws.Abort(); } catch { /* ignore */ }
                string safeMsg = MushAuthClient.RedactSensitiveTokens(ex.Message);
                HandleSocketLoopFailureForEpoch(epoch, rt, null, $"WebSocket send failed ({safeMsg})");
            }
            finally
            {
                rt.ReleaseLoop();
            }
        }
    }
}
