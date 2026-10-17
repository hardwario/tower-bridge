<script lang="ts">
  import { api } from "../lib/api";
  import { i18n, t } from "../lib/i18n/index.svelte";
  import { live } from "../lib/live.svelte";
  import { renderManual } from "../lib/manual";
  import { theme } from "../lib/theme.svelte";

  /** Pages outside the app that the manual may link to, by their key for `open_link`. */
  const LINKS: Record<string, "docs" | "store" | "academy"> = {
    "https://docs.hardwario.com/tower/": "docs",
    "https://www.hardwario.store/": "store",
    "https://hardwario.academy/": "academy",
  };

  const manual = $derived(renderManual(i18n.lang, t(), live.app?.version ?? "", theme.effective === "dark"));

  let article = $state<HTMLElement>();
  let active = $state("");
  /** Screenshot shown enlarged over the page. */
  let zoomed = $state<{ src: string; alt: string } | null>(null);

  let headings: HTMLElement[] = [];
  let root: HTMLElement | null = null;
  /**
   * While a click scrolls to a chapter, the contents keep showing that
   * chapter instead of sweeping through every one passed on the way.
   */
  let jumping = false;
  let settle: ReturnType<typeof setTimeout> | undefined;

  /** Highlight the section being read in the table of contents. */
  function spy() {
    const top = (root?.getBoundingClientRect().top ?? 0) + 80;
    let current = headings[0]?.id ?? "";
    for (const h of headings) if (h.getBoundingClientRect().top <= top) current = h.id;
    active = current;
  }

  function onScroll() {
    if (!jumping) return spy();
    // The jump is over once the scrolling stops.
    clearTimeout(settle);
    settle = setTimeout(() => (jumping = false), 150);
  }

  async function show(id: string) {
    const target = article?.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
    if (!target) return;
    active = id;
    jumping = true;
    clearTimeout(settle);
    // Screenshots still loading above the chapter would push it down mid-scroll.
    const pending = [...(article?.querySelectorAll("img") ?? [])].filter((img) => !img.complete);
    const wait = new Promise((resolve) => setTimeout(resolve, 1000));
    await Promise.race([Promise.all(pending.map((img) => img.decode().catch(() => {}))), wait]);
    target.scrollIntoView({ behavior: "smooth", block: "start" });
    // Also when there is nothing to scroll (already there).
    settle = setTimeout(() => (jumping = false), 400);
  }

  /** Keep the webview on the manual: in-page anchors scroll, known pages open in the browser. */
  function onclick(e: MouseEvent) {
    const target = e.target as Element;
    if (target instanceof HTMLImageElement && target.closest(".screenshot")) {
      zoomed = { src: target.currentSrc || target.src, alt: target.alt };
      return;
    }
    const link = target.closest("a");
    if (!link) return;
    e.preventDefault();
    const href = link.getAttribute("href") ?? "";
    if (href.startsWith("#")) show(href.slice(1));
    else if (LINKS[href]) api.openLink(LINKS[href]);
  }

  $effect(() => {
    if (!article) return;
    void manual;
    const main = article.closest("main");
    root = main;
    headings = [...article.querySelectorAll<HTMLElement>("h2[id], h3[id]")];
    spy();
    main?.addEventListener("scroll", onScroll, { passive: true });
    return () => main?.removeEventListener("scroll", onScroll);
  });
</script>

