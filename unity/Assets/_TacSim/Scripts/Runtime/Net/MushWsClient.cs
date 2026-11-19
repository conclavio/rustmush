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
        public const int MaxOutboundFrameBytes = 272 * 1024;
        public const int MaxInboundQueueSize = 2048;
        public const int MaxOutboundQueueSize = 512;
        private const int ConnectTimeoutMs = 15000;
        private static readonly float[] BackoffSeconds = { 0.5f, 1.0f, 2.0f, 5.0f, 15.0f };
        private static readonly Regex Http401Regex = new Regex(
            @"(?<![0-9])(HTTP/\d(?:\.\d)?\s+401|status\s+code\s*['""]?401['""]?|401\s+Unauthorized|close\s+code\s*4401|'\s*401\s*')(?![0-9])",
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
        private readonly SemaphoreSlim _connectGate = new SemaphoreSlim(1, 1);
        private readonly SemaphoreSlim _outboundSignal = new SemaphoreSlim(0);
        private SocketRuntime _runtime;
        private readonly ConcurrentQueue<(long epoch, MushEnvelope env)> _inbound = new ConcurrentQueue<(long, MushEnvelope)>();
        private readonly ConcurrentQueue<string> _priorityOutbound = new ConcurrentQueue<string>();
        private readonly ConcurrentQueue<string> _outbound = new ConcurrentQueue<string>();
        private readonly ConcurrentQueue<(long epoch, int? code, string message)> _socketDrops = new ConcurrentQueue<(long, int?, string)>();
        private readonly Dictionary<string, JsonNode> _panelFeeds = new Dictionary<string, JsonNode>(StringComparer.Ordinal);
        private readonly Dictionary<string, ViewStreamCursor> _viewCursors = new Dictionary<string, ViewStreamCursor>(StringComparer.Ordinal);
        private readonly HashSet<string> _advertisedPanels = new HashSet<string>(StringComparer.Ordinal);

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
        public int OutboundQueueCount => _priorityOutbound.Count + _outbound.Count;
        public int ReconnectAttemptCount => _reconnectAttempt;
        public MushObjRef ActiveCharacter { get; private set; }
        public string ConnectedServer { get; private set; }
        public IReadOnlyCollection<string> AdvertisedPanels => _advertisedPanels;

        public bool HasAdvertisedPanel(string feed) => !string.IsNullOrEmpty(feed) && _advertisedPanels.Contains(feed);

        public event Action<MushRoomView> OnRoomReceived;
        public event Action<MushNarrativeEvent> OnNarrativeReceived;
        public event Action<MushChannelEvent> OnChannelReceived;
        public event Action<MushComposingEvent> OnComposingReceived;
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

        public static bool IsAuthRejectionException(Exception ex)
        {
            for (Exception cur = ex; cur != null; cur = cur.InnerException)
            {
                if (cur is HttpRequestException hre && hre.StatusCode == HttpStatusCode.Unauthorized)
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
            DrainQueue(_priorityOutbound);
            DrainQueue(_outbound);

            long epoch = Interlocked.Increment(ref _sessionEpoch);
            DrainQueue(_inbound);
            DrainQueue(_socketDrops);
            _subscribedForCurrentSocket = false;

            var cts = new CancellationTokenSource();
            var ws = new ClientWebSocket();
            ws.Options.AddSubProtocol($"rustmush.v1.{_activeAuth.Token}");
            ws.Options.KeepAliveInterval = TimeSpan.FromSeconds(20);

            string wsBase = MushAuthClient.ToWebSocketBaseUrl(_activeAuth.BaseHttpUrl);
            string query = BuildWsQuery(_activeCharKey, _lastSeq);
            var uri = new Uri($"{wsBase}/ws?{query}");

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

            if (_intentionalDisconnect || Interlocked.Read(ref _sessionEpoch) != epoch)
            {
                try { ws.Abort(); } catch { /* ignore */ }
                try { ws.Dispose(); } catch { /* ignore */ }
                try { cts.Dispose(); } catch { /* ignore */ }
                return;
            }

            var rt = new SocketRuntime(ws, cts);
            lock (_socketLock)
            {
                _runtime = rt;
            }

            _isConnected = true;
            // NOTE (U-H04): Do NOT reset _reconnectAttempt here; wait until the server actually completes `hello`
            // so immediate 1012/1013 post-upgrade closes still obey exponential backoff instead of 2 Hz storms.
            _nextReconnectAtMonotonicSec = -1.0;
            State = MushWsConnectionState.Online;

            // Declare structured true_color Web client capabilities immediately upon attach
            EnqueuePriorityRaw(MushProtocol.BuildCapsCommand(120, 40));
            OnConnectionStatusChanged?.Invoke($"Connected to {uri.Host}");

            _ = Task.Run(() => ReceiveLoopAsync(rt, epoch));
            _ = Task.Run(() => SendLoopAsync(rt, epoch));
        }

        public void Disconnect()
        {
            _hasAttemptedConnect = true;
            Interlocked.Increment(ref _sessionEpoch);
            _intentionalDisconnect = true;
            _nextReconnectAtMonotonicSec = -1.0;
            _reconnectInFlight = false;
            _subscribedForCurrentSocket = false;
            State = MushWsConnectionState.Disconnected;
            DrainQueue(_inbound);
            DrainQueue(_socketDrops);
            DrainQueue(_priorityOutbound);
            DrainQueue(_outbound);
            CloseSocketOnly();
        }

        private void CloseSocketOnly()
        {
            _isConnected = false;
            SocketRuntime prev;
            lock (_socketLock)
            {
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
            if (string.IsNullOrWhiteSpace(commandLine)) return;
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
                _intentionalDisconnect = false;
                EnqueueInboundBounded(Interlocked.Read(ref _sessionEpoch), env);
            }
        }

        public bool TryDequeueOutbound(out string rawJson)
        {
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
                DrainQueue(_inbound);
                return;
            }

            if (!_isConnected && !_reconnectInFlight && _nextReconnectAtMonotonicSec > 0.0 && MonotonicSeconds() >= _nextReconnectAtMonotonicSec)
            {
                _ = AttemptAutoReconnectAsync();
            }

            int processed = 0;
            while (processed < 64 && _inbound.TryDequeue(out var item))
            {
                if (item.epoch != currentEpoch || _intentionalDisconnect)
                    continue;
                processed++;
                var env = item.env;
                // Never overwrite _lastSeq with 0 (out-of-band hello/resumed/ping frames carry seq: 0),
                // but accept lower sequence numbers after a non-replayed fresh incarnation.
                if (env.Seq > 0)
                    _lastSeq = env.Seq;
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
                    feed == "tv.status" || feed == "arena.cockpit" || feed == "arena.observer" || feed == "arena.replay")
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
                        for (int i = 0; i < panels.Count; i++)
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
                    var entries = d["entries"].AsArray() ?? d["players"].AsArray();
                    if (entries != null)
                    {
                        var names = new List<string>(entries.Count);
                        for (int i = 0; i < entries.Count; i++)
                        {
                            var e = entries[i];
                            string nm = e.IsObject ? e.Str("name", e.Str("character", string.Empty)) : e.AsString(string.Empty);
                            if (!string.IsNullOrEmpty(nm)) names.Add(nm);
                        }
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

                case "reply":
                    var inner = d["event"];
                    if (inner.IsObject)
                        DispatchEvent(inner.Str("t", string.Empty), inner["d"]);
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
                cursor = new ViewStreamCursor();
                _viewCursors[viewId] = cursor;
            }

            var disp = MushProtocol.EvaluateViewCursor(cursor, d);
            switch (disp)
            {
                case ViewCursorDisposition.ApplySnapshot:
                {
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

        private void EnqueueInboundBounded(long epoch, MushEnvelope env)
        {
            while (_inbound.Count >= MaxInboundQueueSize && _inbound.TryDequeue(out _)) { }
            _inbound.Enqueue((epoch, env));
        }

        private static void DrainQueue<T>(ConcurrentQueue<T> q)
        {
            while (q.TryDequeue(out _)) { }
        }

        private async Task ReceiveLoopAsync(SocketRuntime rt, long epoch)
        {
            var ws = rt.Socket;
            var ct = rt.Cts.Token;
            var buffer = new byte[16384];
            try
            {
                while (!ct.IsCancellationRequested && ws.State == WebSocketState.Open)
                {
                    using (var ms = new MemoryStream())
                    {
                        WebSocketReceiveResult res;
                        bool oversized = false;
                        do
                        {
                            res = await ws.ReceiveAsync(new ArraySegment<byte>(buffer), ct);
                            if (res.MessageType == WebSocketMessageType.Close)
                            {
                                _isConnected = false;
                                int? closeCode = res.CloseStatus.HasValue ? (int)res.CloseStatus.Value : (int?)null;
                                string desc = !string.IsNullOrEmpty(res.CloseStatusDescription)
                                    ? MushAuthClient.RedactSensitiveTokens(res.CloseStatusDescription)
                                    : $"WebSocket closed ({(closeCode.HasValue ? closeCode.Value.ToString() : "EOF")})";
                                _socketDrops.Enqueue((epoch, closeCode, desc));
                                return;
                            }
                            if (ms.Length + res.Count > MaxInboundFrameBytes)
                            {
                                oversized = true;
                                break;
                            }
                            ms.Write(buffer, 0, res.Count);
                        } while (!res.EndOfMessage);

                        if (oversized)
                        {
                            _isConnected = false;
                            try { ws.Abort(); } catch { /* ignore */ }
                            _socketDrops.Enqueue((epoch, null, $"Inbound frame exceeded {MaxInboundFrameBytes / (1024 * 1024)} MiB limit"));
                            return;
                        }

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
                                    if (env.EventType == "ping" && env.Data != null && env.Data.IsObject)
                                    {
                                        ulong nonce = env.Data.ULong("nonce", 0);
                                        EnqueuePriorityRaw(MushProtocol.BuildPongCommand(nonce));
                                        env.PongAlreadySent = true;
                                    }
                                    EnqueueInboundBounded(epoch, env);
                                }
                            }
                            catch (Exception)
                            {
                                // Isolate malformed frame without tearing down the socket or trapping ?resume=<_lastSeq>.
                                int errs = Interlocked.Increment(ref _consecutiveParseErrors);
                                if (errs >= 3)
                                    _lastSeq = 0;
                            }
                        }
                    }
                }
            }
            catch (OperationCanceledException) { }
            catch (Exception ex)
            {
                _isConnected = false;
                string safeMsg = MushAuthClient.RedactSensitiveTokens(ex.Message);
                _socketDrops.Enqueue((epoch, null, $"WebSocket interrupted ({safeMsg})"));
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
                while (!ct.IsCancellationRequested && ws.State == WebSocketState.Open)
                {
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
                _isConnected = false;
                try { ws.Abort(); } catch { /* ignore */ }
                string safeMsg = MushAuthClient.RedactSensitiveTokens(ex.Message);
                _socketDrops.Enqueue((epoch, null, $"WebSocket send failed ({safeMsg})"));
            }
            finally
            {
                rt.ReleaseLoop();
            }
        }
    }
}
