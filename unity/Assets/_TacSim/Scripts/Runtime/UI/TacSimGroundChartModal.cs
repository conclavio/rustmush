using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using TacSim.Core;
using TacSim.Net;
using TacSim.World;
using UnityEngine;
using UnityEngine.EventSystems;
using UnityEngine.UI;

namespace TacSim.UI
{
    public sealed class GroundChartPort
    {
        public string Glyph = "P";
        public string Port = string.Empty;
        public int LatMdeg;
        public int LonMdeg;
        public bool Drawn;
    }

    public sealed class GroundChartSite
    {
        public string Letter = "A";
        public string Site = string.Empty;
        public string Entry = string.Empty;
        public string Venue = string.Empty;
        public int LatMdeg;
        public int LonMdeg;
        public string Survey = string.Empty;
        public string Tenure = string.Empty;
        public string Passage = string.Empty;
        public bool Here;
        public string Best = string.Empty;
        public bool Dark;
        public string BestLine = string.Empty;
    }

    /// <summary>
    /// Decoded live `body.ground` planetary survey grid &amp; prospecting site listing
    /// (mirrors `crates/tacsim/src/port/ground_chart.rs` and `web/src/lib/quarry.ts`).
    /// </summary>
    public sealed class GroundChartFrame
    {
        public string Title = string.Empty;
        public int Width;
        public int Height;
        public char[] Glyphs;
        public string Body = string.Empty;
        public string Name = string.Empty;
        public string Family = string.Empty;
        public string FamilyWord = string.Empty;
        public string Footer = string.Empty;
        public readonly List<string> Families = new List<string>();
        public readonly List<GroundChartPort> Ports = new List<GroundChartPort>();
        public readonly List<GroundChartSite> Sites = new List<GroundChartSite>();
        public readonly List<string> Lines = new List<string>();

        public static Vector2 ProjectLatLonToUv(int latMdeg, int lonMdeg)
        {
            float u = Mathf.Clamp01(lonMdeg / 360000f);
            float v = Mathf.Clamp01((latMdeg + 90000f) / 180000f);
            return new Vector2(u, v);
        }

        public static GroundChartFrame FromJson(JsonNode raw)
        {
            if (raw == null || !raw.IsObject || raw.Bool("refused", false))
                return null;

            var g = raw["ground"];
            if (g == null || !g.IsObject)
                return null;

            int w = Mathf.Clamp(raw.Int("width", 56), 1, 256);
            int h = Mathf.Clamp(raw.Int("height", 28), 1, 256);
            var rowsNode = raw["rows"];
            char[] glyphs = (rowsNode != null && rowsNode.IsArray && rowsNode.AsArray().Count > 0)
                ? TacSimTerrainDecoder.DecodeRleGlyphRows(rowsNode, w, h)
                : new char[w * h];

            var frame = new GroundChartFrame
            {
                Title = raw.Str("title", g.Str("name", "Planetary Ground Survey")),
                Width = w,
                Height = h,
                Glyphs = glyphs,
                Body = g.Str("body", string.Empty),
                Name = g.Str("name", string.Empty),
                Family = g.Str("family", string.Empty),
                FamilyWord = g.Str("family_word", g.Str("family", "volatiles")),
                Footer = g.Str("footer", string.Empty)
            };

            var fams = g["families"].AsArray();
            if (fams != null)
            {
                for (int i = 0; i < fams.Count; i++)
                {
                    string f = fams[i].AsString().Trim();
                    if (!string.IsNullOrEmpty(f))
                        frame.Families.Add(f);
                }
            }

            var portsArr = g["ports"].AsArray();
            if (portsArr != null)
            {
                for (int i = 0; i < portsArr.Count; i++)
                {
                    var p = portsArr[i];
                    if (!p.IsObject) continue;
                    frame.Ports.Add(new GroundChartPort
                    {
                        Glyph = p.Str("glyph", "P"),
                        Port = p.Str("port", string.Empty),
                        LatMdeg = p.Int("lat_mdeg", 0),
                        LonMdeg = p.Int("lon_mdeg", 0),
                        Drawn = p.Bool("drawn", true)
                    });
                }
            }

            var sitesArr = g["sites"].AsArray();
            if (sitesArr != null)
            {
                for (int i = 0; i < sitesArr.Count; i++)
                {
                    var s = sitesArr[i];
                    if (!s.IsObject) continue;
                    frame.Sites.Add(new GroundChartSite
                    {
                        Letter = s.Str("letter", "?"),
                        Site = s.Str("site", string.Empty),
                        Entry = s.Str("entry", string.Empty),
                        Venue = s.Str("venue", string.Empty),
                        LatMdeg = s.Int("lat_mdeg", 0),
                        LonMdeg = s.Int("lon_mdeg", 0),
                        Survey = s.Str("survey", string.Empty),
                        Tenure = s.Str("tenure", string.Empty),
                        Passage = s.Str("passage", string.Empty),
                        Here = s.Bool("here", false),
                        Best = s.Str("best", string.Empty),
                        Dark = s.Bool("dark", false),
                        BestLine = s.Str("best_line", string.Empty)
                    });
                }
            }

            var linesArr = g["lines"].AsArray();
            if (linesArr != null)
            {
                for (int i = 0; i < linesArr.Count; i++)
                {
                    string ln = linesArr[i].AsString();
                    if (!string.IsNullOrWhiteSpace(ln))
                        frame.Lines.Add(ln);
                }
            }

            return frame;
        }

        public GroundChartSite FindSiteByVenueOrLetter(string key)
        {
            if (string.IsNullOrWhiteSpace(key)) return null;
            string trimmed = key.Trim();
            for (int i = 0; i < Sites.Count; i++)
            {
                if (string.Equals(Sites[i].Venue, trimmed, StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(Sites[i].Letter, trimmed, StringComparison.OrdinalIgnoreCase) ||
                    string.Equals(Sites[i].Site, trimmed, StringComparison.OrdinalIgnoreCase))
                {
                    return Sites[i];
                }
            }
            return null;
        }

        public GroundChartSite FindNearestSite(Vector2 uv, float maxDistUv = 0.08f)
        {
            GroundChartSite best = null;
            float bestDist = maxDistUv;
            for (int i = 0; i < Sites.Count; i++)
            {
                Vector2 suv = ProjectLatLonToUv(Sites[i].LatMdeg, Sites[i].LonMdeg);
                float d = Vector2.Distance(uv, suv);
                if (d < bestDist)
                {
                    bestDist = d;
                    best = Sites[i];
                }
            }
            return best;
        }
    }

