import { describe, expect, it } from "vitest";

import guideSource from "../../../docs/playthrough.md?raw";

import { renderMarkdown, slugify } from "./markdown";

describe("markdown", () => {
  it("escapes raw HTML in the source and inside code", () => {
    expect(renderMarkdown("<script>alert(1)</script>")).not.toContain("<script");
    expect(renderMarkdown("```\n> <b>\n```")).toContain("&gt; &lt;b&gt;");
  });

  it("refuses a javascript: link target", () => {
    const html = renderMarkdown("[click](javascript:alert(1))");
    // Left as plain text: no link is made.
    expect(html).not.toContain("<a ");
    expect(html).not.toContain("href");
  });

  it("opens external links in a new tab without an opener, and keeps fragments in place", () => {
    expect(renderMarkdown("[x](https://e.test/a)")).toContain(
      '<a href="https://e.test/a" target="_blank" rel="noopener noreferrer">x</a>',
    );
    expect(renderMarkdown("[help](#help)")).toContain('<a href="#help">help</a>');
  });

  it("gives headings unique ids", () => {
    const html = renderMarkdown("## Part 1: Connect\n\n## Part 1: Connect\n\n## If `x` fails");
    expect(html).toContain('<h2 id="part-1-connect">');
    expect(html).toContain('<h2 id="part-1-connect-1">');
    expect(html).toContain('<h2 id="if-x-fails">');
    expect(slugify("Before you start")).toBe("before-you-start");
  });

  it("wraps tables so they can scroll", () => {
    const html = renderMarkdown("| A | B |\n|---|---|\n| 1 | `2` |");
    expect(html).toContain('<div class="table-wrap"><table>');
    expect(html).toContain("<td><code>2</code></td>");
  });

  it("keeps a fenced block inside a numbered step", () => {
    const html = renderMarkdown("1. Step one.\n\n   ```\n   > look\n   ```\n\n2. Step two.");
    expect(html).toMatch(/<li>\s*<p>Step one\.<\/p>\s*<pre><code>&gt; look\n<\/code><\/pre>/);
    expect(html.match(/<ol>/g)?.length).toBe(1);
  });

  it("renders the bundled playtest guide with no Markdown left over", () => {
    const html = renderMarkdown(guideSource);
    expect(html).toContain("<h1");
    expect(html).toContain("<table>");
    expect(html).not.toContain("```");
    expect(html).not.toMatch(/\|---/);
  });
});
