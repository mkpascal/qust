#!/usr/bin/env node
// Writes the state fixtures (reference/states/*.json). Each file is self-contained: what Rust
// would send (the `player-state` snapshot and playlist, or the settings, shortcuts, updater and
// background-helper answers, or the Stream information status and details), plus the analyzer
// frame and the user actions that lead to the state. Run it again after changing the lists below;
// hand-written fixtures (any file not listed here) are left alone.
//
// Fixture fields (see README.md): window, description, drawer, state, playlist, frame, settings,
// shortcuts, update, background, status, info, errors, actions, time / times, scales, themes,
// accent, now, version, size.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const OUT = path.join(ROOT, 'states');
const NOW = 1794400000; // 2026-11-12, for "Last checked … ago"
const ALL = [1, 1.25, 1.5, 2];
const ORANGE = { base: '#da3b01', light: '#ff9f6e', dark: '#a92e00' };

const C = {
    c9: { title: 'C9 Radio', url: 'http://stream.c9.fr/c9-128.mp3', preload: null },
    fip: { title: 'FIP', url: 'https://icecast.radiofrance.fr/fip-hifi.aac', preload: null },
    paradise: { title: 'Radio Paradise', url: 'http://stream.radioparadise.com/aac-320', preload: null },
    groove: { title: 'SomaFM Groove Salad', url: 'https://ice2.somafm.com/groovesalad-128-mp3', preload: null },
    kexp: { title: 'KEXP', url: 'https://kexp-mp3-128.streamguys1.com:8443/kexp128.mp3', preload: null },
};
const CARTRIDGES = [C.c9, C.fip, C.paradise, C.groove, null, C.kexp];
const EMPTY = [null, null, null, null, null, null];

const MUSIC = 'C:\\Users\\Me\\Music\\Daft Punk\\Discovery\\';
const PLAYLIST = [
    { id: 1, title: 'One More Time', path: MUSIC + '01 One More Time.flac', duration: 320.4, missing: false },
    { id: 2, title: 'Aerodynamic', path: MUSIC + '02 Aerodynamic.flac', duration: 212.6, missing: false },
    { id: 3, title: 'Digital Love', path: MUSIC + '03 Digital Love.flac', duration: 301.2, missing: false },
    { id: 4, title: 'Radio Nova', url: 'http://novazz.ice.infomaniak.ch/novazz-128.mp3', duration: null, missing: false },
    { id: 5, title: 'Harder, Better, Faster, Stronger', path: 'D:\\Old\\Harder Better Faster Stronger.mp3', duration: null, missing: true },
    { id: 6, title: 'Episode 142: a conversation about the history of electronic music, with far too long a title', url: 'https://cdn.podcasts.example.com/feeds/ep-142.mp3', duration: 3725, missing: false },
];
const ARTISTS = ['Air', 'Justice', 'Cassius', 'Phoenix', 'Étienne de Crécy', 'Motorbass', 'Kavinsky', 'Sébastien Tellier', 'Breakbot', 'Mr. Oizo'];
const MANY = Array.from({ length: 60 }, (_, i) => ({
    id: 100 + i,
    title: `${ARTISTS[i % ARTISTS.length]} - Track ${String(i + 1).padStart(2, '0')}`,
    path: `C:\\Users\\Me\\Music\\Mix\\${String(i + 1).padStart(2, '0')}.mp3`,
    duration: 150 + ((i * 37) % 240) + 0.4,
    missing: false,
}));

const SHORTCUTS = {
    play: { accelerator: 'Ctrl+Alt+Insert', registered: true },
    stop: { accelerator: 'Ctrl+Alt+KeyH', registered: true },
    toggle: { accelerator: '', registered: false },
};
const UPDATE = { installed: true, current: '1.1.5', status: 'idle', version: null, progress: null, error: null, lastCheck: NOW - 2 * 3600 - 300, auto: true };

function playerState(over = {}) {
    return {
        state: 'stopped',
        cartridges: CARTRIDGES,
        activeSlot: 0,
        activeId: null,
        instant: false,
        station: 'C9 Radio',
        title: 'Stopped',
        titleDim: true,
        bitrate: null,
        samplerate: null,
        channels: null,
        volume: 70,
        balance: 0,
        colors: 'classic',
        elapsed: 0,
        duration: null,
        position: null,
        seekable: false,
        ...over,
    };
}

const PLAYING = { state: 'playing', title: 'Daft Punk - Around the World', titleDim: false, bitrate: 128, samplerate: 44100, channels: 2, elapsed: 8.4 };
const LONG_TITLE = 'Superman Lovers - Starlight (Midnight Train to Paris Extended Club Mix) [Remastered 2024]';

const fixtures = {};

function player(name, description, { state = {}, ...rest } = {}) {
    fixtures[name] = {
        window: 'player',
        description,
        drawer: false,
        state: playerState(state),
        playlist: [],
        frame: null,
        shortcuts: SHORTCUTS,
        update: UPDATE,
        now: NOW,
        actions: [],
        ...rest,
    };
}

