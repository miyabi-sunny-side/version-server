import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import Home from "./Home.svelte";

const RELEASES = [
  {
    repo: "miyabi-sunny-side/task-server",
    tag: "v0.3.12",
    published_at: "2026-09-03T11:34:00Z",
    assets: [],
    source: "webhook",
    received_at: "2026-09-03T11:35:02Z",
  },
  {
    repo: "miyabi-sunny-side/task-worker",
    tag: "v0.1.7",
    published_at: null,
    assets: [],
    source: "poll",
    received_at: "2026-09-03T13:46:31Z",
  },
];

function jsonResponse(payload: unknown): Response {
  return new Response(JSON.stringify(payload), { status: 200 });
}

function listContainer(): HTMLElement {
  const container = document.querySelector<HTMLElement>("[data-state]");
  if (!container) {
    throw new Error("list container with data-state was not found");
  }
  return container;
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("Home", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("lists each repo's latest tag, source and received time", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(RELEASES));
    vi.stubGlobal("fetch", fetchMock);

    render(Home);
    expect(listContainer().dataset.state).toBe("loading");

    await waitFor(() => expect(listContainer().dataset.state).toBe("success"));
    const rows = screen.getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain("miyabi-sunny-side/task-server");
    expect(rows[0].textContent).toContain("v0.3.12");
    expect(rows[0].textContent).toContain("webhook");
    expect(rows[0].textContent).toContain("2026-09-03T11:35:02Z");
    expect(rows[1].textContent).toContain("poll");
    expect(fetchMock).toHaveBeenCalledWith(
      "/v1/versions",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });

  it("reloads when the tab becomes visible again and on the interval", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(RELEASES));
    vi.stubGlobal("fetch", fetchMock);

    render(Home);
    await waitFor(() => expect(listContainer().dataset.state).toBe("success"));
    expect(fetchMock).toHaveBeenCalledTimes(1);

    setVisibility("hidden");
    vi.advanceTimersByTime(10_000);
    expect(fetchMock).toHaveBeenCalledTimes(1);

    setVisibility("visible");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));

    vi.advanceTimersByTime(10_000);
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
  });

  it("keeps the drawn list when a background reload fails", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(RELEASES))
      .mockRejectedValueOnce(new Error("offline"));
    vi.stubGlobal("fetch", fetchMock);

    render(Home);
    await waitFor(() => expect(listContainer().dataset.state).toBe("success"));

    setVisibility("hidden");
    setVisibility("visible");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(listContainer().dataset.state).toBe("success");
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("shows the empty state before anything was received", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockResolvedValue(jsonResponse([])),
    );

    render(Home);

    await waitFor(() => expect(listContainer().dataset.state).toBe("empty"));
  });

  it("shows the error state and recovers through the retry button", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(jsonResponse(RELEASES));
    vi.stubGlobal("fetch", fetchMock);

    render(Home);
    await waitFor(() => expect(listContainer().dataset.state).toBe("error"));

    await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
    await waitFor(() => expect(listContainer().dataset.state).toBe("success"));
  });
});
