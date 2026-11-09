const SLOTS = 6;
const LIVE_RESUME_SECONDS = 10;
const INFO_INTERVAL = 2000;
const STATUS_INTERVAL = 15000;
// Reconnecting after a drop: right away, then a try every second for 15 tries, then every 15 s
// until the time set in Settings (`reconnect-minutes`, 0 = never give up) runs out.
const RETRY_FAST_TRIES = 15;
const RETRY_FAST_DELAY = 1000;
const RETRY_SLOW_DELAY = 15000;
// A try that hasn't started playing by then failed (the proxy gives up connecting after 10 s).
const ATTEMPT_TIMEOUT = 12000;
// Playing, but the clock hasn't moved for this long: the stream or the audio engine is stuck.
const STALL_TIMEOUT = 3000;
// Playing this long again ends a reconnect sequence; a drop before that continues it.
const RECOVERED_TIMEOUT = 10000;
// The webview opens at most six connections to one host (the stream proxy).
const PROXY_CONNECTIONS = 6;
// Waiting this long for a playlist file's duration counts as unknown.
const PROBE_TIMEOUT = 8000;

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
    nameLabel: document.getElementById('nameLabel'),
    urlLabel: document.getElementById('urlLabel'),
    saveButton: document.getElementById('saveButton'),
    playlistButton: document.getElementById('playlistButton'),
    addFileButton: document.getElementById('addFileButton'),
    addUrlButton: document.getElementById('addUrlButton'),
    drawer: document.getElementById('drawer'),
    playlist: document.getElementById('playlist'),
    playlistCount: document.getElementById('playlistCount'),
};

// cartridges[i] = { title, url } or null, for slots player1..player6
let cartridges = new Array(SLOTS).fill(null);
// What plays: a cartridge (activeSlot) or a playlist entry (activeEntry), never both. An entry is
// { title, path } for a local file or { title, url } for a stream or online file, with its
// duration once known (null for unknown or live).
let activeSlot = null;
let activeEntry = null;
let playlist = [];
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
let dialogEntry = null;
let settings = { colors: 'classic', autoplay: false, reconnectMinutes: 5 };
// The reconnect sequence in progress: { since, tries, startedAt, timer, recoveredAt }, or null.
let retry = null;
// Why playback stopped on its own ("Connection lost"), shown in place of "Stopped".
let notice = null;
// Last seen progress of the active stream and of the audio engine, for the stall watchdog.
let progress = null;
// Instant switch (the lightning button): every loaded cartridge streams, muted but the active one.
let instant = false;
// One <audio> per source (several in instant mode), all routed through the visualizer's graph,
// keyed by keyOf(): the URL, or "file:" and the path.
const streams = new Map();
// The source that was paused. The clock shows clockBase (time carried over a reconnect) plus the
// active stream's time since clockStart (its time when it became active). A file's clock is its
// position.
let pausedKey = null;
let clockStart = 0;
let clockBase = 0;
let backgroundRetry = null;
// The last position of the active file, for picking up there if its element has to be replaced.
let filePosition = null;
// Playlist entries skipped in a row because they wouldn't play.
let skips = 0;

/* Streams */

function active() {
    return (activeSlot !== null && cartridges[activeSlot]) || activeEntry || null;
}

function keyOf(source) {
    return source.path ? 'file:' + source.path : source.url;
}

function mediaUrl(source) {
    return source.path ? fileUrl(source.path) : proxied(source.url);
}

// A file (local, or online with a known length) rather than a live stream: it ends, it resumes
// where it was paused, and running out of buffer means waiting rather than reconnecting.
function finite(element) {
    return !!element && Number.isFinite(element.duration);
}

function current() {
    const source = active();
    return source ? streams.get(keyOf(source)) || null : null;
}

