/**
 * Uninstalling one program, step by step: options → its own uninstaller →
 * leftover search → choosing what goes → into the quarantine → done.
 *
 * "Reste suchen" starts the same wizard without the uninstaller.
 * Nothing is ever deleted here: leftovers go into the quarantine, and
 * "Rückgängig" puts them back.
 */

import {
  Button,
  Dialog,
  EmptyState,
  Hint,
  Icon,
  ICONS,
  Nyu,
  Segmented,
  Spinner,
  Tag,
  Toggle,
} from '@uwusuite/design';
import { useId, useState, type ReactNode } from 'react';
import {
  api,
  canQuiet,
  errorText,
  formatBytes,
  isCancelled,
  isRegistry,
  sum,
  type App,
  type Failure,
  type Leftover,
  type PlatformInfo,
  type ScanLevel,
  type SweepReport,
} from '../lib/api';
import { t } from '../lib/i18n';
import { useSettings } from '../lib/settings';
import { toasts } from '../lib/toasts';
import { Art } from './common';

export type WizardMode = 'uninstall' | 'leftovers';

type Step =
  | { kind: 'options' }
  | { kind: 'uninstalling' }
  | { kind: 'scanning' }
  | { kind: 'review'; found: Leftover[] }
  | { kind: 'sweeping' }
  | { kind: 'done'; report: SweepReport; restored: boolean }
  | { kind: 'cancelled' }
  | { kind: 'failed'; message: string };

const key = (item: Leftover) => `${item.kind}\u0000${item.path}\u0000${item.value ?? ''}`;

