/**
 * The English catalogue: German string → English string, one file per area of
 * the app. See `lib/i18n.ts`.
 */

import app from './app.json';
import autostart from './autostart.json';
import programs from './programs.json';
import quarantine from './quarantine.json';
import settings from './settings.json';
import trace from './trace.json';
import wizard from './wizard.json';

export const EN: Readonly<Record<string, string>> = {
  ...app,
  ...autostart,
  ...programs,
  ...quarantine,
  ...settings,
  ...trace,
  ...wizard,
};