const cartridge = (slot) => `#cartridges > .cartridge:nth-child(${slot + 1})`;
const row = (n) => `#playlist > .row:nth-of-type(${n})`;

/* Player: transport and display */

player('empty', 'Nothing loaded: six empty cartridges, the "Load a cartridge to start" hint, Play, Pause and Stop disabled.', {
    state: { cartridges: EMPTY, activeSlot: null, station: 'No cartridge loaded', title: 'Load a cartridge to start', volume: 50 },
    scales: ALL,
});
player('stopped', 'Cartridges loaded, the first one active (accent fill), stopped: "Stopped" dim, idle clock, Play enabled.', { scales: ALL });
player('stopped-none-active', 'Cartridges loaded but none active yet (first launch after loading): Play disabled.', {
    state: { activeSlot: null, station: 'No cartridge loaded', title: 'Load a cartridge to start' },
});
for (const [suffix, time, label] of [['', 0.45, 'mid-sweep'], ['-loader-early', 0.05, 'pulse entering on the left'], ['-loader-late', 0.95, 'pulse leaving on the right']]) {
    player('connecting' + suffix, `Connecting: "Connecting…" dim, play icon on the clock, the loader (row-of-blocks pulse) on the analyzer, ${label}; the active cartridge shows its equalizer.`, {
        state: { state: 'connecting', title: 'Connecting…', titleDim: true },
        frame: { shape: 'empty', loader: 1, loaderTime: time },
        scales: suffix ? undefined : ALL,
    });
}
player('playing', 'Playing a cartridge, title fits, 128 kbps 44.1 kHz stereo, classic bars. Variants freeze the cartridge equalizer at 0 and 450 ms.', {
    state: PLAYING,
    frame: { shape: 'music' },
    times: [0, 450],
    scales: ALL,
});
player('playing-marquee', 'Playing with a title wider than its line: it scrolls back and forth (marquee). Variants at 0 %, 12 % (end of the hold), 50 % and 100 % of one sweep.', {
    state: { ...PLAYING, title: LONG_TITLE, elapsed: 754 },
    frame: { shape: 'music' },
    times: ['0%', '12%', '50%', '100%'],
    scales: ALL,
});
player('playing-hours', 'Playing for over an hour: the clock shows h:mm:ss.', { state: { ...PLAYING, elapsed: 3725.2 }, frame: { shape: 'music' } });
player('buffering-reconnecting', 'The stream dropped and is reconnecting: state buffering, "Reconnecting…" dim, loader over falling bars.', {
    state: { ...PLAYING, state: 'buffering', title: 'Reconnecting…', titleDim: true, elapsed: 125 },
    frame: { shape: 'quiet', loader: 1, loaderTime: 0.3 },
});
player('buffering', 'Buffering without a reconnect (a file waiting for data): the title stays, the loader crossfades in (level 0.5).', {
    state: { ...PLAYING, state: 'buffering', elapsed: 42 },
    frame: { shape: 'music', loader: 0.5, loaderTime: 0.6 },
});
player('paused', 'Paused: "(paused)" after the title, pause icon on the clock, Play enabled and Pause disabled, the bars falling.', {
    state: { ...PLAYING, state: 'paused', title: 'Daft Punk - Around the World (paused)', elapsed: 83 },
    frame: { shape: 'quiet' },
    scales: ALL,
});
player('notice-connection-lost', 'Stopped by itself after reconnecting ran out: "Connection lost" in place of "Stopped".', { state: { title: 'Connection lost' } });
player('notice-couldnt-play', 'A stream that never played: "Couldn\'t play this stream".', { state: { title: "Couldn't play this stream" } });

/* Display details */

player('specs-mono', 'Mono with a bitrate: 64 kbps, 22.05 kHz shown as 22.1, MONO lit.', { state: { ...PLAYING, bitrate: 64, samplerate: 22050, channels: 1 }, frame: { shape: 'music' } });
player('specs-stereo-no-bitrate', 'Stereo without a bitrate (header lacks it): "--" kbps, 48 kHz.', { state: { ...PLAYING, bitrate: null, samplerate: 48000, channels: 2 }, frame: { shape: 'music' } });
player('specs-mono-no-bitrate', 'Mono without a bitrate.', { state: { ...PLAYING, bitrate: null, samplerate: 32000, channels: 1 }, frame: { shape: 'music' } });
player('specs-unknown', 'Playing before any detail is known: "--" kbps, "--" kHz, neither channel lit.', { state: { ...PLAYING, bitrate: null, samplerate: null, channels: null }, frame: { shape: 'music' } });
player('long-names', 'Long cartridge names and hosts (ellipsis), a long station line.', {
    state: {
        ...PLAYING,
        station: 'Radio Paradise - Main Mix (320 kbps AAC, listener supported, commercial free)',
        cartridges: [
            { title: 'Radio Paradise - Main Mix (320k AAC)', url: 'http://stream.radioparadise.com/aac-320', preload: null },
            { title: 'Nightride FM Synthwave Radio', url: 'https://stream.nightride.fm/nightride.m4a', preload: null },
            C.paradise,
            { title: 'W', url: 'http://a-very-long-host-name-for-a-stream.example-broadcasting.com:8000/live', preload: null },
            null,
            C.kexp,
        ],
    },
    frame: { shape: 'music' },
});

