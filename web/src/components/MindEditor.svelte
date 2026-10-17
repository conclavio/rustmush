<script lang="ts">
  import { onMount } from "svelte";
  import { basicSetup } from "codemirror";
  import { Compartment, EditorState } from "@codemirror/state";
  import { EditorView } from "@codemirror/view";
  import { setDiagnostics, type Diagnostic } from "@codemirror/lint";
  import { rhaiLanguage } from "$lib/rhai-language";

  interface SourceDiagnostic { message: string; line: number | null; column: number | null }
  let { value, disabled = false, diagnostics = [], onchange }:
    { value: string; disabled?: boolean; diagnostics?: SourceDiagnostic[]; onchange: (source: string) => void } = $props();
  let host: HTMLDivElement;
  let view: EditorView | undefined = $state();
  const editability = new Compartment();
  let error = $state("");
  onMount(() => {
    const editor = new EditorView({ parent: host, doc: value, extensions: [
      basicSetup, rhaiLanguage, EditorView.lineWrapping,
      editability.of(EditorState.readOnly.of(disabled)),
      EditorView.contentAttributes.of({ "aria-label": "Rhai mind source" }),
      EditorState.transactionFilter.of(transaction => {
        if (!transaction.docChanged) return transaction;
        const source = transaction.newDoc.toString();
        if (new TextEncoder().encode(source).length > 32 * 1024 || source.includes("\0")) {
          error = "Source must fit 32 KiB and contain no NUL characters.";
          return [];
        }
        error = "";
        return transaction;
      }),
      EditorView.updateListener.of(update => { if (update.docChanged) onchange(update.state.doc.toString()); }),
      EditorView.theme({ "&": { border: "1px solid #666", minHeight: "18rem", maxHeight: "55vh" },
        ".cm-scroller": { overflow: "auto", fontFamily: "monospace" },
        ".cm-content": { whiteSpace: "pre-wrap" } }),
    ] });
    view = editor;
    return () => { editor.destroy(); view = undefined; };
  });
  $effect(() => {
    if (view && view.state.doc.toString() !== value) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: value } });
    }
  });
  $effect(() => { view?.dispatch({ effects: editability.reconfigure(EditorState.readOnly.of(disabled)) }); });
  $effect(() => {
    if (!view) return;
    const placed: Diagnostic[] = [];
    for (const diagnostic of diagnostics.slice(0, 64)) {
      if (diagnostic.line === null || diagnostic.line < 1 || diagnostic.line > view.state.doc.lines) continue;
      const line = view.state.doc.line(diagnostic.line);
      // Rhai columns count Unicode scalars; CodeMirror offsets count UTF-16 units.
      const offset = [...line.text].slice(0, Math.max(0, (diagnostic.column ?? 1) - 1)).join("").length;
      const from = Math.min(line.to, line.from + offset);
      placed.push({ from, to: Math.min(line.to, from + 1), severity: "error", message: diagnostic.message });
    }
    view.dispatch(setDiagnostics(view.state, placed));
  });
</script>

<div bind:this={host}></div>
{#if error}<p role="alert">{error}</p>{/if}
