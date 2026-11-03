// The settings window. Every change is saved and broadcast as a `settings-changed` event, which
// the player (script.js) applies right away.

const THEMES = { system: 'Use system setting', light: 'Light', dark: 'Dark' };
const COLORS = { classic: 'Classic', white: 'White', accent: 'Accent color' };

const { Menu } = window.__TAURI__.menu;
const { LogicalPosition } = window.__TAURI__.dpi;
const { emit } = window.__TAURI__.event;
const settingsWindow = window.__TAURI__.window.getCurrentWindow();

const themeCombo = document.getElementById('themeCombo');
const colorsCombo = document.getElementById('colorsCombo');
const autoplayToggle = document.getElementById('autoplayToggle');
const autoplayLabel = document.getElementById('autoplayLabel');

let settings = { theme: 'system', colors: 'classic', autoplay: false };

function render() {
    themeCombo.firstElementChild.textContent = THEMES[settings.theme];
    colorsCombo.firstElementChild.textContent = COLORS[settings.colors];
    autoplayToggle.checked = settings.autoplay;
    autoplayLabel.textContent = settings.autoplay ? 'On' : 'Off';
}

function change(key, storageKey, value) {
    settings[key] = value;
    saveData(storageKey, value);
    emit('settings-changed', { key, value });
    render();
}

// Choices open as a native menu under the button, like a Windows combo box.
async function showChoices(button, choices, current, choose) {
    const rect = button.getBoundingClientRect();
    const items = Object.entries(choices).map(([value, text]) => ({
        text,
        checked: value === current,
        action: () => choose(value),
    }));
    const menu = await Menu.new({ items });
    await menu.popup(new LogicalPosition(rect.left, rect.bottom + 2));
}

themeCombo.addEventListener('click', () => showChoices(themeCombo, THEMES, settings.theme, (theme) => {
    settings.theme = theme;
    invoke('set_theme', { theme });
    render();
}));
colorsCombo.addEventListener('click', () => showChoices(colorsCombo, COLORS, settings.colors, (value) => change('colors', 'visualizer-colors', value)));
autoplayToggle.addEventListener('change', () => change('autoplay', 'autoplay', autoplayToggle.checked));

/* Global shortcuts */

const MODIFIER_CODES = ['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight'];
const shortcutButtons = [...document.querySelectorAll('.shortcut')];
let shortcuts = {};
let recording = null;

function renderShortcuts(errors = {}) {
    for (const button of shortcutButtons) {
        const action = button.dataset.action;
        const binding = shortcuts[action];
        const status = document.querySelector(`[data-status="${action}"]`);
        const isRecording = recording === action;
        const assigned = !!binding && !!binding.accelerator;
        button.classList.toggle('is-recording', isRecording);
        button.classList.toggle('is-unavailable', assigned && !binding.registered && !isRecording);
        button.classList.toggle('is-unassigned', !assigned && !isRecording);
        button.replaceChildren(...(isRecording
            ? [document.createTextNode('Press a shortcut…')]
            : assigned ? shortcutKeys(binding.accelerator).map((key) => Object.assign(document.createElement('kbd'), { textContent: key }))
            : [document.createTextNode('Not set')]));

        // The description doubles as the status line; its original text is the hint.
        if (!status.dataset.hint) status.dataset.hint = status.textContent;
        let text = status.dataset.hint;
        let error = false;
        if (isRecording) text = 'Esc to cancel, Backspace to reset';
        else if (errors[action]) [text, error] = [errors[action], true];
        else if (assigned && !binding.registered) [text, error] = ['Another app is using this shortcut. Pick a different one.', true];
        status.textContent = text;
        status.classList.toggle('is-error', error);
    }
}

async function startRecording(action) {
    if (recording) return;
    recording = action;
    renderShortcuts();
    shortcuts = await invoke('pause_shortcuts', { paused: true });
}

async function finishRecording(accelerator) {
    const action = recording;
    recording = null;
    const errors = {};
    if (accelerator !== null) {
        try {
            await invoke('set_shortcut', { action, accelerator });
        } catch (error) {
            errors[action] = String(error);
        }
    }
    shortcuts = await invoke('pause_shortcuts', { paused: false });
    renderShortcuts(errors);
}

shortcutButtons.forEach((button) => button.addEventListener('click', () => startRecording(button.dataset.action)));

document.addEventListener('keydown', (event) => {
    if (!recording) {
        if (event.key === 'Escape') settingsWindow.close();
        return;
    }
    event.preventDefault();
    if (MODIFIER_CODES.includes(event.code)) return;
    if (event.code === 'Escape') return finishRecording(null);
    if (event.code === 'Backspace' || event.code === 'Delete') return finishRecording('');
    const modifiers = [];
    if (event.ctrlKey) modifiers.push('Ctrl');
    if (event.altKey) modifiers.push('Alt');
    if (event.shiftKey) modifiers.push('Shift');
    if (event.metaKey) modifiers.push('Super');
    const standalone = /^F\d+$/.test(event.code) || event.code.startsWith('Media');
    if (!modifiers.length && !standalone) return;
    finishRecording([...modifiers, event.code].join('+'));
});

