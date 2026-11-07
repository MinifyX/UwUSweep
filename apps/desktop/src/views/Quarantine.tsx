/** Every sweep that can still be undone, until its days run out. */

import { Button, Card, Dialog, EmptyState, Hint, ICONS, Spinner } from '@uwusuite/design';
import { useCallback, useEffect, useState } from 'react';
import {
  api,
  errorText,
  formatBytes,
  formatDate,
  isCancelled,
  sum,
  type QuarantineBatch,
} from '../lib/api';
import { t } from '../lib/i18n';
import { useSettings } from '../lib/settings';
import { toasts } from '../lib/toasts';
import { Art, ViewHeader } from './common';

const DAY = 86_400;

export function Quarantine() {
  const settings = useSettings();
  const [batches, setBatches] = useState<QuarantineBatch[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [purging, setPurging] = useState<QuarantineBatch | null>(null);

  const load = useCallback(() => {
    setError(null);
    api
      .quarantineList()
      .then(setBatches)
      .catch((err) => {
        setBatches((old) => old ?? []);
        setError(errorText(err));
      });
  }, []);
  useEffect(load, [load]);

  const fail = (err: unknown) => {
    if (isCancelled(err)) toasts.show(t('Abgebrochen'));
    else toasts.show(t('Das ging nicht.'), { tone: 'error', detail: errorText(err) });
  };

  const restore = async (batch: QuarantineBatch) => {
    setBusy(batch.id);
    try {
      const failed = await api.quarantineRestore(batch.id);
      if (failed.length) {
        toasts.show(t('{count} Sachen konnte ich nicht zurücklegen.', { count: failed.length }), {
          tone: 'error',
          detail: failed.map(([path, why]) => `${path}: ${why}`).join('\n'),
        });
      } else {
        toasts.show(t('{name} ist wieder da.', { name: batch.appName }), { tone: 'success' });
      }
      load();
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
    }
  };

  const purge = async (batch: QuarantineBatch) => {
    setPurging(null);
    setBusy(batch.id);
    try {
      await api.quarantinePurge(batch.id);
      toasts.show(t('Endgültig gelöscht.'), { tone: 'success' });
      load();
    } catch (err) {
      fail(err);
    } finally {
      setBusy(null);
    }
  };

  const daysLeft = (batch: QuarantineBatch) =>
    Math.max(
      0,
      Number(settings.quarantineDays) - Math.floor((Date.now() / 1000 - batch.created) / DAY),
    );

  return (
    <section aria-labelledby="quarantine-title">
      <ViewHeader
        title={<span id="quarantine-title">{t('Quarantäne')}</span>}
        subtitle={t('Hier wartet alles, was ich weggeräumt habe, {days} Tage lang.', {
          days: settings.quarantineDays,
        })}
      />
      {error && (
        <Hint tone="danger" className="mx-6 mb-4">
          {error}
        </Hint>
      )}
      {batches === null ? (
        <div role="status" className="flex items-center gap-3 px-6 py-8 text-muted">
          <Spinner /> {t('Einen Moment …')}
        </div>
      ) : batches.length === 0 ? (
        <EmptyState
          art={<Art mood="happy" />}
          title={t('Die Quarantäne ist leer')}
          body={t('Wenn du Reste wegräumst, landen sie erst mal hier.')}
        />
      ) : (
        <ul className="flex flex-col gap-3 px-6 pb-6">
          {batches.map((batch) => (
            <li key={batch.id}>
              <Card
                title={batch.appName}
                subtitle={[
                  formatDate(batch.created),
                  t('{count} Sachen', { count: batch.items.length }),
                  formatBytes(sum(batch.items.map((item) => item.size))),
                  t('läuft ab in {days} Tagen', { days: daysLeft(batch) }),
                ]
                  .filter(Boolean)
                  .join(' · ')}
              >
                <div className="flex flex-wrap gap-2">
                  <Button
                    variant="primary"
                    size="sm"
                    icon={ICONS.restore}
                    busy={busy === batch.id}
                    disabled={busy !== null}
                    onClick={() => restore(batch)}
                  >
                    {t('Wiederherstellen')}
                  </Button>
                  <Button
                    variant="secondary"
                    size="sm"
                    icon={ICONS.folderOpen}
                    onClick={() => api.quarantineReveal(batch.id).catch(fail)}
                  >
                    {t('Ordner zeigen')}
                  </Button>
                  <Button
                    variant="danger"
                    size="sm"
                    icon={ICONS.delete}
                    disabled={busy !== null}
                    onClick={() => setPurging(batch)}
                  >
                    {t('Endgültig löschen')}
                  </Button>
                </div>
              </Card>
            </li>
          ))}
        </ul>
      )}
      <Dialog
        open={purging !== null}
        onClose={() => setPurging(null)}
        width="sm"
        tone="warning"
        title={t('Endgültig löschen?')}
        footer={
          <>
            <Button variant="secondary" onClick={() => setPurging(null)}>
              {t('Behalten')}
            </Button>
            <Button variant="danger" onClick={() => purging && purge(purging)}>
              {t('Endgültig löschen')}
            </Button>
          </>
        }
      >
        <p>
          {t(
            'Die Reste von {name} sind dann für immer weg. Das lässt sich nicht rückgängig machen.',
            {
              name: purging?.appName ?? '',
            },
          )}
        </p>
      </Dialog>
    </section>
  );
}
