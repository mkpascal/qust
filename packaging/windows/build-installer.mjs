// Builds Qust's Windows installer from a built qust.exe, with NSIS:
//
//   node packaging/windows/build-installer.mjs --exe src-tauri/target/release/qust.exe --version 1.2.0
//     [--out build/bin] [--icon src-tauri/icons/icon.ico] [--makensis <path>] [--define NAME=VALUE]…
//
// writes <out>/Qust_<version>_x64-setup.exe, the name the releases have always used. Sign it
// for the updaters afterwards (qust-sign). makensis is looked for in --makensis, $MAKENSIS, the
// PATH, Program Files and the copy Tauri downloads (%LOCALAPPDATA%\tauri\NSIS). --define passes
// the script's overridable names, only to build an isolated test installer.

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, statSync } from 'node:fs';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..', '..');

const args = process.argv.slice(2);
const options = { define: [] };
for (let i = 0; i < args.length; i++) {
  const name = args[i];
  if (!name.startsWith('--') || i + 1 >= args.length) fail(`unexpected argument ${name}`);
  const value = args[++i];
  if (name === '--define') options.define.push(value);
  else options[name.slice(2)] = value;
}

function fail(message) {
  console.error(`build-installer: ${message}`);
  process.exit(1);
}

const version = options.version?.replace(/^v+/, '');
const match = version?.match(/^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/);
if (!match) fail('--version x.y.z is required (semver)');
// The file version resource takes four numbers only.
const versionWithBuild = `${match[1]}.${match[2]}.${match[3]}.0`;

if (!options.exe) fail('--exe <path to qust.exe> is required');
const exe = resolve(options.exe);
if (!existsSync(exe) || !statSync(exe).isFile()) fail(`${exe} not found`);

const out = resolve(options.out ?? join(root, 'build', 'bin'));
mkdirSync(out, { recursive: true });
const productName = options.define.find((define) => define.startsWith('PRODUCTNAME='))?.slice('PRODUCTNAME='.length) ?? 'Qust';
const outFile = join(out, `${productName}_${version}_x64-setup.exe`);

const icon = options.icon ? resolve(options.icon) : join(root, 'src-tauri', 'icons', 'icon.ico');
if (options.icon && !existsSync(icon)) fail(`${icon} not found`);

function findMakensis() {
  const candidates = [
    options.makensis,
    process.env.MAKENSIS,
    ...(process.env.PATH ?? '').split(delimiter).map((dir) => dir && join(dir, 'makensis.exe')),
    process.env['ProgramFiles(x86)'] && join(process.env['ProgramFiles(x86)'], 'NSIS', 'makensis.exe'),
    process.env.ProgramFiles && join(process.env.ProgramFiles, 'NSIS', 'makensis.exe'),
    process.env.LOCALAPPDATA && join(process.env.LOCALAPPDATA, 'tauri', 'NSIS', 'makensis.exe'),
  ];
  return candidates.find((candidate) => candidate && existsSync(candidate));
}
const makensis = findMakensis();
if (!makensis) fail('makensis not found: install NSIS 3 (e.g. `choco install nsis`) or pass --makensis');

const defines = [
  `VERSION=${version}`,
  `VERSIONWITHBUILD=${versionWithBuild}`,
  `MAINBINARYSRCPATH=${exe}`,
  `OUTFILE=${outFile}`,
  ...(existsSync(icon) ? [`INSTALLERICON=${icon}`] : []),
  ...options.define,
];
for (const define of defines) {
  if (!/^[A-Z_]+=[^"\r\n]*$/.test(define)) fail(`bad define ${define}`);
}

// /INPUTCHARSET UTF8 so the script's text reads the same on any code page; /WX turns warnings
// into errors, so a broken script doesn't quietly make a broken installer.
execFileSync(makensis, ['/V3', '/WX', '/INPUTCHARSET', 'UTF8', ...defines.map((define) => `/D${define}`), join(here, 'installer.nsi')], {
  stdio: 'inherit',
});
console.log(`build-installer: wrote ${outFile}`);
