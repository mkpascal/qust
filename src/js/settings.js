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

const SHORTCUT_HINT = "Works even when Qust isn't focused";
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
        button.classList.toggle('is-recording', isRecording);
        button.classList.toggle('is-unavailable', !!binding && !binding.registered && !isRecording);
        button.replaceChildren(...(isRecording
            ? [document.createTextNode('Press a shortcut…')]
            : binding ? shortcutKeys(binding.accelerator).map((key) => Object.assign(document.createElement('kbd'), { textContent: key })) : []));

        let text = SHORTCUT_HINT;
        let error = false;
        if (isRecording) text = 'Esc to cancel, Backspace for the default';
        else if (errors[action]) [text, error] = [errors[action], true];
        else if (binding && !binding.registered) [text, error] = ['Another app is using this shortcut. Pick a different one.', true];
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
    render();
})();
