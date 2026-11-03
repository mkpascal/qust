const SLOTS = 6;
const LIVE_RESUME_SECONDS = 10;
const INFO_INTERVAL = 2000;
const STATUS_INTERVAL = 15000;

const spectrum = new Spectrum(document.getElementById('canvas'));

const ui = {
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
    playButton: document.getElementById('playButton'),
    pauseButton: document.getElementById('pauseButton'),
    stopButton: document.getElementById('stopButton'),
    cartridges: document.getElementById('cartridges'),
    dialog: document.getElementById('dialog'),
    dialogForm: document.getElementById('dialogForm'),
    dialogTitle: document.getElementById('dialogTitle'),
    dialogError: document.getElementById('dialogError'),
    streamTitle: document.getElementById('streamTitle'),
    streamURL: document.getElementById('streamURL'),
    ejectButton: document.getElementById('ejectButton'),
    cancelButton: document.getElementById('cancelButton'),
    settingsButton: document.getElementById('settingsButton'),
    infoButton: document.getElementById('infoButton'),
    instantButton: document.getElementById('instantButton'),
};

// cartridges[i] = { title, url } or null, for slots player1..player6
let cartridges = new Array(SLOTS).fill(null);
let activeSlot = null;
// stopped | connecting | playing | buffering | paused
let state = 'stopped';
let pausedAt = 0;
let infoTimer = null;
let lastStatusFetch = 0;
let streamInfo = {};
let statusTitle = null;
// times playback stalled (the "waiting" event) since the stream started
let stalls = 0;
let dialogSlot = null;
let settings = { colors: 'classic', autoplay: false };
// Instant switch (the lightning button): every loaded cartridge streams, muted but the active one.
let instant = false;
// One <audio> per stream URL (several in instant mode), all routed through the visualizer's graph.
const streams = new Map();
// The URL that was paused, and the active stream's time when it became active (for the clock).
let pausedUrl = null;
let clockStart = 0;

/* Streams */

function current() {
    const cartridge = cartridges[activeSlot];
    return cartridge ? streams.get(cartridge.url) || null : null;
}

function streamFor(url) {
    if (streams.has(url)) return streams.get(url);
    const element = new Audio();
    element.crossOrigin = 'anonymous';
    element.preload = 'none';
    element.addEventListener('playing', () => {
        if (element === current() && state !== 'stopped' && state !== 'paused') setState('playing');
        else renderCartridges();
    });
    element.addEventListener('waiting', () => {
        if (element !== current() || state !== 'playing') return renderCartridges();
        stalls++;
        setState('buffering');
    });
    element.addEventListener('error', () => {
        if (element !== current() || state === 'stopped') return renderCartridges();
        stop();
        setTitle("Couldn't play this stream", true);
    });
    spectrum.attach(element);
    streams.set(url, element);
    return element;
}

function unload(url) {
    const element = streams.get(url);
    element.pause();
    element.removeAttribute('src');
    element.load();
    spectrum.detach(element);
    streams.delete(url);
}

// Already playing and ahead of the buffer: switching to it is instant.
function isLive(element) {
    return !!element.getAttribute('src') && !element.paused && !element.error && element.readyState >= 3;
}

// Keeps the right streams connected: the active one while playing, plus every other loaded
// cartridge in instant mode (muted). Everything else is disconnected.
function syncStreams() {
    const active = cartridges[activeSlot] ? cartridges[activeSlot].url : null;
    const wanted = new Set(instant && state !== 'stopped' ? cartridges.filter(Boolean).map((cartridge) => cartridge.url) : []);
    if (active && state !== 'stopped') wanted.add(active);
    for (const url of [...streams.keys()]) {
        if (!wanted.has(url)) unload(url);
    }
    for (const url of wanted) {
        if (url === active) continue;
        const element = streamFor(url);
        if (!element.getAttribute('src') || element.error) element.src = proxied(url);
        if (element.paused) element.play().catch(() => {});
    }
    streams.forEach((element, url) => spectrum.setLevel(element, url === active ? 1 : 0));
    renderCartridges();
}

function setInstant(on) {
    instant = on;
    ui.instantButton.setAttribute('aria-pressed', String(on));
    ui.instantButton.title = on
        ? 'Instant switch is on: every cartridge streams in the background, so switching is immediate (uses more data)'
        : 'Instant switch: keep every cartridge streaming in the background for immediate switching (uses more data)';
    syncStreams();
}

/* Playback */

