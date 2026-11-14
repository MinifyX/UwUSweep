/**
 * The window: title bar, the sidebar with the four views, and what floats
 * above them (the uninstall wizard, settings, the command palette, toasts).
 *
 * The app list lives here because three places need it: the Programme view,
 * the command palette's search and the wizard.
 */

import {
  CommandPalette,
  detectPlatform,
  Icon,
  ICONS,
  matchesAccelerator,
  NavIndicator,
  revealFrom,
  TitleBar,
  TitleBarAction,
  Toaster,
  UwuLabels,
  useTypeScale,
  withShortcut,
  Wordmark,
  cx,
  type CommandGroup,
} from '@uwusuite/design';
import { setMacMenu, useTauriWindow } from '@uwusuite/design/tauri';
import type { LucideIcon } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api, errorText, type App as AppInfo, type PlatformInfo } from './lib/api';
import { N_, t, useLanguage } from './lib/i18n';
import { getSettings, useSettings } from './lib/settings';
import { toasts } from './lib/toasts';
import { Autostart } from './views/Autostart';
import { AppDetail } from './views/AppDetail';
import { Programs } from './views/Programs';
import { Quarantine } from './views/Quarantine';
import { SettingsDialog } from './views/SettingsDialog';
import { Traces } from './views/Traces';
import { UninstallWizard, type WizardMode } from './views/UninstallWizard';

type View = 'programs' | 'autostart' | 'trace' | 'quarantine';

const VIEWS: { id: View; label: string; icon: LucideIcon }[] = [
  { id: 'programs', label: N_('Programme'), icon: ICONS.files },
  { id: 'autostart', label: N_('Autostart'), icon: ICONS.start },
  { id: 'trace', label: N_('Mitschreiben'), icon: ICONS.history },
  { id: 'quarantine', label: N_('Quarantäne'), icon: ICONS.archive },
];

const SHORTCUTS = { palette: 'CmdOrCtrl+K', search: 'CmdOrCtrl+F', settings: 'CmdOrCtrl+,' };