export function UninstallWizard({
  app,
  mode,
  info,
  onClose,
}: {
  app: App;
  mode: WizardMode;
  info: PlatformInfo | null;
  /** `changed`: the app list should be read again. */
  onClose: (changed: boolean) => void;
}) {
  const settings = useSettings();
  const [step, setStep] = useState<Step>({ kind: 'options' });
  const [level, setLevel] = useState<ScanLevel>(settings.scanLevel);
  const [restorePoint, setRestorePoint] = useState(settings.restorePoint);
  const [quiet, setQuiet] = useState(false);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [changed, setChanged] = useState(false);
  const [undoing, setUndoing] = useState(false);

  const busy = ['uninstalling', 'scanning', 'sweeping'].includes(step.kind);
  const restorePointShown = mode === 'uninstall' && Boolean(info?.restorePointSupported);
  const macBundle = app.source.kind === 'macApp';

  const scan = async () => {
    setStep({ kind: 'scanning' });
    try {
      const found = await api.leftoversScan(app.id, level);
      setChosen(new Set(found.filter((item) => item.confidence === 'high').map(key)));
      setStep({ kind: 'review', found });
    } catch (error) {
      setStep({ kind: 'failed', message: errorText(error) });
    }
  };

  const start = async () => {
    if (mode === 'leftovers') return scan();
    setStep({ kind: 'uninstalling' });
    try {
      const result = await api.uninstallStart(
        app.id,
        quiet && canQuiet(app),
        restorePointShown && restorePoint,
      );
      setChanged(true);
      if (result.restorePointError) {
        toasts.show(t('Der Wiederherstellungspunkt ging nicht. Ich habe trotzdem weitergemacht.'), {
          tone: 'error',
          detail: result.restorePointError,
        });
      }
    } catch (error) {
      if (isCancelled(error)) return setStep({ kind: 'cancelled' });
      return setStep({ kind: 'failed', message: errorText(error) });
    }
    await scan();
  };

  const sweep = async (found: Leftover[]) => {
    const items = found.filter((item) => chosen.has(key(item)));
    setStep({ kind: 'sweeping' });
    try {
      const report = await api.quarantinePut(app.name, items);
      setStep({ kind: 'done', report, restored: false });
    } catch (error) {
      setStep({ kind: 'failed', message: errorText(error) });
    }
  };

  const undo = async (report: SweepReport) => {
    setUndoing(true);
    const failed: Failure[] = [];
    for (const batch of report.batches) {
      try {
        failed.push(...(await api.quarantineRestore(batch.id)));
      } catch (error) {
        failed.push([batch.appName, errorText(error)]);
      }
    }
    setUndoing(false);
    if (failed.length) {
      toasts.show(t('{count} Sachen konnte ich nicht zurücklegen.', { count: failed.length }), {
        tone: 'error',
        detail: failed.map(([path, why]) => `${path}: ${why}`).join('\n'),
      });
    } else {
      toasts.show(t('Alles wieder an seinem Platz.'), { tone: 'success' });
    }
    setStep({ kind: 'done', report, restored: true });
  };

  const close = () => {
    if (!busy) onClose(changed);
  };

  let body: ReactNode;
  let footer: ReactNode;
  switch (step.kind) {
    case 'options':
      body = (
        <div className="flex flex-col gap-4">
          <p>
            {mode === 'uninstall'
              ? t(
                  'Ich starte das Deinstallationsprogramm von {name} und suche danach nach Resten.',
                  {
                    name: app.name,
                  },
                )
              : t('Ich suche nach Dateien und Einträgen, die {name} hinterlassen hat.', {
                  name: app.name,
                })}
          </p>
          {restorePointShown && (
            <Toggle
              checked={restorePoint}
              onChange={setRestorePoint}
              label={t('Wiederherstellungspunkt anlegen')}
              description={t('Damit kannst du Windows auf den Stand von vorher zurücksetzen.')}
            />
          )}
          {mode === 'uninstall' && canQuiet(app) && (
            <Toggle
              checked={quiet}
              onChange={setQuiet}
              label={t('Unbeaufsichtigt')}
              description={t('Ohne Rückfragen des Programms deinstallieren.')}
            />
          )}
          <div className="flex flex-col gap-2">
            <Segmented
              label={t('Suchtiefe')}
              value={level}
              onChange={setLevel}
              options={[
                { value: 'safe', label: t('Sicher') },
                { value: 'deep', label: t('Gründlich') },
              ]}
            />
            <p className="text-meta text-muted">
              {level === 'safe'
                ? t('Nur Orte, die genau nach dem Programm heißen.')
                : t('Auch ähnliche Namen und Systemorte. Findet mehr, schau genauer hin.')}
            </p>
          </div>
        </div>
      );
      footer = (
        <>
          <Button variant="secondary" onClick={close}>
            {t('Abbrechen')}
          </Button>
          <Button variant="primary" onClick={start}>
            {mode === 'uninstall' ? t('Deinstallieren') : t('Reste suchen')}
          </Button>
        </>
      );
      break;
    case 'uninstalling':
      body = (
        <Progress>
          {macBundle
            ? t('Wird in die Quarantäne verschoben …')
            : t(
                'Das Deinstallationsprogramm läuft. Schau nach seinem Fenster, es braucht vielleicht deine Antwort.',
              )}
        </Progress>
      );
      break;
    case 'scanning':
      body = <Progress>{t('Ich suche nach Resten …')}</Progress>;
      break;
    case 'sweeping':
      body = <Progress>{t('Ich räume auf …')}</Progress>;
      break;
    case 'review': {
      const { found } = step;
      if (found.length === 0) {
        body = (
          <EmptyState
            compact
            art={<Art mood="happy" />}
            title={t('Nichts übrig geblieben')}
            body={t('{name} hat sauber hinter sich aufgeräumt. Yay!', { name: app.name })}
          />
        );
        footer = (
          <Button variant="primary" onClick={close}>
            {t('Fertig')}
          </Button>
        );
        break;
      }
      const files = found.filter((item) => !isRegistry(item.kind));
      const registry = found.filter((item) => isRegistry(item.kind));
      const picked = found.filter((item) => chosen.has(key(item)));
      const toggle = (item: Leftover, on: boolean) =>
        setChosen((old) => {
          const next = new Set(old);
          if (on) next.add(key(item));
          else next.delete(key(item));
          return next;
        });
      body = (
        <div className="flex flex-col gap-4">
          <p>
            {t('{count} Reste gefunden. Sichere Treffer sind schon ausgewählt.', {
              count: found.length,
            })}
          </p>
          {found.some((item) => item.confidence === 'medium') && (
            <Hint tone="warning">
              {t(
                'Unsichere Treffer sind nicht ausgewählt: Ihr Name passt nur teilweise. Schau sie dir an, bevor du sie mitnimmst.',
              )}
            </Hint>
          )}
          <LeftoverGroup
            title={t('Dateien und Ordner')}
            items={files}
            chosen={chosen}
            onToggle={toggle}
          />
          <LeftoverGroup title={t('Registry')} items={registry} chosen={chosen} onToggle={toggle} />
        </div>
      );
      footer = (
        <>
          <span className="mr-auto text-meta text-muted" aria-live="polite">
            {t('{count} ausgewählt · {size}', {
              count: picked.length,
              size: formatBytes(sum(picked.map((item) => item.size))),
            })}
          </span>
          <Button variant="secondary" onClick={close}>
            {t('Nichts verschieben')}
          </Button>
          <Button
            variant="primary"
            icon={ICONS.archive}
            disabled={picked.length === 0}
            onClick={() => sweep(found)}
          >
            {t('In Quarantäne verschieben')}
          </Button>
        </>
      );
      break;
    }
    case 'done': {
      const { report, restored } = step;
      const moved = report.batches.flatMap((batch) => batch.items);
      body = (
        <div className="flex flex-col items-center gap-3 text-center">
          <div className="w-[150px]">
            <Nyu shell="box" mood={restored ? 'uwu' : 'cheer'} title="" />
          </div>
          <h3 className="text-section font-bold" role="status">
            {restored
              ? t('Rückgängig gemacht')
              : t('{count} Reste in der Quarantäne', { count: moved.length })}
          </h3>
          {!restored && (
            <p className="text-muted">
              {t('{size} frei geworden. Sie bleiben {days} Tage, dann lösche ich sie.', {
                size: formatBytes(sum(moved.map((item) => item.size))),
                days: settings.quarantineDays,
              })}
            </p>
          )}
          {report.failed.length > 0 && <FailedList failed={report.failed} />}
        </div>
      );
      footer = (
        <>
          {!restored && report.batches.length > 0 && (
            <Button
              variant="secondary"
              icon={ICONS.undo}
              busy={undoing}
              onClick={() => undo(report)}
            >
              {t('Rückgängig')}
            </Button>
          )}
          <Button variant="primary" onClick={close} disabled={undoing}>
            {t('Fertig')}
          </Button>
        </>
      );
      break;
    }
    case 'cancelled':
      body = (
        <EmptyState
          compact
          art={<Art mood="sleepy" />}
          title={t('Abgebrochen')}
          body={t('Alles gut, es wurde nichts verändert.')}
        />
      );
      footer = (
        <Button variant="primary" onClick={close}>
          {t('Schließen')}
        </Button>
      );
      break;
    case 'failed':
      body = (
        <div className="flex flex-col gap-3">
          <Hint tone="danger">{step.message}</Hint>
          {mode === 'uninstall' && (
            <p>{t('Du kannst trotzdem nach Resten suchen, falls schon etwas entfernt wurde.')}</p>
          )}
        </div>
      );
      footer = (
        <>
          <Button variant="secondary" onClick={close}>
            {t('Schließen')}
          </Button>
          <Button variant="primary" icon={ICONS.search} onClick={scan}>
            {t('Reste suchen')}
          </Button>
        </>
      );
      break;
  }

  return (
    <Dialog
      open
      onClose={close}
      width="lg"
      closeOnOutsideClick={false}
      title={
        mode === 'uninstall'
          ? t('{name} deinstallieren', { name: app.name })
          : t('Reste von {name}', { name: app.name })
      }
      footer={footer}
    >
      {body}
    </Dialog>
  );
}