function streamFor(key) {
    if (streams.has(key)) return streams.get(key);
    const element = new Audio();
    element.crossOrigin = 'anonymous';
    element.preload = 'none';
    element.addEventListener('playing', () => {
        if (element !== current() || state === 'stopped' || state === 'paused') return renderCartridges();
        if (retry) {
            clearTimeout(retry.timer);
            retry.recoveredAt = Date.now();
        }
        skips = 0;
        setState('playing');
    });
    // The buffer ran out: a stream reconnects rather than wait for it to refill; a file downloading
    // slower than it plays just waits.
    element.addEventListener('waiting', () => {
        if (element !== current() || state !== 'playing') return renderCartridges();
        if (finite(element)) setState('buffering');
        else lost();
    });
    element.addEventListener('durationchange', () => {
        if (element !== current() || !activeEntry || keyOf(activeEntry) !== key) return;
        const duration = finite(element) ? element.duration : null;
        if (activeEntry.duration === duration) return;
        activeEntry.duration = duration;
        renderRow(activeEntry);
        renderPlaylistStatus();
        savePlaylist();
    });
    // The connection closed or failed, or a file ended. A source that never played isn't retried
    // (wrong address, missing file): the playlist skips it.
    const dropped = (event) => {
        if (streams.get(key) !== element) return;
        // A background stream, or the active cartridge preloading while stopped (instant switch).
        if (element !== current() || state === 'stopped') {
            renderCartridges();
            retryBackground();
        } else if (state === 'stopped' || state === 'paused') {
            return;
        } else if (event.type === 'ended' && finite(element)) {
            playNext();
        } else if (state === 'connecting' && !retry) {
            couldNotPlay();
        } else {
            lost();
        }
    };
    element.addEventListener('error', dropped);
    element.addEventListener('ended', dropped);
    spectrum.attach(element);
    streams.set(key, element);
    return element;
}

function unload(key) {
    const element = streams.get(key);
    element.pause();
    element.removeAttribute('src');
    element.load();
    spectrum.detach(element);
    streams.delete(key);
}

// Already playing and ahead of the buffer: switching to it is instant.
function isLive(element) {
    return !!element.getAttribute('src') && !element.paused && !element.error && element.readyState >= 3;
}

// Keeps the right streams connected: the active one while playing, plus, in instant mode, every
// loaded cartridge (muted), as many as the proxy's connections allow, even while stopped. Only the
// 6 cartridges preload, never playlist entries. Everything else is disconnected.
function syncStreams() {
    const source = active();
    const activeKey = source && state !== 'stopped' ? keyOf(source) : null;
    const wanted = new Map();
    if (activeKey) wanted.set(activeKey, source);
    if (instant) {
        let room = PROXY_CONNECTIONS - (activeKey && !source.path ? 1 : 0);
        for (const cartridge of cartridges) {
            if (!cartridge || wanted.has(cartridge.url) || room <= 0) continue;
            wanted.set(cartridge.url, cartridge);
            room--;
        }
    }
    for (const key of [...streams.keys()]) {
        if (!wanted.has(key)) unload(key);
    }
    for (const [key, item] of wanted) {
        if (key === activeKey) continue;
        const element = streamFor(key);
        if (!element.getAttribute('src') || element.error || element.ended) element.src = mediaUrl(item);
        if (element.paused) element.play().catch(() => {});
    }
    streams.forEach((element, key) => spectrum.setLevel(element, key === activeKey ? 1 : 0));
    renderCartridges();
    renderPlaylistState();
}

// A background stream (instant switch) dropped: reconnect it a little later.
function retryBackground() {
    if (backgroundRetry) return;
    backgroundRetry = setTimeout(() => {
        backgroundRetry = null;
        syncStreams();
    }, RETRY_SLOW_DELAY);
}

function setInstant(on) {
    instant = on;
    ui.instantButton.setAttribute('aria-pressed', String(on));
    ui.instantButton.title = on
        ? 'Instant switch is on: every cartridge streams in the background, even when stopped, so starting or switching is immediate (uses more data)'
        : 'Instant switch: keep every cartridge streaming in the background, even when stopped, for immediate starting and switching (uses more data)';
    syncStreams();
}

/* Playback */

function play() {
    const source = active();
    if (!source) return;
    endRetry();
    notice = null;
    spectrum.start();
    const key = keyOf(source);
    const element = streamFor(key);
    // A file resumes where it was paused; a stream only shortly after, or it rejoins the live point.
    const resume = state === 'paused' && pausedKey === key && !!element.getAttribute('src') &&
        (finite(element) || (Date.now() - pausedAt) / 1000 < LIVE_RESUME_SECONDS);
    const live = !resume && isLive(element) && !finite(element);
    if (!resume) {
        stalls = 0;
        streamInfo = {};
        statusTitle = null;
        lastStatusFetch = 0;
        clockBase = 0;
        clockStart = live ? element.currentTime : 0;
        filePosition = null;
        if (!live) element.src = mediaUrl(source);
    }
    setState(live ? 'playing' : resume ? 'buffering' : 'connecting');
    element.play().catch(() => {});
    startInfo();
    syncStreams();
}

