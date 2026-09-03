<script lang="ts">
  import { fetchVersions, type Release } from "../lib/api";

  type ListState = "loading" | "empty" | "error" | "success";

  // How often the list re-reads the server while the tab is visible. The
  // server is on the LAN and the list is small, so a short period is fine.
  export const RELOAD_MS = 10_000;

  let releases = $state<Release[]>([]);
  let listState = $state<ListState>("loading");

  let controller: AbortController | undefined;
  let loadedOnce = false;

  async function load() {
    controller?.abort();
    controller = new AbortController();
    if (!loadedOnce) {
      listState = "loading";
    }
    try {
      releases = await fetchVersions(controller.signal);
      listState = releases.length === 0 ? "empty" : "success";
      loadedOnce = true;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      // A background reload that fails leaves the drawn list alone.
      if (!loadedOnce) {
        listState = "error";
      }
    }
  }

  function onVisibilityChange() {
    if (document.visibilityState === "visible") {
      void load();
    }
  }

  $effect(() => {
    void load();
    document.addEventListener("visibilitychange", onVisibilityChange);
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") {
        void load();
      }
    }, RELOAD_MS);
    return () => {
      controller?.abort();
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  });
</script>

<section class="content" data-state={listState}>
  {#if listState === "loading"}
    <p class="state">
      <span class="spinner" aria-hidden="true"></span>読み込み中…
    </p>
  {:else if listState === "empty"}
    <p class="state">まだ release を受信していません</p>
  {:else if listState === "error"}
    <div class="state-wrap">
      <p class="state error">読み込みに失敗しました</p>
      <button class="btn" type="button" onclick={() => void load()}>
        再試行
      </button>
    </div>
  {:else}
    <ul class="cards">
      {#each releases as release (release.repo)}
        <li class="card">
          <span class="repo">{release.repo}</span>
          <span class="tail">
            <span class="badge">{release.tag}</span>
            <span class="badge muted">{release.source}</span>
            <time class="received" datetime={release.received_at}>
              {release.received_at}
            </time>
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style lang="sass">
  .cards
    display: flex
    flex-direction: column
    gap: var(--sp-2)
    margin: 0
    padding: 0
    list-style: none

  .card
    display: flex
    flex-wrap: wrap
    align-items: baseline
    justify-content: space-between
    gap: var(--sp-2)
    padding: 10px
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    color: var(--c-on-surface)

  .repo
    font-size: var(--fs-md)
    font-weight: 500
    overflow-wrap: anywhere

  .tail
    display: flex
    flex-wrap: wrap
    align-items: baseline
    gap: var(--sp-2)

  .badge
    padding: 1px 8px
    border: 1px solid var(--c-border)
    border-radius: 9999px
    font-size: var(--fs-xs)
    color: var(--c-on-surface)

  .badge.muted
    color: var(--c-muted)

  .received
    font-size: var(--fs-xs)
    color: var(--c-muted)
</style>
