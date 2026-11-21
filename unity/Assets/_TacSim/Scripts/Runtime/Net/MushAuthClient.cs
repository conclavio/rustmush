using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using TacSim.Core;
using UnityEngine;
using UnityEngine.Networking;

namespace TacSim.Net
{
    public sealed class MushCharacterSlot
    {
        public string Key;
        public string Name;
    }

    public sealed class MushAuthSession
    {
        public string BaseHttpUrl;
        public string Token;
        public string Account;
        public List<MushCharacterSlot> Characters = new List<MushCharacterSlot>();
    }

    /// <summary>
    /// Bounded streaming DownloadHandlerScript enforcing a strict byte ceiling on HTTP responses
    /// (both success and error bodies, with or without Content-Length). Returning false from
    /// ReceiveData immediately aborts UnityWebRequest before unbounded memory is accumulated (EXT-P2-6).
    /// </summary>
    public sealed class BoundedDownloadHandler : DownloadHandlerScript
    {
        public const int DefaultChunkBufferBytes = 8 * 1024;

        private readonly int _maxBytes;
        private readonly Action _onOverflow;
        private readonly MemoryStream _buffer;
        private long _receivedBytes;
        private bool _overflowed;
        private bool _disposed;

        public BoundedDownloadHandler(int maxBytes, Action onOverflow = null)
            : base(new byte[Math.Min(Math.Max(maxBytes, 1), DefaultChunkBufferBytes)])
        {
            if (maxBytes <= 0)
                throw new ArgumentOutOfRangeException(nameof(maxBytes), "maxBytes must be positive.");
            _maxBytes = maxBytes;
            _onOverflow = onOverflow;
            _buffer = new MemoryStream(Math.Min(maxBytes, 4096));
        }

        public int MaxBytes => _maxBytes;
        public long ReceivedBytes => _receivedBytes;
        public bool Overflowed => _overflowed;

        public void FeedContentLengthHeaderForTest(ulong contentLength) =>
            ReceiveContentLengthHeader(contentLength);

        public bool FeedChunkForTest(byte[] chunk, int length) =>
            ReceiveData(chunk, length);

        protected override void ReceiveContentLengthHeader(ulong contentLength)
        {
            if (contentLength > (ulong)_maxBytes)
            {
                MarkOverflow();
            }
        }

        protected override bool ReceiveData(byte[] data, int dataLength)
        {
            if (_overflowed || _disposed)
                return false;
            if (dataLength < 0)
            {
                MarkOverflow();
                return false;
            }
            if (data == null || dataLength == 0)
                return true;
            if (dataLength > data.Length)
            {
                MarkOverflow();
                return false;
            }
            if (_receivedBytes + (long)dataLength > _maxBytes)
            {
                MarkOverflow();
                return false;
            }
            _buffer.Write(data, 0, dataLength);
            _receivedBytes += dataLength;
            return true;
        }

        private void MarkOverflow()
        {
            if (_overflowed) return;
            _overflowed = true;
            try
            {
                _buffer.SetLength(0);
                _buffer.Capacity = 0;
            }
            catch
            {
                // Ignore if already disposed
            }
            try
            {
                _onOverflow?.Invoke();
            }
            catch
            {
                // Best-effort abort callback
            }
        }

        protected override byte[] GetData()
        {
            if (_overflowed || _disposed)
                return Array.Empty<byte>();
            return _buffer.ToArray();
        }

        protected override string GetText()
        {
            if (_overflowed || _disposed || _buffer.Length == 0)
                return string.Empty;
            return Encoding.UTF8.GetString(_buffer.GetBuffer(), 0, (int)_buffer.Length);
        }

        public override void Dispose()
        {
            if (!_disposed)
            {
                _disposed = true;
                _buffer.Dispose();
            }
            base.Dispose();
        }
    }

    /// <summary>
    /// HTTP client for mush-net-web authentication (/api/auth/login and /api/auth/register).
    /// </summary>
    public static class MushAuthClient
    {
        /// <summary>Maximum allowed authentication HTTP response body size (64 KiB, EXT-P2-6).</summary>
        public const int MaxAuthResponseBytes = 64 * 1024;

        /// <summary>Automatic redirect limit on credential-bearing auth POSTs (disabled, EXT-P1-2).</summary>
        public const int AuthRedirectLimit = 0;

