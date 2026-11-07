/** What starts with the computer, grouped by where it is set, each with a switch. */

import { Button, Card, EmptyState, Hint, ICONS, Spinner, Switch, Tag } from '@uwusuite/design';
import { useCallback, useEffect, useState } from 'react';
import { api, errorText, isCancelled, type AutostartEntry, type AutostartPlace } from '../lib/api';
import { N_, t } from '../lib/i18n';
import { toasts } from '../lib/toasts';
import { Art, ViewHeader } from './common';

const PLACES: Record<AutostartPlace, string> = {
  runUser: N_('Registry: nur für dich'),
  runMachine: N_('Registry: für alle'),
  startupUser: N_('Autostart-Ordner: nur für dich'),
  startupCommon: N_('Autostart-Ordner: für alle'),
  launchAgentUser: N_('Launch Agents: nur für dich'),
  launchAgentSystem: N_('Launch Agents: für alle'),
  launchDaemon: N_('Launch Daemons'),
  xdg: N_('Autostart der Arbeitsumgebung'),
  systemdUser: N_('systemd (Benutzer)'),
};

export function Autostart() {
  const [entries, setEntries] = useState<AutostartEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);

  const load = useCallback(() => {
    setError(null);
    api
      .autostartList()
      .then(setEntries)
      .catch((err) => {
        setEntries((old) => old ?? []);
        setError(errorText(err));
      });
  }, []);
  useEffect(load, [load]);

  const set = async (entry: AutostartEntry, enabled: boolean) => {
    setPending(entry.id);
    try {
      await api.autostartSet(entry, enabled);
      setEntries((old) => old?.map((e) => (e.id === entry.id ? { ...e, enabled } : e)) ?? old);
      toasts.show(
        enabled
          ? t('{name} startet wieder mit.', { name: entry.name })
          : t('{name} startet nicht mehr mit.', { name: entry.name }),
        { tone: 'success' },
      );
    } catch (err) {
      if (isCancelled(err)) toasts.show(t('Abgebrochen'));
      else toasts.show(t('Das ging nicht.'), { tone: 'error', detail: errorText(err) });
    } finally {
      setPending(null);
    }
  };

  const groups = new Map<AutostartPlace, AutostartEntry[]>();
  for (const entry of entries ?? []) {
    groups.set(entry.place, [...(groups.get(entry.place) ?? []), entry]);
  }

  return (
    <section aria-labelledby="autostart-title">
      <ViewHeader
        title={<span id="autostart-title">{t('Autostart')}</span>}
        subtitle={t('Was beim Anmelden von selbst startet. Aus heißt nur: startet nicht mehr mit.')}
        actions={
          <Button variant="secondary" size="sm" icon={ICONS.refresh} onClick={load}>
            {t('Neu laden')}
          </Button>
        }
      />
      {error && (
        <Hint tone="danger" className="mx-6 mb-4">
          {error}
        </Hint>
      )}
      {entries === null ? (
        <div role="status" className="flex items-center gap-3 px-6 py-8 text-muted">
          <Spinner /> {t('Einen Moment …')}
        </div>
      ) : entries.length === 0 ? (
        <EmptyState
          art={<Art mood="sleepy" />}
          title={t('Nichts im Autostart')}
          body={t('Dein Computer startet ganz ohne Mitläufer. Schön ruhig.')}
        />
      ) : (
        <div className="flex flex-col gap-4 px-6 pb-6">
          {[...groups].map(([place, list]) => (
            <Card key={place} title={t(PLACES[place])}>
              <ul className="flex flex-col divide-y divide-hairline">
                {list.map((entry) => (
                  <li key={entry.id} className="flex items-center gap-4 py-2.5">
                    <div className="min-w-0 flex-1">
                      <p className="flex flex-wrap items-center gap-2 font-semibold">
                        {entry.name}
                        {entry.needsAdmin && <Tag tone="warning">{t('Admin')}</Tag>}
                      </p>
                      <p
                        className="uwu-mono truncate text-caption text-muted"
                        title={entry.command}
                      >
                        {entry.command}
                      </p>
                    </div>
                    <Switch
                      checked={entry.enabled}
                      disabled={pending === entry.id}
                      onChange={(on) => set(entry, on)}
                      label={t('{name} beim Anmelden starten', { name: entry.name })}
                    />
                  </li>
                ))}
              </ul>
            </Card>
          ))}
        </div>
      )}
    </section>
  );
}