    /// <summary>
    /// Live `[G]` Planetary Ground Survey &amp; Landing Sites modal driven strictly by `mushd`'s `body.ground` feed.
    /// Renders the 56x28 equirectangular mineral survey wash map, port &amp; site beacons, ore-family wash switcher,
    /// site dossier, and one-click corridor sailing plan actions (`+ship/plan`, `+ship/order depart`, `+chart`).
    /// </summary>
    public sealed class TacSimGroundChartModal : MonoBehaviour
    {
        public event Action<string> OnRefreshGroundChartRequested; // (familyOrNull)
        public event Action<string> OnCommandRequested;
        public event Action<string> OnOpenSailingPlanRequested;

        private const int TexWidth = 560;
        private const int TexHeight = 280;
        private const int MaxSiteRows = 12;
        private const int MaxFamilyButtons = 6;

        private sealed class PooledSiteRow
        {
            public GameObject Root;
            public Image Bg;
            public Button Btn;
            public Text Label;
            public string BoundVenue;
            public string BoundLetter;
        }

        private sealed class PooledFamilyBtn
        {
            public GameObject Root;
            public Image Bg;
            public Button Btn;
            public Text Label;
            public string BoundFamily;
        }

        private Font _font;
        private GameObject _overlay;
        private Text _headingText;
        private Text _footerText;
        private Text _dossierText;
        private GameObject _questBanner;
        private Text _questBannerText;
        private Button _questWashBtn;
        private Text _questWashBtnLbl;
        private RectTransform _sitesListBoxRect;
        private TacSimQuestJournal _questJournal;
        private TacSimTradeAndShipState _tradeState;
        private string _questRecommendedFamily = string.Empty;
        private string _questReturnPortId = string.Empty;
        private RawImage _mapImage;
        private RectTransform _mapRect;
        private Texture2D _mapTex;
        private Color32[] _pixelBuffer;
        private readonly List<PooledSiteRow> _sitePool = new List<PooledSiteRow>(MaxSiteRows);
        private readonly List<PooledFamilyBtn> _familyPool = new List<PooledFamilyBtn>(MaxFamilyButtons);

        public bool IsOpen => _overlay != null && _overlay.activeSelf;
        public bool HasQuestMiningBanner => _questBanner != null && _questBanner.activeSelf;
        public string QuestMiningBannerText => _questBannerText != null ? _questBannerText.text : string.Empty;
        public string QuestBannerButtonLabelSnapshot => _questWashBtnLbl != null ? _questWashBtnLbl.text : string.Empty;
        public GroundChartFrame CurrentFrame { get; private set; }
        public string ActiveFamily { get; private set; } = "volatiles";
        public string SelectedSiteVenue { get; private set; } = string.Empty;
        public string SelectedSiteLetter { get; private set; } = string.Empty;
        public string FirstListedSiteVenueSnapshot =>
            _sitePool.Count > 0 && _sitePool[0].Root != null && _sitePool[0].Root.activeSelf
                ? (_sitePool[0].BoundVenue ?? string.Empty)
                : string.Empty;
        public string FirstListedSiteLabelSnapshot =>
            _sitePool.Count > 0 && _sitePool[0].Root != null && _sitePool[0].Root.activeSelf && _sitePool[0].Label != null
                ? _sitePool[0].Label.text
                : string.Empty;

        private static readonly string[] DefaultFamilies = { "volatiles", "metals", "strategic", "carbon" };

        private sealed class GroundMapPointerRelay : MonoBehaviour, IPointerClickHandler
        {
            public Action<PointerEventData> OnClick;
            public void OnPointerClick(PointerEventData eventData) => OnClick?.Invoke(eventData);
        }

        public static Color32 WashColorForGlyph(char g)
        {
            switch (g)
            {
                case '.': return new Color32(42, 46, 56, 255);    // barren
                case '-': return new Color32(84, 102, 78, 255);   // lean
                case '+': return new Color32(112, 138, 166, 255); // fair
                case '*': return new Color32(218, 166, 66, 255);  // rich
                case '=': return new Color32(138, 184, 212, 255); // ice
                case '^': return new Color32(74, 78, 86, 255);    // ridge
                case 'O': return new Color32(44, 42, 40, 255);    // crater rim
                case ' ': return new Color32(16, 20, 28, 255);
                default: return new Color32(142, 192, 216, 255);  // site/port mark
            }
        }

        public void BuildModal(Transform parent, Font sharedFont)
        {
            _font = sharedFont != null ? sharedFont : Resources.GetBuiltinResource<Font>("LegacyRuntime.ttf");
            _sitePool.Clear();
            _familyPool.Clear();

            _overlay = CreatePanel(parent, "PlanetaryGroundChartModal", Vector2.zero, Vector2.one, Vector2.zero, Vector2.zero, new Color(0.01f, 0.03f, 0.06f, 0.84f));

            var card = CreatePanel(_overlay.transform, "GroundChartCard", new Vector2(0.06f, 0.07f), new Vector2(0.94f, 0.93f), Vector2.zero, Vector2.zero, new Color(0.03f, 0.06f, 0.11f, 0.99f));
            CreatePanel(card.transform, "TopAccent", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(0f, -3f), Vector2.zero, new Color(0.32f, 0.92f, 0.72f, 0.95f));

            _headingText = CreateText(card.transform, "PLANETARY GROUND SURVEY CHART [G]  //  WAITING FOR LIVE BODY.GROUND...", 16, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.48f, 0.96f, 0.84f));
            SetRect(_headingText.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(18f, -44f), new Vector2(-240f, -8f));

            var refreshBtn = CreateButton(card.transform, "REFRESH GROUND", new Color(0.14f, 0.42f, 0.52f), () => OnRefreshGroundChartRequested?.Invoke(ActiveFamily));
            SetRect(refreshBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-232f, -40f), new Vector2(-106f, -10f));

            var closeBtn = CreateButton(card.transform, "CLOSE [ESC]", new Color(0.42f, 0.16f, 0.18f), () => SetOpen(false));
            SetRect(closeBtn.GetComponent<RectTransform>(), new Vector2(1f, 1f), new Vector2(1f, 1f), new Vector2(-98f, -40f), new Vector2(-14f, -10f));

