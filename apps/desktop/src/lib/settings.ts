/**
 * The settings, kept in the page's own storage.
 *
 * Only preferences live here. The Rust side gets what concerns it (the log
 * level, the quarantine's kept days) from `App.tsx` on start and on change.
 */

import {
  applyAppearance as applyToRoot,
  QUERIES,
  resolveAppearance,
  TEXT_SIZE_CHOICES,
  type ContrastSetting,
  type MotionSetting,
  type TextSizeChoice,
  type ThemeSetting,
} from '@uwusuite/design';
import { useSyncExternalStore } from 'react';
import type { ScanLevel } from './api';
import { language } from './i18n';

export type { ContrastSetting, MotionSetting, ThemeSetting };
/** German or English; "system" follows the language the system prefers. */
export type LanguageSetting = 'system' | 'de' | 'en';
/** How many days a sweep stays in the quarantine before it is deleted for good. */
export type QuarantineDays = '7' | '14' | '30' | '90';
export const QUARANTINE_DAYS: readonly QuarantineDays[] = ['7', '14', '30', '90'];

export type Settings = {
  language: LanguageSetting;
  theme: ThemeSetting;
  contrast: ContrastSetting;
  motion: MotionSetting;
  textSize: TextSizeChoice;
  quarantineDays: QuarantineDays;
  scanLevel: ScanLevel;
  /** Windows: a restore point before every uninstall, preselected in the wizard. */
  restorePoint: boolean;
  /** Every step into the log file, for an issue. */
  detailedLog: boolean;
};

export const DEFAULT_SETTINGS: Settings = {
  language: 'system',
  theme: 'system',
  contrast: 'system',
  motion: 'system',
  textSize: 'system',
  quarantineDays: '30',
  scanLevel: 'safe',
  restorePoint: true,
  detailedLog: false,
};

const KEY = 'uwusweep.settings';

/** Stored values are checked one by one; anything unexpected falls back to its default. */
export function sanitize(raw: unknown): Settings {
  const input = typeof raw === 'object' && raw !== null ? (raw as Record<string, unknown>) : {};
  const oneOf = <T>(value: unknown, allowed: readonly T[], fallback: T): T =>
    allowed.includes(value as T) ? (value as T) : fallback;
  const bool = (value: unknown, fallback: boolean) =>
    typeof value === 'boolean' ? value : fallback;
  const d = DEFAULT_SETTINGS;
  return {
    language: oneOf(input.language, ['system', 'de', 'en'] as const, d.language),
    theme: oneOf(input.theme, ['system', 'light', 'dark'] as const, d.theme),
    contrast: oneOf(input.contrast, ['system', 'normal', 'high'] as const, d.contrast),
    motion: oneOf(input.motion, ['system', 'on', 'off'] as const, d.motion),
    textSize: oneOf(input.textSize, TEXT_SIZE_CHOICES, d.textSize),
    quarantineDays: oneOf(input.quarantineDays, QUARANTINE_DAYS, d.quarantineDays),
    scanLevel: oneOf(input.scanLevel, ['safe', 'deep'] as const, d.scanLevel),
    restorePoint: bool(input.restorePoint, d.restorePoint),
    detailedLog: bool(input.detailedLog, d.detailedLog),
  };
}

function load(): Settings {
  try {
    const raw = window.localStorage.getItem(KEY);
    return sanitize(raw ? JSON.parse(raw) : {});
  } catch {
    return DEFAULT_SETTINGS;
  }
}

let current = load();
const listeners = new Set<() => void>();

export function getSettings(): Settings {
  return current;
}

export function updateSettings(patch: Partial<Settings>) {
  current = sanitize({ ...current, ...patch });
  try {
    window.localStorage.setItem(KEY, JSON.stringify(current));
  } catch {
    // Private storage can be unavailable; the change still holds for this run.
  }
  for (const listener of listeners) listener();
}

export function subscribeSettings(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useSettings(): Settings {
  return useSyncExternalStore(subscribeSettings, getSettings);
}

/**
 * Theme, contrast, motion and language onto <html>, now and on every change.
 * Called from `main.tsx` before React mounts, so the first paint is right
 * (the CSP allows no inline boot script).
 */
export function applyAppearance() {
  const apply = () => {
    applyToRoot(resolveAppearance(current));
    document.documentElement.lang = language(current);
  };
  apply();
  subscribeSettings(apply);
  for (const query of Object.values(QUERIES)) {
    window.matchMedia(query).addEventListener('change', apply);
  }
}
