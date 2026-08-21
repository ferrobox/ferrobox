import { type FormEvent, useState } from "react";
import { AlertCircle, Bell, Loader2, Send, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import type { WebhookEventDto } from "@/api/generated/WebhookEventDto";
import type { WebhookResponse } from "@/api/generated/WebhookResponse";
import {
  useCreateWebhook,
  useDeleteWebhook,
  usePingWebhook,
  useUpdateWebhook,
  useWebhookDeliveries,
  useWebhooks,
} from "@/api/queries";
import { ConfirmDeleteDialog } from "@/components/repository/ConfirmDeleteDialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";

const EVENTS: readonly { value: WebhookEventDto; label: string; hint: string }[] = [
  {
    value: "assay.completed",
    label: "Ensaye completado",
    hint: "Incluye recuento de hallazgos (Critical, High, …). El install no espera.",
  },
  {
    value: "package.published",
    label: "Paquete publicado",
    hint: "Al publicar en un Forge o al cachear en un Mirror.",
  },
];

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

export function WebhooksPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const webhooksQuery = useWebhooks(repositoryId, canWrite);
  const createWebhook = useCreateWebhook(repositoryId);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [events, setEvents] = useState<WebhookEventDto[]>(["assay.completed"]);

  async function onCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (events.length === 0) {
      toast.error("Elige al menos un evento");
      return;
    }
    try {
      await createWebhook.mutateAsync({
        name: name.trim(),
        url: url.trim(),
        secret: secret.trim() || undefined,
        events,
        enabled: true,
      });
      setName("");
      setUrl("");
      setSecret("");
      toast.success("Aviso creado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo crear el aviso");
    }
  }

  function toggleEvent(value: WebhookEventDto) {
    setEvents((current) =>
      current.includes(value) ? current.filter((item) => item !== value) : [...current, value],
    );
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Avisos HTTP</CardTitle>
          <CardDescription>
            FerroBox envía un POST JSON cuando ocurre un evento. El publish y el install no
            esperan a la respuesta: un destino caído no bloquea el catálogo.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {!canWrite ? (
            <p className="text-sm text-muted-foreground">
              Solo quien puede escribir en este repositorio configura avisos.
            </p>
          ) : null}

          {webhooksQuery.isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}

          {webhooksQuery.isError ? (
            isMissingMigration(webhooksQuery.error.message) ? (
              <Alert>
                <AlertCircle />
                <AlertTitle>Falta una migración SQL</AlertTitle>
                <AlertDescription>
                  Desde el directorio <code className="font-mono">backend</code> ejecuta{" "}
                  <code className="font-mono">sqlx migrate run</code> y reinicia el backend.
                </AlertDescription>
              </Alert>
            ) : (
              <Alert variant="destructive">
                <AlertCircle />
                <AlertTitle>No se pudieron cargar los avisos</AlertTitle>
                <AlertDescription>{webhooksQuery.error.message}</AlertDescription>
              </Alert>
            )
          ) : null}

          {canWrite ? (
            <form onSubmit={(event) => void onCreate(event)} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="webhook-name">Nombre</Label>
                  <Input
                    id="webhook-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder="Slack CI"
                    required
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="webhook-url">URL</Label>
                  <Input
                    id="webhook-url"
                    type="url"
                    value={url}
                    onChange={(event) => setUrl(event.target.value)}
                    placeholder="https://example.test/ferrobox"
                    required
                  />
                </div>
              </div>
              <div className="space-y-2">
                <Label htmlFor="webhook-secret">Secreto HMAC (opcional)</Label>
                <Input
                  id="webhook-secret"
                  type="password"
                  autoComplete="off"
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                  placeholder="Se envía como X-FerroBox-Signature"
                />
              </div>
              <fieldset className="space-y-2">
                <legend className="text-sm font-medium">Eventos</legend>
                {EVENTS.map((item) => (
                  <label key={item.value} className="flex cursor-pointer items-start gap-2 text-sm">
                    <input
                      type="checkbox"
                      className="mt-1 size-4 accent-primary"
                      checked={events.includes(item.value)}
                      onChange={() => toggleEvent(item.value)}
                    />
                    <span>
                      <span className="font-medium">{item.label}</span>
                      <span className="block text-xs text-muted-foreground">{item.hint}</span>
                    </span>
                  </label>
                ))}
              </fieldset>
              <Button type="submit" disabled={createWebhook.isPending || name.trim().length === 0}>
                {createWebhook.isPending ? <Loader2 className="animate-spin" /> : <Bell />}
                Crear aviso
              </Button>
            </form>
          ) : null}
        </CardContent>
      </Card>

      {webhooksQuery.data?.map((webhook) => (
        <WebhookCard key={webhook.id} repositoryId={repositoryId} webhook={webhook} canWrite={canWrite} />
      ))}
    </div>
  );
}

