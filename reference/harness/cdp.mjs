// A headless Microsoft Edge (the Chromium build WebView2 is made from) driven over the DevTools
// protocol. Headless only: no window ever appears on the desktop. Its own temporary profile, so it
// never touches the user's Edge, and it is killed on exit.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const EDGE_PATHS = [
    process.env.QUST_REF_EDGE,
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
].filter(Boolean);

export function edgePath() {
    const found = EDGE_PATHS.find((candidate) => fs.existsSync(candidate));
    if (!found) throw new Error('msedge.exe not found; set QUST_REF_EDGE');
    return found;
}

export async function launch() {
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'qust-ref-edge-'));
    const args = [
        '--headless=new',
        '--remote-debugging-port=0',
        `--user-data-dir=${profile}`,
        '--no-first-run',
        '--no-default-browser-check',
        '--disable-extensions',
        '--disable-sync',
        '--disable-background-networking',
        '--disable-component-update',
        '--disable-default-apps',
        '--mute-audio',
        '--force-color-profile=srgb',
        '--allow-file-access-from-files',
        // As main.rs gives every WebView2 (build_window).
        '--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection',
        // Extra flags for experiments (space-separated).
        ...(process.env.QUST_REF_EDGE_ARGS ? process.env.QUST_REF_EDGE_ARGS.split(' ').filter(Boolean) : []),
        'about:blank',
    ];
    const child = spawn(edgePath(), args, { stdio: 'ignore', windowsHide: true });
    const portFile = path.join(profile, 'DevToolsActivePort');
    const started = Date.now();
    while (!fs.existsSync(portFile) || fs.readFileSync(portFile, 'utf8').split('\n').length < 2) {
        if (Date.now() - started > 20000) throw new Error('Edge did not start');
        await new Promise((resolve) => setTimeout(resolve, 50));
    }
    const [port, wsPath] = fs.readFileSync(portFile, 'utf8').trim().split('\n');
    const browser = new Connection(`ws://127.0.0.1:${port}${wsPath}`);
    await browser.open();
    const version = await browser.send('Browser.getVersion');
    const close = async () => {
        // Browser.close may never answer (the socket goes first): don't wait for it.
        await Promise.race([browser.send('Browser.close').catch(() => {}), new Promise((resolve) => setTimeout(resolve, 1500))]);
        browser.ws.close();
        child.kill();
        child.unref();
        for (let i = 0; i < 20; i++) {
            try {
                fs.rmSync(profile, { recursive: true, force: true });
                break;
            } catch (error) {
                await new Promise((resolve) => setTimeout(resolve, 100));
            }
        }
    };
    process.on('exit', () => child.kill());
    return { browser, version, close };
}

export class Connection {
    constructor(url) {
        this.url = url;
        this.id = 0;
        this.pending = new Map();
        this.handlers = new Map();
    }

    open() {
        return new Promise((resolve, reject) => {
            this.ws = new WebSocket(this.url);
            this.ws.onopen = () => resolve();
            this.ws.onerror = (error) => reject(error);
            this.ws.onmessage = (message) => this.receive(JSON.parse(message.data));
        });
    }

    receive(message) {
        if (message.id !== undefined) {
            const pending = this.pending.get(message.id);
            if (!pending) return;
            this.pending.delete(message.id);
            if (message.error) pending.reject(new Error(`${pending.method}: ${message.error.message}`));
            else pending.resolve(message.result);
            return;
        }
        const key = (message.sessionId || '') + ' ' + message.method;
        for (const handler of this.handlers.get(key) || []) handler(message.params);
    }

    send(method, params = {}, sessionId) {
        const id = ++this.id;
        const message = { id, method, params };
        if (sessionId) message.sessionId = sessionId;
        return new Promise((resolve, reject) => {
            this.pending.set(id, { resolve, reject, method });
            this.ws.send(JSON.stringify(message));
        });
    }

    on(method, handler, sessionId = '') {
        const key = sessionId + ' ' + method;
        if (!this.handlers.has(key)) this.handlers.set(key, new Set());
        this.handlers.get(key).add(handler);
        return () => this.handlers.get(key).delete(handler);
    }
}

// One tab, as a flat session on the browser connection.
export class Page {
    constructor(browser, targetId, sessionId) {
        this.browser = browser;
        this.targetId = targetId;
        this.sessionId = sessionId;
    }

    static async create(browser) {
        const { targetId } = await browser.send('Target.createTarget', { url: 'about:blank', newWindow: true, background: false });
        const { sessionId } = await browser.send('Target.attachToTarget', { targetId, flatten: true });
        const page = new Page(browser, targetId, sessionId);
        await page.send('Page.enable');
        await page.send('Runtime.enable');
        await page.send('DOM.enable');
        await page.send('CSS.enable');
        // Every tab renders as if focused and visible (else rAF stops in background tabs).
        await page.send('Emulation.setFocusEmulationEnabled', { enabled: true });
        await page.send('Page.setWebLifecycleState', { state: 'active' });
        return page;
    }

    send(method, params) {
        return this.browser.send(method, params, this.sessionId);
    }

    on(method, handler) {
        return this.browser.on(method, handler, this.sessionId);
    }

    once(method, timeout = 15000) {
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
                off();
                reject(new Error(`timeout waiting for ${method}`));
            }, timeout);
            const off = this.on(method, (params) => {
                clearTimeout(timer);
                off();
                resolve(params);
            });
        });
    }

    // Evaluates an expression in the page's global scope (so script.js's top-level functions and
    // let/const bindings are reachable), awaiting promises.
    async evaluate(expression) {
        const result = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
        if (result.exceptionDetails) {
            const details = result.exceptionDetails;
            throw new Error(`page error: ${details.exception ? details.exception.description : details.text}\n  in: ${expression.slice(0, 200)}`);
        }
        return result.result.value;
    }

    async close() {
        await this.browser.send('Target.closeTarget', { targetId: this.targetId });
    }
}
