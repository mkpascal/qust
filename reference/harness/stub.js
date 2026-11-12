// Stands in for Tauri in a plain browser, so the unmodified pages (reference/page) run as they do in
// the app. Injected before any page script, after `window.__REF_FIXTURE__` (the state file being
// rendered). Commands answer from the fixture; everything the page sends is recorded in
// window.__REF__.calls, and the harness pushes events and analyzer frames through window.__REF__.
(function () {
    const F = window.__REF_FIXTURE__ || {};
    const clone = (value) => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)));

    // A fixed "now" (Settings shows "Last checked 2 h ago").
    if (typeof F.now === 'number') {
        const now = F.now * 1000;
        Date.now = () => now;
    }

    const calls = [];
    const menus = [];
    const emitted = [];
    const listeners = new Map();
    let channel = null;
    let dragDrop = null;
    let nextRid = 0;
    let nextListener = 0;

    function deliver(event, payload) {
        for (const [id, handler] of listeners.get(event) || []) {
            handler({ event, id, payload: clone(payload) });
        }
    }

    const answers = {
        player_attach(args) {
            channel = args.frames;
            return { state: F.state, playlist: { entries: F.playlist || [] } };
        },
        get_data({ key }) {
            const settings = F.settings || {};
            return key in settings ? settings[key] : null;
        },
        set_data: () => null,
        delete_data: () => null,
        get_shortcuts: () => F.shortcuts || {},
        pause_shortcuts: () => F.shortcuts || {},
        set_shortcut: () => F.shortcuts || {},
        update_status: () => F.update || { installed: false, status: 'idle', auto: true },
        get_background: () => F.background || { available: true, enabled: false, startsWithWindows: false },
        set_background: ({ enabled }) => Object.assign({}, F.background, { enabled }),
        player_status: () => F.status || null,
        stream_info: () => F.info || {},
        playlist_add_files: () => [],
        playlist_drop: () => [],
        playlist_add_url: () => F.addedIds || [],
        popup_menu: () => null,
    };

    function invoke(cmd, args) {
        calls.push({ cmd, args: args === undefined ? null : sanitize(args) });
        const errors = F.errors || {};
        if (cmd in errors) return Promise.reject(errors[cmd]);
        const answer = answers[cmd];
        return Promise.resolve(answer ? clone(answer(args || {})) : null);
    }

    function sanitize(args) {
        const out = {};
        for (const [key, value] of Object.entries(args)) {
            out[key] = value instanceof Channel ? '[Channel]' : value;
        }
        return out;
    }

    class Channel {
        constructor() {
            this.id = ++nextListener;
            this.onmessage = () => {};
        }
    }

    class Menu {
        constructor(options) {
            this.rid = ++nextRid;
            this.items = (options.items || []).map((item) => {
                const copy = {};
                for (const [key, value] of Object.entries(item)) {
                    if (typeof value !== 'function') copy[key] = value;
                }
                return copy;
            });
            this.actions = (options.items || []).map((item) => item.action || null);
        }

        static async new(options) {
            for (const item of options.items || []) {
                // Tauri rejects an accelerator it can't parse; the harness accepts any.
                if ('accelerator' in item && typeof item.accelerator !== 'string') throw new Error('bad accelerator');
            }
            const menu = new Menu(options);
            menus.push(menu);
            return menu;
        }

        async popup() {}
    }

    class Logical {
        constructor(a, b) {
            this.type = 'Logical';
            [this.x, this.y, this.width, this.height] = [a, b, a, b];
        }
    }

    const currentWindow = {
        label: F.window === 'player' ? 'main' : F.window,
        close() {
            calls.push({ cmd: 'window.close', args: null });
            return Promise.resolve();
        },
    };

    window.__TAURI__ = {
        core: { invoke, Channel },
        event: {
            listen(event, handler) {
                const id = ++nextListener;
                if (!listeners.has(event)) listeners.set(event, new Map());
                listeners.get(event).set(id, handler);
                return Promise.resolve(() => listeners.get(event).delete(id));
            },
            emit(event, payload) {
                emitted.push({ event, payload: clone(payload) });
                deliver(event, payload);
                return Promise.resolve();
            },
        },
        menu: { Menu },
        window: { getCurrentWindow: () => currentWindow },
        webview: {
            getCurrentWebview: () => ({
                label: currentWindow.label,
                onDragDropEvent(handler) {
                    dragDrop = handler;
                    return Promise.resolve(() => {});
                },
            }),
        },
        app: { getVersion: () => Promise.resolve(F.version || '1.1.5') },
        dpi: { LogicalPosition: Logical, LogicalSize: Logical, PhysicalPosition: Logical, PhysicalSize: Logical },
    };

    window.__QUST__ = { accent: F.accent || null, debug: false };

    window.__REF__ = {
        calls,
        menus,
        emitted,
        emit: deliver,
        // An analyzer frame (77 bytes), as the Rust channel sends it: an ArrayBuffer.
        frame(bytes) {
            if (!channel) throw new Error('no frame channel (player_attach not called)');
            channel.onmessage(new Uint8Array(bytes).buffer);
        },
        // A Tauri drag-and-drop event ({ type: 'enter' | 'over' | 'drop' | 'leave', position, paths }).
        dragDrop(payload) {
            if (!dragDrop) throw new Error('no drag-drop handler');
            return dragDrop({ event: 'tauri://drag-drop', id: 0, payload });
        },
        hasChannel: () => !!channel,
    };
})();
