// The installer speaks like UwUSweep's default, playful tone, in German or English.

const de = {
  welcomeTitle: 'Hallo! Ich bin Nyu',
  welcomeBody:
    'Ich richte UwUSweep in ein paar Sekunden auf deinem {computer} ein. Dann räumen wir zusammen auf: Programme deinstallieren und die Reste gleich mit wegfegen.',
  againTitle: 'Schön, dich wiederzusehen!',
  againBody:
    'UwUSweep {installed} ist schon da. Ich bringe es auf {version}, deine Einstellungen bleiben, wo sie sind.',
  install: 'Installieren',
  update: 'Aktualisieren',
  reinstall: 'Neu installieren',
  options: 'Optionen',
  fewerOptions: 'Weniger Optionen',
  folder: 'Speicherort',
  change: 'Ändern …',
  desktopShortcut: 'Verknüpfung auf dem Desktop',
  runningTitle: 'UwUSweep ist gerade offen',
  runningBody:
    'Ich schließe es kurz, damit ich weitermachen kann. Eine laufende Deinstallation oder Reste-Suche wird dabei abgebrochen.',
  closeAndContinue: 'Schließen und weiter',
  back: 'Zurück',
  progressInstall: 'Nyu richtet alles ein …',
  progressUninstall: 'Nyu räumt auf …',
  quotes: [
    'Holt den Besen aus der Kammer …',
    'Fegt Staubflusen zusammen …',
    'Schüttelt die Kehrschaufel aus …',
    'Schaut unter den Teppich …',
    'Übt Ohrenwackeln …',
    'Stapelt Kisten in der Quarantäne …',
    'Sortiert Krümel nach Größe …',
  ],
  doneTitle: 'Fertig! ✧',
  doneBody: 'UwUSweep ist bereit. Zeit für ein bisschen Frühjahrsputz.',
  tipsTitle: 'Kleine Tipps',
  tips: [
    'Gefundene Reste landen erst in der Quarantäne. Mit einem Klick holst du sie zurück, nach 30 Tagen lösche ich sie endgültig.',
    'Installierst du etwas Neues, lass UwUSweep mitschreiben. Dann weiß ich später genau, was alles dazugehört.',
    'Fragt dein System nach einem Admin-Passwort, dann nur, weil eine Deinstallation oder ein Aufräumen es wirklich braucht.',
  ],
  start: 'UwUSweep starten',
  close: 'Schließen',
  errorTitle: 'Hoppla, das hat nicht geklappt',
  retry: 'Nochmal versuchen',
  uninstallTitle: 'Schade, dass du gehst …',
  uninstallBody: 'Ich entferne UwUSweep von diesem {computer}.',
  removeInstead: 'Deinstallieren …',
  computerWindows: 'PC',
  computerMac: 'Mac',
  computerLinux: 'Rechner',
  keepData: 'Einstellungen, Quarantäne und Aufzeichnungen behalten',
  keepDataHint:
    'Praktisch, falls du wiederkommst. Sonst lösche ich alles, auch die Reste, die noch in der Quarantäne liegen.',
  uninstall: 'Deinstallieren',
  keep: 'Doch behalten',
  goodbyeTitle: 'Tschüss! (｡•́︿•̀｡)',
  goodbyeBody: 'UwUSweep ist entfernt. Komm gern wieder vorbei.',
  soundOn: 'Ton an',
  soundOff: 'Ton aus',
  minimize: 'Minimieren',
  devBuild: 'Entwicklungs-Build: UwUSweep ist hier nicht mit drin.',
  sandbox: 'Testmodus: Es wird nur in einem Testordner installiert.',
  footer: 'Version {version} · Open Source',
};

const en: typeof de = {
  welcomeTitle: "Hi! I'm Nyu",
  welcomeBody:
    "I'll set up UwUSweep on your {computer} in a few seconds. Then we tidy up together: uninstall programs and sweep away what they leave behind.",
  againTitle: 'Nice to see you again!',
  againBody:
    "UwUSweep {installed} is already here. I'll bring it to {version}, and your settings stay where they are.",
  install: 'Install',
  update: 'Update',
  reinstall: 'Reinstall',
  options: 'Options',
  fewerOptions: 'Fewer options',
  folder: 'Location',
  change: 'Change …',
  desktopShortcut: 'Shortcut on the desktop',
  runningTitle: 'UwUSweep is open right now',
  runningBody:
    "I'll close it for a moment so I can carry on. A running uninstall or leftover search will be cut short.",
  closeAndContinue: 'Close and continue',
  back: 'Back',
  progressInstall: 'Nyu is setting everything up …',
  progressUninstall: 'Nyu is tidying up …',
  quotes: [
    'Fetching the broom …',
    'Sweeping up dust bunnies …',
    'Shaking out the dustpan …',
    'Peeking under the rug …',
    'Practicing ear wiggles …',
    'Stacking boxes in the quarantine …',
    'Sorting crumbs by size …',
  ],
  doneTitle: 'All done! ✧',
  doneBody: 'UwUSweep is ready. Time for a little spring cleaning.',
  tipsTitle: 'Little tips',
  tips: [
    'Leftovers go to the quarantine first. One click brings them back; after 30 days I delete them for good.',
    'Installing something new? Let UwUSweep take notes. Then I know later exactly what belongs to it.',
    'If your system asks for an admin password, it is only because an uninstall or a cleanup really needs it.',
  ],
  start: 'Start UwUSweep',
  close: 'Close',
  errorTitle: "Oops, that didn't work",
  retry: 'Try again',
  uninstallTitle: 'Sad to see you go …',
  uninstallBody: "I'll remove UwUSweep from this {computer}.",
  removeInstead: 'Uninstall …',
  computerWindows: 'PC',
  computerMac: 'Mac',
  computerLinux: 'computer',
  keepData: 'Keep settings, quarantine and traces',
  keepDataHint:
    'Handy if you come back. Otherwise I delete everything, including leftovers still in the quarantine.',
  uninstall: 'Uninstall',
  keep: 'Keep it',
  goodbyeTitle: 'Bye! (｡•́︿•̀｡)',
  goodbyeBody: 'UwUSweep is removed. Come back any time.',
  soundOn: 'Sound on',
  soundOff: 'Sound off',
  minimize: 'Minimize',
  devBuild: "Development build: UwUSweep isn't packed inside.",
  sandbox: 'Test mode: installs into a test folder only.',
  footer: 'Version {version} · Open source',
};

export type Texts = typeof de;

export const texts: Texts = navigator.language.toLowerCase().startsWith('de') ? de : en;

export function fill(template: string, values: Record<string, string>) {
  return template.replace(/\{(\w+)\}/g, (_, key: string) => values[key] ?? '');
}
