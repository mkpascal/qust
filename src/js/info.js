// The Stream information window. The player (script.js) broadcasts its status as `player-status`;
// the stream's details come from the proxy's /info endpoint (proxy.rs), polled every second.

const STATES = { stopped: 'Stopped', connecting: 'Connecting', buffering: 'Buffering', playing: 'Playing', paused: 'Paused' };
const FORMATS = { 'audio/mpeg': 'MP3', 'audio/aac': 'AAC', 'audio/aacp': 'AAC+', 'audio/ogg': 'Ogg', 'application/ogg': 'Ogg', 'audio/flac': 'FLAC', 'audio/opus': 'Opus' };

const { emit } = window.__TAURI__.event;
const infoWindow = window.__TAURI__.window.getCurrentWindow();

let status = {};
let info = {};

function set(id, value) {
    const element = document.getElementById(id);
    const text = value === null || value === undefined || value === '' ? '—' : String(value);
    if (element.textContent !== text) element.textContent = text;
    element.title = text;
}

function bytes(count) {
    if (count === undefined) return null;
    if (count < 1e6) return `${Math.round(count / 1000)} KB`;
    return `${(count / 1e6).toFixed(1)} MB`;
}

function duration(seconds) {
    if (seconds === undefined || seconds === null) return null;
    const h = Math.floor(seconds / 3600);
    const m = Math.floor(seconds / 60) % 60;
    const s = String(seconds % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

function host(url) {
    try {
        return new URL(url).host;
    } catch (error) {
        return null;
    }
}

function render() {
    const active = status.url && status.state !== 'stopped';
    const details = active ? info : {};
    const contentType = details.contentType ? details.contentType.split(';')[0].trim().toLowerCase() : null;
    const title = details.title || status.statusTitle;

    set('station', status.station || 'Not playing');
    set('title', active ? title || details.name || status.station : 'Start a cartridge to see its stream details');
    set('state', STATES[status.state] || 'Stopped');
    document.getElementById('state').dataset.state = status.state || 'stopped';

    set('format', details.format || FORMATS[contentType]);
    set('bitrate', details.bitrate ? `${details.bitrate} kbps` : null);
    set('samplerate', details.samplerate ? `${(details.samplerate / 1000).toLocaleString('en', { maximumFractionDigits: 1 })} kHz` : null);
    set('channels', { 1: 'Mono', 2: 'Stereo' }[details.channels] || (details.channels ? `${details.channels} channels` : null));

    set('rate', details.rate ? `${details.rate} kbps` : null);
    set('buffer', active && status.buffer !== null ? `${status.buffer.toFixed(1)} s` : null);
    set('received', details.bytes !== undefined ? bytes(details.bytes + details.metaBytes) : null);
    set('connected', duration(details.connectedSeconds));

    set('stalls', active ? status.stalls : null);
    set('reconnects', details.reconnects);
    set('titleChanges', details.titleChanges);
    set('host', active ? host(status.url) : null);

    set('icyName', details.name);
    set('icyTitle', details.title);
    set('icyGenre', details.genre);
    set('icyDescription', details.description);
    set('icyWebsite', details.website);
    set('icyServer', details.server);
    set('icyContentType', details.contentType);
    set('icyMetaint', active ? (details.metaint ? `${details.metaint.toLocaleString('en')} bytes` : 'None') : null);
    set('icyMetaBytes', details.metaBytes !== undefined && details.metaint ? `${details.metaBytes.toLocaleString('en')} bytes` : null);
}

async function poll() {
    if (!status.url || status.state === 'stopped') {
        info = {};
    } else {
        try {
            info = await streamDetails(status.url);
        } catch (error) {
            info = {};
        }
    }
    render();
}

listen('player-status', ({ payload }) => {
    const changed = payload.url !== status.url || payload.state !== status.state;
    status = payload;
    if (changed) poll();
    else render();
});

setInterval(poll, 1000);
emit('player-status-request');
render();

document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') infoWindow.close();
});