function WebhookCard({
  repositoryId,
  webhook,
  canWrite,
}: {
  repositoryId: string;
  webhook: WebhookResponse;
  canWrite: boolean;
}) {
  const updateWebhook = useUpdateWebhook(repositoryId);
  const deleteWebhook = useDeleteWebhook(repositoryId);
  const pingWebhook = usePingWebhook(repositoryId);
  const deliveriesQuery = useWebhookDeliveries(repositoryId, webhook.id, canWrite);

  async function onToggle() {
    try {
      await updateWebhook.mutateAsync({
        webhookId: webhook.id,
        payload: {
          name: webhook.name,
          url: webhook.url,
          events: webhook.events,
          enabled: !webhook.enabled,
        },
      });
      toast.success(webhook.enabled ? "Aviso pausado" : "Aviso activado");
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo actualizar el aviso");
    }
  }

  async function onPing() {
    try {
      const delivery = await pingWebhook.mutateAsync(webhook.id);
      if (delivery.status === "success") {
        toast.success("El destino respondió bien");
      } else {
        toast.error(delivery.error ?? "El destino no respondió 2xx");
      }
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo probar el aviso");
    }
  }

  async function onDelete() {
    try {
      await deleteWebhook.mutateAsync(webhook.id);
      toast.success(`Aviso «${webhook.name}» eliminado`);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "No se pudo eliminar el aviso");
      throw err;
    }
  }

  return (
    <Card>
      <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <CardTitle className="text-base">{webhook.name}</CardTitle>
          <CardDescription className="break-all font-mono text-xs">{webhook.url}</CardDescription>
          <p className="mt-2 text-xs text-muted-foreground">
            {webhook.events.map(eventLabel).join(" · ")}
            {webhook.has_secret ? " · firma HMAC" : ""}
            {webhook.enabled ? "" : " · pausado"}
          </p>
        </div>
        {canWrite ? (
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => void onToggle()}
              disabled={updateWebhook.isPending}
            >
              {webhook.enabled ? "Pausar" : "Activar"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void onPing()}
              disabled={pingWebhook.isPending}
            >
              {pingWebhook.isPending ? <Loader2 className="animate-spin" /> : <Send />}
              Probar
            </Button>
            <ConfirmDeleteDialog
              title={`Eliminar «${webhook.name}»`}
              description="Se dejarán de enviar avisos a esta URL. El historial de envíos también se borra."
              pending={deleteWebhook.isPending}
              onConfirm={onDelete}
              trigger={
                <Button size="sm" variant="ghost">
                  <Trash2 />
                  Eliminar
                </Button>
              }
            />
          </div>
        ) : null}
      </CardHeader>
      <CardContent>
        {deliveriesQuery.isPending ? <Skeleton className="h-16 w-full rounded-md" /> : null}
        {deliveriesQuery.data && deliveriesQuery.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">Todavía no hay envíos. Usa Probar para un ping.</p>
        ) : null}
        {deliveriesQuery.data && deliveriesQuery.data.length > 0 ? (
          <ul className="space-y-2 text-sm">
            {deliveriesQuery.data.map((delivery) => (
              <li key={delivery.id} className="rounded-md border border-border px-3 py-2">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="font-medium">{delivery.event}</span>
                  <span
                    className={
                      delivery.status === "success" ? "text-emerald-500" : "text-destructive"
                    }
                  >
                    {delivery.status === "success" ? "OK" : "Falló"}
                    {delivery.http_status != null ? ` · HTTP ${delivery.http_status}` : ""}
                  </span>
                </div>
                <p className="mt-1 text-xs text-muted-foreground">
                  {formatWhen(delivery.created_at)}
                  {delivery.error ? ` · ${delivery.error}` : ""}
                </p>
              </li>
            ))}
          </ul>
        ) : null}
      </CardContent>
    </Card>
  );
}

function eventLabel(event: WebhookEventDto): string {
  return EVENTS.find((item) => item.value === event)?.label ?? event;
}

function formatWhen(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}
