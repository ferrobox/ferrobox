import { type FormEvent, useState } from "react";
import { AlertCircle, Lock, Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { WormResponse } from "@/api/generated/WormResponse";
import { useSaveWorm, useWorm } from "@/api/queries";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

export function WormPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError, error } = useWorm(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("worm.title")}</CardTitle>
        <CardDescription>{t("worm.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}
        {isError && error.message ? (
          isMissingMigration(error.message) ? (
            <MigrationAlert />
          ) : (
            <Alert variant="destructive">
              <AlertCircle />
              <AlertTitle>{t("worm.loadFailed")}</AlertTitle>
              <AlertDescription>{error.message}</AlertDescription>
            </Alert>
          )
        ) : null}
        {isPending ? null : (
          <WormForm repositoryId={repositoryId} canWrite={canWrite} snapshot={data} />
        )}
      </CardContent>
    </Card>
  );
}

function MigrationAlert() {
  const { t } = useTranslation();
  return (
    <Alert variant="destructive">
      <AlertCircle />
      <AlertTitle>{t("retention.migrationTitle")}</AlertTitle>
      <AlertDescription className="gap-2">
        <p>{t("worm.migrationBody")}</p>
        <pre className="mt-1 w-full overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs text-foreground">
          sqlx migrate run
        </pre>
        <p>{t("worm.migrationHint")}</p>
      </AlertDescription>
    </Alert>
  );
}

function WormForm({
  repositoryId,
  canWrite,
  snapshot,
}: {
  repositoryId: string;
  canWrite: boolean;
  snapshot: WormResponse | undefined;
}) {
  const { t } = useTranslation();
  const saveWorm = useSaveWorm(repositoryId);
  const [enabled, setEnabled] = useState(snapshot?.enabled ?? false);
  const [schemaError, setSchemaError] = useState(false);

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      await saveWorm.mutateAsync({ enabled });
      setSchemaError(false);
      toast.success(enabled ? t("worm.enabledSaved") : t("worm.disabledSaved"));
    } catch (err) {
      const message = err instanceof ApiError ? err.message : "";
      if (isMissingMigration(message)) {
        setSchemaError(true);
      }
      toast.error(err instanceof ApiError ? err.message : t("worm.saveFailed"));
    }
  }

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-5">
      {schemaError ? <MigrationAlert /> : null}

      <label className="flex cursor-pointer items-start gap-2">
        <input
          type="checkbox"
          className="mt-1 size-4 accent-primary"
          checked={enabled}
          onChange={(event) => setEnabled(event.target.checked)}
          disabled={!canWrite}
        />
        <span>
          <span className="font-medium">{t("worm.enable")}</span>
          <span className="block text-xs text-muted-foreground">{t("worm.enableHint")}</span>
        </span>
      </label>

      {canWrite ? (
        <Button type="submit" variant="outline" disabled={saveWorm.isPending}>
          {saveWorm.isPending ? <Lock className="animate-pulse" /> : <Save />}
          {saveWorm.isPending ? t("common.saving") : t("common.save")}
        </Button>
      ) : (
        <p className="text-sm text-muted-foreground">{t("worm.readOnly")}</p>
      )}
    </form>
  );
}
