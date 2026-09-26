import { type FormEvent, useState } from "react";
import { AlertCircle, Bell, Loader2, Send, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
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

const EVENT_VALUES: readonly WebhookEventDto[] = ["assay.completed", "package.published"];

function isMissingMigration(message: string): boolean {
  return message.includes("sqlx migrate run");
}

function eventCopy(
  event: WebhookEventDto,
  t: (key: string) => string,
): { label: string; hint: string } {
  if (event === "assay.completed") {
    return { label: t("webhooks.assayCompleted"), hint: t("webhooks.assayCompletedHint") };
  }
  return { label: t("webhooks.packagePublished"), hint: t("webhooks.packagePublishedHint") };
}

export function WebhooksPanel({
  repositoryId,
  canWrite,
}: {
  repositoryId: string;
  canWrite: boolean;
}) {
  const { t } = useTranslation();
  const webhooksQuery = useWebhooks(repositoryId, canWrite);
  const createWebhook = useCreateWebhook(repositoryId);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [events, setEvents] = useState<WebhookEventDto[]>(["assay.completed"]);

  async function onCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (events.length === 0) {
      toast.error(t("webhooks.needEvent"));
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
      toast.success(t("webhooks.created"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("webhooks.createFailed"));
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
          <CardTitle>{t("webhooks.title")}</CardTitle>
          <CardDescription>{t("webhooks.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {!canWrite ? (
            <p className="text-sm text-muted-foreground">{t("webhooks.readOnly")}</p>
          ) : null}

          {webhooksQuery.isPending ? <Skeleton className="h-24 w-full rounded-md" /> : null}

          {webhooksQuery.isError ? (
            isMissingMigration(webhooksQuery.error.message) ? (
              <Alert>
                <AlertCircle />
                <AlertTitle>{t("retention.migrationTitle")}</AlertTitle>
                <AlertDescription>{t("webhooks.migrationBody")}</AlertDescription>
              </Alert>
            ) : (
              <Alert variant="destructive">
                <AlertCircle />
                <AlertTitle>{t("webhooks.loadFailed")}</AlertTitle>
                <AlertDescription>{webhooksQuery.error.message}</AlertDescription>
              </Alert>
            )
          ) : null}

          {canWrite ? (
            <form onSubmit={(event) => void onCreate(event)} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="webhook-name">{t("common.name")}</Label>
                  <Input
                    id="webhook-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder={t("webhooks.namePlaceholder")}
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
                <Label htmlFor="webhook-secret">{t("webhooks.secret")}</Label>
                <Input
                  id="webhook-secret"
                  type="password"
                  autoComplete="off"
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                  placeholder={t("webhooks.secretPlaceholder")}
                />
              </div>
              <fieldset className="space-y-2">
                <legend className="text-sm font-medium">{t("webhooks.events")}</legend>
                {EVENT_VALUES.map((value) => {
                  const copy = eventCopy(value, t);
                  return (
                    <label key={value} className="flex cursor-pointer items-start gap-2 text-sm">
                      <input
                        type="checkbox"
                        className="mt-1 size-4 accent-primary"
                        checked={events.includes(value)}
                        onChange={() => toggleEvent(value)}
                      />
                      <span>
                        <span className="font-medium">{copy.label}</span>
                        <span className="block text-xs text-muted-foreground">{copy.hint}</span>
                      </span>
                    </label>
                  );
                })}
              </fieldset>
              <Button type="submit" disabled={createWebhook.isPending || name.trim().length === 0}>
                {createWebhook.isPending ? <Loader2 className="animate-spin" /> : <Bell />}
                {t("webhooks.create")}
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
  const { t } = useTranslation();
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
      toast.success(webhook.enabled ? t("webhooks.paused") : t("webhooks.enabled"));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("webhooks.updateFailed"));
    }
  }

  async function onPing() {
    try {
      const delivery = await pingWebhook.mutateAsync(webhook.id);
      if (delivery.status === "success") {
        toast.success(t("webhooks.pingOk"));
      } else {
        toast.error(delivery.error ?? t("webhooks.pingFailed"));
      }
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("webhooks.testFailed"));
    }
  }

  async function onDelete() {
    try {
      await deleteWebhook.mutateAsync(webhook.id);
      toast.success(t("webhooks.deleted", { name: webhook.name }));
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : t("webhooks.deleteFailed"));
      throw err;
    }
  }

  const extras = [
    webhook.has_secret ? t("webhooks.hmac") : null,
    webhook.enabled ? null : t("webhooks.pausedSuffix"),
  ].filter((item): item is string => item != null);

  return (
    <Card>
      <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <CardTitle className="text-base">{webhook.name}</CardTitle>
          <CardDescription className="break-all font-mono text-xs">{webhook.url}</CardDescription>
          <p className="mt-2 text-xs text-muted-foreground">
            {webhook.events.map((event) => eventCopy(event, t).label).join(" · ")}
            {extras.length > 0 ? ` · ${extras.join(" · ")}` : ""}
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
              {webhook.enabled ? t("webhooks.pause") : t("webhooks.enable")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void onPing()}
              disabled={pingWebhook.isPending}
            >
              {pingWebhook.isPending ? <Loader2 className="animate-spin" /> : <Send />}
              {t("webhooks.test")}
            </Button>
            <ConfirmDeleteDialog
              title={t("webhooks.deleteTitle", { name: webhook.name })}
              description={t("webhooks.deleteBody")}
              pending={deleteWebhook.isPending}
              onConfirm={onDelete}
              trigger={
                <Button size="sm" variant="ghost">
                  <Trash2 />
                  {t("common.delete")}
                </Button>
              }
            />
          </div>
        ) : null}
      </CardHeader>
      <CardContent>
        {deliveriesQuery.isPending ? <Skeleton className="h-16 w-full rounded-md" /> : null}
        {deliveriesQuery.data && deliveriesQuery.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("webhooks.noDeliveries")}</p>
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
                    {delivery.status === "success" ? "OK" : t("webhooks.failed")}
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

function formatWhen(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}
