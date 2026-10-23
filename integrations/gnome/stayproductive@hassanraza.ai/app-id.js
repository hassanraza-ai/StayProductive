// Pure helpers shared by the extension and its unit tests.
//
// Keeping this file free of GNOME Shell imports lets `gjs -m` test matching
// without starting a compositor. This is original clean-room code.

export function normalizeAppId(value) {
    return String(value).trim().toLowerCase().replace(/\.desktop$/, '');
}

export function appIdsFromValues(values) {
    const ids = new Set();
    for (const value of values) {
        if (typeof value !== 'string' || value.length === 0)
            continue;
        const normalized = normalizeAppId(value);
        if (!normalized)
            continue;
        ids.add(normalized);
        const tail = normalized.split('.').pop();
        if (tail && tail !== normalized)
            ids.add(tail);
    }
    return ids;
}

export function appIdsForWindow(win) {
    const values = [];
    for (const getter of [
        'get_wm_class',
        'get_wm_class_instance',
        'get_gtk_application_id',
        'get_sandboxed_app_id',
    ]) {
        try {
            const value = win[getter]?.();
            if (value)
                values.push(value);
        } catch {
            // Some window types do not implement every accessor.
        }
    }
    return appIdsFromValues(values);
}

export function findBlockedApp(policy, ids) {
    if (!policy || !Array.isArray(policy.apps))
        return null;
    return (
        policy.apps.find(item =>
            ids.has(normalizeAppId(item?.app ?? ''))
        ) ?? null
    );
}

export function parseTimestamp(value) {
    if (typeof value !== 'string' || value.length === 0)
        return null;
    const parsed = Date.parse(value);
    return Number.isFinite(parsed) ? parsed : null;
}

export function policyIsCurrent(policy, nowMs = Date.now()) {
    if (!policy || policy.version !== 1)
        return false;
    const validUntil = parseTimestamp(policy.valid_until);
    return validUntil !== null && validUntil >= nowMs;
}

export const REASON_TEXT = {
    'R-PROFILE': 'A focus profile is active.',
    'R-STRICT': 'A strict session is active.',
    'R-QUICK': 'A quick block is active.',
    'R-LIMIT': 'A usage limit has been reached.',
    'R-LIMIT-DAILY': 'The daily limit for this app has been reached.',
    'R-LIMIT-HOURLY': 'The hourly limit for this app has been reached.',
    'R-LAUNCH': 'This app was launched during a focus session.',
    'R-PICKUP': 'This app is blocked while pickup protection is active.',
    'R-WAIT': 'This app is blocked for the configured waiting period.',
    'R-KEYWORD': 'A keyword rule matched.',
    'R-SITE': 'A site rule matched.',
    'R-SURFACE': 'This surface is blocked.',
};

export function reasonText(code) {
    return REASON_TEXT[code] ?? 'A StayProductive rule applies.';
}