            // Left pane: Ore-Family Wash Bar + 2:1 Equirectangular Survey Wash Map + Legend/Footer
            var leftPane = CreatePanel(card.transform, "GroundMapPane", new Vector2(0f, 0f), new Vector2(0.58f, 1f), new Vector2(16f, 14f), new Vector2(-8f, -52f), new Color(0.02f, 0.04f, 0.08f, 0.96f));

            var famRow = CreatePanel(leftPane.transform, "GroundFamilyRow", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(8f, -38f), new Vector2(-8f, -6f), Color.clear);
            var fHlg = famRow.AddComponent<HorizontalLayoutGroup>();
            fHlg.spacing = 5f;
            fHlg.childControlWidth = true;
            fHlg.childControlHeight = true;
            fHlg.childForceExpandWidth = true;

            for (int i = 0; i < MaxFamilyButtons; i++)
            {
                var btn = CreateButton(famRow.transform, i < DefaultFamilies.Length ? DefaultFamilies[i].ToUpperInvariant() : string.Empty, new Color(0.12f, 0.24f, 0.34f), null);
                var entry = new PooledFamilyBtn
                {
                    Root = btn.gameObject,
                    Bg = btn.GetComponent<Image>(),
                    Btn = btn,
                    Label = btn.GetComponentInChildren<Text>(),
                    BoundFamily = i < DefaultFamilies.Length ? DefaultFamilies[i] : string.Empty
                };
                btn.onClick.AddListener(() =>
                {
                    if (!string.IsNullOrEmpty(entry.BoundFamily))
                        SelectFamily(entry.BoundFamily);
                });
                entry.Root.SetActive(i < DefaultFamilies.Length);
                _familyPool.Add(entry);
            }

            var viewportGo = new GameObject("GroundMapViewport", typeof(RectTransform), typeof(RawImage));
            viewportGo.transform.SetParent(leftPane.transform, false);
            _mapRect = viewportGo.GetComponent<RectTransform>();
            SetRect(_mapRect, new Vector2(0f, 0f), new Vector2(1f, 1f), new Vector2(10f, 44f), new Vector2(-10f, -46f));
            _mapImage = viewportGo.GetComponent<RawImage>();
            _mapImage.raycastTarget = true;

            _mapTex = new Texture2D(TexWidth, TexHeight, TextureFormat.RGBA32, false)
            {
                filterMode = FilterMode.Point,
                wrapMode = TextureWrapMode.Clamp
            };
            _pixelBuffer = new Color32[TexWidth * TexHeight];
            _mapImage.texture = _mapTex;

            var relay = viewportGo.AddComponent<GroundMapPointerRelay>();
            relay.OnClick = HandleMapClick;

            _footerText = CreateText(leftPane.transform, "Wash: [.] Barren  [-] Lean  [+] Fair  [*] Rich  |  Click any beacon to inspect landing site & plot passage", 11, FontStyle.Normal, TextAnchor.MiddleLeft, new Color(0.68f, 0.84f, 0.94f));
            SetRect(_footerText.rectTransform, new Vector2(0f, 0f), new Vector2(1f, 0f), new Vector2(12f, 8f), new Vector2(-12f, 38f));

            // Right pane: Prospecting Sites Ledger + Dossier + One-Click Actions
            var rightPane = CreatePanel(card.transform, "GroundSitesSidebar", new Vector2(0.58f, 0f), new Vector2(1f, 1f), new Vector2(8f, 14f), new Vector2(-16f, -52f), new Color(0.03f, 0.06f, 0.10f, 0.96f));

            var listHdr = CreateText(rightPane.transform, "PLANETARY PROSPECTING SITES & LANDING VENUES", 13, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(0.76f, 0.92f, 1f));
            SetRect(listHdr.rectTransform, new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -30f), new Vector2(-10f, -4f));

