// Writes the release's latest.json, which both updaters read: the Tauri versions of Qust
// (tauri-plugin-updater 2.14) and the native one (crates/updater). Both pick the first key they
// find of `{os}-{arch}-{installer}` then `{os}-{arch}`, so each file is listed under both:
//   windows-x86_64-nsis, windows-x86_64              -> Qust_<version>_x64-setup.exe
//   darwin-{aarch64,x86_64}-app, darwin-{aarch64,x86_64} -> Qust_<version>_universal.app.tar.gz
// Every entry needs a url and a signature, or the Tauri updater refuses the whole file, and
// pub_date must be RFC 3339. The signature is the content of the .sig next to each file
// (written by qust-sign), and must name this version (`version:<x.y.z>` in its trusted comment).
//
//   node packaging/latest-json.mjs --version 1.2.0 \
//     --windows dist/Qust_1.2.0_x64-setup.exe --macos dist/Qust_1.2.0_universal.app.tar.gz \
//     [--base-url https://github.com/mkpascal/qust/releases/download/v1.2.0] [--notes "…"] \
//     [--out latest.json] [--allow-missing]
//
// The URLs name the version's own release, not releases/latest: a release published between a
// check and its download must not swap the file under a signature made for another one.

import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { basename } from 'node:path';

const args = process.argv.slice(2);
const options = {};
for (let i = 0; i < args.length; i++) {
  const name = args[i];
  if (!name.startsWith('--')) fail(`unexpected argument ${name}`);
  if (name === '--allow-missing') {
    options.allowMissing = true;
  } else {
    if (i + 1 >= args.length) fail(`${name} needs a value`);
    options[name.slice(2)] = args[++i];
  }
}

function fail(message) {
  console.error(`latest-json: ${message}`);
  process.exit(1);
}

const version = options.version?.replace(/^v+/, '');
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/.test(version)) {
  fail('--version x.y.z is required (semver)');
}
const baseUrl = (options['base-url'] ?? `https://github.com/mkpascal/qust/releases/download/v${version}`).replace(/\/+$/, '');
if (!baseUrl.startsWith('https://')) fail('--base-url must be https');

function entry(file) {
  if (!existsSync(file)) fail(`${file} not found`);
  const sigFile = `${file}.sig`;
  if (!existsSync(sigFile)) fail(`${sigFile} not found: sign the file first`);
  const signature = readFileSync(sigFile, 'utf8').trim();
  // The Tauri updater decodes it with a strict base64 decoder: no line breaks or spaces.
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(signature)) fail(`${sigFile} is not one line of base64`);
  const text = Buffer.from(signature, 'base64').toString('utf8');
  const lines = text.split('\n');
  if (!lines[0].startsWith('untrusted comment: ') || !lines[2]?.startsWith('trusted comment: ')) {
    fail(`${sigFile} is not a minisign signature`);
  }
  const signed = lines[2].slice('trusted comment: '.length).split('\t').find((field) => field.startsWith('version:'));
  if (signed !== `version:${version}`) fail(`${sigFile} is signed for ${signed ?? 'no version'}, not ${version}`);
  return { signature, url: `${baseUrl}/${encodeURIComponent(basename(file))}` };
}

const platforms = {};
if (options.windows) {
  const windows = entry(options.windows);
  platforms['windows-x86_64-nsis'] = windows;
  platforms['windows-x86_64'] = windows;
}
if (options.macos) {
  const macos = entry(options.macos);
  for (const arch of ['aarch64', 'x86_64']) {
    platforms[`darwin-${arch}-app`] = macos;
    platforms[`darwin-${arch}`] = macos;
  }
}
if (!options.allowMissing && (!options.windows || !options.macos)) {
  // A newer release with no entry for a platform is an error for that platform's updater.
  fail('both --windows and --macos are needed (or --allow-missing)');
}

const manifest = {
  version,
  notes: options.notes ?? '',
  pub_date: new Date().toISOString(),
  platforms,
};
const json = `${JSON.stringify(manifest, null, 2)}\n`;
if (options.out) {
  writeFileSync(options.out, json);
  console.log(`latest-json: wrote ${options.out}`);
} else {
  process.stdout.write(json);
}
