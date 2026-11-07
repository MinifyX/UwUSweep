/** One program up close, with the two things to do with it. */

import { Avatar, avatarColor, Button, Card, Hint, ICONS } from '@uwusuite/design';
import { formatBytes, formatDate, type App } from '../lib/api';
import { t } from '../lib/i18n';
import { AppTags, sourceLabel } from './common';

export function AppDetail({
  app,
  onBack,
  onUninstall,
  onLeftovers,
}: {
  app: App;
  onBack: () => void;
  onUninstall: () => void;
  onLeftovers: () => void;
}) {
  const rows: [string, string | null | undefined][] = [
    [t('Hersteller'), app.publisher],
    [t('Version'), app.version],
    [t('Größe'), formatBytes(app.size)],
    [t('Installiert am'), formatDate(app.installedOn)],
    [t('Herkunft'), sourceLabel(app.source)],
    [t('Ordner'), app.installDir],
  ];
  return (
    <section aria-labelledby="detail-title" className="px-6 py-5">
      <Button variant="ghost" size="sm" icon={ICONS.back} onClick={onBack}>
        {t('Alle Programme')}
      </Button>
      <div className="mt-4 flex items-center gap-4">
        <Avatar name={app.name} color={avatarColor(app.name)} size="lg" />
        <div className="min-w-0">
          <h1 id="detail-title" className="text-title font-bold break-words">
            {app.name}
          </h1>
          <AppTags app={app} />
        </div>
      </div>
      <Card className="mt-5">
        <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-2 text-body">
          {rows
            .filter(([, value]) => value)
            .map(([label, value]) => (
              <div key={label} className="contents">
                <dt className="text-muted">{label}</dt>
                <dd className="min-w-0 break-all">{value}</dd>
              </div>
            ))}
        </dl>
      </Card>
      {app.needsAdmin && (
        <Hint tone="warning" className="mt-4">
          {t('Dafür brauche ich gleich Admin-Rechte. Dein System fragt dich danach.')}
        </Hint>
      )}
      <div className="mt-5 flex flex-wrap gap-3">
        <Button variant="primary" icon={ICONS.delete} onClick={onUninstall}>
          {t('Deinstallieren')}
        </Button>
        <Button variant="secondary" icon={ICONS.search} onClick={onLeftovers}>
          {t('Reste suchen')}
        </Button>
      </div>
    </section>
  );
}
