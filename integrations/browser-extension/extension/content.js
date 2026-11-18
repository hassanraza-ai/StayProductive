// In-page enforcement for blocks the pre-request layer cannot express.
//
// The content script asks the daemon (through the service worker) on every
// navigation and same-document URL change. When the daemon blocks, it covers
// the page with a closed shadow root so page styles cannot hide it. The URL is
// left untouched: the address bar still shows the page and the user can edit
// or leave it.
//
// This layer is best effort. A page that removes the overlay before it is
// rendered, a browser where the extension is not installed, and non-browser
// clients are out of scope; see the integration README.

(function () {
  const api = globalThis.browser ?? globalThis.chrome;
  const { describe } = globalThis.StayProductiveBlockPage;
  if (!api?.runtime?.sendMessage || !describe) return;

  const POLL_MS = 750;
  let host = null;
  let mediaObserver = null;
  let lastUrl = location.href;
  let checking = false;
  let checkAgain = false;

  const styles = `
    :host { all: initial; }
    .backdrop {
      position: fixed; inset: 0; z-index: 2147483647;
      display: flex; align-items: center; justify-content: center;
      padding: 24px; box-sizing: border-box;
      background: #14161a; color: #f6f5f4;
      font-family: system-ui, -apple-system, "Cantarell", "Noto Sans", sans-serif;
      line-height: 1.5;
    }
    .card { width: min(34rem, 100%); }
    .eyebrow {
      margin: 0 0 12px; font-size: 0.8rem; font-weight: 700;
      letter-spacing: 0.12em; text-transform: uppercase; color: #9ec1ff;
    }
    h1 { margin: 0 0 12px; font-size: 1.75rem; line-height: 1.2; }
    p { margin: 0 0 10px; color: #d7d3cf; }
    .target { color: #ffffff; font-weight: 600; overflow-wrap: anywhere; }
    .url {
      display: block; margin: 16px 0; padding: 10px 12px;
      border: 1px solid #3a3f47; border-radius: 8px;
      background: #1d2026; color: #c9c5c1;
      font: 0.85rem ui-monospace, "Cantarell Mono", monospace;
      white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    }
    .actions { display: flex; gap: 10px; flex-wrap: wrap; margin-top: 18px; }
    button {
      appearance: none; border-radius: 8px; padding: 10px 18px;
      font: inherit; font-weight: 600; cursor: pointer;
      border: 1px solid #4a505a; background: #262a31; color: #f6f5f4;
    }
    button.primary { background: #3584e4; border-color: #3584e4; color: #ffffff; }
    button:focus-visible { outline: 3px solid #9ec1ff; outline-offset: 2px; }
    .hint { margin-top: 18px; font-size: 0.85rem; color: #a9a49f; }
  `;

  function build(decision) {
    const info = describe(decision, new Date());
    const shadow = host.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = styles;

    const backdrop = document.createElement("div");
    backdrop.className = "backdrop";

    const card = document.createElement("div");
    card.className = "card";
    card.setAttribute("role", "alertdialog");
    card.setAttribute("aria-modal", "true");
    card.setAttribute("aria-labelledby", "stayproductive-title");
    card.setAttribute("aria-describedby", "stayproductive-detail");

    const eyebrow = document.createElement("p");
    eyebrow.className = "eyebrow";
    eyebrow.textContent = "StayProductive";

    const title = document.createElement("h1");
    title.id = "stayproductive-title";
    title.textContent = info.title;

    const detail = document.createElement("p");
    detail.id = "stayproductive-detail";
    detail.textContent = info.detail;

    const targetLine = document.createElement("p");
    if (info.target) {
      const label = document.createElement("span");
      label.textContent = `${info.target} is covered. `;
      label.className = "target";
      targetLine.append(label);
    }
    targetLine.append(document.createTextNode(info.countdown));

    const url = document.createElement("code");
    url.className = "url";
    url.textContent = location.href;
    url.title = location.href;

    const actions = document.createElement("div");
    actions.className = "actions";

    const back = document.createElement("button");
    back.type = "button";
    back.className = "primary";
    back.textContent = "Go back";
    back.addEventListener("click", () => {
      if (history.length > 1) history.back();
      else history.go(0);
    });

    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "Check again";
    retry.addEventListener("click", () => check(true));

    const status = document.createElement("p");
    status.className = "hint";
    status.setAttribute("aria-live", "polite");
    status.textContent =
      "Close the tab or go back. Your local StayProductive service enforces this block.";

    actions.append(back, retry);
    card.append(eyebrow, title, detail, targetLine, url, actions, status);
    backdrop.append(card);
    shadow.append(style, backdrop);

    card.addEventListener("keydown", (event) => {
      if (event.key !== "Tab") return;
      const focusable = [back, retry];
      const index = focusable.indexOf(shadow.activeElement);
      event.preventDefault();
      const next = event.shiftKey
        ? (index <= 0 ? focusable.length - 1 : index - 1)
        : (index + 1) % focusable.length;
      focusable[next].focus();
    });

    return { back, status };
  }

  function pauseMedia() {
    for (const media of document.querySelectorAll("audio, video")) {
      try {
        media.pause();
      } catch (_error) {
        // A page can replace the prototype; the block still stands.
      }
    }
  }

  function watchMedia() {
    if (mediaObserver || !document.documentElement) return;
    mediaObserver = new MutationObserver(pauseMedia);
    mediaObserver.observe(document.documentElement, { childList: true, subtree: true });
    pauseMedia();
  }

  function unmount() {
    mediaObserver?.disconnect();
    mediaObserver = null;
    host?.remove();
    host = null;
  }

  function mount(decision) {
    if (!document.documentElement) return;
    unmount();
    host = document.createElement("div");
    host.id = "stayproductive-block";
    const { back } = build(decision);
    document.documentElement.append(host);
    watchMedia();
    back.focus({ preventScroll: true });
  }

  async function check(fromRetry = false) {
    if (checking) {
      checkAgain = true;
      return;
    }
    checking = true;
    try {
      const response = await api.runtime.sendMessage({
        type: "decision",
        url: location.href,
        title: document.title || "",
      });
      if (!response?.ok) {
        // Fail open when the daemon cannot answer. The toolbar badge shows the
        // degraded state; a local error must not look like a user rule.
        unmount();
        return;
      }
      if (response.data?.blocked) {
        mount(response.data);
      } else {
        unmount();
        if (fromRetry) location.reload();
      }
    } catch (_error) {
      unmount();
    } finally {
      checking = false;
      if (checkAgain) {
        checkAgain = false;
        check();
      }
    }
  }

  function urlChanged() {
    if (location.href === lastUrl) return;
    lastUrl = location.href;
    check();
  }

  addEventListener("popstate", urlChanged, true);
  addEventListener("hashchange", urlChanged, true);
  setInterval(urlChanged, POLL_MS);
  check();
})();
