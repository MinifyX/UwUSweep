// Builds UwUSweep's downloads for the system it runs on: on Windows and
// macOS the app packed into the setup with Nyu in it, on Linux the app as
// .deb, .rpm and a portable folder.
//
//   pnpm build:setup                       for this machine
//   pnpm build:setup --target <triple>     another architecture (macOS:
//                                          universal-apple-darwin for both
//                                          Intel and Apple silicon, as the
//                                          release has it)
//
// Everything lands in target/installers/, under the names the release
// publishes — no version in them, so a link to the newest release's file
// stays the same forever:
//
//   Windows  UwUSweep-windows-x64-setup.exe           what people run
//            UwUSweep-windows-arm64-setup.exe         the same for ARM
//   macOS    UwUSweep-macos-universal.dmg             what people open
//   Linux    UwUSweep-linux-<arch>.deb                the app for apt/dpkg
//            UwUSweep-linux-<arch>.rpm                the app for dnf/zypper/rpm
//            UwUSweep-linux-<arch>-portable.tar.gz    unpack and run
//
// Nothing here signs anything, and nothing needs to: UwUSweep 0.1 has no
// updater, so there is no update signature, and there is no code-signing
// certificate either (macOS gets Apple's ad-hoc seal, see below). CI builds
// every one of these (.github/workflows/installers.yml) and a tag publishes
// them as a draft release.

import { execFileSync, execSync } from 'node:child_process';
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const run = (command, env = {}) =>
  execSync(command, { cwd: root, stdio: 'inherit', env: { ...process.env, ...env } });

const fail = (message) => {
  console.error(`\n✗ ${message}`);
  process.exit(1);
};

const targetIndex = process.argv.indexOf('--target');
const target = targetIndex > 0 ? process.argv[targetIndex + 1] : undefined;
const targetArg = target ? ` --target ${target}` : '';

const { version } = JSON.parse(
  readFileSync(join(root, 'apps/desktop/src-tauri/tauri.conf.json'), 'utf8'),
);
{
  const setup = JSON.parse(
    readFileSync(join(root, 'apps/setup/src-tauri/tauri.conf.json'), 'utf8'),
  );
  if (setup.version !== version) fail(`apps/setup says ${setup.version}, the app says ${version}.`);
}

const release = join(root, 'target', ...(target ? [target] : []), 'release');
const bundles = join(release, 'bundle');
const out = join(root, 'target', 'installers');
mkdirSync(out, { recursive: true });

/** The architecture as release file names say it. */
function arch() {
  if (target === 'universal-apple-darwin') return 'universal';
  const name = target ?? `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-host`;
  return name.startsWith('aarch64') ? 'arm64' : 'x64';
}

/** A Tauri config override, as a file: quoting JSON on a command line is a trap on every shell. */
function configFile(name, config) {
  const path = join(mkdtempSync(join(tmpdir(), 'uwusweep-build-')), `${name}.json`);
  writeFileSync(path, JSON.stringify(config));
  return path;
}

function only(dir, test, what) {
  const found = existsSync(dir) ? readdirSync(dir).filter(test) : [];
  if (found.length !== 1) fail(`Expected one ${what} in ${dir}, found ${found.length}.`);
  return join(dir, found[0]);
}

const produced = [];

