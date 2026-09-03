import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "./App.svelte";

describe("App", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("draws the header and the version list on the one page", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockResolvedValue(
        new Response(
          JSON.stringify([
            {
              repo: "o/r",
              tag: "v1.0.0",
              published_at: null,
              assets: [],
              source: "poll",
              received_at: "2026-09-03T00:00:00Z",
            },
          ]),
          { status: 200 },
        ),
      ),
    );

    render(App);

    expect(screen.getByRole("link", { name: "version-server" })).toBeTruthy();
    await waitFor(() =>
      expect(screen.getByRole("listitem").textContent).toContain("v1.0.0"),
    );
  });
});