/* Analyzer colors */

for (const colors of ['classic', 'white', 'accent', 'off']) {
    player(`colors-${colors}`, `Analyzer colors "${colors}" with a representative frame of bars and peaks${colors === 'off' ? ' (off: Rust sends no bars, the screen stays black)' : ''}.`, {
        state: { ...PLAYING, colors },
        frame: { shape: 'music' },
        scales: colors === 'off' ? undefined : ALL,
    });
}
for (const colors of ['classic', 'accent']) {
    player(`colors-${colors}-full`, `Analyzer "${colors}" with every bar at 15 rows and peaks at the top row: the whole row palette.`, { state: { ...PLAYING, colors }, frame: { shape: 'full' } });
}
for (const colors of ['classic', 'white']) {
    player(`loader-crossfade-${colors}`, `Loader crossfading with the bars (frame level 0.5): bars at 50 % opacity under the pulse at 50 % (${colors}).`, {
        state: { ...PLAYING, state: 'buffering', colors, elapsed: 61 },
        frame: { shape: 'music', loader: 0.5, loaderTime: 0.5 },
    });
}
player('colors-off-loading', 'Analyzer off while connecting: the loader still shows.', {
    state: { state: 'connecting', title: 'Connecting…', titleDim: true, colors: 'off' },
    frame: { shape: 'empty', loader: 1, loaderTime: 0.45 },
});

/* Instant switch */

player('instant-stopped', 'Instant switch on while stopped: every loaded cartridge streams; bolts solid once live, pulsing while loading (variants at 0, 400 and 800 ms of the pulse).', {
    state: {
        instant: true,
        cartridges: [{ ...C.c9, preload: 'live' }, { ...C.fip, preload: 'live' }, { ...C.paradise, preload: 'loading' }, { ...C.groove, preload: 'live' }, null, { ...C.kexp, preload: 'loading' }],
    },
    times: [0, 400, 800],
});
player('instant-playing', 'Instant switch on while playing the second cartridge: the others show bolts, the active one its equalizer.', {
    state: {
        ...PLAYING,
        instant: true,
        activeSlot: 1,
        station: 'FIP',
        title: 'Thelonious Monk - Round Midnight',
        cartridges: [{ ...C.c9, preload: 'live' }, { ...C.fip, preload: null }, { ...C.paradise, preload: 'live' }, { ...C.groove, preload: 'loading' }, null, { ...C.kexp, preload: 'live' }],
    },
    frame: { shape: 'music' },
    scales: ALL,
});

/* Seek bar (a playlist file with a duration) */

const FILE = { ...PLAYING, activeSlot: null, activeId: 3, station: 'Playlist · 3 of 6', title: 'Digital Love', bitrate: 1011, samplerate: 44100, channels: 2, duration: 301.2, seekable: true };
for (const [name, position] of [['start', 0], ['quarter', 75.3], ['middle', 150.6], ['end', 301.2]]) {
    player(`seek-${name}`, `Playing a playlist file: the seek bar over volume and balance side by side, at ${position} s of 5:01.`, {
        state: { ...FILE, position, elapsed: position },
        playlist: PLAYLIST,
        frame: { shape: 'music' },
        scales: name === 'middle' ? ALL : undefined,
    });
}
player('seek-disabled', 'A playlist file selected but stopped: the seek bar shows (duration known) but is disabled at 0.', {
    state: { ...FILE, state: 'stopped', title: 'Stopped', titleDim: true, seekable: false, position: null, elapsed: 0, bitrate: null, samplerate: null, channels: null },
    playlist: PLAYLIST,
});
player('seek-hours', 'A file over an hour long: h:mm:ss in the seek text.', {
    state: { ...FILE, activeId: 6, station: 'Playlist · 6 of 6', title: PLAYLIST[5].title, duration: 3725, position: 1801, elapsed: 1801, bitrate: 128, samplerate: 44100 },
    playlist: PLAYLIST,
    frame: { shape: 'music' },
});
player('seek-dragging', 'Dragging the seek bar (pointer held on its rail at 70 %): the thumb\'s inner dot shrinks, the time follows the pointer.', {
    state: { ...FILE, position: 75.3, elapsed: 75.3 },
    playlist: PLAYLIST,
    frame: { shape: 'music' },
    actions: [{ press: { selector: '#seekSlider .slider-rail', fx: 0.7 } }],
});

/* Sliders */

