import { type FormEvent, useState } from "react";
import { AlertCircle, FlaskConical, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import type { RetentionPolicyRequest } from "@/api/generated/RetentionPolicyRequest";
import type { RetentionPolicyResponse } from "@/api/generated/RetentionPolicyResponse";
import {
  useApplyRetention,
  useDryRunRetention,
  useRetentionPolicy,
  useSaveRetentionPolicy,
} from "@/api/queries";
import {
  appliedCatalogMessage,
  previewCatalogMessage,
} from "@/components/cleanup/cleanupMessages";
import { CleanupPreviewTable } from "@/components/cleanup/CleanupPreviewTable";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";

const EMPTY_POLICY: RetentionPolicyResponse = { keep_last: null, keep_days: null };

function parseLimit(value: string): number | null | undefined {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return null;
  }
  const parsed = Number.parseInt(trimmed, 10);
  if (!Number.isFinite(parsed)) {
    return undefined;
  }
  return parsed;
}

function limitToInput(value: number | null): string {
  return value == null ? "" : String(value);
}

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

function payloadFromFields(
  keepLast: string,
  keepDays: string,
  invalidMessage: string,
): RetentionPolicyRequest | undefined {
  const last = parseLimit(keepLast);
  const days = parseLimit(keepDays);
  if (last === undefined || days === undefined) {
    toast.error(invalidMessage);
    return undefined;
  }
  return { keep_last: last ?? undefined, keep_days: days ?? undefined };
}

export function RetentionPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError, error } = useRetentionPolicy(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("retention.title")}</CardTitle>
        <CardDescription>{t("retention.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}
        {isError && error.message ? (
          isMissingMigration(error.message) ? (
            <MigrationAlert />
          ) : (
            <Alert variant="destructive">
              <AlertCircle />
              <AlertTitle>{t("retention.loadFailed")}</AlertTitle>
              <AlertDescription>
                {t("retention.loadFailedHint", { message: error.message })}
              </AlertDescription>
            </Alert>
          )
        ) : null}
        {isPending ? null : (
          <RetentionForm
            repositoryId={repositoryId}
            canWrite={canWrite}
            policy={data ?? EMPTY_POLICY}
          />
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
        <p>{t("retention.migrationBody")}</p>
        <pre className="mt-1 w-full overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs text-foreground">
          sqlx migrate run
        </pre>
        <p>{t("retention.migrationHint")}</p>
      </AlertDescription>
    </Alert>
  );
}

function RetentionForm({
  repositoryId,
  canWrite,
  policy,
}: {
  repositoryId: string;
  canWrite: boolean;
  policy: RetentionPolicyResponse;
}) {
  const { t } = useTranslation();
  const savePolicy = useSaveRetentionPolicy(repositoryId);
  const dryRun = useDryRunRetention(repositoryId);
  const applyRetention = useApplyRetention(repositoryId);
  const [keepLast, setKeepLast] = useState(limitToInput(policy.keep_last));
  const [keepDays, setKeepDays] = useState(limitToInput(policy.keep_days));
  const [preview, setPreview] = useState<CleanupPreviewResponse | null>(null);
  const [schemaError, setSchemaError] = useState(false);

  function rememberSchemaError(err: unknown) {
    const message = err instanceof ApiError ? err.message : "";
    if (isMissingMigration(message)) {
      setSchemaError(true);
    }
  }

  function onFieldsChange(nextLast: string, nextDays: string) {
    setKeepLast(nextLast);
    setKeepDays(nextDays);
    setPreview(null);
  }

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const payload = payloadFromFields(keepLast, keepDays, t("retention.limitsInvalid"));
    if (!payload) {
      return;
    }
    try {
      await savePolicy.mutateAsync(payload);
      setSchemaError(false);
      toast.success(t("retention.saved"));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : t("retention.saveFailed"));
    }
  }

  async function onSimulate() {
    const payload = payloadFromFields(keepLast, keepDays, t("retention.limitsInvalid"));
    if (!payload) {
      return;
    }
    try {
      const result = await dryRun.mutateAsync(payload);
      setPreview(result);
      setSchemaError(false);
      toast.success(previewCatalogMessage(result, t));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : t("retention.simulateFailed"));
    }
  }

  async function onApply() {
    const payload = payloadFromFields(keepLast, keepDays, t("retention.limitsInvalid"));
    if (!payload) {
      throw new Error(t("retention.limitsInvalid"));
    }
    try {
      const result = await applyRetention.mutateAsync(payload);
      setPreview(result);
      setSchemaError(false);
      toast.success(appliedCatalogMessage(result, t));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : t("retention.applyFailed"));
      throw err;
    }
  }

  const applyDescription =
    preview === null
      ? t("retention.applyHint")
      : t("retention.applyWithPreview", { preview: previewCatalogMessage(preview, t) });

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-5">
      {schemaError ? <MigrationAlert /> : null}

      <ol className="space-y-4 text-sm">
        <li className="space-y-3">
          <p className="font-medium text-foreground">{t("retention.step1")}</p>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="keep-last">{t("retention.keepLast")}</Label>
              <Input
                id="keep-last"
                inputMode="numeric"
                placeholder={t("retention.noLimit")}
                value={keepLast}
                onChange={(event) => onFieldsChange(event.target.value, keepDays)}
                disabled={!canWrite}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="keep-days">{t("retention.keepDays")}</Label>
              <Input
                id="keep-days"
                inputMode="numeric"
                placeholder={t("retention.noLimit")}
                value={keepDays}
                onChange={(event) => onFieldsChange(keepLast, event.target.value)}
                disabled={!canWrite}
              />
            </div>
          </div>
        </li>
        {canWrite ? (
          <li className="space-y-3">
            <p className="font-medium text-foreground">{t("retention.step2")}</p>
            <p className="text-muted-foreground">{t("retention.step2Hint")}</p>
            <Button
              type="button"
              variant="outline"
              disabled={dryRun.isPending}
              onClick={() => void onSimulate()}
            >
              <FlaskConical />
              {dryRun.isPending ? t("common.simulating") : t("common.simulate")}
            </Button>
          </li>
        ) : (
          <p className="text-muted-foreground">
            {t("retention.readOnly")}
          </p>
        )}
      </ol>

      {preview ? (
        <CleanupPreviewTable
          preview={preview}
          summary={
            preview.dry_run ? previewCatalogMessage(preview, t) : appliedCatalogMessage(preview, t)
          }
        />
      ) : null}

      {canWrite ? (
        <div className="space-y-3">
          <p className="text-sm font-medium text-foreground">{t("retention.step3")}</p>
          <p className="text-sm text-muted-foreground">{t("retention.step3Hint")}</p>
          <div className="flex flex-wrap gap-2">
            <Button type="submit" variant="outline" disabled={savePolicy.isPending}>
              <Save />
              {savePolicy.isPending ? t("common.saving") : t("retention.saveNoApply")}
            </Button>
            <ConfirmDeleteDialog
              title={t("retention.applyTitle")}
              description={applyDescription}
              confirmLabel={t("common.apply")}
              pending={applyRetention.isPending}
              onConfirm={onApply}
              trigger={
                <Button type="button" variant="destructive" disabled={preview === null}>
                  <Trash2 />
                  {t("retention.applyNow")}
                </Button>
              }
            />
          </div>
        </div>
      ) : null}
    </form>
  );
}