function pause() {
    if (state === 'stopped' || state === 'paused') return;
    endRetry();
    const element = current();
    if (element) element.pause();
    pausedKey = active() ? keyOf(active()) : null;
    pausedAt = Date.now();
    setState('paused');
}

function stop() {
    endRetry();
    notice = null;
    setState('stopped');
    syncStreams();
    spectrum.stop();
    stopInfo();
    streamInfo = {};
    statusTitle = null;
    renderInfo();
}

// Stops and says why in place of the title.
function fail(text) {
    stop();
    notice = text;
    renderTitle();
}

// The active source never started. In the playlist, that entry is marked and the next one plays,
// until every entry has been tried.
function couldNotPlay() {
    if (!activeEntry) return fail("Couldn't play this stream");
    activeEntry.missing = true;
    renderPlaylistState();
    skips++;
    if (skips < playlist.length && playNext()) return;
    skips = 0;
    fail(activeEntry.path ? "Couldn't play this file" : "Couldn't play this stream");
}

// The playlist entry after the active one (none after the last: the playlist plays once).
function playNext() {
    const index = activeEntry ? playlist.indexOf(activeEntry) : -1;
    const next = index >= 0 ? playlist[index + 1] : null;
    if (!next) {
        stop();
        return false;
    }
    selectEntry(next);
    revealRow(next);
    return true;
}

/* Reconnecting */

function endRetry() {
    if (retry) clearTimeout(retry.timer);
    retry = null;
}

// The active stream dropped or got stuck: reconnect right away the first time, then on the
// schedule above. Each try that fails (error, closed, or not playing in time) calls this again.
function lost() {
    if (state === 'stopped' || state === 'paused' || !active()) return;
    if (state === 'playing') stalls++;
    if (!retry) retry = { since: Date.now(), tries: 0, startedAt: 0, timer: null, recoveredAt: 0 };
    clearTimeout(retry.timer);
    retry.recoveredAt = 0;
    // Counted from the start of the last try, which may itself have taken a while to fail.
    const interval = retry.tries < RETRY_FAST_TRIES ? RETRY_FAST_DELAY : RETRY_SLOW_DELAY;
    const delay = retry.tries === 0 ? 0 : Math.max(0, interval - (Date.now() - retry.startedAt));
    const limit = settings.reconnectMinutes * 60000;
    if (limit && Date.now() + delay - retry.since > limit) return fail('Connection lost');
    retry.timer = setTimeout(reconnect, delay);
    if (state !== 'buffering') setState('buffering');
}

function reconnect() {
    const source = active();
    if (!retry || !source || state === 'stopped' || state === 'paused') return;
    retry.tries++;
    retry.startedAt = Date.now();
    // A fresh element: the old one may hold a dead connection or a stuck decoder. A file picks up
    // where it was; a stream's clock carries on from where it was.
    const key = keyOf(source);
    const old = streams.get(key);
    let position = null;
    if (old) position = finite(old) ? old.currentTime : null;
    else if (filePosition && filePosition.key === key) position = filePosition.time;
    clockBase = position === null ? elapsed() : 0;
    clockStart = 0;
    if (old) unload(key);
    const element = streamFor(key);
    element.src = mediaUrl(source);
    if (position !== null) {
        element.addEventListener('loadedmetadata', () => { element.currentTime = position; }, { once: true });
    }
    element.play().catch(() => {});
    syncStreams();
    renderTitle();
    retry.timer = setTimeout(lost, ATTEMPT_TIMEOUT);
}

// The audio engine stopped rendering (its clock froze, e.g. after the output device changed or
// the PC woke up): start a new one. An element can feed only one AudioContext, so every stream
// gets a new element.
function restartAudio() {
    const element = current();
    if (finite(element)) filePosition = { key: keyOf(active()), time: element.currentTime };
    clockBase = elapsed();
    clockStart = 0;
    for (const key of [...streams.keys()]) unload(key);
    spectrum.reset();
    spectrum.start();
}

