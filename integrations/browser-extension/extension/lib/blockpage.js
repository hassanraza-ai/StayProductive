// Text and DOM for both the inline overlay and the redirected block page.
//
// Classic script: loaded by the content script before `content.js`, by
// `blocked.html` before `blocked.js`, and by Node tests through `vm`.
(function (root) {
  function pad(value) {
    return String(value).padStart(2, "0");
  }

  function parseDate(value) {
    if (!value) return null;
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? null : date;
  }

  // "until 14:30" for today, a weekday plus time within a week, otherwise a
  // short date. The end time is always local, matching the UI.
  function formatUntil(value, now = new Date()) {
    const date = parseDate(value);
    if (!date) return "";
    const sameDay =
      date.getFullYear() === now.getFullYear() &&
      date.getMonth() === now.getMonth() &&
      date.getDate() === now.getDate();
    const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
    if (sameDay) return `until ${time}`;
    const days = Math.round((date - now) / 86_400_000);
    if (days > 0 && days < 7) {
      const weekday = date.toLocaleDateString(undefined, { weekday: "long" });
      return `until ${weekday} ${time}`;
    }
    return `until ${date.toLocaleDateString(undefined, {
      day: "numeric",
      month: "short",
      year: date.getFullYear() === now.getFullYear() ? undefined : "numeric",
    })} ${time}`;
  }

  function hostFromUrl(value) {
    try {
      return new URL(value).host;
    } catch (_error) {
      return "";
    }
  }

  // A daemon decision, or the parameters of the redirected block page.
  function describe(decision, now = new Date()) {
    const reason = decision.reason || "R-SITE";
    const reasonText =
      typeof root.StayProductivePolicy?.reasonText === "function"
        ? root.StayProductivePolicy.reasonText(reason)
        : "This page is blocked";
    const until = formatUntil(decision.until, now);
    const target = decision.target || "";
    const host = /[/:?#@]/.test(target) ? hostFromUrl(target) : target;
    return {
      title: reasonText,
      detail:
        typeof root.StayProductivePolicy?.reasonDetail === "function"
          ? root.StayProductivePolicy.reasonDetail(reason)
          : "A rule in an active profile covers this page.",
      reason,
      target: host,
      until,
      countdown: until ? `${until}.` : "until you turn the profile off.",
    };
  }

  root.StayProductiveBlockPage = { describe, formatUntil, hostFromUrl };
})(globalThis);