window.addEventListener('blur', () => {
    if (recording) finishRecording(null);
});

/* Background helper (background.rs) */

const backgroundToggle = document.getElementById('backgroundToggle');
const backgroundLabel = document.getElementById('backgroundLabel');
const backgroundStatus = document.getElementById('backgroundStatus');

// Unavailable outside the installed version: shown off and disabled, since a test or portable
// build shares the installed copy's settings.
function renderBackground(background, error) {
    const available = background.available !== false;
    backgroundToggle.disabled = !available;
    backgroundToggle.checked = available && background.enabled;
    backgroundLabel.textContent = backgroundToggle.checked ? 'On' : 'Off';
    backgroundToggle.closest('.setting').classList.toggle('is-disabled', !available);
    if (!backgroundStatus.dataset.hint) backgroundStatus.dataset.hint = backgroundStatus.textContent;
    backgroundStatus.textContent = error || (available ? backgroundStatus.dataset.hint : 'Works in the installed version of Qust.');
    backgroundStatus.classList.toggle('is-error', !!error);
}

backgroundToggle.addEventListener('change', async () => {
    const enabled = backgroundToggle.checked;
    try {
        renderBackground(await invoke('set_background', { enabled }));
    } catch (error) {
        renderBackground({ enabled: !enabled }, String(error));
    }
});

/* Updates (updater.rs) */

const updateButton = document.getElementById('updateButton');
const updateStatus = document.getElementById('updateStatus');
const autoUpdateToggle = document.getElementById('autoUpdateToggle');
const autoUpdateLabel = document.getElementById('autoUpdateLabel');
let update = {};

function ago(timestamp) {
    const seconds = Math.max(0, Math.floor(Date.now() / 1000) - timestamp);
    if (seconds < 60) return 'just now';
    if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
    if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
    return `${Math.floor(seconds / 86400)} days ago`;
}

function renderUpdate() {
    const checked = update.lastCheck ? `Last checked ${ago(update.lastCheck)}.` : 'Not checked yet.';
    let text = checked;
    let label = 'Check for updates';
    let busy = false;
    let error = false;
    if (!update.installed) {
        text = 'Updates work in the installed version of Qust.';
        busy = true;
    } else if (update.status === 'checking') {
        text = 'Checking for updates…';
        busy = true;
    } else if (update.status === 'downloading') {
        text = `Downloading version ${update.version}… ${update.progress || 0}%`;
        busy = true;
    } else if (update.status === 'ready') {
        text = `Version ${update.version} is ready to install.`;
        label = 'Restart and update';
    } else if (update.status === 'installing') {
        text = 'Installing…';
        busy = true;
    } else if (update.status === 'error') {
        text = `Couldn't update: ${update.error}`;
        label = 'Try again';
        error = true;
    } else if (update.status === 'current') {
        text = `You're up to date. ${checked}`;
    }
    updateStatus.textContent = text;
    updateStatus.classList.toggle('is-error', error);
    updateButton.textContent = label;
    updateButton.disabled = busy;
    updateButton.classList.toggle('accent', update.status === 'ready');
    autoUpdateToggle.disabled = !update.installed;
    autoUpdateToggle.checked = update.auto !== false;
    autoUpdateLabel.textContent = autoUpdateToggle.checked ? 'On' : 'Off';
}

updateButton.addEventListener('click', () => {
    invoke(update.status === 'ready' ? 'install_update' : 'check_for_updates').catch(() => {});
});

autoUpdateToggle.addEventListener('change', () => {
    update.auto = autoUpdateToggle.checked;
    saveData('auto-update', autoUpdateToggle.checked);
    renderUpdate();
});

listen('update-status', ({ payload }) => {
    update = payload;
    renderUpdate();
});

(async function init() {
    const [theme, colors, autoplay, version, bindings] = await Promise.all([
        loadData('theme'),
        loadData('visualizer-colors'),
        loadData('autoplay'),
        window.__TAURI__.app.getVersion(),
        invoke('get_shortcuts'),
    ]);
    shortcuts = bindings;
    renderShortcuts();
    if (theme in THEMES) settings.theme = theme;
    if (colors in COLORS) settings.colors = colors;
    settings.autoplay = autoplay === true;
    document.getElementById('version').textContent = version;
    update = await invoke('update_status');
    renderUpdate();
    renderBackground(await invoke('get_background'));
    render();
})();