player('volume-0', 'Volume 0: the muted speaker icon, "0%".', { state: { volume: 0 } });
player('volume-low', 'Volume 25: the one-wave speaker icon (below 50).', { state: { volume: 25, balance: -60 } });
player('volume-high', 'Volume 100 and balance right 45.', { state: { volume: 100, balance: 45 } });
player('balance-left', 'Balance full left: "L 100".', { state: { volume: 50, balance: -100 } });
player('slider-hover', 'The pointer over the volume slider: the thumb\'s inner dot grows (scale 1.167).', { state: PLAYING, frame: { shape: 'music' }, actions: [{ force: { '#volumeSlider': ['hover'] } }] });
player('slider-dragging', 'Dragging the volume slider (pointer held at 30 % of the rail): the inner dot shrinks (scale 0.833).', {
    state: PLAYING,
    frame: { shape: 'music' },
    actions: [{ press: { selector: '#volumeSlider .slider-rail', fx: 0.3 } }],
});
player('balance-snap', 'Balance dragged to 3 % right of center: within 4 % it snaps to C.', {
    state: { balance: 30 },
    actions: [{ press: { selector: '#balanceSlider .slider-rail', fx: 0.515 } }],
});

/* Buttons and focus */

const STATES_SET = (states) => ({
    '#playButton': states,
    '#pauseButton': states,
    '#infoButton': states,
    [`${cartridge(1)} > .cartridge-body`]: states,
    [`${cartridge(0)} > .cartridge-eject`]: states,
    [`${cartridge(4)} > .cartridge-body`]: states,
    '#addUrlButton': states,
});
player('buttons-hover', 'Hover (forced) on Play (accent), Pause (disabled), Stream information, cartridge 2, the active cartridge\'s eject strip, the empty cartridge 5 and Add URL.', {
    actions: [{ force: STATES_SET(['hover']) }],
});
player('buttons-pressed', 'Pressed (:active, forced) on the same controls as buttons-hover.', { actions: [{ force: STATES_SET(['hover', 'active']) }] });
player('buttons-focus', 'Keyboard focus rings (:focus-visible, forced) on Play, cartridge 3, the active cartridge\'s eject, Playlist and the balance slider.', {
    actions: [{ force: { '#playButton': ['focus', 'focus-visible'], [`${cartridge(2)} > .cartridge-body`]: ['focus', 'focus-visible'], [`${cartridge(0)} > .cartridge-eject`]: ['focus', 'focus-visible'], '#playlistButton': ['focus', 'focus-visible'], '#balanceSlider': ['focus', 'focus-visible'] } }],
    scales: ALL,
});
player('buttons-hover-playing', 'Playing: hover on Pause and Stop, on the active (playing) cartridge, and pressed on Settings.', {
    state: PLAYING,
    frame: { shape: 'music' },
    actions: [{ force: { '#pauseButton': ['hover'], '#stopButton': ['hover'], [`${cartridge(0)} > .cartridge-body`]: ['hover'], '#settingsButton': ['hover', 'active'] } }],
});
player('instant-hover', 'Instant switch on (accent toggle), hovered; Play pressed.', {
    state: { instant: true, cartridges: [{ ...C.c9, preload: 'live' }, { ...C.fip, preload: 'live' }, { ...C.paradise, preload: 'live' }, { ...C.groove, preload: 'live' }, null, { ...C.kexp, preload: 'live' }] },
    actions: [{ force: { '#instantButton': ['hover'], '#playButton': ['hover', 'active'] } }],
});
player('update-dot', 'A downloaded update is waiting: the dot on the Settings cog.', { update: { ...UPDATE, status: 'ready', version: '1.2.0' } });

/* Cartridge drag and drop */

player('cartridge-drag', 'Dragging the first cartridge (held) over the third: the ghost follows the pointer (scale 1.04, shadow), the source is faded, the target outlined.', {
    actions: [{ press: cartridge(0) }, { dragTo: { selector: cartridge(2), dx: -20, dy: 6 }, steps: 6 }],
    scales: ALL,
});
player('cartridge-drag-empty', 'Dragging the second cartridge over the empty fifth slot while the first plays.', {
    state: PLAYING,
    frame: { shape: 'music' },
    actions: [{ press: cartridge(1) }, { dragTo: { selector: cartridge(4), dx: 10, dy: -8 }, steps: 6 }],
});
player('cartridge-dropped', 'Just dropped: the first cartridge swapped with the third, both play the "dropped" scale-in (0.25 s). Variants at 0, 60, 125 and 250 ms.', {
    actions: [
        { emit: 'player-state', payload: playerState({ cartridges: [C.paradise, C.fip, C.c9, C.groove, null, C.kexp], activeSlot: 2 }) },
        { eval: 'moveCartridge(0, 2)' },
    ],
    times: [0, 60, 125, 250],
});

/* Playlist drawer */

