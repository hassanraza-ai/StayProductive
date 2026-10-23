// StayProductive GNOME Shell integration.
//
// The daemon publishes a short-lived policy file. This extension reads it,
// watches the focused window, and covers blocked applications with a local
// overlay. It never contacts the network and never reads window contents.
//
// This file is original clean-room code for StayProductive.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {
    appIdsForWindow,
    findBlockedApp,
    parseTimestamp,
    policyIsCurrent,
    reasonText,
} from './app-id.js';

const EXTENSION_UUID = 'stayproductive@hassanraza.ai';
const HEARTBEAT_SECONDS = 2;

export default class StayProductiveExtension extends Extension {
    enable() {
        this._policy = null;
        this._lastUntil = null;
        this._tickId = 0;
        this._ticks = 0;
        this._monitorsChangedId = 0;
        this._focusChangedId = 0;

        this._policyPath = GLib.build_filenamev([
            GLib.get_user_data_dir(),
            'stayproductive',
            'enforcement.json',
        ]);
        this._heartbeatPath = GLib.build_filenamev([
            GLib.get_user_runtime_dir(),
            'stayproductive',
            'gnome-extension.json',
        ]);

        this._buildOverlay();

        this._focusChangedId = global.display.connect(
            'notify::focus-window',
            () => this._evaluate()
        );
        this._monitorsChangedId = Main.layoutManager.connect(
            'monitors-changed',
            () => this._resizeOverlay()
        );

        this._refreshPolicy();
        this._writeHeartbeat();
        this._evaluate();
        this._tickId = GLib.timeout_add_seconds(
            GLib.PRIORITY_DEFAULT,
            1,
            () => this._tick()
        );
    }

    disable() {
        if (this._tickId) {
            GLib.source_remove(this._tickId);
            this._tickId = 0;
        }
        if (this._focusChangedId) {
            global.display.disconnect(this._focusChangedId);
            this._focusChangedId = 0;
        }
        if (this._monitorsChangedId) {
            Main.layoutManager.disconnect(this._monitorsChangedId);
            this._monitorsChangedId = 0;
        }
        this._removeHeartbeat();
        this._destroyOverlay();
        this._policy = null;
    }

