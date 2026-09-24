import { type FormEvent, useState } from "react";
import { AlertCircle, FlaskConical, Save } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { AdmissionEffectDto } from "@/api/generated/AdmissionEffectDto";
import type { AdmissionEventResponse } from "@/api/generated/AdmissionEventResponse";
import type { AdmissionPolicyRequest } from "@/api/generated/AdmissionPolicyRequest";
import type { AdmissionPolicyResponse } from "@/api/generated/AdmissionPolicyResponse";
import type { AdmissionPreviewResponse } from "@/api/generated/AdmissionPreviewResponse";
import {
  useAdmissionEvents,
  useAdmissionPolicy,
  useDryRunAdmission,
  useSaveAdmissionPolicy,
} from "@/api/queries";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const EMPTY_POLICY: AdmissionPolicyResponse = {
  enabled: false,
  when: "pull",
  predicate: "not_signed",
  effect: "deny",
};

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

function payloadFromFields(
  enabled: boolean,
  effect: AdmissionEffectDto,
): AdmissionPolicyRequest {
  return {
    enabled,
    when: "pull",
    predicate: "not_signed",
    effect,
  };
}

export function AdmissionPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { data, isPending, isError, error } = useAdmissionPolicy(repositoryId, true);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Políticas</CardTitle>
        <CardDescription>
          Si una imagen no está firmada (Cosign / Notation), deniega el pull o solo avisa. No
          exige verificación criptográfica. En un Mirror rige esta regla; un Alloy usa la de
          cada miembro Forge o Mirror. Simular ignora si la regla está activada. Los avisos y
          denegaciones del pull quedan abajo.
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
                {error.message}. Puedes configurar y simular igual; guardar necesita la tabla de
                políticas.
              </AlertDescription>
            </Alert>
          )
        ) : null}
        {isPending ? null : (
          <AdmissionForm
            repositoryId={repositoryId}
            canWrite={canWrite}
            policy={data ?? EMPTY_POLICY}
          />
        )}
        <AdmissionEventLog repositoryId={repositoryId} />
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
        <p>Luego reinicia el backend. Simular funciona sin migrar; guardar, no.</p>
      </AlertDescription>
    </Alert>
  );
}

function AdmissionForm({
  repositoryId,
  canWrite,
  policy,
}: {
  repositoryId: string;
  canWrite: boolean;
  policy: AdmissionPolicyResponse;
}) {
  const savePolicy = useSaveAdmissionPolicy(repositoryId);
  const dryRun = useDryRunAdmission(repositoryId);
  const [enabled, setEnabled] = useState(policy.enabled);
  const [effect, setEffect] = useState<AdmissionEffectDto>(policy.effect);
  const [preview, setPreview] = useState<AdmissionPreviewResponse | null>(null);
  const [schemaError, setSchemaError] = useState(false);

  function rememberSchemaError(err: unknown) {
    const message = err instanceof ApiError ? err.message : "";
    if (isMissingMigration(message)) {
      setSchemaError(true);
    }
  }

  function onFieldsChange(nextEnabled: boolean, nextEffect: AdmissionEffectDto) {
    setEnabled(nextEnabled);
    setEffect(nextEffect);
    setPreview(null);
  }

  async function onSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      await savePolicy.mutateAsync(payloadFromFields(enabled, effect));
      setSchemaError(false);
      toast.success(
        enabled
          ? "Política guardada. Los pulls de imágenes sin firma se evaluarán a partir de ahora."
          : "Política guardada. Está desactivada: el pull no se bloquea.",
      );
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : "No se pudo guardar la política");
    }
  }

  async function onSimulate() {
    try {
      const result = await dryRun.mutateAsync(payloadFromFields(enabled, effect));
      setPreview(result);
      setSchemaError(false);
      toast.success(previewMessage(result));
    } catch (err) {
      rememberSchemaError(err);
      toast.error(err instanceof ApiError ? err.message : "No se pudo simular la política");
    }
  }

  return (
    <form onSubmit={(event) => void onSave(event)} className="space-y-5">
      {schemaError ? <MigrationAlert /> : null}

      <ol className="space-y-4 text-sm">
        <li className="space-y-3">
          <p className="font-medium text-foreground">1. Configurar</p>
          <label className="flex cursor-pointer items-start gap-2">
            <input
              type="checkbox"
              className="mt-1 size-4 accent-primary"
              checked={enabled}
              onChange={(event) => onFieldsChange(event.target.checked, effect)}
              disabled={!canWrite}
            />
            <span>
              <span className="font-medium">Activar</span>
              <span className="block text-xs text-muted-foreground">
                Sin marcar, la regla se guarda pero no bloquea ni avisa en el pull.
              </span>
            </span>
          </label>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>Cuándo</Label>
              <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm">
                Al bajar una imagen
              </p>
            </div>
            <div className="space-y-2">
              <Label>Si</Label>
              <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm">
                No está firmada
              </p>
            </div>
          </div>
          <div className="space-y-2">
            <Label htmlFor="admission-effect">Entonces</Label>
            <Select
              value={effect}
              onValueChange={(value) =>
                onFieldsChange(enabled, value as AdmissionEffectDto)
              }
              disabled={!canWrite}
            >
              <SelectTrigger id="admission-effect">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="deny">Denegar el pull</SelectItem>
                <SelectItem value="warn">Avisar y dejar pasar</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </li>
        {canWrite ? (
          <li className="space-y-3">
            <p className="font-medium text-foreground">2. Simular</p>
            <p className="text-muted-foreground">
              Recorre el inventario como si la regla estuviera activa. No bloquea pulls.
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
            Solo lectura: un usuario Developer o Admin puede simular y guardar.
          </p>
        )}
      </ol>

      {preview ? <AdmissionPreviewTable preview={preview} /> : null}

      {canWrite ? (
        <div className="space-y-3">
          <p className="text-sm font-medium text-foreground">3. Guardar</p>
          <p className="text-sm text-muted-foreground">
            Guardar aplica la regla a los pulls siguientes. Simula primero para ver qué imágenes
            quedarían fuera.
          </p>
          <Button type="submit" variant="outline" disabled={savePolicy.isPending}>
            <Save />
            {savePolicy.isPending ? "Guardando…" : "Guardar"}
          </Button>
        </div>
      ) : null}
    </form>
  );
}

