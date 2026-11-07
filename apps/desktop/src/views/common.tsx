/** Small pieces several views share. */

import { Nyu, Tag, type NyuMood } from '@uwusuite/design';
import type { ReactNode } from 'react';
import type { App, Source } from '../lib/api';
import { t } from '../lib/i18n';

/** "Desktop-App", "Store", "Flatpak" … for a source pill. */
export function sourceLabel(source: Source): string {
  switch (source.kind) {
    case 'registry':
      return t('Desktop-App');
    case 'store':
      return t('Store');
    case 'macApp':
      return t('App');
    case 'package':
      return source.manager;
    case 'flatpak':
      return source.user ? t('Flatpak (du)') : 'Flatpak';
    case 'snap':
      return 'Snap';
  }
}

export function AppTags({ app }: { app: App }) {
  return (
    <span className="flex flex-wrap gap-1.5">
      <Tag>{sourceLabel(app.source)}</Tag>
      {app.needsAdmin && <Tag tone="warning">{t('Admin')}</Tag>}
    </span>
  );
}

/** Nyu for an empty state, always in UwUSweep's box. */
export function Art({ mood = 'uwu' }: { mood?: NyuMood }) {
  return <Nyu shell="box" mood={mood} title="" />;
}

/** A view's heading row: title, a line under it, actions on the right. */
export function ViewHeader({
  title,
  subtitle,
  actions,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-3 px-6 pt-6 pb-4">
      <div className="min-w-0">
        <h1 className="text-title font-bold">{title}</h1>
        {subtitle && <p className="mt-1 text-meta text-muted">{subtitle}</p>}
      </div>
      {actions && <div className="flex flex-wrap gap-2">{actions}</div>}
    </header>
  );
}