if (process.platform === 'win32') {
  console.log(`\n▸ Building UwUSweep ${version}`);
  run(`pnpm --filter @uwusweep/desktop tauri build --no-bundle${targetArg}`);
  const app = join(release, 'uwusweep-desktop.exe');
  if (!existsSync(app)) fail(`Missing ${app}`);

  console.log('\n▸ Packing it into the setup');
  run(`pnpm --filter @uwusweep/setup tauri build --no-bundle${targetArg}`, {
    UWUSWEEP_SETUP_PAYLOAD: app,
  });
  const setup = join(out, `UwUSweep-windows-${arch()}-setup.exe`);
  copyFileSync(join(release, 'uwusweep-setup.exe'), setup);
  produced.push(setup);
} else if (process.platform === 'darwin') {
  // Ad-hoc signed: no Apple developer ID, but a sealed bundle, which Apple
  // silicon insists on and which keeps the app intact through the setup.
  const macOS = { signingIdentity: '-', minimumSystemVersion: '11.0' };
  const apps = mkdtempSync(join(tmpdir(), 'uwusweep-apps-'));

  console.log(`\n▸ Building UwUSweep ${version}`);
  run(
    `pnpm --filter @uwusweep/desktop tauri build --bundles app${targetArg} --config "${configFile('app', { bundle: { macOS } })}"`,
  );
  execFileSync('ditto', [join(bundles, 'macos', 'UwUSweep.app'), join(apps, 'UwUSweep.app')]);

  console.log('\n▸ Packing it into the setup');
  const setupConfig = configFile('setup', {
    bundle: { active: true, targets: ['dmg'], macOS },
  });
  run(
    `pnpm --filter @uwusweep/setup tauri build --bundles dmg${targetArg} --config "${setupConfig}"`,
    {
      UWUSWEEP_SETUP_PAYLOAD: apps,
    },
  );
  const dmg = join(out, `UwUSweep-macos-${arch()}.dmg`);
  copyFileSync(
    only(join(bundles, 'dmg'), (name) => name.endsWith('.dmg'), 'disk image'),
    dmg,
  );
  produced.push(dmg);
  rmSync(apps, { recursive: true, force: true });
} else {
  // The AppImage, unpacked, is the portable folder: it runs from its AppDir,
  // brings its own WebKit and needs no FUSE.
  const apps = mkdtempSync(join(tmpdir(), 'uwusweep-apps-'));
  const unpackAppImage = (image, name) => {
    const work = mkdtempSync(join(tmpdir(), 'uwusweep-appimage-'));
    execFileSync('chmod', ['+x', image]);
    execFileSync(image, ['--appimage-extract'], { cwd: work, stdio: 'ignore' });
    renameSync(join(work, 'squashfs-root'), join(apps, name));
    rmSync(work, { recursive: true, force: true });
  };

  console.log(`\n▸ Building UwUSweep ${version}`);
  run(`pnpm --filter @uwusweep/desktop tauri build --bundles appimage${targetArg}`);
  unpackAppImage(
    only(join(bundles, 'appimage'), (name) => name.endsWith('.AppImage'), 'AppImage of UwUSweep'),
    'UwUSweep',
  );
  // The next build bundles into the same folders.
  rmSync(join(bundles, 'appimage'), { recursive: true, force: true });

  // The packages install system-wide, to /usr, as package `uwusweep`. Tauri
  // names the package after productName in kebab case, which would make
  // "UwUSweep" `uw-u-sweep`; the menu entry keeps saying UwUSweep through
  // the template in tauri.conf.json, and the AUR package repacks this .deb
  // under the same name.
  console.log(`\n▸ Packaging UwUSweep ${version} as .deb and .rpm`);
  const packages = (bundles, config = {}) =>
    run(
      `pnpm --filter @uwusweep/desktop tauri build --bundles ${bundles}${targetArg} --config "${configFile('packages', { productName: 'uwusweep', ...config })}"`,
    );
  // rpm allows no '-' in a version: a prerelease becomes release 0.<pre>,
  // which sorts before the final release 1.
  const [core, pre] = version.split(/-(.*)/);
  if (pre) {
    packages('deb');
    packages('rpm', { version: core, bundle: { linux: { rpm: { release: `0.${pre}` } } } });
  } else {
    packages('deb,rpm');
  }
  const deb = join(out, `UwUSweep-linux-${arch()}.deb`);
  copyFileSync(
    only(join(bundles, 'deb'), (name) => name.endsWith('.deb'), 'deb'),
    deb,
  );
  const rpm = join(out, `UwUSweep-linux-${arch()}.rpm`);
  copyFileSync(
    only(join(bundles, 'rpm'), (name) => name.endsWith('.rpm'), 'rpm'),
    rpm,
  );
  produced.push(deb, rpm);

  // One folder, UwUSweep/, that runs where it lands. tar, not zip: the
  // AppDir needs its modes and symlinks.
  console.log('\n▸ Packing the portable folder');
  const staging = mkdtempSync(join(tmpdir(), 'uwusweep-portable-'));
  const folder = join(staging, 'UwUSweep');
  execFileSync('cp', ['-a', join(apps, 'UwUSweep'), folder]);
  const launcher = join(folder, 'uwusweep');
  writeFileSync(
    launcher,
    '#!/bin/sh\n# Starts UwUSweep from this folder.\nhere=$(dirname "$(readlink -f "$0")")\nexec "$here/AppRun" "$@"\n',
  );
  chmodSync(launcher, 0o755);
  writeFileSync(
    join(folder, 'README.txt'),
    [
      `UwUSweep ${version}, portable`,
      '',
      'Start it with ./uwusweep (or ./AppRun) in this folder. Nothing is installed:',
      'the folder can live anywhere and be deleted when you are done.',
      '',
      'It brings its own WebKit. To uninstall packages that need root it asks',
      'through pkexec (package polkit or policykit-1).',
      '',
      'To update, fetch the newest UwUSweep-linux-<arch>-portable.tar.gz from',
      'https://github.com/MinifyX/UwUSweep/releases',
      '',
    ].join('\n'),
  );
  const portable = join(out, `UwUSweep-linux-${arch()}-portable.tar.gz`);
  execFileSync('tar', [
    '--owner=0',
    '--group=0',
    '--numeric-owner',
    '-czf',
    portable,
    '-C',
    staging,
    'UwUSweep',
  ]);
  rmSync(staging, { recursive: true, force: true });
  produced.push(portable);
  rmSync(apps, { recursive: true, force: true });
}

for (const file of produced) console.log(`✧ ${file}`);
