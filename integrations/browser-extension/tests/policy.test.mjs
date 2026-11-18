import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const {
  hostRegex,
  reasonText,
  summarizePolicy,
  toDnrRules,
} = require("../extension/lib/policy.js");

test("an exact host matches only that host", () => {
  const re = new RegExp(hostRegex("example.com", false));
  assert.equal(re.test("https://example.com/"), true);
  assert.equal(re.test("http://example.com:8080/a?b=1"), true);
  assert.equal(re.test("https://user@example.com/x"), true);
  assert.equal(re.test("https://www.example.com/"), false);
  assert.equal(re.test("https://notexample.com/"), false);
});

test("a suffix host matches subdomains but not the bare domain", () => {
  const re = new RegExp(hostRegex("example.com", true));
  assert.equal(re.test("https://www.example.com/"), true);
  assert.equal(re.test("https://a.b.example.com/x"), true);
  assert.equal(re.test("https://example.com/"), false);
  assert.equal(re.test("https://example.com.evil.test/"), false);
});

test("quick blocks outrank normal allows but not strict allows", () => {
  const rules = toDnrRules({
    site_blocks: [
      { host: "example.com", suffix: true, reason: "R-SITE" },
      { host: "news.test", suffix: false, reason: "R-QUICK", quick: true },
    ],
    site_allows: [
      { host: "good.example.com", suffix: false },
      { host: "safe.test", suffix: false, strict: true },
    ],
  });
  const byHost = (host) =>
    rules.find((rule) => rule.condition.regexFilter.includes(host.replaceAll(".", "\\.")));
  assert.equal(byHost("good.example.com").action.type, "allow");
  assert.equal(byHost("good.example.com").priority, 70);
  assert.equal(byHost("safe.test").priority, 100);
  const quick = rules.find((rule) => rule.action.type === "redirect" && rule.priority === 90);
  assert.match(quick.action.redirect.extensionPath, /R-QUICK/);
});

test("embedded frames are blocked outright, main frames get the page", () => {
  const rules = toDnrRules({
    site_blocks: [{ host: "example.com", suffix: true, reason: "R-SITE" }],
  });
  assert.equal(rules.length, 2);
  assert.equal(rules[0].condition.resourceTypes[0], "main_frame");
  assert.equal(rules[0].action.type, "redirect");
  assert.equal(rules[1].condition.resourceTypes[0], "sub_frame");
  assert.equal(rules[1].action.type, "block");
  assert.equal(rules[0].condition.regexFilter, rules[1].condition.regexFilter);
});

test("a redirect carries the reason, target, and optional end", () => {
  const [rule] = toDnrRules({
    site_blocks: [
      { host: "example.com", suffix: false, reason: "R-SITE", until: "2026-11-05T12:00:00Z" },
    ],
  });
  assert.equal(rule.action.type, "redirect");
  assert.equal(rule.condition.resourceTypes[0], "main_frame");
  assert.match(rule.action.redirect.extensionPath, /reason=R-SITE/);
  assert.match(rule.action.redirect.extensionPath, /target=example.com/);
  assert.match(rule.action.redirect.extensionPath, /until=2026-11-05T12%3A00%3A00Z/);
});

test("summary exposes the limitations instead of claiming coverage", () => {
  const summary = summarizePolicy({
    site_blocks: [{ host: "a.test" }],
    keywords: [{ text: "games" }, { text: "shorts" }],
    limitations: ["site_limits", "surface_rules"],
    refresh_after_seconds: 15,
  });
  assert.deepEqual(summary, {
    blocks: 1,
    allows: 0,
    keywords: 2,
    limitations: ["site_limits", "surface_rules"],
    refreshAfter: 15,
  });
  assert.equal(reasonText("R-KEYWORD"), "This page matches a blocked keyword");
  assert.equal(reasonText("unknown"), "This page is blocked");
});
