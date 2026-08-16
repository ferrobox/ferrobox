import { type FormEvent, useState } from "react";
import { AlertCircle, Eraser, Save, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { CleanupReportResponse } from "@/api/generated/CleanupReportResponse";
import type { RetentionPolicyResponse } from "@/api/generated/RetentionPolicyResponse";
import {
  useApplyRetention,
  useCollectGarbage,
  useRetentionPolicy,
  useSaveRetentionPolicy,
} from "@/api/queries";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { formatBytes } from "@/lib/format";

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

function reportMessage(report: CleanupReportResponse): string {
  return `Eliminadas ${String(report.dropped_versions)} versiones y ${String(report.deleted_artifacts)} binarios (${formatBytes(report.freed_bytes)}).`;
}

function limitToInput(value: number | null): string {
  return value == null ? "" : String(value);
}

export function RetentionPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { data, isPending, isError, error } = useRetentionPolicy(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Retención y basura</CardTitle>
        <CardDescription>
          Conserva las N versiones más recientes y/o las publicadas en los últimos N días. Vacío =
          no hay límite. Aplicar borra el resto y los binarios huérfanos. No bloquea install ni
          publish.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}
        {isError ? (
          <Alert variant="destructive">
            <AlertCircle />
            <AlertTitle>No se pudo cargar la política</AlertTitle>
            <AlertDescription>{error.message}</AlertDescription>
          </Alert>
        ) : null}
        {data ? (
          <RetentionForm repositoryId={repositoryId} canWrite={canWrite} policy={data} />
        ) : null}
      </CardContent>
    </Card>
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
  const savePolicy = useSaveRetentionPolicy(repositoryId);
  const applyRetention = useApplyRetention(repositoryId);
  const collectGarbage = useCollectGarbage(repositoryId);
  const [keepLast, setKeepLast] = useState(limitToInput(policy.keep_last));
  const [keepDays, setKeepDays] = useState(limitToInput(policy.keep_days));

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const last = parseLimit(keepLast);
    const days = parseLimit(keepDays);
    if (last === undefined || days === undefined) {
      toast.error("Los límites deben ser números enteros, o quedar vacíos");
      return;
    }
    try {
      await savePolicy.mutateAsync({ keep_last: last ?? undefined, keep_days: days ?? undefined });
      toast.success("Política de retención guardada. No se ha borrado nada todavía.");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo guardar la política");
    }
  }

  async function onApply() {
    try {
      const report = await applyRetention.mutateAsync();
      toast.success(reportMessage(report));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo aplicar la retención");
      throw err;
    }
  }

  async function onGc() {
    try {
      const report = await collectGarbage.mutateAsync();
      toast.success(reportMessage(report));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo recolectar la basura");
      throw err;
    }
  }

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor="keep-last">Últimas versiones por paquete</Label>
          <Input
            id="keep-last"
            inputMode="numeric"
            placeholder="Sin límite"
            value={keepLast}
            onChange={(event) => setKeepLast(event.target.value)}
            disabled={!canWrite}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="keep-days">Días a conservar</Label>
          <Input
            id="keep-days"
            inputMode="numeric"
            placeholder="Sin límite"
            value={keepDays}
            onChange={(event) => setKeepDays(event.target.value)}
            disabled={!canWrite}
          />
        </div>
      </div>
      {canWrite ? (
        <div className="flex flex-wrap gap-2">
          <Button type="submit" variant="outline" disabled={savePolicy.isPending}>
            <Save />
            {savePolicy.isPending ? "Guardando…" : "Guardar política"}
          </Button>
          <ConfirmDeleteDialog
            title="Aplicar retención"
            description="Se borrarán las versiones fuera de la política y los binarios que queden huérfanos. En un Mirror, el siguiente install puede volver a cachearlas. Esta acción no se puede deshacer."
            confirmLabel="Aplicar"
            pending={applyRetention.isPending}
            onConfirm={onApply}
            trigger={
              <Button type="button" variant="outline">
                <Trash2 />
                Aplicar ahora
              </Button>
            }
          />
          <ConfirmDeleteDialog
            title="Recolectar basura"
            description="Se borrarán solo los binarios que ya no están referenciados por el índice (capas OCI huérfanas, restos de un borrado). Las versiones publicadas no se tocan."
            confirmLabel="Recolectar"
            pending={collectGarbage.isPending}
            onConfirm={onGc}
            trigger={
              <Button type="button" variant="outline">
                <Eraser />
                Recolectar basura
              </Button>
            }
          />
        </div>
      ) : null}
    </form>
  );
}
