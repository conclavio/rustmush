using System;
using System.Collections.Generic;
using System.Text;
using System.Threading.Tasks;
using TacSim.Core;
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
        public static string NormalizeBaseUrl(string raw)
        {
            if (string.IsNullOrWhiteSpace(raw))
                return "http://127.0.0.1:8080";
            string s = raw.Trim().TrimEnd('/');
            if (!s.StartsWith("http://", StringComparison.OrdinalIgnoreCase) &&
                !s.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
            {
                s = "http://" + s;
            }
            return s;
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
            string url = baseHttpUrl + endpoint;
            byte[] bytes = Encoding.UTF8.GetBytes(payload.ToJson());

            using (var req = new UnityWebRequest(url, UnityWebRequest.kHttpVerbPOST))
            {
                req.uploadHandler = new UploadHandlerRaw(bytes);
                req.downloadHandler = new DownloadHandlerBuffer();
                req.SetRequestHeader("Content-Type", "application/json");
                req.timeout = 10;

                var op = req.SendWebRequest();
                while (!op.isDone)
                {
                    await Task.Yield();
                }

                string text = req.downloadHandler != null ? req.downloadHandler.text : string.Empty;
                if (req.result != UnityWebRequest.Result.Success)
                {
                    var errNode = JsonNode.Parse(text);
                    string serverMsg = errNode.IsObject ? errNode.Str("error", text) : text;
                    if (string.IsNullOrWhiteSpace(serverMsg)) serverMsg = req.error;
                    throw new InvalidOperationException($"Auth failed ({req.responseCode}): {serverMsg}");
                }

                var json = JsonNode.Parse(text);
                if (!json.IsObject || string.IsNullOrEmpty(json.Str("token")))
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
