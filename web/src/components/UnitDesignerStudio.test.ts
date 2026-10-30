// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, it } from "vitest";
import type { GameApp } from "$lib/game.svelte";
import UnitDesignerStudio from "./UnitDesignerStudio.svelte";

/** A connection whose `unit_design` check answers `answer`. */
function fakeApp(answer: unknown): { app: GameApp; sent: unknown[] } {
  const sent: unknown[] = [];
  let handler: ((e: unknown) => void) | null = null;
  const client = {
    on: (_t: string, fn: (e: unknown) => void) => {
      handler = fn;
      return () => {
        handler = null;
      };
    },
    invoke: (request: unknown) => {
      sent.push(request);
      queueMicrotask(() => handler?.({ d: { id: 1, result: { service: "unit_design", result: answer } } }));
      return 1;
    },
  };
  return { app: { client } as unknown as GameApp, sent };
}

describe("UnitDesignerStudio (doc 31 cut UD.2)", () => {
  it("opens on a template draft with its four panes", () => {
    localStorage.clear();
    const { app } = fakeApp(null);
    render(UnitDesignerStudio, { props: { app } });
    for (const pane of ["Chassis and model", "Rules", "Look", "Compare"]) {
      expect(screen.getByRole("region", { name: pane })).toBeTruthy();
    }
    expect((screen.getByLabelText("Unit id") as HTMLInputElement).value).toBe("new_design");
    // The draft's id is in the published-design namespace (D-31-9).
    expect(screen.getByRole("button", { name: "Export" }).hasAttribute("disabled")).toBe(true);
  });

  it("sends the draft to the server and shows its refusal", async () => {
    localStorage.clear();
    const { app, sent } = fakeApp({ status: "checked", refusals: ["a unit has at most 16 sections"], pack: null });
    render(UnitDesignerStudio, { props: { app } });
    await fireEvent.click(screen.getByRole("button", { name: "Check on server" }));
    await tick();
    await new Promise((r) => setTimeout(r, 0));
    await tick();
    expect(sent).toHaveLength(1);
    expect((sent[0] as { service: string }).service).toBe("unit_design");
    expect(screen.getByText("rules: a unit has at most 16 sections")).toBeTruthy();
  });

  it("publishes a checked design and says when it is fieldable (doc 31 cut UD.3)", async () => {
    localStorage.clear();
    const answers: unknown[] = [
      { status: "checked", refusals: [], pack: { manifest: { id: "tacsim.pack.new_design" }, entries: {} } },
      { status: "published", id: "tacsim.unit.new_design", version: 1, refusals: [] },
    ];
    const sent: unknown[] = [];
    let handler: ((e: unknown) => void) | null = null;
    let next = 1;
    const client = {
      on: (_t: string, fn: (e: unknown) => void) => {
        handler = fn;
        return () => {
          handler = null;
        };
      },
      invoke: (request: unknown) => {
        sent.push(request);
        const id = next++;
        const answer = answers.shift();
        queueMicrotask(() => handler?.({ d: { id, result: { service: "unit_design", result: answer } } }));
        return id;
      },
    };
    render(UnitDesignerStudio, { props: { app: { client } as unknown as GameApp } });
    const settle = async () => {
      await tick();
      await new Promise((r) => setTimeout(r, 0));
      await tick();
    };
    await fireEvent.click(screen.getByRole("button", { name: "Check on server" }));
    await settle();
    const publishButton = screen.getByRole("button", { name: "Publish" });
    // No picture module under jsdom, so the look check is clean and Publish opens.
    expect(publishButton.hasAttribute("disabled")).toBe(false);
    await fireEvent.click(publishButton);
    await settle();
    expect((sent[1] as { request: { op: string; look: unknown } }).request.op).toBe("publish");
    expect((sent[1] as { request: { look: unknown } }).request.look).toBeTruthy();
    expect(screen.getByText(/fieldable after the next restart/)).toBeTruthy();
  });

  it("shows the test hit for a clicked doll part", async () => {
    localStorage.clear();
    const { app } = fakeApp(null);
    render(UnitDesignerStudio, { props: { app } });
    const part = screen.getAllByTitle("turret")[0]!;
    await fireEvent.click(part);
    expect(screen.getByText("turret destroyed:")).toBeTruthy();
  });
});
