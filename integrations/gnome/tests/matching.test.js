// Unit tests for the pure matching helpers used by the GNOME extension.
// Run with: gjs -m integrations/gnome/tests/matching.test.js

import {
    appIdsForWindow,
    appIdsFromValues,
    findBlockedApp,
    parseTimestamp,
    policyIsCurrent,
    reasonText,
} from '../stayproductive@hassanraza.ai/app-id.js';

function assert(condition, message) {
    if (!condition)
        throw new Error(message);
}

function fakeWindow(values) {
    const win = {};
    for (const [getter, value] of Object.entries(values)) {
        win[getter] = () => {
            if (value instanceof Error)
                throw value;
            return value;
        };
    }
    return win;
}

function testNormalisation() {
    const ids = appIdsFromValues([
        'org.mozilla.firefox',
        'Firefox.desktop',
        'com.example.BlockMeTest',
        '',
        null,
        42,
    ]);
    assert(ids.has('org.mozilla.firefox'), 'full desktop id is kept');
    assert(ids.has('firefox'), 'desktop suffix is normalized');
    assert(ids.has('com.example.blockmetest'), 'ids are lower-cased');
    assert(ids.has('blockmetest'), 'trailing component is an alias');
    assert(!ids.has(''), 'empty values are ignored');
    assert(ids.size === 4, `unexpected ids: ${[...ids]}`);
}

function testWindowAccessors() {
    const win = fakeWindow({
        get_wm_class: 'firefox',
        get_wm_class_instance: 'Navigator',
        get_gtk_application_id: 'org.mozilla.firefox',
        get_sandboxed_app_id: new Error('not a sandboxed app'),
    });
    const ids = appIdsForWindow(win);
    assert(ids.has('firefox'), 'WM_CLASS is used');
    assert(ids.has('navigator'), 'WM_CLASS instance is used');
    assert(ids.has('org.mozilla.firefox'), 'GApplication id is used');
}

function testMatching() {
    const policy = {
        version: 1,
        valid_until: new Date(Date.now() + 30_000).toISOString(),
        apps: [
            {app: 'firefox', reason: 'R-PROFILE'},
            {app: 'org.gnome.Nautilus', reason: 'R-QUICK'},
        ],
    };
    const firefox = appIdsForWindow(
        fakeWindow({get_wm_class: 'org.mozilla.firefox'})
    );
    assert(findBlockedApp(policy, firefox)?.reason === 'R-PROFILE', 'alias matches');
    const nautilus = appIdsForWindow(
        fakeWindow({get_gtk_application_id: 'org.gnome.Nautilus'})
    );
    assert(findBlockedApp(policy, nautilus)?.reason === 'R-QUICK', 'desktop id matches');
    const allowed = appIdsFromValues(['com.example.Editor']);
    assert(findBlockedApp(policy, allowed) === null, 'unlisted app is allowed');
    assert(findBlockedApp(null, firefox) === null, 'missing policy allows');
}

function testPolicyFreshness() {
    const current = {
        version: 1,
        valid_until: new Date(Date.now() + 30_000).toISOString(),
    };
    assert(policyIsCurrent(current), 'fresh policy is current');
    assert(!policyIsCurrent({...current, valid_until: new Date(Date.now() - 1).toISOString()}), 'expired policy is stale');
    assert(!policyIsCurrent({...current, version: 2}), 'unknown version is stale');
    assert(!policyIsCurrent({version: 1, valid_until: 'not a date'}), 'invalid date is stale');
    assert(parseTimestamp(current.valid_until) > 0, 'timestamp parses');
    assert(parseTimestamp(null) === null, 'missing timestamp is null');
}

function testReasonText() {
    assert(reasonText('R-QUICK').includes('quick block'), 'known reason');
    assert(reasonText('R-LIMIT-DAILY').includes('daily limit'), 'daily limit reason');
    assert(reasonText('R-UNKNOWN').includes('StayProductive'), 'fallback reason');
}

testNormalisation();
testWindowAccessors();
testMatching();
testPolicyFreshness();
testReasonText();
print('matching.test.js: ok');
