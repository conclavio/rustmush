// @vitest-environment jsdom
//
// The unit screen and its widgets, mounted (doc 43 cut TS.19). These replace
// the source probes `stores/unit.test.ts` carried for the Svelte halves its
// node environment could not render: each case below drives the component
// with the input the probe's comment was about and reads the DOM, so a
// rewrite that keeps the behaviour passes and one that loses it fails,
// whatever the markup is spelled like.

import { fireEvent, render, screen, within } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import GenericView from "./GenericView.svelte";
import UnitScreen from "./UnitScreen.svelte";
import KeyValue from "./widgets/KeyValue.svelte";
import Table from "./widgets/Table.svelte";
import type { GameApp } from "$lib/game.svelte";
import { parseGenericView } from "$lib/genericview";
import { journalRows, NO_UNIT_LINE, UNIT_PANE_TITLES, UnitStore } from "$lib/stores/unit.svelte";

/** A journal delta for `#7`, the contract fixture F3's shape. */
const DELTA = {
  object: { id: "#7", name: "Atlas" },
  from_revision: 3,
  to_revision: 4,
  now: 1757000000,
  cause: "gauss",
  entries: [
    { kind: "cell", addr: "facet/center_torso/rear/armor", from: 6, to: 1 },
    { kind: "section", section: "center_torso", from: "ok", to: "damaged" },
  ],
};

/** The screen's whole reach: the unit store and the contacts' generic view. */
function mountUnit(set: (unit: UnitStore) => void): void {
  const unit = new UnitStore();
  set(unit);
  const app = { unit, genericViews: { get: () => null } } as unknown as GameApp;
  render(UnitScreen, { props: { app } });
}

describe("KeyValue and Table take what comes off the wire", () => {
  it("KeyValue renders every row when producers repeat an id", () => {
    // Svelte throws `each_key_duplicate` out of the whole flush on a repeated
    // key; these keys are producer-chosen ids, and a repeat is legal upstream.
    render(KeyValue, {
      props: {
        rows: [
          { key: "tactics.mobility", value: "granted" },
          { key: "tactics.mobility", value: "granted twice" },
        ],
      },
    });
    expect(screen.getAllByText("tactics.mobility")).toHaveLength(2);
    expect(screen.getByText("granted twice")).toBeTruthy();
  });

  it("Table renders repeated rows and repeated cells", () => {
    render(Table, {
      props: {
        columns: [
          { key: "a", label: "a" },
          { key: "b", label: "b" },
        ],
        rows: [
          { a: "same", b: "same" },
          { a: "same", b: "same" },
        ],
      },
    });
    const body = screen.getAllByRole("rowgroup")[1]!;
    expect(within(body).getAllByRole("row")).toHaveLength(2);
    expect(within(body).getAllByText("same")).toHaveLength(4);
  });
});

describe("UnitScreen branches as the store answers", () => {
  it("says the refusal first, whatever else the store holds", () => {
    mountUnit((u) => {
      u.unavailable = "Unit view refused.";
      u.answered = true;
      u.journal = journalRows(DELTA)!;
    });
    expect(screen.getByText("Unit view refused.")).toBeTruthy();
    expect(screen.queryByText("Loading…")).toBeNull();
    expect(screen.queryByText(NO_UNIT_LINE)).toBeNull();
  });

  it("is loading until the server answers", () => {
    mountUnit(() => {});
    expect(screen.getByText("Loading…")).toBeTruthy();
    expect(screen.queryByText(NO_UNIT_LINE)).toBeNull();
  });

  it("says there is no subject once answered without one", () => {
    mountUnit((u) => {
      u.answered = true;
    });
    expect(screen.getByText(NO_UNIT_LINE)).toBeTruthy();
    expect(screen.queryByRole("region")).toBeNull();
  });

  it("renders the ring under the no-subject line", () => {
    // The one actor who can produce an after-action report is a staffer,
    // who often holds no station at all.
    mountUnit((u) => {
      u.answered = true;
      u.journal = journalRows(DELTA)!;
    });
    expect(screen.getByText(NO_UNIT_LINE)).toBeTruthy();
    const after = screen.getByRole("region", { name: "afteraction" });
    const journal = screen.getByRole("region", { name: "journal" });
    expect(within(after).getByText(UNIT_PANE_TITLES.afteraction!)).toBeTruthy();
    expect(within(after).getByText("facet/center_torso/rear/armor")).toBeTruthy();
    expect(within(journal).getAllByText("gauss").length).toBeGreaterThan(0);
  });
});

describe("GenericView rows are reachable from the keyboard", () => {
  const view = parseGenericView({
    schema: {
      id: "jobs",
      version: 1,
      shape: "list",
      columns: [{ key: "title", label: "title" }],
      actionKey: "_open",
    },
    rows: [{ title: "Open one", _open: "@job 3" }, { title: "Read only" }],
  })!;

  it("an actionable row is a focusable button that prefills on Enter and Space", async () => {
    const onPrefill = vi.fn();
    render(GenericView, { props: { view, onPrefill } });
    const row = screen.getByRole("button", { name: /Open one/ });
    expect(row.tabIndex).toBe(0);
    await fireEvent.keyDown(row, { key: "Enter" });
    await fireEvent.keyDown(row, { key: " " });
    await fireEvent.keyDown(row, { key: "x" });
    expect(onPrefill.mock.calls).toEqual([["@job 3"], ["@job 3"]]);
  });

  it("a row with no action is neither a button nor in the tab order", () => {
    render(GenericView, { props: { view, onPrefill: vi.fn() } });
    const row = screen.getByText("Read only").closest("tr")!;
    expect(row.getAttribute("role")).toBeNull();
    expect(row.hasAttribute("tabindex")).toBe(false);
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });
});
