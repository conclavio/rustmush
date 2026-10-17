import { StreamLanguage, type StreamParser } from "@codemirror/language";

interface RhaiState { comment: number; quote: string | null; rawHashes: number | null }
const keywords = new Set("fn private let const if else switch case default for in while loop do until break continue return throw try catch import export as share".split(" "));
const atoms = new Set(["true", "false", "this"]);

/** Rhai lexical highlighting only; syntax/meaning remains the server compiler's. */
export const rhaiParser: StreamParser<RhaiState> = {
  startState: () => ({ comment: 0, quote: null, rawHashes: null }),
  copyState: state => ({ ...state }),
  token(stream, state) {
    if (state.comment) {
      while (!stream.eol()) {
        if (stream.match("/*")) state.comment++;
        else if (stream.match("*/")) { if (--state.comment === 0) break; }
        else stream.next();
      }
      return "comment";
    }
    if (state.rawHashes !== null) {
      const end = `"${"#".repeat(state.rawHashes)}`;
      while (!stream.eol()) {
        if (stream.match(end)) { state.rawHashes = null; break; }
        stream.next();
      }
      return "string";
    }
    if (state.quote !== null) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === "\\") stream.next();
        else if (ch === state.quote) { state.quote = null; break; }
      }
      // Backticks span lines; ordinary unterminated strings are compiler errors.
      if (state.quote !== "`") state.quote = null;
      return "string";
    }
    if (stream.eatSpace()) return null;
    if (stream.match("//")) { stream.skipToEnd(); return "comment"; }
    if (stream.match("/*")) { state.comment = 1; return "comment"; }
    const raw = stream.match(/^r(#+)?"/);
    if (raw && typeof raw !== "boolean") { state.rawHashes = raw[1]?.length ?? 0; return "string"; }
    const ch = stream.peek();
    if (ch === '"' || ch === "'" || ch === "`") { state.quote = ch; stream.next(); return "string"; }
    if (stream.match(/^(?:0x[\da-fA-F_]+|0b[01_]+|0o[0-7_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d[\d_]*)?)/)) return "number";
    if (stream.match(/^[A-Za-z_][A-Za-z_\d]*/)) {
      const word = stream.current();
      return keywords.has(word) ? "keyword" : atoms.has(word) ? "atom" : "variableName";
    }
    stream.next();
    return ch && /[+\-*\/%=!<>&|^~?:]/.test(ch) ? "operator" : "punctuation";
  },
  languageData: { commentTokens: { line: "//", block: { open: "/*", close: "*/" } } },
};
export const rhaiLanguage = StreamLanguage.define(rhaiParser);
