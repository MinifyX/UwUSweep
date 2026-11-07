// The text of a GitHub release: what changed, in German and English, from
// release-notes/<version>.json, then which file is for which system.
//
//   node scripts/release-body.mjs <version> [--out <file>]
//
// CI's release job (.github/workflows/installers.yml) writes it for the draft
// release a tag creates. Without --out it prints it, to read before tagging.

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const REPOSITORY = 'MinifyX/UwUSweep';

/** Every file a release carries, besides SHA256SUMS.txt. The release job checks for exactly these. */
export const RELEASE_FILES = [
  'UwUSweep-windows-x64-setup.exe',
  'UwUSweep-windows-arm64-setup.exe',
  'UwUSweep-macos-universal.dmg',
  'UwUSweep-linux-x64.deb',
  'UwUSweep-linux-x64.rpm',
  'UwUSweep-linux-x64-portable.tar.gz',
  'UwUSweep-linux-arm64.deb',
  'UwUSweep-linux-arm64.rpm',
  'UwUSweep-linux-arm64-portable.tar.gz',
];

/** The notes for a version, checked: both languages, neither empty. */
export function readNotes(version) {
  const file = join(root, 'release-notes', `${version}.json`);
  if (!existsSync(file)) throw new Error(`release-notes/${version}.json is missing.`);
  const notes = JSON.parse(readFileSync(file, 'utf8'));
  for (const language of ['de', 'en']) {
    if (typeof notes[language] !== 'string' || !notes[language].trim()) {
      throw new Error(`release-notes/${version}.json has no '${language}' text.`);
    }
  }
  return notes;
}

export function releaseBody(version, notes = readNotes(version)) {
  const guide = `https://github.com/${REPOSITORY}#installation`;
  const code = (name) => `\`${name}\``;
  // [system (de), system (en), files]
  const rows = [
    ['Windows (x64)', 'Windows (x64)', code('UwUSweep-windows-x64-setup.exe')],
    ['Windows auf ARM', 'Windows on ARM', code('UwUSweep-windows-arm64-setup.exe')],
    [
      'macOS (Intel & Apple-Chip)',
      'macOS (Intel & Apple chip)',
      code('UwUSweep-macos-universal.dmg'),
    ],
    [
      'Ubuntu / Debian',
      'Ubuntu / Debian',
      `${code('UwUSweep-linux-x64.deb')} · ARM: ${code('UwUSweep-linux-arm64.deb')}`,
    ],
    [
      'Fedora / openSUSE',
      'Fedora / openSUSE',
      `${code('UwUSweep-linux-x64.rpm')} · ARM: ${code('UwUSweep-linux-arm64.rpm')}`,
    ],
    // The release job attaches uwusweep-bin's PKGBUILD, until it is on the AUR.
    ['Arch Linux', 'Arch Linux', `${code('PKGBUILD')} → ${code('makepkg -si')}`],
    [
      'Linux portabel',
      'Linux portable',
      `${code('UwUSweep-linux-x64-portable.tar.gz')} · ARM: ${code('…-arm64-portable.tar.gz')}`,
    ],
  ];
  const table = (column) =>
    ['| | |', '|---|---|', ...rows.map((row) => `| ${row[column]} | ${row[2]} |`)].join('\n');
  const de = [
    table(0),
    'Windows: warnt es („Der Computer wurde durch Windows geschützt“), **Weitere Informationen → Trotzdem ausführen**.',
    'macOS: die `.dmg` öffnen und **UwUSweep Setup** starten. UwUSweep ist nicht bei Apple notarisiert (keine Developer ID): sagt macOS, es könne das Programm nicht prüfen, unter **Systemeinstellungen → Datenschutz & Sicherheit → Trotzdem öffnen** freigeben.',
    'Linux: `.deb` und `.rpm` installieren systemweit (für Deinstallationen mit Root-Rechten fragt UwUSweep über `pkexec`, Paket polkit); die portable Version einfach entpacken und `./UwUSweep/uwusweep` starten.',
    'UwUSweep aktualisiert sich noch nicht selbst: Für eine neue Version einfach das neue Setup oder Paket über das alte installieren.',
  ];
  const en = [
    table(1),
    'Windows: if it warns that it "protected your PC", **More info → Run anyway**.',
    "macOS: open the `.dmg` and start **UwUSweep Setup**. UwUSweep isn't notarized by Apple (no developer ID): if macOS says it can't check the app, allow it under **System Settings → Privacy & Security → Open Anyway**.",
    'Linux: the `.deb` and `.rpm` install system-wide (for uninstalls that need root UwUSweep asks through `pkexec`, package polkit); the portable one you just unpack and start with `./UwUSweep/uwusweep`.',
    "UwUSweep doesn't update itself yet: for a new version, install the new setup or package over the old one.",
  ];
  return [
    `## Deutsch\n\n${notes.de}\n`,
    `## English\n\n${notes.en}\n`,
    `## Herunterladen\n\n${de.join('\n\n')}\n\n[Anleitung](${guide})\n`,
    `## Downloads\n\n${en.join('\n\n')}\n\n[Install guide](${guide})\n`,
    'Prüfsummen · checksums: `SHA256SUMS.txt`\n',
  ].join('\n');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  const outAt = args.indexOf('--out');
  const out = outAt >= 0 ? args.splice(outAt, 2)[1] : undefined;
  const version = args[0]?.replace(/^v/, '');
  if (!version) {
    console.error('Usage: node scripts/release-body.mjs <version> [--out <file>]');
    process.exit(1);
  }
  let body;
  try {
    body = releaseBody(version);
  } catch (error) {
    console.error(`✗ ${error.message}`);
    process.exit(1);
  }
  if (out) writeFileSync(out, body);
  else process.stdout.write(body);
}
