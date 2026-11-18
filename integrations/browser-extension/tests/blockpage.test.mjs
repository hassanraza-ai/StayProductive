import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = path.join(here, "..", "extension");
const context = { Date, URL, Number, String, console };
context.globalThis = context;
vm.createContext(context);
for (const file of ["lib/policy.js", "lib/blockpage.js"]) {
  vm.runInContext(fs.readFileSync(path.join(extension, file), "utf8"), context);
}
const { describe, formatUntil, hostFromUrl } = context.StayProductiveBlockPage;

function local(year, month, day, hour, minute) {
  return new Date(year, month - 1, day, hour, minute);
}

test("a same-day end time is shown as a local clock time", () => {
  const now = local(2026, 11, 18, 10, 0);
  const until = local(2026, 11, 18, 15, 30).toISOString();
  assert.equal(formatUntil(until, now), "until 15:30");
});

test("a later weekday is named and an absent end is explained", () => {
  const now = local(2026, 11, 18, 10, 0);
  const later = local(2026, 11, 20, 9, 5).toISOString();
  const text = formatUntil(later, now);
  assert.match(text, /^until .+ 09:05$/);
  assert.notEqual(text, "until 09:05");
  assert.equal(formatUntil(null, now), "");
  assert.equal(formatUntil("not a date", now), "");
});

test("describe keeps the reason, host, and honest end wording", () => {
  const now = local(2026, 11, 18, 10, 0);
  const keyword = describe(
    {
      reason: "R-KEYWORD",
      target: "https://news.example.com/a?q=shorts",
      until: local(2026, 11, 18, 12, 0).toISOString(),
    },
    now,
  );
  assert.equal(keyword.title, "This page matches a blocked keyword");
  assert.match(keyword.detail, /keyword/);
  assert.equal(keyword.target, "news.example.com");
  assert.equal(keyword.countdown, "until 12:00.");

  const site = describe({ reason: "R-SITE", target: "example.com" }, now);
  assert.equal(site.until, "");
  assert.equal(site.countdown, "until you turn the profile off.");
  assert.equal(site.detail, "A rule in an active profile covers this site. Nothing was uploaded.");

  const unknown = describe({ reason: "R-SOMETHING", target: "" }, now);
  assert.equal(unknown.title, "This page is blocked");
  assert.equal(unknown.target, "");
});

test("hostFromUrl never throws on malformed input", () => {
  assert.equal(hostFromUrl("https://example.com:8443/x"), "example.com:8443");
  assert.equal(hostFromUrl("not a url"), "");
});