        private static readonly Regex SubprotocolTokenRegex =
            new Regex(@"rustmush\.v1\.[^\s'""<>\]\)},;]+", RegexOptions.Compiled | RegexOptions.IgnoreCase);
        private static readonly Regex BearerTokenRegex =
            new Regex(@"(Bearer\s+)[^\s'""<>\]\)},;]+", RegexOptions.Compiled | RegexOptions.IgnoreCase);
        private static readonly Regex JsonTokenRegex =
            new Regex(@"(""token""\s*:\s*"")[^""]+("")", RegexOptions.Compiled | RegexOptions.IgnoreCase);
        private static readonly Regex UrlUserInfoRegex =
            new Regex(@"(://)[^/\s@?#]+@", RegexOptions.Compiled);

        public static string RedactSensitiveTokens(string input)
        {
            if (string.IsNullOrEmpty(input)) return string.Empty;
            string s = SubprotocolTokenRegex.Replace(input, "rustmush.v1.[REDACTED]");
            s = BearerTokenRegex.Replace(s, "$1[REDACTED]");
            s = JsonTokenRegex.Replace(s, "$1[REDACTED]$2");
            s = UrlUserInfoRegex.Replace(s, "$1[REDACTED]@");
            return s;
        }

        /// <summary>
        /// Strips any embedded <c>user:pass@</c> userinfo, query string, or fragment from a server URL
        /// before logging, displaying in HUD notices, or persisting to <c>PlayerPrefs</c>.
        /// </summary>
        public static string SanitizeUrlForDisplayOrStorage(string rawUrl)
        {
            if (string.IsNullOrWhiteSpace(rawUrl))
                return string.Empty;

            string trimmed = rawUrl.Trim();
            if (Uri.TryCreate(trimmed, UriKind.Absolute, out Uri absUri))
            {
                string schemeAndServer = absUri.GetComponents(UriComponents.SchemeAndServer, UriFormat.UriEscaped);
                string path = absUri.AbsolutePath;
                if (string.IsNullOrEmpty(path) || path == "/")
                    return schemeAndServer;
                return schemeAndServer + path.TrimEnd('/');
            }

            // Handle schemeless URLs (e.g. `user:pass@host:8080/path?q=1#f`).
            int qOrFrag = trimmed.IndexOfAny(new[] { '?', '#' });
            if (qOrFrag >= 0)
                trimmed = trimmed.Substring(0, qOrFrag);
            int schemeIdx = trimmed.IndexOf("://", StringComparison.Ordinal);
            string prefix = schemeIdx >= 0 ? trimmed.Substring(0, schemeIdx + 3) : string.Empty;
            string rest = schemeIdx >= 0 ? trimmed.Substring(schemeIdx + 3) : trimmed;
            int slashIdx = rest.IndexOf('/');
            string authorityPart = slashIdx >= 0 ? rest.Substring(0, slashIdx) : rest;
            string pathPart = slashIdx >= 0 ? rest.Substring(slashIdx) : string.Empty;
            int atIdx = authorityPart.LastIndexOf('@');
            if (atIdx >= 0)
                authorityPart = authorityPart.Substring(atIdx + 1);
            return (prefix + authorityPart + pathPart).TrimEnd('/');
        }

