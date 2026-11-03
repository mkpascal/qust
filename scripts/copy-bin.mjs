// Copies the app and installers from the last `tauri build` into build/bin/ (git-ignored).
import fs from 'node:fs';
import path from 'node:path';

const release = 'src-tauri/target/release';
const out = 'build/bin';
const sources = [
  path.join(release, 'qust.exe'),
  ...['nsis', 'dmg'].flatMap((dir) => {
    const bundle = path.join(release, 'bundle', dir);
    return fs.existsSync(bundle) ? fs.readdirSync(bundle).filter((f) => /\.(exe|dmg)$/.test(f)).map((f) => path.join(bundle, f)) : [];
  }),
];

fs.mkdirSync(out, { recursive: true });
for (const source of sources.filter((f) => fs.existsSync(f))) {
  const name = path.basename(source) === 'qust.exe' ? 'Qust.exe' : path.basename(source);
  fs.copyFileSync(source, path.join(out, name));
  console.log(`copied ${path.join(out, name)}`);
}
