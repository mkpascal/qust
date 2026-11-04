// Glue between the page and the Rust side (src-tauri). window.__QUST__ is injected by main.rs.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const QUST = window.__QUST__ || {};

if (QUST.accent) {
    const style = document.documentElement.style;
    style.setProperty('--accent-base', QUST.accent.base);
    style.setProperty('--accent-light', QUST.accent.light);
    style.setProperty('--accent-dark', QUST.accent.dark);
}

function saveData(key, value) {
    invoke('set_data', { key, value });
}

function deleteData(key) {
    invoke('delete_data', { key });
}

async function loadData(key) {
    const value = await invoke('get_data', { key });
    return value === null ? undefined : value;
}

function proxied(url) {
    return QUST.proxy + '?url=' + encodeURIComponent(url);
}

// The proxy's details for a stream it relays (proxy.rs), and the title from the server's Icecast
// status page. Both through IPC, so they never wait behind the stream connections.
function streamDetails(url) {
    return invoke('stream_info', { url });
}

function icecastTitle(url) {
    return invoke('icecast_title', { url });
}

// Global shortcuts are stored as accelerators built from KeyboardEvent.code ("Ctrl+Alt+KeyH").
// shortcutKeys gives the labels to show: ["Ctrl", "Alt", "H"].
function shortcutKeys(accelerator) {
    const names = { Super: 'Win', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right', Backquote: '`' };
    return accelerator.split('+').map((part) =>
        names[part] || part.replace(/^Key([A-Z])$/, '$1').replace(/^Digit(\d)$/, '$1').replace(/^Numpad/, 'Num '));
}

// Right-click never shows the browser's menu. Text fields get a native edit menu and values
// marked .copyable (or <dd>) a native Copy; anything else is up to the page (the player builds its
// own menu in script.js).
document.addEventListener('contextmenu', async (event) => {
    event.preventDefault();
    const field = event.target.closest('input, textarea');
    const value = event.target.closest('.copyable, dd');
    if (!field && !value) return;
    event.stopImmediatePropagation();

    let items;
    if (field) {
        field.focus();
        items = [{ item: 'Cut' }, { item: 'Copy' }, { item: 'Paste' }, { item: 'Separator' }, { item: 'SelectAll' }];
    } else {
        const selection = window.getSelection();
        if (selection.isCollapsed || !value.contains(selection.anchorNode)) selection.selectAllChildren(value);
        items = [{ item: 'Copy' }];
    }
    const menu = await window.__TAURI__.menu.Menu.new({ items });
    await menu.popup();
});

// No reload, find or print.
document.addEventListener('keydown', (event) => {
    const key = event.key.toLowerCase();
    const browserKey = ['f3', 'f5', 'f7'].includes(key) ||
        ((event.ctrlKey || event.metaKey) && ['r', 'p', 'f', 'g', 'u', 'j'].includes(key));
    if (browserKey) event.preventDefault();
});