const drawer = (name, description, options = {}) => player(name, description, { drawer: true, playlist: PLAYLIST, ...options });
drawer('drawer-empty', 'Drawer open with no tracks: the loaded cartridges pinned above a divider, then the drop hint.', { playlist: [], scales: ALL });
drawer('drawer-empty-no-cartridges', 'Drawer open with nothing at all: only the drop hint.', { playlist: [], state: { cartridges: EMPTY, activeSlot: null, station: 'No cartridge loaded', title: 'Load a cartridge to start', volume: 50 } });
drawer('drawer-entries', 'Drawer with six tracks: files with durations, a stream (host instead of a duration), a missing file (struck through), a long title; the first cartridge active (its pinned row in accent).', { scales: ALL });
drawer('drawer-playing-entry', 'Playing the third track: its row is active and shows the animated equalizer in place of its number (variants at 0 and 450 ms).', {
    state: { ...FILE, position: 61, elapsed: 61 },
    frame: { shape: 'music' },
    times: [0, 450],
});
drawer('drawer-active-stopped', 'The third track active but stopped: accent row without the equalizer.', {
    state: { ...FILE, state: 'stopped', title: 'Stopped', titleDim: true, seekable: false, position: null, elapsed: 0, bitrate: null, samplerate: null, channels: null },
});
drawer('drawer-select-single', 'One track selected: the selection fill and the accent pill on its left.', { actions: [{ eval: 'select(2)' }] });
drawer('drawer-select-multi', 'Shift-selection over four tracks (2 to 5) plus a Ctrl-selected cartridge row: "5 selected" in the status line.', {
    actions: [{ eval: 'select(2)' }, { eval: 'select(5, { extend: true })' }, { eval: "select('slot1', { toggle: true })" }],
    scales: ALL,
});
drawer('drawer-row-hover', 'Hover on an unselected row and on a selected one.', {
    actions: [{ eval: 'select(3)' }, { force: { [row(4)]: ['hover'], [row(9)]: ['hover'] } }],
});
drawer('drawer-many', 'Sixty tracks: the list scrolls (custom 10 px scrollbar), at the top.', { playlist: MANY });
drawer('drawer-many-scrolled', 'Sixty tracks scrolled to the middle, one selected, the 25th playing.', {
    playlist: MANY,
    state: { ...FILE, activeId: 124, station: 'Playlist · 25 of 60', title: MANY[24].title, duration: MANY[24].duration, position: 40, elapsed: 40 },
    frame: { shape: 'music' },
    actions: [{ eval: 'select(127)' }, { scroll: '#playlist', top: 640 }],
});
drawer('drop-line-before', 'The insertion line before the third track (showDropIndex(2), as shown while files or dragged tracks hover there).', {
    actions: [{ eval: 'showDropIndex(2)' }],
});
drawer('drop-line-after', 'Insertion line after the last track (drop at the end).', { actions: [{ eval: 'showDropIndex(entries.length)' }] });
drawer('drop-empty', 'Files dragged over an empty drawer: the drop hint turns accent.', {
    playlist: [],
    actions: [{ dragDrop: { type: 'over', position: { x: 430, y: 420 }, paths: ['C:\\Users\\Me\\Music\\a.mp3'] } }],
});
drawer('drop-over-rows', 'Files dragged over the rows through the real drag-drop path (position between the 2nd and 3rd tracks).', {
    actions: [{ eval: "window.__dropY = (() => { const r = rowOf.get(3).getBoundingClientRect(); return r.top + 2; })()" }, { eval: "window.__REF__.dragDrop({ type: 'over', position: { x: 430 * devicePixelRatio, y: window.__dropY * devicePixelRatio }, paths: ['C:\\\\a.mp3'] })" }],
});
drawer('reorder-drag', 'Reordering: the second track pressed and dragged down (pointer held): the selection moves, rows show the grabbing cursor and the insertion line.', {
    actions: [{ press: { selector: row(8), fx: 0.4 } }, { dragTo: { selector: row(11), fx: 0.4, fy: 0.1 }, steps: 8 }],
});
drawer('drawer-long-durations', 'Track numbers and durations with many digits (hour-long tracks, 3-digit numbers are not reached).', {
    playlist: PLAYLIST.map((entry, index) => ({ ...entry, duration: [36000, 3599, 59.6, null, null, 7322][index] })),
});

/* Dialogs */