function Progress({ children }: { children: ReactNode }) {
  return (
    <div role="status" aria-live="polite" className="flex items-center gap-3 py-6">
      <Spinner />
      <span>{children}</span>
    </div>
  );
}

function LeftoverGroup({
  title,
  items,
  chosen,
  onToggle,
}: {
  title: string;
  items: Leftover[];
  chosen: Set<string>;
  onToggle: (item: Leftover, on: boolean) => void;
}) {
  const base = useId();
  if (items.length === 0) return null;
  return (
    <fieldset className="flex flex-col gap-1">
      <legend className="mb-1 text-meta font-semibold text-muted">{title}</legend>
      {items.map((item, index) => {
        const id = `${base}-${index}`;
        const registry = isRegistry(item.kind);
        return (
          <div
            key={key(item)}
            className="grid grid-cols-[auto_1fr_auto] items-start gap-3 rounded-control px-2 py-2 hover:bg-elevated"
          >
            <input
              id={id}
              type="checkbox"
              className="mt-1 size-4 accent-[var(--color-pink-solid)]"
              checked={chosen.has(key(item))}
              onChange={(event) => onToggle(item, event.target.checked)}
            />
            <label htmlFor={id} className="min-w-0">
              <span className="uwu-mono block text-meta break-all">
                {item.path}
                {item.value ? ` → ${item.value}` : ''}
              </span>
              <span className="mt-1 flex flex-wrap items-center gap-1.5 text-caption text-muted">
                {item.confidence === 'medium' && <Tag tone="warning">{t('Unsicher')}</Tag>}
                {item.needsAdmin && <Tag tone="warning">{t('Admin')}</Tag>}
                <span>{item.reason === 'trace' ? t('aus einer Aufzeichnung') : item.reason}</span>
                {item.size != null && <span>· {formatBytes(item.size)}</span>}
              </span>
            </label>
            {!registry && (
              <Button
                variant="ghost"
                size="sm"
                icon={ICONS.folderOpen}
                onClick={() =>
                  api.revealPath(item.path).catch((error) =>
                    toasts.show(t('Das ging nicht.'), {
                      tone: 'error',
                      detail: errorText(error),
                    }),
                  )
                }
              >
                {t('Im Ordner zeigen')}
              </Button>
            )}
          </div>
        );
      })}
    </fieldset>
  );
}

export function FailedList({ failed }: { failed: Failure[] }) {
  return (
    <div className="w-full text-left">
      <p className="mb-1 flex items-center gap-2 font-semibold text-danger-ink">
        <Icon icon={ICONS.warning} size="sm" />
        {t('Das hat nicht geklappt:')}
      </p>
      <ul className="flex max-h-40 flex-col gap-1 overflow-auto text-meta">
        {failed.map(([path, why]) => (
          <li key={path}>
            <span className="uwu-mono break-all">{path}</span>{' '}
            <span className="text-muted">({why === 'cancelled' ? t('abgebrochen') : why})</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
