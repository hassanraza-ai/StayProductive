// Native-messaging bridge and declarativeNetRequest policy cache.
//
// The daemon decides; this worker only asks it and applies the best-effort
// host rules it returns. It never stores page URLs.

if (typeof importScripts === "function") importScripts("lib/policy.js");

const api = globalThis.browser ?? globalThis.chrome;
const { HOST_ID, summarizePolicy, toDnrRules } = globalThis.StayProductivePolicy;
const APP = /Firefox/i.test(navigator.userAgent)
  ? "firefox"
  : /Edg\//.test(navigator.userAgent)
    ? "microsoft-edge"
    : "google-chrome";

const POLICY_ALARM = "stayproductive-policy";
const REFRESH_MS = 30_000;
let port = null;
let nextId = 1;
let lastRefresh = 0;
let refreshing = null;
const pending = new Map();

function connect() {
  if (port) return port;
  port = api.runtime.connectNative(HOST_ID);
  port.onMessage.addListener((message) => {
    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    if (message.ok) entry.resolve(message.data);
    else entry.reject(new Error(message.error || "native host error"));
  });
  port.onDisconnect.addListener(() => {
    const error = api.runtime.lastError;
    port = null;
    for (const entry of pending.values()) {
      entry.reject(new Error(error?.message || "native host disconnected"));
    }
    pending.clear();
    setBadge("!");
  });
  return port;
}

function nativeRequest(command, args = {}) {
  return new Promise((resolve, reject) => {
    const id = `${Date.now()}-${nextId++}`;
    pending.set(id, { resolve, reject });
    try {
      connect().postMessage({ id, command, args });
    } catch (error) {
      pending.delete(id);
      reject(error);
    }
  });
}

function setBadge(text, color = "#c01c28") {
  if (!api.action?.setBadgeText) return;
  api.action.setBadgeText({ text });
  if (text && api.action.setBadgeBackgroundColor) {
    api.action.setBadgeBackgroundColor({ color });
  }
}

async function applyRules(policy) {
  if (!api.declarativeNetRequest?.updateDynamicRules) return;
  const existing = await api.declarativeNetRequest.getDynamicRules();
  const removeRuleIds = existing.map((rule) => rule.id);
  const addRules = toDnrRules(policy, 1);
  await api.declarativeNetRequest.updateDynamicRules({ removeRuleIds, addRules });
}

// A failed refresh keeps the last known host rules. That is deliberate: the
// daemon is the authority, and dropping the pre-request net would silently
// weaken an active block. The badge says the extension is degraded.
async function refreshPolicy() {
  if (refreshing) return refreshing;
  refreshing = (async () => {
    try {
      const policy = await nativeRequest("browser_policy");
      await applyRules(policy);
      lastRefresh = Date.now();
      setBadge("");
      const summary = summarizePolicy(policy);
      await api.storage.local.set({ policy: summary, checkedAt: Date.now() });
      return summary;
    } catch (_error) {
      setBadge("!");
      await api.storage.local.set({
        policy: null,
        checkedAt: Date.now(),
        degraded: "The StayProductive daemon is not reachable.",
      });
      return null;
    } finally {
      refreshing = null;
    }
  })();
  return refreshing;
}

function maybeRefresh() {
  if (Date.now() - lastRefresh > REFRESH_MS) refreshPolicy();
}

api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type !== "decision" || typeof message.url !== "string") return false;
  nativeRequest("browser_decision", {
    url: message.url,
    title: message.title || "",
    app: APP,
  })
    .then((data) => {
      sendResponse({ ok: true, data });
      maybeRefresh();
    })
    .catch((error) => sendResponse({ ok: false, error: error.message }));
  return true;
});

api.runtime.onInstalled.addListener(() => {
  refreshPolicy();
  api.alarms?.create(POLICY_ALARM, { periodInMinutes: 1 });
});
api.runtime.onStartup.addListener(() => {
  refreshPolicy();
  api.alarms?.create(POLICY_ALARM, { periodInMinutes: 1 });
});
api.alarms?.onAlarm.addListener((alarm) => {
  if (alarm.name === POLICY_ALARM) refreshPolicy();
});
refreshPolicy();