player('dialog-load', 'Load cartridge (an empty slot): Name focused (accent underline), empty fields with placeholders, no Eject.', { actions: [{ eval: 'openDialog(4)' }], scales: ALL });
player('dialog-edit-cartridge', 'Edit cartridge: filled fields and the Eject button. Variants freeze the opening animation (0.25 s scale and fade) at 0, 60, 125 ms and its end.', {
    actions: [{ eval: 'openDialog(0)' }],
    times: [0, 60, 125, 250],
});
player('dialog-add-url', 'Add URL: "Name (optional)", the URL field focused, "Add".', { actions: [{ eval: 'openUrlDialog()' }] });
drawer('dialog-edit-track-file', 'Edit track for a local file: the path read-only (no fill, secondary text), over the open drawer.', { actions: [{ eval: 'openEntryDialog(1)' }] });
drawer('dialog-edit-track-url', 'Edit track for a stream entry: an editable URL.', { actions: [{ eval: 'openEntryDialog(4)' }] });
player('dialog-error', 'Add URL refused by Rust: the error under the fields, focus back on the URL.', {
    errors: { playlist_add_url: 'Enter an address starting with http:// or https://' },
    actions: [{ eval: 'openUrlDialog()' }, { eval: "ui.streamURL.value = 'stream.example.com/live'" }, { eval: 'ui.dialogForm.requestSubmit()' }, { wait: 50 }],
});
player('dialog-buttons', 'Dialog buttons: Save hovered, Eject pressed, Cancel with the focus ring; a textbox hovered.', {
    actions: [{ eval: 'openDialog(0)' }, { force: { '#saveButton': ['hover'], '#ejectButton': ['hover', 'active'], '#cancelButton': ['focus', 'focus-visible'], '#streamURL': ['hover'] } }],
});

/* Second accent */

player('accent-orange-playing', 'A very different accent (illustrative orange palette) while playing on instant switch: what follows the accent (cartridge, channel badge, state icon, slider fills, Play, bolts, analyzer accent mode).', {
    accent: ORANGE,
    state: { ...PLAYING, colors: 'accent', instant: true, cartridges: [{ ...C.c9, preload: null }, { ...C.fip, preload: 'live' }, { ...C.paradise, preload: 'loading' }, { ...C.groove, preload: 'live' }, null, { ...C.kexp, preload: 'live' }] },
    frame: { shape: 'music' },
});
drawer('accent-orange-drawer', 'Orange accent: drawer with a selection, the playing row and a focused textbox dialog.', {
    accent: ORANGE,
    state: { ...FILE, position: 150, elapsed: 150 },
    frame: { shape: 'music' },
    actions: [{ eval: 'select(2)' }, { eval: 'showDropIndex(4)' }],
});

/* Context menus (native: recorded in the layout file's "menus", the render shows the selection they cause) */

const menu = (name, description, options) => player(name, description, { themes: ['dark'], ...options });
menu('menu-player-stopped', 'Right-click on the player area while stopped: Play (with the shortcut) / Stop (disabled) / Stream information / Settings.', { actions: [{ contextmenu: '#screen' }] });
menu('menu-player-playing', 'Right-click on the player area while playing: Pause / Stop.', { state: PLAYING, frame: { shape: 'music' }, actions: [{ contextmenu: '.transport' }] });
menu('menu-player-nothing', 'Right-click with nothing loaded: Play and Stop disabled.', { state: { cartridges: EMPTY, activeSlot: null, station: 'No cartridge loaded', title: 'Load a cartridge to start' }, actions: [{ contextmenu: '#screen' }] });
menu('menu-cartridge', 'Right-click on a loaded cartridge: Play / Edit… / Eject.', { actions: [{ contextmenu: cartridge(1) }] });
menu('menu-cartridge-empty', 'Right-click on an empty cartridge: Load cartridge….', { actions: [{ contextmenu: cartridge(4) }] });
menu('menu-row-track', 'Right-click on an unselected track: it becomes the selection; Play / Edit… / Remove / Add file… / Add URL… / Clear playlist.', { drawer: true, playlist: PLAYLIST, actions: [{ contextmenu: row(9) }] });
menu('menu-row-tracks', 'Right-click inside a 3-track selection: "Remove 3 tracks".', { drawer: true, playlist: PLAYLIST, actions: [{ eval: 'select(1)' }, { eval: 'select(3, { extend: true })' }, { contextmenu: row(8) }] });
menu('menu-row-cartridge', 'Right-click on a pinned cartridge row: Play / Edit cartridge… / Add file… / Add URL… / Clear playlist.', { drawer: true, playlist: PLAYLIST, actions: [{ contextmenu: row(2) }] });
menu('menu-drawer-empty', 'Right-click on the empty drawer: Add file… / Add URL… / Clear playlist (disabled).', { drawer: true, playlist: [], actions: [{ contextmenu: '.playlist-empty' }] });
menu('menu-shortcut-unavailable', 'Right-click on the player while stopped with the Play shortcut owned by another app: no accelerator on Play.', {
    shortcuts: { ...SHORTCUTS, play: { accelerator: 'Ctrl+Alt+Insert', registered: false } },
    actions: [{ contextmenu: '#screen' }],
});

/* Settings */

