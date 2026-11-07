/** Every installed program: search, sort, filter by where it came from. */

import {
  Avatar,
  avatarColor,
  Button,
  EmptyState,
  Hint,
  Icon,
  ICONS,
  Segmented,
  Spinner,
  TextInput,
  cx,
} from '@uwusuite/design';
import type { RefObject } from 'react';
import { useMemo, useState } from 'react';
import { formatBytes, formatDate, type App, type PlatformInfo, type SourceKind } from '../lib/api';
import { N_, t, useLanguage } from '../lib/i18n';
import { AppTags, Art, ViewHeader } from './common';

type Sort = 'name' | 'size' | 'date';
type Filter = 'all' | 'desktop' | 'store' | 'package' | 'flatpak' | 'snap';

const FILTER_SOURCES: Record<Exclude<Filter, 'all'>, SourceKind[]> = {
  desktop: ['registry', 'macApp'],
  store: ['store'],
  package: ['package'],
  flatpak: ['flatpak'],
  snap: ['snap'],
};

const FILTER_LABELS: Record<Filter, string> = {
  all: N_('Alle'),
  desktop: N_('Desktop-Apps'),
  store: N_('Store'),
  package: N_('Pakete'),
  flatpak: 'Flatpak',
  snap: 'Snap',
};

/** Which filters make sense where: macOS has only app bundles. */
function filtersFor(os: PlatformInfo['os'] | undefined): Filter[] {
  if (os === 'windows') return ['all', 'desktop', 'store'];
  if (os === 'linux') return ['all', 'package', 'flatpak', 'snap'];
  return [];
}

export function Programs({
  apps,
  error,
  os,
  search,
  onSearch,
  searchRef,
  onReload,
  onOpen,
}: {
  apps: App[] | null;
  error: string | null;
  os: PlatformInfo['os'] | undefined;
  search: string;
  onSearch: (value: string) => void;
  searchRef: RefObject<HTMLInputElement>;
  onReload: () => void;
  onOpen: (app: App) => void;
}) {
  const lang = useLanguage();
  const [sort, setSort] = useState<Sort>('name');
  const [filter, setFilter] = useState<Filter>('all');
  const filters = filtersFor(os);

  const shown = useMemo(() => {
    const words = search.toLowerCase().split(/\s+/).filter(Boolean);
    const list = (apps ?? []).filter((app) => {
      if (filter !== 'all' && !FILTER_SOURCES[filter].includes(app.source.kind)) return false;
      const hay = `${app.name} ${app.publisher ?? ''}`.toLowerCase();
      return words.every((word) => hay.includes(word));
    });
    const byName = (a: App, b: App) => a.name.localeCompare(b.name, lang);
    list.sort((a, b) => {
      if (sort === 'size') return (b.size ?? 0) - (a.size ?? 0) || byName(a, b);
      if (sort === 'date') return (b.installedOn ?? '').localeCompare(a.installedOn ?? '');
      return byName(a, b);
    });
    return list;
  }, [apps, search, filter, sort, lang]);

  return (
    <section aria-labelledby="programs-title">
      <ViewHeader
        title={<span id="programs-title">{t('Programme')}</span>}
        subtitle={
          apps
            ? t('{count} Programme installiert', { count: apps.length })
            : t('Ich schaue nach, was installiert ist …')
        }
        actions={
          <Button variant="secondary" size="sm" icon={ICONS.refresh} onClick={onReload}>
            {t('Neu laden')}
          </Button>
        }
      />
      <div className="flex flex-wrap items-center gap-3 px-6 pb-4">
        <label className="relative min-w-[14rem] flex-1">
          <span className="sr-only">{t('Programme durchsuchen')}</span>
          <Icon
            icon={ICONS.search}
            size="sm"
            className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted"
          />
          <TextInput
            ref={searchRef}
            type="search"
            value={search}
            onChange={(event) => onSearch(event.target.value)}
            placeholder={t('Suchen …')}
            className="pl-9"
          />
        </label>
        <Segmented
          label={t('Sortieren nach')}
          value={sort}
          onChange={setSort}
          options={[
            { value: 'name', label: t('Name') },
            { value: 'size', label: t('Größe') },
            { value: 'date', label: t('Datum') },
          ]}
        />
        {filters.length > 1 && (
          <Segmented
            label={t('Herkunft')}
            value={filters.includes(filter) ? filter : 'all'}
            onChange={setFilter}
            options={filters.map((value) => ({ value, label: t(FILTER_LABELS[value]) }))}
          />
        )}
      </div>
      {error && (
        <Hint tone="danger" className="mx-6 mb-4">
          {t('Die Programmliste ging nicht: {error}', { error })}
        </Hint>
      )}
      {apps === null ? (
        <div role="status" className="flex items-center gap-3 px-6 py-8 text-muted">
          <Spinner /> {t('Ich schaue nach, was installiert ist …')}
        </div>
      ) : shown.length === 0 ? (
        <EmptyState
          art={<Art mood="puzzled" />}
          title={search ? t('Nichts gefunden') : t('Keine Programme')}
          body={
            search
              ? t('Zu „{query}“ passt kein Programm.', { query: search })
              : t('Hier ist es gerade ganz leer.')
          }
        />
      ) : (
        <ul className="flex flex-col gap-1 px-4 pb-6" aria-label={t('Programme')}>
          {shown.map((app) => (
            <li key={app.id}>
              <button
                type="button"
                onClick={() => onOpen(app)}
                className={cx(
                  'grid w-full grid-cols-[auto_1fr_auto] items-center gap-3 rounded-card px-3 py-2.5 text-left',
                  'hover:bg-elevated focus-visible:shadow-focus focus-visible:outline-none',
                )}
              >
                <Avatar name={app.name} color={avatarColor(app.name)} />
                <span className="min-w-0">
                  <span className="block truncate font-semibold">{app.name}</span>
                  <span className="block truncate text-meta text-muted">
                    {[app.publisher, app.version].filter(Boolean).join(' · ')}
                  </span>
                </span>
                <span className="flex flex-col items-end gap-1 text-meta text-muted">
                  <span>
                    {[formatBytes(app.size), formatDate(app.installedOn)]
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                  <AppTags app={app} />
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