            _questBanner = CreatePanel(rightPane.transform, "QuestMiningBanner", new Vector2(0f, 1f), new Vector2(1f, 1f), new Vector2(10f, -64f), new Vector2(-10f, -34f), new Color(0.24f, 0.18f, 0.05f, 0.96f));
            _questBannerText = CreateText(_questBanner.transform, string.Empty, 11, FontStyle.Bold, TextAnchor.MiddleLeft, new Color(1f, 0.88f, 0.44f));
            _questBannerText.supportRichText = true;
            SetRect(_questBannerText.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-112f, 0f));
            _questWashBtn = CreateButton(_questBanner.transform, "CARBON WASH", new Color(0.62f, 0.44f, 0.12f), () => TriggerQuestBannerAction());
            SetRect(_questWashBtn.GetComponent<RectTransform>(), new Vector2(1f, 0f), new Vector2(1f, 1f), new Vector2(-106f, 3f), new Vector2(-4f, -3f));
            _questWashBtnLbl = _questWashBtn.GetComponentInChildren<Text>();
            _questBanner.SetActive(false);

            var listBox = CreatePanel(rightPane.transform, "SitesListBox", new Vector2(0f, 0.44f), new Vector2(1f, 1f), new Vector2(10f, 4f), new Vector2(-10f, -34f), new Color(0.02f, 0.04f, 0.07f, 0.92f));
            _sitesListBoxRect = listBox.GetComponent<RectTransform>();
            var vlg = listBox.AddComponent<VerticalLayoutGroup>();
            vlg.spacing = 4f;
            vlg.padding = new RectOffset(6, 6, 6, 6);
            vlg.childControlWidth = true;
            vlg.childControlHeight = false;
            vlg.childForceExpandWidth = true;
            vlg.childForceExpandHeight = false;

            for (int i = 0; i < MaxSiteRows; i++)
            {
                var rowGo = CreatePanel(listBox.transform, $"GroundSiteRow_{i}", new Vector2(0f, 1f), new Vector2(1f, 1f), Vector2.zero, new Vector2(0f, 26f), new Color(0.07f, 0.13f, 0.20f, 0.92f));
                rowGo.GetComponent<RectTransform>().sizeDelta = new Vector2(0f, 26f);
                var bg = rowGo.GetComponent<Image>();
                bg.raycastTarget = true;
                var btn = rowGo.AddComponent<Button>();
                btn.targetGraphic = bg;
                var lbl = CreateText(rowGo.transform, string.Empty, 11, FontStyle.Bold, TextAnchor.MiddleLeft, Color.white);
                lbl.supportRichText = true;
                SetRect(lbl.rectTransform, Vector2.zero, Vector2.one, new Vector2(8f, 0f), new Vector2(-8f, 0f));

                var row = new PooledSiteRow { Root = rowGo, Bg = bg, Btn = btn, Label = lbl };
                btn.onClick.AddListener(() => SelectSite(row.BoundVenue, row.BoundLetter));
                rowGo.SetActive(false);
                _sitePool.Add(row);
            }

            var dossierBox = CreatePanel(rightPane.transform, "GroundSiteDossier", new Vector2(0f, 0.16f), new Vector2(1f, 0.43f), new Vector2(10f, 4f), new Vector2(-10f, -4f), new Color(0.04f, 0.08f, 0.13f, 0.95f));
            _dossierText = CreateText(dossierBox.transform, "Click <b>REFRESH GROUND</b> or press <b>[G]</b> while in-system to load the live planetary survey wash (`body.ground`).", 12, FontStyle.Normal, TextAnchor.UpperLeft, new Color(0.86f, 0.94f, 0.98f));
            _dossierText.supportRichText = true;
            _dossierText.horizontalOverflow = HorizontalWrapMode.Wrap;
            _dossierText.verticalOverflow = VerticalWrapMode.Truncate;
            SetRect(_dossierText.rectTransform, Vector2.zero, Vector2.one, new Vector2(10f, 6f), new Vector2(-10f, -6f));

            var act1 = CreatePanel(rightPane.transform, "GroundActs1", new Vector2(0f, 0.08f), new Vector2(1f, 0.15f), new Vector2(10f, 2f), new Vector2(-10f, -2f), Color.clear);
            var a1 = act1.AddComponent<HorizontalLayoutGroup>();
            a1.spacing = 6f; a1.childControlWidth = true; a1.childControlHeight = true; a1.childForceExpandWidth = true;
            CreateButton(act1.transform, "PLAN PASSAGE (+ship/plan)", new Color(0.14f, 0.46f, 0.58f), DispatchPlanPassage);
            CreateButton(act1.transform, "QUEUE DEPART", new Color(0.16f, 0.54f, 0.40f), DispatchQueueDepart);

            var act2 = CreatePanel(rightPane.transform, "GroundActs2", new Vector2(0f, 0.01f), new Vector2(1f, 0.08f), new Vector2(10f, 2f), new Vector2(-10f, -2f), Color.clear);
            var a2 = act2.AddComponent<HorizontalLayoutGroup>();
            a2.spacing = 6f; a2.childControlWidth = true; a2.childControlHeight = true; a2.childForceExpandWidth = true;
            CreateButton(act2.transform, "CHART WINDOW (+chart)", new Color(0.24f, 0.34f, 0.52f), DispatchChartWindow);
            CreateButton(act2.transform, "OPEN SAILING PLAN", new Color(0.46f, 0.34f, 0.14f), DispatchOpenSailingPlan);

            RedrawMapTexture();
            _overlay.SetActive(false);
        }

        private void OnDestroy()
        {
            if (_questJournal != null)
            {
                _questJournal.OnQuestsPanelUpdated -= RefreshQuestMiningBanner;
                _questJournal = null;
            }
            if (_tradeState != null)
            {
                _tradeState.OnStateChanged -= RefreshQuestMiningBanner;
                _tradeState = null;
            }
            if (_mapTex != null)
            {
                if (Application.isPlaying) Destroy(_mapTex);
                else DestroyImmediate(_mapTex);
                _mapTex = null;
            }
        }

        public void BindQuestJournal(TacSimQuestJournal journal)
        {
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated -= RefreshQuestMiningBanner;
            _questJournal = journal;
            if (_questJournal != null)
                _questJournal.OnQuestsPanelUpdated += RefreshQuestMiningBanner;
            RefreshQuestMiningBanner();
        }

        public void BindTradeState(TacSimTradeAndShipState trade)
        {
            if (_tradeState != null)
                _tradeState.OnStateChanged -= RefreshQuestMiningBanner;
            _tradeState = trade;
            if (_tradeState != null)
                _tradeState.OnStateChanged += RefreshQuestMiningBanner;
            RefreshQuestMiningBanner();
        }

        public bool TriggerQuestBannerAction()
        {
            if (!string.IsNullOrEmpty(_questReturnPortId))
            {
                OnCommandRequested?.Invoke($"+ship/plan {_questReturnPortId}");
                return true;
            }
            return ApplyQuestWashFilter();
        }

        public bool ApplyQuestWashFilter()
        {
            if (string.IsNullOrEmpty(_questRecommendedFamily)) return false;
            SelectFamily(_questRecommendedFamily);
            return true;
        }

        private static bool MatchesQuestOreText(string text, in QuestSurfaceMiningStatus qMine)
        {
            if (string.IsNullOrEmpty(text)) return false;
            string low = text.ToLowerInvariant();
            string slug = (qMine.TargetOreSlug ?? string.Empty).ToLowerInvariant();
            string spaced = slug.Replace('_', ' ');
            string fam = (qMine.RecommendedFamily ?? string.Empty).ToLowerInvariant();
            if (!string.IsNullOrEmpty(slug) && (low.Contains(slug, StringComparison.Ordinal) || low.Contains(spaced, StringComparison.Ordinal)))
                return true;
            return !string.IsNullOrEmpty(fam) && low.Contains(fam, StringComparison.Ordinal);
        }

        private static bool MatchesQuestSite(GroundChartSite site, in QuestSurfaceMiningStatus qMine)
        {
            if (site == null) return false;
            return MatchesQuestOreText(site.BestLine, qMine) || MatchesQuestOreText(site.Best, qMine);
        }

        public bool TryGetRecommendedQuestSite(out GroundChartSite site)
        {
            site = null;
            if (CurrentFrame == null || _questJournal == null) return false;
            if (!_questJournal.TryGetActiveSurfaceMiningStatus(out var qMine) || qMine.MineDone)
                return false;

            for (int i = 0; i < CurrentFrame.Sites.Count; i++)
            {
                var s = CurrentFrame.Sites[i];
                if (!s.Here && !string.IsNullOrEmpty(s.Venue) && MatchesQuestSite(s, qMine))
                {
                    site = s;
                    return true;
                }
            }
            string[] starterVenues = { "commons", "ahuna" };
            for (int v = 0; v < starterVenues.Length; v++)
            {
                for (int i = 0; i < CurrentFrame.Sites.Count; i++)
                {
                    var s = CurrentFrame.Sites[i];
                    if (!s.Here && string.Equals(s.Venue, starterVenues[v], StringComparison.OrdinalIgnoreCase))
                    {
                        site = s;
                        return true;
                    }
                }
            }
            for (int i = 0; i < CurrentFrame.Sites.Count; i++)
            {
                var s = CurrentFrame.Sites[i];
                if (!s.Here && !string.IsNullOrEmpty(s.Venue) &&
                    !string.IsNullOrEmpty(s.Tenure) &&
                    s.Tenure.IndexOf("open", StringComparison.OrdinalIgnoreCase) >= 0)
                {
                    site = s;
                    return true;
                }
            }
            return false;
        }

        private void RefreshQuestMiningBanner()
        {
            if (_questBanner == null) return;
            _questReturnPortId = string.Empty;
            if (_questJournal != null && _questJournal.TryGetActiveSurfaceMiningStatus(out var qMine))
            {
                _questRecommendedFamily = qMine.RecommendedFamily;
                var tracked = _questJournal.GetTrackedActiveQuest();
                int stepNo = TacSimQuestJournal.GuidedStepNumber(tracked?.Key);
                string stepTag = stepNo > 0 ? $" [Step {stepNo}/7]" : string.Empty;
                string scanMark = qMine.ScanDone ? "<color=#55EE99>[✓]</color>" : "<color=#99BBCC>[ ]</color>";
                string soundMark = qMine.SoundDone ? "<color=#55EE99>[✓]</color>" : "<color=#99BBCC>[ ]</color>";
                string mineMark = qMine.MineDone ? "<color=#55EE99>[✓]</color>" : "<color=#99BBCC>[ ]</color>";
                string holdTxt = _tradeState != null
                    ? $"  ·  Hold: {_tradeState.GetHoldCommodityQuantity(qMine.TargetOreSlug)}t"
                    : string.Empty;
                string extra = string.Empty;
                if (qMine.MineDone)
                {
                    string retPortId;
                    string retPortLabel;
                    if (_questJournal.TryGetActiveDestinationPort(out var returnDest))
                    {
                        retPortId = returnDest.PortId;
                        retPortLabel = returnDest.PortLabel;
                    }
                    else
                    {
                        retPortId = TacSimQuestJournal.InferStationPortFromRoomEntry(tracked?.NextRoomEntry);
                        if (string.IsNullOrEmpty(retPortId)) retPortId = "ceres";
                        retPortLabel = retPortId switch
                        {
                            "stickney" => "Stickney Yards",
                            "luna" => "Aitken Basin",
                            "deimos" => "Voltaire Base",
                            "titan" => "Ligeia Dock",
                            _ => "Ceres Freeport"
                        };
                    }
                    _questReturnPortId = retPortId;
                    extra = $"  ·  <color=#55EE99>Return: {MushProtocol.EscapeUnityRichText(retPortLabel)} (+ship/plan {MushProtocol.EscapeUnityRichText(retPortId)})</color>";
                }
                else if (TryGetRecommendedQuestSite(out var recSite))
                {
                    extra = $"  ·  <color=#FFDD44>Site: [!]{MushProtocol.EscapeUnityRichText(recSite.Site)} (+ship/plan {MushProtocol.EscapeUnityRichText(recSite.Venue)})</color>";
                }
                if (_questBannerText != null)
                    _questBannerText.text = $"[QUEST]{stepTag} {scanMark} Scan  {soundMark} Sound  {mineMark} Mine {MushProtocol.EscapeUnityRichText(qMine.TargetOreSlug)}{holdTxt}{extra}";
                if (_questWashBtnLbl != null)
                {
                    _questWashBtnLbl.text = !string.IsNullOrEmpty(_questReturnPortId)
                        ? $"RETURN: {_questReturnPortId.ToUpperInvariant()}"
                        : $"{qMine.RecommendedFamily.ToUpperInvariant()} WASH";
                }
                _questBanner.SetActive(true);
                if (_sitesListBoxRect != null)
                    _sitesListBoxRect.offsetMax = new Vector2(-10f, -68f);
            }
            else
            {
                _questRecommendedFamily = string.Empty;
                _questBanner.SetActive(false);
                if (_sitesListBoxRect != null)
                    _sitesListBoxRect.offsetMax = new Vector2(-10f, -34f);
            }
        }

        public void ResetSessionState()
        {
            CurrentFrame = null;
            SelectedSiteVenue = string.Empty;
            SelectedSiteLetter = string.Empty;
            ActiveFamily = "volatiles";
            SetOpen(false);
            RefreshUi();
        }

        public void SetOpen(bool open)
        {
            if (_overlay == null) return;
            _overlay.SetActive(open);
            if (open)
            {
                OnRefreshGroundChartRequested?.Invoke(ActiveFamily);
                RefreshUi();
            }
        }

        public void ToggleOpen() => SetOpen(!IsOpen);

        public void SelectFamily(string family)
        {
            if (string.IsNullOrWhiteSpace(family)) return;
            ActiveFamily = family.Trim().ToLowerInvariant();
            RefreshFamilyButtons();
            OnRefreshGroundChartRequested?.Invoke(ActiveFamily);
        }

        public void SelectSite(string venue, string letter = null)
        {
            SelectedSiteVenue = venue ?? string.Empty;
            SelectedSiteLetter = letter ?? string.Empty;
            RefreshSiteRowsAndDossier();
            RedrawMapTexture();
        }

        public void ApplyGroundFrame(JsonNode data)
        {
            var frame = GroundChartFrame.FromJson(data);
            if (frame == null) return;
            CurrentFrame = frame;
            if (!string.IsNullOrWhiteSpace(frame.FamilyWord))
                ActiveFamily = frame.FamilyWord.Trim().ToLowerInvariant();

            // Preserve selected site if still present, else default to current `here` site,
            // then recommended active-quest landing site, else first listed site.
            var existing = frame.FindSiteByVenueOrLetter(!string.IsNullOrEmpty(SelectedSiteVenue) ? SelectedSiteVenue : SelectedSiteLetter);
            if (existing == null && frame.Sites.Count > 0)
            {
                for (int i = 0; i < frame.Sites.Count; i++)
                {
                    if (frame.Sites[i].Here) { existing = frame.Sites[i]; break; }
                }
                if (existing == null && TryGetRecommendedQuestSite(out var recSite))
                    existing = recSite;
                if (existing == null)
                    existing = frame.Sites[0];
            }
            if (existing != null)
            {
                SelectedSiteVenue = existing.Venue;
                SelectedSiteLetter = existing.Letter;
            }
            RefreshUi();
        }

        private bool TryResolveSelectedVenue(out string safeVenue)
        {
            safeVenue = null;
            if (string.IsNullOrWhiteSpace(SelectedSiteVenue)) return false;
            return MushProtocol.TryValidateSafeCommandToken(SelectedSiteVenue, out safeVenue, maxLen: 48);
        }

        public void DispatchPlanPassage()
        {
            if (TryResolveSelectedVenue(out string venue))
                OnCommandRequested?.Invoke($"+ship/plan {venue}");
            else
                OnCommandRequested?.Invoke("+chart/ground");
        }

        public void DispatchQueueDepart()
        {
            if (TryResolveSelectedVenue(out string venue))
                OnCommandRequested?.Invoke($"+ship/order depart {venue}");
        }

        public void DispatchChartWindow()
        {
            if (TryResolveSelectedVenue(out string venue))
                OnCommandRequested?.Invoke($"+chart {venue}");
            else
                OnCommandRequested?.Invoke("+chart/ground");
        }

        public void DispatchOpenSailingPlan()
        {
            string venue = TryResolveSelectedVenue(out string safeVenue) ? safeVenue : string.Empty;
            SetOpen(false);
            OnOpenSailingPlanRequested?.Invoke(venue);
        }

        private void HandleMapClick(PointerEventData eventData)
        {
            if (CurrentFrame == null || _mapRect == null || eventData == null) return;
            if (!RectTransformUtility.ScreenPointToLocalPointInRectangle(_mapRect, eventData.position, eventData.pressEventCamera, out Vector2 local))
                return;
            Rect r = _mapRect.rect;
            if (r.width <= 1f || r.height <= 1f) return;
            Vector2 uv = new Vector2((local.x - r.x) / r.width, (local.y - r.y) / r.height);
            var hit = CurrentFrame.FindNearestSite(uv);
            if (hit != null)
                SelectSite(hit.Venue, hit.Letter);
        }

        private void RefreshUi()
        {
            if (_headingText != null)
            {
                _headingText.text = CurrentFrame != null
                    ? $"PLANETARY GROUND SURVEY [G]  //  {CurrentFrame.Title.ToUpperInvariant()} [{ActiveFamily.ToUpperInvariant()}]"
                    : "PLANETARY GROUND SURVEY CHART [G]  //  PRESS REFRESH GROUND FOR LIVE SURVEY";
            }
            if (_footerText != null)
            {
                _footerText.text = CurrentFrame != null && !string.IsNullOrEmpty(CurrentFrame.Footer)
                    ? $"{CurrentFrame.Footer}  |  Wash: [.] Barren [-] Lean [+] Fair [*] Rich"
                    : "Wash: [.] Barren  [-] Lean  [+] Fair  [*] Rich  |  Click any beacon to inspect landing site & plot passage";
            }
            RefreshQuestMiningBanner();
            RefreshFamilyButtons();
            RefreshSiteRowsAndDossier();
            RedrawMapTexture();
        }

        private void RefreshFamilyButtons()
        {
            IReadOnlyList<string> fams = (CurrentFrame != null && CurrentFrame.Families.Count > 0)
                ? (IReadOnlyList<string>)CurrentFrame.Families
                : DefaultFamilies;

            for (int i = 0; i < _familyPool.Count; i++)
            {
                var item = _familyPool[i];
                if (i < fams.Count)
                {
                    item.BoundFamily = fams[i].ToLowerInvariant();
                    if (item.Label != null) item.Label.text = fams[i].ToUpperInvariant();
                    bool sel = string.Equals(item.BoundFamily, ActiveFamily, StringComparison.OrdinalIgnoreCase);
                    item.Bg.color = sel ? new Color(0.18f, 0.56f, 0.72f, 1f) : new Color(0.10f, 0.20f, 0.30f, 0.94f);
                    item.Root.SetActive(true);
                }
                else
                {
                    item.BoundFamily = string.Empty;
                    item.Root.SetActive(false);
                }
            }
        }

        private void RefreshSiteRowsAndDossier()
        {
            int count = CurrentFrame != null ? CurrentFrame.Sites.Count : 0;
            QuestSurfaceMiningStatus qMineRow = default;
            bool hasQuestMining = _questJournal != null && _questJournal.TryGetActiveSurfaceMiningStatus(out qMineRow);
            string recVenue = TryGetRecommendedQuestSite(out var recRowSite) ? recRowSite.Venue : string.Empty;
            List<GroundChartSite> orderedSites = null;
            if (count > 0)
            {
                if (hasQuestMining && !qMineRow.MineDone)
                {
                    orderedSites = new List<GroundChartSite>(count);
                    var recSites = new List<GroundChartSite>(2);
                    var restSites = new List<GroundChartSite>(count);
                    for (int i = 0; i < count; i++)
                    {
                        var s = CurrentFrame.Sites[i];
                        if (MatchesQuestSite(s, qMineRow))
                            orderedSites.Add(s);
                        else if (!string.IsNullOrEmpty(recVenue) && string.Equals(s.Venue, recVenue, StringComparison.OrdinalIgnoreCase))
                            recSites.Add(s);
                        else
                            restSites.Add(s);
                    }
                    orderedSites.AddRange(recSites);
                    orderedSites.AddRange(restSites);
                }
                else
                {
                    orderedSites = CurrentFrame.Sites;
                }
            }
            for (int i = 0; i < _sitePool.Count; i++)
            {
                var row = _sitePool[i];
                if (orderedSites != null && i < orderedSites.Count)
                {
                    var s = orderedSites[i];
                    row.BoundVenue = s.Venue;
                    row.BoundLetter = s.Letter;
                    bool sel = (!string.IsNullOrEmpty(SelectedSiteVenue) && string.Equals(s.Venue, SelectedSiteVenue, StringComparison.OrdinalIgnoreCase)) ||
                               (!string.IsNullOrEmpty(SelectedSiteLetter) && string.Equals(s.Letter, SelectedSiteLetter, StringComparison.OrdinalIgnoreCase));
                    bool isQuestSite = hasQuestMining && !qMineRow.MineDone &&
                                       (MatchesQuestSite(s, qMineRow) ||
                                        (!string.IsNullOrEmpty(recVenue) && string.Equals(s.Venue, recVenue, StringComparison.OrdinalIgnoreCase)));
                    row.Bg.color = sel
                        ? new Color(0.16f, 0.44f, 0.58f, 0.98f)
                        : isQuestSite
                            ? new Color(0.20f, 0.16f, 0.05f, 0.94f)
                            : new Color(0.06f, 0.12f, 0.18f, 0.90f);
                    string questTag = isQuestSite ? "<color=#FFDD44>[!]</color> " : string.Empty;
                    string hereTag = s.Here ? " <color=#66FFAA>[HERE]</color>" : string.Empty;
                    string darkTag = s.Dark ? " <color=#FFAA55>[DARK]</color>" : string.Empty;
                    string pass = !string.IsNullOrEmpty(s.Passage) ? $" · <color=#99CCEE>{MushProtocol.EscapeUnityRichText(s.Passage)}</color>" : string.Empty;
                    row.Label.text = $"{questTag}<color=#FFDD66>[{MushProtocol.EscapeUnityRichText(s.Letter)}]</color> {MushProtocol.EscapeUnityRichText(s.Site)}{hereTag}{darkTag}{pass}";
                    row.Root.SetActive(true);
                }
                else
                {
                    row.BoundVenue = null;
                    row.BoundLetter = null;
                    row.Root.SetActive(false);
                }
            }

            if (_dossierText == null) return;
            var active = CurrentFrame?.FindSiteByVenueOrLetter(!string.IsNullOrEmpty(SelectedSiteVenue) ? SelectedSiteVenue : SelectedSiteLetter);
            if (active == null)
            {
                _dossierText.text = CurrentFrame != null && CurrentFrame.Lines.Count > 0
                    ? MushProtocol.EscapeUnityRichText(string.Join("\n", CurrentFrame.Lines))
                    : "Click <b>REFRESH GROUND</b> to pull the server's live planetary survey wash (`body.ground`).";
                return;
            }

            var sb = new StringBuilder(256);
            sb.AppendFormat("<color=#FFDD66><b>[{0}] {1}</b></color>  <color=#88CCEE>(venue: {2})</color>\n",
                MushProtocol.EscapeUnityRichText(active.Letter),
                MushProtocol.EscapeUnityRichText(active.Site),
                MushProtocol.EscapeUnityRichText(active.Venue));
            sb.AppendFormat(CultureInfo.InvariantCulture, "COORDS: <b>{0:0.0}° lat, {1:0.0}° lon</b>", active.LatMdeg / 1000.0, active.LonMdeg / 1000.0);
            if (!string.IsNullOrEmpty(active.Tenure))
                sb.AppendFormat("   |   TENURE: <b>{0}</b>", MushProtocol.EscapeUnityRichText(active.Tenure));
            sb.AppendLine();
            if (!string.IsNullOrEmpty(active.Survey))
                sb.AppendLine($"SURVEY: <color=#88EEAA>{MushProtocol.EscapeUnityRichText(active.Survey)}</color>");
            if (!string.IsNullOrEmpty(active.Passage))
                sb.AppendLine($"PASSAGE: <color=#99DDFF>{MushProtocol.EscapeUnityRichText(active.Passage)}</color>");
            if (!string.IsNullOrEmpty(active.BestLine))
                sb.AppendLine($"READINGS: {MushProtocol.EscapeUnityRichText(active.BestLine)}");
            if (hasQuestMining && !qMineRow.MineDone &&
                (MatchesQuestSite(active, qMineRow) ||
                 (!string.IsNullOrEmpty(recVenue) && string.Equals(active.Venue, recVenue, StringComparison.OrdinalIgnoreCase))))
            {
                sb.AppendLine($"<color=#FFDD44><b>[QUEST TARGET SITE]</b> Recommended for {MushProtocol.EscapeUnityRichText(qMineRow.TargetOreSlug)} (+ship/plan {MushProtocol.EscapeUnityRichText(active.Venue)})</color>");
            }
            if (hasQuestMining)
                sb.AppendLine($"<color=#FFDD44>[PROSPECTING]</color> <color=#77DDFF>{MushProtocol.EscapeUnityRichText(qMineRow.FormatPrepHintLine())}</color>");
            _dossierText.text = sb.ToString();
        }

        private void RedrawMapTexture()
        {
            if (_mapTex == null || _pixelBuffer == null) return;

            Color32 bg = new Color32(10, 16, 26, 255);
            for (int i = 0; i < _pixelBuffer.Length; i++) _pixelBuffer[i] = bg;

            if (CurrentFrame != null && CurrentFrame.Glyphs != null && CurrentFrame.Width > 0 && CurrentFrame.Height > 0)
            {
                int gw = CurrentFrame.Width;
                int gh = CurrentFrame.Height;
                for (int py = 0; py < TexHeight; py++)
                {
                    int gy = Mathf.Clamp((py * gh) / TexHeight, 0, gh - 1);
                    int rowBase = py * TexWidth;
                    int gRowBase = gy * gw;
                    for (int px = 0; px < TexWidth; px++)
                    {
                        int gx = Mathf.Clamp((px * gw) / TexWidth, 0, gw - 1);
                        _pixelBuffer[rowBase + px] = WashColorForGlyph(CurrentFrame.Glyphs[gRowBase + gx]);
                    }
                }
            }

            // Equirectangular lat/lon graticule lines (equator, ±45°, 60° longitude bands)
            Color32 gridCol = new Color32(50, 96, 124, 255);
            for (int i = 1; i < 4; i++)
            {
                int y = (TexHeight * i) / 4;
                for (int x = 0; x < TexWidth; x += 2) _pixelBuffer[y * TexWidth + x] = gridCol;
            }
            for (int i = 1; i < 6; i++)
            {
                int x = (TexWidth * i) / 6;
                for (int y = 0; y < TexHeight; y += 2) _pixelBuffer[y * TexWidth + x] = gridCol;
            }

            if (CurrentFrame != null)
            {
                // Draw planetary ports as golden squares
                Color32 portCol = new Color32(246, 206, 76, 255);
                for (int i = 0; i < CurrentFrame.Ports.Count; i++)
                {
                    Vector2 uv = GroundChartFrame.ProjectLatLonToUv(CurrentFrame.Ports[i].LatMdeg, CurrentFrame.Ports[i].LonMdeg);
                    int cx = Mathf.Clamp(Mathf.RoundToInt(uv.x * (TexWidth - 1)), 4, TexWidth - 5);
                    int cy = Mathf.Clamp(Mathf.RoundToInt(uv.y * (TexHeight - 1)), 4, TexHeight - 5);
                    DrawFilledBox(cx, cy, 4, portCol);
                }

                // Draw prospecting sites as bright cyan/white discs + quest/selection reticle
                QuestSurfaceMiningStatus qMineMap = default;
                bool hasQuestMining = _questJournal != null && _questJournal.TryGetActiveSurfaceMiningStatus(out qMineMap) && !qMineMap.MineDone;
                string recMapVenue = hasQuestMining && TryGetRecommendedQuestSite(out var recMapSite) ? recMapSite.Venue : string.Empty;
                for (int i = 0; i < CurrentFrame.Sites.Count; i++)
                {
                    var s = CurrentFrame.Sites[i];
                    Vector2 uv = GroundChartFrame.ProjectLatLonToUv(s.LatMdeg, s.LonMdeg);
                    int cx = Mathf.Clamp(Mathf.RoundToInt(uv.x * (TexWidth - 1)), 5, TexWidth - 6);
                    int cy = Mathf.Clamp(Mathf.RoundToInt(uv.y * (TexHeight - 1)), 5, TexHeight - 6);
                    bool sel = (!string.IsNullOrEmpty(SelectedSiteVenue) && string.Equals(s.Venue, SelectedSiteVenue, StringComparison.OrdinalIgnoreCase)) ||
                               (!string.IsNullOrEmpty(SelectedSiteLetter) && string.Equals(s.Letter, SelectedSiteLetter, StringComparison.OrdinalIgnoreCase));
                    bool isQuestSite = hasQuestMining &&
                                       (MatchesQuestSite(s, qMineMap) ||
                                        (!string.IsNullOrEmpty(recMapVenue) && string.Equals(s.Venue, recMapVenue, StringComparison.OrdinalIgnoreCase)));

                    Color32 siteCol = s.Here
                        ? new Color32(96, 255, 170, 255)
                        : isQuestSite
                            ? new Color32(255, 224, 96, 255)
                            : new Color32(146, 224, 255, 255);
                    DrawFilledBox(cx, cy, s.Here ? 4 : 3, siteCol);
                    if (isQuestSite && !sel)
                        DrawHollowRing(cx, cy, 6, new Color32(255, 210, 72, 255));
                    if (sel)
                        DrawHollowRing(cx, cy, 8, new Color32(255, 220, 90, 255));
                }
            }

            _mapTex.SetPixels32(_pixelBuffer);
            _mapTex.Apply(false);
        }

        private void DrawFilledBox(int cx, int cy, int halfSize, Color32 col)
        {
            for (int dy = -halfSize; dy <= halfSize; dy++)
            {
                int y = cy + dy;
                if (y < 0 || y >= TexHeight) continue;
                int row = y * TexWidth;
                for (int dx = -halfSize; dx <= halfSize; dx++)
                {
                    int x = cx + dx;
                    if (x >= 0 && x < TexWidth)
                        _pixelBuffer[row + x] = col;
                }
            }
        }

        private void DrawHollowRing(int cx, int cy, int radius, Color32 col)
        {
            int rMin = (radius - 1) * (radius - 1);
            int rMax = (radius + 1) * (radius + 1);
            for (int dy = -radius - 1; dy <= radius + 1; dy++)
            {
                int y = cy + dy;
                if (y < 0 || y >= TexHeight) continue;
                int row = y * TexWidth;
                for (int dx = -radius - 1; dx <= radius + 1; dx++)
                {
                    int x = cx + dx;
                    if (x < 0 || x >= TexWidth) continue;
                    int d2 = dx * dx + dy * dy;
                    if (d2 >= rMin && d2 <= rMax)
                        _pixelBuffer[row + x] = col;
                }
            }
        }

        private GameObject CreatePanel(Transform parent, string name, Vector2 aMin, Vector2 aMax, Vector2 oMin, Vector2 oMax, Color bg)
        {
            var go = new GameObject(name, typeof(RectTransform), typeof(Image));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = bg.a > 0.01f;
            SetRect(go.GetComponent<RectTransform>(), aMin, aMax, oMin, oMax);
            return go;
        }

        private Text CreateText(Transform parent, string content, int size, FontStyle style, TextAnchor align, Color color)
        {
            var go = new GameObject("Text", typeof(RectTransform), typeof(Text));
            go.transform.SetParent(parent, false);
            var t = go.GetComponent<Text>();
            t.font = _font;
            t.text = content;
            t.fontSize = size;
            t.fontStyle = style;
            t.alignment = align;
            t.color = color;
            t.supportRichText = false;
            t.raycastTarget = false;
            return t;
        }

        private Button CreateButton(Transform parent, string label, Color bg, Action onClick)
        {
            var go = new GameObject("Btn_" + label, typeof(RectTransform), typeof(Image), typeof(Button));
            go.transform.SetParent(parent, false);
            var img = go.GetComponent<Image>();
            img.color = bg;
            img.raycastTarget = true;
            var btn = go.GetComponent<Button>();
            btn.targetGraphic = img;
            if (onClick != null) btn.onClick.AddListener(() => onClick.Invoke());
            var txt = CreateText(go.transform, label, 11, FontStyle.Bold, TextAnchor.MiddleCenter, Color.white);
            SetRect(txt.rectTransform, Vector2.zero, Vector2.one, new Vector2(4f, 2f), new Vector2(-4f, -2f));
            return btn;
        }

        private static void SetRect(RectTransform rt, Vector2 anchorMin, Vector2 anchorMax, Vector2 offsetMin, Vector2 offsetMax)
        {
            rt.anchorMin = anchorMin;
            rt.anchorMax = anchorMax;
            rt.offsetMin = offsetMin;
            rt.offsetMax = offsetMax;
        }
    }
}