const SETTINGS = { theme: 'system', 'visualizer-colors': 'classic', autoplay: false, 'reconnect-minutes': 5 };
function settings(name, description, rest = {}) {
    fixtures[name] = {
        window: 'settings',
        description,
        settings: SETTINGS,
        shortcuts: SHORTCUTS,
        update: UPDATE,
        background: { available: true, enabled: false, startsWithWindows: false },
        version: '1.1.5',
        now: NOW,
        actions: [],
        ...rest,
    };
}
settings('settings-default', 'Settings as first opened: system theme, classic colors, Play on launch off, reconnect 5 minutes, helper off, default shortcuts (Play/Stop/Start unassigned), checked 2 h ago.', { scales: ALL });
settings('settings-scrolled', 'The settings list is taller than the 850 px window: scrolled to the bottom (About), with the 8 px scrollbar.', {
    actions: [{ scroll: 'body', top: 10000 }],
});
settings('settings-changed', 'Every choice changed: dark theme, accent colors, play on launch on, reconnect until stopped, helper on, every shortcut assigned (Win, F-key).', {
    settings: { theme: 'dark', 'visualizer-colors': 'accent', autoplay: true, 'reconnect-minutes': 0 },
    background: { available: true, enabled: true, startsWithWindows: true },
    shortcuts: { play: { accelerator: 'Ctrl+Alt+Insert', registered: true }, stop: { accelerator: 'Super+Shift+Digit1', registered: true }, toggle: { accelerator: 'F9', registered: true } },
});
settings('settings-light-colors', 'Light theme chosen, white colors, 1 hour.', { settings: { theme: 'light', 'visualizer-colors': 'white', autoplay: false, 'reconnect-minutes': 60 } });
settings('settings-shortcut-recording', 'Recording the Stop shortcut: dashed accent box, "Press a shortcut…", the hint "Esc to cancel, Backspace to reset".', { actions: [{ click: '[data-action="stop"]' }] });
settings('settings-shortcut-error', 'A recorded shortcut refused: the error in red under Play / Stop / Start.', {
    errors: { set_shortcut: 'Already used for another action' },
    actions: [{ click: '[data-action="toggle"]' }, { key: { key: 'h', code: 'KeyH', modifiers: 3 } }, { wait: 50 }],
});
settings('settings-shortcut-unavailable', 'The Play shortcut owned by another app at startup: crossed-out grey keys and the red hint.', { shortcuts: { ...SHORTCUTS, play: { accelerator: 'Ctrl+Alt+Insert', registered: false } } });
settings('settings-shortcuts-unassigned', 'No shortcut at all: "Not set" in the three boxes.', {
    shortcuts: { play: { accelerator: '', registered: false }, stop: { accelerator: '', registered: false }, toggle: { accelerator: '', registered: false } },
});
settings('settings-background-unavailable', 'Not the installed version: the helper toggle disabled and faded, "Works in the installed version of Qust."', {
    background: { available: false, enabled: false, startsWithWindows: false },
});
settings('settings-background-error', 'Turning the helper on failed: the toggle back off and the error in red.', {
    errors: { set_background: "Couldn't change the Windows startup entry: Access is denied. (os error 5)" },
    actions: [{ click: '#backgroundToggle + .toggle-track' }, { wait: 50 }],
});
const UPDATES = {
    'not-installed': [{ installed: false }, 'A debug or portable build: "Updates work in the installed version of Qust.", button and auto toggle disabled.'],
    'never-checked': [{ lastCheck: null }, '"Not checked yet."'],
    checking: [{ status: 'checking' }, 'Checking: button disabled.'],
    downloading: [{ status: 'downloading', version: '1.2.0', progress: 42 }, 'Downloading 42 %: button disabled.'],
    ready: [{ status: 'ready', version: '1.2.0' }, 'Ready: "Restart and update" in accent.'],
    installing: [{ status: 'installing', version: '1.2.0' }, 'Installing: button disabled.'],
    error: [{ status: 'error', error: "Couldn't reach the update server: timed out" }, 'Error in red, "Try again".'],
    current: [{ status: 'current', lastCheck: NOW - 20 }, '"You\'re up to date. Last checked just now."'],
    'days-ago': [{ status: 'current', lastCheck: NOW - 3 * 86400 }, 'Checked 3 days ago.'],
    'auto-off': [{ auto: false, lastCheck: NOW - 25 * 60 }, 'Check automatically off; checked 25 min ago.'],
};
for (const [suffix, [over, description]] of Object.entries(UPDATES)) settings(`settings-update-${suffix}`, `Updates: ${description}`, { update: { ...UPDATE, ...over } });
settings('settings-hover', 'Hover (forced) on the Theme combo, the Play on launch toggle (off), the helper toggle (on), the Play shortcut and the update button.', {
    background: { available: true, enabled: true, startsWithWindows: true },
    actions: [{ force: { '#themeCombo': ['hover'], '.setting:nth-of-type(3) .toggle': ['hover'], '.setting:nth-of-type(5) .toggle': ['hover'], '[data-action="play"]': ['hover'], '#updateButton': ['hover'] } }],
});
settings('settings-pressed', 'Pressed (forced :active) on the Visualizer colors combo and the update button.', {
    actions: [{ force: { '#colorsCombo': ['hover', 'active'], '#updateButton': ['hover', 'active'] } }],
});
settings('settings-focus', 'Keyboard focus rings: the Reconnect combo, the Play on launch toggle (ring on the track), the Stop shortcut and the update button.', {
    actions: [{ force: { '#reconnectCombo': ['focus', 'focus-visible'], '#autoplayToggle': ['focus', 'focus-visible'], '[data-action="stop"]': ['focus', 'focus-visible'], '#updateButton': ['focus', 'focus-visible'] } }],
});
settings('settings-toggle-switching', 'Play on launch just switched on: the thumb\'s slide (0.15 s) frozen at 0, 40 and 150 ms.', {
    actions: [{ click: '#autoplayToggle + .toggle-track' }],
    times: [0, 40, 150],
    transitions: true,
});
settings('settings-accent-orange', 'Settings with the orange accent: toggles on, a ready update, kbd keys.', {
    accent: ORANGE,
    settings: { ...SETTINGS, autoplay: true },
    background: { available: true, enabled: true, startsWithWindows: true },
    update: { ...UPDATE, status: 'ready', version: '1.2.0' },
});

