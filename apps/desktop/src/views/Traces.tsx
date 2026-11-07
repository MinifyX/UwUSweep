/**
 * Install traces: a snapshot before an install and one after. What is new
 * afterwards belongs to the program, so the leftover search finds it later
 * even when its name does not give it away.
 */

import { Button, Card, EmptyState, Field, Hint, ICONS, Spinner, TextInput } from '@uwusuite/design';
import { useCallback, useEffect, useState } from 'react';
import { api, errorText, formatBytes, formatDate, isRegistry, sum, type Trace } from '../lib/api';
import { t } from '../lib/i18n';
import { toasts } from '../lib/toasts';
import { Art, ViewHeader } from './common';

type Phase = 'idle' | 'starting' | 'recording' | 'finishing';

// The snapshot lives in Rust; this remembers it when the view is left and opened again.
let recording = false;

export function Traces() {
  const [phase, setPhase] = useState<Phase>(recording ? 'recording' : 'idle');
  const [name, setName] = useState('');
  const [result, setResult] = useState<Trace | null>(null);
  const [traces, setTraces] = useState<Trace[] | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .tracesList()
      .then(setTraces)
      .catch((err) => {
        setTraces((old) => old ?? []);
        toasts.show(t('Das ging nicht.'), { tone: 'error', detail: errorText(err) });
      });
  }, []);
  useEffect(load, [load]);

  const fail = (err: unknown) =>
    toasts.show(t('Das ging nicht.'), { tone: 'error', detail: errorText(err) });

  const start = async () => {
    setResult(null);
    setPhase('starting');
    try {
      await api.traceStart();
      recording = true;
      setPhase('recording');
    } catch (err) {
      setPhase('idle');
      fail(err);
    }
  };

  const finish = async () => {
    setPhase('finishing');
    try {
      const trace = await api.traceFinish(name);
      recording = false;
      setName('');
      setResult(trace);
      setPhase('idle');
      load();
    } catch (err) {
      setPhase('recording');
      fail(err);
    }
  };

  const cancel = async () => {
    await api.traceCancel().catch(() => undefined);
    recording = false;
    setPhase('idle');
  };

  const pick = async () => {
    try {
      const path = await api.installPick();
      if (path) await api.installerOpen(path);
    } catch (err) {
      fail(err);
    }
  };

  const remove = async (trace: Trace) => {
    try {
      await api.traceDelete(trace.id);
      toasts.show(t('Aufzeichnung „{name}“ gelöscht.', { name: trace.name }), { tone: 'success' });
      load();
    } catch (err) {
      fail(err);
    }
  };

  return (
    <section aria-labelledby="trace-title">
      <ViewHeader
        title={<span id="trace-title">{t('Mitschreiben')}</span>}
        subtitle={t(
          'Ich merke mir, was eine Installation alles anlegt. Später finde ich so jeden Rest.',
        )}
      />
      <div className="flex flex-col gap-4 px-6 pb-6">
        <Card>
          <div role="status" aria-live="polite" className="flex flex-col gap-4">
            {phase === 'idle' && (
              <>
                <p>
                  {t(
                    'Starte die Aufzeichnung, installiere dann das Programm und klick auf „Fertig“.',
                  )}
                </p>
                <div>
                  <Button variant="primary" icon={ICONS.start} onClick={start}>
                    {t('Aufzeichnung starten')}
                  </Button>
                </div>
              </>
            )}
            {(phase === 'starting' || phase === 'finishing') && (
              <p className="flex items-center gap-3">
                <Spinner />
                {phase === 'starting'
                  ? t('Ich mache einen Schnappschuss … das dauert einen Moment.')
                  : t('Ich vergleiche mit vorher …')}
              </p>
            )}
            {phase === 'recording' && (
              <>
                <p className="font-semibold">{t('Läuft – installiere jetzt das Programm.')}</p>
                <div>
                  <Button variant="secondary" icon={ICONS.folderOpen} onClick={pick}>
                    {t('Installer auswählen …')}
                  </Button>
                </div>
                <Field
                  label={t('Name der Aufzeichnung')}
                  hint={t('Leer lassen, dann nehme ich den Namen des neuen Programms.')}
                >
                  {(id, describedBy) => (
                    <TextInput
                      id={id}
                      aria-describedby={describedBy}
                      value={name}
                      maxLength={120}
                      onChange={(event) => setName(event.target.value)}
                    />
                  )}
                </Field>
                <div className="flex flex-wrap gap-2">
                  <Button variant="primary" icon={ICONS.done} onClick={finish}>
                    {t('Fertig')}
                  </Button>
                  <Button variant="ghost" onClick={cancel}>
                    {t('Abbrechen')}
                  </Button>
                </div>
              </>
            )}
          </div>
          {result && (
            <Hint tone="success" className="mt-4">
              {t('„{name}“ gespeichert: {count} neue Sachen, {apps} neue Programme, {size}.', {
                name: result.name,
                count: result.items.length,
                apps: result.appIds.length,
                size: formatBytes(sum(result.items.map((item) => item.size))),
              })}
            </Hint>
          )}
        </Card>

        <h2 className="mt-2 text-section font-bold">{t('Gespeicherte Aufzeichnungen')}</h2>
        {traces === null ? (
          <p role="status" className="flex items-center gap-3 text-muted">
            <Spinner /> {t('Einen Moment …')}
          </p>
        ) : traces.length === 0 ? (
          <EmptyState
            compact
            art={<Art mood="sleepy" />}
            title={t('Noch keine Aufzeichnungen')}
            body={t('Schreib bei der nächsten Installation mit, dann steht sie hier.')}
          />
        ) : (
          <ul className="flex flex-col gap-3">
            {traces.map((trace) => {
              const open = openId === trace.id;
              const files = trace.items.filter((item) => !isRegistry(item.kind)).length;
              return (
                <li key={trace.id}>
                  <Card
                    title={trace.name}
                    subtitle={[
                      formatDate(trace.created),
                      t('{files} Dateien, {keys} Registry-Einträge', {
                        files,
                        keys: trace.items.length - files,
                      }),
                    ].join(' · ')}
                    aside={
                      <div className="flex gap-2">
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-expanded={open}
                          onClick={() => setOpenId(open ? null : trace.id)}
                        >
                          {open ? t('Weniger') : t('Details')}
                        </Button>
                        <Button
                          variant="danger"
                          size="sm"
                          icon={ICONS.delete}
                          onClick={() => remove(trace)}
                        >
                          {t('Löschen')}
                        </Button>
                      </div>
                    }
                  >
                    {open && (
                      <ul className="flex max-h-72 flex-col gap-1 overflow-auto text-meta">
                        {trace.items.map((item) => (
                          <li
                            key={`${item.kind}:${item.path}:${item.value ?? ''}`}
                            className="uwu-mono break-all"
                          >
                            {item.path}
                            {item.value ? ` → ${item.value}` : ''}
                          </li>
                        ))}
                      </ul>
                    )}
                  </Card>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}
