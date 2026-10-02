import { useTranslation } from "react-i18next";

import { useOsvFeed } from "@/api/queries";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

function importedLabel(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

export function OsvFeedCard() {
  const { t } = useTranslation();
  const feed = useOsvFeed(true);

  return (
    <Card className="gap-3 py-4">
      <CardHeader className="px-4">
        <CardTitle className="text-sm">{t("feed.title")}</CardTitle>
      </CardHeader>
      <CardContent className="px-4 text-sm">
        {feed.isPending ? (
          <p className="text-muted-foreground">{t("feed.loading")}</p>
        ) : feed.isError ? (
          <p className="text-destructive">{t("feed.loadFailed")}</p>
        ) : feed.data ? (
          <dl className="grid gap-2 sm:grid-cols-3">
            <div>
              <dt className="text-xs text-muted-foreground">{t("feed.dataset")}</dt>
              <dd className="font-mono text-foreground">{feed.data.dataset}</dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">{t("feed.imported")}</dt>
              <dd className="text-foreground">{importedLabel(feed.data.imported_at)}</dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">{t("feed.mode")}</dt>
              <dd className="font-mono text-foreground">{feed.data.mode}</dd>
            </div>
          </dl>
        ) : (
          <p className="text-muted-foreground">{t("feed.empty")}</p>
        )}
      </CardContent>
    </Card>
  );
}
