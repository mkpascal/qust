// The player window, as a view. The player itself (cartridges, playlist, playback, reconnecting,
// settings, persistence) is in Rust: the page renders what it is sent (`player-state`,
// `player-playlist`, `player-entry`), paints the analyzer frames, and turns what the user does into
// commands. It keeps only view state: the selection, drags, the dialog, the drawer, slider drags
// and the title marquee.

const SLOTS = 6;
const DRAG_THRESHOLD = 5;
// The seek bar's value is the position in ten-thousandths.
const SEEK_STEPS = 10000;
const AUDIO_FILE = /\.(mp3|m4a|aac|mp4|ogg|oga|opus|flac|wav|webm|weba)$/i;

const spectrum = new Spectrum(document.getElementById('canvas'));
const { Channel } = window.__TAURI__.core;
const { Menu } = window.__TAURI__.menu;

const ui = {
    screen: document.getElementById('screen'),
    clock: document.getElementById('clock'),
    clockText: document.getElementById('clockText'),
    stateIcon: document.getElementById('stateIcon'),
    station: document.getElementById('station'),
    titleRow: document.getElementById('titleRow'),
    title: document.getElementById('title'),
    kbps: document.getElementById('kbps'),
    khz: document.getElementById('khz'),
    mono: document.getElementById('mono'),
    stereo: document.getElementById('stereo'),
    sliders: document.getElementById('sliders'),
    seekSlider: document.getElementById('seekSlider'),
    seekTime: document.getElementById('seekTime'),
    volumeIcon: document.getElementById('volumeIcon'),
    volumeValue: document.getElementById('volumeValue'),
    balanceValue: document.getElementById('balanceValue'),
    playButton: document.getElementById('playButton'),
    pauseButton: document.getElementById('pauseButton'),
    stopButton: document.getElementById('stopButton'),
    instantButton: document.getElementById('instantButton'),
    infoButton: document.getElementById('infoButton'),
    settingsButton: document.getElementById('settingsButton'),
    cartridges: document.getElementById('cartridges'),
    dialog: document.getElementById('dialog'),
    dialogForm: document.getElementById('dialogForm'),
    dialogTitle: document.getElementById('dialogTitle'),
    dialogError: document.getElementById('dialogError'),
    nameLabel: document.getElementById('nameLabel'),
    urlLabel: document.getElementById('urlLabel'),
    streamTitle: document.getElementById('streamTitle'),
    streamURL: document.getElementById('streamURL'),
    saveButton: document.getElementById('saveButton'),
    ejectButton: document.getElementById('ejectButton'),
    cancelButton: document.getElementById('cancelButton'),
    playlistButton: document.getElementById('playlistButton'),
    addFileButton: document.getElementById('addFileButton'),
    addUrlButton: document.getElementById('addUrlButton'),
    drawer: document.getElementById('drawer'),
    playlist: document.getElementById('playlist'),
    playlistCount: document.getElementById('playlistCount'),
};

// The last state from Rust (null until player_attach answers), and the playlist entries in order,
// with the index of each id and their total length.
let player = null;
let entries = [];
const entryIndex = new Map();
let totalDuration = 0;
let shortcuts = {};

/* DOM updates. The state comes every 250 ms while playing: these touch the DOM only when a value
   actually changed. */

function setText(element, text) {
    text = String(text);
    if (element.textContent !== text) element.textContent = text;
}

function setClass(element, name, on) {
    if (element.classList.contains(name) !== on) element.classList.toggle(name, on);
}

function setAttr(element, name, value) {
    value = String(value);
    if (element.getAttribute(name) !== value) element.setAttribute(name, value);
}

function setProp(element, name, value) {
    if (element[name] !== value) element[name] = value;
}

/* What plays */

// The rows of the playlist show items: an entry by its id (a number), or a pinned cartridge by its
// slot ("slot0" to "slot5").
function cartridgeKey(slot) {
    return 'slot' + slot;
}

function isTrack(item) {
    return typeof item === 'number';
}

function slotOf(item) {
    return Number(item.slice(4));
}

function entryOf(id) {
    return entries[entryIndex.get(id)];
}

// The active cartridge or playlist entry, as a row item, or null.
function activeItem() {
    if (!player) return null;
    if (player.activeSlot !== null && player.cartridges[player.activeSlot]) return cartridgeKey(player.activeSlot);
    return player.activeId;
}

function isPlaying() {
    return player.state !== 'stopped' && player.state !== 'paused';
}

function playItem(item) {
    if (isTrack(item)) invoke('playlist_play', { id: item });
    else invoke('cartridge_play', { slot: slotOf(item) });
}

function host(url) {
    try {
        return new URL(url).host;
    } catch (error) {
        return url;
    }
}

function fileTitle(path) {
    return path.split(/[\\/]/).pop().replace(/\.[^.]+$/, '');
}

// An online file's name, or a stream's host: what Rust names an entry left without a name.
function urlTitle(url) {
    try {
        const parsed = new URL(url);
        const name = decodeURIComponent(parsed.pathname.split('/').pop() || '');
        return AUDIO_FILE.test(name) ? name.replace(/\.[^.]+$/, '') : parsed.host;
    } catch (error) {
        return url;
    }
}