function countPhrase(count: number, singular: string, plural: string): string {
  return `${count} ${count === 1 ? singular : plural}`;
}

function previewMessage(preview: AdmissionPreviewResponse): string {
  const allowed = countPhrase(preview.allowed, "pasaría", "pasarían");
  if (preview.matches.length === 0) {
    return `Ninguna imagen dispararía la regla. ${allowed}.`;
  }
  return `${countPhrase(preview.matches.length, "imagen dispararía", "imágenes dispararían")} la regla. ${allowed}.`;
}

function formatEventTime(value: string): string {
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) {
    return value;
  }
  return new Date(parsed).toLocaleString();
}

function AdmissionEventLog({ repositoryId }: { repositoryId: string }) {
  const { data, isPending, isError, error } = useAdmissionEvents(repositoryId, true);

  return (
    <div className="space-y-3 border-t border-border pt-5">
      <div>
        <p className="text-sm font-medium text-foreground">Avisos y denegaciones</p>
        <p className="text-sm text-muted-foreground">
          Últimos pulls que dispararon la regla activa. Avisar deja pasar y apunta aquí.
        </p>
      </div>
      {isPending ? <Skeleton className="h-16 w-full rounded-md" /> : null}
      {isError && error.message ? (
        <p className="text-sm text-muted-foreground">{error.message}</p>
      ) : null}
      {data && data.length === 0 ? (
        <p className="text-sm text-muted-foreground">Todavía no hay rastro de pulls.</p>
      ) : null}
      {data && data.length > 0 ? <AdmissionEventTable events={data} /> : null}
    </div>
  );
}

function AdmissionEventTable({ events }: { events: AdmissionEventResponse[] }) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Cuándo</TableHead>
          <TableHead>Imagen</TableHead>
          <TableHead>Etiqueta</TableHead>
          <TableHead>Efecto</TableHead>
          <TableHead>Motivo</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {events.map((event) => (
          <TableRow key={event.id}>
            <TableCell className="text-xs text-muted-foreground">
              {formatEventTime(event.created_at)}
            </TableCell>
            <TableCell className="font-mono text-xs">{event.name}</TableCell>
            <TableCell className="font-mono text-xs">{event.reference}</TableCell>
            <TableCell>
              <Badge variant={event.effect === "deny" ? "destructive" : "outline"}>
                {event.effect === "deny" ? "Denegado" : "Aviso"}
              </Badge>
            </TableCell>
            <TableCell>{event.reason}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

function AdmissionPreviewTable({ preview }: { preview: AdmissionPreviewResponse }) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">Simulación</Badge>
        <p className="text-sm text-muted-foreground">{previewMessage(preview)}</p>
      </div>
      {preview.matches.length === 0 ? null : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Imagen</TableHead>
              <TableHead>Etiqueta</TableHead>
              <TableHead>Efecto</TableHead>
              <TableHead>Motivo</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {preview.matches.map((item) => (
              <TableRow key={`${item.name}:${item.version}:${item.effect}`}>
                <TableCell className="font-mono text-xs">{item.name}</TableCell>
                <TableCell className="font-mono text-xs">{item.version}</TableCell>
                <TableCell>{item.effect === "deny" ? "Denegar" : "Avisar"}</TableCell>
                <TableCell>{item.reason}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
