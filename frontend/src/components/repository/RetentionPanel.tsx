import { type FormEvent, useState } from "react";
import { AlertCircle, Eraser, FlaskConical, Save, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { CleanupPreviewResponse } from "@/api/generated/CleanupPreviewResponse";
import type { CleanupReportResponse } from "@/api/generated/CleanupReportResponse";
import type { RetentionPolicyRequest } from "@/api/generated/RetentionPolicyRequest";
import type { RetentionPolicyResponse } from "@/api/generated/RetentionPolicyResponse";
import {
  useApplyRetention,
  useCollectGarbage,
  useDryRunRetention,
  useRetentionPolicy,
  useSaveRetentionPolicy,
} from "@/api/queries";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatBytes } from "@/lib/format";

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

function reportMessage(report: CleanupReportResponse | CleanupPreviewResponse): string {
  return `Eliminadas ${String(report.dropped_versions)} versiones y ${String(report.deleted_artifacts)} binarios (${formatBytes(report.freed_bytes)}).`;
}

function previewMessage(preview: CleanupPreviewResponse): string {
  if (preview.items.length === 0) {
    return "Nada que borrar con esta política.";
  }
  return `Se eliminarían ${String(preview.dropped_versions)} versiones y ${String(preview.deleted_artifacts)} binarios (${formatBytes(preview.freed_bytes)}).`;
}

