/**
 * The Rust side, typed: one function per `#[tauri::command]` in
 * `src-tauri/src/lib.rs`, and the shapes of `uwusweep-core`'s model as they
 * arrive as JSON (camelCase). Errors arrive as plain strings; `"cancelled"`
 * means the user said no to the admin prompt or closed the uninstaller.
 */

import { invoke } from '@tauri-apps/api/core';
import { locale } from './i18n';

export type Command = { kind: 'line'; line: string } | { kind: 'argv'; argv: string[] };

export type Source =
  | { kind: 'registry'; hive: string; key: string; wow64: boolean; msiProductCode?: string | null }
  | { kind: 'store'; packageFullName: string; packageFamilyName: string }
  | { kind: 'macApp'; bundlePath: string; bundleId?: string | null }
  | { kind: 'package'; manager: 'dpkg' | 'pacman' | 'rpm'; name: string }
  | { kind: 'flatpak'; appId: string; user: boolean }
  | { kind: 'snap'; name: string };

export type SourceKind = Source['kind'];

export type App = {
  id: string;
  name: string;
  publisher?: string | null;
  version?: string | null;
  installDir?: string | null;
  size?: number | null;
  /** YYYY-MM-DD. */
  installedOn?: string | null;
  source: Source;
  uninstall?: Command | null;
  quietUninstall?: Command | null;
  wingetId?: string | null;
  needsAdmin: boolean;
  hints: string[];
};

export type ScanLevel = 'safe' | 'deep';
export type Confidence = 'high' | 'medium';
export type ItemKind = 'file' | 'dir' | 'regKey' | 'regValue';

export type Leftover = {
  kind: ItemKind;
  path: string;
  value?: string | null;
  size?: number | null;
  confidence: Confidence;
  reason: string;
  needsAdmin: boolean;
};

export type AutostartPlace =
  | 'runUser'
  | 'runMachine'
  | 'startupUser'
  | 'startupCommon'
  | 'launchAgentUser'
  | 'launchAgentSystem'
  | 'launchDaemon'
  | 'xdg'
  | 'systemdUser';

export type AutostartEntry = {
  id: string;
  name: string;
  command: string;
  place: AutostartPlace;
  location: string;
  enabled: boolean;
  needsAdmin: boolean;
};

export type QuarantineItem = {
  kind: ItemKind;
  original: string;
  value?: string | null;
  stored: string;
  size?: number | null;
};

export type QuarantineBatch = {
  id: string;
  appName: string;
  /** Seconds since the epoch. */
  created: number;
  items: QuarantineItem[];
  needsAdmin: boolean;
};

export type Trace = {
  id: string;
  name: string;
  created: number;
  appIds: string[];
  items: Leftover[];
};

export type PlatformInfo = {
  os: 'windows' | 'macos' | 'linux';
  elevated: boolean;
  restorePointSupported: boolean;
  version: string;
};

/** Path and why it failed. */
export type Failure = [string, string];

export type SweepReport = { batches: QuarantineBatch[]; failed: Failure[] };

export const api = {
  platformInfo: () => invoke<PlatformInfo>('platform_info'),
  appsList: () => invoke<App[]>('apps_list'),
  leftoversScan: (appId: string, level: ScanLevel) =>
    invoke<Leftover[]>('leftovers_scan', { appId, level }),
  uninstallStart: (appId: string, quiet: boolean, restorePoint: boolean) =>
    invoke<{ restorePointError?: string | null }>('uninstall_start', {
      appId,
      quiet,
      restorePoint,
    }),
  quarantinePut: (appName: string, items: Leftover[]) =>
    invoke<SweepReport>('quarantine_put', { appName, items }),
  quarantineList: () => invoke<QuarantineBatch[]>('quarantine_list'),
  quarantineRestore: (id: string) => invoke<Failure[]>('quarantine_restore', { id }),
  quarantinePurge: (id: string) => invoke<void>('quarantine_purge', { id }),
  quarantineCleanup: (days: number) => invoke<number>('quarantine_cleanup', { days }),
  quarantineReveal: (id: string) => invoke<void>('quarantine_reveal', { id }),
  autostartList: () => invoke<AutostartEntry[]>('autostart_list'),
  autostartSet: (entry: AutostartEntry, enabled: boolean) =>
    invoke<void>('autostart_set', { entry, enabled }),
  traceStart: () => invoke<void>('trace_start'),
  traceFinish: (name: string) => invoke<Trace>('trace_finish', { name }),
  traceCancel: () => invoke<void>('trace_cancel'),
  tracesList: () => invoke<Trace[]>('traces_list'),
  traceDelete: (id: string) => invoke<void>('trace_delete', { id }),
  installPick: () => invoke<string | null>('install_pick'),
  installerOpen: (path: string) => invoke<void>('installer_open', { path }),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  openLink: (url: string) => invoke<void>('open_link', { url }),
  logDetail: (on: boolean) => invoke<void>('log_detail', { on }),
  openLogFolder: () => invoke<void>('open_log_folder'),
};

export const isCancelled = (error: unknown) => String(error) === 'cancelled';

/** An error from Rust as one line. */
export const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

/** 1.4 GB, 320 KB … in the app's language. */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return '';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toLocaleString(locale(), { maximumFractionDigits: digits })} ${units[unit]}`;
}

/** A day as the app's language writes it; accepts YYYY-MM-DD or epoch seconds. */
export function formatDate(value: string | number | null | undefined): string {
  if (value == null || value === '') return '';
  const date =
    typeof value === 'number' ? new Date(value * 1000) : new Date(`${value.slice(0, 10)}T12:00`);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleDateString(locale(), { day: 'numeric', month: 'short', year: 'numeric' });
}

export const sum = (values: (number | null | undefined)[]) =>
  values.reduce<number>((total, value) => total + (value ?? 0), 0);

export const isRegistry = (kind: ItemKind) => kind === 'regKey' || kind === 'regValue';

/** Whether the unattended uninstall can be offered for this app. */
export const canQuiet = (app: App) => Boolean(app.quietUninstall || app.wingetId);
