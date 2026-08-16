import { type FormEvent, useState } from "react";
import { AlertCircle, FlaskConical, Save, Trash2 } from "lucide-react";
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
        <CardTitle>Retención</CardTitle>
        <CardDescription>
          Conserva las N versiones más recientes y/o las publicadas en los últimos N días. Vacío =
          no hay límite. Aplicar las saca del catálogo; install y publish no se bloquean. El disco
          se libera en Configuración → Recolección de basura.
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
      toast.success("Política guardada. No se ha sacado nada del catálogo; pulsa Simular para ver el efecto.");
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
      toast.success(previewCatalogMessage(result));
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
      toast.success(appliedCatalogMessage(result));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : "No se pudo aplicar la retención");
      throw err;
    }
  }

  const applyDescription =
    preview === null
      ? "Se guardará la política y se sacarán del catálogo las versiones fuera de ella. El disco no se borra hasta la recolección de basura. En un Mirror, el siguiente install puede volver a cachearlas. Esta acción no se puede deshacer."
      : `${previewCatalogMessage(preview)} Se guardará la política y se sacará esa lista del catálogo. El disco no se borra hasta la recolección de basura. En un Mirror, el siguiente install puede volver a cachearlas. Esta acción no se puede deshacer.`;

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
            <p className="font-medium text-foreground">2. Simular</p>
            <p className="text-muted-foreground">
              Usa los números del formulario, no hace falta guardar. No saca nada del catálogo.
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
            Solo lectura: un usuario Developer o Admin puede simular y aplicar.
          </p>
        )}
      </ol>

      {preview ? (
        <CleanupPreviewTable
          preview={preview}
          summary={preview.dry_run ? previewCatalogMessage(preview) : appliedCatalogMessage(preview)}
        />
      ) : null}

      {canWrite ? (
        <div className="space-y-3">
          <p className="text-sm font-medium text-foreground">3. Aplicar o solo guardar</p>
          <p className="text-sm text-muted-foreground">
            Aplicar guarda estos números y saca del catálogo lo que mostró Simular. Simula primero.
            Guardar no toca el catálogo. El disco se libera después, en Configuración.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button type="submit" variant="outline" disabled={savePolicy.isPending}>
              <Save />
              {savePolicy.isPending ? "Guardando…" : "Guardar sin aplicar"}
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
          </div>
        </div>
      ) : null}
    </form>
  );
}
