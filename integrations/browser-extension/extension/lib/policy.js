// Pure policy translation shared by the service worker and Node tests.
//
// Loaded as a classic script in the extension context and with `require` in
// Node. The daemon is authoritative: this module only turns its
// `browser_policy` snapshot into declarativeNetRequest rules so an obviously
// blocked navigation can be stopped before the request. Keywords, site
// limits, and surface rules still go through the daemon decision.
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.StayProductivePolicy = api;
})(globalThis, function () {
  const HOST_ID = "io.github.stayproductive.native";

  const REASON_TEXT = {
    "R-SITE": "This site is blocked",
    "R-ADULT": "This site is filtered",
    "R-KEYWORD": "This page matches a blocked keyword",
    "R-QUICK": "A quick block is active",
    "R-PROFILE": "The active profile blocks this site",
    "R-STRICT": "Strict mode is active",
  };

  function reasonText(reason) {
    return REASON_TEXT[reason] || "This page is blocked";
  }

  function reasonDetail(reason) {
    if (reason === "R-KEYWORD") {
      return "The address or search query matches a keyword in an active profile.";
    }
    if (reason === "R-QUICK") {
      return "You started a quick block that covers this site.";
    }
    if (reason === "R-ADULT") {
      return "Adult-site filtering is on for an active profile.";
    }
    return "A rule in an active profile covers this site. Nothing was uploaded.";
  }

  function escapeHost(host) {
    return String(host).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  // `example.com` is exact; `*.example.com` matches subdomains but not the
  // bare domain, matching HostPattern in the core.
  function hostRegex(host, suffix) {
    const h = escapeHost(host);
    const user = "(?:[^/?#@]*@)?";
    const port = "(?::\\d+)?";
    if (suffix) {
      return `^https?://${user}(?:[^/?#@]+\\.)+${h}${port}(?:[/?#]|$)`;
    }
    return `^https?://${user}${h}${port}(?:[/?#]|$)`;
  }

  function ruleFor(entry, id, action, priority, resourceTypes) {
    const target = encodeURIComponent(entry.host);
    const reason = encodeURIComponent(entry.reason || "R-SITE");
    const until = entry.until ? `&until=${encodeURIComponent(entry.until)}` : "";
    let ruleAction;
    if (action === "allow") ruleAction = { type: "allow" };
    else if (action === "block") ruleAction = { type: "block" };
    else {
      ruleAction = {
        type: "redirect",
        redirect: {
          extensionPath: `/blocked.html?reason=${reason}&target=${target}${until}`,
        },
      };
    }
    return {
      id,
      priority,
      action: ruleAction,
      condition: {
        regexFilter: hostRegex(entry.host, Boolean(entry.suffix)),
        isUrlFilterCaseSensitive: false,
        resourceTypes,
      },
    };
  }

  // Priority order mirrors the daemon's precedence closely enough for a
  // pre-request net: a strict allow beats a quick block, a quick block beats a
  // normal allow, and a normal allow beats a normal site rule. Main frames get
  // the explanation page; embedded frames are blocked outright so an embedded
  // player cannot bypass the host rule.
  function toDnrRules(policy, firstId = 1) {
    const rules = [];
    let id = firstId;
    for (const allow of policy.site_allows || []) {
      if (!allow.host) continue;
      rules.push(
        ruleFor(allow, id++, "allow", allow.strict ? 100 : 70, ["main_frame", "sub_frame"]),
      );
    }
    for (const block of policy.site_blocks || []) {
      if (!block.host) continue;
      const priority = block.quick ? 90 : 50;
      rules.push(ruleFor(block, id++, "redirect", priority, ["main_frame"]));
      rules.push(ruleFor(block, id++, "block", priority, ["sub_frame"]));
    }
    return rules;
  }

  function summarizePolicy(policy) {
    return {
      blocks: (policy.site_blocks || []).length,
      allows: (policy.site_allows || []).length,
      keywords: (policy.keywords || []).length,
      limitations: policy.limitations || [],
      refreshAfter: Number(policy.refresh_after_seconds || 30),
    };
  }

  return {
    HOST_ID,
    escapeHost,
    hostRegex,
    reasonText,
    reasonDetail,
    summarizePolicy,
    toDnrRules,
  };
});
