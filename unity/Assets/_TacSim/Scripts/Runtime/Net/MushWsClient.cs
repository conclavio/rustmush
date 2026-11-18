using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Net.WebSockets;
using System.Text;
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
        private static readonly float[] BackoffSeconds = { 0.5f, 1.0f, 2.0f, 5.0f, 15.0f };

        private ClientWebSocket _ws;
        private CancellationTokenSource _cts;
        private readonly ConcurrentQueue<(long epoch, MushEnvelope env)> _inbound = new ConcurrentQueue<(long, MushEnvelope)>();
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
        private bool _intentionalDisconnect = true;
        private bool _reconnectInFlight;
        private int _reconnectAttempt;
        private double _nextReconnectAtMonotonicSec = -1.0;

        private static double MonotonicSeconds() =>
            Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency;

        public bool IsConnected => _isConnected && _ws != null && _ws.State == WebSocketState.Open;
        public MushWsConnectionState State { get; private set; } = MushWsConnectionState.Disconnected;
        public ulong LastSeq => _lastSeq;
        public int OutboundQueueCount => _outbound.Count;
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
                _panelFeeds.Clear();
                _viewCursors.Clear();
                _advertisedPanels.Clear();
            }

            await OpenSocketInternalAsync(isReconnect: !resetResume && _lastSeq > 0);
        }

        private async Task OpenSocketInternalAsync(bool isReconnect)
        {
            if (_activeAuth == null || string.IsNullOrEmpty(_activeAuth.Token))
                throw new InvalidOperationException("No active session credentials.");

            CloseSocketOnly();
            while (_outbound.TryDequeue(out _)) { }

            long epoch = Interlocked.Increment(ref _sessionEpoch);
            while (_inbound.TryDequeue(out _)) { }
            while (_socketDrops.TryDequeue(out _)) { }

            _cts = new CancellationTokenSource();
            _ws = new ClientWebSocket();
            _ws.Options.AddSubProtocol($"rustmush.v1.{_activeAuth.Token}");
            _ws.Options.KeepAliveInterval = TimeSpan.FromSeconds(20);

            string wsBase = MushAuthClient.ToWebSocketBaseUrl(_activeAuth.BaseHttpUrl);
            string query = BuildWsQuery(_activeCharKey, _lastSeq);
            var uri = new Uri($"{wsBase}/ws?{query}");

            State = isReconnect ? MushWsConnectionState.Reconnecting : MushWsConnectionState.Connecting;
            OnConnectionStatusChanged?.Invoke(isReconnect
                ? $"Reconnecting to {uri.Host} (resume seq {_lastSeq})..."
                : $"Connecting to {uri.Host}...");

            await _ws.ConnectAsync(uri, _cts.Token);
            _isConnected = true;
            _reconnectAttempt = 0;
            _nextReconnectAtMonotonicSec = -1.0;
            State = MushWsConnectionState.Online;

            // Declare structured true_color Web client capabilities immediately upon attach
            EnqueueRaw(MushProtocol.BuildCapsCommand(120, 40));
            OnConnectionStatusChanged?.Invoke($"Connected to {uri.Host}");

            var capturedWs = _ws;
            var capturedToken = _cts.Token;
            _ = Task.Run(() => ReceiveLoopAsync(capturedWs, capturedToken, epoch));
            _ = Task.Run(() => SendLoopAsync(capturedWs, capturedToken));
        }

        public void Disconnect()
        {
            Interlocked.Increment(ref _sessionEpoch);
            _intentionalDisconnect = true;
            _nextReconnectAtMonotonicSec = -1.0;
            _reconnectInFlight = false;
            State = MushWsConnectionState.Disconnected;
            while (_inbound.TryDequeue(out _)) { }
            while (_socketDrops.TryDequeue(out _)) { }
            while (_outbound.TryDequeue(out _)) { }
            CloseSocketOnly();
        }

        private void CloseSocketOnly()
        {
            _isConnected = false;
            if (_cts != null)
            {
                try { _cts.Cancel(); } catch { /* ignore */ }
                _cts.Dispose();
                _cts = null;
            }
            if (_ws != null)
            {
                try { _ws.Dispose(); } catch { /* ignore */ }
                _ws = null;
            }
        }

        public void Dispose()
        {
            Disconnect();
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
                _inbound.Enqueue((Interlocked.Read(ref _sessionEpoch), env));
            }
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
                while (_inbound.TryDequeue(out _)) { }
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
                DispatchEvent(env.EventType, env.Data);
            }
        }

        private void HandleSocketDroppedOnMainThread(int? closeCode, string detail)
        {
            if (_intentionalDisconnect) return;

            var disp = ClassifyCloseCode(closeCode);
            if (disp == WsCloseDisposition.FatalSessionEnded)
            {
                _intentionalDisconnect = true;
                _activeAuth = null;
                _lastSeq = 0;
                State = MushWsConnectionState.SessionEnded;
                string reason = string.IsNullOrWhiteSpace(detail) ? "Session ended or credentials expired." : detail;
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
                delaySec = BackoffSeconds[idx];
                _reconnectAttempt++;
            }

            State = MushWsConnectionState.Reconnecting;
            _nextReconnectAtMonotonicSec = MonotonicSeconds() + delaySec;
            OnConnectionStatusChanged?.Invoke($"{detail} — reconnecting in {delaySec:0.#}s...");
        }

        private async Task AttemptAutoReconnectAsync()
        {
            if (_intentionalDisconnect || _activeAuth == null || _reconnectInFlight)
                return;

            _reconnectInFlight = true;
            _nextReconnectAtMonotonicSec = -1.0;
            try
            {
                await OpenSocketInternalAsync(isReconnect: true);
            }
            catch (Exception ex)
            {
                // Detect HTTP 401 / 4401 refusal surfaced during WebSocket upgrade
                string msg = ex.Message ?? string.Empty;
                if (msg.Contains("401") || msg.Contains("4401"))
                {
                    HandleSocketDroppedOnMainThread(CloseCodeBadSession, "Session token expired; please sign in again.");
                }
                else
                {
                    HandleSocketDroppedOnMainThread(null, $"Reconnect failed ({ex.Message})");
                }
            }
            finally
            {
                _reconnectInFlight = false;
            }
        }

        private void SubscribeStandardPanels()
        {
            foreach (string feed in _advertisedPanels)
            {
                if (feed == "space.cockpit" || feed == "space.holotank" || feed == "space.docking" || feed == "sky.chart" ||
                    feed == "tv.status" || feed == "arena.cockpit" || feed == "arena.observer" || feed == "arena.replay")
                {
                    SendPanelSubscribe(feed);
                }
            }
        }

        private void DispatchEvent(string type, JsonNode d)
        {
            switch (type)
            {
                case "hello":
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
                    if (!d.Bool("replayed", true))
                    {
                        _lastSeq = 0;
                        _panelFeeds.Clear();
                        _viewCursors.Clear();
                        SubscribeStandardPanels();
                        SendLine("look");
                    }
                    break;

                case "session_info":
                    ActiveCharacter = MushProtocol.ParseObjRef(d["character"]);
                    break;

                case "ping":
                    ulong nonce = (ulong)Math.Max(0L, d.Long("nonce", 0));
                    EnqueueRaw(MushProtocol.BuildPongCommand(nonce));
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

            if (kind == "delta" && _panelFeeds.TryGetValue(feed, out JsonNode existing) && existing.IsObject && payload.IsObject)
            {
                existing.MergeObjectDelta(payload);
                OnPanelUpdated?.Invoke(feed, existing);
            }
            else
            {
                var copy = payload != null ? payload.Clone() : JsonNode.NewObject();
                _panelFeeds[feed] = copy;
                OnPanelUpdated?.Invoke(feed, copy);
            }
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
                    SendPanelSubscribe(viewId);
                    break;
                }
                case ViewCursorDisposition.IgnoreDuplicateOrStale:
                    break;
            }
        }

        private void EnqueueRaw(string json)
        {
            if (string.IsNullOrEmpty(json)) return;
            _outbound.Enqueue(json);
        }

        private async Task ReceiveLoopAsync(ClientWebSocket ws, CancellationToken ct, long epoch)
        {
            var buffer = new byte[16384];
            try
            {
                while (!ct.IsCancellationRequested && ws.State == WebSocketState.Open)
                {
                    using (var ms = new MemoryStream())
                    {
                        WebSocketReceiveResult res;
                        do
                        {
                            res = await ws.ReceiveAsync(new ArraySegment<byte>(buffer), ct);
                            if (res.MessageType == WebSocketMessageType.Close)
                            {
                                _isConnected = false;
                                int? closeCode = res.CloseStatus.HasValue ? (int)res.CloseStatus.Value : (int?)null;
                                string desc = !string.IsNullOrEmpty(res.CloseStatusDescription)
                                    ? res.CloseStatusDescription
                                    : $"WebSocket closed ({(closeCode.HasValue ? closeCode.Value.ToString() : "EOF")})";
                                _socketDrops.Enqueue((epoch, closeCode, desc));
                                return;
                            }
                            ms.Write(buffer, 0, res.Count);
                        } while (!res.EndOfMessage);

                        if (res.MessageType == WebSocketMessageType.Text)
                        {
                            string json = Encoding.UTF8.GetString(ms.GetBuffer(), 0, (int)ms.Length);
                            var env = MushProtocol.ParseEnvelope(json);
                            if (env != null) _inbound.Enqueue((epoch, env));
                        }
                    }
                }
            }
            catch (OperationCanceledException) { }
            catch (Exception ex)
            {
                _isConnected = false;
                _socketDrops.Enqueue((epoch, null, $"WebSocket interrupted ({ex.Message})"));
            }
        }

        private async Task SendLoopAsync(ClientWebSocket ws, CancellationToken ct)
        {
            try
            {
                while (!ct.IsCancellationRequested && ws.State == WebSocketState.Open)
                {
                    if (_outbound.TryDequeue(out string msg))
                    {
                        byte[] bytes = Encoding.UTF8.GetBytes(msg);
                        await ws.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, ct);
                    }
                    else
                    {
                        await Task.Delay(12, ct);
                    }
                }
            }
            catch (OperationCanceledException) { }
            catch (Exception)
            {
                // ReceiveLoopAsync handles socket drop notification on the session epoch queue.
            }
        }
    }
}