<div class="view manual">
  <div class="page-head">
    <h1>{manual.title}</h1>
  </div>

  <div class="layout">
    <nav class="toc" aria-label={t().manual.contents}>
      <div class="toc-title">{t().manual.contents}</div>
      {#each manual.sections as s (s.id)}
        <a
          href="#{s.id}"
          class:sub={s.level > 2}
          class:active={active === s.id}
          aria-current={active === s.id ? "location" : undefined}
          onclick={(e) => {
            e.preventDefault();
            show(s.id);
          }}>{s.title}</a
        >
      {/each}
    </nav>

    <!-- The manual is bundled with the app from docs/manual, never fetched. -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
    <article class="card prose" bind:this={article} {onclick}>
      {@html manual.html}
    </article>
  </div>
</div>

{#if zoomed}
  <button class="lightbox" aria-label={t().manual.close} onclick={() => (zoomed = null)}>
    <img src={zoomed.src} alt={zoomed.alt} />
  </button>
{/if}

<svelte:window onkeydown={(e) => e.key === "Escape" && (zoomed = null)} />

<style>
  .layout {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: 20px;
    align-items: start;
  }
  .toc {
    position: sticky;
    top: 18px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    font-size: 13px;
  }
  .toc-title {
    padding: 0 10px 6px;
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .toc a {
    padding: 5px 10px;
    border-radius: var(--radius-sm);
    color: var(--text-2);
    text-decoration: none;
    line-height: 1.3;
  }
  .toc a.sub {
    padding-left: 22px;
    font-size: 12.5px;
  }
  .toc a:hover {
    background: var(--accent-weak);
    color: var(--text);
  }
  /* Colour and a bar, never a heavier weight: bolder text is wider, so the
     menu would reflow and flicker while the active chapter changes. */
  .toc a.active {
    color: var(--text);
    background: var(--surface-2);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .prose {
    padding: 8px 28px 24px;
    line-height: 1.6;
    font-size: 14px;
  }
  .prose :global(h2) {
    margin: 28px 0 8px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
    font-size: 18px;
    letter-spacing: -0.01em;
    scroll-margin-top: 16px;
  }
  .prose :global(h2:first-child) {
    border-top: none;
    margin-top: 12px;
  }
  .prose :global(h3) {
    margin: 22px 0 6px;
    font-size: 15px;
    scroll-margin-top: 16px;
  }
  .prose :global(p),
  .prose :global(ul) {
    margin: 0 0 10px;
    color: var(--text-2);
  }
  .prose :global(ul) {
    padding-left: 20px;
  }
  .prose :global(li) {
    margin: 3px 0;
  }
  .prose :global(strong) {
    color: var(--text);
    font-weight: 600;
  }
  .prose :global(code) {
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    font-family: var(--mono);
    font-size: 12.5px;
  }
  .prose :global(figure) {
    margin: 14px 0 16px;
  }
  .prose :global(.diagram svg) {
    display: block;
    width: 100%;
    max-width: 680px;
    height: auto;
  }
  .prose :global(.screenshot img) {
    display: block;
    cursor: zoom-in;
    max-width: 100%;
    height: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: 0 2px 10px rgba(20, 20, 30, 0.06);
  }
  .prose :global(table) {
    margin: 4px 0 14px;
    border-collapse: collapse;
    font-size: 13.5px;
  }
  .prose :global(th),
  .prose :global(td) {
    padding: 7px 14px 7px 0;
    border-bottom: 1px solid var(--border);
    text-align: left;
    vertical-align: top;
    color: var(--text-2);
  }
  .prose :global(th) {
    color: var(--text);
    font-weight: 600;
  }
  .prose :global(td:first-child) {
    color: var(--text);
    font-weight: 500;
    white-space: nowrap;
  }
  .prose :global(kbd) {
    display: inline-block;
    min-width: 20px;
    padding: 0 6px;
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 5px;
    background: var(--surface-2);
    font-family: var(--font);
    font-size: 12px;
    line-height: 18px;
    text-align: center;
    color: var(--text);
  }
  .prose :global(ol) {
    margin: 0 0 10px;
    padding-left: 22px;
    color: var(--text-2);
  }
  .prose :global(a) {
    color: var(--accent);
    text-decoration: none;
  }
  .prose :global(a:hover) {
    text-decoration: underline;
  }
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 32px;
    border: none;
    background: rgba(10, 10, 16, 0.72);
    cursor: zoom-out;
  }
  .lightbox img {
    max-width: 100%;
    max-height: 100%;
    border-radius: var(--radius-sm);
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.4);
  }
  @media (max-width: 760px) {
    .layout {
      grid-template-columns: 1fr;
    }
    .toc {
      position: static;
    }
  }
</style>