function payloadFromFields(
  keepLast: string,
  keepDays: string,
): RetentionPolicyRequest | undefined {
  const last = parseLimit(keepLast);
  const days = parseLimit(keepDays);
  if (last === undefined || days === undefined) {
    toast.error("Los límites deben ser números enteros, o quedar vacíos");
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
  const { data, isPending, isError, error } = useRetentionPolicy(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Retención y basura</CardTitle>
        <CardDescription>
          Configura los límites, pulsa Simular para ver qué se borraría sin tocar nada, y solo
          entonces Aplicar. Vacío = no hay límite. No bloquea install ni publish.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}
        {isError && error.message ? (
          isMissingMigration(error.message) ? (
            <MigrationAlert />
          ) : (
            <Alert variant="destructive">
              <AlertCircle />
              <AlertTitle>No se pudo cargar la política guardada</AlertTitle>
              <AlertDescription>
                {error.message}. Puedes configurar y simular igual; guardar y aplicar necesitan la
                tabla de políticas.
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
  return (
    <Alert variant="destructive">
      <AlertCircle />
      <AlertTitle>Falta la migración SQL</AlertTitle>
      <AlertDescription className="gap-2">
        <p>
          La tabla de políticas no existe. Desde el directorio <code>backend/</code> ejecuta:
        </p>
        <pre className="mt-1 w-full overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs text-foreground">
          sqlx migrate run
        </pre>
        <p>Luego reinicia el backend. Simular funciona sin migrar; guardar y aplicar, no.</p>
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
  const savePolicy = useSaveRetentionPolicy(repositoryId);
  const dryRun = useDryRunRetention(repositoryId);
  const applyRetention = useApplyRetention(repositoryId);
  const collectGarbage = useCollectGarbage(repositoryId);
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
    const payload = payloadFromFields(keepLast, keepDays);
    if (!payload) {
      return;
    }
    try {
      await savePolicy.mutateAsync(payload);
      setSchemaError(false);
      toast.success("Política guardada. No se ha borrado nada; pulsa Simular para ver el efecto.");
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : "No se pudo guardar la política");
    }
  }

  async function onSimulate() {
    const payload = payloadFromFields(keepLast, keepDays);
    if (!payload) {
      return;
    }
    try {
      const result = await dryRun.mutateAsync(payload);
      setPreview(result);
      setSchemaError(false);
      toast.success(previewMessage(result));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : "No se pudo simular la retención");
    }
  }

  async function onApply() {
    const payload = payloadFromFields(keepLast, keepDays);
    if (!payload) {
      throw new Error("política inválida");
    }
    try {
      const result = await applyRetention.mutateAsync(payload);
      setPreview(result);
      setSchemaError(false);
      toast.success(reportMessage(result));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : "No se pudo aplicar la retención");
      throw err;
    }
  }

  async function onGc() {
    try {
      const report = await collectGarbage.mutateAsync();
      toast.success(reportMessage(report));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : "No se pudo recolectar la basura");
      throw err;
    }
  }

  const applyDescription =
    preview === null
      ? "Se guardará la política del formulario y se borrarán las versiones fuera de ella, más los binarios huérfanos. Simula antes para ver la lista. En un Mirror, el siguiente install puede volver a cachearlas. Esta acción no se puede deshacer."
      : `${previewMessage(preview)} Se guardará la política del formulario y se borrará esa lista. En un Mirror, el siguiente install puede volver a cachearlas. Esta acción no se puede deshacer.`;

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-5">
      {schemaError ? <MigrationAlert /> : null}

      <ol className="space-y-4 text-sm">
        <li className="space-y-3">
          <p className="font-medium text-foreground">1. Configurar</p>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="keep-last">Últimas versiones por paquete</Label>
              <Input
                id="keep-last"
                inputMode="numeric"
                placeholder="Sin límite"
                value={keepLast}
                onChange={(event) => onFieldsChange(event.target.value, keepDays)}
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
                onChange={(event) => onFieldsChange(keepLast, event.target.value)}
                disabled={!canWrite}
              />
            </div>
          </div>
        </li>
        {canWrite ? (
          <li className="space-y-3">
            <p className="font-medium text-foreground">2. Simular (dry-run)</p>
            <p className="text-muted-foreground">
              Usa los números del formulario, no hace falta guardar. No borra nada.
            </p>
            <Button
              type="button"
              variant="outline"
              disabled={dryRun.isPending}
              onClick={() => void onSimulate()}
            >
              <FlaskConical />
              {dryRun.isPending ? "Simulando…" : "Simular"}
            </Button>
          </li>
        ) : (
          <p className="text-muted-foreground">
            Solo lectura: un usuario Writer o Admin puede simular y aplicar.
          </p>
        )}
      </ol>

      {preview ? <PreviewTable preview={preview} /> : null}

      {canWrite ? (
        <div className="space-y-3">
          <p className="text-sm font-medium text-foreground">3. Aplicar o solo guardar</p>
          <p className="text-sm text-muted-foreground">
            Aplicar guarda estos números y borra lo que mostró Simular. Simula primero: así no hay
            sorpresas. Guardar no borra.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button type="submit" variant="outline" disabled={savePolicy.isPending}>
              <Save />
              {savePolicy.isPending ? "Guardando…" : "Guardar sin borrar"}
            </Button>
            <ConfirmDeleteDialog
              title="Aplicar retención"
              description={applyDescription}
              confirmLabel="Aplicar"
              pending={applyRetention.isPending}
              onConfirm={onApply}
              trigger={
                <Button type="button" variant="destructive" disabled={preview === null}>
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
        </div>
      ) : null}
    </form>
  );
}

function PreviewTable({ preview }: { preview: CleanupPreviewResponse }) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={preview.dry_run ? "outline" : "destructive"}>
          {preview.dry_run ? "Simulación" : "Aplicado"}
        </Badge>
        <p className="text-sm text-muted-foreground">
          {preview.dry_run ? previewMessage(preview) : reportMessage(preview)}
        </p>
      </div>
      {preview.items.length === 0 ? null : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Paquete</TableHead>
              <TableHead>Versión</TableHead>
              <TableHead>Motivo</TableHead>
              <TableHead className="text-right">Tamaño</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {preview.items.map((item) => (
              <TableRow key={`${item.name}:${item.version}:${item.reason}`}>
                <TableCell className="font-mono text-xs">
                  {item.name.length > 0 ? item.name : "—"}
                </TableCell>
                <TableCell className="font-mono text-xs">{item.version}</TableCell>
                <TableCell>{item.reason}</TableCell>
                <TableCell className="text-right">{formatBytes(item.size_bytes)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