export function App() {
  const lang = useLanguage();
  const settings = useSettings();
  useTypeScale({ font: 'uwu', textSize: settings.textSize });
  const controls = useTauriWindow();
  const os = useMemo(() => detectPlatform(), []);

  const [info, setInfo] = useState<PlatformInfo | null>(null);
  const [view, setView] = useState<View>('programs');
  const [apps, setApps] = useState<AppInfo[] | null>(null);
  const [appsError, setAppsError] = useState<string | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [wizard, setWizard] = useState<{ app: AppInfo; mode: WizardMode } | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteQuery, setPaletteQuery] = useState('');
  const [search, setSearch] = useState('');
  const searchRef = useRef<HTMLInputElement>(null);
  const frameRef = useRef<HTMLDivElement>(null);

  // Once per start: the window is uncovered from the wordmark.
  useEffect(() => {
    if (frameRef.current) revealFrom(frameRef.current);
  }, []);

  const loadApps = useCallback(() => {
    setAppsError(null);
    api
      .appsList()
      .then(setApps)
      .catch((error) => {
        setApps((old) => old ?? []);
        setAppsError(errorText(error));
      });
  }, []);

  useEffect(() => {
    api.platformInfo().then(setInfo, () => undefined);
    loadApps();
    // Old sweeps go once per start, with the kept days from the settings.
    api
      .quarantineCleanup(Number(getSettings().quarantineDays))
      .catch((error) => console.warn('quarantine cleanup', error));
  }, [loadApps]);

  useEffect(() => {
    api.logDetail(settings.detailedLog).catch(() => undefined);
  }, [settings.detailedLog]);

  useEffect(() => {
    setMacMenu({ appName: 'UwUSweep', lang, onSettings: () => setSettingsOpen(true) }, os).catch(
      () => undefined,
    );
  }, [lang, os]);

  const show = useCallback((next: View) => {
    setView(next);
    setOpenId(null);
  }, []);

  const focusSearch = useCallback(() => {
    show('programs');
    // The list mounts first when another view was open.
    requestAnimationFrame(() => searchRef.current?.focus());
  }, [show]);

  useEffect(() => {
    const apple = os === 'mac';
    const onKey = (event: KeyboardEvent) => {
      if (matchesAccelerator(event, SHORTCUTS.palette, apple)) {
        event.preventDefault();
        setPaletteQuery('');
        setPaletteOpen(true);
      } else if (matchesAccelerator(event, SHORTCUTS.search, apple)) {
        event.preventDefault();
        focusSearch();
      } else if (matchesAccelerator(event, SHORTCUTS.settings, apple)) {
        event.preventDefault();
        setSettingsOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [os, focusSearch]);

  const opened = openId ? apps?.find((app) => app.id === openId) : undefined;

  const groups: CommandGroup[] = [
    {
      id: 'views',
      title: t('Ansichten'),
      items: [
        ...VIEWS.map((item) => ({
          id: item.id,
          label: t(item.label),
          text: t(item.label),
          icon: <Icon icon={item.icon} size="sm" />,
          onSelect: () => show(item.id),
        })),
        {
          id: 'settings',
          label: t('Einstellungen'),
          text: t('Einstellungen'),
          icon: <Icon icon={ICONS.settings} size="sm" />,
          onSelect: () => setSettingsOpen(true),
        },
      ],
    },
  ];
  if (paletteQuery.trim() && apps?.length) {
    groups.push({
      id: 'apps',
      title: t('Programme'),
      items: apps.map((app) => ({
        id: `app:${app.id}`,
        label: app.name,
        text: app.name,
        detail: app.publisher ?? undefined,
        keywords: app.publisher ? [app.publisher] : [],
        onSelect: () => {
          setView('programs');
          setOpenId(app.id);
        },
      })),
    });
  }

  return (
    <UwuLabels labels={lang}>
      <div ref={frameRef} className="sweep-frame">
        <TitleBar
          platform={os}
          controls={controls}
          brand={
            <span data-uwu-reveal-origin>
              <Wordmark product="UwUSweep" shell="box" />
            </span>
          }
          actions={
            <>
              <TitleBarAction
                label={withShortcut(t('Suchen'), SHORTCUTS.palette, os, lang)}
                onClick={() => setPaletteOpen(true)}
              >
                <Icon icon={ICONS.search} size="sm" />
              </TitleBarAction>
              <TitleBarAction
                label={withShortcut(t('Einstellungen'), SHORTCUTS.settings, os, lang)}
                onClick={() => setSettingsOpen(true)}
              >
                <Icon icon={ICONS.settings} size="sm" />
              </TitleBarAction>
            </>
          }
        />
        <div className="sweep-body">
          <nav aria-label={t('Ansichten')} className="border-r border-hairline bg-surface p-2">
            <ul className="flex flex-col gap-1">
              {VIEWS.map((item) => (
                <li key={item.id}>
                  <button
                    type="button"
                    aria-current={view === item.id ? 'page' : undefined}
                    onClick={() => show(item.id)}
                    className={cx(
                      'flex h-10 w-full items-center gap-3 rounded-control px-3 text-left text-body',
                      'focus-visible:shadow-focus focus-visible:outline-none',
                      view === item.id
                        ? 'bg-pink-tint font-semibold text-pink-ink'
                        : 'text-ink hover:bg-elevated',
                    )}
                  >
                    <Icon icon={item.icon} size="md" />
                    <span className="sweep-nav-label">{t(item.label)}</span>
                  </button>
                </li>
              ))}
            </ul>
            <NavIndicator />
          </nav>
          <main key={opened ? 'detail' : view} className="sweep-main uwu-page">
            {view === 'programs' &&
              (opened ? (
                <AppDetail
                  app={opened}
                  onBack={() => setOpenId(null)}
                  onUninstall={() => setWizard({ app: opened, mode: 'uninstall' })}
                  onLeftovers={() => setWizard({ app: opened, mode: 'leftovers' })}
                />
              ) : (
                <Programs
                  apps={apps}
                  error={appsError}
                  os={info?.os}
                  search={search}
                  onSearch={setSearch}
                  searchRef={searchRef}
                  onReload={loadApps}
                  onOpen={(app) => setOpenId(app.id)}
                />
              ))}
            {view === 'autostart' && <Autostart />}
            {view === 'trace' && <Traces />}
            {view === 'quarantine' && <Quarantine />}
          </main>
        </div>
      </div>
      {wizard && (
        <UninstallWizard
          app={wizard.app}
          mode={wizard.mode}
          info={info}
          onClose={(changed) => {
            setWizard(null);
            if (changed) {
              setOpenId(null);
              loadApps();
            }
          }}
        />
      )}
      <SettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} info={info} />
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        groups={groups}
        query={paletteQuery}
        onQueryChange={setPaletteQuery}
        placeholder={t('Ansicht oder Programm suchen …')}
      />
      <Toaster store={toasts} />
    </UwuLabels>
  );
}
