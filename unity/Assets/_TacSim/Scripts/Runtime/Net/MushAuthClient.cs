using System;
using System.Collections.Generic;
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
    /// HTTP client for mush-net-web authentication (/api/auth/login and /api/auth/register).
    /// </summary>
    public static class MushAuthClient
    {
        private static readonly Regex SubprotocolTokenRegex =
            new Regex(@"rustmush\.v1\.[^\s'""<>\]\)},;]+", RegexOptions.Compiled | RegexOptions.IgnoreCase);
        private static readonly Regex BearerTokenRegex =
            new Regex(@"(Bearer\s+)[^\s'""<>\]\)},;]+", RegexOptions.Compiled | RegexOptions.IgnoreCase);
        private static readonly Regex JsonTokenRegex =
            new Regex(@"(""token""\s*:\s*"")[^""]+("")", RegexOptions.Compiled | RegexOptions.IgnoreCase);

        public static string RedactSensitiveTokens(string input)
        {
            if (string.IsNullOrEmpty(input)) return string.Empty;
            string s = SubprotocolTokenRegex.Replace(input, "rustmush.v1.[REDACTED]");
            s = BearerTokenRegex.Replace(s, "$1[REDACTED]");
            s = JsonTokenRegex.Replace(s, "$1[REDACTED]$2");
            return s;
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
                     !s.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
            {
                s = (IsLoopbackAuthority(s) ? "http://" : "https://") + s;
            }
            return s;
        }

        public static bool IsLoopbackUrl(string normalizedUrl)
        {
            if (string.IsNullOrWhiteSpace(normalizedUrl)) return true;
            if (Uri.TryCreate(normalizedUrl, UriKind.Absolute, out Uri u))
                return IsLoopbackAuthority(u.Authority);
            return IsLoopbackAuthority(normalizedUrl);
        }

        public static bool IsLoopbackAuthority(string rawAuthority)
        {
            if (string.IsNullOrWhiteSpace(rawAuthority))
                return true;
            string hostPart = rawAuthority.Trim();
            int slash = hostPart.IndexOf('/');
            if (slash >= 0) hostPart = hostPart.Substring(0, slash);
            if (hostPart.StartsWith("[", StringComparison.Ordinal))
            {
                int close = hostPart.IndexOf(']');
                string v6 = close > 1 ? hostPart.Substring(1, close - 1) : hostPart;
                return IPAddress.TryParse(v6, out IPAddress v6Addr) && IPAddress.IsLoopback(v6Addr);
            }
            string host = hostPart;
            int firstColon = hostPart.IndexOf(':');
            int lastColon = hostPart.LastIndexOf(':');
            if (firstColon >= 0 && firstColon == lastColon)
                host = hostPart.Substring(0, firstColon);
            host = host.Trim();
            if (string.Equals(host, "localhost", StringComparison.OrdinalIgnoreCase))
                return true;
            return IPAddress.TryParse(host, out IPAddress addr) && IPAddress.IsLoopback(addr);
        }

        public static bool IsLoopbackUri(Uri uri)
        {
            if (uri == null || string.IsNullOrWhiteSpace(uri.Host))
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

        public static Task<MushAuthSession> LoginAsync(string baseUrl, string account, string password)
        {
            var body = JsonNode.NewObject();
            body["account"] = JsonNode.FromString(account ?? string.Empty);
            body["password"] = JsonNode.FromString(password ?? string.Empty);
            return PostAuthAsync(NormalizeBaseUrl(baseUrl), "/api/auth/login", body);
        }

        public static Task<MushAuthSession> RegisterAsync(string baseUrl, string account, string password, string characterName)
        {
            var body = JsonNode.NewObject();
            body["account"] = JsonNode.FromString(account ?? string.Empty);
            body["password"] = JsonNode.FromString(password ?? string.Empty);
            body["char_name"] = JsonNode.FromString(characterName ?? account ?? "Pilot");
            return PostAuthAsync(NormalizeBaseUrl(baseUrl), "/api/auth/register", body);
        }

        private static async Task<MushAuthSession> PostAuthAsync(string baseHttpUrl, string endpoint, JsonNode payload)
        {
            if (baseHttpUrl.StartsWith("http://", StringComparison.OrdinalIgnoreCase) && !IsLoopbackUrl(baseHttpUrl))
            {
                try { Debug.LogWarning($"[MushAuthClient] Authenticating over unencrypted remote HTTP ({baseHttpUrl}); use https:// in production."); }
                catch { /* headless test runner */ }
            }

            string url = baseHttpUrl + endpoint;
            byte[] bytes = Encoding.UTF8.GetBytes(payload.ToJson());

            using (var req = new UnityWebRequest(url, UnityWebRequest.kHttpVerbPOST))
            {
                req.uploadHandler = new UploadHandlerRaw(bytes);
                req.downloadHandler = new DownloadHandlerBuffer();
                req.SetRequestHeader("Content-Type", "application/json");
                req.timeout = 10;

                var tcs = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
                var op = req.SendWebRequest();
                if (op.isDone)
                    tcs.TrySetResult(true);
                else
                    op.completed += _ => tcs.TrySetResult(true);
                await tcs.Task;

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
        }
    }
}
