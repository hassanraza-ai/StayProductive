// The pre-request block page. It reads only the reason, host, and end time the
// service worker put in the redirect, and never echoes them as HTML.

(function () {
  const params = new URLSearchParams(location.search);
  const decision = {
    blocked: true,
    reason: params.get("reason") || "R-SITE",
    target: params.get("target") || "",
    until: params.get("until") || "",
  };
  const info = globalThis.StayProductiveBlockPage.describe(decision, new Date());

  document.title = `${info.title} — StayProductive`;
  document.getElementById("title").textContent = info.title;
  document.getElementById("detail").textContent = info.detail;
  document.getElementById("target").textContent = info.target
    ? `${info.target} is covered.`
    : "";
  document.getElementById("until").textContent = info.countdown;

  const back = document.getElementById("back");
  const retry = document.getElementById("retry");

  back.addEventListener("click", () => {
    if (history.length > 1) history.back();
    else window.close();
  });

  // `document.referrer` is the blocked URL when the browser provides it. If it
  // is absent the page cannot reconstruct the address, so the button stays
  // hidden rather than guessing.
  const original = document.referrer;
  if (/^https?:/i.test(original)) {
    retry.hidden = false;
    retry.addEventListener("click", () => location.replace(original));
  }

  if (info.until && decision.until) {
    const until = document.getElementById("until");
    setInterval(() => {
      until.textContent = globalThis.StayProductiveBlockPage.describe(
        decision,
        new Date(),
      ).countdown;
    }, 30_000);
  }

  document.querySelector("button.primary").focus();
})();