/* Stream information */

const STREAM_INFO = {
    title: 'Superman Lovers - Starlight',
    name: 'C9 Radio',
    genre: 'Electronic, House',
    description: 'La radio electro de la Côte d\'Azur',
    website: 'https://www.c9.fr',
    server: 'Icecast 2.4.4',
    contentType: 'audio/mpeg',
    format: 'MP3',
    bitrate: 128,
    rate: 131,
    samplerate: 44100,
    channels: 2,
    metaint: 16000,
    metaBytes: 18432,
    bytes: 12_345_678,
    titleChanges: 4,
    reconnects: 1,
    connectedSeconds: 754,
};
function info(name, description, rest = {}) {
    fixtures[name] = { window: 'info', description, status: null, info: {}, actions: [], ...rest };
}
const STATUS = { state: 'playing', station: 'C9 Radio', url: 'http://stream.c9.fr/c9-128.mp3', buffer: 2.43, stalls: 2, statusTitle: null };
info('info-stopped', 'Nothing playing: "Not playing", the hint, Stopped badge, every value an em dash.', { scales: ALL });
info('info-playing-stream', 'A stream with full ICY metadata: Playing badge in accent.', { status: STATUS, info: STREAM_INFO, scales: ALL });
info('info-playing-file', 'A local FLAC file: no ICY metadata ("None" interval), no host.', {
    status: { ...STATUS, station: 'Digital Love', url: MUSIC + '03 Digital Love.flac', buffer: 0, stalls: 0 },
    info: { format: 'FLAC', bitrate: 1011, samplerate: 44100, channels: 2, bytes: 38_052_118, metaBytes: 0, connectedSeconds: 61, reconnects: 0, titleChanges: 0 },
});
info('info-status-title', 'A stream without ICY titles: the title from the Icecast status page, metadata interval "None", AAC+ from the content type, bitrate 0 shown as a dash, 6 channels.', {
    status: { ...STATUS, station: 'FIP', url: 'https://icecast.radiofrance.fr/fip-hifi.aac', statusTitle: 'Thelonious Monk - Round Midnight' },
    info: { name: 'FIP', contentType: 'audio/aacp', bitrate: 0, rate: 192, samplerate: 48000, channels: 6, metaint: 0, metaBytes: 0, bytes: 812_000, titleChanges: 0, reconnects: 0, connectedSeconds: 3725, server: 'Icecast 2.4.0-kh15' },
});
info('info-long-values', 'Long values truncated with an ellipsis (title, genre, description, website, server).', {
    status: { ...STATUS, station: 'Radio Paradise - Main Mix (320 kbps AAC, listener supported, commercial free)' },
    info: {
        ...STREAM_INFO,
        title: LONG_TITLE,
        name: 'Radio Paradise - Main Mix (320 kbps AAC, listener supported, commercial free)',
        genre: 'Eclectic, Rock, World, Electronica, Ambient, Folk, Jazz, Classical, Blues',
        description: 'An eclectic online mix of rock, pop, world, electronica and more, DJ-curated and listener-supported since the year 2000',
        website: 'https://radioparadise.com/listen/stations/main-mix?utm_source=icecast&utm_medium=stream',
        server: 'Icecast 2.4.99.2 (Radio Paradise build with extended statistics)',
        contentType: 'audio/aac; charset=utf-8; codecs="mp4a.40.2"',
        format: null,
        bitrate: 320,
        metaBytes: 2_345_678,
        bytes: 987_654_321,
    },
});
for (const state of ['connecting', 'buffering', 'paused']) {
    info(`info-${state}`, `The state badge while ${state} (only "playing" is in accent).`, { status: { ...STATUS, state }, info: STREAM_INFO });
}

/* Write */

fs.mkdirSync(OUT, { recursive: true });
let count = 0;
for (const [name, fixture] of Object.entries(fixtures)) {
    const clean = JSON.parse(JSON.stringify(fixture));
    fs.writeFileSync(path.join(OUT, name + '.json'), JSON.stringify(clean, null, 2) + '\n');
    count++;
}
console.log(`${count} fixtures written to ${path.relative(process.cwd(), OUT) || OUT}`);
