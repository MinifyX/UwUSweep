/** Settings: appearance, language, how uninstalling behaves, the log, about. */

import {
  Button,
  Card,
  Dialog,
  ICONS,
  Segmented,
  SettingRow,
  Switch,
  TEXT_SIZE_CHOICES,
  TEXT_SIZE_LABELS,
} from '@uwusuite/design';
import { api, errorText, type PlatformInfo } from '../lib/api';
import { t, useLanguage } from '../lib/i18n';
import { QUARANTINE_DAYS, updateSettings, useSettings } from '../lib/settings';
import { toasts } from '../lib/toasts';

const REPO = 'https://github.com/MinifyX/UwUSweep';

export function SettingsDialog({
  open,
  onClose,
  info,
}: {
  open: boolean;
  onClose: () => void;
  info: PlatformInfo | null;
}) {
  const s = useSettings();
  const lang = useLanguage();
  const fail = (error: unknown) =>
    toasts.show(t('Das ging nicht.'), { tone: 'error', detail: errorText(error) });

  return (
    <Dialog open={open} onClose={onClose} width="lg" title={t('Einstellungen')}>
      <div className="flex flex-col gap-4">
        <Card title={t('Darstellung')}>
          <SettingRow label={t('Design')}>
            <Segmented
              label={t('Design')}
              value={s.theme}
              onChange={(theme) => updateSettings({ theme })}
              options={[
                { value: 'system', label: t('System') },
                { value: 'light', label: t('Hell') },
                { value: 'dark', label: t('Dunkel') },
              ]}
            />
          </SettingRow>
          <SettingRow label={t('Kontrast')}>
            <Segmented
              label={t('Kontrast')}
              value={s.contrast}
              onChange={(contrast) => updateSettings({ contrast })}
              options={[
                { value: 'system', label: t('System') },
                { value: 'normal', label: t('Normal') },
                { value: 'high', label: t('Hoch') },
              ]}
            />
          </SettingRow>
          <SettingRow label={t('Textgröße')}>
            <Segmented
              label={t('Textgröße')}
              value={s.textSize}
              onChange={(textSize) => updateSettings({ textSize })}
              options={TEXT_SIZE_CHOICES.map((value) => ({
                value,
                label: TEXT_SIZE_LABELS[lang][value],
              }))}
            />
          </SettingRow>
          <SettingRow
            label={t('Animationen')}
            description={t('System folgt „Bewegung reduzieren“.')}
          >
            <Segmented
              label={t('Animationen')}
              value={s.motion}
              onChange={(motion) => updateSettings({ motion })}
              options={[
                { value: 'system', label: t('System') },
                { value: 'on', label: t('An') },
                { value: 'off', label: t('Aus') },
              ]}
            />
          </SettingRow>
          <SettingRow label={t('Sprache')}>
            <Segmented
              label={t('Sprache')}
              value={s.language}
              onChange={(language) => updateSettings({ language })}
              options={[
                { value: 'system', label: t('System') },
                { value: 'de', label: 'Deutsch' },
                { value: 'en', label: 'English' },
              ]}
            />
          </SettingRow>
        </Card>

        <Card title={t('Aufräumen')}>
          <SettingRow
            label={t('Suchtiefe')}
            description={t('Gründlich findet mehr, aber auch mehr Unsicheres.')}
          >
            <Segmented
              label={t('Suchtiefe')}
              value={s.scanLevel}
              onChange={(scanLevel) => updateSettings({ scanLevel })}
              options={[
                { value: 'safe', label: t('Sicher') },
                { value: 'deep', label: t('Gründlich') },
              ]}
            />
          </SettingRow>
          <SettingRow
            label={t('Quarantäne aufheben')}
            description={t('Danach lösche ich weggeräumte Reste endgültig.')}
          >
            <Segmented
              label={t('Quarantäne aufheben')}
              value={s.quarantineDays}
              onChange={(quarantineDays) => updateSettings({ quarantineDays })}
              options={QUARANTINE_DAYS.map((value) => ({
                value,
                label: t('{days} Tage', { days: value }),
              }))}
            />
          </SettingRow>
          {info?.restorePointSupported && (
            <SettingRow
              label={t('Wiederherstellungspunkt')}
              description={t('Vor jeder Deinstallation vorgeschlagen.')}
            >
              <Switch
                checked={s.restorePoint}
                onChange={(restorePoint) => updateSettings({ restorePoint })}
                label={t('Wiederherstellungspunkt')}
              />
            </SettingRow>
          )}
        </Card>

        <Card title={t('Fehlersuche')}>
          <SettingRow
            label={t('Ausführliches Protokoll')}
            description={t('Schreibt jeden Schritt mit. Hilft, wenn du einen Fehler meldest.')}
          >
            <Switch
              checked={s.detailedLog}
              onChange={(detailedLog) => updateSettings({ detailedLog })}
              label={t('Ausführliches Protokoll')}
            />
          </SettingRow>
          <SettingRow label={t('Protokolle')}>
            <Button
              variant="secondary"
              size="sm"
              icon={ICONS.folderOpen}
              onClick={() => api.openLogFolder().catch(fail)}
            >
              {t('Ordner öffnen')}
            </Button>
          </SettingRow>
        </Card>

        <Card title={t('Über UwUSweep')}>
          <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-2 text-body">
            <dt className="text-muted">{t('Version')}</dt>
            <dd>{info?.version ?? '–'}</dd>
            <dt className="text-muted">{t('Lizenz')}</dt>
            <dd>GPL-3.0</dd>
            <dt className="text-muted">{t('Quellcode')}</dt>
            <dd>
              <Button
                variant="ghost"
                size="sm"
                icon={ICONS.openExternal}
                onClick={() => api.openLink(REPO).catch(fail)}
              >
                github.com/MinifyX/UwUSweep
              </Button>
            </dd>
          </dl>
        </Card>
      </div>
    </Dialog>
  );
}