    _buildOverlay() {
        this._overlay = new St.BoxLayout({
            vertical: true,
            reactive: true,
            can_focus: true,
            visible: false,
            style: 'background-color: rgba(13, 13, 16, 0.97);',
        });

        this._content = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'spacing: 14px; padding: 48px;',
        });
        this._overlay.add_child(this._content);

        this._icon = new St.Icon({
            icon_name: 'channel-secure-symbolic',
            icon_size: 72,
            style: 'color: #f66151;',
            x_align: Clutter.ActorAlign.CENTER,
        });
        this._content.add_child(this._icon);

        this._heading = new St.Label({
            text: 'StayProductive is holding the line',
            style: 'font-size: 30px; font-weight: 700; color: #ffffff;',
            x_align: Clutter.ActorAlign.CENTER,
        });
        this._content.add_child(this._heading);

        this._appLabel = new St.Label({
            text: '',
            style: 'font-size: 18px; color: #c0bfbc;',
            x_align: Clutter.ActorAlign.CENTER,
        });
        this._content.add_child(this._appLabel);

        this._reasonLabel = new St.Label({
            text: '',
            style: 'font-size: 15px; color: #9a9996;',
            x_align: Clutter.ActorAlign.CENTER,
        });
        this._content.add_child(this._reasonLabel);

        this._untilLabel = new St.Label({
            text: '',
            style: 'font-size: 15px; color: #9a9996;',
            x_align: Clutter.ActorAlign.CENTER,
        });
        this._content.add_child(this._untilLabel);

        this._footer = new St.Label({
            text: 'Enforced locally. Switch to an allowed window to dismiss this screen.',
            style: 'font-size: 12px; color: #77767b;',
            x_align: Clutter.ActorAlign.CENTER,
        });
        this._content.add_child(this._footer);

        this._overlay.connect('button-press-event', () => Clutter.EVENT_STOP);
        this._overlay.connect('key-press-event', () => Clutter.EVENT_STOP);

        Main.layoutManager.addChrome(this._overlay, {
            affectsInputRegion: true,
            affectsStruts: false,
        });
        this._resizeOverlay();
    }

    _destroyOverlay() {
        if (!this._overlay)
            return;
        Main.layoutManager.removeChrome(this._overlay);
        this._overlay.destroy();
        this._overlay = null;
        this._content = null;
        this._icon = null;
        this._heading = null;
        this._appLabel = null;
        this._reasonLabel = null;
        this._untilLabel = null;
        this._footer = null;
    }

    _resizeOverlay() {
        if (!this._overlay)
            return;
        this._overlay.set_position(0, 0);
        this._overlay.set_size(global.screen_width, global.screen_height);
    }

    _tick() {
        this._ticks += 1;
        this._refreshPolicy();
        if (this._ticks % HEARTBEAT_SECONDS === 0)
            this._writeHeartbeat();
        this._evaluate();
        return GLib.SOURCE_CONTINUE;
    }

    _refreshPolicy() {
        let contents;
        try {
            const [ok, bytes] = GLib.file_get_contents(this._policyPath);
            if (!ok)
                throw new Error('policy file is missing');
            contents = new TextDecoder('utf-8').decode(bytes);
        } catch {
            this._policy = null;
            return;
        }

        let policy;
        try {
            policy = JSON.parse(contents);
        } catch {
            this._policy = null;
            return;
        }

        if (!policyIsCurrent(policy)) {
            this._policy = null;
            return;
        }
        this._policy = policy;
    }

    _writeHeartbeat() {
        const text = JSON.stringify({
            active: true,
            checked_at: new Date().toISOString(),
            shell_version: Config.PACKAGE_VERSION,
            policy_loaded: Boolean(this._policy),
            blocked: Boolean(this._overlay?.visible),
        });
        try {
            GLib.mkdir_with_parents(
                GLib.path_get_dirname(this._heartbeatPath),
                0o700
            );
            const file = Gio.File.new_for_path(this._heartbeatPath);
            file.replace_contents(
                new TextEncoder().encode(text),
                null,
                false,
                Gio.FileCreateFlags.REPLACE_DESTINATION,
                null
            );
            file.set_attribute_uint32(
                'unix::mode',
                0o600,
                Gio.FileQueryInfoFlags.NONE,
                null
            );
        } catch (error) {
            console.warn(`${EXTENSION_UUID}: cannot write heartbeat: ${error}`);
        }
    }

    _removeHeartbeat() {
        try {
            Gio.File.new_for_path(this._heartbeatPath).delete(null);
        } catch {
            // Already gone.
        }
    }

    _evaluate() {
        if (!this._overlay)
            return;

        if (Main.sessionMode.isLocked || Main.overview.visible) {
            this._hideOverlay();
            return;
        }

        if (!this._policy) {
            this._hideOverlay();
            return;
        }

        const win = global.display.focus_window;
        if (!win) {
            this._hideOverlay();
            return;
        }

        const ids = appIdsForWindow(win);
        if (ids.size === 0) {
            this._hideOverlay();
            return;
        }
        if ([...ids].some(id => id.includes('stayproductive'))) {
            this._hideOverlay();
            return;
        }

        const entry = findBlockedApp(this._policy, ids);
        if (!entry) {
            this._hideOverlay();
            return;
        }

        this._showOverlay(win, entry);
    }

    _showOverlay(win, entry) {
        const appName = this._displayName(win);
        if (this._appLabel.get_text() !== appName)
            this._appLabel.set_text(appName);

        const reason = reasonText(entry.reason);
        if (this._reasonLabel.get_text() !== reason)
            this._reasonLabel.set_text(reason);

        const until = parseTimestamp(entry.until);
        const untilText = until === null
            ? 'This block has no scheduled end.'
            : `Ends at ${new Date(until).toLocaleTimeString([], {
                  hour: '2-digit',
                  minute: '2-digit',
              })}`;
        if (untilText !== this._lastUntil) {
            this._untilLabel.set_text(untilText);
            this._lastUntil = untilText;
        }

        if (!this._overlay.visible) {
            this._resizeOverlay();
            this._overlay.show();
            this._overlay.raise_top();
            this._overlay.grab_key_focus();
        }
    }

    _hideOverlay() {
        if (this._overlay?.visible)
            this._overlay.hide();
    }

    _displayName(win) {
        try {
            const title = win.get_title();
            if (title && title.length > 0)
                return title;
        } catch {
            // Fall through to the app id.
        }
        for (const id of appIdsForWindow(win))
            return id;
        return 'This application';
    }
}
