// The web client's one Markdown renderer (markdown-it), for documents the
// server or the bundle ships: the playtest guide on the login page today,
// and help topics later.
//
// Safety: `html: false` escapes any raw HTML in the source, and markdown-it's
// own link check refuses `javascript:`, `vbscript:`, `file:` and non-image
// `data:` targets. So the output is safe to place with `{@html}`.

import MarkdownIt from "markdown-it";

/** A stable id for a heading, for in-page links. */
export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");
}

function build(): MarkdownIt {
  const md = new MarkdownIt({ html: false, linkify: false, typographer: false });

  // Heading ids from the heading's plain text, made unique per document.
  md.core.ruler.push("heading_ids", (state) => {
    const seen = new Map<string, number>();
    const tokens = state.tokens;
    for (let i = 0; i < tokens.length; i += 1) {
      const open = tokens[i];
      if (open?.type !== "heading_open") continue;
      const text = (tokens[i + 1]?.children ?? [])
        .filter((t) => t.type === "text" || t.type === "code_inline")
        .map((t) => t.content)
        .join("");
      const base = slugify(text) || "section";
      const n = seen.get(base) ?? 0;
      seen.set(base, n + 1);
      open.attrSet("id", n === 0 ? base : `${base}-${n}`);
    }
  });

  // External links open in a new tab, without an opener.
  const defaultLinkOpen =
    md.renderer.rules.link_open ??
    ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
  md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
    const token = tokens[idx];
    if (token && /^https?:\/\//i.test(token.attrGet("href") ?? "")) {
      token.attrSet("target", "_blank");
      token.attrSet("rel", "noopener noreferrer");
    }
    return defaultLinkOpen(tokens, idx, options, env, self);
  };

  // Tables scroll sideways inside a wrapper on a narrow screen.
  md.renderer.rules.table_open = () => '<div class="table-wrap"><table>\n';
  md.renderer.rules.table_close = () => "</table></div>\n";

  return md;
}

const renderer = build();

/** Renders a Markdown document to HTML that is safe for `{@html}`. */
export function renderMarkdown(source: string): string {
  return renderer.render(source);
}