function play() {
    const cartridge = cartridges[activeSlot];
    if (!cartridge) return;
    spectrum.start();
    const element = streamFor(cartridge.url);
    const resume = state === 'paused' && pausedUrl === cartridge.url && (Date.now() - pausedAt) / 1000 < LIVE_RESUME_SECONDS;
    const live = !resume && isLive(element);
    if (!resume) {
        stalls = 0;
        streamInfo = {};
        statusTitle = null;
        lastStatusFetch = 0;
        clockStart = live ? element.currentTime : 0;
        if (!live) element.src = proxied(cartridge.url);
    }
    setState(live ? 'playing' : resume ? 'buffering' : 'connecting');
    element.play().catch(() => {});
    startInfo();
    syncStreams();
}

function pause() {
    if (state === 'stopped' || state === 'paused') return;
    const element = current();
    if (element) element.pause();
    pausedUrl = cartridges[activeSlot] ? cartridges[activeSlot].url : null;
    pausedAt = Date.now();
    setState('paused');
}

function stop() {
    setState('stopped');
    syncStreams();
    spectrum.stop();
    stopInfo();
    streamInfo = {};
    statusTitle = null;
    renderInfo();
}

function setState(next) {
    state = next;
    const icon = { playing: 'play', buffering: 'play', connecting: 'play', paused: 'pause' }[state] || 'stop';
    ui.stateIcon.setAttribute('href', '#i-' + icon);
    ui.clock.classList.toggle('is-idle', state === 'stopped');
    spectrum.setLoading(state === 'connecting' || state === 'buffering');
    ui.playButton.disabled = activeSlot === null || state === 'playing' || state === 'buffering' || state === 'connecting';
    ui.pauseButton.disabled = state === 'stopped' || state === 'paused';
    ui.stopButton.disabled = state === 'stopped';
    renderCartridges();
    renderTitle();
    renderClock();
    emitStatus();
}

/* Stream details: ICY title and format from the proxy, Icecast status page as a title fallback */

function startInfo() {
    stopInfo();
    pollInfo();
    infoTimer = setInterval(pollInfo, INFO_INTERVAL);
}

function stopInfo() {
    clearInterval(infoTimer);
    infoTimer = null;
}

async function pollInfo() {
    const cartridge = cartridges[activeSlot];
    if (!cartridge) return;
    try {
        streamInfo = await (await fetch(proxiedInfo(cartridge.url))).json();
    } catch (error) {
        streamInfo = {};
    }
    if (!streamInfo.title && Date.now() - lastStatusFetch > STATUS_INTERVAL) {
        lastStatusFetch = Date.now();
        statusTitle = await getMetaDataFromStream(cartridge.url);
    }
    renderInfo();
}

function renderInfo() {
    const info = state === 'stopped' ? {} : streamInfo;
    ui.kbps.textContent = info.bitrate || '--';
    ui.khz.textContent = info.samplerate ? formatKhz(info.samplerate) : '--';
    const channels = state === 'stopped' ? null : info.channels || spectrum.channels();
    ui.mono.classList.toggle('is-on', channels === 1);
    ui.stereo.classList.toggle('is-on', channels >= 2);
    renderTitle();
}

function formatKhz(samplerate) {
    const khz = samplerate / 1000;
    return Number.isInteger(khz) ? String(khz) : khz.toFixed(1);
}

async function getMetaDataFromStream(streamUrl) {
    try {
        const statusUrl = new URL(streamUrl);
        statusUrl.pathname = '/status-json.xsl';
        const data = await (await fetch(proxied(statusUrl.href))).json();
        const sources = [].concat(data.icestats.source || []);
        if (streamUrl.endsWith('/')) {
            return (sources[0] && sources[0].title) || null;
        }
        const streamPath = new URL(streamUrl).pathname;
        const source = sources.find((s) => s.listenurl && new URL(s.listenurl).pathname === streamPath);
        return (source && source.title) || null;
    } catch (error) {
        return null;
    }
}

/* Display */

function renderTitle() {
    const cartridge = cartridges[activeSlot];
    ui.station.textContent = cartridge ? cartridge.title : 'No cartridge loaded';
    if (!cartridge) {
        setTitle('Load a cartridge to start', true);
    } else if (state === 'stopped') {
        setTitle('Stopped', true);
    } else if (state === 'connecting') {
        setTitle('Connecting…', true);
    } else {
        const title = streamInfo.title || statusTitle || streamInfo.name || cartridge.title;
        setTitle(state === 'paused' ? title + ' (paused)' : title, false);
    }
}

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

