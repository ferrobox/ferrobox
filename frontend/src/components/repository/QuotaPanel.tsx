import { type FormEvent, useState } from "react";
import { AlertCircle, HardDrive, Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { QuotaRequest } from "@/api/generated/QuotaRequest";
import type { QuotaResponse } from "@/api/generated/QuotaResponse";
import { useQuota, useSaveQuota } from "@/api/queries";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { formatBytes } from "@/lib/format";

const MIB = 1024 * 1024;
const GIB = MIB * 1024;
const TIB = GIB * 1024;
const MAX_LIMIT_BYTES = 10 * TIB;

type Unit = "MiB" | "GiB" | "TiB";

const UNIT_BYTES: Record<Unit, number> = {
  MiB: MIB,
  GiB: GIB,
  TiB: TIB,
};

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

function splitLimit(limitBytes: number | null): { value: string; unit: Unit } {
  if (limitBytes == null) {
    return { value: "", unit: "GiB" };
  }
  if (limitBytes % TIB === 0 && limitBytes >= TIB) {
    return { value: String(limitBytes / TIB), unit: "TiB" };
  }
  if (limitBytes % GIB === 0 && limitBytes >= GIB) {
    return { value: String(limitBytes / GIB), unit: "GiB" };
  }
  if (limitBytes % MIB === 0) {
    return { value: String(limitBytes / MIB), unit: "MiB" };
  }
  return { value: String(limitBytes / MIB), unit: "MiB" };
}

function payloadFromFields(
  value: string,
  unit: Unit,
  messages: { limitPositive: string; limitRange: string },
): QuotaRequest | undefined {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return { limit_bytes: undefined };
  }
  const parsed = Number.parseFloat(trimmed.replace(",", "."));
  if (!Number.isFinite(parsed) || parsed <= 0) {
    toast.error(messages.limitPositive);
    return undefined;
  }
  const limitBytes = Math.round(parsed * UNIT_BYTES[unit]);
  if (limitBytes < 1 || limitBytes > MAX_LIMIT_BYTES) {
    toast.error(messages.limitRange);
    return undefined;
  }
  return { limit_bytes: limitBytes };
}

export function QuotaPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const { data, isPending, isError, error } = useQuota(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("quota.title")}</CardTitle>
        <CardDescription>{t("quota.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}
        {isError && error.message ? (
          isMissingMigration(error.message) ? (
            <MigrationAlert />
          ) : (
            <Alert variant="destructive">
              <AlertCircle />
              <AlertTitle>{t("quota.loadFailed")}</AlertTitle>
              <AlertDescription>{error.message}</AlertDescription>
            </Alert>
          )
        ) : null}
        {isPending ? null : (
          <QuotaForm repositoryId={repositoryId} canWrite={canWrite} snapshot={data} />
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
        <p>{t("quota.migrationBody")}</p>
        <pre className="mt-1 w-full overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs text-foreground">
          sqlx migrate run
        </pre>
        <p>{t("quota.migrationHint")}</p>
      </AlertDescription>
    </Alert>
  );
}

function QuotaForm({
  repositoryId,
  canWrite,
  snapshot,
}: {
  repositoryId: string;
  canWrite: boolean;
  snapshot: QuotaResponse | undefined;
}) {
  const { t } = useTranslation();
  const saveQuota = useSaveQuota(repositoryId);
  const initial = splitLimit(snapshot?.limit_bytes ?? null);
  const [value, setValue] = useState(initial.value);
  const [unit, setUnit] = useState<Unit>(initial.unit);
  const [schemaError, setSchemaError] = useState(false);
  const usedBytes = snapshot?.used_bytes ?? 0;
  const savedLimit = snapshot?.limit_bytes ?? null;
  const overLimit = savedLimit != null && usedBytes > savedLimit;
  const percent =
    savedLimit != null && savedLimit > 0 ? Math.min(100, (usedBytes / savedLimit) * 100) : 0;

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const payload = payloadFromFields(value, unit, {
      limitPositive: t("quota.limitPositive"),
      limitRange: t("quota.limitRange"),
    });
    if (!payload) {
      return;
    }
    try {
      await saveQuota.mutateAsync(payload);
      setSchemaError(false);
      toast.success(
        payload.limit_bytes == null ? t("quota.unlimitedSaved") : t("quota.limitSaved"),
      );
    } catch (err) {
      const message = err instanceof ApiError ? err.message : "";
      if (isMissingMigration(message)) {
        setSchemaError(true);
      }
      toast.error(err instanceof ApiError ? err.message : t("quota.saveFailed"));
    }
  }

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-5">
      {schemaError ? <MigrationAlert /> : null}

      <div className="space-y-2">
        <p className="text-sm font-medium text-foreground">{t("quota.usage")}</p>
        <p className="text-sm text-muted-foreground">
          {savedLimit == null
            ? t("quota.usedUnlimited", { used: formatBytes(usedBytes) })
            : t("quota.usedOf", { used: formatBytes(usedBytes), limit: formatBytes(savedLimit) })}
        </p>
        {savedLimit != null ? (
          <div className="h-2 overflow-hidden rounded-full bg-muted">
            <div
              className={`h-full ${overLimit ? "bg-destructive" : "bg-primary"}`}
              style={{ width: `${percent.toFixed(1)}%` }}
            />
          </div>
        ) : null}
        {overLimit ? <p className="text-sm text-destructive">{t("quota.overLimit")}</p> : null}
      </div>

      <div className="space-y-3">
        <p className="text-sm font-medium text-foreground">{t("quota.cap")}</p>
        <div className="flex flex-wrap items-end gap-2">
          <div className="space-y-2">
            <Label htmlFor="quota-limit">{t("quota.amount")}</Label>
            <Input
              id="quota-limit"
              inputMode="decimal"
              placeholder={t("quota.unlimitedPlaceholder")}
              value={value}
              onChange={(event) => setValue(event.target.value)}
              disabled={!canWrite}
              className="w-40"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="quota-unit">{t("quota.unit")}</Label>
            <Select
              value={unit}
              onValueChange={(next) => setUnit(next as Unit)}
              disabled={!canWrite}
            >
              <SelectTrigger id="quota-unit">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="MiB">MiB</SelectItem>
                <SelectItem value="GiB">GiB</SelectItem>
                <SelectItem value="TiB">TiB</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
        <p className="text-sm text-muted-foreground">{t("quota.emptyHint")}</p>
      </div>

      {canWrite ? (
        <Button type="submit" variant="outline" disabled={saveQuota.isPending}>
          {saveQuota.isPending ? <HardDrive className="animate-pulse" /> : <Save />}
          {saveQuota.isPending ? t("common.saving") : t("common.save")}
        </Button>
      ) : (
        <p className="text-sm text-muted-foreground">{t("quota.readOnly")}</p>
      )}
    </form>
  );
}