// Every second: wake a suspended audio engine, and catch a stream whose clock stopped without an
// event (the player would otherwise sit on "playing" with no sound).
function watch() {
    const element = current();
    if (state === 'stopped' || state === 'paused') {
        progress = null;
        return;
    }
    spectrum.wake();
    if (retry && retry.recoveredAt && Date.now() - retry.recoveredAt > RECOVERED_TIMEOUT) endRetry();
    if (state !== 'playing' || !element) {
        progress = null;
        return;
    }
    const now = Date.now();
    const engine = spectrum.clock();
    if (!progress || progress.element !== element || progress.time !== element.currentTime) {
        progress = { element, time: element.currentTime, engine, at: now };
        return;
    }
    if (now - progress.at < STALL_TIMEOUT) return;
    const engineStuck = engine !== null && engine === progress.engine;
    progress = null;
    if (engineStuck) restartAudio();
    lost();
}

setInterval(watch, 1000);

function setState(next) {
    state = next;
    const icon = { playing: 'play', buffering: 'play', connecting: 'play', paused: 'pause' }[state] || 'stop';
    ui.stateIcon.setAttribute('href', '#i-' + icon);
    ui.clock.classList.toggle('is-idle', state === 'stopped');
    spectrum.setLoading(state === 'connecting' || state === 'buffering');
    ui.playButton.disabled = !active() || state === 'playing' || state === 'buffering' || state === 'connecting';
    ui.pauseButton.disabled = state === 'stopped' || state === 'paused';
    ui.stopButton.disabled = state === 'stopped';
    renderCartridges();
    renderPlaylistState();
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
    const source = active();
    if (!source) return;
    // The answer is dropped if the station changed meanwhile.
    const stillActive = () => active() === source;
    let info = {};
    try {
        info = await streamDetails(source.path || source.url);
    } catch (error) {}
    if (!stillActive()) return;
    // Keep the last title across a reconnect, until the new connection sends one.
    if (retry && !info.title && streamInfo.title) info.title = streamInfo.title;
    // A file whose header doesn't give its bitrate (Ogg, M4A, FLAC): its average.
    const element = current();
    if (!info.bitrate && info.size && finite(element)) info.bitrate = Math.round(info.size * 8 / element.duration / 1000);
    streamInfo = info;
    // Only streams have an Icecast status page.
    const stream = !source.path && !finite(element);
    if (stream && !streamInfo.title && Date.now() - lastStatusFetch > STATUS_INTERVAL) {
        lastStatusFetch = Date.now();
        const title = await icecastTitle(source.url).catch(() => null);
        if (!stillActive()) return;
        statusTitle = title;
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

/* Display */

function renderTitle() {
    const source = active();
    if (activeEntry) {
        const index = playlist.indexOf(activeEntry);
        ui.station.textContent = index >= 0 ? `Playlist · ${index + 1} of ${playlist.length}` : 'Playlist';
    } else {
        ui.station.textContent = source ? source.title : 'No cartridge loaded';
    }
    if (!source) {
        setTitle('Load a cartridge to start', true);
    } else if (state === 'stopped') {
        setTitle(notice || 'Stopped', true);
    } else if (state === 'connecting') {
        setTitle('Connecting…', true);
    } else if (retry && state === 'buffering') {
        setTitle('Reconnecting…', true);
    } else {
        const title = activeEntry
            ? streamInfo.title || statusTitle || activeEntry.title
            : streamInfo.title || statusTitle || streamInfo.name || source.title;
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

function elapsed() {
    const element = current();
    return clockBase + (element ? Math.max(0, (element.currentTime || 0) - clockStart) : 0);
}

function renderClock() {
    const seconds = state === 'stopped' ? 0 : Math.floor(elapsed());
    const h = Math.floor(seconds / 3600);
    const m = Math.floor(seconds / 60) % 60;
    const s = String(seconds % 60).padStart(2, '0');
    ui.clockText.textContent = h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${String(m).padStart(2, '0')}:${s}`;
    renderSeek();
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
        const stream = !empty && instant && (slot !== activeSlot || state === 'stopped') ? streams.get(cartridge.url) : null;
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
    activeEntry = null;
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
    renderPlaylist();
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

// The dialog edits a cartridge (dialogSlot) or a playlist entry (dialogEntry), or, with neither,
// adds a URL to the playlist.
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

function openEntryDialog(entry) {
    dialogSlot = null;
    dialogEntry = entry;
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

function openDialog(slot) {
    dialogEntry = null;
    dialogSlot = slot;
    const cartridge = cartridges[slot];
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

function openUrlDialog() {
    dialogSlot = null;
    dialogEntry = null;
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
    dialogSlot = null;
    dialogEntry = null;
}

// Renames a playlist entry, and for a URL entry changes its address. The active entry reconnects
// to a new address if it was playing.
function saveEntry(entry, name, url) {
    if (url && url !== entry.url) {
        const wasPlaying = entry === activeEntry && state !== 'stopped';
        if (wasPlaying) stop();
        entry.url = url;
        entry.duration = null;
        entry.missing = false;
        if (wasPlaying) play();
    }
    entry.title = name || (entry.path ? fileTitle(entry.path) : urlTitle(entry.url));
    savePlaylist();
    renderRow(entry);
    renderPlaylistState();
    renderPlaylistStatus();
    renderTitle();
    emitStatus();
}

ui.dialogForm.addEventListener('submit', (event) => {
    event.preventDefault();
    if (dialogEntry && dialogEntry.path) {
        const entry = dialogEntry;
        const name = ui.streamTitle.value.trim();
        closeDialog();
        saveEntry(entry, name, null);
        return;
    }
    const url = ui.streamURL.value.trim();
    let valid = false;
    try {
        valid = ['http:', 'https:'].includes(new URL(url).protocol);
    } catch (error) {}
    if (!valid) {
        ui.dialogError.textContent = 'Enter an address starting with http:// or https://';
        ui.dialogError.hidden = false;
        ui.streamURL.focus();
        return;
    }

    if (dialogEntry) {
        const entry = dialogEntry;
        const name = ui.streamTitle.value.trim();
        closeDialog();
        saveEntry(entry, name, url);
        return;
    }

    if (dialogSlot === null) {
        const name = ui.streamTitle.value.trim();
        closeDialog();
        addEntries([{ url, title: name || null }]);
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
    renderPlaylist();
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
    renderPlaylist();
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

/* Playlist: the drawer under the player. The loaded cartridges are pinned at the top (they play
   from here, but move and eject only above); below them come the files and URLs added by dropping
   them on the window, "Add file" or "Add URL". Files play where they are (fileUrl, proxy.rs). */

const AUDIO_FILE = /\.(mp3|m4a|aac|mp4|ogg|oga|opus|flac|wav|webm|weba)$/i;
let drawerOpen = false;
// Selected rows (cartridges and entries). Shift extends the selection from anchor; the arrow keys
// move from focusItem.
let selection = new Set();
let anchor = null;
let focusItem = null;
// The item each row shows, in order (row.dataset.row is its index), and the row of each item.
let rowItems = [];
const rowOf = new Map();
let rowDrag = null;
let suppressRowClick = false;
let dropRow = null;
let probing = false;

const savePlaylist = debounce(() => {
    saveData('playlist', playlist.map(({ title, path, url, duration }) => (path ? { title, path, duration } : { title, url, duration })));
    if (activeEntry) saveData('playlist-index', playlist.indexOf(activeEntry));
}, 400);

function fileTitle(path) {
    return path.split(/[\\/]/).pop().replace(/\.[^.]+$/, '');
}

// An online file's name, or a stream's host.
function urlTitle(url) {
    try {
        const parsed = new URL(url);
        const name = decodeURIComponent(parsed.pathname.split('/').pop() || '');
        return AUDIO_FILE.test(name) ? name.replace(/\.[^.]+$/, '') : parsed.host;
    } catch (error) {
        return url;
    }
}

function toEntry(item) {
    const duration = typeof item.duration === 'number' && item.duration > 0 ? item.duration : null;
    if (item.path) return { title: item.title || fileTitle(item.path), path: item.path, duration };
    if (item.url) return { title: item.title || urlTitle(item.url), url: item.url, duration };
    return null;
}

function isTrack(item) {
    return !!item && !cartridges.includes(item);
}

function formatTime(seconds) {
    seconds = Math.round(seconds);
    const h = Math.floor(seconds / 3600);
    const m = Math.floor(seconds / 60) % 60;
    const s = String(seconds % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

// Adds files or URLs ({ path | url, title?, duration? }) at `index`, selects them and opens the
// drawer. With nothing to play yet, the first one becomes what Play plays.
function addEntries(items, index = playlist.length) {
    const entries = items.map(toEntry).filter(Boolean);
    if (!entries.length) return;
    playlist.splice(Math.min(index, playlist.length), 0, ...entries);
    if (!active()) activeEntry = entries[0];
    selection = new Set(entries);
    anchor = focusItem = entries[0];
    savePlaylist();
    renderPlaylist();
    setState(state);
    if (!drawerOpen) setDrawer(true);
    revealRow(entries[entries.length - 1]);
    revealRow(entries[0]);
    probeDurations();
}

// Removing the active entry stops it; the next one becomes what Play plays.
function removeEntries(entries) {
    const removing = new Set(entries);
    const first = playlist.findIndex((entry) => removing.has(entry));
    if (first < 0) return;
    const remaining = playlist.filter((entry) => !removing.has(entry));
    if (activeEntry && removing.has(activeEntry)) {
        if (state !== 'stopped') stop();
        const index = playlist.indexOf(activeEntry);
        activeEntry = playlist.slice(index + 1).find((entry) => !removing.has(entry)) || remaining[remaining.length - 1] || null;
    }
    playlist = remaining;
    const next = playlist[Math.min(first, playlist.length - 1)];
    selection = new Set(next ? [next] : []);
    anchor = focusItem = next || null;
    savePlaylist();
    renderPlaylist();
    setState(state);
}

// Moves the selected entries before `index` (an index in the playlist as it is).
function moveSelection(index) {
    const moving = playlist.filter((entry) => selection.has(entry));
    if (!moving.length) return;
    const before = playlist.slice(index).find((entry) => !selection.has(entry)) || null;
    const rest = playlist.filter((entry) => !selection.has(entry));
    rest.splice(before ? rest.indexOf(before) : rest.length, 0, ...moving);
    playlist = rest;
    savePlaylist();
    renderPlaylist();
    renderTitle();
}

function selectEntry(entry) {
    activeSlot = null;
    activeEntry = entry;
    entry.missing = false;
    saveData('last-selected-player', 'playlist');
    saveData('playlist-index', playlist.indexOf(entry));
    play();
}

function playItem(item) {
    if (isTrack(item)) selectEntry(item);
    else selectSlot(cartridges.indexOf(item));
}

function renderPlaylist() {
    const list = ui.playlist;
    const scroll = list.scrollTop;
    list.textContent = '';
    rowItems = [];
    rowOf.clear();
    dropRow = null;
    const fragment = document.createDocumentFragment();
    cartridges.forEach((cartridge, slot) => {
        if (cartridge) fragment.appendChild(makeRow(cartridge, `Cartridge ${slot + 1}`));
    });
    if (rowItems.length) {
        const divider = document.createElement('div');
        divider.className = 'playlist-divider';
        fragment.appendChild(divider);
    }
    playlist.forEach((entry, index) => fragment.appendChild(makeRow(entry, index + 1)));
    if (!playlist.length) {
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
    row.querySelector('.row-title').textContent = item.title;
    let detail = '';
    if (!isTrack(item)) detail = host(item.url);
    else if (item.duration) detail = formatTime(item.duration);
    else if (item.url) detail = host(item.url);
    row.querySelector('.row-detail').textContent = detail;
    row.title = item.path || item.url;
}

function renderPlaylistState() {
    const source = active();
    const playing = state !== 'stopped' && state !== 'paused';
    rowOf.forEach((row, item) => {
        const isActive = item === source;
        row.classList.toggle('is-active', isActive);
        row.classList.toggle('is-playing', isActive && playing);
        row.classList.toggle('is-selected', selection.has(item));
        row.classList.toggle('is-missing', !!item.missing);
        row.setAttribute('aria-selected', String(selection.has(item)));
    });
}

function renderPlaylistStatus() {
    const parts = [playlist.length === 1 ? '1 track' : `${playlist.length} tracks`];
    const total = playlist.reduce((sum, entry) => sum + (entry.duration || 0), 0);
    if (total) parts.push(formatTime(total));
    if (selection.size > 1) parts.push(`${selection.size} selected`);
    ui.playlistCount.textContent = parts.join(' · ');
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

function select(item, { toggle = false, extend = false } = {}) {
    if (extend && anchor && rowOf.has(anchor)) {
        const from = rowItems.indexOf(anchor);
        const to = rowItems.indexOf(item);
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
    if (item) {
        select(item, { toggle: event.ctrlKey || event.metaKey, extend: event.shiftKey });
    } else if (!event.ctrlKey && !event.shiftKey) {
        selection.clear();
        renderPlaylistState();
        renderPlaylistStatus();
    }
});

ui.playlist.addEventListener('dblclick', (event) => {
    const item = itemOf(event.target);
    if (item) playItem(item);
});

ui.playlist.addEventListener('keydown', (event) => {
    const index = focusItem ? rowItems.indexOf(focusItem) : -1;
    const moves = { ArrowUp: -1, ArrowDown: 1, PageUp: -10, PageDown: 10 };
    let to = null;
    if (event.key in moves) to = index < 0 ? 0 : index + moves[event.key];
    else if (event.key === 'Home') to = 0;
    else if (event.key === 'End') to = rowItems.length - 1;
    if (to !== null) {
        const item = rowItems[Math.max(0, Math.min(rowItems.length - 1, to))];
        if (item) {
            select(item, { extend: event.shiftKey });
            revealRow(item);
        }
    } else if (event.key === 'Enter' && focusItem) {
        playItem(focusItem);
    } else if (event.key === 'Delete' || event.key === 'Backspace') {
        removeEntries(playlist.filter((entry) => selection.has(entry)));
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
        selection = new Set(playlist);
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
    if (!drawerOpen || !playlist.length || x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return playlist.length;
    const first = rowOf.get(playlist[0]);
    const offset = y - rect.top + ui.playlist.scrollTop - first.offsetTop;
    return Math.max(0, Math.min(playlist.length, Math.round(offset / first.offsetHeight)));
}

// The insertion line for a drop (null hides it).
function showDropIndex(index) {
    if (dropRow) dropRow.classList.remove('drop-before', 'drop-after');
    dropRow = null;
    ui.playlist.classList.toggle('is-drop-empty', index !== null && !playlist.length);
    if (index === null || !playlist.length) return;
    dropRow = rowOf.get(playlist[Math.min(index, playlist.length - 1)]);
    dropRow.classList.add(index < playlist.length ? 'drop-before' : 'drop-after');
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

// Files and folders dropped on the window (Tauri hands over their paths). Hovering opens the
// drawer and shows where they'll go.
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
        addEntries(await invoke('expand_paths', { paths: payload.paths }), index);
        return;
    }
    if (!drawerOpen) setDrawer(true);
    document.body.classList.add('is-dropping');
    showDropIndex(dropIndex(x, y));
});

// Durations of local files, read one at a time in the background.
async function probeDurations() {
    if (probing) return;
    probing = true;
    let changed = false;
    for (;;) {
        const entry = playlist.find((item) => item.path && !item.duration && !item.probed);
        if (!entry) break;
        entry.probed = true;
        const duration = await probeDuration(fileUrl(entry.path));
        if (duration === false) entry.missing = true;
        else if (duration) entry.duration = duration;
        if (duration !== null) changed = true;
        renderRow(entry);
        renderPlaylistState();
    }
    probing = false;
    if (changed) {
        renderPlaylistStatus();
        savePlaylist();
    }
}

// A file's duration in seconds, null if unknown, false if it can't be played.
function probeDuration(url) {
    return new Promise((resolve) => {
        const audio = new Audio();
        audio.preload = 'metadata';
        const done = (value) => {
            clearTimeout(timer);
            audio.removeAttribute('src');
            audio.load();
            resolve(value);
        };
        const timer = setTimeout(() => done(null), PROBE_TIMEOUT);
        audio.addEventListener('loadedmetadata', () => done(Number.isFinite(audio.duration) ? audio.duration : null));
        audio.addEventListener('error', () => done(false));
        audio.src = url;
    });
}

// The window grows to show the drawer (playlist.rs), and remembers it for next time.
function setDrawer(open, animate = true) {
    drawerOpen = open;
    ui.playlistButton.setAttribute('aria-expanded', String(open));
    ui.drawer.inert = !open;
    if (animate) invoke('set_playlist_open', { open });
}

async function addFiles() {
    addEntries(await invoke('pick_files'));
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

// onInput follows every change; onCommit (optional) gets the value the user settles on: at the
// end of a drag, or after a key or wheel step.
function makeSlider(element, { min, max, value, center, onInput, onCommit }) {
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

    set(value, false);
    // Not while the user drags it (the seek bar follows playback otherwise).
    return { set: (next) => { if (!dragging) set(next, false); } };
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

/* Seek bar: shown for a playlist file with a known duration (local, or online with a length), in
   place of the stacked volume and balance. Its value is the position in ten-thousandths. */

const SEEK_STEPS = 10000;
const seekSlider = document.getElementById('seekSlider');
const seekTime = document.getElementById('seekTime');

function seekDuration() {
    const element = current();
    if (finite(element)) return element.duration;
    return activeEntry && activeEntry.duration ? activeEntry.duration : null;
}

const seek = makeSlider(seekSlider, {
    min: 0,
    max: SEEK_STEPS,
    value: 0,
    onInput(value) {
        const duration = seekDuration();
        if (duration) seekTime.innerHTML = `<b>${formatTime(value / SEEK_STEPS * duration)}</b> / ${formatTime(duration)}`;
    },
    onCommit(value) {
        const element = current();
        if (!finite(element) || state === 'stopped') return;
        element.currentTime = value / SEEK_STEPS * element.duration;
        renderClock();
    },
});

// Called with the clock: switches the card's layout and follows playback.
function renderSeek() {
    const duration = seekDuration();
    document.getElementById('sliders').classList.toggle('is-seekable', !!duration);
    if (!duration) return;
    const element = current();
    const playable = state !== 'stopped' && finite(element);
    seekSlider.setAttribute('aria-disabled', String(!playable));
    seekSlider.tabIndex = playable ? 0 : -1;
    seek.set(playable ? Math.min(1, element.currentTime / element.duration) * SEEK_STEPS : 0);
}

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
// Clicking the analyzer cycles its colors, "off" included (also in Settings).
const COLOR_CYCLE = ['classic', 'white', 'accent', 'off'];

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

document.getElementById('screen').addEventListener('click', () => {
    const value = COLOR_CYCLE[(COLOR_CYCLE.indexOf(settings.colors) + 1) % COLOR_CYCLE.length];
    settings.colors = value;
    applySettings();
    saveData('visualizer-colors', value);
    window.__TAURI__.event.emit('settings-changed', { key: 'colors', value });
});

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
    const source = active();
    let buffer = 0;
    const audio = current();
    if (audio && audio.buffered.length) buffer = Math.max(0, audio.buffered.end(audio.buffered.length - 1) - audio.currentTime);
    window.__TAURI__.event.emit('player-status', {
        state,
        station: source ? source.title : null,
        url: source ? source.path || source.url : null,
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
    if (event.target.closest('#drawer')) {
        const item = itemOf(event.target);
        if (item && !selection.has(item)) select(item);
        const tracks = playlist.filter((entry) => selection.has(entry));
        if (item && !isTrack(item)) {
            items.push(
                { text: 'Play', action: () => playItem(item) },
                { text: 'Edit cartridge…', action: () => openDialog(cartridges.indexOf(item)) },
                { item: 'Separator' },
            );
        } else if (item) {
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
            { text: 'Clear playlist', enabled: playlist.length > 0, action: () => removeEntries(playlist) },
        );
    } else if (slot >= 0 && cartridges[slot]) {
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
                : { text: 'Play', enabled: !!active(), accelerator: shortcutLabel('play'), action: play },
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

    const [savedPlaylist, playlistIndex, playlistOpen] = await Promise.all([loadData('playlist'), loadData('playlist-index'), loadData('playlist-open')]);
    if (Array.isArray(savedPlaylist)) playlist = savedPlaylist.map(toEntry).filter(Boolean);
    if (last === 'playlist' && playlist[playlistIndex]) activeEntry = playlist[playlistIndex];
    // The window was built at the right height already (playlist.rs).
    setDrawer(playlistOpen === true, false);
    renderPlaylist();
    probeDurations();

    const [savedVolume, savedBalance] = await Promise.all([loadData('volume'), loadData('balance')]);
    if (typeof savedVolume === 'number') volume.set(savedVolume);
    if (typeof savedBalance === 'number') balance.set(savedBalance);

    const [colors, autoplay, reconnectMinutes] = await Promise.all([loadData('visualizer-colors'), loadData('autoplay'), loadData('reconnect-minutes')]);
    if (COLOR_CYCLE.includes(colors)) settings.colors = colors;
    settings.autoplay = autoplay === true;
    if (typeof reconnectMinutes === 'number' && reconnectMinutes >= 0) settings.reconnectMinutes = reconnectMinutes;
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