function renderClock() {
    const element = current();
    const seconds = state === 'stopped' || !element ? 0 : Math.max(0, Math.floor((element.currentTime || 0) - clockStart));
    const h = Math.floor(seconds / 3600);
    const m = Math.floor(seconds / 60) % 60;
    const s = String(seconds % 60).padStart(2, '0');
    ui.clockText.textContent = h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${String(m).padStart(2, '0')}:${s}`;
}

setInterval(renderClock, 250);

/* Cartridges */

function renderCartridges() {
    if (ui.cartridges.children.length !== SLOTS) {
        ui.cartridges.innerHTML = '';
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
            element.querySelector('.cartridge-body').addEventListener('click', () => {
                if (cartridges[slot]) selectSlot(slot);
                else openDialog(slot);
            });
            element.querySelector('.cartridge-eject').addEventListener('click', () => openDialog(slot));
            ui.cartridges.appendChild(element);
        }
    }

    cartridges.forEach((cartridge, slot) => {
        const element = ui.cartridges.children[slot];
        const empty = !cartridge;
        element.classList.toggle('is-empty', empty);
        element.classList.toggle('is-active', !empty && slot === activeSlot);
        element.classList.toggle('is-playing', !empty && slot === activeSlot && state !== 'stopped' && state !== 'paused');
        // Instant switch: a bolt on the other cartridges, faint while connecting, solid once live.
        const stream = !empty && instant && slot !== activeSlot && state !== 'stopped' ? streams.get(cartridge.url) : null;
        element.classList.toggle('is-preloaded', !!stream && isLive(stream));
        element.classList.toggle('is-preloading', !!stream && !isLive(stream));
        element.querySelector('.cartridge-plus').style.display = empty ? '' : 'none';
        element.querySelector('.cartridge-name').textContent = empty ? 'Load' : cartridge.title;
        element.querySelector('.cartridge-host').textContent = empty ? '' : host(cartridge.url);
        element.querySelector('.cartridge-host').hidden = empty;
        element.querySelector('.cartridge-eject').hidden = empty;
        element.querySelector('.cartridge-body').setAttribute('aria-label', empty ? `Load cartridge ${slot + 1}` : `Play ${cartridge.title}`);
    });
}

function host(url) {
    try {
        return new URL(url).host;
    } catch (error) {
        return url;
    }
}

function selectSlot(slot) {
    const cartridge = cartridges[slot];
    if (!instant && state !== 'stopped') stop();
    activeSlot = slot;
    saveData('last-selected-player', 'player' + (slot + 1));
    saveData('last-selected-title', cartridge.title);
    saveData('last-selected-url', cartridge.url);
    play();
}

/* Reordering: drag a cartridge onto another slot to swap them, or onto an empty slot to move it.
   Pointer events rather than HTML drag and drop, which Tauri takes over on Windows for file drops. */

const DRAG_THRESHOLD = 5;
let drag = null;
let suppressClick = false;

function saveSlot(slot) {
    const cartridge = cartridges[slot];
    if (cartridge) {
        saveData(`player${slot + 1}-title`, cartridge.title);
        saveData(`player${slot + 1}-url`, cartridge.url);
    } else {
        deleteData(`player${slot + 1}-title`);
        deleteData(`player${slot + 1}-url`);
    }
}

function moveCartridge(from, to) {
    [cartridges[from], cartridges[to]] = [cartridges[to], cartridges[from]];
    saveSlot(from);
    saveSlot(to);
    if (activeSlot === from) activeSlot = to;
    else if (activeSlot === to) activeSlot = from;
    if (activeSlot !== null) saveData('last-selected-player', 'player' + (activeSlot + 1));
    renderCartridges();
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
    if (slot < 0 || !cartridges[slot]) return;
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

function openDialog(slot) {
    dialogSlot = slot;
    const cartridge = cartridges[slot];
    ui.dialogTitle.textContent = cartridge ? 'Edit cartridge' : 'Load cartridge';
    ui.streamTitle.value = cartridge ? cartridge.title : '';
    ui.streamURL.value = cartridge ? cartridge.url : '';
    ui.ejectButton.hidden = !cartridge;
    ui.dialogError.hidden = true;
    ui.dialog.hidden = false;
    ui.streamTitle.focus();
}

function closeDialog() {
    ui.dialog.hidden = true;
    dialogSlot = null;
}

ui.dialogForm.addEventListener('submit', (event) => {
    event.preventDefault();
    const url = ui.streamURL.value.trim();
    let valid = false;
    try {
        valid = ['http:', 'https:'].includes(new URL(url).protocol);
    } catch (error) {}
    if (!valid) {
        ui.dialogError.textContent = 'Enter a stream address starting with http:// or https://';
        ui.dialogError.hidden = false;
        ui.streamURL.focus();
        return;
    }

    const slot = dialogSlot;
    const previous = cartridges[slot];
    const title = ui.streamTitle.value.trim() || host(url);
    cartridges[slot] = { title, url };
    saveData(`player${slot + 1}-title`, title);
    saveData(`player${slot + 1}-url`, url);
    closeDialog();

    if (slot === activeSlot) {
        saveData('last-selected-title', title);
        saveData('last-selected-url', url);
        if (previous && previous.url !== url && state !== 'stopped') {
            stop();
            play();
        }
    }
    setState(state);
    syncStreams();
});

function eject(slot) {
    cartridges[slot] = null;
    deleteData(`player${slot + 1}-title`);
    deleteData(`player${slot + 1}-url`);
    if (slot === activeSlot) {
        stop();
        activeSlot = null;
        deleteData('last-selected-player');
        deleteData('last-selected-title');
        deleteData('last-selected-url');
    }
    setState(state);
    syncStreams();
}

ui.ejectButton.addEventListener('click', () => {
    const slot = dialogSlot;
    closeDialog();
    eject(slot);
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

/* Sliders */

function makeSlider(element, { min, max, value, center, onInput }) {
    const rail = element.querySelector('.slider-rail');
    const span = max - min;
    let current = value;
    let dragging = false;

    function set(next, snap) {
        next = Math.round(Math.min(max, Math.max(min, next)));
        if (snap && center !== undefined && Math.abs(next - center) <= span * 0.04) next = center;
        current = next;
        const ratio = (next - min) / span;
        const origin = center === undefined ? 0 : (center - min) / span;
        element.style.setProperty('--value', ratio);
        element.style.setProperty('--from', Math.min(origin, ratio));
        element.style.setProperty('--to', Math.max(origin, ratio));
        element.setAttribute('aria-valuenow', next);
        onInput(next);
    }

    function fromPointer(event) {
        const rect = rail.getBoundingClientRect();
        set(min + ((event.clientX - rect.left) / rect.width) * span, true);
    }

    function endDrag() {
        dragging = false;
        element.classList.remove('is-dragging');
    }

    element.addEventListener('pointerdown', (event) => {
        if (event.button !== 0) return;
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
        set(current + (event.deltaY < 0 ? 1 : -1) * span * 0.05, false);
    }, { passive: false });
    element.addEventListener('keydown', (event) => {
        const steps = { ArrowRight: 0.05, ArrowUp: 0.05, ArrowLeft: -0.05, ArrowDown: -0.05, PageUp: 0.1, PageDown: -0.1 };
        if (event.key in steps) set(current + steps[event.key] * span, false);
        else if (event.key === 'Home') set(min, false);
        else if (event.key === 'End') set(max, false);
        else return;
        event.preventDefault();
    });

    set(value, false);
    return { set: (next) => set(next, false) };
}

function debounce(fn, delay) {
    let timer;
    return (...args) => {
        clearTimeout(timer);
        timer = setTimeout(() => fn(...args), delay);
    };
}

const saveVolume = debounce((value) => saveData('volume', value), 300);
const saveBalance = debounce((value) => saveData('balance', value), 300);

const volume = makeSlider(document.getElementById('volumeSlider'), {
    min: 0,
    max: 100,
    value: 50,
    onInput(value) {
        spectrum.setVolume(value / 100);
        document.getElementById('volumeValue').textContent = value + '%';
        document.getElementById('volumeIcon').setAttribute('href', value === 0 ? '#i-vol-0' : value < 50 ? '#i-vol-1' : '#i-vol-2');
        saveVolume(value);
    },
});

const balance = makeSlider(document.getElementById('balanceSlider'), {
    min: -100,
    max: 100,
    value: 0,
    center: 0,
    onInput(value) {
        spectrum.setBalance(value / 100);
        document.getElementById('balanceValue').textContent = value === 0 ? 'C' : (value < 0 ? 'L ' : 'R ') + Math.abs(value);
        saveBalance(value);
    },
});

/* Settings (edited in the settings window, settings.js) */

const { Menu } = window.__TAURI__.menu;

function applySettings() {
    const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent-light').trim();
    spectrum.setColors(settings.colors, accent);
}

function openSettings() {
    closeDialog();
    invoke('open_panel', { name: 'settings' });
}

function openInfo() {
    closeDialog();
    invoke('open_panel', { name: 'info' });
}

ui.settingsButton.addEventListener('click', openSettings);

// A dot on the cog when a downloaded update is waiting (installed from Settings).
function showUpdate(update) {
    const ready = update.status === 'ready';
    ui.settingsButton.classList.toggle('has-update', ready);
    ui.settingsButton.title = ready ? `Settings: version ${update.version} is ready to install` : 'Settings';
}

listen('update-status', ({ payload }) => showUpdate(payload));
invoke('update_status').then(showUpdate);
ui.infoButton.addEventListener('click', openInfo);

/* Player status for the Stream information window (info.js): sent every second, on state
   changes, and when that window asks for it. */

function emitStatus() {
    const cartridge = cartridges[activeSlot];
    let buffer = 0;
    const audio = current();
    if (audio && audio.buffered.length) buffer = Math.max(0, audio.buffered.end(audio.buffered.length - 1) - audio.currentTime);
    window.__TAURI__.event.emit('player-status', {
        state,
        station: cartridge ? cartridge.title : null,
        url: cartridge ? cartridge.url : null,
        buffer: state === 'stopped' ? null : buffer,
        stalls,
        statusTitle,
    });
}

setInterval(emitStatus, 1000);
listen('player-status-request', emitStatus);

let shortcuts = {};

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
    if (payload.key === 'shortcuts') {
        shortcuts = payload.value;
        applyShortcuts();
    } else {
        settings[payload.key] = payload.value;
        applySettings();
    }
});

/* Right-click menu */

document.addEventListener('contextmenu', async (event) => {
    if (!ui.dialog.hidden) return;

    const element = event.target.closest('.cartridge');
    const slot = element ? [...ui.cartridges.children].indexOf(element) : -1;
    const items = [];
    if (slot >= 0 && cartridges[slot]) {
        items.push(
            { text: 'Play', action: () => selectSlot(slot) },
            { text: 'Edit…', action: () => openDialog(slot) },
            { text: 'Eject', action: () => eject(slot) },
        );
    } else if (slot >= 0) {
        items.push({ text: 'Load cartridge…', action: () => openDialog(slot) });
    } else {
        const playing = state !== 'stopped' && state !== 'paused';
        items.push(
            playing
                ? { text: 'Pause', action: pause }
                : { text: 'Play', enabled: activeSlot !== null, accelerator: shortcutLabel('play'), action: play },
            { text: 'Stop', enabled: state !== 'stopped', accelerator: shortcutLabel('stop'), action: stop },
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
    await menu.popup();
});

/* Buttons and shortcuts */

ui.playButton.addEventListener('click', play);
ui.instantButton.addEventListener('click', () => {
    setInstant(!instant);
    saveData('instant-switch', instant);
});
ui.pauseButton.addEventListener('click', pause);
ui.stopButton.addEventListener('click', stop);

listen('play-action', () => {
    if (state === 'stopped' || state === 'paused') play();
});
listen('stop-action', stop);
listen('toggle-action', () => {
    if (state === 'stopped' || state === 'paused') play();
    else stop();
});

/* Startup */

(async function init() {
    for (let slot = 0; slot < SLOTS; slot++) {
        const [title, url] = await Promise.all([loadData(`player${slot + 1}-title`), loadData(`player${slot + 1}-url`)]);
        if (url) cartridges[slot] = { title: title || host(url), url };
    }

    const last = await loadData('last-selected-player');
    const lastSlot = last ? parseInt(String(last).replace('player', ''), 10) - 1 : -1;
    if (cartridges[lastSlot]) activeSlot = lastSlot;

    const [savedVolume, savedBalance] = await Promise.all([loadData('volume'), loadData('balance')]);
    if (typeof savedVolume === 'number') volume.set(savedVolume);
    if (typeof savedBalance === 'number') balance.set(savedBalance);

    const [colors, autoplay] = await Promise.all([loadData('visualizer-colors'), loadData('autoplay')]);
    if (['classic', 'white', 'accent'].includes(colors)) settings.colors = colors;
    settings.autoplay = autoplay === true;
    applySettings();
    setInstant((await loadData('instant-switch')) === true);
    shortcuts = await invoke('get_shortcuts');
    applyShortcuts();

    setState('stopped');
    renderInfo();
    // Opened by the background helper to play (Play / Stop / Start pressed while Qust was closed).
    const requested = await invoke('take_play_request');
    if ((requested || settings.autoplay) && activeSlot !== null) play();
})();