function formatTime(seconds) {
    seconds = Math.round(seconds);
    const h = Math.floor(seconds / 3600);
    const m = Math.floor(seconds / 60) % 60;
    const s = String(seconds % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

function formatKhz(samplerate) {
    const khz = samplerate / 1000;
    return Number.isInteger(khz) ? String(khz) : khz.toFixed(1);
}

/* Rendering the state */

function render(next) {
    const previous = player;
    player = next;
    renderTransport();
    renderDisplay();
    renderCartridges();
    volume.set(player.volume);
    balance.set(player.balance);
    renderSeek();
    if (!previous || previous.colors !== player.colors) applyColors();
    if (!previous || !sameCartridges(previous.cartridges, player.cartridges)) renderPlaylist();
    else renderActiveRows();
}

function renderTransport() {
    const { state } = player;
    const icon = { playing: 'play', buffering: 'play', connecting: 'play', paused: 'pause' }[state] || 'stop';
    setAttr(ui.stateIcon, 'href', '#i-' + icon);
    setClass(ui.clock, 'is-idle', state === 'stopped');
    const seconds = state === 'stopped' ? 0 : Math.max(0, Math.floor(player.elapsed));
    const h = Math.floor(seconds / 3600);
    const m = Math.floor(seconds / 60) % 60;
    const s = String(seconds % 60).padStart(2, '0');
    setText(ui.clockText, h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${String(m).padStart(2, '0')}:${s}`);
    setProp(ui.playButton, 'disabled', activeItem() === null || state === 'playing' || state === 'buffering' || state === 'connecting');
    setProp(ui.pauseButton, 'disabled', state === 'stopped' || state === 'paused');
    setProp(ui.stopButton, 'disabled', state === 'stopped');
    setAttr(ui.instantButton, 'aria-pressed', player.instant);
    setProp(ui.instantButton, 'title', player.instant
        ? 'Instant switch is on: every cartridge streams in the background, even when stopped, so starting or switching is immediate (uses more data)'
        : 'Instant switch: keep every cartridge streaming in the background, even when stopped, for immediate starting and switching (uses more data)');
}

function renderDisplay() {
    setText(ui.station, player.station);
    setTitle(player.title, !!player.titleDim);
    setText(ui.kbps, player.bitrate || '--');
    setText(ui.khz, player.samplerate ? formatKhz(player.samplerate) : '--');
    setClass(ui.mono, 'is-on', player.channels === 1);
    setClass(ui.stereo, 'is-on', player.channels >= 2);
}

// A title wider than its line scrolls back and forth.
function setTitle(text, dim) {
    if (ui.title.textContent === text && ui.title.classList.contains('is-dim') === dim) return;
    ui.title.textContent = text;
    ui.title.classList.toggle('is-dim', dim);
    ui.titleRow.classList.remove('is-scrolling');
    const overflow = ui.title.offsetWidth - ui.titleRow.clientWidth;
    if (overflow > 0) {
        ui.titleRow.classList.add('is-scrolling');
        ui.titleRow.style.setProperty('--distance', -(overflow + 18) + 'px');
        ui.titleRow.style.setProperty('--duration', Math.max(6, overflow / 20 + 4) + 's');
    }
}

function applyColors() {
    const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent-light').trim();
    spectrum.setColors(player.colors, accent);
}

/* Cartridges */

// The parts of each cartridge's element, built with the first state.
const cartridgeParts = [];

function buildCartridges() {
    for (let slot = 0; slot < SLOTS; slot++) {
        const element = document.createElement('div');
        element.className = 'cartridge';
        element.innerHTML = `
            <button class="cartridge-body">
              <span class="equalizer" aria-hidden="true"><i></i><i></i><i></i></span>
              <svg class="cartridge-bolt" aria-hidden="true"><use href="#i-bolt" /></svg>
              <svg class="cartridge-plus"><use href="#i-plus" /></svg>
              <span class="cartridge-name"></span>
              <span class="cartridge-host"></span>
            </button>
            <button class="cartridge-eject" title="Edit or eject" aria-label="Edit or eject"><svg><use href="#i-eject" /></svg></button>`;
        const parts = {
            body: element.querySelector('.cartridge-body'),
            plus: element.querySelector('.cartridge-plus'),
            name: element.querySelector('.cartridge-name'),
            host: element.querySelector('.cartridge-host'),
            eject: element.querySelector('.cartridge-eject'),
        };
        parts.body.addEventListener('click', () => {
            if (player.cartridges[slot]) invoke('cartridge_play', { slot });
            else openDialog(slot);
        });
        parts.eject.addEventListener('click', () => openDialog(slot));
        cartridgeParts.push(parts);
        ui.cartridges.appendChild(element);
    }
}

function renderCartridges() {
    if (!cartridgeParts.length) buildCartridges();
    for (let slot = 0; slot < SLOTS; slot++) {
        const cartridge = player.cartridges[slot];
        const element = ui.cartridges.children[slot];
        const parts = cartridgeParts[slot];
        const empty = !cartridge;
        const isActive = !empty && slot === player.activeSlot;
        setClass(element, 'is-empty', empty);
        setClass(element, 'is-active', isActive);
        setClass(element, 'is-playing', isActive && isPlaying());
        // Instant switch: a bolt on the other cartridges, faint while connecting, solid once live.
        setClass(element, 'is-preloaded', !empty && cartridge.preload === 'live');
        setClass(element, 'is-preloading', !empty && cartridge.preload === 'loading');
        setProp(parts.plus.style, 'display', empty ? '' : 'none');
        setText(parts.name, empty ? 'Load' : cartridge.title);
        setText(parts.host, empty ? '' : host(cartridge.url));
        setProp(parts.host, 'hidden', empty);
        setProp(parts.eject, 'hidden', empty);
        setAttr(parts.body, 'aria-label', empty ? `Load cartridge ${slot + 1}` : `Play ${cartridge.title}`);
    }
}

function sameCartridges(a, b) {
    return a.every((cartridge, slot) => {
        const other = b[slot];
        return cartridge === other || (!!cartridge && !!other && cartridge.title === other.title && cartridge.url === other.url);
    });
}

/* Reordering: drag a cartridge onto another slot to swap them, or onto an empty slot to move it.
   Pointer events rather than HTML drag and drop, which Tauri takes over on Windows for file drops. */

let drag = null;
let suppressClick = false;

async function moveCartridge(from, to) {
    await invoke('cartridge_move', { from, to });
    for (const slot of [from, to]) {
        const element = ui.cartridges.children[slot];
        element.classList.remove('is-dropped');
        void element.offsetWidth;
        element.classList.add('is-dropped');
        element.addEventListener('animationend', function done(event) {
            if (event.animationName !== 'dropped') return;
            element.classList.remove('is-dropped');
            element.removeEventListener('animationend', done);
        });
    }
}

function endDrag(drop) {
    if (!drag) return;
    const { active, slot, target, ghost, element } = drag;
    drag = null;
    if (!active) return;
    ghost.remove();
    element.classList.remove('is-dragging');
    [...ui.cartridges.children].forEach((child) => child.classList.remove('is-drop-target'));
    suppressClick = true;
    if (drop && target !== null) moveCartridge(slot, target);
}

ui.cartridges.addEventListener('pointerdown', (event) => {
    suppressClick = false;
    if (event.button !== 0 || event.target.closest('.cartridge-eject')) return;
    const element = event.target.closest('.cartridge');
    const slot = [...ui.cartridges.children].indexOf(element);
    if (slot < 0 || !player.cartridges[slot]) return;
    drag = { slot, element, pointerId: event.pointerId, x: event.clientX, y: event.clientY, active: false, target: null };
});

document.addEventListener('pointermove', (event) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    if (!drag.active) {
        if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < DRAG_THRESHOLD) return;
        const rect = drag.element.getBoundingClientRect();
        drag.active = true;
        drag.offsetX = drag.x - rect.left;
        drag.offsetY = drag.y - rect.top;
        drag.ghost = drag.element.cloneNode(true);
        drag.ghost.classList.remove('is-dropped', 'is-drop-target');
        drag.ghost.classList.add('cartridge-ghost');
        drag.ghost.style.width = `${rect.width}px`;
        drag.ghost.style.height = `${rect.height}px`;
        document.body.appendChild(drag.ghost);
        drag.element.classList.add('is-dragging');
    }
    drag.ghost.style.transform = `translate(${event.clientX - drag.offsetX}px, ${event.clientY - drag.offsetY}px) scale(1.04)`;
    const over = document.elementFromPoint(event.clientX, event.clientY);
    const slot = over && over.closest('.cartridge') ? [...ui.cartridges.children].indexOf(over.closest('.cartridge')) : -1;
    drag.target = slot >= 0 && slot !== drag.slot ? slot : null;
    [...ui.cartridges.children].forEach((child, index) => child.classList.toggle('is-drop-target', index === drag.target));
});

document.addEventListener('pointerup', (event) => {
    if (drag && event.pointerId === drag.pointerId) endDrag(true);
});
document.addEventListener('pointercancel', () => endDrag(false));
document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && drag) endDrag(false);
});

// The click that ends a drag must not also play or open the cartridge.
ui.cartridges.addEventListener('click', (event) => {
    if (!suppressClick) return;
    suppressClick = false;
    event.stopPropagation();
}, true);

/* Dialog */

// What the dialog edits: { kind: 'cartridge', slot }, { kind: 'entry', id, file } or
// { kind: 'url' } (Add URL); null while it is closed.
let dialogTarget = null;
let saving = false;

function showDialog({ title, nameLabel, namePlaceholder, urlLabel, save, name, url, ejectable, fixedUrl = false }) {
    ui.dialogTitle.textContent = title;
    ui.nameLabel.textContent = nameLabel;
    ui.streamTitle.placeholder = namePlaceholder;
    ui.urlLabel.textContent = urlLabel;
    ui.saveButton.textContent = save;
    ui.streamTitle.value = name;
    ui.streamURL.value = url;
    // A local file's path is shown but not edited.
    ui.streamURL.readOnly = fixedUrl;
    ui.ejectButton.hidden = !ejectable;
    ui.dialogError.hidden = true;
    ui.dialog.hidden = false;
}

function openDialog(slot) {
    const cartridge = player.cartridges[slot];
    dialogTarget = { kind: 'cartridge', slot };
    showDialog({
        title: cartridge ? 'Edit cartridge' : 'Load cartridge',
        nameLabel: 'Name',
        namePlaceholder: 'My station',
        urlLabel: 'Stream URL',
        save: 'Save',
        name: cartridge ? cartridge.title : '',
        url: cartridge ? cartridge.url : '',
        ejectable: !!cartridge,
    });
    ui.streamTitle.focus();
}

function openEntryDialog(id) {
    const entry = entryOf(id);
    if (!entry) return;
    dialogTarget = { kind: 'entry', id, file: !!entry.path };
    showDialog({
        title: 'Edit track',
        nameLabel: 'Name',
        namePlaceholder: entry.path ? fileTitle(entry.path) : urlTitle(entry.url),
        urlLabel: entry.path ? 'File' : 'Stream or audio file URL',
        save: 'Save',
        name: entry.title,
        url: entry.path || entry.url,
        ejectable: false,
        fixedUrl: !!entry.path,
    });
    ui.streamTitle.focus();
}

function openUrlDialog() {
    dialogTarget = { kind: 'url' };
    showDialog({
        title: 'Add URL',
        nameLabel: 'Name (optional)',
        namePlaceholder: 'Taken from the address',
        urlLabel: 'Stream or audio file URL',
        save: 'Add',
        name: '',
        url: '',
        ejectable: false,
    });
    ui.streamURL.focus();
}

function closeDialog() {
    ui.dialog.hidden = true;
    dialogTarget = null;
}

// An empty name is left to Rust, which names it from the address (or the file).
function saveDialog(target, title, url) {
    if (target.kind === 'cartridge') return invoke('cartridge_save', { slot: target.slot, title, url });
    if (target.kind === 'entry') return invoke('playlist_edit', { id: target.id, title, url: target.file ? null : url });
    return invoke('playlist_add_url', { url, title });
}

// Rust checks the address: on an error the dialog stays open and shows it.
ui.dialogForm.addEventListener('submit', async (event) => {
    event.preventDefault();
    const target = dialogTarget;
    if (!target || saving) return;
    saving = true;
    try {
        const added = await saveDialog(target, ui.streamTitle.value.trim(), ui.streamURL.value.trim());
        if (dialogTarget === target) closeDialog();
        if (target.kind === 'url') selectAdded(added);
    } catch (error) {
        if (dialogTarget !== target) return;
        ui.dialogError.textContent = String(error);
        ui.dialogError.hidden = false;
        ui.streamURL.focus();
    } finally {
        saving = false;
    }
});

ui.ejectButton.addEventListener('click', () => {
    const { slot } = dialogTarget;
    closeDialog();
    invoke('cartridge_eject', { slot });
});

ui.cancelButton.addEventListener('click', closeDialog);

ui.dialog.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
        event.preventDefault();
        closeDialog();
    } else if (event.key === 'Tab') {
        const focusable = [...ui.dialog.querySelectorAll('input, button')].filter((element) => !element.hidden);
        const index = focusable.indexOf(document.activeElement);
        const next = event.shiftKey ? index - 1 : index + 1;
        if (next < 0 || next >= focusable.length) {
            event.preventDefault();
            focusable[(next + focusable.length) % focusable.length].focus();
        }
    }
});

/* Playlist: the drawer under the player. The loaded cartridges are pinned at the top (they play
   from here, but move and eject only above); below them come the files and URLs added by dropping
   them on the window, "Add file" or "Add URL". */

let drawerOpen = false;
// Selected rows (cartridges and entries). Shift extends the selection from anchor; the arrow keys
// move from focusItem.
let selection = new Set();
let anchor = null;
let focusItem = null;
// The item each row shows, in order (row.dataset.row is its index), and the row of each item.
let rowItems = [];
const rowOf = new Map();
// The active row and whether it plays, as the rows show them.
let shownActive = null;
let shownPlaying = false;
// The ids of entries just added, to select and reveal once their rows exist.
let added = null;
let rowDrag = null;
let suppressRowClick = false;
let dropRow = null;

function setEntries(list) {
    entries = list;
    entryIndex.clear();
    totalDuration = 0;
    list.forEach((entry, index) => {
        entryIndex.set(entry.id, index);
        totalDuration += entry.duration || 0;
    });
}

// One entry changed (its duration found, marked missing, edited): only its row is redrawn.
function updateEntry(entry) {
    const index = entryIndex.get(entry.id);
    if (index === undefined) return;
    totalDuration += (entry.duration || 0) - (entries[index].duration || 0);
    entries[index] = entry;
    renderRow(entry.id);
    renderRowState(entry.id);
    renderPlaylistStatus();
}

function selectedTracks() {
    return entries.filter((entry) => selection.has(entry.id)).map((entry) => entry.id);
}

// After an add: the new entries are selected and shown, in the drawer, opened if need be. Their
// rows (player-playlist) can come before or after their ids (the command's answer).
function selectAdded(ids) {
    if (!ids || !ids.length) return;
    added = ids;
    if (!drawerOpen) setDrawer(true);
    showAdded();
}

function showAdded() {
    if (!added || !added.every((id) => rowOf.has(id))) return;
    const ids = added;
    added = null;
    selection = new Set(ids);
    anchor = focusItem = ids[0];
    renderPlaylistState();
    renderPlaylistStatus();
    revealRow(ids[ids.length - 1]);
    revealRow(ids[0]);
}

// The selection moves to the entry after the removed ones (or the last one), as the list will be.
function removeEntries(ids) {
    const removing = new Set(ids);
    const first = entries.findIndex((entry) => removing.has(entry.id));
    if (first < 0) return;
    const remaining = entries.filter((entry) => !removing.has(entry.id));
    const next = remaining[Math.min(first, remaining.length - 1)];
    selection = new Set(next ? [next.id] : []);
    anchor = focusItem = next ? next.id : null;
    invoke('playlist_remove', { ids });
}

// Moves the selected entries before `index` (an index in the playlist as it is).
function moveSelection(index) {
    const ids = selectedTracks();
    if (ids.length) invoke('playlist_move', { ids, index });
}

function renderPlaylist() {
    if (!player) return;
    const list = ui.playlist;
    const scroll = list.scrollTop;
    list.textContent = '';
    rowItems = [];
    rowOf.clear();
    dropRow = null;
    const fragment = document.createDocumentFragment();
    player.cartridges.forEach((cartridge, slot) => {
        if (cartridge) fragment.appendChild(makeRow(cartridgeKey(slot), `Cartridge ${slot + 1}`));
    });
    if (rowItems.length) {
        const divider = document.createElement('div');
        divider.className = 'playlist-divider';
        fragment.appendChild(divider);
    }
    entries.forEach((entry, index) => fragment.appendChild(makeRow(entry.id, index + 1)));
    if (!entries.length) {
        const empty = document.createElement('div');
        empty.className = 'playlist-empty';
        empty.innerHTML = '<b>Drop music files or folders here</b><span>or use Add file and Add URL. Files play from where they are.</span>';
        fragment.appendChild(empty);
    }
    list.appendChild(fragment);
    list.scrollTop = scroll;
    for (const item of selection) {
        if (!rowOf.has(item)) selection.delete(item);
    }
    renderPlaylistStatus();
    renderPlaylistState();
    showAdded();
}

// `number` is the track number, or a cartridge's label (shown as a radio icon).
function makeRow(item, number) {
    const row = document.createElement('div');
    const cartridge = typeof number === 'string';
    row.className = cartridge ? 'row is-cartridge' : 'row';
    row.setAttribute('role', 'option');
    row.dataset.row = rowItems.length;
    row.innerHTML = `<span class="row-number"><span class="row-eq" aria-hidden="true"><i></i><i></i><i></i></span><span class="row-index"></span></span><span class="row-title"></span><span class="row-detail"></span>`;
    const index = row.querySelector('.row-index');
    if (cartridge) {
        index.innerHTML = '<svg><use href="#i-radio" /></svg>';
        index.title = number;
    } else {
        index.textContent = number;
    }
    rowItems.push(item);
    rowOf.set(item, row);
    renderRow(item);
    return row;
}

function renderRow(item) {
    const row = rowOf.get(item);
    if (!row) return;
    let title;
    let detail;
    if (isTrack(item)) {
        const entry = entryOf(item);
        title = entry.title;
        detail = entry.duration ? formatTime(entry.duration) : entry.url ? host(entry.url) : '';
        row.title = entry.path || entry.url;
    } else {
        const cartridge = player.cartridges[slotOf(item)];
        title = cartridge.title;
        detail = host(cartridge.url);
        row.title = cartridge.url;
    }
    row.querySelector('.row-title').textContent = title;
    row.querySelector('.row-detail').textContent = detail;
}

// Row classes. A selection change goes over every row; a state update only over the rows whose
// active or playing status changed, since the state comes four times a second and a playlist can
// hold thousands of entries.
function renderPlaylistState() {
    shownActive = activeItem();
    shownPlaying = isPlaying();
    rowOf.forEach((row, item) => renderRowState(item, row));
}

function renderActiveRows() {
    const item = activeItem();
    const playing = isPlaying();
    if (item === shownActive && playing === shownPlaying) return;
    const previous = shownActive;
    shownActive = item;
    shownPlaying = playing;
    renderRowState(previous);
    renderRowState(item);
}

function renderRowState(item, row = rowOf.get(item)) {
    if (!row) return;
    const isActive = item === shownActive;
    const selected = selection.has(item);
    setClass(row, 'is-active', isActive);
    setClass(row, 'is-playing', isActive && shownPlaying);
    setClass(row, 'is-selected', selected);
    setClass(row, 'is-missing', isTrack(item) && !!entryOf(item).missing);
    setAttr(row, 'aria-selected', selected);
}

function renderPlaylistStatus() {
    const parts = [entries.length === 1 ? '1 track' : `${entries.length} tracks`];
    if (totalDuration) parts.push(formatTime(totalDuration));
    if (selection.size > 1) parts.push(`${selection.size} selected`);
    setText(ui.playlistCount, parts.join(' · '));
}

// Scrolls the list (only the list: the page itself never scrolls) to show an item's row.
function revealRow(item) {
    const row = rowOf.get(item);
    if (!row) return;
    const list = ui.playlist;
    if (row.offsetTop < list.scrollTop) list.scrollTop = row.offsetTop - 4;
    else if (row.offsetTop + row.offsetHeight > list.scrollTop + list.clientHeight) list.scrollTop = row.offsetTop + row.offsetHeight - list.clientHeight + 4;
}

function itemOf(target) {
    const row = target.closest('.row');
    return row ? rowItems[Number(row.dataset.row)] : null;
}

function rowIndex(item) {
    const row = rowOf.get(item);
    return row ? Number(row.dataset.row) : -1;
}

function select(item, { toggle = false, extend = false } = {}) {
    if (extend && anchor !== null && rowOf.has(anchor)) {
        const from = rowIndex(anchor);
        const to = rowIndex(item);
        selection = new Set(rowItems.slice(Math.min(from, to), Math.max(from, to) + 1));
    } else if (toggle) {
        if (selection.has(item)) selection.delete(item);
        else selection.add(item);
        anchor = item;
    } else {
        selection = new Set([item]);
        anchor = item;
    }
    focusItem = item;
    renderPlaylistState();
    renderPlaylistStatus();
}

ui.playlist.addEventListener('click', (event) => {
    if (suppressRowClick) {
        suppressRowClick = false;
        return;
    }
    const item = itemOf(event.target);
    if (item !== null) {
        select(item, { toggle: event.ctrlKey || event.metaKey, extend: event.shiftKey });
    } else if (!event.ctrlKey && !event.shiftKey) {
        selection.clear();
        renderPlaylistState();
        renderPlaylistStatus();
    }
});

ui.playlist.addEventListener('dblclick', (event) => {
    const item = itemOf(event.target);
    if (item !== null) playItem(item);
});

ui.playlist.addEventListener('keydown', (event) => {
    const index = focusItem !== null ? rowIndex(focusItem) : -1;
    const moves = { ArrowUp: -1, ArrowDown: 1, PageUp: -10, PageDown: 10 };
    let to = null;
    if (event.key in moves) to = index < 0 ? 0 : index + moves[event.key];
    else if (event.key === 'Home') to = 0;
    else if (event.key === 'End') to = rowItems.length - 1;
    if (to !== null) {
        const item = rowItems[Math.max(0, Math.min(rowItems.length - 1, to))];
        if (item !== undefined) {
            select(item, { extend: event.shiftKey });
            revealRow(item);
        }
    } else if (event.key === 'Enter' && focusItem !== null && rowOf.has(focusItem)) {
        playItem(focusItem);
    } else if (event.key === 'Delete' || event.key === 'Backspace') {
        removeEntries(selectedTracks());
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
        selection = new Set(entries.map((entry) => entry.id));
        renderPlaylistState();
        renderPlaylistStatus();
    } else {
        return;
    }
    event.preventDefault();
});

// Where a drop at (x, y) goes in the playlist: at the nearest boundary between tracks under it,
// the start when over the cartridges, the end anywhere else. Rows all have the same height.
function dropIndex(x, y) {
    const rect = ui.playlist.getBoundingClientRect();
    if (!drawerOpen || !entries.length || x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return entries.length;
    const first = rowOf.get(entries[0].id);
    const offset = y - rect.top + ui.playlist.scrollTop - first.offsetTop;
    return Math.max(0, Math.min(entries.length, Math.round(offset / first.offsetHeight)));
}

// The insertion line for a drop (null hides it).
function showDropIndex(index) {
    if (dropRow) dropRow.classList.remove('drop-before', 'drop-after');
    dropRow = null;
    ui.playlist.classList.toggle('is-drop-empty', index !== null && !entries.length);
    if (index === null || !entries.length) return;
    dropRow = rowOf.get(entries[Math.min(index, entries.length - 1)].id);
    dropRow.classList.add(index < entries.length ? 'drop-before' : 'drop-after');
}

function autoscroll(y) {
    const rect = ui.playlist.getBoundingClientRect();
    if (y < rect.top + 24) ui.playlist.scrollTop -= 12;
    else if (y > rect.bottom - 24) ui.playlist.scrollTop += 12;
}

// Reordering tracks: drag the selection (pointer events, like the cartridges).
ui.playlist.addEventListener('pointerdown', (event) => {
    suppressRowClick = false;
    if (event.button !== 0 || event.ctrlKey || event.shiftKey || event.metaKey) return;
    const item = itemOf(event.target);
    if (!isTrack(item)) return;
    if (!selection.has(item)) select(item);
    rowDrag = { pointerId: event.pointerId, y: event.clientY, active: false, index: null };
});

document.addEventListener('pointermove', (event) => {
    if (!rowDrag || event.pointerId !== rowDrag.pointerId) return;
    if (!rowDrag.active) {
        if (Math.abs(event.clientY - rowDrag.y) < DRAG_THRESHOLD) return;
        rowDrag.active = true;
        ui.playlist.classList.add('is-reordering');
    }
    autoscroll(event.clientY);
    rowDrag.index = dropIndex(event.clientX, event.clientY);
    showDropIndex(rowDrag.index);
});

function endRowDrag(drop) {
    if (!rowDrag) return;
    const { active: dragging, index } = rowDrag;
    rowDrag = null;
    if (!dragging) return;
    ui.playlist.classList.remove('is-reordering');
    showDropIndex(null);
    suppressRowClick = true;
    if (drop && index !== null) moveSelection(index);
}

document.addEventListener('pointerup', (event) => {
    if (rowDrag && event.pointerId === rowDrag.pointerId) endRowDrag(true);
});
document.addEventListener('pointercancel', () => endRowDrag(false));
document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && rowDrag) endRowDrag(false);
});

// Files and folders dropped on the window (Tauri hands over their paths; Rust expands folders and
// playlists). Hovering opens the drawer and shows where they'll go.
window.__TAURI__.webview.getCurrentWebview().onDragDropEvent(async ({ payload }) => {
    if (payload.type === 'leave' || !ui.dialog.hidden) {
        document.body.classList.remove('is-dropping');
        showDropIndex(null);
        return;
    }
    const x = payload.position.x / devicePixelRatio;
    const y = payload.position.y / devicePixelRatio;
    if (payload.type === 'drop') {
        document.body.classList.remove('is-dropping');
        const index = dropIndex(x, y);
        showDropIndex(null);
        selectAdded(await invoke('playlist_drop', { paths: payload.paths, index }));
        return;
    }
    if (!drawerOpen) setDrawer(true);
    document.body.classList.add('is-dropping');
    showDropIndex(dropIndex(x, y));
});

// The window grows to show the drawer (playlist.rs), and remembers it for next time.
function setDrawer(open, animate = true) {
    drawerOpen = open;
    ui.playlistButton.setAttribute('aria-expanded', String(open));
    ui.drawer.inert = !open;
    if (animate) invoke('set_playlist_open', { open });
}

async function addFiles() {
    selectAdded(await invoke('playlist_add_files'));
}

ui.playlistButton.addEventListener('click', () => setDrawer(!drawerOpen));
ui.addFileButton.addEventListener('click', addFiles);
ui.addUrlButton.addEventListener('click', openUrlDialog);

// The page is taller than the window while the drawer is closed; focusing or revealing something
// in it must not scroll the page.
for (const scroller of [window, document.body]) {
    scroller.addEventListener('scroll', () => {
        document.documentElement.scrollTop = 0;
        document.body.scrollTop = 0;
    });
}

/* Sliders */

// onRender shows every value, the user's or the player's; onInput (optional) gets only the user's
// changes, and onCommit (optional) the value the user settles on: at the end of a drag, or after a
// key or wheel step. Values set from the player never reach onInput or onCommit.
function makeSlider(element, { min, max, value, center, onRender, onInput, onCommit }) {
    const rail = element.querySelector('.slider-rail');
    const span = max - min;
    let current = null;
    let dragging = false;

    const clamp = (next) => Math.round(Math.min(max, Math.max(min, next)));

    function show(next) {
        current = next;
        const ratio = (next - min) / span;
        const origin = center === undefined ? 0 : (center - min) / span;
        element.style.setProperty('--value', ratio);
        element.style.setProperty('--from', Math.min(origin, ratio));
        element.style.setProperty('--to', Math.max(origin, ratio));
        element.setAttribute('aria-valuenow', next);
        onRender(next);
    }

    function set(next, snap) {
        next = clamp(next);
        if (snap && center !== undefined && Math.abs(next - center) <= span * 0.04) next = center;
        if (next === current) return;
        show(next);
        if (onInput) onInput(next);
    }

    function fromPointer(event) {
        const rect = rail.getBoundingClientRect();
        set(min + ((event.clientX - rect.left) / rect.width) * span, true);
    }

    const disabled = () => element.getAttribute('aria-disabled') === 'true';

    function commit() {
        if (onCommit) onCommit(current);
    }

    function endDrag() {
        if (dragging) commit();
        dragging = false;
        element.classList.remove('is-dragging');
    }

    element.addEventListener('pointerdown', (event) => {
        if (event.button !== 0 || disabled()) return;
        dragging = true;
        element.setPointerCapture(event.pointerId);
        element.classList.add('is-dragging');
        fromPointer(event);
    });
    element.addEventListener('pointermove', (event) => {
        if (dragging) fromPointer(event);
    });
    element.addEventListener('pointerup', endDrag);
    element.addEventListener('pointercancel', endDrag);
    element.addEventListener('lostpointercapture', endDrag);
    element.addEventListener('dblclick', () => set(center !== undefined ? center : current, false));
    element.addEventListener('wheel', (event) => {
        event.preventDefault();
        if (disabled()) return;
        set(current + (event.deltaY < 0 ? 1 : -1) * span * 0.05, false);
        commit();
    }, { passive: false });
    element.addEventListener('keydown', (event) => {
        if (disabled()) return;
        const steps = { ArrowRight: 0.05, ArrowUp: 0.05, ArrowLeft: -0.05, ArrowDown: -0.05, PageUp: 0.1, PageDown: -0.1 };
        if (event.key in steps) set(current + steps[event.key] * span, false);
        else if (event.key === 'Home') set(min, false);
        else if (event.key === 'End') set(max, false);
        else return;
        commit();
        event.preventDefault();
    });

    show(clamp(value));
    // From the player: not while the user drags it, and only when the value differs.
    return {
        set(next) {
            next = clamp(next);
            if (!dragging && next !== current) show(next);
        },
    };
}

const volume = makeSlider(document.getElementById('volumeSlider'), {
    min: 0,
    max: 100,
    value: 50,
    onRender(value) {
        ui.volumeValue.textContent = value + '%';
        ui.volumeIcon.setAttribute('href', value === 0 ? '#i-vol-0' : value < 50 ? '#i-vol-1' : '#i-vol-2');
    },
    onInput(value) {
        invoke('player_set_volume', { value });
    },
});

const balance = makeSlider(document.getElementById('balanceSlider'), {
    min: -100,
    max: 100,
    value: 0,
    center: 0,
    onRender(value) {
        ui.balanceValue.textContent = value === 0 ? 'C' : (value < 0 ? 'L ' : 'R ') + Math.abs(value);
    },
    onInput(value) {
        invoke('player_set_balance', { value });
    },
});

/* Seek bar: shown for a playlist file with a known duration (local, or online with a length), in
   place of the stacked volume and balance. */

// The bar's value (the user's while dragging) and the duration it is shown against.
let seekValue = 0;
let seekDuration = null;

function renderSeekTime() {
    if (!seekDuration) return;
    const html = `<b>${formatTime(seekValue / SEEK_STEPS * seekDuration)}</b> / ${formatTime(seekDuration)}`;
    if (ui.seekTime.innerHTML !== html) ui.seekTime.innerHTML = html;
}

const seek = makeSlider(ui.seekSlider, {
    min: 0,
    max: SEEK_STEPS,
    value: 0,
    onRender(value) {
        seekValue = value;
        renderSeekTime();
    },
    onCommit(value) {
        if (player && player.seekable && player.duration) invoke('player_seek', { seconds: value / SEEK_STEPS * player.duration });
    },
});

function renderSeek() {
    const { duration, position, seekable } = player;
    setClass(ui.sliders, 'is-seekable', !!duration);
    seekDuration = duration || null;
    if (!duration) return;
    setAttr(ui.seekSlider, 'aria-disabled', !seekable);
    setProp(ui.seekSlider, 'tabIndex', seekable ? 0 : -1);
    seek.set(seekable && position !== null ? Math.min(1, position / duration) * SEEK_STEPS : 0);
    renderSeekTime();
}

/* Panels, settings and updates */

function openSettings() {
    closeDialog();
    invoke('open_panel', { name: 'settings' });
}

function openInfo() {
    closeDialog();
    invoke('open_panel', { name: 'info' });
}

ui.settingsButton.addEventListener('click', openSettings);
ui.infoButton.addEventListener('click', openInfo);

// Clicking the analyzer cycles its colors, "off" included (also in Settings). The new value comes
// back in the state.
ui.screen.addEventListener('click', () => invoke('player_cycle_colors'));

// A dot on the cog when a downloaded update is waiting (installed from Settings).
function showUpdate(update) {
    const ready = update.status === 'ready';
    ui.settingsButton.classList.toggle('has-update', ready);
    ui.settingsButton.title = ready ? `Settings: version ${update.version} is ready to install` : 'Settings';
}

listen('update-status', ({ payload }) => showUpdate(payload));
invoke('update_status').then(showUpdate);

function shortcutLabel(action) {
    const binding = shortcuts[action];
    return binding && binding.registered ? shortcutKeys(binding.accelerator).join('+') : null;
}

function applyShortcuts() {
    const play = shortcutLabel('play');
    const stop = shortcutLabel('stop');
    ui.playButton.title = play ? `Play (${play})` : 'Play';
    ui.stopButton.title = stop ? `Stop (${stop})` : 'Stop';
}

listen('settings-changed', ({ payload }) => {
    if (payload.key !== 'shortcuts') return;
    shortcuts = payload.value;
    applyShortcuts();
});

invoke('get_shortcuts').then((value) => {
    shortcuts = value;
    applyShortcuts();
});

/* Right-click menu */

document.addEventListener('contextmenu', async (event) => {
    if (!ui.dialog.hidden || !player) return;

    const element = event.target.closest('.cartridge');
    const slot = element ? [...ui.cartridges.children].indexOf(element) : -1;
    const items = [];
    if (event.target.closest('#drawer')) {
        const item = itemOf(event.target);
        if (item !== null && !selection.has(item)) select(item);
        const tracks = selectedTracks();
        if (item !== null && !isTrack(item)) {
            items.push(
                { text: 'Play', action: () => playItem(item) },
                { text: 'Edit cartridge…', action: () => openDialog(slotOf(item)) },
                { item: 'Separator' },
            );
        } else if (item !== null) {
            items.push(
                { text: 'Play', action: () => playItem(item) },
                { text: 'Edit…', action: () => openEntryDialog(item) },
                { text: tracks.length > 1 ? `Remove ${tracks.length} tracks` : 'Remove', action: () => removeEntries(tracks) },
                { item: 'Separator' },
            );
        }
        items.push(
            { text: 'Add file…', action: addFiles },
            { text: 'Add URL…', action: openUrlDialog },
            { text: 'Clear playlist', enabled: entries.length > 0, action: () => removeEntries(entries.map((entry) => entry.id)) },
        );
    } else if (slot >= 0 && player.cartridges[slot]) {
        items.push(
            { text: 'Play', action: () => invoke('cartridge_play', { slot }) },
            { text: 'Edit…', action: () => openDialog(slot) },
            { text: 'Eject', action: () => invoke('cartridge_eject', { slot }) },
        );
    } else if (slot >= 0) {
        items.push({ text: 'Load cartridge…', action: () => openDialog(slot) });
    } else {
        items.push(
            isPlaying()
                ? { text: 'Pause', action: () => invoke('player_pause') }
                : { text: 'Play', enabled: activeItem() !== null, accelerator: shortcutLabel('play'), action: () => invoke('player_play') },
            { text: 'Stop', enabled: player.state !== 'stopped', accelerator: shortcutLabel('stop'), action: () => invoke('player_stop') },
        );
    }
    items.forEach((item) => { if (!item.accelerator) delete item.accelerator; });
    if (items.length) items.push({ item: 'Separator' });
    items.push({ text: 'Stream information', action: openInfo }, { text: 'Settings', action: openSettings });

    let menu;
    try {
        menu = await Menu.new({ items });
    } catch (error) {
        // A shortcut label the menu can't parse: show the menu without them.
        items.forEach((item) => delete item.accelerator);
        menu = await Menu.new({ items });
    }
    await popupMenu(menu);
});

/* Buttons */

ui.playButton.addEventListener('click', () => invoke('player_play'));
ui.pauseButton.addEventListener('click', () => invoke('player_pause'));
ui.stopButton.addEventListener('click', () => invoke('player_stop'));
ui.instantButton.addEventListener('click', () => {
    if (player) invoke('player_set_instant', { on: !player.instant });
});

/* Startup */

// Player events that come before player_attach has answered are newer than its snapshot (Rust
// sends them only once the page has attached), so they are applied after it.
let early = [];

function follow(name, handler) {
    return listen(name, ({ payload }) => {
        if (early) early.push(() => handler(payload));
        else handler(payload);
    });
}

(async function init() {
    await Promise.all([
        follow('player-state', render),
        follow('player-playlist', (payload) => {
            setEntries(payload.entries);
            renderPlaylist();
        }),
        follow('player-entry', updateEntry),
    ]);
    const frames = new Channel();
    frames.onmessage = (frame) => spectrum.paint(frame);
    const [open, snapshot] = await Promise.all([loadData('playlist-open'), invoke('player_attach', { frames })]);
    // The window was built at the right height already (playlist.rs).
    setDrawer(open === true, false);
    setEntries(snapshot.playlist.entries);
    render(snapshot.state);
    early.forEach((apply) => apply());
    early = null;
})();