        public static string NormalizeBaseUrl(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw))
                return "http://127.0.0.1:8080";
            string s = raw.Trim().TrimEnd('/');
            if (s.StartsWith("wss://", StringComparison.OrdinalIgnoreCase))
            {
                s = "https://" + s.Substring("wss://".Length);
            }
            else if (s.StartsWith("ws://", StringComparison.OrdinalIgnoreCase))
            {
                s = "http://" + s.Substring("ws://".Length);
            }
            else if (!s.StartsWith("http://", StringComparison.OrdinalIgnoreCase) &&
                     !s.StartsWith("https://", StringComparison.OrdinalIgnoreCase) &&
                     s.IndexOf("://", StringComparison.Ordinal) < 0)
            {
                s = (IsLoopbackAuthority(s) ? "http://" : "https://") + s;
            }
            return s;
        }

        public static bool IsLoopbackUrl(string normalizedUrl)
        {
            if (string.IsNullOrWhiteSpace(normalizedUrl))
                return false;
            string trimmed = normalizedUrl.Trim();
            if (trimmed.IndexOfAny(new[] { '@', '?', '#', '\\' }) >= 0)
                return false;
            if (Uri.TryCreate(trimmed, UriKind.Absolute, out Uri u))
                return IsLoopbackUri(u);
            return IsLoopbackAuthority(trimmed);
        }

        public static bool IsLoopbackAuthority(string rawAuthority)
        {
            if (string.IsNullOrWhiteSpace(rawAuthority))
                return false;
            string hostPart = rawAuthority.Trim();
            if (MushProtocol.ContainsControlChars(hostPart) ||
                hostPart.IndexOfAny(new[] { '@', '?', '#', '\\', ' ', '\t' }) >= 0)
            {
                return false;
            }
            int slash = hostPart.IndexOf('/');
            if (slash >= 0) hostPart = hostPart.Substring(0, slash);
            if (string.IsNullOrEmpty(hostPart))
                return false;

            if (hostPart.StartsWith("[", StringComparison.Ordinal))
            {
                int close = hostPart.IndexOf(']');
                if (close <= 1)
                    return false;
                if (close + 1 < hostPart.Length)
                {
                    if (hostPart[close + 1] != ':' || close + 2 >= hostPart.Length)
                        return false;
                    for (int i = close + 2; i < hostPart.Length; i++)
                    {
                        if (hostPart[i] < '0' || hostPart[i] > '9')
                            return false;
                    }
                }
                string v6 = hostPart.Substring(1, close - 1);
                return IPAddress.TryParse(v6, out IPAddress v6Addr) && IPAddress.IsLoopback(v6Addr);
            }

            string host = hostPart;
            int firstColon = hostPart.IndexOf(':');
            int lastColon = hostPart.LastIndexOf(':');
            if (firstColon >= 0 && firstColon == lastColon)
            {
                string portPart = hostPart.Substring(firstColon + 1);
                if (portPart.Length == 0)
                    return false;
                for (int i = 0; i < portPart.Length; i++)
                {
                    if (portPart[i] < '0' || portPart[i] > '9')
                        return false;
                }
                host = hostPart.Substring(0, firstColon);
            }
            host = host.Trim();
            if (string.IsNullOrEmpty(host))
                return false;
            if (string.Equals(host, "localhost", StringComparison.OrdinalIgnoreCase))
                return true;
            return IPAddress.TryParse(host, out IPAddress addr) && IPAddress.IsLoopback(addr);
        }

        public static bool IsLoopbackUri(Uri uri)
        {
            if (uri == null || string.IsNullOrWhiteSpace(uri.Host) || !string.IsNullOrEmpty(uri.UserInfo))
                return false;
            return IsLoopbackAuthority(uri.Host);
        }

        public static string ExtractErrorMessage(string responseBody, string fallbackError)
        {
            string serverMsg = responseBody;
            if (!string.IsNullOrWhiteSpace(responseBody))
            {
                try
                {
                    var errNode = JsonNode.Parse(responseBody);
                    if (errNode != null && errNode.IsObject)
                        serverMsg = errNode.Str("error", responseBody);
                }
                catch (FormatException)
                {
                    serverMsg = responseBody;
                }
            }
            return string.IsNullOrWhiteSpace(serverMsg) ? (fallbackError ?? "Unknown error") : serverMsg;
        }

        public static string ToWebSocketBaseUrl(string httpBaseUrl)
        {
            string norm = NormalizeBaseUrl(httpBaseUrl);
            if (norm.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
                return "wss://" + norm.Substring("https://".Length);
            if (norm.StartsWith("http://", StringComparison.OrdinalIgnoreCase))
                return "ws://" + norm.Substring("http://".Length);
            return "ws://" + norm;
        }

        public static string ExtractErrorMessageFromResponse(long responseCode, string responseText, string transportError)
        {
            string serverMsg = null;
            if (!string.IsNullOrWhiteSpace(responseText))
            {
                try
                {
                    var errNode = JsonNode.Parse(responseText);
                    if (errNode != null && errNode.IsObject)
                        serverMsg = errNode.Str("error", errNode.Str("message", null));
                }
                catch
                {
                    // Non-JSON proxy/gateway response (e.g., 502/503 HTML or 429 plain text).
                }

                if (string.IsNullOrWhiteSpace(serverMsg))
                {
                    string trimmed = responseText.Trim();
                    bool looksLikeHtml = trimmed.StartsWith("<", StringComparison.Ordinal);
                    if (!looksLikeHtml)
                        serverMsg = trimmed.Length > 200 ? trimmed.Substring(0, 200) + "..." : trimmed;
                }
            }

            if (string.IsNullOrWhiteSpace(serverMsg))
                serverMsg = !string.IsNullOrWhiteSpace(transportError) ? transportError : $"HTTP {responseCode}";

            return RedactSensitiveTokens(serverMsg);
        }

        internal static bool DefaultAllowInsecureNonLoopback()
        {
#if UNITY_EDITOR || DEVELOPMENT_BUILD
            return true;
#else
            return false;
#endif
        }

        public static Uri BuildValidatedAuthEndpointUri(
            string baseUrl,
            string endpoint,
            bool allowInsecureNonLoopback = false)
        {
            string normalizedBase = NormalizeBaseUrl(baseUrl);
            if (MushProtocol.ContainsControlChars(normalizedBase) ||
                normalizedBase.IndexOf('?') >= 0 ||
                normalizedBase.IndexOf('#') >= 0)
            {
                throw new InvalidOperationException("Auth server base URL must not contain query strings, fragments, or control characters.");
            }
            if (!Uri.TryCreate(normalizedBase, UriKind.Absolute, out Uri baseUri))
                throw new InvalidOperationException("Invalid auth server URL.");
            if (!string.IsNullOrEmpty(baseUri.UserInfo) ||
                !string.IsNullOrEmpty(baseUri.Query) ||
                !string.IsNullOrEmpty(baseUri.Fragment))
            {
                throw new InvalidOperationException("Auth server base URL must not contain credentials (userinfo), query strings, or fragments.");
            }

            bool isHttps = string.Equals(baseUri.Scheme, "https", StringComparison.OrdinalIgnoreCase);
            bool isHttp = string.Equals(baseUri.Scheme, "http", StringComparison.OrdinalIgnoreCase);
            if (!isHttps && !isHttp)
            {
                throw new InvalidOperationException(
                    $"Unsupported auth server URL scheme '{baseUri.Scheme}'; use https://.");
            }

            if (!isHttps && !IsLoopbackUri(baseUri))
            {
                if (!allowInsecureNonLoopback)
                {
                    throw new InvalidOperationException(
                        $"Refusing cleartext HTTP authentication ('{baseUri.Scheme}://{baseUri.Authority}') to non-loopback host in non-dev build; use https://.");
                }
                string safeLogUrl = SanitizeUrlForDisplayOrStorage(normalizedBase);
                try { Debug.LogWarning($"[MushAuthClient] Authenticating over unencrypted remote HTTP ({safeLogUrl}); use https:// in production."); }
                catch { /* headless test runner */ }
            }

            string cleanEndpoint = string.IsNullOrWhiteSpace(endpoint)
                ? string.Empty
                : (endpoint.StartsWith("/", StringComparison.Ordinal) ? endpoint : "/" + endpoint);
            string basePath = baseUri.AbsolutePath.TrimEnd('/');
            var builder = new UriBuilder(baseUri)
            {
                UserName = string.Empty,
                Password = string.Empty,
                Path = basePath + cleanEndpoint,
                Query = string.Empty,
                Fragment = string.Empty
            };
            Uri fullUri = builder.Uri;

            if (!string.Equals(fullUri.Scheme, baseUri.Scheme, StringComparison.OrdinalIgnoreCase) ||
                !string.Equals(fullUri.Authority, baseUri.Authority, StringComparison.OrdinalIgnoreCase) ||
                !string.IsNullOrEmpty(fullUri.UserInfo))
            {
                throw new InvalidOperationException("Auth endpoint URI changed target origin.");
            }

            return fullUri;
        }

        public static UnityWebRequest CreateAuthPostRequest(
            Uri endpointUri,
            byte[] payloadBytes,
            out BoundedDownloadHandler downloadHandler)
        {
            if (endpointUri == null)
                throw new ArgumentNullException(nameof(endpointUri));

            var req = new UnityWebRequest(endpointUri.AbsoluteUri, UnityWebRequest.kHttpVerbPOST);
            downloadHandler = new BoundedDownloadHandler(MaxAuthResponseBytes, () =>
            {
                try { req.Abort(); } catch { /* ignore */ }
            });
            req.uploadHandler = new UploadHandlerRaw(payloadBytes ?? Array.Empty<byte>());
            req.downloadHandler = downloadHandler;
            req.SetRequestHeader("Content-Type", "application/json");
            req.timeout = 10;
            // Never automatically follow redirects on credential-bearing auth POSTs (EXT-P1-2):
            // prevents redirect-based downgrade to cleartext HTTP or credential leakage to another origin.
            req.redirectLimit = AuthRedirectLimit;
            return req;
        }

        public static MushAuthSession ProcessAuthResponse(
            string baseHttpUrl,
            Uri requestUri,
            UnityWebRequest req,
            BoundedDownloadHandler downloadHandler,
            bool allowInsecureNonLoopback = false)
        {
            if (req == null)
                throw new ArgumentNullException(nameof(req));

            if (downloadHandler != null && downloadHandler.Overflowed)
            {
                throw new InvalidOperationException(
                    $"Auth response exceeded maximum allowed size ({downloadHandler.MaxBytes} bytes).");
            }

            if (req.responseCode >= 300 && req.responseCode < 400)
            {
                throw new InvalidOperationException(
                    $"Auth failed ({req.responseCode}): Redirects are disabled for authentication requests.");
            }

            // Defense-in-depth: if the transport somehow mutated req.url across scheme or authority,
            // refuse before processing credentials/session tokens.
            if (!string.IsNullOrWhiteSpace(req.url) && Uri.TryCreate(req.url, UriKind.Absolute, out Uri finalUri))
            {
                bool finalHttps = string.Equals(finalUri.Scheme, "https", StringComparison.OrdinalIgnoreCase);
                if (!finalHttps && !allowInsecureNonLoopback && !IsLoopbackUri(finalUri))
                {
                    throw new InvalidOperationException(
                        $"Refusing cleartext HTTP authentication response ('{finalUri.Scheme}://{finalUri.Authority}'); use https://.");
                }
                if (requestUri != null &&
                    (!string.Equals(finalUri.Scheme, requestUri.Scheme, StringComparison.OrdinalIgnoreCase) ||
                     !string.Equals(finalUri.Authority, requestUri.Authority, StringComparison.OrdinalIgnoreCase)))
                {
                    throw new InvalidOperationException(
                        "Auth request redirected across origin or scheme; refusing response.");
                }
            }

            string text = req.downloadHandler != null ? req.downloadHandler.text : string.Empty;
            if (req.result != UnityWebRequest.Result.Success)
            {
                string safeMsg = ExtractErrorMessageFromResponse(req.responseCode, text, req.error);
                throw new InvalidOperationException($"Auth failed ({req.responseCode}): {safeMsg}");
            }

            JsonNode json;
            try
            {
                json = JsonNode.Parse(text);
            }
            catch (Exception)
            {
                throw new InvalidOperationException($"Auth server returned invalid JSON ({req.responseCode}).");
            }

            if (json == null || !json.IsObject || string.IsNullOrEmpty(json.Str("token")))
                throw new InvalidOperationException("Auth response did not include a session token.");

            var session = new MushAuthSession
            {
                BaseHttpUrl = baseHttpUrl,
                Token = json.Str("token", string.Empty),
                Account = json.Str("account", string.Empty)
            };

            var chars = json["characters"].AsArray();
            if (chars != null)
            {
                for (int i = 0; i < chars.Count; i++)
                {
                    var c = chars[i];
                    if (!c.IsObject) continue;
                    session.Characters.Add(new MushCharacterSlot
                    {
                        Key = c.Str("key", c.Str("id", string.Empty)),
                        Name = c.Str("name", "Pilot")
                    });
                }
            }
            return session;
        }

        public static Task<MushAuthSession> LoginAsync(string baseUrl, string account, string password)
        {
            var body = JsonNode.NewObject();
            body["account"] = JsonNode.FromString(account ?? string.Empty);
            body["password"] = JsonNode.FromString(password ?? string.Empty);
            return PostAuthAsync(NormalizeBaseUrl(baseUrl), "/api/auth/login", body, DefaultAllowInsecureNonLoopback());
        }

        public static Task<MushAuthSession> RegisterAsync(string baseUrl, string account, string password, string characterName)
        {
            var body = JsonNode.NewObject();
            body["account"] = JsonNode.FromString(account ?? string.Empty);
            body["password"] = JsonNode.FromString(password ?? string.Empty);
            body["char_name"] = JsonNode.FromString(characterName ?? account ?? "Pilot");
            return PostAuthAsync(NormalizeBaseUrl(baseUrl), "/api/auth/register", body, DefaultAllowInsecureNonLoopback());
        }

        private static async Task<MushAuthSession> PostAuthAsync(
            string baseHttpUrl,
            string endpoint,
            JsonNode payload,
            bool allowInsecureNonLoopback)
        {
            Uri endpointUri = BuildValidatedAuthEndpointUri(baseHttpUrl, endpoint, allowInsecureNonLoopback);
            byte[] bytes = Encoding.UTF8.GetBytes(payload.ToJson());

            using (var req = CreateAuthPostRequest(endpointUri, bytes, out BoundedDownloadHandler downloadHandler))
            {
                var tcs = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
                var op = req.SendWebRequest();
                if (op.isDone)
                    tcs.TrySetResult(true);
                else
                    op.completed += _ => tcs.TrySetResult(true);
                await tcs.Task;

                return ProcessAuthResponse(baseHttpUrl, endpointUri, req, downloadHandler, allowInsecureNonLoopback);
            }
        }
    }
}
